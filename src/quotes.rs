//! Rewrites single-quoted strings (`'sin'`) into double-quoted strings
//! (`"sin"`). JS treats single and double quotes interchangeably as general
//! string literals; Rhai's single-quote syntax is a *character* literal
//! (exactly one character), so any multi-character JS string written with
//! single quotes fails to lex at all ("Invalid character").
//!
//! Runs before every other preprocessing pass, since `srcscan`'s
//! string/comment mask (which they all rely on) needs a consistent
//! double-quoted view of the source to work with.

pub fn rewrite_single_quoted_strings(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(src.len());

    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    let mut i = 0;
    while i < n {
        let c = chars[i];

        if in_line_comment {
            out.push(c);
            if c == '\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            out.push(c);
            if c == '*' && chars.get(i + 1) == Some(&'/') {
                out.push('/');
                i += 2;
                in_block_comment = false;
                continue;
            }
            i += 1;
            continue;
        }
        if in_double {
            out.push(c);
            if c == '\\' && i + 1 < n {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }

        match c {
            '"' => {
                in_double = true;
                out.push(c);
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                in_line_comment = true;
                out.push_str("//");
                i += 2;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                in_block_comment = true;
                out.push_str("/*");
                i += 2;
            }
            '\'' => {
                out.push('"');
                i += 1;
                while i < n {
                    let c2 = chars[i];
                    if c2 == '\\' && i + 1 < n {
                        let next = chars[i + 1];
                        if next == '\'' {
                            // no longer needs escaping once delimited by "
                            out.push('\'');
                        } else {
                            out.push('\\');
                            out.push(next);
                        }
                        i += 2;
                        continue;
                    }
                    if c2 == '"' {
                        out.push_str("\\\"");
                        i += 1;
                        continue;
                    }
                    if c2 == '\'' {
                        i += 1;
                        break;
                    }
                    out.push(c2);
                    i += 1;
                }
                out.push('"');
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::rewrite_single_quoted_strings;

    #[test]
    fn converts_simple_single_quoted_string() {
        assert_eq!(rewrite_single_quoted_strings(".ease('sin')"), ".ease(\"sin\")");
    }

    #[test]
    fn leaves_double_quoted_strings_alone() {
        let src = "text(\"hello\")";
        assert_eq!(rewrite_single_quoted_strings(src), src);
    }

    #[test]
    fn unescapes_embedded_single_quote() {
        assert_eq!(rewrite_single_quoted_strings("text('it\\'s')"), "text(\"it's\")");
    }

    #[test]
    fn escapes_embedded_double_quote() {
        assert_eq!(rewrite_single_quoted_strings("text('say \"hi\"')"), "text(\"say \\\"hi\\\"\")");
    }

    #[test]
    fn ignores_single_quotes_inside_double_quoted_strings() {
        let src = "text(\"it's fine\")";
        assert_eq!(rewrite_single_quoted_strings(src), src);
    }

    #[test]
    fn ignores_single_quotes_inside_comments() {
        let src = "osc(60) // don't touch this";
        assert_eq!(rewrite_single_quoted_strings(src), src);
    }

    #[test]
    fn handles_multiple_single_quoted_strings() {
        assert_eq!(
            rewrite_single_quoted_strings("initImage(s0,'photo.jpg')"),
            "initImage(s0,\"photo.jpg\")"
        );
    }
}
