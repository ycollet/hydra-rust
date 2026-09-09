//! Shared groundwork for the hydra.js-compatibility preprocessing passes
//! (`asi`, `numlit`, `arrow`): classifies which characters of a source
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

/// Returns a mask the same length as `chars`: `true` at every index that is
/// part of a string literal (delimiters included) or a `//`/`/* */` comment.
pub fn mask_strings_and_comments(chars: &[char]) -> Vec<bool> {
    let n = chars.len();
    let mut mask = vec![false; n];

    let mut in_string: Option<StrKind> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    let mut i = 0;
    while i < n {
        let c = chars[i];

        if in_line_comment {
            if c == '\n' {
                // The terminating newline is the comment's boundary, not
                // its content - leave it unmasked so passes that care
                // about line breaks (e.g. asi's semicolon insertion) still
                // see it as a real one.
                in_line_comment = false;
            } else {
                mask[i] = true;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            mask[i] = true;
            if c == '*' && chars.get(i + 1) == Some(&'/') {
                mask[i + 1] = true;
                i += 2;
                in_block_comment = false;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(kind) = in_string {
            mask[i] = true;
            if kind != StrKind::Raw && c == '\\' && i + 1 < n {
                mask[i + 1] = true;
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
                mask[i] = true;
                i += 1;
            }
            '\'' => {
                in_string = Some(StrKind::Single);
                mask[i] = true;
                i += 1;
            }
            '`' => {
                in_string = Some(StrKind::Raw);
                mask[i] = true;
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                in_line_comment = true;
                mask[i] = true;
                mask[i + 1] = true;
                i += 2;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                in_block_comment = true;
                mask[i] = true;
                mask[i + 1] = true;
                i += 2;
            }
            _ => i += 1,
        }
    }

    mask
}

#[cfg(test)]
mod tests {
    use super::mask_strings_and_comments;

    fn mask_of(src: &str) -> Vec<bool> {
        let chars: Vec<char> = src.chars().collect();
        mask_strings_and_comments(&chars)
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
}
