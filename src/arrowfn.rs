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
//! The parameter list may be parenthesized (`(a, b)`) or, for a single
//! parameter, bare (`v => ...`) - both are real, seen shapes; the bare form
//! is otherwise indistinguishable from an argument-position callback
//! (`.fast(x=>x*2)`, left alone by `arrow.rs`) only because *this* pass
//! requires an actual assignment (`target = ...`) before it, which that
//! shape doesn't have.
//!
//! An empty parameter list (`()=>...`) is ambiguous on its own: with an
//! *expression* body it's the reactive-value idiom (already fully handled
//! upstream by `arrow::strip_zero_arg_arrows`/`patcall::rewrite_pattern_calls`,
//! and never reaches this pass as an assignment target since it's a
//! function-argument-position thing, not an assignment RHS) - but with a
//! *block* body it can only be a real named helper (`let update = () => {
//! ...; return v; }`), since the reactive idiom is expression-only by
//! construction. So an empty parameter list is accepted here too, but only
//! when paired with a block body.
//!
//! Two different assignment targets are handled, differently:
//! - **A bare identifier** (`let name = ...` / `name = ...`, no `let`
//!   required): becomes a real Rhai `fn` declaration, since it may be
//!   called elsewhere with real arguments.
//! - **A property path** (`obj.prop = ...`, however deeply dotted): the
//!   *entire statement* is dropped. Every real instance of this shape seen
//!   in the corpus is JS/p5.js/DOM-style event-handler wiring (`p.setup =
//!   () => {...}`, `img.onload = () => {...}`) on an object hydra-rust has
//!   no model of at all - Rhai has no way to declare a function "on" an
//!   arbitrary property path the way `fn` declares a global one, and
//!   nothing here would ever invoke it even if it somehow parsed. Since
//!   the alternative is an unconditional hard parse error, dropping it is
//!   strictly no worse and often unblocks the rest of the script.
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

/// If an arrow-function assignment starts at `i` (optionally preceded by a
/// `let`/`const` keyword, which `i` already points past), returns the
/// replacement text and the index just past the whole statement (its
/// terminating `;`, if any right after a block body, included - consumed
/// rather than left dangling after a declaration that doesn't need one).
/// The replacement is a real `fn name(...) {...}` declaration for a
/// bare-identifier target, or the empty string (statement dropped) for a
/// property-path target - see the module doc comment.
fn try_rewrite(chars: &[char], mask: &[bool], i: usize) -> Option<(String, usize)> {
    let mut j = i;
    let word_end = ident_end(chars, j);
    let word: String = chars[j..word_end].iter().collect();
    if word == "let" || word == "const" {
        j = word_end;
        skip_ws(chars, mask, &mut j);
    }

    let (target, mut j) = parse_dotted_target(chars, mask, j)?;
    skip_ws(chars, mask, &mut j);

    if chars.get(j) != Some(&'=') || matches!(chars.get(j + 1), Some('=') | Some('>')) {
        return None;
    }
    j += 1;
    skip_ws(chars, mask, &mut j);

    let (names, mut j) = parse_arrow_params(chars, mask, j)?;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'=') || chars.get(j + 1) != Some(&'>') {
        return None;
    }
    j += 2;
    skip_ws(chars, mask, &mut j);

    let is_block_body = chars.get(j) == Some(&'{');
    let (body, after) = if is_block_body {
        let body_close = matching_close(chars, mask, j, '{', '}')?;
        (chars[j..=body_close].iter().collect::<String>(), body_close + 1)
    } else {
        let (expr, semi_end) = scan_expr_body(chars, mask, j);
        (format!("{{ {} }}", expr.trim()), semi_end)
    };

    if target.len() > 1 {
        // Property-path target: drop the whole statement (see module doc).
        return Some((String::new(), after));
    }

    if names.is_empty() && !is_block_body {
        return None; // the reactive-value idiom, handled upstream
    }

    let rewritten = format!("fn {}({}) {body}", target[0], names.join(","));
    Some((rewritten, after))
}

/// Parses an assignment target: a bare identifier, or a dotted property
/// path (`a.b.c`) of any length. Returns the dot-separated segments and the
/// index just past the last one.
fn parse_dotted_target(chars: &[char], mask: &[bool], start: usize) -> Option<(Vec<String>, usize)> {
    if !chars.get(start).is_some_and(|c| is_ident_start(*c)) {
        return None;
    }
    let mut segs = Vec::new();
    let mut j = start;
    loop {
        let seg_start = j;
        j = ident_end(chars, j);
        segs.push(chars[seg_start..j].iter().collect());
        let mut k = j;
        skip_ws(chars, mask, &mut k);
        if chars.get(k) == Some(&'.') {
            k += 1;
            skip_ws(chars, mask, &mut k);
            if !chars.get(k).is_some_and(|c| is_ident_start(*c)) {
                return None;
            }
            j = k;
        } else {
            break;
        }
    }
    Some((segs, j))
}

/// Parses an arrow function's parameter list: parenthesized (`(a, b=1)`,
/// possibly empty) or, for a single parameter, bare (`v`). Returns the
/// parameter names (defaults are stripped, same as the parenthesized form
/// already did) and the index just past the list.
fn parse_arrow_params(chars: &[char], mask: &[bool], j: usize) -> Option<(Vec<String>, usize)> {
    if chars.get(j) == Some(&'(') {
        let close = matching_close(chars, mask, j, '(', ')')?;
        let params = parse_params(chars, mask, j + 1, close);
        Some((params.into_iter().map(|(n, _)| n).collect(), close + 1))
    } else if chars.get(j).is_some_and(|c| is_ident_start(*c)) {
        let end = ident_end(chars, j);
        Some((vec![chars[j..end].iter().collect()], end))
    } else {
        None
    }
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
    fn drops_a_property_assigned_arrow_entirely() {
        // Rhai has no way to declare a function "on" a property path the
        // way `fn` declares a global one, and every real instance of this
        // shape in the corpus is JS/p5.js event-handler wiring that
        // nothing here would ever invoke anyway - so the whole statement
        // is dropped rather than left as a guaranteed parse error.
        assert_eq!(rewrite_named_arrows("window.star = (s,v) => s+v;"), "");
    }

    #[test]
    fn drops_a_deeply_dotted_property_assigned_arrow() {
        assert_eq!(rewrite_named_arrows("p1.canvas.style.opacity = () => 1;"), "");
    }

    #[test]
    fn drops_a_block_bodied_property_assigned_arrow_keeping_a_trailing_semicolon() {
        assert_eq!(
            rewrite_named_arrows("p1.mousePressed = () => { doThing(); };"),
            ";"
        );
    }

    #[test]
    fn rewrites_zero_param_block_bodied_named_arrow() {
        // unlike the expression-bodied form (the reactive-value idiom,
        // handled upstream), a block body can only be a real helper.
        assert_eq!(
            rewrite_named_arrows("let update = () => { let v = 1; v };"),
            "fn update() { let v = 1; v };"
        );
    }

    #[test]
    fn rewrites_bare_single_param_named_arrow_without_parens() {
        assert_eq!(
            rewrite_named_arrows("let react = v => (a.fft[0]*v);"),
            "fn react(v) { (a.fft[0]*v) }"
        );
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
