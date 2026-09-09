use std::sync::{Arc, Mutex};

use rhai::{Array, CustomType, Dynamic, Engine, ImmutableString, Scope, TypeBuilder};

use crate::argtrunc;
use crate::arrow;
use crate::asi;
use crate::autolet;
use crate::jskeywords;
use crate::mathjs;
use crate::numlit;
use crate::patcall;
use crate::quotes;
use crate::ternary;
use crate::text::{self, TextData};
#[cfg(feature = "audio")]
use crate::audio::NUM_FFT_BINS;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum RenderMode {
    #[default]
    Single0,
    Single(usize),
    All,
}

#[cfg(feature = "webcam")]
#[derive(Debug, Clone)]
pub enum SourceRequest {
    InitCam { slot: usize, camera_index: u32 },
}

#[cfg(feature = "audio")]
#[derive(Debug, Clone, Copy)]
pub enum AudioRequest {
    SetBins(usize),
    SetCutoff(f32),
    SetScale(f32),
    SetSmooth(f32),
}

pub struct EvalResult {
    pub shaders: [Option<String>; 4],
    pub render_mode: RenderMode,
    pub text_data: Option<TextData>,
    #[cfg(feature = "webcam")]
    pub source_requests: Vec<SourceRequest>,
    #[cfg(feature = "audio")]
    pub audio_requests: Vec<AudioRequest>,
}

/// The `a` audio object (`a.fft[i]`, `a.setBins(...)`, ...).
#[cfg(feature = "audio")]
#[derive(Debug, Clone, Copy)]
struct Audio;

/// Returned by `a.fft`; indexing it yields a `GlslExpr` reading `iFft[i]`.
#[cfg(feature = "audio")]
#[derive(Debug, Clone, Copy)]
struct AudioFft;

/// The `mouse` object (`mouse.x`, `mouse.y`), matching real hydra.js.
#[derive(Debug, Clone, Copy)]
struct Mouse;

/// The `window` object (`window.innerWidth`, `window.innerHeight`), a
/// browser-DOM stand-in some sketches reference for canvas size instead of
/// (or alongside) the bare `width`/`height` globals - both map to the same
/// `iResolution` uniform here.
#[derive(Debug, Clone, Copy)]
struct Window;

#[derive(Debug, Clone)]
enum Arg {
    Lit(f64),
    Expr(String),
}

#[derive(Debug, Clone)]
struct GlslExpr(String);

#[derive(Debug, Clone)]
struct Pattern {
    values: Vec<f64>,
    speed: f64,
    offset: f64,
    smooth: bool,
}

impl Pattern {
    fn from_array(arr: Array) -> Self {
        let values = arr
            .iter()
            .map(|d| {
                d.as_float()
                    .unwrap_or_else(|_| d.as_int().map(|i| i as f64).unwrap_or(0.0))
            })
            .collect();
        Self { values, speed: 1.0, offset: 0.0, smooth: false }
    }

    fn to_glsl(&self) -> String {
        let n = self.values.len();
        if n == 0 {
            return "0.0".into();
        }
        if n == 1 {
            return fmt_f(self.values[0]);
        }

        let offset_part = if self.offset != 0.0 {
            format!(" + {}", fmt_f(self.offset))
        } else {
            String::new()
        };
        let base = format!("mod(iTime * {}{}, {}.0)", fmt_f(self.speed), offset_part, n);

        let terms: Vec<String> = if self.smooth {
            self.values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let next = self.values[(i + 1) % n];
                    format!(
                        "mix({}, {}, fract({base})) * step(abs(floor({base}) - {}.0), 0.5)",
                        fmt_f(*v),
                        fmt_f(next),
                        i
                    )
                })
                .collect()
        } else {
            self.values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    format!(
                        "{} * step(abs(floor({base}) - {}.0), 0.5)",
                        fmt_f(*v),
                        i
                    )
                })
                .collect()
        };

        format!("({})", terms.join(" + "))
    }
}

#[derive(Debug, Clone, CustomType)]
pub struct Node {
    ops: Vec<Op>,
}

#[derive(Debug, Clone)]
enum Op {
    Source {
        func: &'static str,
        args: Vec<Arg>,
    },
    Geo {
        func: &'static str,
        args: Vec<Arg>,
    },
    Color {
        func: &'static str,
        args: Vec<Arg>,
    },
    Blend {
        func: &'static str,
        other: Node,
        args: Vec<Arg>,
    },
    Modulate {
        func: &'static str,
        other: Node,
        args: Vec<Arg>,
    },
}

#[derive(Clone, Copy)]
enum OpKind {
    Source,
    Geo,
    Color,
    Blend,
    Modulate,
}

struct FnMeta {
    name: &'static str,
    kind: OpKind,
    defaults: &'static [f64],
}

