//! Drops excess trailing call arguments beyond what each hydra function
//! actually takes. JavaScript silently ignores extra arguments beyond a
//! function's declared parameters; Rhai has no such leniency and hard-errors
//! ("Function not found") instead. Real sketches routinely pass a few extra
//! numbers (typos, copy-paste, muscle memory), e.g. `rotate(4, 0.1, 0)`
//! (rotate only takes 2 params) or a `color(...)` call with ten arguments
//! instead of four.

use crate::srcscan::mask_strings_and_comments;

/// Function name -> max number of arguments accepted *inside its parens* at
/// the call site. For blend/modulate-kind functions this includes the
/// leading "other" operand (e.g. `modulate(other, amount)` -> 2), since
/// from a pure text-truncation point of view it's just another
/// comma-separated argument.
const MAX_ARGS: &[(&str, usize)] = &[
    // sources
    ("osc", 3),
    ("noise", 2),
    ("voronoi", 3),
    ("shape", 3),
    ("gradient", 1),
    ("solid", 4),
    ("rings", 2),
    ("checker", 2),
    // geo
    ("rotate", 2),
    ("scale", 5),
    ("scroll", 4),
    ("kaleid", 1),
    ("pixelate", 2),
    ("repeat", 4),
    ("scrollX", 2),
    ("scrollY", 2),
    ("repeatX", 2),
    ("repeatY", 2),
    ("polar", 0),
    ("cart", 0),
    ("fold", 1),
    // color
    ("color", 4),
    ("invert", 1),
    ("contrast", 1),
    ("brightness", 1),
    ("saturate", 1),
    ("hue", 1),
    ("posterize", 2),
    ("luma", 2),
    ("colorama", 1),
    ("shift", 4),
    ("thresh", 2),
    ("r", 2),
    ("g", 2),
    ("b", 2),
    ("a", 2),
    ("sum", 4),
    // blend (other + extras)
    ("add", 2),
    ("mult", 2),
    ("blend", 2),
    ("diff", 1),
    ("layer", 1),
    ("mask", 1),
    ("sub", 2),
    // modulate (other + extras)
    ("modulate", 2),
    ("modulateScale", 3),
    ("modulateRotate", 3),
    ("modulateRepeat", 5),
    ("modulateRepeatX", 3),
    ("modulateRepeatY", 3),
    ("modulateKaleid", 2),
    ("modulateScrollX", 3),
    ("modulateScrollY", 3),
    ("modulatePixelate", 3),
    ("modulateHue", 2),
];

pub fn truncate_extra_args(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    process(&chars, &mask)
}

fn process(chars: &[char], mask: &[bool]) -> String {
    let n = chars.len();
    let mut out = String::with_capacity(chars.len());

    let mut i = 0;
    while i < n {
        if !mask[i] && is_ident_char(chars[i]) && !preceded_by_ident(chars, i) {
            let ident_end = ident_end(chars, i);
            let ident: String = chars[i..ident_end].iter().collect();

            if let Some(&(_, max_args)) = MAX_ARGS.iter().find(|(name, _)| *name == ident) {
                let mut p = ident_end;
                skip_ws(chars, mask, &mut p);
                if chars.get(p) == Some(&'(')
                    && let Some(close) = matching_paren(chars, mask, p)
                {
                    out.push_str(&ident);
                    out.extend(&chars[ident_end..p]);
                    out.push('(');
                    out.push_str(&truncate_arg_list(chars, mask, p + 1, close, max_args));
                    out.push(')');
                    i = close + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

/// Splits `chars[start..end]` on top-level commas (respecting nested
/// brackets and strings/comments), keeps only the first `max_args` of them
/// (each recursively re-processed, so nested calls get truncated too), and
/// rejoins them with `,`.
fn truncate_arg_list(chars: &[char], mask: &[bool], start: usize, end: usize, max_args: usize) -> String {
    let args = split_top_level(chars, mask, start, end);
    args.into_iter()
        .take(max_args)
        .map(|(s, e)| process(&chars[s..e], &mask[s..e]))
        .collect::<Vec<_>>()
        .join(",")
}

/// Returns the (start, end) char ranges of each top-level comma-separated
/// argument in `chars[start..end]`. An all-whitespace range (empty arg
/// list) yields no arguments.
fn split_top_level(chars: &[char], mask: &[bool], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut arg_start = start;
    let mut i = start;

    while i < end {
        if !mask[i] {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    args.push((arg_start, i));
                    arg_start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    // The trailing segment (after the last top-level comma, or the whole
    // range if there were none) only counts as an argument if it has any
    // real content, or a comma already implied one ("foo()" -> no args,
    // but "foo(1,)" -> two, the second empty).
    let has_content = (arg_start..end).any(|k| !mask[k] && !chars[k].is_whitespace());
    if has_content || !args.is_empty() {
        args.push((arg_start, end));
    }

    args
}

fn matching_paren(chars: &[char], mask: &[bool], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
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

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn preceded_by_ident(chars: &[char], i: usize) -> bool {
    i > 0 && is_ident_char(chars[i - 1])
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
    use super::truncate_extra_args;

    #[test]
    fn truncates_extra_args() {
        assert_eq!(truncate_extra_args("rotate(4,0.1,0)"), "rotate(4,0.1)");
    }

    #[test]
    fn leaves_correct_arity_alone() {
        assert_eq!(truncate_extra_args("rotate(4,0.1)"), "rotate(4,0.1)");
    }

    #[test]
    fn leaves_fewer_args_alone() {
        assert_eq!(truncate_extra_args("rotate(4)"), "rotate(4)");
    }

    #[test]
    fn truncates_many_extra_color_args() {
        assert_eq!(
            truncate_extra_args("color(11,0.5,0.4,0.9,0.2,0.011,5,22,0.5,-1)"),
            "color(11,0.5,0.4,0.9)"
        );
    }

    #[test]
    fn does_not_split_nested_call_args() {
        assert_eq!(truncate_extra_args("rotate(0.1,noise(4,0.1))"), "rotate(0.1,noise(4,0.1))");
    }

    #[test]
    fn truncates_nested_call_args_too() {
        assert_eq!(
            truncate_extra_args("rotate(0.1,noise(4,0.1,99))"),
            "rotate(0.1,noise(4,0.1))"
        );
    }

    #[test]
    fn handles_method_chain_form() {
        assert_eq!(truncate_extra_args("osc(60).rotate(4,0.1,0).out()"), "osc(60).rotate(4,0.1).out()");
    }

    #[test]
    fn truncates_blend_other_plus_extras() {
        assert_eq!(truncate_extra_args("osc(1).modulate(o1,0.5,9)"), "osc(1).modulate(o1,0.5)");
    }

    #[test]
    fn zero_arg_function_drops_everything() {
        assert_eq!(truncate_extra_args("polar(1,2)"), "polar()");
    }

    #[test]
    fn ignores_unrelated_identifiers() {
        assert_eq!(truncate_extra_args("time"), "time");
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"rotate(1,2,3)\") // rotate(1,2,3)";
        assert_eq!(truncate_extra_args(src), src);
    }
}
