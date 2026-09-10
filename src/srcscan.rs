//! Shared groundwork for the hydra.js-compatibility preprocessing passes
//! (`asi`, `numlit`, `arrow`, ...): classifies which characters of a source
//! string lie inside a string literal or a comment, so each pass only has
//! to worry about its own token pattern rather than re-implementing
//! string/comment skipping.

#[derive(Clone, Copy, PartialEq)]
enum StrKind {
    /// `"..."` — supports backslash escapes.
    Double,
    /// `'x'` — Rhai character literal, also supports backslash escapes.
    Single,
    /// `` `...` `` — Rhai raw string, no escape processing.
    Raw,
}

/// What a given character is part of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Region {
    Code,
    StringLit,
    Comment,
}

/// Classifies every character in `chars` as code, string-literal content
/// (delimiters included), or comment content (delimiters included; for a
/// line comment, its terminating newline is classified as `Code` - it's
/// the comment's boundary, not its content, so passes that care about line
/// breaks, e.g. `asi`'s semicolon insertion, still see it as a real one).
pub fn classify(chars: &[char]) -> Vec<Region> {
    let n = chars.len();
    let mut region = vec![Region::Code; n];

    let mut in_string: Option<StrKind> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    let mut i = 0;
    while i < n {
        let c = chars[i];

        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            } else {
                region[i] = Region::Comment;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            region[i] = Region::Comment;
            if c == '*' && chars.get(i + 1) == Some(&'/') {
                region[i + 1] = Region::Comment;
                i += 2;
                in_block_comment = false;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(kind) = in_string {
            region[i] = Region::StringLit;
            if kind != StrKind::Raw && c == '\\' && i + 1 < n {
                region[i + 1] = Region::StringLit;
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
            i += 1;
            continue;
        }

        match c {
            '"' => {
                in_string = Some(StrKind::Double);
                region[i] = Region::StringLit;
                i += 1;
            }
            '\'' => {
                in_string = Some(StrKind::Single);
                region[i] = Region::StringLit;
                i += 1;
            }
            '`' => {
                in_string = Some(StrKind::Raw);
                region[i] = Region::StringLit;
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                in_line_comment = true;
                region[i] = Region::Comment;
                region[i + 1] = Region::Comment;
                i += 2;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                in_block_comment = true;
                region[i] = Region::Comment;
                region[i + 1] = Region::Comment;
                i += 2;
            }
            _ => i += 1,
        }
    }

    region
}

/// Returns a mask the same length as `chars`: `true` at every index that is
/// part of a string literal (delimiters included) or a `//`/`/* */` comment.
pub fn mask_strings_and_comments(chars: &[char]) -> Vec<bool> {
    classify(chars).iter().map(|r| *r != Region::Code).collect()
}

#[cfg(test)]
mod tests {
    use super::{classify, mask_strings_and_comments, Region};

    fn mask_of(src: &str) -> Vec<bool> {
        let chars: Vec<char> = src.chars().collect();
        mask_strings_and_comments(&chars)
    }

    fn classify_of(src: &str) -> Vec<Region> {
        let chars: Vec<char> = src.chars().collect();
        classify(&chars)
    }

    #[test]
    fn masks_double_quoted_string() {
        let src = "a(\"x.y\")b";
        let mask = mask_of(src);
        let masked: String = src
            .chars()
            .zip(mask.iter())
            .map(|(c, m)| if *m { c } else { '_' })
            .collect();
        assert_eq!(masked, "__\"x.y\"__");
        assert!(!mask[0] && !mask[1]);
        assert!(mask[2] && mask[6]);
        assert!(!mask[7] && !mask[8]);
    }

    #[test]
    fn masks_line_comment_to_end_of_line() {
        let src = "a // b.c\nd";
        let mask = mask_of(src);
        assert!(!mask[0]);
        assert!(mask[2] && mask[7]);
        assert!(!mask[9]);
    }

    #[test]
    fn masks_block_comment_across_lines() {
        let src = "a /* b\nc */ d";
        let mask = mask_of(src);
        assert!(!mask[0]);
        assert!(mask[2] && mask[6] && mask[10]);
        assert!(!mask[12]);
    }

    #[test]
    fn line_comments_terminating_newline_is_not_masked() {
        // regression test: a line comment's own newline must classify as
        // Code (it's the comment's boundary, not its content) - asi's
        // semicolon-insertion logic depends on seeing it as a real
        // newline, not something to skip over.
        let src = "a // comment\nb";
        let regions = classify_of(src);
        let newline_idx = src.find('\n').unwrap();
        assert_eq!(regions[newline_idx], Region::Code);
    }

    #[test]
    fn masks_backtick_raw_string() {
        let src = "a(`x.y`)b";
        let mask = mask_of(src);
        assert!(!mask[0] && !mask[1]);
        assert!(mask[2] && mask[6]);
        assert!(!mask[7] && !mask[8]);
    }

    #[test]
    fn masks_single_quoted_char_literal() {
        let src = "a('x')b";
        let mask = mask_of(src);
        assert!(!mask[0] && !mask[1]);
        assert!(mask[2] && mask[4]);
        assert!(!mask[5] && !mask[6]);
    }

    #[test]
    fn escaped_double_quote_does_not_end_the_string_early() {
        let src = r#"a("x\"y")b"#;
        let mask = mask_of(src);
        // the whole "x\"y" span (including the escaped quote) must stay
        // masked as one string, not end at the escaped `"`.
        let close_idx = src.rfind('"').unwrap();
        assert!(mask[close_idx]);
        assert!(!mask[close_idx + 1]); // the real closing `)`
    }

    #[test]
    fn slash_slash_inside_a_string_is_not_a_comment() {
        // the single most common real-world case: a URL passed to
        // initVideo/initImage/loadScript. If this ever regresses, every
        // preprocessing pass downstream would silently start treating the
        // rest of the line (or file, for a block-comment-shaped URL) as a
        // comment instead of code.
        let src = r#"initVideo(0, "https://example.com/v.mp4")"#;
        let regions = classify_of(src);
        let slashes = src.find("//").unwrap();
        assert_eq!(regions[slashes], Region::StringLit);
        assert_eq!(regions[slashes + 1], Region::StringLit);
        // and code after the string must be seen as real code, not comment
        let close_paren = src.rfind(')').unwrap();
        assert_eq!(regions[close_paren], Region::Code);
    }

    #[test]
    fn block_comment_markers_inside_a_string_are_not_a_comment() {
        let src = r#"text("/* not a comment */")"#;
        let regions = classify_of(src);
        let after_string = src.rfind(')').unwrap();
        assert_eq!(regions[after_string], Region::Code);
    }

    #[test]
    fn distinguishes_string_from_comment_regions() {
        let src = "\"a\" // b";
        let regions = classify_of(src);
        assert_eq!(regions[1], Region::StringLit); // the 'a'
        assert_eq!(regions[6], Region::Comment); // the 'b'
    }

    #[test]
    fn unterminated_string_stays_masked_to_end_of_input() {
        let src = "a(\"unterminated";
        let mask = mask_of(src);
        assert!(!mask[0] && !mask[1]);
        assert!(mask[2] && mask[src.len() - 1]);
    }

    #[test]
    fn empty_input_classifies_to_an_empty_vec() {
        assert!(classify_of("").is_empty());
    }
}
