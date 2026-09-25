//! Rewrites a JS destructuring declaration (`const { a, b } = EXPR;` /
//! `let [x, y] = EXPR;`) into a plain `let NAME = ();` per bound name,
//! discarding `EXPR` entirely. Rhai's own `let`/`const` bind exactly one
//! plain identifier - no destructuring-pattern target at all ("Expecting
//! name of a variable" the instant the parser sees `{`/`[` where a name
//! was expected).
//!
//! By far the most common real shape is extracting named exports from a
//! dynamically-imported module (`const { sculptToHydraRenderer } = await
//! import("https://...")`) - `import`/`await` are already stripped as bare
//! keywords elsewhere in the pipeline, so `EXPR` here is never anything
//! meaningful regardless of how the destructuring target itself is
//! handled: hydra-rust has no dynamic module loading (see `loadScript`'s
//! own no-op treatment), so whatever names would have been extracted were
//! always going to be undefined. Binding each one to `()` - the same
//! "undefined JS value" stand-in used throughout this pipeline - means a
//! later real reference to it fails as an ordinary "Function not
//! found"/"unsupported operation" instead of a hard parse error that
//! stops the whole script, exactly like `loadScript`'s own no-op treatment
//! unblocks whatever real hydra content follows it.
//!
//! `EXPR` is discarded unevaluated rather than assigned to a temporary and
//! indexed/property-accessed to genuinely extract real values - safer for
//! the dominant case above (indexing into whatever a stripped `import(...)`
//! call degrades into would just be a *different*, still-broken shape),
//! at the cost of not correctly supporting the rarer case where `EXPR` is
//! a real, meaningful call (`const [x, y] = orbitWithNoise(...)`) - `x`/
//! `y` become `()` instead of their real values there, a real (small,
//! deliberately accepted) loss of fidelity rather than a crash.
//!
//! Deliberately narrow, staying provably safe rather than clever: only a
//! flat pattern of plain identifiers is recognized - a shorthand property
//! (`{ a }`), a renamed one (`{ a: b }`, binds `b`), a rest element (`{
//! ...rest }` / `[...rest]`, binds `rest`), or an array element (`[a, b]`,
//! empty holes skipped). A default value (`{ a = 1 }`) or a nested pattern
//! (`{ a: { b } }`) bails out entirely rather than guessing, leaving that
//! (rare) shape as the parse error it already was.
//!
//! Runs after `asi`: the destructuring pattern's own `{`/`[` keeps `asi`'s
//! bracket-depth tracking correctly above zero while scanning across it
//! (no ambiguity like `ifstmt.rs`'s own `if`-header problem - a pattern
//! that hasn't closed yet never looks like a "complete line" on its own),
//! so by this point the whole declaration - pattern, `=`, and value - is
//! already properly `;`-terminated, either by the author or by `asi`,
//! making "scan to the next top-level `;`" a reliable way to find where
//! the value expression (about to be discarded) ends.

use crate::srcscan::mask_strings_and_comments;

