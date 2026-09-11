//! Rewrites JS's postfix/prefix increment/decrement operators (`i++`,
//! `i--`, `++i`, `--i`) into the equivalent `i += 1` / `i -= 1` - Rhai has
//! no `++`/`--` operator at all ("Unknown operator"). Only handles a bare
//! identifier operand (`i++`, not `arr[i]++` or `obj.prop++`), and always
//! produces the post/pre-neutral `+= 1` form: the pre/post distinction
//! only matters when the expression's own *value* is used, which doesn't
//! happen in a hydra sketch - these appear almost exclusively as a
//! `for`-loop's update clause or a bare statement in a loop body.

use crate::srcscan::mask_strings_and_comments;

pub fn rewrite_increment_decrement(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mask = mask_strings_and_comments(&chars);
    let n = chars.len();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;

    while i < n {
        if mask[i] {
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // Prefix: ++IDENT / --IDENT.
        if matches!(chars[i], '+' | '-')
            && is_op_pair(&chars, &mask, i)
            && chars.get(i + 2).is_some_and(|c| is_ident_start(*c) && !mask[i + 2])
        {
            let op = chars[i];
            let start = i + 2;
            let end = ident_end(&chars, start);
            out.extend(&chars[start..end]);
            out.push_str(if op == '+' { "+=1" } else { "-=1" });
            i = end;
            continue;
        }

        // Postfix: IDENT++ / IDENT--.
        if is_ident_start(chars[i]) && !preceded_by_ident_char(&chars, i) {
            let end = ident_end(&chars, i);
            if end + 1 < n && !mask[end] && matches!(chars[end], '+' | '-') && is_op_pair(&chars, &mask, end) {
                let op = chars[end];
                out.extend(&chars[i..end]);
                out.push_str(if op == '+' { "+=1" } else { "-=1" });
                i = end + 2;
                continue;
            }
        }

        out.push(chars[i]);
        i += 1;
    }

    out
}

/// True if `chars[pos]` and `chars[pos + 1]` are the same (unmasked)
/// character, one of `+`/`-` (i.e. `++` or `--`).
fn is_op_pair(chars: &[char], mask: &[bool], pos: usize) -> bool {
    chars.get(pos + 1) == Some(&chars[pos]) && !mask.get(pos + 1).copied().unwrap_or(true)
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

#[cfg(test)]
mod tests {
    use super::rewrite_increment_decrement;

    #[test]
    fn rewrites_postfix_increment() {
        assert_eq!(rewrite_increment_decrement("i++"), "i+=1");
    }

    #[test]
    fn rewrites_postfix_decrement() {
        assert_eq!(rewrite_increment_decrement("i--"), "i-=1");
    }

    #[test]
    fn rewrites_prefix_increment() {
        assert_eq!(rewrite_increment_decrement("++i"), "i+=1");
    }

    #[test]
    fn rewrites_prefix_decrement() {
        assert_eq!(rewrite_increment_decrement("--i"), "i-=1");
    }

    #[test]
    fn rewrites_inside_a_for_loop_header() {
        assert_eq!(rewrite_increment_decrement("for(i=0;i<n;i++){}"), "for(i=0;i<n;i+=1){}");
    }

    #[test]
    fn leaves_plain_subtraction_and_addition_alone() {
        let src = "a+b-c";
        assert_eq!(rewrite_increment_decrement(src), src);
    }

    #[test]
    fn leaves_longer_identifier_names_alone() {
        // must not misfire in the middle of `abc` while scanning for `++`
        let src = "abc + def";
        assert_eq!(rewrite_increment_decrement(src), src);
    }

    #[test]
    fn ignores_inside_strings_and_comments() {
        let src = "text(\"i++\") // i++";
        assert_eq!(rewrite_increment_decrement(src), src);
    }
}