const FUNCTIONS: &[FnMeta] = &[
    FnMeta { name: "osc", kind: OpKind::Source, defaults: &[60.0, 0.1, 0.0] },
    FnMeta { name: "noise", kind: OpKind::Source, defaults: &[10.0, 0.1] },
    FnMeta { name: "voronoi", kind: OpKind::Source, defaults: &[5.0, 0.3, 0.3] },
    FnMeta { name: "shape", kind: OpKind::Source, defaults: &[3.0, 0.3, 0.01] },
    FnMeta { name: "gradient", kind: OpKind::Source, defaults: &[0.0] },
    FnMeta { name: "solid", kind: OpKind::Source, defaults: &[0.0, 0.0, 0.0, 1.0] },
    FnMeta { name: "rings", kind: OpKind::Source, defaults: &[8.0, 0.1] },
    FnMeta { name: "checker", kind: OpKind::Source, defaults: &[4.0, 4.0] },
    FnMeta { name: "rotate", kind: OpKind::Geo, defaults: &[10.0, 0.0] },
    FnMeta { name: "scale", kind: OpKind::Geo, defaults: &[1.5, 1.0, 1.0, 0.5, 0.5] },
    FnMeta { name: "scroll", kind: OpKind::Geo, defaults: &[0.5, 0.5, 0.0, 0.0] },
    FnMeta { name: "kaleid", kind: OpKind::Geo, defaults: &[4.0] },
    FnMeta { name: "pixelate", kind: OpKind::Geo, defaults: &[20.0, 20.0] },
    FnMeta { name: "repeat", kind: OpKind::Geo, defaults: &[3.0, 3.0, 0.0, 0.0] },
    FnMeta { name: "scrollX", kind: OpKind::Geo, defaults: &[0.5, 0.0] },
    FnMeta { name: "scrollY", kind: OpKind::Geo, defaults: &[0.5, 0.0] },
    FnMeta { name: "repeatX", kind: OpKind::Geo, defaults: &[3.0, 0.0] },
    FnMeta { name: "repeatY", kind: OpKind::Geo, defaults: &[3.0, 0.0] },
    FnMeta { name: "polar", kind: OpKind::Geo, defaults: &[] },
    FnMeta { name: "cart", kind: OpKind::Geo, defaults: &[] },
    FnMeta { name: "fold", kind: OpKind::Geo, defaults: &[1.0] },
    FnMeta { name: "color", kind: OpKind::Color, defaults: &[1.0, 1.0, 1.0, 1.0] },
    FnMeta { name: "invert", kind: OpKind::Color, defaults: &[1.0] },
    FnMeta { name: "contrast", kind: OpKind::Color, defaults: &[1.6] },
    FnMeta { name: "brightness", kind: OpKind::Color, defaults: &[0.4] },
    FnMeta { name: "saturate", kind: OpKind::Color, defaults: &[2.0] },
    FnMeta { name: "hue", kind: OpKind::Color, defaults: &[0.4] },
    FnMeta { name: "posterize", kind: OpKind::Color, defaults: &[3.0, 0.6] },
    FnMeta { name: "luma", kind: OpKind::Color, defaults: &[0.5, 0.1] },
    FnMeta { name: "colorama", kind: OpKind::Color, defaults: &[0.005] },
    FnMeta { name: "shift", kind: OpKind::Color, defaults: &[0.5, 0.0, 0.0, 0.0] },
    FnMeta { name: "thresh", kind: OpKind::Color, defaults: &[0.5, 0.04] },
    FnMeta { name: "r", kind: OpKind::Color, defaults: &[1.0, 0.0] },
    FnMeta { name: "g", kind: OpKind::Color, defaults: &[1.0, 0.0] },
    FnMeta { name: "b", kind: OpKind::Color, defaults: &[1.0, 0.0] },
    FnMeta { name: "a", kind: OpKind::Color, defaults: &[1.0, 0.0] },
    FnMeta { name: "sum", kind: OpKind::Color, defaults: &[1.0, 1.0, 1.0, 1.0] },
    FnMeta { name: "add", kind: OpKind::Blend, defaults: &[1.0] },
    FnMeta { name: "mult", kind: OpKind::Blend, defaults: &[1.0] },
    FnMeta { name: "blend", kind: OpKind::Blend, defaults: &[0.5] },
    FnMeta { name: "diff", kind: OpKind::Blend, defaults: &[] },
    FnMeta { name: "layer", kind: OpKind::Blend, defaults: &[] },
    FnMeta { name: "mask", kind: OpKind::Blend, defaults: &[] },
    FnMeta { name: "sub", kind: OpKind::Blend, defaults: &[1.0] },
    FnMeta { name: "modulate", kind: OpKind::Modulate, defaults: &[0.1] },
    FnMeta { name: "modulateScale", kind: OpKind::Modulate, defaults: &[1.0, 1.0] },
    FnMeta { name: "modulateRotate", kind: OpKind::Modulate, defaults: &[1.0, 0.0] },
    FnMeta { name: "modulateRepeat", kind: OpKind::Modulate, defaults: &[3.0, 3.0, 0.5, 0.5] },
    FnMeta { name: "modulateRepeatX", kind: OpKind::Modulate, defaults: &[3.0, 0.5] },
    FnMeta { name: "modulateRepeatY", kind: OpKind::Modulate, defaults: &[3.0, 0.5] },
    FnMeta { name: "modulateKaleid", kind: OpKind::Modulate, defaults: &[4.0] },
    FnMeta { name: "modulateScrollX", kind: OpKind::Modulate, defaults: &[0.5, 0.0] },
    FnMeta { name: "modulateScrollY", kind: OpKind::Modulate, defaults: &[0.5, 0.0] },
    FnMeta { name: "modulatePixelate", kind: OpKind::Modulate, defaults: &[10.0, 3.0] },
    FnMeta { name: "modulateHue", kind: OpKind::Modulate, defaults: &[1.0] },
];

impl Node {
    fn source(func: &'static str, args: Vec<Arg>) -> Self {
        Self { ops: vec![Op::Source { func, args }] }
    }

    fn push_geo(mut self, func: &'static str, args: Vec<Arg>) -> Self {
        self.ops.push(Op::Geo { func, args });
        self
    }

    fn push_color(mut self, func: &'static str, args: Vec<Arg>) -> Self {
        self.ops.push(Op::Color { func, args });
        self
    }

    fn push_blend(mut self, func: &'static str, other: Node, args: Vec<Arg>) -> Self {
        self.ops.push(Op::Blend { func, other, args });
        self
    }

    fn push_modulate(mut self, func: &'static str, other: Node, args: Vec<Arg>) -> Self {
        self.ops.push(Op::Modulate { func, other, args });
        self
    }
}

fn fill_args(provided: &[Arg], defaults: &'static [f64]) -> Vec<Arg> {
    let mut args = provided.to_vec();
    for d in defaults.iter().skip(args.len()) {
        args.push(Arg::Lit(*d));
    }
    args
}

