//! Rewrites JS ternary expressions (`cond ? a : b`) into Rhai's `if`/`else`
//! expression form (`if cond { a } else { b }`). Rhai has no `?:` operator
//! at all ("Unknown operator: '?'"); `if`/`else` blocks are valid Rhai
//! expressions (they evaluate to their last statement's value), so this is
//! a direct, mechanical translation.
//!
//! Boundaries are found structurally rather than by parsing full expression
//! grammar: a top-level `,`, `;`, or bare `=` marks where a condition/branch
//! starts or ends, and brackets are recursed into so nested calls
//! (`osc(cond?a:b)`) and chained ternaries (`a?b:c?d:e`, right-associative)
//! are each handled independently.

use crate::srcscan::mask_strings_and_comments;

pub fn rewrite_ternaries(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    transform(&chars, &mask, 0, chars.len())
}

fn is_bare_equals(chars: &[char], i: usize) -> bool {
    if chars[i] != '=' {
        return false;
    }
    if matches!(chars.get(i + 1), Some('=') | Some('>')) {
        return false; // ==, =>
    }
    if i > 0 && matches!(chars[i - 1], '=' | '!' | '<' | '>' | '+' | '-' | '*' | '/' | '%') {
        return false; // !=, <=, >=, +=, -=, *=, /=, %=
    }
    true
}

/// If `s` (after trimming leading whitespace) starts with the whole word
/// `return`, returns the text following it (unstripped of its own leading
/// whitespace). Used so `return cond?a:b` becomes `return if cond {a} else
/// {b}` rather than the malformed `if return cond {a} else {b}` - `return`
/// isn't a `,`/`;`/`=` boundary, so without this it gets swept into the
/// ternary's condition text along with everything after it.
fn strip_return_prefix(s: &str) -> Option<&str> {
    let rest = s.trim_start().strip_prefix("return")?;
    let boundary = rest.chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
    boundary.then_some(rest)
}

/// If `s` (after trimming leading whitespace) starts with an arrow
/// function's header - `(params) =>` or a bare `ident =>` - returns the
/// header text (params/ident plus the `=>` itself) and the remainder.
/// Same idea as `strip_return_prefix`: a named arrow-function assignment
/// (`let pick = (x) => x>0.5 ? 1 : 0`, handled by `arrowfn` once this pass
/// is done) has no `,`/`;`/bare-`=` boundary between its header and body,
/// so without this the whole `(x) =>` gets swept into the ternary's
/// condition, producing the malformed `if (x) => x>0.5 {1} else {0}`
/// instead of `(x) => if x>0.5 {1} else {0}`.
fn strip_arrow_header(s: &str) -> Option<(String, String)> {
    let chars: Vec<char> = s.trim_start().chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();

    let mut j;
    if chars.first() == Some(&'(') {
        let close = matching_close(&chars, &mask, 0);
        if close >= n {
            return None;
        }
        j = close + 1;
    } else if chars.first().is_some_and(|c| c.is_alphabetic() || *c == '_') {
        j = 0;
        while j < n && (chars[j].is_alphanumeric() || chars[j] == '_') {
            j += 1;
        }
    } else {
        return None;
    }

    while j < n && (chars[j].is_whitespace() || mask[j]) {
        j += 1;
    }
    if chars.get(j) != Some(&'=') || chars.get(j + 1) != Some(&'>') {
        return None;
    }
    j += 2;
    Some((chars[..j].iter().collect(), chars[j..].iter().collect()))
}

fn matching_close(chars: &[char], mask: &[bool], open_idx: usize) -> usize {
    let open = chars[open_idx];
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => unreachable!(),
    };
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < chars.len() {
        if !mask[i] {
            if chars[i] == open {
                depth += 1;
            } else if chars[i] == close {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
        }
        i += 1;
    }
    chars.len()
}

/// Copies a bracketed group verbatim into `out`, recursively transforming
/// its contents. Returns the index just past the closing bracket.
fn copy_bracket(chars: &[char], mask: &[bool], i: usize, end: usize, out: &mut String) -> usize {
    let close = matching_close(chars, mask, i).min(end);
    out.push(chars[i]);
    out.push_str(&transform(chars, mask, i + 1, close));
    if close < end && close < chars.len() {
        out.push(chars[close]);
        close + 1
    } else {
        close
    }
}

