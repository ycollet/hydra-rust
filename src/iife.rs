//! Unwraps the zero-parameter "immediately invoked function expression"
//! (IIFE) wrapper real hydra.js sketches sometimes use to wrap their entire
//! body - typically to `await` an extension-library load before running
//! the real chain code: `(() => { BODY })()`, optionally followed by
//! promise `.then(...)`/`.catch(...)`/`.finally(...)` handlers.
//! (`async`/`await` are stripped earlier in the pipeline by `jskeywords`,
//! so by the time this runs the wrapper always looks like a plain
//! non-async arrow.) Rhai has no `=>` closure syntax at all, so this
//! construct is a hard parse failure otherwise - reduces the whole thing
//! to a bare Rhai block `{ BODY }`, which Rhai evaluates as a normal
//! sequence of statements, dropping the trailing `.then`/`.catch`/
//! `.finally` handlers entirely (there's nothing async to react to here).
//!
//! Deliberately narrow: only matches a wrapper with an *empty* parameter
//! list (`()=>`). A non-empty one (`(hydra) => { ... }`) is left alone,
//! since dropping the wrapper would leave `hydra` unbound inside the body.

use crate::srcscan::mask_strings_and_comments;

pub fn unwrap_iife(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());

    let mut i = 0;
    while i < n {
        if !mask[i]
            && let Some((body_start, body_end, after)) = match_iife(&chars, &mask, i)
        {
            out.push('{');
            out.extend(&chars[body_start..body_end]);
            out.push('}');
            i = after;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

fn skip_ws(chars: &[char], mask: &[bool], j: &mut usize) {
    while *j < chars.len() && (chars[*j].is_whitespace() || mask[*j]) {
        *j += 1;
    }
}

fn matching_close(chars: &[char], mask: &[bool], open_idx: usize, open: char, close: char) -> Option<usize> {
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

/// If a zero-param IIFE (`(()=>{BODY})(...)`, optionally chained with
/// `.then(...)`/`.catch(...)`/`.finally(...)`) starts at `i`, returns
/// `(body_start, body_end, after)`: the body's char range, and the index
/// just past the whole construct (including any promise-handler chain).
fn match_iife(chars: &[char], mask: &[bool], i: usize) -> Option<(usize, usize, usize)> {
    if chars.get(i) != Some(&'(') {
        return None;
    }
    let mut j = i + 1;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'(') {
        return None;
    }
    j += 1;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&')') {
        return None; // only the empty-param form is handled
    }
    j += 1;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'=') || chars.get(j + 1) != Some(&'>') {
        return None;
    }
    j += 2;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'{') {
        return None; // not a block-bodied arrow: nothing for us to unwrap
    }
    let body_open = j;
    let body_close = matching_close(chars, mask, body_open, '{', '}')?;
    let body_start = body_open + 1;
    let body_end = body_close;

    j = body_close + 1;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&')') {
        return None; // closes the wrapping "(" around the arrow itself
    }
    j += 1;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'(') {
        return None; // the immediate-invocation call
    }
    let call_close = matching_close(chars, mask, j, '(', ')')?;
    j = call_close + 1;

    loop {
        let mut k = j;
        skip_ws(chars, mask, &mut k);
        if chars.get(k) != Some(&'.') {
            break;
        }
        k += 1;
        let word_start = k;
        while chars.get(k).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            k += 1;
        }
        let word: String = chars[word_start..k].iter().collect();
        if !matches!(word.as_str(), "then" | "catch" | "finally") {
            break;
        }
        skip_ws(chars, mask, &mut k);
        if chars.get(k) != Some(&'(') {
            break;
        }
        let Some(close) = matching_close(chars, mask, k, '(', ')') else { break };
        j = close + 1;
    }

    Some((body_start, body_end, j))
}

#[cfg(test)]
mod tests {
    use super::unwrap_iife;

    #[test]
    fn unwraps_plain_iife() {
        assert_eq!(unwrap_iife("(()=>{osc(60).out()})()"), "{osc(60).out()}");
    }

    #[test]
    fn drops_trailing_catch_handler() {
        assert_eq!(
            unwrap_iife("(()=>{osc(60).out()})().catch(e=>log(e))"),
            "{osc(60).out()}"
        );
    }

    #[test]
    fn drops_then_and_catch_chain() {
        assert_eq!(
            unwrap_iife("(()=>{osc(60).out()})().then(x=>log(x)).catch(e=>log(e))"),
            "{osc(60).out()}"
        );
    }

    #[test]
    fn leaves_non_empty_params_alone() {
        let src = "((hydra)=>{osc(60).out()})(hydraSynth)";
        assert_eq!(unwrap_iife(src), src);
    }

    #[test]
    fn leaves_non_block_body_alone() {
        // strip_zero_arg_arrows (arrow.rs) already handles this shape
        let src = "(()=>time*0.1)()";
        assert_eq!(unwrap_iife(src), src);
    }

    #[test]
    fn leaves_normal_code_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(unwrap_iife(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"(()=>{x})()\") // (()=>{x})()";
        assert_eq!(unwrap_iife(src), src);
    }

    #[test]
    fn preserves_body_with_nested_braces() {
        assert_eq!(
            unwrap_iife("(()=>{if a { b } else { c }})()"),
            "{if a { b } else { c }}"
        );
    }
}