fn as_arg(d: Dynamic) -> Arg {
    if let Ok(v) = d.as_float() {
        Arg::Lit(v)
    } else if let Ok(v) = d.as_int() {
        Arg::Lit(v as f64)
    } else if d.is::<GlslExpr>() {
        Arg::Expr(d.cast::<GlslExpr>().0)
    } else if d.is::<Pattern>() {
        Arg::Expr(d.cast::<Pattern>().to_glsl())
    } else if d.is_array() {
        Arg::Expr(Pattern::from_array(d.into_array().unwrap()).to_glsl())
    } else {
        Arg::Lit(0.0)
    }
}

// s0-s3 constants are 100-103; indices below 100 are internal buffers.
fn idx_to_source(idx: i64) -> Node {
    if idx >= 100 {
        Node::source("ext_src", vec![Arg::Lit((idx - 100) as f64)])
    } else {
        Node::source("src", vec![Arg::Lit(idx as f64)])
    }
}

/// Converts the "other" operand of a blend/modulate call into a `Node`.
/// Real hydra.js lets you write e.g. `.modulate(s0, 0.5)`, treating `s0`/`o1`
/// as first-class chainable source objects; here they're bare `i64`
/// constants, so this accepts that raw index directly (equivalent to
/// `src(s0)`) as well as an already-built `Node` chain.
fn as_node(d: Dynamic) -> Result<Node, Box<rhai::EvalAltResult>> {
    if d.is::<Node>() {
        Ok(d.cast::<Node>())
    } else if let Ok(idx) = d.as_int() {
        Ok(idx_to_source(idx))
    } else {
        Err(format!("expected a source or a chain, found {}", d.type_name()).into())
    }
}

fn dyn_to_f64(d: Dynamic) -> f64 {
    d.as_float().unwrap_or_else(|_| d.as_int().map(|i| i as f64).unwrap_or(0.0))
}

fn fmt_f(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.1}") } else { format!("{v}") }
}

fn fmt_arg(a: &Arg) -> String {
    match a {
        Arg::Lit(v) => fmt_f(*v),
        Arg::Expr(s) => s.clone(),
    }
}

fn fmt_args(args: &[Arg]) -> String {
    args.iter().map(fmt_arg).collect::<Vec<_>>().join(", ")
}

struct Emitter {
    lines: Vec<String>,
    st_counter: usize,
    var_counter: usize,
    depth: usize,
}

const MAX_DEPTH: usize = 16;

impl Emitter {
    fn new() -> Self {
        Self { lines: Vec::new(), st_counter: 0, var_counter: 0, depth: 0 }
    }

    fn next_st(&mut self) -> String {
        let name = format!("_st{}", self.st_counter);
        self.st_counter += 1;
        name
    }

    fn next_var(&mut self) -> String {
        let name = format!("_n{}", self.var_counter);
        self.var_counter += 1;
        name
    }

    fn compile_node(&mut self, node: &Node) -> Result<String, String> {
        if self.depth >= MAX_DEPTH {
            return Err("nesting too deep (max 16)".into());
        }
        self.depth += 1;

        let mut source: Option<&Op> = None;
        let mut geo_mods: Vec<&Op> = Vec::new();
        let mut colors: Vec<&Op> = Vec::new();

        for op in &node.ops {
            match op {
                Op::Source { .. } => {
                    if source.is_some() {
                        return Err("chain must have exactly one source".into());
                    }
                    source = Some(op);
                }
                Op::Geo { .. } | Op::Modulate { .. } => geo_mods.push(op),
                Op::Color { .. } | Op::Blend { .. } => colors.push(op),
            }
        }

        let source = source.ok_or("chain must start with a source (osc, noise, src, etc.)")?;

        let mut current_st = "st".to_string();
        for op in geo_mods.iter().rev() {
            match op {
                Op::Geo { func, args } => {
                    let new_st = self.next_st();
                    let a = fmt_args(args);
                    let sep = if a.is_empty() { "" } else { ", " };
                    self.lines.push(format!(
                        "  vec2 {new_st} = {func}({current_st}{sep}{a});"
                    ));
                    current_st = new_st;
                }
                Op::Modulate { func, other, args } => {
                    let sub_var = self.compile_node(other)?;
                    let new_st = self.next_st();
                    let a = fmt_args(args);
                    let sep = if a.is_empty() { "" } else { ", " };
                    self.lines.push(format!(
                        "  vec2 {new_st} = {func}({current_st}, {sub_var}{sep}{a});"
                    ));
                    current_st = new_st;
                }
                _ => unreachable!(),
            }
        }

        let Op::Source { func, args } = source else { unreachable!() };
        let current_var = self.next_var();

        if *func == "src" {
            let buf_idx = match args.first() {
                Some(Arg::Lit(v)) => (*v as usize).min(3),
                _ => 0,
            };
            self.lines.push(format!(
                "  vec4 {current_var} = texture(iBuffer{buf_idx}, {current_st});"
            ));
        } else if *func == "text_src" {
            self.lines.push(format!(
                "  vec4 {current_var} = texture(iText0, vec2({current_st}.x, 1.0 - {current_st}.y));"
            ));
        } else if *func == "ext_src" {
            let src_idx = match args.first() {
                Some(Arg::Lit(v)) => (*v as usize).min(3),
                _ => 0,
            };
            // Force alpha=1.0: cameras produce RGB data, GL fills alpha=1.0 for RGB textures
            // but we force it here too for clarity and to guard against format changes
            self.lines.push(format!(
                "  vec4 {current_var} = vec4(texture(iSource{src_idx}, vec2({current_st}.x, 1.0 - {current_st}.y)).rgb, 1.0);"
            ));
        } else {
            let a = fmt_args(args);
            let sep = if a.is_empty() { "" } else { ", " };
            self.lines.push(format!(
                "  vec4 {current_var} = {func}({current_st}{sep}{a});"
            ));
        }

        let mut prev_var = current_var;
        for op in &colors {
            match op {
                Op::Color { func, args } => {
                    let new_var = self.next_var();
                    let a = fmt_args(args);
                    let sep = if a.is_empty() { "" } else { ", " };
                    self.lines.push(format!(
                        "  vec4 {new_var} = {func}({prev_var}{sep}{a});"
                    ));
                    prev_var = new_var;
                }
                Op::Blend { func, other, args } => {
                    let sub_var = self.compile_node(other)?;
                    let new_var = self.next_var();
                    let a = fmt_args(args);
                    let sep = if a.is_empty() { "" } else { ", " };
                    self.lines.push(format!(
                        "  vec4 {new_var} = {func}({prev_var}, {sub_var}{sep}{a});"
                    ));
                    prev_var = new_var;
                }
                _ => unreachable!(),
            }
        }

        self.depth -= 1;
        Ok(prev_var)
    }
}

