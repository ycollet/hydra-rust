//! Strips JS-style "named argument" call syntax: real sketches often call
//! hydra functions with each argument prefixed by its (real) parameter
//! name and an `=`, e.g. `noise(scale=153.413, offset=0.165)` instead of
//! the equivalent positional `noise(153.413, 0.165)`. This isn't a real
//! JS named-argument feature - it's JS's ordinary assignment-expression-
//! as-value trick (`foo(x = 5)` assigns global `x` *and* passes `5` as
//! the argument), (mis)used purely as self-documenting call-site syntax:
//! the "assigned" name is always just the real parameter's name, and
//! never referenced again as an actual variable.
//!
//! Rhai has no assignment-as-expression at all, so any argument shaped
//! like `name = value` was a hard "Expecting ',' to separate the
//! arguments" parse failure - regardless of which function was being
//! called, since this is a general JS idiom, not something specific to
//! any one hydra function. Strips the `name =` prefix from each such
//! argument, keeping just its value.
//!
//! A parenthesized group is deliberately left **untouched** (copied
//! verbatim, not recursed into) when it's actually a *parameter list*
//! rather than a call's arguments - `function name(min=0, max=1) {...}`
//! or `(min=0, max=1) => ...` also have `name = value`-shaped entries,
//! but there they're real default parameter values that
//! `jsfunctions`/`arrowfn` (which run later) need to see intact.
//! Detected structurally: a `(` immediately preceded by the `function`
//! keyword, or whose matching `)` is immediately followed by `=>`.

use crate::srcscan::mask_strings_and_comments;

pub fn strip_named_args(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    transform(&chars, &mask, 0, chars.len())
}

