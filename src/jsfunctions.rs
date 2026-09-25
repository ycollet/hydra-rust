//! Rewrites JS *named* function declarations (`function name(a, b=1) {
//! ... }`) into Rhai's `fn` syntax (`fn name(a, b) { ... }`). Real hydra.js
//! sketches sometimes define small helper functions this way (easing
//! curves, custom math). Rhai has its own, similarly-shaped
//! function-definition syntax - spelled `fn` instead of `function` - so
//! this is close to a 1:1 textual translation for the named-declaration
//! form: only the keyword needs to change; the body is left untouched.
//!
//! Rhai has no default parameter values at all, so a JS default like
//! `max=1` can't be kept as-is. But real call sites routinely rely on the
//! JS default actually applying (`r()` where `r` is declared
//! `function r(min=0,max=1)`), and Rhai resolves overloads by arity - so
//! when every default is *trailing* (once one parameter has a default,
//! every parameter after it does too), each shorter arity is synthesized
//! as its own `fn` that just forwards to the next arity up with that
//! default value spliced in, cascading down to the full-arity original:
//! `fn r(min,max) {BODY}`, `fn r(min) {r(min,1)}`, `fn r() {r(0)}`. If the
//! defaults aren't all trailing (rare, unidiomatic JS), no shims are
//! generated - only the header is stripped, same as before.
//!
//! Deliberately left alone: anonymous `function(...) { ... }` expressions
//! (used as JS closures/callbacks, which can capture outer-scope
//! variables - Rhai's `fn`-defined functions can't, so this wouldn't be a
//! safe like-for-like translation).
//!
//! Runs `asi::insert_missing_semicolons` on the body's own inner content
//! before copying it, even though the file-wide `asi` pass (step 18)
//! hasn't run yet at this point (step 6, before `autolet`): the file-wide
//! pass never inserts a `;` while bracket depth is above 0 anyway - true
//! for everything inside this function's own `{`/`}` regardless of when
//! it runs - so a multi-statement body with no explicit `;` between its
//! own lines (`function f(e) {\n  x = e.a\n  y = e.b\n}`, common real JS
//! style) would otherwise reach Rhai's parser exactly as broken as if
//! this pass hadn't run `asi` locally at all. Same idea `forloop.rs`
//! already uses for its own loop bodies, and `arrowfn.rs` for its
//! function-*value* equivalent of this same declaration form.

use crate::asi;
use crate::srcscan::mask_strings_and_comments;

pub fn rewrite_function_decls(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());

    let mut i = 0;
    while i < n {
        if !mask[i] && is_ident_start(chars[i]) && !preceded_by_ident_or_dot(&chars, i) {
            let end = ident_end(&chars, i);
            if chars[i..end].iter().collect::<String>() == "function"
                && let Some(after) = rewrite_one(&chars, &mask, end, &mut out)
            {
                i = after;
                continue;
            }
            out.extend(&chars[i..end]);
            i = end;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

/// If a named function declaration's header starts right after the
/// `function` keyword at `after_kw`, writes the rewritten `fn name(params)
/// {BODY}` declaration (plus, if every default parameter is trailing, a
/// cascade of arity-shim overloads - see the module doc comment) to `out`
/// and returns the index just past the whole declaration, body included
/// (the caller resumes normal copying from there).
fn rewrite_one(chars: &[char], mask: &[bool], after_kw: usize, out: &mut String) -> Option<usize> {
    let mut j = after_kw;
    skip_ws(chars, mask, &mut j);
    if !chars.get(j).is_some_and(|c| is_ident_start(*c)) {
        return None; // anonymous function expression - leave alone
    }
    let name_start = j;
    let name_end = ident_end(chars, name_start);
    let name: String = chars[name_start..name_end].iter().collect();
    j = name_end;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'(') {
        return None;
    }
    let open = j;
    let close = matching_close(chars, mask, open, '(', ')')?;

    j = close + 1;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'{') {
        return None; // not block-bodied - nothing for us to unwrap safely
    }
    let body_open = j;
    let body_close = matching_close(chars, mask, body_open, '{', '}')?;

    let params = parse_params(chars, mask, open + 1, close);
    let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();

    out.push_str("fn ");
    out.push_str(&name);
    out.push('(');
    out.push_str(&names.join(","));
    out.push_str(") {");
    out.push_str(&asi::insert_missing_semicolons(
        &chars[body_open + 1..body_close].iter().collect::<String>(),
    ));
    out.push('}');

    // Trailing-default shim cascade: only when every defaulted parameter
    // is followed solely by other defaulted parameters (`a,b=1,c=2`, not
    // `a=1,b`) - otherwise a shorter arity wouldn't unambiguously fill in
    // the missing values, so no shims are emitted (the header above is
    // still stripped-but-correct for the full-arity call form).
    let first_default = params.iter().position(|(_, d)| d.is_some());
    if let Some(first_default) = first_default
        && params[first_default..].iter().all(|(_, d)| d.is_some())
    {
        for arity in first_default..params.len() {
            let short_names = &names[..arity];
            let mut forward_args: Vec<String> = short_names.iter().map(|s| s.to_string()).collect();
            forward_args.push(params[arity].1.clone().unwrap());
            out.push_str("\nfn ");
            out.push_str(&name);
            out.push('(');
            out.push_str(&short_names.join(","));
            out.push_str(") { ");
            out.push_str(&name);
            out.push('(');
            out.push_str(&forward_args.join(","));
            out.push_str(") }");
        }
    }

    Some(body_close + 1)
}