fn compile_node(node: &Node) -> Result<String, String> {
    let mut emitter = Emitter::new();
    let final_var = emitter.compile_node(node)?;
    let body = emitter.lines.join("\n");
    Ok(format!(
        "void mainImage(out vec4 c, in vec2 st) {{\n{body}\n  c = {final_var};\n}}"
    ))
}

struct PatchState {
    buffers: [Option<Node>; 4],
    render_mode: RenderMode,
    text_data: Option<TextData>,
    #[cfg(feature = "webcam")]
    source_requests: Vec<SourceRequest>,
    #[cfg(feature = "audio")]
    audio_requests: Vec<AudioRequest>,
}

fn register_functions(engine: &mut Engine) {
    for meta in FUNCTIONS {
        match meta.kind {
            OpKind::Source => register_source(engine, meta),
            OpKind::Geo => register_geo(engine, meta),
            OpKind::Color => register_color(engine, meta),
            OpKind::Blend => register_blend(engine, meta),
            OpKind::Modulate => register_modulate(engine, meta),
        }
    }
}

fn register_glsl_ops(engine: &mut Engine) {
    macro_rules! binop {
        ($op:literal) => {
            engine.register_fn($op, |a: GlslExpr, b: GlslExpr| -> GlslExpr {
                GlslExpr(format!(concat!("({} ", $op, " {})"), a.0, b.0))
            });
            engine.register_fn($op, |a: GlslExpr, b: f64| -> GlslExpr {
                GlslExpr(format!(concat!("({} ", $op, " {})"), a.0, fmt_f(b)))
            });
            engine.register_fn($op, |a: f64, b: GlslExpr| -> GlslExpr {
                GlslExpr(format!(concat!("({} ", $op, " {})"), fmt_f(a), b.0))
            });
            engine.register_fn($op, |a: GlslExpr, b: i64| -> GlslExpr {
                GlslExpr(format!(concat!("({} ", $op, " {})"), a.0, fmt_f(b as f64)))
            });
            engine.register_fn($op, |a: i64, b: GlslExpr| -> GlslExpr {
                GlslExpr(format!(concat!("({} ", $op, " {})"), fmt_f(a as f64), b.0))
            });
        };
    }

    binop!("+");
    binop!("-");
    binop!("*");
    binop!("/");

    // `%` maps to GLSL's `mod()` builtin, not the `%` operator (which in
    // GLSL only applies to integers) - GLSL is float-typed throughout here.
    engine.register_fn("%", |a: GlslExpr, b: GlslExpr| -> GlslExpr {
        GlslExpr(format!("mod({}, {})", a.0, b.0))
    });
    engine.register_fn("%", |a: GlslExpr, b: f64| -> GlslExpr {
        GlslExpr(format!("mod({}, {})", a.0, fmt_f(b)))
    });
    engine.register_fn("%", |a: f64, b: GlslExpr| -> GlslExpr {
        GlslExpr(format!("mod({}, {})", fmt_f(a), b.0))
    });
    engine.register_fn("%", |a: GlslExpr, b: i64| -> GlslExpr {
        GlslExpr(format!("mod({}, {})", a.0, fmt_f(b as f64)))
    });
    engine.register_fn("%", |a: i64, b: GlslExpr| -> GlslExpr {
        GlslExpr(format!("mod({}, {})", fmt_f(a as f64), b.0))
    });

    engine.register_fn("-", |a: GlslExpr| -> GlslExpr {
        GlslExpr(format!("(-{})", a.0))
    });

    macro_rules! glsl_fn {
        ($name:literal) => {
            engine.register_fn($name, |a: GlslExpr| -> GlslExpr {
                GlslExpr(format!(concat!($name, "({})"), a.0))
            });
        };
    }

    glsl_fn!("sin");
    glsl_fn!("cos");
    glsl_fn!("tan");
    glsl_fn!("asin");
    glsl_fn!("acos");
    glsl_fn!("atan");
    glsl_fn!("abs");
    glsl_fn!("fract");
    glsl_fn!("floor");
    glsl_fn!("ceil");
    glsl_fn!("sqrt");
    glsl_fn!("sign");
    glsl_fn!("exp");
    glsl_fn!("log");

    // Plain-number overloads (no GlslExpr involved) for the same unary
    // functions - e.g. `sin(4)` with a static literal, not a reactive value.
    engine.register_fn("sin", |x: f64| -> f64 { x.sin() });
    engine.register_fn("sin", |x: i64| -> f64 { (x as f64).sin() });
    engine.register_fn("cos", |x: f64| -> f64 { x.cos() });
    engine.register_fn("cos", |x: i64| -> f64 { (x as f64).cos() });
    engine.register_fn("tan", |x: f64| -> f64 { x.tan() });
    engine.register_fn("tan", |x: i64| -> f64 { (x as f64).tan() });
    engine.register_fn("asin", |x: f64| -> f64 { x.asin() });
    engine.register_fn("asin", |x: i64| -> f64 { (x as f64).asin() });
    engine.register_fn("acos", |x: f64| -> f64 { x.acos() });
    engine.register_fn("acos", |x: i64| -> f64 { (x as f64).acos() });
    engine.register_fn("atan", |x: f64| -> f64 { x.atan() });
    engine.register_fn("atan", |x: i64| -> f64 { (x as f64).atan() });
    engine.register_fn("abs", |x: f64| -> f64 { x.abs() });
    engine.register_fn("abs", |x: i64| -> i64 { x.abs() });
    engine.register_fn("fract", |x: f64| -> f64 { x.fract() });
    engine.register_fn("fract", |x: i64| -> f64 { (x as f64).fract() });
    engine.register_fn("floor", |x: f64| -> f64 { x.floor() });
    engine.register_fn("floor", |x: i64| -> i64 { x });
    engine.register_fn("ceil", |x: f64| -> f64 { x.ceil() });
    engine.register_fn("ceil", |x: i64| -> i64 { x });
    engine.register_fn("sqrt", |x: f64| -> f64 { x.sqrt() });
    engine.register_fn("sqrt", |x: i64| -> f64 { (x as f64).sqrt() });
    engine.register_fn("sign", |x: f64| -> f64 { x.signum() });
    engine.register_fn("sign", |x: i64| -> i64 { x.signum() });
    engine.register_fn("exp", |x: f64| -> f64 { x.exp() });
    engine.register_fn("exp", |x: i64| -> f64 { (x as f64).exp() });
    engine.register_fn("log", |x: f64| -> f64 { x.ln() });
    engine.register_fn("log", |x: i64| -> f64 { (x as f64).ln() });

    macro_rules! glsl_fn2 {
        ($name:literal) => {
            engine.register_fn($name, |a: GlslExpr, b: GlslExpr| -> GlslExpr {
                GlslExpr(format!(concat!($name, "({}, {})"), a.0, b.0))
            });
            engine.register_fn($name, |a: GlslExpr, b: f64| -> GlslExpr {
                GlslExpr(format!(concat!($name, "({}, {})"), a.0, fmt_f(b)))
            });
            engine.register_fn($name, |a: f64, b: GlslExpr| -> GlslExpr {
                GlslExpr(format!(concat!($name, "({}, {})"), fmt_f(a), b.0))
            });
            engine.register_fn($name, |a: GlslExpr, b: i64| -> GlslExpr {
                GlslExpr(format!(concat!($name, "({}, {})"), a.0, fmt_f(b as f64)))
            });
            engine.register_fn($name, |a: i64, b: GlslExpr| -> GlslExpr {
                GlslExpr(format!(concat!($name, "({}, {})"), fmt_f(a as f64), b.0))
            });
        };
    }

    // Two-argument passthroughs. `atan` is registered again here for GLSL's
    // two-argument `atan(y, x)` overload (JS's `Math.atan2`), alongside the
    // one-argument form above.
    glsl_fn2!("pow");
    glsl_fn2!("min");
    glsl_fn2!("max");
    glsl_fn2!("atan");

    // Plain-number overloads (no GlslExpr involved) for the same four names.
    engine.register_fn("pow", |a: f64, b: f64| -> f64 { a.powf(b) });
    engine.register_fn("min", |a: f64, b: f64| -> f64 { a.min(b) });
    engine.register_fn("max", |a: f64, b: f64| -> f64 { a.max(b) });
    engine.register_fn("atan", |a: f64, b: f64| -> f64 { a.atan2(b) });
}

