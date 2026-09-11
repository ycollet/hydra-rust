//! Rewrites JS object literals (`{key: value, ...}`) into Rhai's map
//! literal syntax (`#{key: value, ...}`) wherever one appears in *value*
//! position - a function-call argument, an assignment's right-hand side,
//! an array element, or a nested property value. Rhai's map literal
//! grammar otherwise already accepts exactly the same `key: value` shape
//! (bare-identifier or quoted-string keys, either works) - the only thing
//! missing is the leading `#`, so this never needs to touch anything
//! *inside* the braces.
//!
//! Real sketches commonly pass one of these into a call they expect some
//! other JS library to understand (`s0.init({src: canvas})`,
//! `P5({mode:"WEBGL"})`, `THREE.MeshBasicMaterial({color: 0x00ff00})`) -
//! hydra-rust doesn't implement any of those libraries, so the call itself
//! still won't do anything meaningful even once this parses. But before
//! this pass, the object literal was an unconditional hard parse error
//! that aborted the *entire* script; parsing it as an (unused) Rhai map
//! lets whatever real hydra content follows it in the same script still
//! run. Same reasoning as `arrowfn`'s handling of property-assigned arrows.
//!
//! The hard part is telling a value-position `{` apart from a *block*
//! (an `if`/`while`/`for`/function body, arrow function body, etc.) -
//! JS itself has this exact ambiguity, resolved by "prefer block unless
//! there's affirmative evidence otherwise" (why a statement-position
//! object literal needs wrapping parens in real JS). This pass uses the
//! same default: a `{` only counts as an object literal if the nearest
//! preceding non-whitespace token is one that can only precede an
//! *expression* - `(`, `,`, `[`, `:`, a bare (non-comparison, non-arrow)
//! `=`, or the `return` keyword. Anything else (including "nothing", i.e.
//! start of input) is left alone as a block.
//!
//! Runs right after `arrow::strip_zero_arg_arrows`, not before: a
//! destructured-parameter reactive-value arrow (`({time})=>expr`) has a
//! `{` preceded by `(`, exactly like a call-argument object literal
//! (`s0.init({src: canvas})`) - indistinguishable from *this* pass's
//! purely-local, backward-looking check alone. `arrow.rs` tells them apart
//! with a stronger, more specific check (a destructure pattern is bare
//! identifiers only, no `:`), so running after it means any `{` this pass
//! still sees immediately after `(` is never a destructure pattern - if it
//! had been one, `arrow.rs` would already have stripped it away.

use crate::srcscan::mask_strings_and_comments;

/// Bare-identifier object keys that Rhai's own grammar reserves for other
/// use (so they need to be quoted, `"default": ...` instead of `default:
/// ...`, to still work as a map key). Real hydra.js/JS has no such
/// restriction, so real sketches use these freely - extend this list if
/// the corpus turns up others.
const RESERVED_KEYS: &[&str] = &["default"];

