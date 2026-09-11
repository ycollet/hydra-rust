//! Rewrites JS `for` loops into their Rhai equivalents. Rhai's own `for`
//! loop is iterator-based only (`for x in expr { ... }`), with no
//! three-clause C-style form at all:
//!
//! - `for (let x of/in expr) { body }` maps almost directly:
//!   `for x in expr { body }`.
//! - `for (init; cond; update) { body }` has no direct Rhai equivalent,
//!   so it's desugared into a `while`, wrapped in a block so the loop
//!   variable stays scoped to it the way JS's `let` in a for-header is:
//!   `{ init; while cond { body update; } }`. An empty `cond` (`for(;;)`)
//!   becomes `true` (JS's own "no condition" semantics).
//!
//! A loop body without `{ }` (`for (...) stmt;`, valid JS, seen in real
//! sketches) is normalized to a block either way.
//!
//! Runs early (right after `iife`), before `autolet`: a bare (no `let`)
//! init clause (`for(i=0;i<n;i++)`, a common real shape) needs to reach
//! `autolet` as an ordinary top-level assignment so it gets its own `let`
//! inserted, same as it would outside a loop.

use crate::asi;
use crate::srcscan::mask_strings_and_comments;

pub fn rewrite_for_loops(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    transform(&chars, &mask, 0, chars.len())
}