fn register_patterns(engine: &mut Engine) {
    // fast()/offset() take defaults in real hydra.js (speed=1, offset=0),
    // matching Pattern::from_array's own defaults - a 0-arg call is a no-op.
    engine.register_fn("fast", |arr: Array| -> Pattern { Pattern::from_array(arr) });
    engine.register_fn("fast", |arr: Array, speed: Dynamic| -> Pattern {
        let mut p = Pattern::from_array(arr);
        p.speed = dyn_to_f64(speed);
        p
    });
    engine.register_fn("smooth", |arr: Array| -> Pattern {
        let mut p = Pattern::from_array(arr);
        p.smooth = true;
        p
    });
    // Real hydra.js's smooth(amount) takes an interpolation-amount argument;
    // hydra-rust's Pattern only supports smoothing fully on or off, so any
    // amount just enables it (an approximation, not a faithful 1:1 port).
    engine.register_fn("smooth", |arr: Array, _amount: Dynamic| -> Pattern {
        let mut p = Pattern::from_array(arr);
        p.smooth = true;
        p
    });
    engine.register_fn("fast", |p: Pattern| -> Pattern { p });
    engine.register_fn("fast", |mut p: Pattern, speed: Dynamic| -> Pattern {
        p.speed = dyn_to_f64(speed);
        p
    });
    engine.register_fn("smooth", |mut p: Pattern| -> Pattern {
        p.smooth = true;
        p
    });
    engine.register_fn("smooth", |mut p: Pattern, _amount: Dynamic| -> Pattern {
        p.smooth = true;
        p
    });
    engine.register_fn("offset", |p: Pattern| -> Pattern { p });
    engine.register_fn("offset", |arr: Array| -> Pattern { Pattern::from_array(arr) });
    engine.register_fn("offset", |mut p: Pattern, o: Dynamic| -> Pattern {
        p.offset = dyn_to_f64(o);
        p
    });
    engine.register_fn("offset", |arr: Array, o: Dynamic| -> Pattern {
        let mut p = Pattern::from_array(arr);
        p.offset = dyn_to_f64(o);
        p
    });
    // ease(name)/fit(lo, hi) are real hydra.js pattern utilities (easing
    // curves, value-range remapping); no interpolation-curve or remapping
    // machinery exists here, so these pass the pattern through unchanged
    // rather than hard-erroring.
    engine.register_fn("ease", |arr: Array| -> Pattern { Pattern::from_array(arr) });
    engine.register_fn("ease", |p: Pattern| -> Pattern { p });
    engine.register_fn("ease", |arr: Array, _name: Dynamic| -> Pattern {
        Pattern::from_array(arr)
    });
    engine.register_fn("ease", |p: Pattern, _name: Dynamic| -> Pattern { p });
    engine.register_fn("fit", |arr: Array| -> Pattern { Pattern::from_array(arr) });
    engine.register_fn("fit", |p: Pattern| -> Pattern { p });
    engine.register_fn("fit", |arr: Array, _lo: Dynamic| -> Pattern { Pattern::from_array(arr) });
    engine.register_fn("fit", |p: Pattern, _lo: Dynamic| -> Pattern { p });
    engine.register_fn("fit", |arr: Array, _lo: Dynamic, _hi: Dynamic| -> Pattern {
        Pattern::from_array(arr)
    });
    engine.register_fn("fit", |p: Pattern, _lo: Dynamic, _hi: Dynamic| -> Pattern { p });

    // Rhai's built-in Array::reverse() mutates in place and returns unit
    // (Rust convention); JS's Array.prototype.reverse() returns the array
    // itself for chaining (`[0,1].reverse().smooth()`). Override to match.
    engine.register_fn("reverse", |mut arr: Array| -> Array {
        arr.reverse();
        arr
    });
}

