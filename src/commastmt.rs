//! Rewrites a bare (non-parenthesized) top-level sequence of comma-joined
//! statements (`loadScript(a), loadScript(b), setResolution(w,h)`) into
//! semicolon-separated Rhai statements. Real sketches commonly load a
//! community extension this way - `await loadScript(url1), await
//! loadScript(url2), setResolution(w, h), canvas.setRelativeSize(1), ...`
//! is a specific, widely-copy-pasted boilerplate line (the `hyper-hydra`/
//! `geikha` canvas extension's setup snippet) - because JS allows a
//! comma-expression as a standalone statement (each operand runs in order;
//! only the last value would matter if it were used, which it isn't
//! here), but Rhai has no comma operator at all. This is a different
//! grammatical position from `commaexpr.rs`'s own comma handling: that
//! module collapses a comma-tuple used *as a value* (`shape(4, (0.01,
//! 0.2), 1)`) down to its last term; this one splits a comma chain used
//! *as a sequence of statements* into separate ones, since there's no
//! surrounding expression context for any single value to matter to.
//!
//! Scoped deliberately narrow, to stay provably safe rather than clever:
//! only touches a comma at true top-level bracket depth - outside every
//! `(`/`[`/`{` in the entire script - since any function call's own
//! argument-list commas are inherently nested one level inside that
//! call's own parens, and any array/object-literal's commas are nested
//! inside its own brackets/braces. A depth-0 comma can therefore only ever
//! be this comma-operator idiom, with one exception this pass explicitly
//! excludes: a `let`/`const` multi-variable declarator list (`let a=1,
//! b=2`) is *also* a bare depth-0 comma, but needs each declarator to
//! repeat the keyword (`let a=1; let b=2;`), not just a punctuation swap -
//! a different, not-yet-handled transform (tracked as a known gap, see
//! README.md), so this pass leaves any comma between `let`/`const` and its
//! next terminating `;` alone rather than guessing.
//!
//! Runs before `asi`: by the time `asi` sees this, each comma-joined call
//! is already its own semicolon-terminated statement, leaving `asi` only
//! its ordinary job of possibly adding one final trailing `;`, rather than
//! having to somehow guess where a bare comma chain's implied statement
//! boundaries are.

use crate::srcscan::mask_strings_and_comments;

pub fn rewrite_top_level_comma_statements(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());

    let mut depth: i32 = 0;
    let mut in_declarator_list = false;
    let mut i = 0;
    while i < n {
        if mask[i] {
            out.push(chars[i]);
            i += 1;
            continue;
        }

        if depth == 0 && !in_declarator_list && starts_declarator_keyword(&chars, i) {
            in_declarator_list = true;
        }

        match chars[i] {
            ',' if depth == 0
                && (!in_declarator_list || comma_exits_declarator_list(&chars, &mask, i + 1)) =>
            {
                out.push(';');
                i += 1;
            }
            ';' if depth == 0 => {
                in_declarator_list = false;
                out.push(chars[i]);
                i += 1;
            }
            c => {
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    _ => {}
                }
                out.push(c);
                i += 1;
            }
        }
    }

    out
}

