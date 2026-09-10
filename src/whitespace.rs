//! Normalizes non-ASCII Unicode whitespace (non-breaking spaces and other
//! Unicode-only space variants, often introduced by copy-pasting from a web
//! page or word processor) to a plain ASCII space. Rhai's lexer only
//! recognizes ASCII space/tab/CR/LF as whitespace; anything else in code
//! position is a hard "Unexpected '<char>'" lex error, even though the
//! character is visually indistinguishable from a normal space and Rust's
//! own `char::is_whitespace()` (used throughout the other preprocessing
//! passes here) already treats it as whitespace.
//!
//! Runs first, unconditionally (no string/comment masking): even inside a
//! string, one of these is only ever going to render identically to a
//! regular space, so there's no reason to preserve the distinction.

pub fn normalize_whitespace(src: &str) -> String {
    src.chars()
        .map(|c| if c.is_whitespace() && !matches!(c, ' ' | '\t' | '\n' | '\r') { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::normalize_whitespace;

    #[test]
    fn replaces_non_breaking_space() {
        assert_eq!(normalize_whitespace("osc(60)\u{00A0}.out()"), "osc(60) .out()");
    }

    #[test]
    fn replaces_other_unicode_spaces() {
        // en quad, em space, ideographic space
        assert_eq!(normalize_whitespace("a\u{2000}b\u{2003}c\u{3000}d"), "a b c d");
    }

    #[test]
    fn leaves_ascii_whitespace_alone() {
        let src = "osc(60)\n\t.out()\r\n";
        assert_eq!(normalize_whitespace(src), src);
    }

    #[test]
    fn leaves_normal_code_alone() {
        let src = "osc(60,0.1,0).out()";
        assert_eq!(normalize_whitespace(src), src);
    }
}
