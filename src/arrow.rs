//! Strips the zero-argument arrow-function wrapper `()=>expr` down to just
//! `expr`. Real hydra.js sketches wrap any per-frame-dynamic value in a
//! `()=>` closure (e.g. `rotate(()=>time*0.1)`); hydra-rust's dynamic
//! values (`time`, `mouse.x`, `a.fft[0]`, and arithmetic on them) already
//! compile straight into live GLSL expressions without needing a wrapping
//! closure, so the wrapper is textually redundant here.
//!
//! Also strips the destructured-parameter form (`({time})=>expr`, or with
//! multiple properties, `({time,mouse})=>expr`) - real hydra.js passes a
//! context object to per-frame callbacks and sketches destructure the
//! pieces they want out of it. The destructured names are simply dropped
//! rather than bound to anything: they're only useful here when they
//! happen to match one of hydra-rust's own globals (`time`, `mouse`, ...),
//! which already resolve correctly in the body without any binding: If a
//! sketch destructures something else, the body will fail with "Variable
//! not found" instead - a plain, graceful degradation, not worse than the
//! hard parse error this replaces.
//!
//! Deliberately left untouched:
//! - multi-param or bare-identifier arrows (`(a,b)=>...`, `x=>...`), used
//!   for a different purpose (pattern/sequencer callbacks)
//! - block-bodied arrows (`()=>{ ... }`), which don't reduce to a bare
//!   expression

use crate::srcscan::mask_strings_and_comments;

pub fn strip_zero_arg_arrows(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());

    let mut i = 0;
    while i < n {
        if !mask[i]
            && let Some(after) = match_zero_arg_arrow(&chars, &mask, i)
        {
            i = after;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

/// If a zero-arg, single-expression arrow (`()=>expr`) or a
/// destructured-parameter arrow (`({name, ...})=>expr`) starts at `i`,
/// returns the index just past its `=>`, i.e. where the wrapped expression
/// begins.
fn match_zero_arg_arrow(chars: &[char], mask: &[bool], i: usize) -> Option<usize> {
    if chars.get(i) != Some(&'(') {
        return None;
    }
    let mut j = i + 1;
    skip_ws(chars, mask, &mut j);

    if chars.get(j) == Some(&')') {
        j += 1;
    } else if chars.get(j) == Some(&'{') {
        j = skip_destructure_pattern(chars, mask, j)?;
    } else {
        return None;
    }

    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'=') || chars.get(j + 1) != Some(&'>') {
        return None;
    }
    j += 2;

    let mut k = j;
    skip_ws(chars, mask, &mut k);
    if chars.get(k) == Some(&'{') {
        return None; // block body: leave it alone
    }

    Some(j)
}

/// If `{ name, name2, ... }` (a destructuring pattern, identifiers only)
/// starts at `j` (pointing at the `{`) and is immediately followed by `)`,
/// returns the index just past that `)`.
fn skip_destructure_pattern(chars: &[char], mask: &[bool], j: usize) -> Option<usize> {
    let mut k = j + 1;
    loop {
        skip_ws(chars, mask, &mut k);
        let start = k;
        while chars.get(k).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            k += 1;
        }
        if k == start {
            return None; // expected an identifier
        }
        skip_ws(chars, mask, &mut k);
        match chars.get(k) {
            Some(',') => {
                k += 1;
                continue;
            }
            Some('}') => {
                k += 1;
                break;
            }
            _ => return None,
        }
    }
    skip_ws(chars, mask, &mut k);
    if chars.get(k) != Some(&')') {
        return None;
    }
    Some(k + 1)
}

fn skip_ws(chars: &[char], mask: &[bool], j: &mut usize) {
    while *j < chars.len() && (chars[*j].is_whitespace() || mask[*j]) {
        *j += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::strip_zero_arg_arrows;

    #[test]
    fn strips_simple_arrow() {
        assert_eq!(strip_zero_arg_arrows("rotate(()=>time*0.1)"), "rotate(time*0.1)");
    }

    #[test]
    fn strips_single_destructured_param() {
        assert_eq!(
            strip_zero_arg_arrows("invert(({time})=>Math.sin(time)*3)"),
            "invert(Math.sin(time)*3)"
        );
    }

    #[test]
    fn strips_multi_destructured_param() {
        assert_eq!(
            strip_zero_arg_arrows("rotate(({time,mouse})=>time*mouse.x)"),
            "rotate(time*mouse.x)"
        );
    }

    #[test]
    fn leaves_malformed_destructure_alone() {
        let src = "rotate(({time)=>time)";
        assert_eq!(strip_zero_arg_arrows(src), src);
    }

    #[test]
    fn strips_arrow_with_spaces() {
        // whitespace around the removed `()=>` tokens is harmless to Rhai
        // and isn't specifically collapsed
        assert_eq!(strip_zero_arg_arrows("rotate( () => time )"), "rotate(  time )");
    }

    #[test]
    fn strips_multiple_arrows() {
        assert_eq!(
            strip_zero_arg_arrows("osc(()=>time,()=>mouse.x)"),
            "osc(time,mouse.x)"
        );
    }

    #[test]
    fn leaves_block_body_alone() {
        let src = "rotate(()=>{ return time; })";
        assert_eq!(strip_zero_arg_arrows(src), src);
    }

    #[test]
    fn leaves_multi_param_alone() {
        let src = ".fast((val,i)=>val*2)";
        assert_eq!(strip_zero_arg_arrows(src), src);
    }

    #[test]
    fn leaves_bare_identifier_param_alone() {
        let src = ".fast(x=>x*2)";
        assert_eq!(strip_zero_arg_arrows(src), src);
    }

    #[test]
    fn leaves_normal_parens_alone() {
        assert_eq!(strip_zero_arg_arrows("rotate((0.1))"), "rotate((0.1))");
    }

    #[test]
    fn ignores_arrows_in_strings_and_comments() {
        let src = "text(\"()=>time\") // ()=>time";
        assert_eq!(strip_zero_arg_arrows(src), src);
    }

    #[test]
    fn strips_variable_assigned_arrow() {
        assert_eq!(strip_zero_arg_arrows("pat = ()=>\nsolid()"), "pat = \nsolid()");
    }
}