pub fn rewrite_object_literals(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let mut out = String::with_capacity(src.len() + 8);
    // Tracks, for each currently-open `{`, whether it's a map literal -
    // needed so a reserved-word key is only requoted while directly inside
    // one, not e.g. inside a nested plain block within a map's value.
    let mut brace_is_map: Vec<bool> = Vec::new();

    let mut i = 0;
    while i < chars.len() {
        if mask[i] {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        match chars[i] {
            '{' => {
                let is_map = is_value_position(&chars, &mask, i);
                if is_map {
                    out.push('#');
                }
                out.push('{');
                brace_is_map.push(is_map);
                i += 1;
            }
            '}' => {
                out.push('}');
                brace_is_map.pop();
                i += 1;
            }
            _ if brace_is_map.last() == Some(&true)
                && let Some(word) = reserved_key_at(&chars, &mask, i) =>
            {
                out.push('"');
                out.push_str(word);
                out.push('"');
                i += word.chars().count();
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }

    out
}

/// If one of `RESERVED_KEYS` starts (as a whole word) at `i` and is
/// immediately followed (skipping whitespace) by `:`, returns it.
fn reserved_key_at<'a>(chars: &[char], mask: &[bool], i: usize) -> Option<&'a str> {
    for &word in RESERVED_KEYS {
        let wlen = word.chars().count();
        if i + wlen > chars.len() || mask[i..i + wlen].iter().any(|m| *m) {
            continue;
        }
        if chars[i..i + wlen].iter().collect::<String>() != word {
            continue;
        }
        if chars.get(i + wlen).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            continue;
        }
        let mut k = i + wlen;
        while k < chars.len() && (mask[k] || chars[k].is_whitespace()) {
            k += 1;
        }
        if chars.get(k) == Some(&':') {
            return Some(word);
        }
    }
    None
}

/// True if the brace at `brace_idx` is preceded (skipping whitespace and
/// masked string/comment regions) by a token that can only precede an
/// expression - meaning this `{` opens a value, not a block.
fn is_value_position(chars: &[char], mask: &[bool], brace_idx: usize) -> bool {
    let mut j = brace_idx;
    while j > 0 {
        j -= 1;
        if mask[j] || chars[j].is_whitespace() {
            continue;
        }
        return match chars[j] {
            '(' | ',' | '[' | ':' => true,
            '=' => {
                let is_multi_char_op = matches!(chars.get(j + 1), Some('=') | Some('>'))
                    || (j > 0 && matches!(chars[j - 1], '!' | '<' | '>' | '='));
                !is_multi_char_op
            }
            _ => ends_with_return_keyword(chars, mask, j),
        };
    }
    false
}

/// True if the word ending at (and including) index `end` is the bare
/// keyword `return`.
fn ends_with_return_keyword(chars: &[char], mask: &[bool], end: usize) -> bool {
    const WORD: &str = "return";
    if end + 1 < WORD.len() {
        return false;
    }
    let start = end + 1 - WORD.len();
    if mask[start..=end].iter().any(|m| *m) {
        return false;
    }
    if chars[start..=end].iter().collect::<String>() != WORD {
        return false;
    }
    start == 0 || !is_ident_char(chars[start - 1])
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::rewrite_object_literals;

    #[test]
    fn rewrites_a_call_argument_object_literal() {
        assert_eq!(
            rewrite_object_literals("s0.init({src: canvas})"),
            "s0.init(#{src: canvas})"
        );
    }

    #[test]
    fn rewrites_an_assignment_right_hand_side() {
        assert_eq!(
            rewrite_object_literals("let cfg = {mode: \"WEBGL\"};"),
            "let cfg = #{mode: \"WEBGL\"};"
        );
    }

    #[test]
    fn rewrites_array_elements() {
        assert_eq!(
            rewrite_object_literals("[{a: 1}, {b: 2}]"),
            "[#{a: 1}, #{b: 2}]"
        );
    }

    #[test]
    fn rewrites_nested_object_literals() {
        assert_eq!(
            rewrite_object_literals("foo({a: {b: 1}})"),
            "foo(#{a: #{b: 1}})"
        );
    }

    #[test]
    fn rewrites_a_multiline_object_literal() {
        let src = "setFunction({\n  name: \"uvoronoid\",\n  type: \"src\",\n})";
        let expected = "setFunction(#{\n  name: \"uvoronoid\",\n  type: \"src\",\n})";
        assert_eq!(rewrite_object_literals(src), expected);
    }

    #[test]
    fn rewrites_after_return() {
        assert_eq!(
            rewrite_object_literals("function f() { return {a: 1}; }"),
            "function f() { return #{a: 1}; }"
        );
    }

    #[test]
    fn leaves_an_if_block_alone() {
        let src = "if (x) { y = 1; }";
        assert_eq!(rewrite_object_literals(src), src);
    }

    #[test]
    fn leaves_a_function_body_alone() {
        let src = "function f() { return 1; }";
        assert_eq!(rewrite_object_literals(src), src);
    }

    #[test]
    fn leaves_an_arrow_function_block_body_alone() {
        let src = "let f = () => { return 1; };";
        assert_eq!(rewrite_object_literals(src), src);
    }

    #[test]
    fn leaves_a_while_block_alone() {
        let src = "while (x) { x = x - 1; }";
        assert_eq!(rewrite_object_literals(src), src);
    }

    #[test]
    fn leaves_a_bare_statement_block_alone() {
        // ambiguous in real JS too (needs wrapping parens to be a value);
        // defaults to "block", same as JS's own disambiguation rule.
        let src = "{ let x = 1; }";
        assert_eq!(rewrite_object_literals(src), src);
    }

    #[test]
    fn does_not_misfire_on_a_comparison_before_the_brace() {
        let src = "if (a == b) { c = 1; }";
        assert_eq!(rewrite_object_literals(src), src);
    }

    #[test]
    fn quotes_a_reserved_word_used_as_a_key() {
        assert_eq!(
            rewrite_object_literals("setFunction({name: \"blur\", default: 1})"),
            "setFunction(#{name: \"blur\", \"default\": 1})"
        );
    }

    #[test]
    fn quotes_a_reserved_word_key_in_a_nested_object_literal() {
        assert_eq!(
            rewrite_object_literals("foo({a: {default: 1}})"),
            "foo(#{a: #{\"default\": 1}})"
        );
    }

    #[test]
    fn does_not_quote_default_used_as_a_plain_value() {
        assert_eq!(
            rewrite_object_literals("foo({a: default})"),
            "foo(#{a: default})"
        );
    }

    #[test]
    fn does_not_quote_default_outside_any_object_literal() {
        let src = "let default = 1;";
        assert_eq!(rewrite_object_literals(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"foo({a:1})\") // foo({a:1})";
        assert_eq!(rewrite_object_literals(src), src);
    }
}
