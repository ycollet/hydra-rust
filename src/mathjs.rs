//! Rewrites JS `Math.*` references into the plain function/constant names
//! hydra-rust already registers (`Math.sin(x)` -> `sin(x)`, `Math.PI` ->
//! a numeric literal), for the subset of `Math` that has a direct
//! equivalent. Anything not in `METHOD_MAP`/`CONST_MAP` is deliberately
//! left alone.

use crate::srcscan::mask_strings_and_comments;

const MATH_PI: &str = "3.141592653589793";
const MATH_E: &str = "2.718281828459045";

/// JS `Math` constant name -> its numeric literal text.
const CONST_MAP: &[(&str, &str)] = &[("PI", MATH_PI), ("E", MATH_E)];

/// JS `Math` method name -> registered Rhai/GLSL function name.
/// `atan2` maps to `atan`, matching GLSL's two-argument `atan(y, x)`
/// overload (registered alongside the one-argument form).
const METHOD_MAP: &[(&str, &str)] = &[
    ("sin", "sin"),
    ("cos", "cos"),
    ("tan", "tan"),
    ("asin", "asin"),
    ("acos", "acos"),
    ("atan2", "atan"),
    ("atan", "atan"),
    ("abs", "abs"),
    ("floor", "floor"),
    ("ceil", "ceil"),
    ("round", "round"),
    ("sqrt", "sqrt"),
    ("sign", "sign"),
    ("exp", "exp"),
    ("log", "log"),
    ("pow", "pow"),
    ("min", "min"),
    ("max", "max"),
    ("random", "random"),
];

pub fn rewrite_math(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());
    const PREFIX: &str = "Math.";

    let mut i = 0;
    while i < n {
        if !mask[i] && matches_at(&chars, &mask, i, PREFIX) && !preceded_by_ident(&chars, i) {
            let ident_start = i + PREFIX.chars().count();
            let ident_end = ident_end(&chars, ident_start);
            let ident: String = chars[ident_start..ident_end].iter().collect();

            if let Some((_, literal)) = CONST_MAP.iter().find(|(js, _)| *js == ident)
                && !is_ident_char(chars.get(ident_end).copied())
            {
                out.push_str(literal);
                i = ident_end;
                continue;
            }

            if let Some((_, glsl_name)) = METHOD_MAP.iter().find(|(js, _)| *js == ident)
                && next_significant_is_paren(&chars, &mask, ident_end)
            {
                out.push_str(glsl_name);
                i = ident_end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

fn matches_at(chars: &[char], mask: &[bool], i: usize, prefix: &str) -> bool {
    let prefix: Vec<char> = prefix.chars().collect();
    if i + prefix.len() > chars.len() {
        return false;
    }
    (0..prefix.len()).all(|k| !mask[i + k] && chars[i + k] == prefix[k])
}

fn preceded_by_ident(chars: &[char], i: usize) -> bool {
    i > 0 && is_ident_char(Some(chars[i - 1]))
}

fn is_ident_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

fn ident_end(chars: &[char], start: usize) -> usize {
    let mut j = start;
    while j < chars.len() && is_ident_char(Some(chars[j])) {
        j += 1;
    }
    j
}

fn next_significant_is_paren(chars: &[char], mask: &[bool], mut j: usize) -> bool {
    while j < chars.len() && (chars[j].is_whitespace() || mask[j]) {
        j += 1;
    }
    chars.get(j) == Some(&'(')
}

#[cfg(test)]
mod tests {
    use super::rewrite_math;

    #[test]
    fn rewrites_known_methods() {
        assert_eq!(rewrite_math("Math.sin(time)"), "sin(time)");
        assert_eq!(rewrite_math("Math.atan2(y,x)"), "atan(y,x)");
    }

    #[test]
    fn rewrites_pi_constant() {
        assert_eq!(rewrite_math("Math.PI*2"), "3.141592653589793*2");
    }

    #[test]
    fn rewrites_e_constant() {
        assert_eq!(rewrite_math("Math.E*2"), "2.718281828459045*2");
    }

    #[test]
    fn rewrites_random_call() {
        // Math.random() is called once at script-load time in real JS
        // (hydra-rust has no per-frame closures either), so a plain
        // eval-time RNG call is a faithful equivalent.
        assert_eq!(rewrite_math("Math.random()"), "random()");
    }

    #[test]
    fn leaves_random_without_call_alone() {
        assert_eq!(rewrite_math("Math.random"), "Math.random");
    }

    #[test]
    fn leaves_non_call_reference_alone() {
        // `Math.sin` with no following call isn't rewritten (would change
        // an unrelated free variable reference into a bare function name).
        assert_eq!(rewrite_math("Math.sin"), "Math.sin");
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"Math.sin(x)\") // Math.PI";
        assert_eq!(rewrite_math(src), src);
    }

    #[test]
    fn does_not_match_partial_identifier() {
        assert_eq!(rewrite_math("fooMath.sin(x)"), "fooMath.sin(x)");
    }
}
