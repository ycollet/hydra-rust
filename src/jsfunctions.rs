//! Rewrites JS *named* function declarations (`function name(a, b=1) {
//! ... }`) into Rhai's `fn` syntax (`fn name(a, b) { ... }`). Real hydra.js
//! sketches sometimes define small helper functions this way (easing
//! curves, custom math). Rhai has its own, similarly-shaped
//! function-definition syntax - spelled `fn` instead of `function`, and
//! without default parameter values - so this is close to a 1:1 textual
//! translation for the named-declaration form: only the keyword and the
//! parameter list (defaults stripped) need to change; the body is left
//! untouched.
//!
//! Deliberately left alone: anonymous `function(...) { ... }` expressions
//! (used as JS closures/callbacks, which can capture outer-scope
//! variables - Rhai's `fn`-defined functions can't, so this wouldn't be a
//! safe like-for-like translation).

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
/// `function` keyword at `after_kw`, writes the rewritten `fn name(params)`
/// header to `out` and returns the index just past the header's `(...)`
/// (the caller resumes normal copying from there, including the body's
/// `{...}`, which needs no changes).
fn rewrite_one(chars: &[char], mask: &[bool], after_kw: usize, out: &mut String) -> Option<usize> {
    let mut j = after_kw;
    skip_ws(chars, mask, &mut j);
    if !chars.get(j).is_some_and(|c| is_ident_start(*c)) {
        return None; // anonymous function expression - leave alone
    }
    let name_start = j;
    let name_end = ident_end(chars, name_start);
    j = name_end;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'(') {
        return None;
    }
    let open = j;
    let close = matching_close(chars, mask, open)?;

    out.push_str("fn ");
    out.extend(&chars[name_start..name_end]);
    out.push('(');
    out.push_str(&strip_param_defaults(chars, mask, open + 1, close));
    out.push(')');

    Some(close + 1)
}

/// Strips `=default` from each top-level comma-separated parameter,
/// keeping just its name.
fn strip_param_defaults(chars: &[char], mask: &[bool], start: usize, end: usize) -> String {
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
            let piece_end = eq_pos.unwrap_or(i);
            let name: String = chars[piece_start..piece_end].iter().collect();
            let name = name.trim();
            if !name.is_empty() {
                pieces.push(name.to_string());
            }
            piece_start = i + 1;
            eq_pos = None;
        }
        i += 1;
    }

    pieces.join(",")
}

fn matching_close(chars: &[char], mask: &[bool], open_idx: usize) -> Option<usize> {
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
    fn strips_default_values() {
        assert_eq!(
            rewrite_function_decls("function r(min=0,max=1) { return max-min; }"),
            "fn r(min,max) { return max-min; }"
        );
    }

    #[test]
    fn handles_no_params() {
        assert_eq!(rewrite_function_decls("function foo() { 1 }"), "fn foo() { 1 }");
    }

    #[test]
    fn handles_single_param_with_default() {
        assert_eq!(rewrite_function_decls("function f(x=5) { x }"), "fn f(x) { x }");
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
