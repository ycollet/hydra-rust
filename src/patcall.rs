//! Rewrites `name()` back to a bare `name` when `name` was defined as a
//! zero-arg arrow (`name = ()=>expr`). Real hydra.js sketches often store a
//! reusable "pattern" in a variable and invoke it later as if it were a
//! function (`pat()`); hydra-rust's arrow-stripping (see `arrow.rs`)
//! already reduces the definition to a plain value (`pat = expr`), so a
//! later `pat()` call needs to become a bare `pat` reference to that same
//! value, not an actual function call (nothing is registered under that
//! name). Must run before `arrow::strip_zero_arg_arrows`, since it needs
//! to see the `()=>` marker to know which names are pattern-bound.

use crate::srcscan::mask_strings_and_comments;
use std::collections::HashSet;

pub fn rewrite_pattern_calls(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let names = collect_pattern_bound_names(&chars, &mask);
    if names.is_empty() {
        return src.to_string();
    }
    rewrite_zero_arg_calls(&chars, &mask, &names)
}

/// Finds every `name = ()=>...` assignment and returns the set of names.
fn collect_pattern_bound_names(chars: &[char], mask: &[bool]) -> HashSet<String> {
    let n = chars.len();
    let mut names = HashSet::new();
    let mut i = 0;
    while i < n {
        if !mask[i] && is_ident_start(chars[i]) && !preceded_by_ident_or_dot(chars, i) {
            let end = ident_end(chars, i);
            let mut p = end;
            skip_ws(chars, mask, &mut p);
            if chars.get(p) == Some(&'=') && chars.get(p + 1) != Some(&'=') {
                let mut q = p + 1;
                skip_ws(chars, mask, &mut q);
                if is_zero_arg_arrow_head(chars, mask, q) {
                    names.insert(chars[i..end].iter().collect());
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    names
}

fn is_zero_arg_arrow_head(chars: &[char], mask: &[bool], mut j: usize) -> bool {
    if chars.get(j) != Some(&'(') {
        return false;
    }
    j += 1;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&')') {
        return false;
    }
    j += 1;
    skip_ws(chars, mask, &mut j);
    chars.get(j) == Some(&'=') && chars.get(j + 1) == Some(&'>')
}

/// Replaces every zero-argument call `name()` where `name` is pattern-bound
/// with a bare `name` (dropping the parens).
fn rewrite_zero_arg_calls(chars: &[char], mask: &[bool], names: &HashSet<String>) -> String {
    let n = chars.len();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < n {
        if !mask[i] && is_ident_start(chars[i]) && !preceded_by_ident_or_dot(chars, i) {
            let end = ident_end(chars, i);
            let ident: String = chars[i..end].iter().collect();
            if names.contains(&ident) {
                let mut p = end;
                skip_ws(chars, mask, &mut p);
                if chars.get(p) == Some(&'(') {
                    let mut q = p + 1;
                    skip_ws(chars, mask, &mut q);
                    if chars.get(q) == Some(&')') {
                        out.push_str(&ident);
                        i = q + 1;
                        continue;
                    }
                }
            }
            out.push_str(&ident);
            i = end;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
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
    use super::rewrite_pattern_calls;

    #[test]
    fn rewrites_pattern_call_to_bare_reference() {
        assert_eq!(
            rewrite_pattern_calls("pat = ()=>osc(30)\nout(pat())"),
            "pat = ()=>osc(30)\nout(pat)"
        );
    }

    #[test]
    fn leaves_non_pattern_calls_alone() {
        let src = "osc(30).out()";
        assert_eq!(rewrite_pattern_calls(src), src);
    }

    #[test]
    fn leaves_calls_with_args_alone() {
        // not the zero-arg "invoke stored pattern" idiom - a genuine call
        // with arguments to something coincidentally sharing the name.
        let src = "pat = ()=>osc(30)\npat(5)";
        assert_eq!(rewrite_pattern_calls(src), src);
    }

    #[test]
    fn does_not_touch_multi_param_arrows() {
        // only the zero-arg arrow idiom is recognized
        let src = "pat = (x)=>osc(x)\npat(5)";
        assert_eq!(rewrite_pattern_calls(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "pat = ()=>osc(30) // pat()";
        assert_eq!(rewrite_pattern_calls(src), src);
    }
}
