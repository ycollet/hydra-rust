//! Rewrites a JS arrow function's parameter-list header into Rhai's own
//! closure syntax (`|params|`) wherever it appears as anything *other*
//! than the right-hand side of a bare assignment - `arrow.rs`'s zero-arg/
//! destructured-parameter cases and `arrowfn.rs`'s named-function-*value*
//! assignment both own that position already (see their own module doc
//! comments).
//!
//! Real sketches commonly use JS's generic `Array` functional methods this
//! way (`arr.reduce((a,b)=>a+b, 0)`, `[0,2,4].map((v,i)=>...)`) - nothing
//! to do with hydra's own chain API, and typically one-time setup
//! computation (palette generation, an audio-FFT sum) rather than
//! per-frame reactive code. Rhai has no `=>` closure syntax at all, but
//! *does* have its own closure syntax (`|params| body`) that - unlike a
//! plain `fn` - can capture outer-scope variables, and Rhai's own
//! `Array::reduce`/`map`/`filter`/... already accept one directly, so
//! this is a purely mechanical header swap: the body (expression or
//! block, whatever ternaries/`Math.*`/etc. it contains) is left
//! completely untouched, since Rhai's closure body grammar is otherwise
//! identical to `fn`'s.
//!
//! Also picks up the zero-argument *block*-bodied case (`()=>{...}`) that
//! `arrow.rs` deliberately leaves alone wherever it isn't at assignment
//! position (its own stripping only ever applies to a value-producing
//! expression body) - `||{...}` is exactly as valid a Rhai closure as any
//! other arity.
//!
//! Deliberately left alone: a destructured-parameter arrow (`({time})=>
//! ...`) - Rhai closures take a plain identifier list, not an object
//! pattern, and `arrow.rs` already handles the *expression*-bodied form of
//! this shape; the far rarer block-bodied form remains an unhandled gap,
//! not addressed here.
//!
//! Runs right after `arrow.rs` (step 14): only needs to see whatever that
//! pass didn't already resolve, and its own transformation is a purely
//! local header swap that doesn't care what any other pass has or hasn't
//! done to the body's own content yet.

use crate::arrow::preceded_by_bare_assignment_eq;
use crate::jsfunctions::{matching_close, parse_params};
use crate::srcscan::mask_strings_and_comments;

pub fn rewrite_argument_position_closures(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());

    let mut i = 0;
    while i < n {
        if !mask[i]
            && !preceded_by_bare_assignment_eq(&chars, &mask, i)
            && let Some((names, after)) = match_arrow_header(&chars, &mask, i)
        {
            out.push('|');
            out.push_str(&names.join(","));
            out.push('|');
            i = after;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

/// If an arrow header - `(params) =>` (params may be empty, or a bare
/// comma-separated identifier list, default values stripped) or a bare
/// single-identifier `ident =>` - starts at `i`, returns its parameter
/// names and the index just past the `=>`. `None` if what's at `i` isn't
/// this shape, or the parenthesized parameter list contains anything
/// other than plain identifiers (a destructuring pattern, `{...}`).
fn match_arrow_header(chars: &[char], mask: &[bool], i: usize) -> Option<(Vec<String>, usize)> {
    let (names, mut j) = if chars.get(i) == Some(&'(') {
        let close = matching_close(chars, mask, i, '(', ')')?;
        if chars[i + 1..close].iter().enumerate().any(|(k, c)| *c == '{' && !mask[i + 1 + k]) {
            return None; // a destructuring pattern - not handled here
        }
        let params = parse_params(chars, mask, i + 1, close);
        (params.into_iter().map(|(n, _)| n).collect::<Vec<_>>(), close + 1)
    } else if chars.get(i).is_some_and(|c| is_ident_start(*c)) && !preceded_by_ident_or_dot(chars, i) {
        let end = ident_end(chars, i);
        (vec![chars[i..end].iter().collect()], end)
    } else {
        return None;
    };

    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'=') || chars.get(j + 1) != Some(&'>') {
        return None;
    }
    Some((names, j + 2))
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
    use super::rewrite_argument_position_closures;

    #[test]
    fn rewrites_a_two_param_reduce_callback() {
        assert_eq!(
            rewrite_argument_position_closures("arr.reduce((a,b)=>a+b, 0)"),
            "arr.reduce(|a,b|a+b, 0)"
        );
    }

    #[test]
    fn rewrites_a_three_param_reduce_callback() {
        assert_eq!(
            rewrite_argument_position_closures("arr.reduce((acc,val,i)=>acc+val, 0)"),
            "arr.reduce(|acc,val,i|acc+val, 0)"
        );
    }

    #[test]
    fn rewrites_a_bare_single_param_callback() {
        assert_eq!(rewrite_argument_position_closures(".fast(x=>x*2)"), ".fast(|x|x*2)");
    }

    #[test]
    fn rewrites_a_zero_param_block_bodied_argument_position_arrow() {
        assert_eq!(
            rewrite_argument_position_closures(".method(()=>{ doThing(); })"),
            ".method(||{ doThing(); })"
        );
    }

    #[test]
    fn rewrites_a_block_bodied_multi_param_callback() {
        assert_eq!(
            rewrite_argument_position_closures("arr.reduce((a,v,i) => { if(i<4) return 0; return a+v; })"),
            "arr.reduce(|a,v,i| { if(i<4) return 0; return a+v; })"
        );
    }

    #[test]
    fn strips_default_parameter_values() {
        assert_eq!(
            rewrite_argument_position_closures("arr.reduce((a,b=1)=>a+b)"),
            "arr.reduce(|a,b|a+b)"
        );
    }

    #[test]
    fn leaves_a_destructured_parameter_arrow_alone() {
        let src = ".method(({time})=>{ doThing(time); })";
        assert_eq!(rewrite_argument_position_closures(src), src);
    }

    #[test]
    fn leaves_an_assignment_target_arrow_alone() {
        // arrowfn.rs owns this position - see the module doc comment.
        let src = "let name = (a,b) => a+b;";
        assert_eq!(rewrite_argument_position_closures(src), src);
    }

    #[test]
    fn leaves_a_property_path_assignment_target_arrow_alone() {
        let src = "window.onclick = (e) => handle(e);";
        assert_eq!(rewrite_argument_position_closures(src), src);
    }

    #[test]
    fn leaves_a_normal_call_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(rewrite_argument_position_closures(src), src);
    }

    #[test]
    fn leaves_a_normal_multi_arg_call_alone() {
        let src = "shape(4, 0.1, 1)";
        assert_eq!(rewrite_argument_position_closures(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"(a,b)=>a+b\") // (a,b)=>a+b";
        assert_eq!(rewrite_argument_position_closures(src), src);
    }

    #[test]
    fn does_not_misfire_on_identifiers_starting_with_no_reserved_prefix() {
        // a bare-identifier arrow with a plain name - not a keyword - is
        // fine to convert; just a sanity check the ident scan itself
        // isn't accidentally over/under-matching a longer identifier.
        assert_eq!(
            rewrite_argument_position_closures("xvalue=>xvalue*2"),
            "|xvalue|xvalue*2"
        );
    }
}
