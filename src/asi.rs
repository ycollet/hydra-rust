//! Automatic semicolon insertion (ASI).
//!
//! Real hydra.js sketches routinely put multiple statements on separate
//! lines with no trailing `;` (JS has ASI, Rhai does not), e.g.:
//! ```text
//! osc(10).out(o0)
//! osc(20).out(o1)
//! render()
//! ```
//! This scans the source once and inserts a `;` at any line break that
//! looks like a genuine statement boundary, while leaving multi-line method
//! chains (`osc(10)\n  .out(o0)`) and multi-line argument lists alone.

#[derive(Clone, Copy, PartialEq)]
enum StrKind {
    /// `"..."` — supports backslash escapes.
    Double,
    /// `'x'` — Rhai character literal, also supports backslash escapes.
    Single,
    /// `` `...` `` — Rhai raw string, no escape processing.
    Raw,
}

/// Characters that, when they're the last significant character on a line,
/// mean the expression clearly continues onto the next line.
fn continues_line(c: char) -> bool {
    matches!(
        c,
        ';' | '{'
            | ','
            | '('
            | '['
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '='
            | '<'
            | '>'
            | '!'
            | '&'
            | '|'
            | '^'
            | '.'
            | ':'
            | '?'
    )
}

/// Characters that, when they *start* the next non-blank line, mean it's a
/// continuation of the previous line rather than a new statement.
fn continues_next_line(c: char) -> bool {
    matches!(
        c,
        '.' | ')'
            | ']'
            | '}'
            | ','
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '='
            | '<'
            | '>'
            | '!'
            | '&'
            | '|'
            | '^'
            | ':'
            | '?'
    )
}

/// Looks ahead from `from` (skipping whitespace and comments) for the first
/// significant character of the next logical line. `None` if there isn't one
/// (rest of the file is blank/comments).
fn peek_next_significant(chars: &[char], mut j: usize) -> Option<char> {
    loop {
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        if j >= chars.len() {
            return None;
        }
        if chars[j] == '/' && chars.get(j + 1) == Some(&'/') {
            while j < chars.len() && chars[j] != '\n' {
                j += 1;
            }
            continue;
        }
        if chars[j] == '/' && chars.get(j + 1) == Some(&'*') {
            j += 2;
            while j + 1 < chars.len() && !(chars[j] == '*' && chars[j + 1] == '/') {
                j += 1;
            }
            j = (j + 2).min(chars.len());
            continue;
        }
        return Some(chars[j]);
    }
}

fn should_insert_semicolon(last: Option<char>, chars: &[char], next_pos: usize) -> bool {
    let Some(last) = last else { return false };
    if continues_line(last) {
        return false;
    }
    match peek_next_significant(chars, next_pos) {
        None => false,
        Some(c) => !continues_next_line(c),
    }
}

/// Inserts `;` at statement-boundary line breaks. Idempotent-ish: running it
/// twice on already-correct code is a no-op (it never inserts next to an
/// existing `;`).
pub fn insert_missing_semicolons(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(src.len() + 16);

    let mut i = 0;
    let mut depth: i32 = 0;
    let mut in_string: Option<StrKind> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut last_significant: Option<char> = None;

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
        if let Some(kind) = in_string {
            out.push(c);
            if kind != StrKind::Raw && c == '\\' && i + 1 < n {
                out.push(chars[i + 1]);
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
                out.push(c);
                last_significant = Some(c);
                i += 1;
            }
            '\'' => {
                in_string = Some(StrKind::Single);
                out.push(c);
                last_significant = Some(c);
                i += 1;
            }
            '`' => {
                in_string = Some(StrKind::Raw);
                out.push(c);
                last_significant = Some(c);
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
            '(' | '[' | '{' => {
                depth += 1;
                out.push(c);
                last_significant = Some(c);
                i += 1;
            }
            ')' | ']' | '}' => {
                depth -= 1;
                out.push(c);
                last_significant = Some(c);
                i += 1;
            }
            '\n' => {
                if depth == 0 && should_insert_semicolon(last_significant, &chars, i + 1) {
                    out.push(';');
                    last_significant = Some(';');
                }
                out.push(c);
                i += 1;
            }
            c if c.is_whitespace() => {
                out.push(c);
                i += 1;
            }
            c => {
                out.push(c);
                last_significant = Some(c);
                i += 1;
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::insert_missing_semicolons;

    #[test]
    fn inserts_between_bare_statements() {
        let out = insert_missing_semicolons("initCam(s0)\nsrc(s0).out()");
        assert_eq!(out, "initCam(s0);\nsrc(s0).out()");
    }

    #[test]
    fn leaves_multiline_chain_alone() {
        let src = "osc(60,0.1,0)\n  .rotate(0.1)\n  .out()";
        assert_eq!(insert_missing_semicolons(src), src);
    }

    #[test]
    fn leaves_existing_semicolons_alone() {
        let src = "a.setBins(4);\nosc(60,0.1,a.fft[0]).out()";
        assert_eq!(insert_missing_semicolons(src), src);
    }

    #[test]
    fn leaves_multiline_call_args_alone() {
        let src = "modulateRepeat(osc(4),\n  3, 3)\n.out()";
        assert_eq!(insert_missing_semicolons(src), src);
    }

    #[test]
    fn ignores_newlines_inside_strings() {
        let src = "text(\"hello\\nworld\")\nout()";
        let out = insert_missing_semicolons(src);
        assert_eq!(out, "text(\"hello\\nworld\");\nout()");
    }

    #[test]
    fn ignores_newlines_inside_line_comments() {
        let src = "osc(60)\n// a comment\n.out()";
        assert_eq!(insert_missing_semicolons(src), src);
    }

    #[test]
    fn handles_three_bare_statements() {
        let out = insert_missing_semicolons("osc(10).out(o0)\nosc(20).out(o1)\nrender()");
        assert_eq!(out, "osc(10).out(o0);\nosc(20).out(o1);\nrender()");
    }

    #[test]
    fn blank_lines_dont_double_insert() {
        let out = insert_missing_semicolons("osc(10).out(o0)\n\nosc(20).out(o1)");
        assert_eq!(out, "osc(10).out(o0);\n\nosc(20).out(o1)");
    }
}