fn transform(chars: &[char], mask: &[bool], start: usize, end: usize) -> String {
    let mut out = String::new();
    let mut i = start;

    while i < end {
        if mask[i] {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if chars[i] == 'f'
            && matches_word(chars, mask, i, "for")
            && !preceded_by_ident_char(chars, i)
            && let Some((rewritten, after)) = try_rewrite_for(chars, mask, i, end)
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

fn try_rewrite_for(chars: &[char], mask: &[bool], i: usize, end: usize) -> Option<(String, usize)> {
    let mut j = i + 3; // past "for"
    skip_ws(chars, mask, &mut j);
    if chars.get(j) != Some(&'(') {
        return None;
    }
    let header_open = j;
    let header_close = matching_close(chars, mask, header_open, '(', ')', end)?;

    j = header_close + 1;
    let (body_inner, after) = find_body(chars, mask, j, end)?;

    if let Some((name, expr)) = split_for_of_in(chars, mask, header_open + 1, header_close) {
        // Nothing follows the body within the loop's own `{ }` here (a
        // Rhai block doesn't need a trailing `;` on its last statement),
        // so asi can safely run on the body alone.
        let body = asi::insert_missing_semicolons(&body_inner);
        return Some((format!("for {name} in {expr} {{ {body} }}"), after));
    }

    let (init, cond, update) = split_c_style(chars, mask, header_open + 1, header_close)?;
    let init_stmt = if init.trim().is_empty() { String::new() } else { format!("{init};") };
    let cond_expr = if cond.trim().is_empty() { "true".to_string() } else { cond };
    let update_stmt = if update.trim().is_empty() { String::new() } else { format!("{update};") };
    // Run asi on the body *and* the trailing update statement together, as
    // one unit: asi decides whether the body's last line needs a `;`
    // based on what significant text follows it, so running it on the
    // body alone (with nothing after it yet) would see "end of input" and
    // wrongly conclude no separator is needed - even though update_stmt
    // is about to be appended right after it.
    let body_and_update = asi::insert_missing_semicolons(&format!("{body_inner}\n{update_stmt}"));
    Some((
        format!("{{ {init_stmt} while {cond_expr} {{ {body_and_update} }} }}"),
        after,
    ))
}

/// Splits a `for (...)`-header's content on its two top-level `;`
/// (bracket-depth-aware), the three-clause C-style form. `None` if there
/// aren't exactly two (so it isn't this form at all).
fn split_c_style(chars: &[char], mask: &[bool], start: usize, end: usize) -> Option<(String, String, String)> {
    let mut depth = 0i32;
    let mut semis = Vec::new();
    let mut i = start;
    while i < end {
        if !mask[i] {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ';' if depth == 0 => semis.push(i),
                _ => {}
            }
        }
        i += 1;
    }
    if semis.len() != 2 {
        return None;
    }
    let init: String = chars[start..semis[0]].iter().collect();
    let cond: String = chars[semis[0] + 1..semis[1]].iter().collect();
    let update: String = chars[semis[1] + 1..end].iter().collect();
    Some((init, cond, update))
}

/// Recognizes the for-of/for-in header shape: an optional `let`/`const`
/// (`var` is already rewritten to `let` by `jskeywords`, which runs
/// earlier), a bare identifier, `of` or `in`, then the iterated
/// expression. Returns `(name, expr)` - both forms compile to Rhai's own
/// `for x in expr`, which iterates values either way.
fn split_for_of_in(chars: &[char], mask: &[bool], start: usize, end: usize) -> Option<(String, String)> {
    let mut j = start;
    skip_ws(chars, mask, &mut j);
    for kw in ["let", "const"] {
        if matches_word(chars, mask, j, kw) {
            j += kw.len();
            skip_ws(chars, mask, &mut j);
            break;
        }
    }
    let name_start = j;
    while j < end && !mask[j] && is_ident_char(chars[j]) {
        j += 1;
    }
    if j == name_start {
        return None;
    }
    let name: String = chars[name_start..j].iter().collect();
    skip_ws(chars, mask, &mut j);
    let matched_kw = ["of", "in"].iter().find(|kw| matches_word(chars, mask, j, kw))?;
    j += matched_kw.len();
    let expr: String = chars[j..end].iter().collect();
    Some((name, expr))
}

/// Finds the loop body starting at (or after whitespace from) `start`:
/// either a `{ ... }` block or a single statement up to the next
/// top-level `;`. Returns the body's own content (recursively
/// transformed, so nested `for` loops are handled too - without its own
/// wrapping braces) and the index just past the whole body.
fn find_body(chars: &[char], mask: &[bool], start: usize, end: usize) -> Option<(String, usize)> {
    let mut j = start;
    skip_ws(chars, mask, &mut j);
    if chars.get(j) == Some(&'{') {
        let close = matching_close(chars, mask, j, '{', '}', end)?;
        return Some((transform(chars, mask, j + 1, close), close + 1));
    }
    let mut depth = 0i32;
    let mut k = j;
    let mut found_semi = false;
    while k < end {
        if !mask[k] {
            match chars[k] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ';' if depth == 0 => {
                    found_semi = true;
                    break;
                }
                _ => {}
            }
        }
        k += 1;
    }
    let inner = transform(chars, mask, j, k);
    let after = if found_semi { k + 1 } else { k };
    Some((format!("{inner};"), after))
}

fn matching_close(chars: &[char], mask: &[bool], open_idx: usize, open: char, close: char, limit: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < limit {
        if !mask[i] {
            if chars[i] == open {
                depth += 1;
            } else if chars[i] == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

/// True if the (whole, word-bounded) keyword `word` starts at `pos`.
fn matches_word(chars: &[char], mask: &[bool], pos: usize, word: &str) -> bool {
    let wchars: Vec<char> = word.chars().collect();
    let wlen = wchars.len();
    if pos + wlen > chars.len() || mask[pos..pos + wlen].iter().any(|m| *m) {
        return false;
    }
    if chars[pos..pos + wlen] != wchars[..] {
        return false;
    }
    !chars.get(pos + wlen).is_some_and(|c| is_ident_char(*c))
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn preceded_by_ident_char(chars: &[char], i: usize) -> bool {
    i > 0 && is_ident_char(chars[i - 1])
}

fn skip_ws(chars: &[char], mask: &[bool], j: &mut usize) {
    while *j < chars.len() && (chars[*j].is_whitespace() || mask[*j]) {
        *j += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite_for_loops;

    #[test]
    fn rewrites_c_style_for_loop_with_block_body() {
        assert_eq!(
            rewrite_for_loops("for(let i=0;i<n;i+=1){ x=i; }"),
            "{ let i=0; while i<n {  x=i; 
i+=1; } }"
        );
    }

    #[test]
    fn rewrites_c_style_for_loop_without_let() {
        assert_eq!(
            rewrite_for_loops("for(i=0;i<q;i+=1){y=i;}"),
            "{ i=0; while i<q { y=i;
i+=1; } }"
        );
    }

    #[test]
    fn rewrites_c_style_for_loop_with_single_statement_body() {
        assert_eq!(
            rewrite_for_loops("for(let i=0;i<n;i+=1)x=i;"),
            "{ let i=0; while i<n { x=i;
i+=1; } }"
        );
    }

    #[test]
    fn rewrites_empty_clauses_as_infinite_loop() {
        assert_eq!(rewrite_for_loops("for(;;){ x=1; }"), "{  while true {  x=1; 
 } }");
    }

    #[test]
    fn rewrites_for_of_loop() {
        assert_eq!(
            rewrite_for_loops("for(let x of arr){ y=x; }"),
            "for x in  arr {  y=x;  }"
        );
    }

    #[test]
    fn rewrites_for_in_loop_without_declaration_keyword() {
        assert_eq!(rewrite_for_loops("for(k in obj){ y=k; }"), "for k in  obj {  y=k;  }");
    }

    #[test]
    fn rewrites_nested_for_loops() {
        assert_eq!(
            rewrite_for_loops("for(i=0;i<n;i+=1){for(j=0;j<n;j+=1){x=i+j;}}"),
            "{ i=0; while i<n { { j=0; while j<n { x=i+j;
j+=1; } };
i+=1; } }"
        );
    }

    #[test]
    fn leaves_non_for_code_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(rewrite_for_loops(src), src);
    }

    #[test]
    fn does_not_misfire_on_identifier_starting_with_for() {
        let src = "format(1,2)";
        assert_eq!(rewrite_for_loops(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"for(i=0;i<3;i++){}\") // for(i=0;i<3;i++){}";
        assert_eq!(rewrite_for_loops(src), src);
    }
}