/// Parses a top-level comma-separated parameter list into `(name,
/// default_text)` pairs, where `default_text` is the (trimmed) source text
/// after a parameter's `=`, if it has one. `pub(crate)`: also used by
/// `arrowfn` for the equivalent named-arrow-function declaration form.
pub(crate) fn parse_params(chars: &[char], mask: &[bool], start: usize, end: usize) -> Vec<(String, Option<String>)> {
    let mut pieces = Vec::new();
    let mut depth = 0i32;
    let mut piece_start = start;
    let mut eq_pos: Option<usize> = None;

    let mut i = start;
    while i <= end {
        let at_boundary = i == end;
        let c = if at_boundary { ',' } else { chars[i] };

        if !at_boundary && !mask[i] {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                '=' if depth == 0 && eq_pos.is_none() && chars.get(i + 1) != Some(&'=') => {
                    eq_pos = Some(i);
                }
                _ => {}
            }
        }

        if c == ',' && depth == 0 {
            let name_end = eq_pos.unwrap_or(i);
            let name: String = chars[piece_start..name_end].iter().collect();
            let name = name.trim().to_string();
            if !name.is_empty() {
                let default = eq_pos.map(|p| chars[p + 1..i].iter().collect::<String>().trim().to_string());
                pieces.push((name, default));
            }
            piece_start = i + 1;
            eq_pos = None;
        }
        i += 1;
    }

    pieces
}

pub(crate) fn matching_close(chars: &[char], mask: &[bool], open_idx: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < chars.len() {
        if !mask[i] {
            if chars[i] == open {
                depth += 1;
            } else if chars[i] == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
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
    use super::rewrite_function_decls;

    #[test]
    fn rewrites_simple_declaration() {
        assert_eq!(
            rewrite_function_decls("function foo(a,b) { return a+b; }"),
            "fn foo(a,b) { return a+b; }"
        );
    }

    #[test]
    fn strips_default_values_and_adds_shim_cascade() {
        // both parameters are trailing-defaulted, so calls with 0 or 1
        // arguments (relying on the JS defaults) need their own overloads,
        // since Rhai has no default parameter values of its own.
        assert_eq!(
            rewrite_function_decls("function r(min=0,max=1) { return max-min; }"),
            "fn r(min,max) { return max-min; }\nfn r() { r(0) }\nfn r(min) { r(min,1) }"
        );
    }

    #[test]
    fn inserts_missing_semicolons_between_the_bodys_own_bare_statements() {
        // real JS style routinely puts each statement on its own line
        // with no `;` - the file-wide `asi` pass (step 18) never reaches
        // inside this function's own `{ }` (bracket depth > 0 there
        // regardless of when it runs), so this pass runs it locally first.
        assert_eq!(
            rewrite_function_decls("function f(e) {\n  x = e.a\n  y = e.b\n}"),
            "fn f(e) {\n  x = e.a;\n  y = e.b\n}"
        );
    }

    #[test]
    fn handles_no_params() {
        assert_eq!(rewrite_function_decls("function foo() { 1 }"), "fn foo() { 1 }");
    }

    #[test]
    fn handles_single_param_with_default() {
        assert_eq!(
            rewrite_function_decls("function f(x=5) { x }"),
            "fn f(x) { x }\nfn f() { f(5) }"
        );
    }

    #[test]
    fn does_not_shim_non_trailing_defaults() {
        // `a=1` is followed by a non-defaulted `b` - a shorter arity
        // couldn't unambiguously fill in the missing values, so this stays
        // a single, non-overloaded declaration (same as before defaults
        // were shimmed at all).
        assert_eq!(
            rewrite_function_decls("function f(a=1,b) { a+b }"),
            "fn f(a,b) { a+b }"
        );
    }

    #[test]
    fn shim_cascades_through_multiple_defaults() {
        assert_eq!(
            rewrite_function_decls("function f(a,b=1,c=2) { a+b+c }"),
            "fn f(a,b,c) { a+b+c }\nfn f(a) { f(a,1) }\nfn f(a,b) { f(a,b,2) }"
        );
    }

    #[test]
    fn leaves_anonymous_function_alone() {
        let src = "var f = function(x) { return x; }";
        assert_eq!(rewrite_function_decls(src), src);
    }

    #[test]
    fn does_not_match_partial_identifier() {
        let src = "myfunction(1,2)";
        assert_eq!(rewrite_function_decls(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"function foo(a) {}\") // function foo(a) {}";
        assert_eq!(rewrite_function_decls(src), src);
    }
}