pub fn eval(code: &str) -> Result<EvalResult, String> {
    let code = &quotes::rewrite_single_quoted_strings(code);
    let code = &numlit::insert_leading_zero(code);
    let code = &jskeywords::rewrite_keywords(code);
    let code = &autolet::insert_missing_let(code);
    let code = &mathjs::rewrite_math(code);
    let code = &argtrunc::truncate_extra_args(code);
    let code = &patcall::rewrite_pattern_calls(code);
    let code = &arrow::strip_zero_arg_arrows(code);
    let code = &ternary::rewrite_ternaries(code);
    let code = &asi::insert_missing_semicolons(code);
    let state = Arc::new(Mutex::new(PatchState {
        buffers: [None, None, None, None],
        render_mode: RenderMode::default(),
        text_data: None,
        #[cfg(feature = "webcam")]
        source_requests: Vec::new(),
        #[cfg(feature = "audio")]
        audio_requests: Vec::new(),
    }));

    let mut engine = Engine::new();
    engine.build_type::<Node>();
    register_functions(&mut engine);
    register_glsl_ops(&mut engine);
    register_patterns(&mut engine);

    {
        let s = state.clone();
        engine.register_fn("out", move |node: Node| {
            s.lock().unwrap().buffers[0] = Some(node);
        });
    }
    {
        let s = state.clone();
        engine.register_fn("out", move |node: Node, idx: i64| {
            let i = idx as usize;
            if i < 4 {
                s.lock().unwrap().buffers[i] = Some(node);
            }
        });
    }
    // s0-s3 constants are 100-103; indices below 100 are internal buffers
    engine.register_fn("src", idx_to_source);
    {
        let s = state.clone();
        engine.register_fn("text", move |txt: ImmutableString| -> Node {
            let data = text::rasterize(&txt);
            s.lock().unwrap().text_data = Some(data);
            Node::source("text_src", vec![])
        });
    }
    {
        let s = state.clone();
        engine.register_fn("render", move || {
            s.lock().unwrap().render_mode = RenderMode::All;
        });
    }
    {
        let s = state.clone();
        engine.register_fn("render", move |idx: i64| {
            s.lock().unwrap().render_mode = RenderMode::Single(idx as usize);
        });
    }
    {
        let s = state.clone();
        engine.register_fn("hush", move || {
            let mut st = s.lock().unwrap();
            st.buffers = [None, None, None, None];
            st.buffers[0] = Some(Node::source("solid", vec![
                Arg::Lit(0.0), Arg::Lit(0.0), Arg::Lit(0.0), Arg::Lit(1.0),
            ]));
        });
    }

    #[cfg(feature = "webcam")]
    {
        // initCam(slot) — default camera (index 0)
        {
            let s = state.clone();
            engine.register_fn("initCam", move |slot: i64| {
                if slot >= 100 {
                    let idx = (slot - 100) as usize;
                    s.lock().unwrap().source_requests.push(SourceRequest::InitCam {
                        slot: idx,
                        camera_index: 0,
                    });
                }
            });
        }
        // initCam(slot, camera_index)
        {
            let s = state.clone();
            engine.register_fn("initCam", move |slot: i64, cam: i64| {
                if slot >= 100 {
                    let idx = (slot - 100) as usize;
                    s.lock().unwrap().source_requests.push(SourceRequest::InitCam {
                        slot: idx,
                        camera_index: cam as u32,
                    });
                }
            });
        }
    }

    #[cfg(feature = "audio")]
    {
        engine.register_get("fft", |_a: &mut Audio| -> AudioFft { AudioFft });

        engine.register_indexer_get(|_f: &mut AudioFft, i: i64| -> GlslExpr {
            GlslExpr(format!("iFft[{}]", (i.max(0) as usize).min(NUM_FFT_BINS - 1)))
        });
        engine.register_indexer_get(|_f: &mut AudioFft, e: GlslExpr| -> GlslExpr {
            GlslExpr(format!("iFft[int(mod({}, {}.0))]", e.0, NUM_FFT_BINS))
        });

        {
            let s = state.clone();
            engine.register_fn("setBins", move |_a: Audio, n: Dynamic| {
                let n = dyn_to_f64(n).max(1.0) as usize;
                s.lock().unwrap().audio_requests.push(AudioRequest::SetBins(n));
            });
        }
        {
            let s = state.clone();
            engine.register_fn("setCutoff", move |_a: Audio, c: Dynamic| {
                s.lock().unwrap().audio_requests.push(AudioRequest::SetCutoff(dyn_to_f64(c) as f32));
            });
        }
        {
            let s = state.clone();
            engine.register_fn("setScale", move |_a: Audio, sc: Dynamic| {
                s.lock().unwrap().audio_requests.push(AudioRequest::SetScale(dyn_to_f64(sc) as f32));
            });
        }
        {
            let s = state.clone();
            engine.register_fn("setSmooth", move |_a: Audio, sm: Dynamic| {
                s.lock().unwrap().audio_requests.push(AudioRequest::SetSmooth(dyn_to_f64(sm) as f32));
            });
        }
        // Real hydra.js's a.show()/a.hide() toggle an on-screen debug graph
        // of the FFT bins; hydra-rust has no such overlay, so these are
        // no-ops kept only so sketches calling them still evaluate.
        engine.register_fn("show", |_a: Audio| {});
        engine.register_fn("hide", |_a: Audio| {});
    }

    // initImage/initVideo/initScreen (external image/video/display capture)
    // and setResolution have no implementation here (no image decoding,
    // video, or screen-capture pipeline, and no script-driven canvas
    // resize) - these are no-ops, logged once per call, purely so sketches
    // that call them still evaluate their other effects instead of hard
    // erroring at this line.
    engine.register_fn("initImage", |idx: i64, url: ImmutableString| {
        log::warn!("initImage({idx}, \"{url}\") ignored: image sources are not supported");
    });
    engine.register_fn("initVideo", |idx: i64, url: ImmutableString| {
        log::warn!("initVideo({idx}, \"{url}\") ignored: video sources are not supported");
    });
    engine.register_fn("initScreen", |idx: i64| {
        log::warn!("initScreen({idx}) ignored: screen capture is not supported");
    });
    engine.register_fn("initScreen", |idx: i64, screen: i64| {
        log::warn!("initScreen({idx}, {screen}) ignored: screen capture is not supported");
    });
    engine.register_fn("setResolution", |w: i64, h: i64| {
        log::warn!("setResolution({w}, {h}) ignored: script-driven resize is not supported");
    });
    engine.register_fn("screencap", || {
        log::warn!("screencap() ignored: saving a screenshot is not supported");
    });
    // Real hydra.js sketches use loadScript(url) to fetch and run an
    // extension library at runtime (custom functions, effects, etc.).
    // There's no dynamic module-loading/execution here, so this is a
    // no-op: the sketch's *other* effects still get a chance to evaluate,
    // even though whatever the extension would have defined won't exist.
    engine.register_fn("loadScript", |url: ImmutableString| {
        log::warn!("loadScript(\"{url}\") ignored: dynamic script loading is not supported");
    });

    engine.register_get("x", |_m: &mut Mouse| -> GlslExpr { GlslExpr("iMouse.x".to_string()) });
    engine.register_get("y", |_m: &mut Mouse| -> GlslExpr { GlslExpr("iMouse.y".to_string()) });
    engine.register_get("innerWidth", |_w: &mut Window| -> GlslExpr {
        GlslExpr("iResolution.x".to_string())
    });
    engine.register_get("innerHeight", |_w: &mut Window| -> GlslExpr {
        GlslExpr("iResolution.y".to_string())
    });

    let mut scope = Scope::new();
    scope.push_constant("o0", 0_i64);
    scope.push_constant("o1", 1_i64);
    scope.push_constant("o2", 2_i64);
    scope.push_constant("o3", 3_i64);
    scope.push_constant("s0", 100_i64);
    scope.push_constant("s1", 101_i64);
    scope.push_constant("s2", 102_i64);
    scope.push_constant("s3", 103_i64);
    // Pushed as a regular (non-constant) variable: real hydra.js lets
    // scripts reassign `time` (e.g. `time = 0` to reset/loop). Reassigning
    // it only rebinds the local script variable for the rest of this
    // evaluation - it doesn't reset the live iTime uniform - but that's a
    // reasonable approximation, and it's strictly better than a hard error.
    scope.push("time", GlslExpr("iTime".to_string()));
    scope.push_constant("beat", GlslExpr("iBeat".to_string()));
    scope.push_constant("tempo", GlslExpr("iTempo".to_string()));
    scope.push_constant("phase", GlslExpr("iPhase".to_string()));
    scope.push_constant("mouseX", GlslExpr("iMouse.x".to_string()));
    scope.push_constant("mouseY", GlslExpr("iMouse.y".to_string()));
    scope.push_constant("width", GlslExpr("iResolution.x".to_string()));
    scope.push_constant("height", GlslExpr("iResolution.y".to_string()));
    // Pushed as a regular (non-constant) variable, same reasoning as `a` below:
    // property-getter dispatch on a constant `Mouse`/`Window` isn't worth risking.
    scope.push("mouse", Mouse);
    scope.push("window", Window);
    // Pushed as a regular (non-constant) variable, unlike the GlslExpr constants above:
    // Rhai forbids mutable-receiver method calls on constants, and `a.setBins(...)`
    // dispatches as one even though the registered fns take `Audio` by value.
    #[cfg(feature = "audio")]
    scope.push("a", Audio);

    let result = engine
        .eval_with_scope::<Dynamic>(&mut scope, code)
        .map_err(|e| e.to_string())?;

    let mut patch = state.lock().unwrap();

    if patch.buffers.iter().all(|b| b.is_none()) && result.is::<Node>() {
        patch.buffers[0] = result.try_cast::<Node>();
    }

    let mut shaders: [Option<String>; 4] = [None, None, None, None];
    for (i, buf) in patch.buffers.iter().enumerate() {
        if let Some(node) = buf {
            shaders[i] = Some(compile_node(node)?);
        }
    }

    Ok(EvalResult {
        shaders,
        render_mode: patch.render_mode,
        text_data: patch.text_data.take(),
        #[cfg(feature = "webcam")]
        source_requests: std::mem::take(&mut patch.source_requests),
        #[cfg(feature = "audio")]
        audio_requests: std::mem::take(&mut patch.audio_requests),
    })
}

