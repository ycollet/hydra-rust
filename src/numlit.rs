//! Normalizes JS-style leading-dot decimal literals (`.5`) into valid Rhai
//! syntax (`0.5`). Real hydra.js sketches routinely drop the leading zero
//! (e.g. `osc(60,.1,0)`); Rhai's number-literal grammar requires it, and
//! without a preceding digit `.` can only mean member access, so a `.`
//! immediately followed by a digit is unambiguous.

#[derive(Clone, Copy, PartialEq)]
enum StrKind {
    Double,
    Single,
    Raw,
}

pub fn insert_leading_zero(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(src.len() + 8);

    let mut i = 0;
    let mut in_string: Option<StrKind> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut prev: Option<char> = None;

    while i < n {
        let c = chars[i];

        if in_line_comment {
            out.push(c);
            if c == '\n' {
                in_line_comment = false;
            }
            prev = Some(c);
            i += 1;
            continue;
        }
        if in_block_comment {
            out.push(c);
            if c == '*' && chars.get(i + 1) == Some(&'/') {
                out.push('/');
                prev = Some('/');
                i += 2;
                in_block_comment = false;
                continue;
            }
            prev = Some(c);
            i += 1;
            continue;
        }
        if let Some(kind) = in_string {
            out.push(c);
            if kind != StrKind::Raw && c == '\\' && i + 1 < n {
                out.push(chars[i + 1]);
                prev = Some(chars[i + 1]);
                i += 2;
                continue;
            }
            let closes = match kind {
                StrKind::Double => c == '"',
                StrKind::Single => c == '\'',
                StrKind::Raw => c == '`',
            };
            if closes {
                in_string = None;
            }
            prev = Some(c);
            i += 1;
            continue;
        }

        match c {
            '"' => {
                in_string = Some(StrKind::Double);
                out.push(c);
            }
            '\'' => {
                in_string = Some(StrKind::Single);
                out.push(c);
            }
            '`' => {
                in_string = Some(StrKind::Raw);
                out.push(c);
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                in_line_comment = true;
                out.push_str("//");
                prev = Some('/');
                i += 2;
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                in_block_comment = true;
                out.push_str("/*");
                prev = Some('*');
                i += 2;
                continue;
            }
            '.' if chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())
                && !prev.is_some_and(|p| p.is_ascii_digit()) =>
            {
                out.push('0');
                out.push('.');
            }
            _ => out.push(c),
        }
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
