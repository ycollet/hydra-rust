//! Extracts real hydra.js `setFunction({...})` custom-GLSL-function
//! descriptors out of the raw source text, *before* any other
//! preprocessing pass (or Rhai itself) ever sees the script.
//!
//! `setFunction` lets a sketch define its own source/geometry/color/blend/
//! modulate function by handing over a hand-written GLSL function *body*
//! (see hydra.js docs) instead of only using the functions this project
//! already knows about - the normal way to add one of those is the
//! hand-authored, two-step `library.glsl` + `FnMeta` process documented in
//! `SPEC.md` §11. A script-defined one obviously can't go through that
//! (compile-time) path, so this module does the Rhai-function-table
//! registration dynamically, at `eval()` time, instead.
//!
//! This has to run first, ahead of every other pass: the `glsl` field is
//! raw shader body text (typically a `` `...` `` template literal), and
//! several later passes rewrite things like bare semicolons, object
//! literals, or keywords that would otherwise also match *inside* that
//! text, corrupting it. Pulling each `setFunction({...})` call out
//! whole - replacing it with nothing - means nothing downstream, Rhai
//! included, ever has to understand the call or its contents.
//!
//! Limitations (documented, not full hydra.js parity):
//! - every input is treated as a plain `float` (like every other
//!   function's parameters in this project - see `SPEC.md`'s "All
//!   parameters are float here"), regardless of the input's declared
//!   `type` field.
//! - a custom function's `glsl` body is spliced in verbatim except for a
//!   `time`/`resolution` -> `iTime`/`iResolution` identifier rename (real
//!   hydra.js sketches reference the former; this project's shaders are
//!   built around the latter, ShaderToy-style, uniform names).
//! - `setFunction(...)` must appear as its own statement; chaining
//!   straight off of its return value (`setFunction({...}).out()`) isn't
//!   supported - call the new function by name in a later statement.

use crate::srcscan::{Region, classify};

/// Which of hydra.js's five `setFunction` signature shapes a custom
/// function uses - mirrors this project's own `OpKind` (kept separate
/// since this module doesn't depend on `eval.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomFnKind {
    Src,
    Coord,
    Color,
    Combine,
    CombineCoord,
}

impl CustomFnKind {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "src" => Some(Self::Src),
            "coord" => Some(Self::Coord),
            "color" => Some(Self::Color),
            "combine" => Some(Self::Combine),
            "combineCoord" => Some(Self::CombineCoord),
            _ => None,
        }
    }
}

/// One parsed `setFunction({...})` call.
#[derive(Debug, Clone)]
pub struct CustomFnDef {
    pub name: String,
    pub kind: CustomFnKind,
    /// Declared parameter names (after the fixed leading argument(s)) and
    /// their default values, in declaration order.
    pub inputs: Vec<(String, f64)>,
    pub glsl_body: String,
}

