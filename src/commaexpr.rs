//! Rewrites a parenthesized, non-call, comma-containing group (`(a, b,
//! c)`) down to just its last term, `(c)` - JS's own comma-operator
//! semantics: every sub-expression is evaluated in order, but only the
//! *last* one's value survives (the rest are pure discards). Rhai has no
//! comma operator at all; a grouping paren containing a bare top-level
//! comma is a hard parse error.
//!
//! Real sketches very commonly write this (`shape(4, (0.01, 0.2 +
//! a.fft[2]), 1)`, `osc(0.5, (o0, mouse.x * 0.000003), 0.6)`) where they
//! plausibly *meant* an array `[a, b]` (hydra-rust's own pattern arrays
//! elsewhere in this pipeline suggest exactly that intent - alternating
//! between two values). But real hydra.js/JS would actually run this
//! exact code via the comma operator, discarding the first value - this
//! pass matches that real, faithful behavior rather than guessing at what
//! the author probably meant.
//!
//! A function call's own argument-list parens (`foo(a, b)`) are correctly
//! left alone - those commas separate real, distinct arguments, not a
//! sequence expression - and so is an arrow function's own parameter list
//! (`(a, b) => ...`), even in places nothing else in this pipeline
//! rewrites it (e.g. an argument-position multi-param arrow like
//! `.fast((val,i)=>val*2)`, deliberately left alone by `arrow.rs`).
//! Distinguished the same way `objlit.rs` tells a block from a value: a
//! `(` immediately preceded (skipping whitespace) by an identifier
//! character or `)`/`]` is a call, and a `)` immediately followed
//! (skipping whitespace) by `=>` is a parameter list - neither is a bare
//! grouping.
//!
//! Runs last, after every other pass (including `arrowfn`): by then, every
//! *assignment-target* arrow's parameter list has already been consumed
//! one way or another, so the only remaining `(a, b) => ...` shapes left
//! to *not* misfire on are argument-position ones - already excluded by
//! the "followed by `=>`" check above regardless of ordering, but running
//! last keeps this pass working on the final, fully-settled shape of the
//! code rather than something still being rewritten around it.

use crate::srcscan::mask_strings_and_comments;

pub fn rewrite_comma_expressions(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    transform(&chars, &mask, 0, chars.len())
}

fn transform(chars: &[char], mask: &[bool], start: usize, end: usize) -> String {
    let mut out = String::new();
    let mut i = start;
    while i < end {
        if !mask[i]
            && chars[i] == '('
            && !preceded_by_call_token(chars, mask, i)
            && let Some(close) = matching_close(chars, mask, i, end)
            && !followed_by_arrow(chars, mask, close)
            && let Some(last_comma) = last_top_level_comma(chars, mask, i + 1, close)
        {
            let kept = transform(chars, mask, last_comma + 1, close);
            out.push('(');
            out.push_str(kept.trim_start());
            out.push(')');
            i = close + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// True if the token immediately before `i` (skipping whitespace and
/// masked regions) is an identifier character or `)`/`]` - meaning this
/// `(` opens a call's argument list, not a bare grouping.
fn preceded_by_call_token(chars: &[char], mask: &[bool], i: usize) -> bool {
    let mut j = i;
    while j > 0 {
        j -= 1;
        if mask[j] || chars[j].is_whitespace() {
            continue;
        }
        return chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == ')' || chars[j] == ']';
    }
    false
}

/// True if the token immediately after `close` (a `)`, skipping
/// whitespace and masked regions) is `=>` - meaning the parens just
/// closed are an arrow function's parameter list, not a grouping.
fn followed_by_arrow(chars: &[char], mask: &[bool], close: usize) -> bool {
    let mut j = close + 1;
    while j < chars.len() && (mask[j] || chars[j].is_whitespace()) {
        j += 1;
    }
    chars.get(j) == Some(&'=') && chars.get(j + 1) == Some(&'>')
}

/// Index of the last top-level (depth-0 relative to `start`) comma in
/// `start..end`, if any.
fn last_top_level_comma(chars: &[char], mask: &[bool], start: usize, end: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut last = None;
    let mut i = start;
    while i < end {
        if !mask[i] {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => last = Some(i),
                _ => {}
            }
        }
        i += 1;
    }
    last
}

fn matching_close(chars: &[char], mask: &[bool], open_idx: usize, limit: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < limit {
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

#[cfg(test)]
mod tests {
    use super::rewrite_comma_expressions;

    #[test]
    fn collapses_a_simple_comma_tuple_to_its_last_value() {
        assert_eq!(rewrite_comma_expressions("(0.01, 0.2)"), "(0.2)");
    }

    #[test]
    fn collapses_a_comma_tuple_used_as_a_call_argument() {
        assert_eq!(
            rewrite_comma_expressions("shape(4, (0.01, 0.2 + a.fft[2]), 1)"),
            "shape(4, (0.2 + a.fft[2]), 1)"
        );
    }

    #[test]
    fn collapses_a_three_element_tuple() {
        assert_eq!(rewrite_comma_expressions("(1,2,3,24)"), "(24)");
    }

    #[test]
    fn recursively_collapses_a_nested_tuple_in_the_kept_tail() {
        assert_eq!(rewrite_comma_expressions("(1, (2,3))"), "((3))");
    }

    #[test]
    fn leaves_a_real_call_argument_list_alone() {
        let src = "shape(4, 0.3, 0.01)";
        assert_eq!(rewrite_comma_expressions(src), src);
    }

    #[test]
    fn leaves_a_chained_calls_argument_list_alone() {
        let src = "osc(60).color(1, 2, 3)";
        assert_eq!(rewrite_comma_expressions(src), src);
    }

    #[test]
    fn leaves_an_arrow_functions_parameter_list_alone() {
        let src = "let f = (a,b) => a+b;";
        assert_eq!(rewrite_comma_expressions(src), src);
    }

    #[test]
    fn leaves_an_argument_position_multi_param_arrow_alone() {
        let src = ".fast((val,i)=>val*2)";
        assert_eq!(rewrite_comma_expressions(src), src);
    }

    #[test]
    fn leaves_a_plain_grouping_paren_alone() {
        let src = "(0.1 + 0.2) * 3";
        assert_eq!(rewrite_comma_expressions(src), src);
    }

    #[test]
    fn leaves_normal_code_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(rewrite_comma_expressions(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"(1,2)\") // (1,2)";
        assert_eq!(rewrite_comma_expressions(src), src);
    }
}
