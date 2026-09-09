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

use crate::srcscan::mask_strings_and_comments;

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

/// Looks ahead from `from` (skipping whitespace, strings and comments) for
/// the first significant character of the next logical line. `None` if
/// there isn't one (rest of the file is blank/comments).
fn peek_next_significant(chars: &[char], mask: &[bool], mut j: usize) -> Option<char> {
    while j < chars.len() && (chars[j].is_whitespace() || mask[j]) {
        j += 1;
    }
    chars.get(j).copied()
}

fn should_insert_semicolon(last: Option<char>, chars: &[char], mask: &[bool], next_pos: usize) -> bool {
    let Some(last) = last else { return false };
    if continues_line(last) {
        return false;
    }
    match peek_next_significant(chars, mask, next_pos) {
        None => false,
        Some(c) => !continues_next_line(c),
    }
}

/// Inserts `;` at statement-boundary line breaks. Idempotent-ish: running it
/// twice on already-correct code is a no-op (it never inserts next to an
/// existing `;`).
pub fn insert_missing_semicolons(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len() + 16);

    let mut depth: i32 = 0;
    let mut last_significant: Option<char> = None;
    // Byte offset in `out` right after the last significant char - where a
    // semicolon needs to be spliced in, since a trailing same-line comment
    // (masked, but its terminating newline isn't - see srcscan.rs) may
    // already be sitting between that char and the newline we're reacting
    // to. Inserting at the newline itself would land the `;` inside the
    // comment, where Rhai's lexer just discards it as more comment text.
    let mut last_significant_end: usize = 0;

    let mut i = 0;
    while i < n {
        let c = chars[i];

        if mask[i] {
            out.push(c);
            i += 1;
            continue;
        }

        match c {
            '(' | '[' | '{' => {
                depth += 1;
                last_significant = Some(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                last_significant = Some(c);
            }
            '\n' => {
                if depth == 0 && should_insert_semicolon(last_significant, &chars, &mask, i + 1) {
                    out.insert(last_significant_end, ';');
                    last_significant = Some(';');
                }
                out.push(c);
                i += 1;
                continue;
            }
            c if c.is_whitespace() => {}
            _ => last_significant = Some(c),
        }

        out.push(c);
        if !c.is_whitespace() {
            last_significant_end = out.len();
        }
        i += 1;
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

    #[test]
    fn inserts_semicolon_before_trailing_line_comment() {
        // regression test: srcscan used to mark a line comment's
        // terminating newline as masked, which made asi's `'\n' => ...`
        // match arm unreachable for any line ending in `// comment`.
        let out = insert_missing_semicolons("a.setScale(4) // comment one\na.setCutoff(7) // comment two");
        assert_eq!(out, "a.setScale(4); // comment one\na.setCutoff(7) // comment two");
    }
}