/// Finds every top-level `setFunction({...})` call in `code`, returning
/// the parsed descriptors plus the source with those calls (and an
/// optional trailing `;`) removed.
pub fn extract(code: &str) -> (String, Vec<CustomFnDef>) {
    let chars: Vec<char> = code.chars().collect();
    let region = classify(&chars);
    let n = chars.len();

    let mut defs = Vec::new();
    let mut out = String::with_capacity(code.len());
    let mut i = 0;
    while i < n {
        if region[i] == Region::Code && ident_matches_at(&chars, &region, i, "setFunction") {
            let mut j = i + "setFunction".len();
            while j < n && chars[j].is_whitespace() {
                j += 1;
            }
            if chars.get(j) == Some(&'(')
                && let Some((args_start, args_end, call_end)) = find_balanced_call(&chars, &region, j)
                && let Some(def) = parse_descriptor(&chars, &region, args_start, args_end)
            {
                defs.push(def);
                let mut k = call_end;
                while k < n && chars[k].is_whitespace() && chars[k] != '\n' {
                    k += 1;
                }
                if chars.get(k) == Some(&';') {
                    k += 1;
                }
                i = k;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }

    (out, defs)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True if the whole word `ident` starts at `i` (as a `Region::Code`
/// token, not part of a longer identifier) - i.e. not immediately
/// preceded or followed by another identifier character.
fn ident_matches_at(chars: &[char], region: &[Region], i: usize, ident: &str) -> bool {
    let wlen = ident.chars().count();
    if i + wlen > chars.len() || region[i..i + wlen].iter().any(|r| *r != Region::Code) {
        return false;
    }
    if chars[i..i + wlen].iter().collect::<String>() != ident {
        return false;
    }
    if i > 0 && is_ident_char(chars[i - 1]) {
        return false;
    }
    !chars.get(i + wlen).is_some_and(|c| is_ident_char(*c))
}

/// Given the index of the `(` of a call, returns `(args_start, args_end,
/// call_end)`: the span of the argument list's text (exclusive of the
/// outer parens) and the index right after the matching `)`. Only
/// `Region::Code` parens count towards balancing, so parens inside a
/// string/comment (e.g. GLSL code written inside the `glsl` field) don't
/// confuse the search.
fn find_balanced_call(chars: &[char], region: &[Region], open_paren: usize) -> Option<(usize, usize, usize)> {
    let mut depth = 0i32;
    let mut i = open_paren;
    let n = chars.len();
    while i < n {
        if region[i] == Region::Code {
            match chars[i] {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((open_paren + 1, i, i + 1));
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Splits the body of a bracketed span (already known to start right
/// after an opening `{`/`[` and end right before its matching `}`/`]`)
/// into top-level comma-separated, trimmed segments - nested
/// brackets/braces/parens (outside strings/comments) don't split.
fn split_top_level(chars: &[char], region: &[Region], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut segments = Vec::new();
    let mut depth = 0i32;
    let mut seg_start = start;
    let mut i = start;
    while i < end {
        if region[i] == Region::Code {
            match chars[i] {
                '{' | '[' | '(' => depth += 1,
                '}' | ']' | ')' => depth -= 1,
                ',' if depth == 0 => {
                    push_trimmed(chars, seg_start, i, &mut segments);
                    seg_start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    push_trimmed(chars, seg_start, end, &mut segments);
    segments.into_iter().filter(|(s, e)| s < e).collect()
}

fn push_trimmed(chars: &[char], mut start: usize, mut end: usize, out: &mut Vec<(usize, usize)>) {
    while start < end && chars[start].is_whitespace() {
        start += 1;
    }
    while end > start && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    out.push((start, end));
}

/// Splits a `key: value` segment (as produced by `split_top_level`) into
/// the key's text and the value's `(start, end)` span. The key/value
/// separator is the first top-level `:` in the segment.
fn split_key_value(chars: &[char], region: &[Region], start: usize, end: usize) -> Option<(String, usize, usize)> {
    let mut depth = 0i32;
    let mut i = start;
    while i < end {
        if region[i] == Region::Code {
            match chars[i] {
                '{' | '[' | '(' => depth += 1,
                '}' | ']' | ')' => depth -= 1,
                ':' if depth == 0 => {
                    let mut ks = start;
                    let mut ke = i;
                    while ks < ke && chars[ks].is_whitespace() {
                        ks += 1;
                    }
                    while ke > ks && chars[ke - 1].is_whitespace() {
                        ke -= 1;
                    }
                    let key_text: String = chars[ks..ke].iter().collect();
                    let key = unquote_if_quoted(&key_text);
                    let mut vs = i + 1;
                    while vs < end && chars[vs].is_whitespace() {
                        vs += 1;
                    }
                    let mut ve = end;
                    while ve > vs && chars[ve - 1].is_whitespace() {
                        ve -= 1;
                    }
                    return Some((key, vs, ve));
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn unquote_if_quoted(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() >= 2 && (c[0] == '"' || c[0] == '\'') && c[c.len() - 1] == c[0] {
        c[1..c.len() - 1].iter().collect()
    } else {
        s.to_string()
    }
}

/// Reads a string/template-literal value starting at `start` (its
/// opening quote/backtick), returning the unescaped (for `"`/`'`) or
/// verbatim (for `` ` ``) content.
fn string_value(chars: &[char], region: &[Region], start: usize) -> Option<String> {
    if region.get(start) != Some(&Region::StringLit) {
        return None;
    }
    let delim = chars[start];
    let mut end = start;
    while end < chars.len() && region[end] == Region::StringLit {
        end += 1;
    }
    // `classify` includes the closing delimiter in the StringLit region,
    // so `end` now points one past it.
    if end < start + 2 {
        return None;
    }
    let inner: String = chars[start + 1..end - 1].iter().collect();
    if delim == '`' {
        Some(inner)
    } else {
        Some(unescape_basic(&inner))
    }
}

fn unescape_basic(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn number_value(chars: &[char], start: usize, end: usize) -> Option<f64> {
    let text: String = chars[start..end].iter().collect();
    text.trim().parse::<f64>().ok()
}

/// Parses the `{name, type, inputs, glsl}` object literal occupying
/// `[args_start, args_end)` (the full argument-list text of a
/// `setFunction(...)` call - expected to be exactly one object literal).
fn parse_descriptor(chars: &[char], region: &[Region], args_start: usize, args_end: usize) -> Option<CustomFnDef> {
    let mut s = args_start;
    let mut e = args_end;
    while s < e && chars[s].is_whitespace() {
        s += 1;
    }
    while e > s && chars[e - 1].is_whitespace() {
        e -= 1;
    }
    if chars.get(s) != Some(&'{') || chars.get(e.checked_sub(1)?) != Some(&'}') {
        return None;
    }

    let mut name = None;
    let mut kind = None;
    let mut glsl_body = None;
    let mut inputs = Vec::new();

    for (seg_start, seg_end) in split_top_level(chars, region, s + 1, e - 1) {
        let (key, vs, ve) = split_key_value(chars, region, seg_start, seg_end)?;
        match key.as_str() {
            "name" => name = string_value(chars, region, vs),
            "type" => kind = string_value(chars, region, vs).and_then(|t| CustomFnKind::from_str(&t)),
            "glsl" => glsl_body = string_value(chars, region, vs),
            "inputs"
                if chars.get(vs) == Some(&'[') && chars.get(ve.checked_sub(1)?) == Some(&']') =>
            {
                for (item_s, item_e) in split_top_level(chars, region, vs + 1, ve - 1) {
                    if let Some((iname, idefault)) =
                        parse_input_item(chars, region, item_s, item_e)
                    {
                        inputs.push((iname, idefault));
                    }
                }
            }
            _ => {}
        }
    }

    Some(CustomFnDef {
        name: name?,
        kind: kind?,
        inputs,
        glsl_body: rename_hydra_uniforms(&glsl_body?),
    })
}

/// Parses one `{type: 'float', name: 'speed', default: 0}` entry from an
/// `inputs` array, returning its name and default (falling back to `0.0`
/// if `default` is missing).
fn parse_input_item(chars: &[char], region: &[Region], start: usize, end: usize) -> Option<(String, f64)> {
    if chars.get(start) != Some(&'{') || chars.get(end.checked_sub(1)?) != Some(&'}') {
        return None;
    }
    let mut name = None;
    let mut default = 0.0;
    for (seg_start, seg_end) in split_top_level(chars, region, start + 1, end - 1) {
        let (key, vs, ve) = split_key_value(chars, region, seg_start, seg_end)?;
        match key.as_str() {
            "name" => name = string_value(chars, region, vs),
            "default" => default = number_value(chars, vs, ve).unwrap_or(0.0),
            _ => {}
        }
    }
    Some((name?, default))
}

/// Real hydra.js shaders reference `time`/`resolution`; this project's
/// generated GLSL instead uses ShaderToy-style `iTime`/`iResolution`
/// uniform names - rewrite bare identifier occurrences so community GLSL
/// snippets work unmodified.
fn rename_hydra_uniforms(glsl: &str) -> String {
    let chars: Vec<char> = glsl.chars().collect();
    let mut out = String::with_capacity(glsl.len());
    let mut i = 0;
    while i < chars.len() {
        for (from, to) in [("resolution", "iResolution"), ("time", "iTime")] {
            let wlen = from.chars().count();
            if chars[i..].iter().take(wlen).collect::<String>() == from
                && !chars.get(i.wrapping_sub(1)).is_some_and(|c| is_ident_char(*c) || *c == '.')
                && !chars.get(i + wlen).is_some_and(|c| is_ident_char(*c))
            {
                out.push_str(to);
                i += wlen;
                continue;
            }
        }
        if i < chars.len() {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_src_function_and_strips_the_call() {
        let src = "setFunction({\n  name: 'gradient2',\n  type: 'src',\n  inputs: [{type: 'float', name: 'speed', default: 0}],\n  glsl: `return vec4(sin(speed*time), st, 1.0);`\n});\ngradient2(0.5).out();";
        let (rewritten, defs) = extract(src);
        assert_eq!(rewritten.trim(), "gradient2(0.5).out();");
        assert_eq!(defs.len(), 1);
        let def = &defs[0];
        assert_eq!(def.name, "gradient2");
        assert_eq!(def.kind, CustomFnKind::Src);
        assert_eq!(def.inputs, vec![("speed".to_string(), 0.0)]);
        assert_eq!(def.glsl_body, "return vec4(sin(speed*iTime), st, 1.0);");
    }

    #[test]
    fn leaves_code_without_set_function_untouched() {
        let src = "osc(10, 0.1, 0.5).out();";
        let (rewritten, defs) = extract(src);
        assert_eq!(rewritten, src);
        assert!(defs.is_empty());
    }

    #[test]
    fn defaults_a_missing_input_default_to_zero() {
        let src = "setFunction({name:'f', type:'coord', inputs:[{name:'amt'}], glsl:`return st;`});";
        let (_, defs) = extract(src);
        assert_eq!(defs[0].inputs, vec![("amt".to_string(), 0.0)]);
    }

    #[test]
    fn supports_multiple_definitions() {
        let src = "setFunction({name:'a', type:'src', inputs:[], glsl:`return vec4(1.0);`});\nsetFunction({name:'b', type:'color', inputs:[], glsl:`return _c0;`});";
        let (rewritten, defs) = extract(src);
        assert_eq!(rewritten.trim(), "");
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "a");
        assert_eq!(defs[1].name, "b");
    }

    #[test]
    fn ignores_set_function_inside_a_string_or_comment() {
        let src = "text(\"setFunction({})\"); // setFunction({})";
        let (rewritten, defs) = extract(src);
        assert_eq!(rewritten, src);
        assert!(defs.is_empty());
    }

    #[test]
    fn does_not_misfire_on_a_longer_identifier() {
        let src = "mySetFunctionWrapper(1);";
        let (rewritten, defs) = extract(src);
        assert_eq!(rewritten, src);
        assert!(defs.is_empty());
    }

    #[test]
    fn parses_without_an_inputs_array() {
        let src = "setFunction({name:'f', type:'combine', glsl:`return mix(_c0,_c1,0.5);`});";
        let (_, defs) = extract(src);
        assert_eq!(defs.len(), 1);
        assert!(defs[0].inputs.is_empty());
    }
}