fn register_source(engine: &mut Engine, meta: &FnMeta) {
    let name = meta.name;
    let defaults = meta.defaults;
    let n = defaults.len();

    engine.register_fn(name, move || Node::source(name, fill_args(&[], defaults)));
    if n >= 1 {
        engine.register_fn(name, move |a: Dynamic| {
            Node::source(name, fill_args(&[as_arg(a)], defaults))
        });
    }
    if n >= 2 {
        engine.register_fn(name, move |a: Dynamic, b: Dynamic| {
            Node::source(name, fill_args(&[as_arg(a), as_arg(b)], defaults))
        });
    }
    if n >= 3 {
        engine.register_fn(name, move |a: Dynamic, b: Dynamic, c: Dynamic| {
            Node::source(name, fill_args(&[as_arg(a), as_arg(b), as_arg(c)], defaults))
        });
    }
    if n >= 4 {
        engine.register_fn(name, move |a: Dynamic, b: Dynamic, c: Dynamic, d: Dynamic| {
            Node::source(name, fill_args(&[as_arg(a), as_arg(b), as_arg(c), as_arg(d)], defaults))
        });
    }
}

fn register_geo(engine: &mut Engine, meta: &FnMeta) {
    let name = meta.name;
    let defaults = meta.defaults;
    let n = defaults.len();

    engine.register_fn(name, move |node: Node| node.push_geo(name, fill_args(&[], defaults)));
    if n >= 1 {
        engine.register_fn(name, move |node: Node, a: Dynamic| {
            node.push_geo(name, fill_args(&[as_arg(a)], defaults))
        });
    }
    if n >= 2 {
        engine.register_fn(name, move |node: Node, a: Dynamic, b: Dynamic| {
            node.push_geo(name, fill_args(&[as_arg(a), as_arg(b)], defaults))
        });
    }
    if n >= 3 {
        engine.register_fn(name, move |node: Node, a: Dynamic, b: Dynamic, c: Dynamic| {
            node.push_geo(name, fill_args(&[as_arg(a), as_arg(b), as_arg(c)], defaults))
        });
    }
    if n >= 4 {
        engine.register_fn(
            name,
            move |node: Node, a: Dynamic, b: Dynamic, c: Dynamic, d: Dynamic| {
                node.push_geo(name, fill_args(&[as_arg(a), as_arg(b), as_arg(c), as_arg(d)], defaults))
            },
        );
    }
    if n >= 5 {
        engine.register_fn(
            name,
            move |node: Node, a: Dynamic, b: Dynamic, c: Dynamic, d: Dynamic, e: Dynamic| {
                node.push_geo(
                    name,
                    fill_args(&[as_arg(a), as_arg(b), as_arg(c), as_arg(d), as_arg(e)], defaults),
                )
            },
        );
    }
}

