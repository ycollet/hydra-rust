//! Wraps a JS brace-less single-statement `if`/`else` body in `{ }`.
//! Real sketches very commonly write `if (cond) stmt;` or `if (cond) stmt1;
//! else stmt2;` with no braces at all (valid JS, and idiomatic for a short
//! guard clause like `if (t < 1) return [1, 0, 0];`) - Rhai's own `if`
//! requires a brace-delimited block on every branch, no bare-statement
//! form at all ("Expecting '{' to start a statement block").
//!
//! Runs *before* `asi`, not after - critically so. `asi` has no idea what
//! an `if`/`for`/`while` header even is; it just tracks bracket depth and
//! reacts to line endings. A brace-less `if (cond)\nBODY` (real, common
//! multi-line style) leaves the condition's own line ending in `)` - not
//! one of `asi`'s "continues" characters - so `asi`, seeing what looks
//! like a complete line followed by a fresh-looking statement, inserts a
//! `;` **right after the condition**, turning it into `if (cond);` before
//! this pass ever runs. That's a *valid-looking* but silently wrong
//! transformation once wrapped in `{ }` (an empty, always-true-or-false
//! branch, with `BODY` now unconditional right after it) - worse than the
//! loud parse error it replaces, because it evaluates successfully with
//! the wrong behavior instead of failing. Running before `asi` avoids the
//! interaction entirely: once a brace-less body is already wrapped in
//! `{ }`, `asi`'s own bracket-depth tracking correctly sees the whole
//! `if`/`else` construct as one balanced unit and never has a chance to
//! split the header from its body.
//!
//! Without `asi` having already run, a body with no explicit `;` of its
//! own needs the same "does this line genuinely end here" newline
//! heuristic `ternary.rs` uses for exactly the same reason (same pass
//! ordering constraint, same fix) - see `ends_statement_at_newline`.
//!
//! `else if` chains are handled without double-wrapping: the token right
//! after `else` being the `if` keyword is treated as "recurse into this as
//! another `if`/`else` structure," not "wrap a single statement that
//! happens to start with `if`" - producing `else if cond { .. } else { .. }`
//! the same way hand-written Rhai would, rather than an extra `else { if
//! .. }` nesting level (equally valid, just not how anyone would write it).
//!
//! A body that's already brace-delimited is left as-is *structurally*, but
//! still recursively reprocessed (its own content might contain further
//! un-braced nested `if`s the outer scan would otherwise skip straight
//! past, since a matched `if` construct is replaced as one atomic span).

use crate::srcscan::mask_strings_and_comments;

pub fn brace_bare_if_bodies(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    transform(&chars, &mask, 0, chars.len())
}

