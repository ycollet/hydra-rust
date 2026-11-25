use std::collections::HashSet;

use super::{Definition, InputDefault, InputKind};
use crate::srcscan::{Region, classify};

pub(super) const MAX_FUNCTIONS: usize = 4096;
pub(super) const MAX_GLSL_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_FUNCTION_BODY: usize = 64 * 1024;
pub(super) const MAX_NAME: usize = 128;
pub(super) const MAX_INPUTS: usize = 32;

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_NAME
        && matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with("gl_")
}

pub(super) fn shader_name(index: usize) -> String {
    format!("hydraJsFunction{index}")
}

pub(super) fn validate(definitions: &[Definition]) -> Result<(), String> {
    let mut bytes = 0;
    let mut functions = HashSet::new();
    for definition in definitions {
        if !identifier(&definition.name)
            || !functions.insert(&definition.name)
            || definition.inputs.len() > MAX_INPUTS
            || definition.glsl.len() > MAX_FUNCTION_BODY
        {
            return Err("Invalid custom function name or size".into());
        }
        bytes += definition.glsl.len();
        if bytes > MAX_GLSL_BYTES {
            return Err("Custom shader registry byte limit exceeded".into());
        }
        let mut names = HashSet::new();
        for input in &definition.inputs {
            if !identifier(&input.name)
                || ["st", "c0", "c1", "_st", "_c0", "_c1"].contains(&input.name.as_str())
                || !names.insert(&input.name)
            {
                return Err("Invalid custom function input name".into());
            }
            match (&input.kind, &input.default) {
                (InputKind::Float, InputDefault::Number(value))
                    if value.is_finite() && (*value as f32).is_finite() => {}
                (InputKind::Sampler, InputDefault::Texture(texture)) if texture.index < 4 => {}
                _ => return Err("Invalid custom function input default".into()),
            }
        }
        if !["src", "coord", "color", "combine", "combineCoord"].contains(&definition.kind.as_str())
        {
            return Err("Unsupported custom function type".into());
        }
    }
    Ok(())
}

fn tokens(body: &str) -> Vec<String> {
    let chars: Vec<_> = body.chars().collect();
    let regions = classify(&chars);
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if regions[index] != Region::Code || chars[index].is_whitespace() {
            index += 1;
        } else if chars[index].is_ascii_alphabetic() || chars[index] == '_' {
            let start = index;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            tokens.push(chars[start..index].iter().collect());
        } else {
            tokens.push(chars[index].to_string());
            index += 1;
        }
    }
    tokens
}

fn declares(body: &[String], name: &str) -> bool {
    let mut depth = 0;
    let mut parentheses_depth = 0;
    let mut brackets_depth = 0;
    let mut declaration = false;
    let mut expect_name = false;
    for (index, token) in body.iter().enumerate() {
        match token.as_str() {
            "{" => depth += 1,
            "}" => depth -= 1,
            "(" => parentheses_depth += 1,
            ")" => parentheses_depth -= 1,
            "[" => brackets_depth += 1,
            "]" => brackets_depth -= 1,
            _ => {}
        }
        if depth != 0 || parentheses_depth != 0 || brackets_depth != 0 {
            continue;
        }
        if token == ";" {
            declaration = false;
            expect_name = false;
        } else if token == "," && declaration {
            expect_name = true;
        } else if expect_name {
            if token == name {
                return true;
            }
            expect_name = false;
        } else if !declaration
            && [
                "float", "int", "uint", "bool", "vec2", "vec3", "vec4", "ivec2", "ivec3", "ivec4",
                "uvec2", "uvec3", "uvec4", "bvec2", "bvec3", "bvec4", "mat2", "mat3", "mat4",
            ]
            .contains(&token.as_str())
            && body
                .get(index + 1)
                .and_then(|token| token.as_bytes().first())
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        {
            declaration = true;
            expect_name = true;
        }
    }
    false
}

fn body(definition: &Definition) -> String {
    let chars: Vec<_> = definition.glsl.chars().collect();
    let regions = classify(&chars);
    let mut output = String::new();
    let mut index = 0;
    let mut struct_pending = false;
    let mut struct_scopes = Vec::new();
    while index < chars.len() {
        if regions[index] == Region::Code
            && (chars[index].is_ascii_alphabetic() || chars[index] == '_')
        {
            let start = index;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            if word == "struct" {
                struct_pending = true;
            }
            let field = struct_scopes.last().copied().unwrap_or(false)
                || chars[..start]
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(index, character)| {
                        regions[*index] == Region::Code && !character.is_whitespace()
                    })
                    .is_some_and(|(_, character)| *character == '.');
            if !field
                && let Some(input) = definition
                    .inputs
                    .iter()
                    .position(|input| input.name == word)
            {
                output.push_str(&format!("hydraJsInput{input}"));
            } else {
                output.push_str(match (word.as_str(), field) {
                    ("time", false) => "iTime",
                    ("resolution", false) => "iResolution",
                    ("texture2D", false) => "texture",
                    _ => &word,
                });
            }
        } else {
            if regions[index] == Region::Code {
                match chars[index] {
                    '{' => {
                        struct_scopes
                            .push(struct_pending || struct_scopes.last().copied().unwrap_or(false));
                        struct_pending = false;
                    }
                    '}' => {
                        struct_scopes.pop();
                    }
                    _ => {}
                }
            }
            output.push(chars[index]);
            index += 1;
        }
    }
    output
}

pub(super) fn compile(definition: &Definition, index: usize) -> Result<String, String> {
    let (returns, fixed) = match definition.kind.as_str() {
        "src" => ("vec4", vec![("vec2", "st")]),
        "coord" => ("vec2", vec![("vec2", "st")]),
        "color" => ("vec4", vec![("vec4", "c0")]),
        "combine" => ("vec4", vec![("vec4", "c0"), ("vec4", "c1")]),
        "combineCoord" => ("vec2", vec![("vec2", "st"), ("vec4", "c0")]),
        _ => return Err("Unsupported custom function type".into()),
    };
    let tokens = tokens(&definition.glsl);
    let mut parameters = Vec::new();
    let mut aliases = String::new();
    for (kind, name) in fixed {
        if declares(&tokens, &format!("_{name}")) {
            return Err(format!(
                "Custom shader redeclares its Hydra argument _{name}"
            ));
        }
        parameters.push(format!("{kind} _{name}"));
        if !declares(&tokens, name) {
            aliases.push_str(&format!("{kind} {name} = _{name};\n"));
        }
    }
    parameters.extend(definition.inputs.iter().enumerate().map(|(index, input)| {
        format!(
            "{} hydraJsInput{index}",
            match input.kind {
                InputKind::Float => "float",
                InputKind::Sampler => "sampler2D",
            }
        )
    }));
    Ok(format!(
        "{returns} {}({}) {{\n{aliases}{}\n}}\n",
        shader_name(index),
        parameters.join(", "),
        body(definition)
    ))
}
