//! Handles JS keywords that Rhai reserves but never assigns any meaning to
//! (`var`, `null`, `new`, `await`, `async`) - any bare appearance of one of
//! these is *already* an unconditional hard syntax error in Rhai today, so
//! substituting or dropping them can only ever help or be neutral; it can't
//! turn previously-valid code invalid.
//!
//! - `var` -> `let`: semantically the same declaration keyword for the
//!   flat, non-nested scripts these sketches are.
//! - `null` -> `()`: Rhai's unit value is the closest equivalent to JS's
//!   null/"no value".
//! - `new`, `await`, `async`: dropped entirely. `new Foo(...)` becomes a
//!   plain `Foo(...)` call (still fails with "Function not found" if `Foo`
//!   isn't registered - e.g. `new P5()`, a whole separate JS library with
//!   no Rhai equivalent - but that's a graceful, already-cataloged failure
//!   mode instead of a hard parse stop). `await`/`async` are meaningless in
//!   this synchronous, single-shot engine, so they're just noise to strip;
//!   this alone doesn't add real async/module-loading support (`await
//!   loadScript(url)` still fails once `loadScript` is reached), but lets
//!   any *other* effects in the same script still evaluate.

use crate::srcscan::mask_strings_and_comments;

const REMOVE: &[&str] = &["new", "await", "async", "import"];
const REPLACE: &[(&str, &str)] = &[("var", "let"), ("null", "()")];

pub fn rewrite_keywords(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());

    let mut i = 0;
    while i < n {
        if !mask[i] && is_ident_start(chars[i]) && !preceded_by_ident(&chars, i) {
            let end = ident_end(&chars, i);
            let word: String = chars[i..end].iter().collect();

            if let Some((_, replacement)) = REPLACE.iter().find(|(w, _)| *w == word) {
                out.push_str(replacement);
                i = end;
                continue;
            }
            if REMOVE.contains(&word.as_str()) {
                // also skip one following space/tab so e.g. "new P5()"
                // doesn't leave "P5()" (nicer output; functionally
                // harmless to Rhai either way).
                i = end;
                if chars.get(i).is_some_and(|c| *c == ' ' || *c == '\t') {
                    i += 1;
                }
                continue;
            }

            out.push_str(&word);
            i = end;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn preceded_by_ident(chars: &[char], i: usize) -> bool {
    i > 0 && (is_ident_char(chars[i - 1]) || chars[i - 1] == '.')
}

fn ident_end(chars: &[char], start: usize) -> usize {
    let mut j = start;
    while j < chars.len() && is_ident_char(chars[j]) {
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::rewrite_keywords;

    #[test]
    fn replaces_var_with_let() {
        assert_eq!(rewrite_keywords("var x = 5"), "let x = 5");
    }

    #[test]
    fn replaces_null_with_unit() {
        assert_eq!(rewrite_keywords("x = null"), "x = ()");
    }

    #[test]
    fn drops_new_keyword() {
        assert_eq!(rewrite_keywords("p1=new P5()"), "p1=P5()");
    }

    #[test]
    fn drops_await_keyword() {
        assert_eq!(rewrite_keywords("await loadScript(url)"), "loadScript(url)");
    }

    #[test]
    fn drops_async_keyword() {
        assert_eq!(rewrite_keywords("async()=>time"), "()=>time");
    }

    #[test]
    fn does_not_match_partial_identifiers() {
        let src = "newValue = varietyOf(nullable)";
        assert_eq!(rewrite_keywords(src), src);
    }

    #[test]
    fn does_not_match_property_names() {
        let src = "foo.new().var().null()";
        assert_eq!(rewrite_keywords(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"var new null await\") // var new null await";
        assert_eq!(rewrite_keywords(src), src);
    }
}