fn transform(chars: &[char], mask: &[bool], start: usize, end: usize) -> String {
    let mut out = String::new();
    let mut seg = String::new();
    let mut i = start;

    while i < end {
        if mask[i] {
            seg.push(chars[i]);
            i += 1;
            continue;
        }
        match chars[i] {
            '(' | '[' | '{' => {
                i = copy_bracket(chars, mask, i, end, &mut seg);
            }
            ',' | ';' => {
                out.push_str(&seg);
                out.push(chars[i]);
                seg.clear();
                i += 1;
            }
            '=' if is_bare_equals(chars, i) => {
                out.push_str(&seg);
                out.push('=');
                seg.clear();
                i += 1;
            }
            '?' if chars.get(i + 1) != Some(&'?') && (i == 0 || chars[i - 1] != '?') => {
                let cond = std::mem::take(&mut seg);
                i += 1;
                let (then_text, next) = scan_branch(chars, mask, i, end, true);
                i = next;
                let (else_text, next) = scan_branch(chars, mask, i, end, false);
                i = next;
                let (arrow_prefix, cond) = match strip_arrow_header(&cond) {
                    Some((header, rest)) => (header, rest),
                    None => (String::new(), cond),
                };
                let (return_prefix, cond) = match strip_return_prefix(&cond) {
                    Some(rest) => ("return ", rest.to_string()),
                    None => ("", cond),
                };
                // Re-run on each extracted branch: catches chained ternaries
                // in the else branch (`a?b:c?d:e`) and any left unconverted
                // because they weren't inside a bracket `scan_branch` recursed into.
                let cond = rewrite_ternaries(cond.trim());
                let then_text = rewrite_ternaries(then_text.trim());
                let else_text = rewrite_ternaries(else_text.trim());
                out.push_str(&format!("{arrow_prefix}{return_prefix}if {cond} {{ {then_text} }} else {{ {else_text} }}"));
            }
            _ => {
                seg.push(chars[i]);
                i += 1;
            }
        }
    }
    out.push_str(&seg);
    out
}

/// Scans a ternary branch: the "then" branch ends at a top-level `:`; the
/// "else" branch ends at a top-level `,`/`;`/bare `=`/end-of-range, or (see
/// `ends_branch_at_newline`) a bare newline that looks like a genuine
/// statement boundary rather than a mid-expression line break. Brackets are
/// recursed into (via `copy_bracket`) so nested ternaries inside them are
/// transformed independently. Returns the branch text and the index just
/// past its terminator (for "then", past the `:`; for "else", at the
/// terminator itself, unconsumed).
fn scan_branch(chars: &[char], mask: &[bool], start: usize, end: usize, stop_at_colon: bool) -> (String, usize) {
    let mut buf = String::new();
    let mut i = start;
    while i < end {
        if mask[i] {
            buf.push(chars[i]);
            i += 1;
            continue;
        }
        match chars[i] {
            '(' | '[' | '{' => {
                i = copy_bracket(chars, mask, i, end, &mut buf);
            }
            ':' if stop_at_colon => {
                return (buf, i + 1);
            }
            ',' | ';' if !stop_at_colon => {
                return (buf, i);
            }
            '=' if !stop_at_colon && is_bare_equals(chars, i) => {
                return (buf, i);
            }
            '\n' if !stop_at_colon && ends_branch_at_newline(chars, mask, &buf, i, end) => {
                return (buf, i);
            }
            _ => {
                buf.push(chars[i]);
                i += 1;
            }
        }
    }
    (buf, i)
}

/// True if a bare newline at `i` looks like a genuine statement boundary
/// rather than a mid-expression line break - i.e. neither the branch text
/// scanned so far nor the next line's first significant character looks
/// like a continuation. Without this, an else-branch with no explicit
/// `,`/`;`/`=` terminator anywhere later in the file (a bare, un-asi'd
/// `hh=height>width?width/height:1` with nothing after it but a newline)
/// swallows the *entire rest of the file* into its own branch text, since
/// the only terminators this scan otherwise recognizes never occur again.
/// Mirrors `asi::insert_missing_semicolons`'s own "does this line
/// genuinely end here" heuristic (same character sets), since this pass
/// runs *before* `asi` and can't rely on a `;` already being there.
fn ends_branch_at_newline(chars: &[char], mask: &[bool], buf: &str, i: usize, end: usize) -> bool {
    let last_significant = buf.trim_end().chars().next_back();
    if last_significant.is_some_and(continues_branch_line) {
        return false;
    }
    let mut j = i + 1;
    while j < end && (chars[j].is_whitespace() || mask[j]) {
        j += 1;
    }
    match chars.get(j).filter(|_| j < end) {
        None => false,
        Some(c) => !continues_branch_line(*c),
    }
}

/// Same character set on both sides of a line break - `asi.rs`'s
/// `continues_line`/`continues_next_line` are two different (if mostly
/// overlapping) sets since a full statement has more shapes than a bare
/// ternary branch does; a branch is just a value expression, so one set
/// suffices here.
fn continues_branch_line(c: char) -> bool {
    matches!(
        c,
        '.' | ')' | ']' | '}' | ',' | '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | ':' | '?'
    )
}

#[cfg(test)]
mod tests {
    use super::rewrite_ternaries;

    #[test]
    fn rewrites_simple_ternary() {
        assert_eq!(rewrite_ternaries("time>1?0.1:0.2"), "if time>1 { 0.1 } else { 0.2 }");
    }

    #[test]
    fn rewrites_ternary_as_function_argument() {
        assert_eq!(
            rewrite_ternaries("osc(60,time>1?0.1:0.2,0)"),
            "osc(60,if time>1 { 0.1 } else { 0.2 },0)"
        );
    }