fn register_color(engine: &mut Engine, meta: &FnMeta) {
    let name = meta.name;
    let defaults = meta.defaults;
    let n = defaults.len();

    engine.register_fn(name, move |node: Node| node.push_color(name, fill_args(&[], defaults)));
    if n >= 1 {
        engine.register_fn(name, move |node: Node, a: Dynamic| {
            node.push_color(name, fill_args(&[as_arg(a)], defaults))
        });
    }
    if n >= 2 {
        engine.register_fn(name, move |node: Node, a: Dynamic, b: Dynamic| {
            node.push_color(name, fill_args(&[as_arg(a), as_arg(b)], defaults))
        });
    }
    if n >= 3 {
        engine.register_fn(name, move |node: Node, a: Dynamic, b: Dynamic, c: Dynamic| {
            node.push_color(name, fill_args(&[as_arg(a), as_arg(b), as_arg(c)], defaults))
        });
    }
    if n >= 4 {
        engine.register_fn(
            name,
            move |node: Node, a: Dynamic, b: Dynamic, c: Dynamic, d: Dynamic| {
                node.push_color(
                    name,
                    fill_args(&[as_arg(a), as_arg(b), as_arg(c), as_arg(d)], defaults),
                )
            },
        );
    }
}

fn register_blend(engine: &mut Engine, meta: &FnMeta) {
    let name = meta.name;
    let defaults = meta.defaults;
    let n = defaults.len();

    engine.register_fn(name, move |node: Node, other: Dynamic| -> Result<Node, Box<rhai::EvalAltResult>> {
        Ok(node.push_blend(name, as_node(other)?, fill_args(&[], defaults)))
    });
    if n >= 1 {
        engine.register_fn(
            name,
            move |node: Node, other: Dynamic, a: Dynamic| -> Result<Node, Box<rhai::EvalAltResult>> {
                Ok(node.push_blend(name, as_node(other)?, fill_args(&[as_arg(a)], defaults)))
            },
        );
    }
}

fn register_modulate(engine: &mut Engine, meta: &FnMeta) {
    let name = meta.name;
    let defaults = meta.defaults;
    let n = defaults.len();

    engine.register_fn(name, move |node: Node, other: Dynamic| -> Result<Node, Box<rhai::EvalAltResult>> {
        Ok(node.push_modulate(name, as_node(other)?, fill_args(&[], defaults)))
    });
    if n >= 1 {
        engine.register_fn(
            name,
            move |node: Node, other: Dynamic, a: Dynamic| -> Result<Node, Box<rhai::EvalAltResult>> {
                Ok(node.push_modulate(name, as_node(other)?, fill_args(&[as_arg(a)], defaults)))
            },
        );
    }
    if n >= 2 {
        engine.register_fn(
            name,
            move |node: Node, other: Dynamic, a: Dynamic, b: Dynamic| -> Result<Node, Box<rhai::EvalAltResult>> {
                Ok(node.push_modulate(
                    name,
                    as_node(other)?,
                    fill_args(&[as_arg(a), as_arg(b)], defaults),
                ))
            },
        );
    }
    if n >= 3 {
        engine.register_fn(
            name,
            move |node: Node, other: Dynamic, a: Dynamic, b: Dynamic, c: Dynamic| -> Result<Node, Box<rhai::EvalAltResult>> {
                Ok(node.push_modulate(
                    name,
                    as_node(other)?,
                    fill_args(&[as_arg(a), as_arg(b), as_arg(c)], defaults),
                ))
            },
        );
    }
    if n >= 4 {
        engine.register_fn(
            name,
            move |node: Node, other: Dynamic, a: Dynamic, b: Dynamic, c: Dynamic, d: Dynamic| -> Result<Node, Box<rhai::EvalAltResult>> {
                Ok(node.push_modulate(
                    name,
                    as_node(other)?,
                    fill_args(&[as_arg(a), as_arg(b), as_arg(c), as_arg(d)], defaults),
                ))
            },
        );
    }
}
