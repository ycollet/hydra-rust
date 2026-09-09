//! Normalizes JS-style leading-dot decimal literals (`.5`) into valid Rhai
//! syntax (`0.5`). Real hydra.js sketches routinely drop the leading zero
//! (e.g. `osc(60,.1,0)`); Rhai's number-literal grammar requires it, and
//! without a preceding digit `.` can only mean member access, so a `.`
//! immediately followed by a digit is unambiguous.

use crate::srcscan::mask_strings_and_comments;

pub fn insert_leading_zero(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len() + 8);
    let mut prev: Option<char> = None;

    let mut i = 0;
    while i < n {
        let c = chars[i];
        if !mask[i]
            && c == '.'
            && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())
            && !prev.is_some_and(|p| p.is_ascii_digit())
        {
            out.push('0');
        }
        out.push(c);
        prev = Some(c);
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::insert_leading_zero;

    #[test]
    fn adds_leading_zero() {
        assert_eq!(insert_leading_zero("osc(60,.1,0)"), "osc(60,0.1,0)");
    }

    #[test]
    fn leaves_normal_floats_alone() {
        assert_eq!(insert_leading_zero("osc(60,0.1,0)"), "osc(60,0.1,0)");
    }

    #[test]
    fn leaves_multi_digit_floats_alone() {
        assert_eq!(insert_leading_zero("scale(1.5)"), "scale(1.5)");
    }

    #[test]
    fn handles_negative_shorthand() {
        assert_eq!(insert_leading_zero("rotate(-.5)"), "rotate(-0.5)");
    }

    #[test]
    fn leaves_method_calls_alone() {
        assert_eq!(insert_leading_zero("osc(60).rotate(0.1)"), "osc(60).rotate(0.1)");
    }

    #[test]
    fn ignores_dot_in_strings() {
        assert_eq!(insert_leading_zero("text(\".5\")"), "text(\".5\")");
    }

    #[test]
    fn ignores_dot_in_comments() {
        assert_eq!(insert_leading_zero("osc(60) // see .5 above"), "osc(60) // see .5 above");
    }
}
