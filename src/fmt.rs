//! Minimal "format code" implementation - `Ctrl`/`Cmd`+`Shift`+`F` in the
//! `hydra` binary - the Rust-native equivalent of upstream Hydra's own
//! `editor: format code`. See `format_code`'s own doc comment for exactly
//! what it does and doesn't do.

use crate::srcscan;

/// Reformats `code` by breaking any single-line method chain (e.g.
/// `osc(60).rotate(0.1).out()`) onto one call per line, indented two
/// spaces deeper than the chain's own first line - mirrors upstream
/// Hydra's `editor: format code` (`Shift`+`Ctrl`+`F`), which always
/// reformats the *entire* script, never just a selection (confirmed
/// against `hydra-synth/hydra`'s own `editor.js`: `formatCode()` calls
/// `beautify(this.cm.getValue(), ...)`, i.e. the whole document, not
/// `this.cm.getSelection()`).
///
/// Deliberately conservative, unlike `js-beautify`:
/// - Operates per "block" (a contiguous run of non-blank lines - the
///   same granularity `Alt`+`Enter`'s "eval block" uses). A block that
///   already spans multiple lines is left completely untouched - this
///   keeps formatting idempotent (running it twice is a no-op) and never
///   risks mis-joining an already-broken chain's own inline comments.
///   Only a genuinely single-line block gets split.
/// - A "chain dot" is only recognized as a `.` immediately followed by
///   an identifier and then `(` - a real method call - at bracket depth
///   0 relative to the block's own start, and only outside string
///   literals/comments (via `srcscan::classify`). This correctly leaves
///   decimal points (`0.5`'s `.` is never followed by an identifier),
///   property access without a call (e.g. `foo.bar` alone), and anything
///   inside a `setFunction` GLSL backtick body alone (it's always a
///   string literal, so `c0.rgb`-style GLSL swizzles are never touched).
/// - Indents with two spaces, not upstream's tabs, matching this
///   project's own existing convention (see `examples/*.hydra`).
pub fn format_code(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let mask = srcscan::mask_strings_and_comments(&chars);
    let n = chars.len();

    // Char-index (start, end) span of each line, `end` excluding the
    // line's own trailing `\n`.
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            lines.push((start, i));
            start = i + 1;
        }
    }
    lines.push((start, n));

    let is_blank = |(s, e): (usize, usize)| chars[s..e].iter().all(|c| c.is_whitespace());

    let mut out = String::with_capacity(code.len() + 32);
    let mut i = 0;
    while i < lines.len() {
        if is_blank(lines[i]) {
            out.extend(chars[lines[i].0..lines[i].1].iter());
            if i + 1 < lines.len() {
                out.push('\n');
            }
            i += 1;
            continue;
        }

        // Expand to the full block: every contiguous non-blank line.
        let block_start = i;
        let mut j = i;
        while j + 1 < lines.len() && !is_blank(lines[j + 1]) {
            j += 1;
        }
        let (bs, _) = lines[block_start];
        let (_, be) = lines[j];

        if block_start == j {
            out.push_str(&format_single_line_block(&chars, &mask, bs, be));
        } else {
            out.extend(chars[bs..be].iter());
        }

        if j + 1 < lines.len() {
            out.push('\n');
        }
        i = j + 1;
    }

    out
}

/// Formats one already-known-single-line block `[s, e)` of `chars`,
/// breaking every depth-0, non-string/comment "chain dot" onto its own
/// line, indented two spaces past the block's own leading whitespace.
fn format_single_line_block(chars: &[char], mask: &[bool], s: usize, e: usize) -> String {
    let indent: String = chars[s..e]
        .iter()
        .take_while(|c| **c == ' ' || **c == '\t')
        .collect();
    let continuation_indent = format!("{indent}  ");

    let is_ident_start = |c: char| c.is_alphabetic() || c == '_';
    let is_ident_continue = |c: char| c.is_alphanumeric() || c == '_';

    let mut out = String::with_capacity(e - s + 16);
    let mut depth: i32 = 0;
    let mut k = s;
    while k < e {
        let c = chars[k];
        if !mask[k] {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                '.' if depth == 0 => {
                    // Only a `.identifier(` at depth 0 counts as a chain
                    // call - not a decimal point or a bare property
                    // access.
                    let mut p = k + 1;
                    if p < e && is_ident_start(chars[p]) {
                        while p < e && is_ident_continue(chars[p]) {
                            p += 1;
                        }
                        if p < e && chars[p] == '(' {
                            out.push('\n');
                            out.push_str(&continuation_indent);
                            out.push('.');
                            k += 1;
                            continue;
                        }
                    }
                }
                _ => {}
            }
        }
        out.push(c);
        k += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::format_code;

    #[test]
    fn breaks_a_single_line_chain_onto_one_call_per_line() {
        let code = "osc(60).rotate(0.1).out()";
        assert_eq!(format_code(code), "osc(60)\n  .rotate(0.1)\n  .out()");
    }

    #[test]
    fn is_idempotent_on_an_already_broken_chain() {
        let code = "osc(60)\n  .rotate(0.1)\n  .out()";
        assert_eq!(format_code(code), code);
    }

    #[test]
    fn formats_each_blank_line_separated_block_independently() {
        let code = "osc(60).out(o0)\n\nsrc(o0).scale(1.5).out(o1)";
        assert_eq!(
            format_code(code),
            "osc(60)\n  .out(o0)\n\nsrc(o0)\n  .scale(1.5)\n  .out(o1)"
        );
    }

    #[test]
    fn preserves_blank_lines_and_trailing_comments() {
        let code = "osc(60).out() // a comment";
        assert_eq!(format_code(code), "osc(60)\n  .out() // a comment");
    }

    #[test]
    fn does_not_mistake_a_decimal_point_for_a_chain_dot() {
        let code = "rotate(0.5).out()";
        assert_eq!(format_code(code), "rotate(0.5)\n  .out()");
    }

    #[test]
    fn leaves_a_glsl_backtick_body_completely_untouched() {
        let code = "setFunction({name: 'f', type: 'color', inputs: [], glsl: `return c0.rgb.x > 0.5 ? c0 : vec4(0.0);`});";
        assert_eq!(format_code(code), code);
    }

    #[test]
    fn preserves_leading_indentation_when_breaking_an_indented_block() {
        let code = "  osc(60).out()";
        assert_eq!(format_code(code), "  osc(60)\n    .out()");
    }
}