pub fn rewrite_destructuring_declarations(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());

    let mut i = 0;
    while i < n {
        if !mask[i]
            && is_ident_start(chars[i])
            && !preceded_by_ident_char(&chars, i)
            && let Some((rewritten, after)) = try_rewrite(&chars, &mask, i, n)
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

fn try_rewrite(chars: &[char], mask: &[bool], i: usize, end: usize) -> Option<(String, usize)> {
    let word_end = ident_end(chars, i);
    let word: String = chars[i..word_end].iter().collect();
    if word != "let" && word != "const" {
        return None;
    }
    let mut j = word_end;
    skip_ws(chars, mask, &mut j, end);

    let names = match chars.get(j) {
        Some('{') => parse_object_pattern(chars, mask, j, end),
        Some('[') => parse_array_pattern(chars, mask, j, end),
        _ => return None,
    }?;
    let close = matching_close(chars, mask, j, end)?;
    let mut k = close + 1;
    skip_ws(chars, mask, &mut k, end);
    if chars.get(k) != Some(&'=') || matches!(chars.get(k + 1), Some('=') | Some('>')) {
        return None;
    }
    k += 1;
    let stmt_end = scan_statement_end(chars, mask, k, end);

    if names.is_empty() {
        return Some((String::new(), stmt_end));
    }
    let rewritten = names.iter().map(|n| format!("let {n} = ();")).collect::<Vec<_>>().join(" ");
    Some((rewritten, stmt_end))
}

/// Parses `{ a, b: c, ...rest }` starting at the `{` in `j`, returning the
/// bound local names. `None` if it contains anything other than a plain
/// shorthand/rename/rest element (a default value or nested pattern).
fn parse_object_pattern(chars: &[char], mask: &[bool], open: usize, end: usize) -> Option<Vec<String>> {
    let close = matching_close(chars, mask, open, end)?;
    let mut names = Vec::new();
    let mut j = open + 1;
    loop {
        skip_ws(chars, mask, &mut j, close);
        if j >= close {
            break;
        }
        if chars.get(j) == Some(&'.') && chars.get(j + 1) == Some(&'.') && chars.get(j + 2) == Some(&'.') {
            j += 3;
            skip_ws(chars, mask, &mut j, close);
        }
        let name_start = j;
        while j < close && !mask[j] && is_ident_char(chars[j]) {
            j += 1;
        }
        if j == name_start {
            return None;
        }
        let mut local = chars[name_start..j].iter().collect::<String>();
        skip_ws(chars, mask, &mut j, close);
        if chars.get(j) == Some(&':') {
            j += 1;
            skip_ws(chars, mask, &mut j, close);
            let rename_start = j;
            while j < close && !mask[j] && is_ident_char(chars[j]) {
                j += 1;
            }
            if j == rename_start {
                return None;
            }
            local = chars[rename_start..j].iter().collect::<String>();
            skip_ws(chars, mask, &mut j, close);
        }
        names.push(local);
        match chars.get(j) {
            Some(',') => {
                j += 1;
                continue;
            }
            None => break,
            _ if j >= close => break,
            _ => return None, // a default value or nested pattern - bail
        }
    }
    Some(names)
}

/// Parses `[a, b, ...rest]` starting at the `[` in `j`, returning the
/// bound names (holes skipped). `None` if any element isn't a plain
/// identifier (a default value or nested pattern).
fn parse_array_pattern(chars: &[char], mask: &[bool], open: usize, end: usize) -> Option<Vec<String>> {
    let close = matching_close(chars, mask, open, end)?;
    let mut names = Vec::new();
    let mut j = open + 1;
    loop {
        skip_ws(chars, mask, &mut j, close);
        if j >= close {
            break;
        }
        if chars.get(j) == Some(&',') {
            j += 1; // a hole - skip
            continue;
        }
        if chars.get(j) == Some(&'.') && chars.get(j + 1) == Some(&'.') && chars.get(j + 2) == Some(&'.') {
            j += 3;
            skip_ws(chars, mask, &mut j, close);
        }
        let name_start = j;
        while j < close && !mask[j] && is_ident_char(chars[j]) {
            j += 1;
        }
        if j == name_start {
            return None;
        }
        names.push(chars[name_start..j].iter().collect::<String>());
        skip_ws(chars, mask, &mut j, close);
        match chars.get(j) {
            Some(',') => {
                j += 1;
                continue;
            }
            None => break,
            _ if j >= close => break,
            _ => return None,
        }
    }
    Some(names)
}

/// Scans a value expression from `j` up to (and including) the next
/// top-level `;`, or to `end` if there isn't one.
fn scan_statement_end(chars: &[char], mask: &[bool], j: usize, end: usize) -> usize {
    let mut depth = 0i32;
    let mut i = j;
    while i < end {
        if !mask[i] {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ';' if depth == 0 => return i + 1,
                _ => {}
            }
        }
        i += 1;
    }
    end
}