fn transform(chars: &[char], mask: &[bool], start: usize, end: usize) -> String {
    let mut out = String::with_capacity(end - start);
    let mut i = start;
    while i < end {
        if !mask[i]
            && chars[i] == 'i'
            && matches_word(chars, i, end, "if")
            && !preceded_by_ident_char(chars, i)
            && let Some((rewritten, after)) = try_wrap_if(chars, mask, i, end)
        {
            out.push_str(&rewritten);
            i = after;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// If an `if (cond) BODY [else BODY2]` construct starts at `i`, returns the
/// fully reconstructed text (every un-braced `BODY`/`BODY2` wrapped in
/// `{ }`, recursively reprocessed either way) and the index just past the
/// whole construct.
fn try_wrap_if(chars: &[char], mask: &[bool], i: usize, end: usize) -> Option<(String, usize)> {
    let mut j = i + 2; // past "if"
    skip_ws(chars, mask, &mut j, end);
    if chars.get(j) != Some(&'(') {
        return None;
    }
    let cond_close = matching_close(chars, mask, j, end)?;
    let cond: String = chars[i..=cond_close].iter().collect();
    j = cond_close + 1;

    let (then_part, after_then) = wrap_body(chars, mask, j, end)?;

    let mut out = format!("{cond} {then_part}");

    // Peek past whitespace for "else" without committing to having
    // skipped it - if there's no "else" here, that whitespace still needs
    // to reach the output via the caller's own untouched copy-through.
    let mut j = after_then;
    skip_ws(chars, mask, &mut j, end);
    if matches_word(chars, j, end, "else") && !preceded_by_ident_char(chars, j) {
        let mut k = j + 4; // past "else"
        skip_ws(chars, mask, &mut k, end);
        let (part, after) = if matches_word(chars, k, end, "if") && !preceded_by_ident_char(chars, k) {
            try_wrap_if(chars, mask, k, end)?
        } else {
            wrap_body(chars, mask, k, end)?
        };
        out.push_str(" else ");
        out.push_str(&part);
        return Some((out, after));
    }

    Some((out, after_then))
}

/// Wraps one `if`/`else` branch's body at `j`: if it's already `{ ... }`,
/// its content is recursively reprocessed and the braces kept as-is;
/// otherwise the body is a single statement ending at the next top-level
/// `;` (or end of input), recursively reprocessed and wrapped in `{ }`.
/// Returns the wrapped text and the index just past it (the closing `}`,
/// or the consumed `;`).
fn wrap_body(chars: &[char], mask: &[bool], j: usize, end: usize) -> Option<(String, usize)> {
    let mut j = j;
    skip_ws(chars, mask, &mut j, end);
    if chars.get(j) == Some(&'{') {
        let close = matching_close(chars, mask, j, end)?;
        let inner = transform(chars, mask, j + 1, close);
        return Some((format!("{{{inner}}}"), close + 1));
    }
    let stmt_end = scan_statement_end(chars, mask, j, end);
    let inner = transform(chars, mask, j, stmt_end);
    Some((format!("{{ {} }}", inner.trim()), stmt_end))
}

/// Scans a single un-braced statement from `j` up to (and including) the
/// next top-level `;`; if there isn't one before a bare newline that looks
/// like a genuine statement boundary (see `ends_statement_at_newline` -
/// `asi` hasn't run yet at this point in the pipeline), stops there
/// instead (newline excluded); falls back to `end` if neither occurs.
fn scan_statement_end(chars: &[char], mask: &[bool], j: usize, end: usize) -> usize {
    let mut depth = 0i32;
    let mut i = j;
    while i < end {
        if !mask[i] {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ';' if depth == 0 => return i + 1,
                '\n' if depth == 0 && ends_statement_at_newline(chars, mask, j, i, end) => {
                    return i;
                }
                _ => {}
            }
        }
        i += 1;
    }
    end
}

/// True if a bare newline at `i` looks like a genuine statement boundary -
/// same idea (and same two character sets) as `ternary.rs`'s
/// `ends_branch_at_newline` and `asi::insert_missing_semicolons`'s own
/// heuristic: neither the statement text scanned so far (from `start`)
/// nor the next line's first significant character looks like a
/// continuation. Two distinct character sets, not one merged set: a line
/// *ending* in `)`/`]`/`}` is ordinarily a complete statement (`foo(a,
/// b)`), but a line *starting* with one of those continues the previous
/// line (closing a multi-line call) - `asi.rs`'s own two sets, duplicated
/// here rather than made `pub(crate)` there, matching this codebase's
/// existing convention of small local per-module helpers.
fn ends_statement_at_newline(chars: &[char], mask: &[bool], start: usize, i: usize, end: usize) -> bool {
    let mut k = i;
    while k > start && (mask[k - 1] || chars[k - 1].is_whitespace()) {
        k -= 1;
    }
    if k > start && continues_line(chars[k - 1]) {
        return false;
    }
    let mut j = i + 1;
    while j < end && (chars[j].is_whitespace() || mask[j]) {
        j += 1;
    }
    match chars.get(j).filter(|_| j < end) {
        None => false,
        Some(c) => !continues_next_line(*c),
    }
}

fn continues_line(c: char) -> bool {
    matches!(
        c,
        ';' | '{' | ',' | '(' | '[' | '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '.' | ':' | '?'
    )
}

fn continues_next_line(c: char) -> bool {
    matches!(
        c,
        '.' | ')' | ']' | '}' | ',' | '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | ':' | '?'
    )
}

fn matches_word(chars: &[char], i: usize, end: usize, word: &str) -> bool {
    let wend = i + word.len();
    wend <= end
        && chars[i..wend].iter().collect::<String>() == word
        && chars.get(wend).is_none_or(|c| !is_ident_char(*c))
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn preceded_by_ident_char(chars: &[char], i: usize) -> bool {
    i > 0 && is_ident_char(chars[i - 1])
}

fn matching_close(chars: &[char], mask: &[bool], open_idx: usize, end: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < end {
        if !mask[i] {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
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

fn skip_ws(chars: &[char], mask: &[bool], j: &mut usize, end: usize) {
    while *j < end && (chars[*j].is_whitespace() || mask[*j]) {
        *j += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::brace_bare_if_bodies;

    #[test]
    fn wraps_a_bare_if_with_no_else() {
        assert_eq!(
            brace_bare_if_bodies("if (t < 1) return [1, 0, 0];"),
            "if (t < 1) { return [1, 0, 0]; }"
        );
    }

    #[test]
    fn wraps_a_multiline_condition_and_body_with_no_semicolon_at_all() {
        // the critical case this pass exists to get right before `asi`
        // ever runs: a brace-less body on its own following line, with no
        // explicit `;` anywhere - if `asi` saw this first, it would insert
        // a `;` right after the condition (not a "continues" character),
        // making the body unconditional instead of failing loudly or
        // wrapping correctly.
        assert_eq!(
            brace_bare_if_bodies("if(time<10)\nrender(o0)\nosc(60).out()"),
            "if(time<10) { render(o0) }\nosc(60).out()"
        );
    }

    #[test]
    fn wraps_a_bare_if_with_a_bare_else() {
        assert_eq!(
            brace_bare_if_bodies("if (a) return 0.1; else return 0.6;"),
            "if (a) { return 0.1; } else { return 0.6; }"
        );
    }

    #[test]
    fn inserts_a_missing_trailing_semicolon_style_terminator() {
        // no explicit `;` at all - falls back to end of input, same as
        // arrowfn.rs's own scan_expr_body does.
        assert_eq!(
            brace_bare_if_bodies("if (a) render(o0)"),
            "if (a) { render(o0) }"
        );
    }

    #[test]
    fn leaves_an_already_braced_if_structurally_alone() {
        let src = "if (a) { foo(); } else { bar(); }";
        assert_eq!(brace_bare_if_bodies(src), src);
    }

    #[test]
    fn handles_an_else_if_chain_without_double_wrapping() {
        assert_eq!(
            brace_bare_if_bodies("if (a) x(); else if (b) y(); else z();"),
            "if (a) { x(); } else if (b) { y(); } else { z(); }"
        );
    }

    #[test]
    fn recursively_fixes_a_nested_bare_if_inside_an_already_braced_body() {
        assert_eq!(
            brace_bare_if_bodies("if (a) { if (b) foo(); }"),
            "if (a) { if (b) { foo(); } }"
        );
    }

    #[test]
    fn only_wraps_the_single_statement_not_what_follows() {
        assert_eq!(
            brace_bare_if_bodies("if (a) render(o0);\nosc(60).out()"),
            "if (a) { render(o0); }\nosc(60).out()"
        );
    }

    #[test]
    fn leaves_a_bare_identifier_starting_with_if_alone() {
        let src = "iffy(1,2)";
        assert_eq!(brace_bare_if_bodies(src), src);
    }

    #[test]
    fn leaves_normal_code_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(brace_bare_if_bodies(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"if (a) return 1;\") // if (a) return 1;";
        assert_eq!(brace_bare_if_bodies(src), src);
    }

    #[test]
    fn condition_containing_brackets_is_matched_correctly() {
        assert_eq!(
            brace_bare_if_bodies("if (a.fft[0] > 0.4) return 0.1;"),
            "if (a.fft[0] > 0.4) { return 0.1; }"
        );
    }
}