    #[test]
    fn rewrites_ternary_in_assignment() {
        // no space is inserted between `=` and `if` (the original spacing
        // is discarded along with the trimmed condition text), but this is
        // still valid Rhai - the tokenizer doesn't need whitespace there.
        assert_eq!(rewrite_ternaries("x = cond ? a : b"), "x =if cond { a } else { b }");
    }

    #[test]
    fn leaves_non_ternary_code_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(rewrite_ternaries(src), src);
    }

    #[test]
    fn handles_nested_call_in_branches() {
        assert_eq!(
            rewrite_ternaries("cond?osc(60):noise(4)"),
            "if cond { osc(60) } else { noise(4) }"
        );
    }

    #[test]
    fn handles_ternary_nested_in_call_inside_branch() {
        assert_eq!(
            rewrite_ternaries("cond?foo(inner?1:2):3"),
            "if cond { foo(if inner { 1 } else { 2 }) } else { 3 }"
        );
    }

    #[test]
    fn handles_chained_ternary_in_else() {
        assert_eq!(
            rewrite_ternaries("a?1:b?2:3"),
            "if a { 1 } else { if b { 2 } else { 3 } }"
        );
    }

    #[test]
    fn moves_return_keyword_outside_if() {
        assert_eq!(
            rewrite_ternaries("return rn()>x?1:-1"),
            "return if rn()>x { 1 } else { -1 }"
        );
    }

    #[test]
    fn leaves_return_of_non_ternary_alone() {
        let src = "return foo(x)";
        assert_eq!(rewrite_ternaries(src), src);
    }

    #[test]
    fn moves_parenthesized_arrow_header_outside_if() {
        // regression test: a named arrow-function assignment
        // (`let pick = (x) => ...`, converted to a real `fn` by `arrowfn`
        // later in the pipeline) has no `,`/`;`/bare-`=` boundary between
        // its `(params) =>` header and body, so without this the whole
        // header got swept into the ternary's condition.
        assert_eq!(
            rewrite_ternaries("(x) => x>0.5 ? 1 : 0"),
            "(x) =>if x>0.5 { 1 } else { 0 }"
        );
    }

    #[test]
    fn moves_bare_single_param_arrow_header_outside_if() {
        assert_eq!(rewrite_ternaries("x => x>0 ? 1 : -1"), "x =>if x>0 { 1 } else { -1 }");
    }

    #[test]
    fn moves_multi_param_arrow_header_outside_if() {
        assert_eq!(
            rewrite_ternaries("(a,b) => a>b ? a : b"),
            "(a,b) =>if a>b { a } else { b }"
        );
    }

    #[test]
    fn does_not_misfire_on_a_plain_parenthesized_condition() {
        // `(a) > b` is not an arrow header - no `=>` follows the `)`.
        let src = "(a)>b?1:0";
        assert_eq!(rewrite_ternaries(src), "if (a)>b { 1 } else { 0 }");
    }

    #[test]
    fn does_not_misfire_on_identifier_starting_with_return() {
        assert_eq!(
            rewrite_ternaries("returnValue?a:b"),
            "if returnValue { a } else { b }"
        );
    }

    #[test]
    fn does_not_treat_arrow_as_bare_equals() {
        // the `=` in `=>` must not itself be mistaken for a boundary-
        // resetting bare assignment: without the fix, the segment gets
        // flushed mid-arrow, leaving a stray `>` glued onto `cond` instead
        // of keeping `x=>cond` intact. Combined with `strip_arrow_header`
        // (which then correctly moves the `x=>` part outside the `if`),
        // the end result is fully valid Rhai - not just "less broken".
        assert_eq!(
            rewrite_ternaries("pat=x=>cond?t:f"),
            "pat=x=>if cond { t } else { f }"
        );
    }

    #[test]
    fn stops_else_branch_at_a_bare_trailing_newline() {
        // a top-level assignment with no `,`/`;`/`=` anywhere later in the
        // file (the common case, since `asi` hasn't run yet at this point
        // in the pipeline) must not swallow the rest of the file into the
        // else branch's own text.
        assert_eq!(
            rewrite_ternaries("hh=height>width?width/height:1\nosc(60).out()"),
            "hh=if height>width { width/height } else { 1 }\nosc(60).out()"
        );
    }

    #[test]
    fn leaves_a_genuine_multiline_chain_continuation_in_the_else_branch_alone() {
        // the next line starts with `.` - a continuation, not a new
        // statement - so the branch must keep scanning past the newline.
        assert_eq!(
            rewrite_ternaries("cond?a:b\n  .out()"),
            "if cond { a } else { b\n  .out() }"
        );
    }

    #[test]
    fn leaves_a_dangling_operator_at_end_of_line_in_the_else_branch_alone() {
        // the branch text itself ends in `+` - clearly not done yet,
        // regardless of what the next line starts with.
        assert_eq!(
            rewrite_ternaries("cond?a:b+\n1"),
            "if cond { a } else { b+\n1 }"
        );
    }

    #[test]
    fn ignores_double_question_mark() {
        // not a real Rhai operator, but make sure we don't misfire on it
        let src = "x??y";
        assert_eq!(rewrite_ternaries(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"a?b:c\") // a?b:c";
        assert_eq!(rewrite_ternaries(src), src);
    }
}
