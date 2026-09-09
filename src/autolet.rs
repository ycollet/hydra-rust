//! Inserts `let ` before the first bare assignment to any name that isn't
//! already declared. Real hydra.js sketches routinely rely on JS's implicit
//! global-variable creation (`speed = 0.8`, `pat = ()=>osc(30)`); Rhai
//! requires an explicit `let` for a variable's first assignment.

use crate::srcscan::mask_strings_and_comments;
use std::collections::HashSet;

/// Names already in scope before the script runs (see the `Scope` setup in
/// `eval::eval`) - assignments to these are reassignments, not declarations.
const BUILTIN_NAMES: &[&str] = &[
    "time", "beat", "tempo", "phase", "mouseX", "mouseY", "mouse", "a", "o0", "o1", "o2", "o3",
    "s0", "s1", "s2", "s3",
];

pub fn insert_missing_let(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len() + 16);
    let mut known: HashSet<String> = BUILTIN_NAMES.iter().map(|s| s.to_string()).collect();
    // Only `(` / `[` depth: `let` is a valid statement inside a `{ ... }`
    // block (e.g. a block-bodied arrow), but never inside a plain
    // parenthesized argument list or array literal - JS allows assignment
    // expressions there (`foo(x = 5)`), Rhai's `let` isn't an expression.
    let mut depth: i32 = 0;

    let mut i = 0;
    while i < n {
        if !mask[i] {
            match chars[i] {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                _ => {}
            }
        }
        if !mask[i]
            && is_ident_start(chars[i])
            && !preceded_by_ident_or_dot(&chars, i)
        {
            let end = ident_end(&chars, i);
            let ident: String = chars[i..end].iter().collect();

            let mut p = end;
            skip_ws(&chars, &mask, &mut p);
            if is_plain_assignment(&chars, &mask, p) {
                if depth == 0 && !known.contains(&ident) && !preceded_by_decl_keyword(&chars, i) {
                    out.push_str("let ");
                    known.insert(ident.clone());
                }
                out.push_str(&ident);
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

fn is_plain_assignment(chars: &[char], mask: &[bool], p: usize) -> bool {
    matches!(mask.get(p), Some(false))
        && chars.get(p) == Some(&'=')
        && !matches!(chars.get(p + 1), Some('='))
}

/// True if the word immediately before `ident_start` (skipping whitespace)
/// is `let` or `const`, meaning this is already a proper declaration.
fn preceded_by_decl_keyword(chars: &[char], ident_start: usize) -> bool {
    let mut p = ident_start;
    while p > 0 && chars[p - 1].is_whitespace() {
        p -= 1;
    }
    for kw in ["let", "const"] {
        let kw_chars: Vec<char> = kw.chars().collect();
        let len = kw_chars.len();
        if p >= len && chars[p - len..p] == kw_chars[..] && (p == len || !is_ident_char(chars[p - len - 1])) {
            return true;
        }
    }
    false
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
    use super::insert_missing_let;

    #[test]
    fn inserts_let_on_first_assignment() {
        assert_eq!(insert_missing_let("speed = 0.8\nosc(60).out()"), "let speed = 0.8\nosc(60).out()");
    }

    #[test]
    fn does_not_double_declare_on_reassignment() {
        let src = "speed = 0.8\nspeed = 1.2";
        assert_eq!(insert_missing_let(src), "let speed = 0.8\nspeed = 1.2");
    }

    #[test]
    fn leaves_explicit_let_alone() {
        let src = "let speed = 0.8";
        assert_eq!(insert_missing_let(src), src);
    }

    #[test]
    fn leaves_builtin_reassignment_alone() {
        // unusual, but shouldn't be double-`let`-ed
        assert_eq!(insert_missing_let("mouseX = 1"), "mouseX = 1");
    }

    #[test]
    fn leaves_comparisons_alone() {
        assert_eq!(insert_missing_let("n == 5"), "n == 5");
    }

    #[test]
    fn does_not_insert_let_inside_call_args() {
        // `let` is a statement, not a valid expression inside a function
        // call's argument list or an array literal - JS allows assignment
        // expressions there; inserting `let` would be a syntax error.
        let src = "shift(0,0,0.4,sin=1)";
        assert_eq!(insert_missing_let(src), src);
    }

    #[test]
    fn does_not_insert_let_inside_array_literal() {
        let src = "[n=1,2,3]";
        assert_eq!(insert_missing_let(src), src);
    }

    #[test]
    fn leaves_property_assignment_alone() {
        assert_eq!(insert_missing_let("foo.x = 5"), "foo.x = 5");
    }

    #[test]
    fn handles_arrow_assigned_pattern() {
        assert_eq!(
            insert_missing_let("pat = ()=>osc(30)\nout(pat)"),
            "let pat = ()=>osc(30)\nout(pat)"
        );
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"speed = 1\") // speed = 1";
        assert_eq!(insert_missing_let(src), src);
    }
}