fn transform(chars: &[char], mask: &[bool], start: usize, end: usize) -> String {
    let mut out = String::new();
    let mut i = start;

    while i < end {
        if mask[i] {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if chars[i] == '(' {
            // No matching close (unbalanced parens): copy the rest of this
            // range verbatim rather than guessing at a bogus argument split.
            let Some(close) = matching_paren(chars, mask, i).filter(|c| *c < end) else {
                out.extend(&chars[i..end]);
                return out;
            };
            if is_declaration_parens(chars, mask, i, close) {
                out.extend(&chars[i..=close]);
            } else {
                out.push('(');
                out.push_str(&process_args(chars, mask, i + 1, close));
                out.push(')');
            }
            i = close + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

/// True if the parens `chars[open..=close]` are a declaration's parameter
/// list rather than a call's arguments: preceded by `function NAME` (the
/// `function` keyword sits *two* words before `(` - past the function's
/// own name), or immediately followed by `=>`.
fn is_declaration_parens(chars: &[char], mask: &[bool], open: usize, close: usize) -> bool {
    if let Some((name_start, _)) = word_ending_before(chars, mask, open)
        && let Some((kw_start, kw_end)) = word_ending_before(chars, mask, name_start)
        && chars[kw_start..kw_end].iter().collect::<String>() == "function"
    {
        return true;
    }
    let mut j = close + 1;
    skip_ws(chars, mask, &mut j);
    chars.get(j) == Some(&'=') && chars.get(j + 1) == Some(&'>')
}

/// The (start, end) range of the identifier ending at `pos` (skipping
/// whitespace/masked chars before it), if any.
fn word_ending_before(chars: &[char], mask: &[bool], pos: usize) -> Option<(usize, usize)> {
    let mut j = pos;
    while j > 0 && (chars[j - 1].is_whitespace() || mask[j - 1]) {
        j -= 1;
    }
    let end = j;
    while j > 0 && !mask[j - 1] && is_ident_char(chars[j - 1]) {
        j -= 1;
    }
    if j == end { None } else { Some((j, end)) }
}

/// Processes a call's argument list: splits on top-level commas, strips a
/// leading `name =` from each argument (a real Rhai assignment/equality
/// is never at the very start of an argument, so this can't misfire on
/// one), and recursively transforms the remainder so nested calls get the
/// same treatment.
fn process_args(chars: &[char], mask: &[bool], start: usize, end: usize) -> String {
    split_top_level(chars, mask, start, end)
        .into_iter()
        .map(|(s, e)| {
            let value_start = strip_named_prefix(chars, mask, s, e).unwrap_or(s);
            transform(chars, mask, value_start, e)
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// If `chars[start..end]` (an argument's full text) begins with
/// `IDENT (ws) = (ws)` (and that `=` isn't `==`/`=>`), returns the index
/// where the value after it starts.
fn strip_named_prefix(chars: &[char], mask: &[bool], start: usize, end: usize) -> Option<usize> {
    let mut j = start;
    while j < end && (chars[j].is_whitespace() || mask[j]) {
        j += 1;
    }
    if j >= end || mask[j] || !is_ident_start(chars[j]) {
        return None;
    }
    let ident_end = {
        let mut k = j;
        while k < end && !mask[k] && is_ident_char(chars[k]) {
            k += 1;
        }
        k
    };
    let mut k = ident_end;
    skip_ws(chars, mask, &mut k);
    if k < end && chars[k] == '=' && !matches!(chars.get(k + 1), Some('=') | Some('>')) {
        k += 1;
        skip_ws(chars, mask, &mut k);
        return Some(k);
    }
    None
}

/// Returns the (start, end) char ranges of each top-level comma-separated
/// segment in `chars[start..end]` (respecting nested brackets/strings/
/// comments). Mirrors `argtrunc::split_top_level`.
fn split_top_level(chars: &[char], mask: &[bool], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut arg_start = start;
    let mut i = start;

    while i < end {
        if !mask[i] {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    args.push((arg_start, i));
                    arg_start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    // The trailing segment (after the last top-level comma, or the whole
    // range if there were none) only counts as an argument if it has any
    // real content, or a comma already implied one ("foo()" -> no args,
    // but "foo(1,)" -> two, the second empty).
    // Unlike argtrunc's version of this check, masked content counts as
    // "real" here too - a lone string-literal argument (`text("...")`) is
    // entirely masked, but is very much a real argument, not an empty one.
    let has_content = (arg_start..end).any(|k| !chars[k].is_whitespace());
    if has_content || !args.is_empty() {
        args.push((arg_start, end));
    }

    args
}

fn matching_paren(chars: &[char], mask: &[bool], open_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < chars.len() {
        if !mask[i] {
            match chars[i] {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn skip_ws(chars: &[char], mask: &[bool], j: &mut usize) {
    while *j < chars.len() && (chars[*j].is_whitespace() || mask[*j]) {
        *j += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::strip_named_args;

    #[test]
    fn strips_single_named_arg() {
        // like argtrunc, rejoining arguments doesn't preserve original
        // inter-argument whitespace - harmless, Rhai doesn't care.
        assert_eq!(strip_named_args("noise(scale=153.413, offset=0.165)"), "noise(153.413,0.165)");
    }

    #[test]
    fn strips_named_arg_mixed_with_positional() {
        assert_eq!(strip_named_args("noise(75,speed=0.5)"), "noise(75,0.5)");
    }

    #[test]
    fn strips_at_any_nesting_depth() {
        assert_eq!(strip_named_args("a(b(x=1),y=2)"), "a(b(1),2)");
    }

    #[test]
    fn leaves_plain_positional_args_alone() {
        let src = "osc(60,0.1,0)";
        assert_eq!(strip_named_args(src), src);
    }

    #[test]
    fn leaves_equality_comparison_alone() {
        let src = "foo(a==b,c)";
        assert_eq!(strip_named_args(src), src);
    }

    #[test]
    fn leaves_arrow_alone() {
        let src = "foo(x=>x*2)";
        assert_eq!(strip_named_args(src), src);
    }

    #[test]
    fn leaves_toplevel_assignment_alone() {
        let src = "speed=0.5\nosc(60).out()";
        assert_eq!(strip_named_args(src), src);
    }

    #[test]
    fn leaves_function_declaration_defaults_alone() {
        // regression test: a `function` declaration's own default
        // parameter values look identical to the named-arg call pattern
        // (`name = value`), but must be left intact for
        // jsfunctions::rewrite_function_decls to parse correctly.
        let src = "function r(min=0,max=1) { return max-min; }";
        assert_eq!(strip_named_args(src), src);
    }

    #[test]
    fn leaves_named_arrow_declaration_defaults_alone() {
        // same idea, for the arrow-function-with-defaults form arrowfn
        // needs to see intact.
        let src = "let r = (min=0,max=1) => max-min;";
        assert_eq!(strip_named_args(src), src);
    }

    #[test]
    fn still_strips_named_args_inside_a_declarations_body() {
        // the declaration-parens guard only protects the header itself -
        // a real call inside the body is still eligible.
        assert_eq!(
            strip_named_args("function f(a) { return noise(scale=1); }"),
            "function f(a) { return noise(1); }"
        );
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"noise(scale=1)\") // noise(scale=1)";
        assert_eq!(strip_named_args(src), src);
    }
}