/// True if `let` or `const` (whole-word) starts right at `i` - `var` is
/// deliberately not checked, since `jskeywords::rewrite_keywords` (an
/// earlier pipeline step) has already rewritten every `var` to `let` by
/// the time this pass runs.
fn starts_declarator_keyword(chars: &[char], i: usize) -> bool {
    ["let", "const"].iter().any(|kw| {
        let end = i + kw.len();
        end <= chars.len()
            && chars[i..end].iter().collect::<String>() == *kw
            && (i == 0 || !is_ident_char(chars[i - 1]))
            && chars.get(end).is_none_or(|c| !is_ident_char(*c))
    })
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True if a depth-0 comma immediately followed (at `j`, skipping
/// whitespace/comments) by this content can *not* be a continuation of an
/// already-open `let`/`const` declarator list - i.e. the comma must split
/// into `;` even though `in_declarator_list` is currently set. Two distinct
/// ways this happens, both stemming from the same root cause: `autolet`
/// (an earlier pipeline step) is comma-agnostic and inserts `let` before
/// *every* bare assignment it finds, regardless of what separates it from
/// the previous one - so a chain that was never a real declarator list at
/// all can still end up *looking* like one by the time this pass runs:
/// - The next token is itself a fresh `let`/`const` (`let rn=1,let a=2`) -
///   a repeated keyword can only mean a separate, independent statement,
///   since a genuine multi-declarator list never repeats it.
/// - The next token isn't even a plausible declarator name at all - not a
///   bare identifier, or an identifier immediately followed by `(`/`.`
///   (a call or a property-path access, e.g. `let S=0.5,0.1` or
///   `let a=1,foo()` - `autolet` only prefixes bare *assignments*, so a
///   literal, call, or member access after the comma was never turned
///   into its own declarator and was never part of one to begin with).
fn comma_exits_declarator_list(chars: &[char], mask: &[bool], mut j: usize) -> bool {
    while j < chars.len() && !mask[j] && chars[j].is_whitespace() {
        j += 1;
    }
    if starts_declarator_keyword(chars, j) {
        return true;
    }
    if !chars.get(j).is_some_and(|c| is_ident_start(*c)) {
        return true;
    }
    let mut k = j;
    while k < chars.len() && is_ident_char(chars[k]) {
        k += 1;
    }
    while k < chars.len() && !mask[k] && chars[k].is_whitespace() {
        k += 1;
    }
    !matches!(chars.get(k), None | Some(',' | ';' | '='))
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::rewrite_top_level_comma_statements;

    #[test]
    fn splits_a_bare_top_level_comma_chain_into_statements() {
        assert_eq!(
            rewrite_top_level_comma_statements(
                "loadScript(\"http://example.com/a.js\"),loadScript(\"http://example.com/b.js\"),setResolution(800,600)"
            ),
            "loadScript(\"http://example.com/a.js\");loadScript(\"http://example.com/b.js\");setResolution(800,600)"
        );
    }

    #[test]
    fn splits_a_chain_that_also_includes_a_method_call() {
        assert_eq!(
            rewrite_top_level_comma_statements("loadScript(\"a\"),canvas.setRelativeSize(1),canvas.foo()"),
            "loadScript(\"a\");canvas.setRelativeSize(1);canvas.foo()"
        );
    }

    #[test]
    fn leaves_a_function_calls_own_argument_commas_alone() {
        let src = "osc(60, 0.1, 0).out()";
        assert_eq!(rewrite_top_level_comma_statements(src), src);
    }

    #[test]
    fn leaves_a_nested_array_or_object_literals_commas_alone() {
        let src = "osc(60, [1,2,3], 0).out()";
        assert_eq!(rewrite_top_level_comma_statements(src), src);
    }

    #[test]
    fn leaves_a_let_multi_declarator_list_alone() {
        // a different, not-yet-handled transform - see the module doc
        // comment. Left untouched (still a parse error afterward), not
        // made worse.
        let src = "let a=1, b=2;";
        assert_eq!(rewrite_top_level_comma_statements(src), src);
    }

    #[test]
    fn resumes_normal_handling_after_a_declarator_lists_own_semicolon() {
        assert_eq!(
            rewrite_top_level_comma_statements("let a=1, b=2;loadScript(\"a\"),loadScript(\"b\")"),
            "let a=1, b=2;loadScript(\"a\");loadScript(\"b\")"
        );
    }

    #[test]
    fn leaves_a_const_multi_declarator_list_alone() {
        let src = "const a=1, b=2;";
        assert_eq!(rewrite_top_level_comma_statements(src), src);
    }

    #[test]
    fn splits_several_independently_let_prefixed_assignments_still_comma_joined() {
        // Shape produced when an earlier pass (`autolet`) has already
        // prefixed each bare assignment in an originally comma-joined
        // chain with its own repeated `let` - each repetition signals a
        // separate statement, not a continuation of one declarator list.
        assert_eq!(
            rewrite_top_level_comma_statements("canvas.setLinear(),let rn=1,let a=2,let dx=rn()"),
            "canvas.setLinear();let rn=1;let a=2;let dx=rn()"
        );
    }

    #[test]
    fn splits_a_bare_literal_after_a_single_let_prefixed_assignment() {
        // `S=0.5,0.1` (JS's comma operator: assign, then a discarded bare
        // literal) - once `autolet` has prefixed the first operand with
        // `let`, `0.1` isn't a plausible declarator name at all, so this
        // can't be a genuine multi-declarator continuation.
        assert_eq!(
            rewrite_top_level_comma_statements("let S=0.5,0.1"),
            "let S=0.5;0.1"
        );
    }

    #[test]
    fn splits_a_call_after_a_single_let_prefixed_assignment() {
        // `a=1,foo()` - `foo` starts like an identifier but is immediately
        // followed by `(`, so it's a call, not a fresh declarator name.
        assert_eq!(
            rewrite_top_level_comma_statements("let a=1,foo()"),
            "let a=1;foo()"
        );
    }

    #[test]
    fn splits_a_property_access_after_a_single_let_prefixed_assignment() {
        assert_eq!(
            rewrite_top_level_comma_statements("let a=1,foo.bar"),
            "let a=1;foo.bar"
        );
    }

    #[test]
    fn does_not_misfire_on_identifiers_starting_with_let_or_const() {
        // word-boundary check: `letters`/`constant` are not the `let`/
        // `const` keyword.
        assert_eq!(
            rewrite_top_level_comma_statements("letters=1,constant=2"),
            "letters=1;constant=2"
        );
    }

    #[test]
    fn leaves_normal_code_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(rewrite_top_level_comma_statements(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"a,b\") // a,b";
        assert_eq!(rewrite_top_level_comma_statements(src), src);
    }
}