fn matching_close(chars: &[char], mask: &[bool], open_idx: usize, end: usize) -> Option<usize> {
    let open = chars[open_idx];
    let close = match open {
        '{' => '}',
        '[' => ']',
        _ => return None,
    };
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < end {
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

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn preceded_by_ident_char(chars: &[char], i: usize) -> bool {
    i > 0 && is_ident_char(chars[i - 1])
}

fn ident_end(chars: &[char], start: usize) -> usize {
    let mut j = start;
    while j < chars.len() && is_ident_char(chars[j]) {
        j += 1;
    }
    j
}

fn skip_ws(chars: &[char], mask: &[bool], j: &mut usize, end: usize) {
    while *j < end && (chars[*j].is_whitespace() || mask[*j]) {
        *j += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite_destructuring_declarations;

    #[test]
    fn rewrites_a_shorthand_object_destructure_from_import() {
        assert_eq!(
            rewrite_destructuring_declarations(
                "const { sculptToHydraRenderer } = (\"https://example.com/x.js\");"
            ),
            "let sculptToHydraRenderer = ();"
        );
    }

    #[test]
    fn rewrites_multiple_shorthand_names() {
        assert_eq!(
            rewrite_destructuring_declarations("const { a, b, c } = strudel;"),
            "let a = (); let b = (); let c = ();"
        );
    }

    #[test]
    fn rewrites_a_renamed_property() {
        assert_eq!(
            rewrite_destructuring_declarations("const { a: renamed } = strudel;"),
            "let renamed = ();"
        );
    }

    #[test]
    fn rewrites_a_rest_element() {
        assert_eq!(
            rewrite_destructuring_declarations("const { src, shape, ...otherControls } = strudel.controls;"),
            "let src = (); let shape = (); let otherControls = ();"
        );
    }

    #[test]
    fn rewrites_an_array_pattern() {
        assert_eq!(
            rewrite_destructuring_declarations("const [x, y] = orbitWithNoise(1, 2, 3);"),
            "let x = (); let y = ();"
        );
    }

    #[test]
    fn skips_holes_in_an_array_pattern() {
        assert_eq!(
            rewrite_destructuring_declarations("const [, y] = pair();"),
            "let y = ();"
        );
    }

    #[test]
    fn discards_a_multiline_value_expression() {
        assert_eq!(
            rewrite_destructuring_declarations("let {\n  a,\n  b\n} = foo(\n  1,\n  2\n);\nosc(60)"),
            "let a = (); let b = ();\nosc(60)"
        );
    }

    #[test]
    fn falls_back_to_end_of_input_without_trailing_semicolon() {
        assert_eq!(
            rewrite_destructuring_declarations("const { a } = strudel"),
            "let a = ();"
        );
    }

    #[test]
    fn leaves_a_default_valued_element_alone() {
        // a real, not-yet-handled gap - see the module doc comment.
        let src = "const { a = 1 } = strudel;";
        assert_eq!(rewrite_destructuring_declarations(src), src);
    }

    #[test]
    fn leaves_a_nested_pattern_alone() {
        let src = "const { a: { b } } = strudel;";
        assert_eq!(rewrite_destructuring_declarations(src), src);
    }

    #[test]
    fn leaves_a_plain_declaration_alone() {
        let src = "let x = 5;";
        assert_eq!(rewrite_destructuring_declarations(src), src);
    }

    #[test]
    fn leaves_normal_code_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(rewrite_destructuring_declarations(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"const { a } = b;\") // const { a } = b;";
        assert_eq!(rewrite_destructuring_declarations(src), src);
    }

    #[test]
    fn does_not_misfire_on_identifiers_starting_with_let_or_const() {
        let src = "letters = 1;\nconstant = 2;";
        assert_eq!(rewrite_destructuring_declarations(src), src);
    }
}
