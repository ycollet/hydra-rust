//! Rewrites JS named-arrow-function assignments (`let name = (a, b) =>
//! EXPR` or `name = (a, b) => { BLOCK }`) into genuine Rhai function
//! declarations (`fn name(a, b) { EXPR }` / `fn name(a, b) { BLOCK }`).
//! Real sketches commonly define small reusable helpers this way - the
//! arrow-function equivalent of `function name(a, b) { ... }` (already
//! handled by `jsfunctions::rewrite_function_decls`), just for authors who
//! prefer the terser syntax. Rhai has no `=>` closure syntax at all, and a
//! *value*-only substitution (as `arrow::strip_zero_arg_arrows` does for
//! the reactive-value idiom) doesn't work here: these are called
//! elsewhere with real arguments, so they need to become real callable
//! functions, not an inlined expression.
//!
//! Deliberately narrow: only the *named* form (assigning to a bare
//! identifier, not `obj.prop = ...`) with a *non-empty* parameter list -
//! an empty one (`()=>...`) is the reactive-value idiom, already fully
//! handled upstream by `arrow::strip_zero_arg_arrows`/
//! `patcall::rewrite_pattern_calls`, and never reaches this pass since its
//! `=>` is already gone by the time this runs.
//!
//! Runs last, after `asi`, for two reasons: every other pass has already
//! rewritten the arrow body's own content (ternaries, `Math.*`, etc.),
//! and - more importantly - the body's end becomes unambiguous: for an
//! expression body (no `{ }`), this just scans for the next top-level `;`,
//! which by this point in the pipeline is always there (either written by
//! the author or inserted by `asi`).

use crate::jsfunctions::{matching_close, parse_params};
use crate::srcscan::mask_strings_and_comments;

pub fn rewrite_named_arrows(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());

    let mut i = 0;
    while i < n {
        if !mask[i]
            && is_ident_start(chars[i])
            && !preceded_by_ident_or_dot(&chars, i)
            && let Some((rewritten, after)) = try_rewrite(&chars, &mask, i)
        {
            out.push_str(&rewritten);
            i = after;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

/// If a named-arrow-function assignment starts at `i` (optionally preceded
/// by a `let`/`const` keyword, which `i` already points past), returns the
/// full `fn name(...) {...}` replacement text and the index just past the
/// whole statement (its terminating `;` included, consumed rather than
/// left dangling after a declaration that doesn't need one).
fn try_rewrite(chars: &[char], mask: &[bool], i: usize) -> Option<(String, usize)> {
    let mut j = i;
    let word_end = ident_end(chars, j);
    let word: String = chars[j..word_end].iter().collect();
    if word == "let" || word == "const" {
        j = word_end;
        skip_ws(chars, mask, &mut j);
    }

    if !chars.get(j).is_some_and(|c| is_ident_start(*c)) {
        return None;
    }
    let name_start = j;
    let name_end = ident_end(chars, name_start);
    let name: String = chars[name_start..name_end].iter().collect();
    j = name_end;
    skip_ws(chars, mask, &mut j);

    if chars.get(j) != Some(&'=') || matches!(chars.get(j + 1), Some('=') | Some('>')) {
        return None;
    }
    j += 1;
    skip_ws(chars, mask, &mut j);

    if chars.get(j) != Some(&'(') {
        return None;
    }
    let open = j;
    let close = matching_close(chars, mask, open, '(', ')')?;
    let params = parse_params(chars, mask, open + 1, close);
    if params.is_empty() {
        return None; // the reactive-value idiom, handled upstream
    }
    let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();

    j = close + 1;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'=') || chars.get(j + 1) != Some(&'>') {
        return None;
    }
    j += 2;
    skip_ws(chars, mask, &mut j);

    let (body, after) = if chars.get(j) == Some(&'{') {
        let body_close = matching_close(chars, mask, j, '{', '}')?;
        (chars[j..=body_close].iter().collect::<String>(), body_close + 1)
    } else {
        let (expr, semi_end) = scan_expr_body(chars, mask, j);
        (format!("{{ {} }}", expr.trim()), semi_end)
    };

    let rewritten = format!("fn {name}({}) {body}", names.join(","));
    Some((rewritten, after))
}

/// Scans an expression-bodied arrow's value from `start` up to (and past)
/// the next top-level `;`, or to the end of input if there isn't one.
/// Returns the expression text (terminator excluded) and the index just
/// past the terminator.
fn scan_expr_body(chars: &[char], mask: &[bool], start: usize) -> (String, usize) {
    let mut depth = 0i32;
    let mut i = start;
    while i < chars.len() {
        if !mask[i] {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ';' if depth == 0 => {
                    return (chars[start..i].iter().collect(), i + 1);
                }
                _ => {}
            }
        }
        i += 1;
    }
    (chars[start..].iter().collect(), chars.len())
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn preceded_by_ident_or_dot(chars: &[char], i: usize) -> bool {
    i > 0 && (is_ident_char(chars[i - 1]) || chars[i - 1] == '.')
}

fn ident_end(chars: &[char], start: usize) -> usize {
    let mut j = start;
    while j < chars.len() && is_ident_char(chars[j]) {
        j += 1;
    }
    j
}

fn skip_ws(chars: &[char], mask: &[bool], j: &mut usize) {
    while *j < chars.len() && (chars[*j].is_whitespace() || mask[*j]) {
        *j += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite_named_arrows;

    #[test]
    fn rewrites_expression_bodied_named_arrow() {
        assert_eq!(
            rewrite_named_arrows("let gradation = (v,w) => solid(v,w);"),
            "fn gradation(v,w) { solid(v,w) }"
        );
    }

    #[test]
    fn rewrites_block_bodied_named_arrow() {
        // unlike the expression-bodied form, a block body's own `}` is an
        // unambiguous terminator - a trailing `;` after it (as JS allows
        // for any statement) is left alone rather than consumed, since
        // it's just a harmless empty statement in Rhai too.
        assert_eq!(
            rewrite_named_arrows("let f = (a,b) => { let c = a+b; c };"),
            "fn f(a,b) { let c = a+b; c };"
        );
    }

    #[test]
    fn rewrites_bare_assignment_without_let() {
        assert_eq!(
            rewrite_named_arrows("el=(s,b,l)=>shape(99,s,b);"),
            "fn el(s,b,l) { shape(99,s,b) }"
        );
    }

    #[test]
    fn rewrites_single_param_named_arrow() {
        assert_eq!(rewrite_named_arrows("let sq = (x) => x*x;"), "fn sq(x) { x*x }");
    }

    #[test]
    fn strips_default_parameter_values() {
        assert_eq!(
            rewrite_named_arrows("let f = (a,b=1) => a+b;"),
            "fn f(a,b) { a+b }"
        );
    }

    #[test]
    fn falls_back_to_end_of_input_without_trailing_semicolon() {
        assert_eq!(rewrite_named_arrows("let f = (a,b) => a+b"), "fn f(a,b) { a+b }");
    }

    #[test]
    fn leaves_zero_param_arrow_alone() {
        // the reactive-value idiom - handled upstream, never touched here
        let src = "let pat = ()=>osc(30);";
        assert_eq!(rewrite_named_arrows(src), src);
    }

    #[test]
    fn leaves_property_assignment_alone() {
        let src = "window.star = (s,v) => s+v;";
        assert_eq!(rewrite_named_arrows(src), src);
    }

    #[test]
    fn leaves_normal_code_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(rewrite_named_arrows(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"f = (a,b) => a\") // f = (a,b) => a";
        assert_eq!(rewrite_named_arrows(src), src);
    }
}
