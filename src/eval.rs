use std::sync::{Arc, Mutex};

use rhai::{Array, CustomType, Dynamic, Engine, ImmutableString, Map, Scope, TypeBuilder};

use crate::argtrunc;
use crate::arrow;
use crate::arrowfn;
use crate::asi;
use crate::autolet;
use crate::closurefn;
use crate::forloop;
use crate::iife;
use crate::increment;
use crate::jsfunctions;
use crate::jskeywords;
use crate::kwargs;
use crate::mathjs;
use crate::commaexpr;
use crate::commastmt;
use crate::numlit;
use crate::objlit;
use crate::patcall;
use crate::quotes;
use crate::ternary;
use crate::text::{self, TextData};
use crate::whitespace;
#[cfg(feature = "audio")]
use crate::audio::NUM_FFT_BINS;
#[cfg(feature = "midi")]
use crate::midi::{NUM_MIDI_CC, NUM_MIDI_ENVELOPES, NUM_MIDI_NOTES};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum RenderMode {
    #[default]
    Single0,
    Single(usize),
    All,
}

/// `o0-o3.setNearest()`/`.setLinear()`/`.setMode(...)` - a buffer's texture
/// sampling mode. Unlike `RenderMode` (which genuinely resets every
/// evaluation), this is meant to be sticky: real hydra.js's WebGL texture
/// object isn't recreated on a re-eval either, so not calling
/// `setNearest()`/etc. again on a later re-eval should leave whatever was
/// last set alone - see `PatchState::buffer_filter`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BufferFilter {
    Linear,
    Nearest,
}

#[cfg(any(feature = "webcam", feature = "image_url", feature = "video", feature = "stream"))]
#[derive(Debug, Clone)]
pub enum SourceRequest {
    #[cfg(feature = "webcam")]
    InitCam { slot: usize, camera_index: u32 },
    #[cfg(feature = "image_url")]
    InitImage { slot: usize, url: String },
    #[cfg(feature = "image_url")]
    InitGif { slot: usize, url: String },
    #[cfg(feature = "video")]
    InitVideo { slot: usize, url: String },
    /// `addr` is a `"host:port"` to connect to directly - see `stream.rs`'s
    /// module doc comment for why this is hydra-rust-to-hydra-rust only,
    /// not real hydra.js's own (currently broken) `initStream`.
    #[cfg(feature = "stream")]
    InitStream { slot: usize, addr: String },
}

#[cfg(feature = "audio")]
#[derive(Debug, Clone, Copy)]
pub enum AudioRequest {
    SetBins(usize),
    SetCutoff(f32),
    SetScale(f32),
    SetSmooth(f32),
    /// `a.show()`/`a.hide()` - toggles the host's on-screen FFT-bins
    /// overlay (see `HydraApp::show_audio_overlay`). Real hydra.js's own
    /// debug graph is drawn on the same canvas the visuals render to;
    /// here it's a separate egui overlay instead, since nothing else
    /// draws directly into the GL output.
    Show,
    Hide,
}

#[cfg(feature = "midi")]
#[derive(Debug, Clone, Copy)]
pub enum MidiRequest {
    Start,
    Pause,
    SetCcSmooth { index: usize, factor: f32 },
    AdsrSlot { slot: usize, note: i64, a: f32, d: f32, s: f32, r: f32 },
    /// `midi.show()`/`midi.hide()` - toggles the host's on-screen MIDI
    /// monitor overlay (see `HydraApp::show_midi_overlay`). Shows the
    /// currently-held notes/velocities and non-zero CC values rather than
    /// real hydra-midi's own scrolling raw-message log - a "current
    /// state" snapshot is simpler to implement and just as useful for
    /// confirming a controller is connected and being read correctly.
    Show,
    Hide,
}

/// `broadcastStream(port)`/`stopBroadcast()` - see `BroadcastManager`
/// (`broadcast.rs`). Mirrors `MidiRequest::Start`/`Pause`'s shape.
#[cfg(feature = "stream")]
#[derive(Debug, Clone, Copy)]
pub enum BroadcastRequest {
    Start(u16),
    Stop,
}

pub struct EvalResult {
    pub shaders: [Option<String>; 4],
    pub render_mode: RenderMode,
    pub text_data: Option<TextData>,
    /// Set only if *this* evaluation's script called `setResolution` with
    /// two statically-known numeric arguments - `None` (including for a
    /// reactive-argument call like `setResolution(window.innerWidth,
    /// window.innerHeight)`) means "no override, track the window size."
    pub render_resolution: Option<(u32, u32)>,
    /// Set only for buffers *this* evaluation's script actually called
    /// `setNearest`/`setLinear`/`setMode` on - see `BufferFilter`'s own
    /// doc comment for why this doesn't reset like `render_mode` does.
    pub buffer_filter: [Option<BufferFilter>; 4],
    #[cfg(any(feature = "webcam", feature = "image_url", feature = "video", feature = "stream"))]
    pub source_requests: Vec<SourceRequest>,
    #[cfg(feature = "audio")]
    pub audio_requests: Vec<AudioRequest>,
    #[cfg(feature = "midi")]
    pub midi_requests: Vec<MidiRequest>,
    /// Set only if *this* evaluation's script called `broadcastStream`/
    /// `stopBroadcast` - `None` if it didn't call either this time (the
    /// app-level `BroadcastManager` itself is what's actually sticky, same
    /// split as `render_resolution`).
    #[cfg(feature = "stream")]
    pub broadcast_request: Option<BroadcastRequest>,
}

/// The `a` audio object (`a.fft[i]`, `a.setBins(...)`, ...).
#[cfg(feature = "audio")]
#[derive(Debug, Clone, Copy)]
struct Audio;

/// Returned by `a.fft`; indexing it yields a `GlslExpr` reading `iFft[i]`.
#[cfg(feature = "audio")]
#[derive(Debug, Clone, Copy)]
struct AudioFft;

/// The `midi` object (`midi.start()`, `.pause()`, `.channel(n)`, ...).
#[cfg(feature = "midi")]
#[derive(Debug, Clone, Copy)]
struct Midi;

/// Returned by `note(...)`: a chainable gate (1 while held, 0 otherwise).
/// Also accepted directly wherever a `GlslExpr` is (see `as_arg`), so a bare
/// `note(60)` is usable as any hydra numeric argument.
#[cfg(feature = "midi")]
#[derive(Debug, Clone, Copy)]
struct MidiNote {
    note: i64,
}

/// Returned by `cc(...)`: a chainable, normalized (0-1) CC value.
#[cfg(feature = "midi")]
#[derive(Debug, Clone, Copy)]
struct MidiCc {
    index: i64,
}

/// Returned by `aft(...)`: a chainable, normalized (0-1) aftertouch value.
/// `note: None` is real hydra-midi's channel-wide aftertouch (MIDI status
/// `0xD0`, `aft()` called with no note argument); `note: Some(n)` is
/// per-note polyphonic key pressure (`0xA0`, `aft(60)`) - both unified
/// under one function in real hydra-midi, distinguished only by whether a
/// note argument was given.
#[cfg(feature = "midi")]
#[derive(Debug, Clone, Copy)]
struct MidiAft {
    note: Option<i64>,
}

/// The `mouse` object (`mouse.x`, `mouse.y`), matching real hydra.js.
#[derive(Debug, Clone, Copy)]
struct Mouse;

/// The `window` object (`window.innerWidth`, `window.innerHeight`), a
/// browser-DOM stand-in some sketches reference for canvas size instead of
/// (or alongside) the bare `width`/`height` globals - both map to the same
/// `iResolution` uniform here.
#[derive(Debug, Clone, Copy)]
struct Window;

/// `pb.setName("...")` (and occasionally `pb.list()`) appears, always as
/// one of the very first statements, in a large number of real sketches -
/// not a hydra.js API at all, but boilerplate some external platform
/// injects when a sketch is exported/shared (`setName`'s argument is
/// always a person's name/handle, never referenced again). No-op, purely
/// so the sketch's *actual* hydra content past this line still evaluates.
#[derive(Debug, Clone, Copy)]
struct Pb;

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
    // 0.0 = no smoothing (stepped), matching real hydra.js's
    // `arr._smooth` (falsy/unset = 0). See array-utils.js's `getValue`.
    smooth: f64,
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
        Self { values, speed: 1.0, offset: 0.0, smooth: 0.0 }
    }

    /// `.fit(low, high)`: remaps each value from the array's own
    /// [min, max] range into [low, high]. Ported from real hydra.js's
    /// array-utils.js: `map(num, in_min, in_max, out_min, out_max) =
    /// (num - in_min) * (out_max - out_min) / (in_max - in_min) + out_min`.
    /// Real hydra.js preserves `_speed`/`_smooth`/`_ease` across `.fit()`
    /// but drops `_offset` - matched here for fidelity.
    fn fit(&self, lo: f64, hi: f64) -> Self {
        let lowest = self.values.iter().cloned().fold(f64::INFINITY, f64::min);
        let highest = self.values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let span = highest - lowest;
        let values = self
            .values
            .iter()
            .map(|v| if span == 0.0 { lo } else { (v - lowest) * (hi - lo) / span + lo })
            .collect();
        Self { values, speed: self.speed, offset: 0.0, smooth: self.smooth }
    }

    fn to_glsl(&self) -> String {
        let n = self.values.len();
        if n == 0 {
            return "0.0".into();
        }
        if n == 1 {
            return fmt_f(self.values[0]);
        }

        // Real hydra.js: index = time * speed * (bpm/60) + offset (see
        // array-utils.js's getValue - the array is stepped once per
        // *beat*, not once per raw second).
        let idx = format!(
            "(iTime * {} * (iTempo / 60.0) + {})",
            fmt_f(self.speed),
            fmt_f(self.offset)
        );

        if self.smooth != 0.0 {
            // _index = index - smooth/2; currValue/nextValue are the
            // values at floor(mod(_index, n))/floor(mod(_index+1, n));
            // t = min(mod(_index, 1)/smooth, 1); linearly interpolate
            // between them (real hydra.js also allows a non-linear
            // .ease() curve here in place of the plain `t`; not yet
            // supported, so this always behaves like the 'linear' default).
            let smooth = fmt_f(self.smooth);
            let sub_idx = format!("({idx} - {smooth} / 2.0)");
            let curr = self.select_term(&format!("mod({sub_idx}, {n}.0)"));
            let next = self.select_term(&format!("mod({sub_idx} + 1.0, {n}.0)"));
            let t = format!("min(mod({sub_idx}, 1.0) / {smooth}, 1.0)");
            format!("(({t}) * (({next}) - ({curr})) + ({curr}))")
        } else {
            format!("({})", self.select_term(&format!("mod({idx}, {n}.0)")))
        }
    }

    /// `values[floor(wrapped_idx)]`, built as a sum of step-masked terms:
    /// GLSL 330 doesn't reliably support indexing an inline-constructed
    /// array as a pure expression, and `to_glsl` only ever produces one
    /// (no local variables), so this stays arithmetic rather than an
    /// actual array lookup.
    fn select_term(&self, wrapped_idx: &str) -> String {
        self.values
            .iter()
            .enumerate()
            .map(|(i, v)| format!("{} * step(abs(floor({wrapped_idx}) - {i}.0), 0.5)", fmt_f(*v)))
            .collect::<Vec<_>>()
            .join(" + ")
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
    // Ported community-extension functions (see SPEC.md §4, loadScript).
    FnMeta { name: "spiral", kind: OpKind::Source, defaults: &[1.0, 5.0, 0.1] },
    FnMeta { name: "turb", kind: OpKind::Source, defaults: &[10.0, 0.1, 3.0] },
    FnMeta { name: "uturb", kind: OpKind::Source, defaults: &[10.0, 0.1, 3.0] },
    FnMeta { name: "unoise", kind: OpKind::Source, defaults: &[10.0, 0.1] },
    FnMeta { name: "whitenoise", kind: OpKind::Source, defaults: &[10.0, 0.0] },
    FnMeta { name: "colornoise", kind: OpKind::Source, defaults: &[10.0, 0.0] },
    FnMeta { name: "warp", kind: OpKind::Source, defaults: &[10.0, 0.1, 2.0, 3.0, 1.0] },
    FnMeta { name: "cwarp", kind: OpKind::Source, defaults: &[10.0, 0.1, 2.0, 3.0, 1.0, 0.5] },
    FnMeta { name: "ncontour", kind: OpKind::Source, defaults: &[0.5, 0.1, 3.0, 5.0, 0.5, 2.0] },
    FnMeta { name: "pulse", kind: OpKind::Source, defaults: &[0.5, 0.05, 0.001] },
    FnMeta { name: "pulsetrain", kind: OpKind::Source, defaults: &[3.0, 0.5, 0.05, 0.001] },
    FnMeta { name: "hextile", kind: OpKind::Source, defaults: &[10.0] },
    FnMeta { name: "concentric", kind: OpKind::Source, defaults: &[100.0, 0.5, 0.5] },
    FnMeta { name: "brick", kind: OpKind::Source, defaults: &[0.25, 0.08, 0.01] },
    FnMeta { name: "wave", kind: OpKind::Source, defaults: &[0.0, 10.0, 3.0, 0.025] },
    FnMeta { name: "lissa", kind: OpKind::Source, defaults: &[0.0, 10.0, 3.0, 0.025] },
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
    FnMeta { name: "inversion", kind: OpKind::Geo, defaults: &[] },
    FnMeta { name: "mirrorX", kind: OpKind::Geo, defaults: &[0.0, 1.0] },
    FnMeta { name: "mirrorY", kind: OpKind::Geo, defaults: &[0.0, 1.0] },
    FnMeta { name: "mirrorX2", kind: OpKind::Geo, defaults: &[0.0, 1.0] },
    FnMeta { name: "mirrorY2", kind: OpKind::Geo, defaults: &[0.0, 1.0] },
    FnMeta { name: "mirrorWrap", kind: OpKind::Geo, defaults: &[] },
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
    FnMeta { name: "colreflect", kind: OpKind::Blend, defaults: &[1.0] },
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
        #[cfg(feature = "midi")]
        if d.is::<MidiNote>() {
            return Arg::Expr(midi_note_glsl(d.cast::<MidiNote>().note));
        } else if d.is::<MidiCc>() {
            return Arg::Expr(midi_cc_glsl(d.cast::<MidiCc>().index));
        } else if d.is::<MidiAft>() {
            return Arg::Expr(midi_aft_glsl(d.cast::<MidiAft>().note));
        }
        Arg::Lit(0.0)
    }
}

#[cfg(feature = "midi")]
fn midi_clamp(i: i64, len: usize) -> usize {
    i.clamp(0, len as i64 - 1) as usize
}

#[cfg(feature = "midi")]
fn midi_note_glsl(note: i64) -> String {
    format!("iMidiNote[{}]", midi_clamp(note, NUM_MIDI_NOTES))
}

#[cfg(feature = "midi")]
fn midi_velocity_glsl(note: i64) -> String {
    format!("iMidiVelocity[{}]", midi_clamp(note, NUM_MIDI_NOTES))
}

#[cfg(feature = "midi")]
fn midi_cc_glsl(index: i64) -> String {
    format!("iMidiCC[{}]", midi_clamp(index, NUM_MIDI_CC))
}

#[cfg(feature = "midi")]
fn midi_cc_smoothed_glsl(index: i64) -> String {
    format!("iMidiCCSmoothed[{}]", midi_clamp(index, NUM_MIDI_CC))
}

/// `note: None` -> real hydra-midi's channel-wide aftertouch; `Some(n)` ->
/// per-note polyphonic aftertouch.
#[cfg(feature = "midi")]
fn midi_aft_glsl(note: Option<i64>) -> String {
    match note {
        Some(n) => format!("iMidiAftertouch[{}]", midi_clamp(n, NUM_MIDI_NOTES)),
        None => "iMidiChannelAftertouch".to_string(),
    }
}

#[cfg(feature = "midi")]
fn midi_envelope_glsl(slot: usize) -> String {
    format!("iMidiEnvelope[{slot}]")
}

/// Real hydra-midi's `range(min=0, max=1)`: linearly remaps a value already
/// assumed to be in `[0, 1]` (every reactive value this module produces is)
/// into `[min, max]`.
#[cfg(feature = "midi")]
fn midi_range(expr: String, lo: Dynamic, hi: Dynamic) -> GlslExpr {
    let lo = fmt_f(dyn_to_f64(lo));
    let hi = fmt_f(dyn_to_f64(hi));
    GlslExpr(format!("(({expr}) * ({hi} - {lo}) + {lo})"))
}

/// Real hydra-midi's `scale(factor)`: multiplies the upstream value.
#[cfg(feature = "midi")]
fn midi_scale(expr: String, factor: Dynamic) -> GlslExpr {
    GlslExpr(format!("(({expr}) * {})", fmt_f(dyn_to_f64(factor))))
}

/// Real hydra-midi's `note(nameOrNumber)`: a plain number passes through
/// unchanged, a name like `"C4"` is parsed via standard scientific-pitch-
/// notation/General-MIDI numbering (`"C4"` -> 60, middle C) - ported from
/// `utils/getNoteNumber.ts`'s actual formula, `offset + (octave + 1) * 12`.
/// (Its own doc comment claims `"C3"` is middle C instead, but that
/// contradicts its own code - `"C3"` -> 48 by this formula, not 60 - so
/// this port follows the code, not the comment.)
#[cfg(feature = "midi")]
fn note_number_from_dynamic(d: Dynamic) -> i64 {
    if d.is::<ImmutableString>() {
        let s = d.cast::<ImmutableString>();
        note_name_to_number(&s).unwrap_or_else(|| {
            log::warn!("note(\"{s}\"): note name not recognized");
            0
        })
    } else {
        dyn_to_f64(d) as i64
    }
}

#[cfg(feature = "midi")]
fn note_name_to_number(name: &str) -> Option<i64> {
    if name.len() < 2 {
        return None;
    }
    let (letter, octave) = name.split_at(name.len() - 1);
    let octave: i64 = octave.parse().ok()?;
    let offset = match letter.to_lowercase().as_str() {
        "c" => 0,
        "c#" | "db" => 1,
        "d" => 2,
        "d#" | "eb" => 3,
        "e" => 4,
        "f" => 5,
        "f#" | "gb" => 6,
        "g" => 7,
        "g#" | "ab" => 8,
        "a" => 9,
        "a#" | "bb" => 10,
        "b" => 11,
        _ => return None,
    };
    Some(offset + (octave + 1) * 12)
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
/// `src(s0)`) as well as an already-built `Node` chain. Also accepts a
/// plain `f64` (`.mult(0.2)`, a common real-sketch idiom): real hydra.js
/// auto-promotes a bare number passed where a texture is expected into a
/// flat color, so this does too - `solid(v, v, v, 1)`, matching `solid`'s
/// own default alpha. `i64` is deliberately kept as "buffer/source index"
/// rather than also being colorized: `o0`/`s0` etc. are themselves `i64`
/// constants, and that meaning is by far the dominant real usage.
fn as_node(d: Dynamic) -> Result<Node, Box<rhai::EvalAltResult>> {
    if d.is::<Node>() {
        Ok(d.cast::<Node>())
    } else if let Ok(idx) = d.as_int() {
        Ok(idx_to_source(idx))
    } else if let Ok(v) = d.as_float() {
        Ok(Node::source("solid", vec![Arg::Lit(v), Arg::Lit(v), Arg::Lit(v), Arg::Lit(1.0)]))
    } else {
        Err(format!("expected a source or a chain, found {}", d.type_name()).into())
    }
}

fn dyn_to_f64(d: Dynamic) -> f64 {
    d.as_float().unwrap_or_else(|_| d.as_int().map(|i| i as f64).unwrap_or(0.0))
}

/// Like `dyn_to_f64`, but returns `None` for anything that isn't a plain
/// number - deliberately *not* falling back to `0.0` for a reactive
/// `GlslExpr` (e.g. `window.innerWidth`) or anything else, since a caller
/// treating that as a real, statically-known value would be wrong (see
/// `setResolution`'s registration for why this matters).
fn dyn_as_static_u32(d: &Dynamic) -> Option<u32> {
    if let Ok(v) = d.as_int() {
        Some(v.max(0) as u32)
    } else {
        d.as_float().ok().map(|v| v.max(0.0) as u32)
    }
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
    render_resolution: Option<(u32, u32)>,
    buffer_filter: [Option<BufferFilter>; 4],
    #[cfg(any(feature = "webcam", feature = "image_url", feature = "video", feature = "stream"))]
    source_requests: Vec<SourceRequest>,
    #[cfg(feature = "audio")]
    audio_requests: Vec<AudioRequest>,
    #[cfg(feature = "midi")]
    midi_requests: Vec<MidiRequest>,
    #[cfg(feature = "midi")]
    next_midi_envelope_slot: usize,
    #[cfg(feature = "stream")]
    broadcast_request: Option<BroadcastRequest>,
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

    // A handful of real sketches do arithmetic on a p5.js instance
    // property we don't populate a real value for (`p1.frameCount * 256`,
    // `mouseX - mouseY`) - reading an unregistered key off the `P5()`
    // stand-in map (see below) yields `()`, same as any other undefined
    // JS value here, and `() * i64`/`() * f64` has no operator otherwise.
    // Treated as `0`, same graceful-fallback spirit as the rest of this
    // sketch's harmless p5.js no-ops - there's no real p5 canvas rendering
    // to make these values meaningful anyway.
    engine.register_fn("*", |_a: (), _b: i64| -> i64 { 0 });
    engine.register_fn("*", |_a: i64, _b: ()| -> i64 { 0 });
    engine.register_fn("*", |_a: (), _b: f64| -> f64 { 0.0 });
    engine.register_fn("*", |_a: f64, _b: ()| -> f64 { 0.0 });

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
    // Real JS's Math.* functions coerce either argument type freely (unlike
    // Rhai, which needs a distinct overload per exact type combination), so
    // every i64/f64 mix is registered too - not just the all-f64 case.
    engine.register_fn("pow", |a: f64, b: f64| -> f64 { a.powf(b) });
    engine.register_fn("pow", |a: i64, b: i64| -> f64 { (a as f64).powf(b as f64) });
    engine.register_fn("pow", |a: f64, b: i64| -> f64 { a.powf(b as f64) });
    engine.register_fn("pow", |a: i64, b: f64| -> f64 { (a as f64).powf(b) });
    engine.register_fn("min", |a: f64, b: f64| -> f64 { a.min(b) });
    engine.register_fn("min", |a: i64, b: i64| -> f64 { (a as f64).min(b as f64) });
    engine.register_fn("min", |a: f64, b: i64| -> f64 { a.min(b as f64) });
    engine.register_fn("min", |a: i64, b: f64| -> f64 { (a as f64).min(b) });
    engine.register_fn("max", |a: f64, b: f64| -> f64 { a.max(b) });
    engine.register_fn("max", |a: i64, b: i64| -> f64 { (a as f64).max(b as f64) });
    engine.register_fn("max", |a: f64, b: i64| -> f64 { a.max(b as f64) });
    engine.register_fn("max", |a: i64, b: f64| -> f64 { (a as f64).max(b) });
    engine.register_fn("atan", |a: f64, b: f64| -> f64 { a.atan2(b) });
    engine.register_fn("atan", |a: i64, b: i64| -> f64 { (a as f64).atan2(b as f64) });
    engine.register_fn("atan", |a: f64, b: i64| -> f64 { a.atan2(b as f64) });
    engine.register_fn("atan", |a: i64, b: f64| -> f64 { (a as f64).atan2(b) });

    // `Math.random()` (rewritten to `random()` by mathjs::rewrite_math) is
    // called once at script-eval time, same as everywhere else it's used in
    // real hydra.js sketches (hydra-rust has no per-frame closures for a
    // "reactive" random() to make sense of anyway) - so a genuine one-shot
    // RNG call here is a faithful equivalent, baking a real random literal
    // into the compiled GLSL, same as JS would. Real JS's Math.random()
    // takes no arguments at all, but silently ignores any extras passed to
    // it rather than erroring (ordinary JS excess-argument tolerance) - real
    // sketches sometimes call it as if it took a range (`random(min, max)`),
    // so 1- and 2-argument overloads are registered too, matching that
    // same "ignore whatever's passed" behavior rather than implementing an
    // actual ranged random that real hydra.js doesn't have either.
    engine.register_fn("random", || -> f64 { next_random_f64() });
    engine.register_fn("random", |_a: Dynamic| -> f64 { next_random_f64() });
    engine.register_fn("random", |_a: Dynamic, _b: Dynamic| -> f64 { next_random_f64() });
}

/// One-shot, dependency-free pseudo-random `f64` in `[0, 1)`, mixed from the
/// system clock and a per-process call counter via splitmix64. Not
/// cryptographic - just needs to vary between calls and patch reloads,
/// which is all `Math.random()` is ever used for in a sketch.
fn next_random_f64() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut z = nanos.wrapping_add(count.wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

fn register_patterns(engine: &mut Engine) {
    // fast()'s default (speed=1) matches Pattern::from_array's own default,
    // so a 0-arg call is a no-op. offset()'s 0-arg default is *not* a
    // no-op - see its own registrations below.
    engine.register_fn("fast", |arr: Array| -> Pattern { Pattern::from_array(arr) });
    engine.register_fn("fast", |arr: Array, speed: Dynamic| -> Pattern {
        let mut p = Pattern::from_array(arr);
        p.speed = dyn_to_f64(speed);
        p
    });
    // Real hydra.js's smooth(amount=1) sets the interpolation window's
    // width in array-steps (array-utils.js: `this._smooth = smooth`).
    engine.register_fn("smooth", |arr: Array| -> Pattern {
        let mut p = Pattern::from_array(arr);
        p.smooth = 1.0;
        p
    });
    engine.register_fn("smooth", |arr: Array, amount: Dynamic| -> Pattern {
        let mut p = Pattern::from_array(arr);
        p.smooth = dyn_to_f64(amount);
        p
    });
    engine.register_fn("fast", |p: Pattern| -> Pattern { p });
    engine.register_fn("fast", |mut p: Pattern, speed: Dynamic| -> Pattern {
        p.speed = dyn_to_f64(speed);
        p
    });
    engine.register_fn("smooth", |mut p: Pattern| -> Pattern {
        p.smooth = 1.0;
        p
    });
    engine.register_fn("smooth", |mut p: Pattern, amount: Dynamic| -> Pattern {
        p.smooth = dyn_to_f64(amount);
        p
    });
    // Real hydra.js's offset(amount=0.5) shifts the pattern's phase by a
    // fraction of one array-step (array-utils.js reduces it `% 1.0`).
    engine.register_fn("offset", |mut p: Pattern| -> Pattern {
        p.offset = 0.5;
        p
    });
    engine.register_fn("offset", |arr: Array| -> Pattern {
        let mut p = Pattern::from_array(arr);
        p.offset = 0.5;
        p
    });
    engine.register_fn("offset", |mut p: Pattern, o: Dynamic| -> Pattern {
        p.offset = dyn_to_f64(o) % 1.0;
        p
    });
    engine.register_fn("offset", |arr: Array, o: Dynamic| -> Pattern {
        let mut p = Pattern::from_array(arr);
        p.offset = dyn_to_f64(o) % 1.0;
        p
    });
    // ease(name) is a real hydra.js pattern utility (named interpolation
    // curve); no curve machinery exists here, so it passes the pattern
    // through unchanged (smoothing still applies, just always linear)
    // rather than hard-erroring.
    engine.register_fn("ease", |arr: Array| -> Pattern { Pattern::from_array(arr) });
    engine.register_fn("ease", |p: Pattern| -> Pattern { p });
    engine.register_fn("ease", |arr: Array, _name: Dynamic| -> Pattern {
        Pattern::from_array(arr)
    });
    engine.register_fn("ease", |p: Pattern, _name: Dynamic| -> Pattern { p });
    // fit(low=0, high=1): remaps the array's own [min,max] into [low,high]
    // (real hydra.js: array-utils.js's `Array.prototype.fit`).
    engine.register_fn("fit", |arr: Array| -> Pattern { Pattern::from_array(arr).fit(0.0, 1.0) });
    engine.register_fn("fit", |p: Pattern| -> Pattern { p.fit(0.0, 1.0) });
    engine.register_fn("fit", |arr: Array, lo: Dynamic| -> Pattern {
        Pattern::from_array(arr).fit(dyn_to_f64(lo), 1.0)
    });
    engine.register_fn("fit", |p: Pattern, lo: Dynamic| -> Pattern { p.fit(dyn_to_f64(lo), 1.0) });
    engine.register_fn("fit", |arr: Array, lo: Dynamic, hi: Dynamic| -> Pattern {
        Pattern::from_array(arr).fit(dyn_to_f64(lo), dyn_to_f64(hi))
    });
    engine.register_fn("fit", |p: Pattern, lo: Dynamic, hi: Dynamic| -> Pattern {
        p.fit(dyn_to_f64(lo), dyn_to_f64(hi))
    });

    // Rhai's built-in Array::reverse() mutates in place and returns unit
    // (Rust convention); JS's Array.prototype.reverse() returns the array
    // itself for chaining (`[0,1].reverse().smooth()`). Override to match.
    engine.register_fn("reverse", |mut arr: Array| -> Array {
        arr.reverse();
        arr
    });
}

/// Runs the fixed JS-compatibility preprocessing pipeline (see SPEC.md §4)
/// and returns the resulting Rhai source, without evaluating it. Exposed
/// (undocumented) purely as a debugging/tooling aid for inspecting what a
/// given sketch looks like right before it's handed to Rhai's parser.
#[doc(hidden)]
pub fn preprocess(code: &str) -> String {
    let code = &whitespace::normalize_whitespace(code);
    let code = &quotes::rewrite_single_quoted_strings(code);
    let code = &numlit::insert_leading_zero(code);
    let code = &jskeywords::rewrite_keywords(code);
    let code = &kwargs::strip_named_args(code);
    let code = &jsfunctions::rewrite_function_decls(code);
    let code = &iife::unwrap_iife(code);
    let code = &increment::rewrite_increment_decrement(code);
    let code = &forloop::rewrite_for_loops(code);
    let code = &autolet::insert_missing_let(code);
    let code = &mathjs::rewrite_math(code);
    let code = &argtrunc::truncate_extra_args(code);
    let code = &patcall::rewrite_pattern_calls(code);
    let code = &arrow::strip_zero_arg_arrows(code);
    let code = &closurefn::rewrite_argument_position_closures(code);
    let code = &objlit::rewrite_object_literals(code);
    let code = &ternary::rewrite_ternaries(code);
    let code = &commastmt::rewrite_top_level_comma_statements(code);
    let code = &asi::insert_missing_semicolons(code);
    let code = &arrowfn::rewrite_named_arrows(code);
    commaexpr::rewrite_comma_expressions(code)
}

pub fn eval(code: &str) -> Result<EvalResult, String> {
    let code = &preprocess(code);
    let state = Arc::new(Mutex::new(PatchState {
        buffers: [None, None, None, None],
        render_mode: RenderMode::default(),
        text_data: None,
        render_resolution: None,
        buffer_filter: [None; 4],
        #[cfg(any(feature = "webcam", feature = "image_url", feature = "video", feature = "stream"))]
        source_requests: Vec::new(),
        #[cfg(feature = "audio")]
        audio_requests: Vec::new(),
        #[cfg(feature = "midi")]
        midi_requests: Vec::new(),
        #[cfg(feature = "midi")]
        next_midi_envelope_slot: 0,
        #[cfg(feature = "stream")]
        broadcast_request: None,
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
    {
        // A buffer-index argument that's a float rather than an int, e.g.
        // `.out(0.1)` (JS silently ignores extra arguments, so
        // `.out(0.1,0.7,0.5)` reaches here as just `.out(0.1)` once
        // argtrunc.rs has already dropped the rest) or `.out(o0+0.9)`
        // (arithmetic on a buffer constant instead of picking a different
        // one) - rounds and clamps into the valid 0..=3 range rather than
        // hard-erroring on a call shape real sketches do actually use.
        let s = state.clone();
        engine.register_fn("out", move |node: Node, idx: f64| {
            let i = (idx.round().clamp(0.0, 3.0)) as usize;
            s.lock().unwrap().buffers[i] = Some(node);
        });
    }
    // A chained `.out()`/`.out(idx)` immediately after another `.out(...)`
    // call (`osc(10).out(o0).out()`, `render(o0).out()`) - the first call
    // already returns `()` (its own render side effect has no further
    // chainable value), so the second one's receiver is `()` rather than a
    // Node. Harmless no-op: there is nothing further to render.
    engine.register_fn("out", |_unit: ()| {});
    engine.register_fn("out", |_unit: (), _idx: Dynamic| {});
    // A bare, receiver-less `out(...)` statement (no preceding chain at
    // all) - seen in a handful of real sketches, most plausibly leftover/
    // broken authoring rather than a real hydra.js idiom. Harmless no-op
    // rather than a hard parse-adjacent failure that would also take out
    // the rest of the script.
    engine.register_fn("out", || {
        log::warn!("out() ignored: no preceding chain to render");
    });
    engine.register_fn("out", |_idx: i64| {
        log::warn!("out(...) ignored: no preceding chain to render");
    });
    // s0-s3 constants are 100-103; indices below 100 are internal buffers
    engine.register_fn("src", idx_to_source);
    // strokeText/fillStrokeText/strokeFillText are the hydra-text.js
    // community extension's stroke/outline text variants (loadScript-
    // loaded, so otherwise entirely absent) - aliased to the exact same
    // rendering as our own text(): "accepted but not faithful" (no
    // separate stroke-vs-fill rendering mode here), the same treatment
    // already given to e.g. smooth()'s approximated interpolation curve.
    // Real hydra.js's text() and friends also accept an optional second
    // `config` argument (font/style overrides); accepted and ignored,
    // since there's no per-call font configuration here at all.
    for name in ["text", "strokeText", "fillStrokeText", "strokeFillText"] {
        let s = state.clone();
        engine.register_fn(name, move |txt: ImmutableString| -> Node {
            let data = text::rasterize(&txt);
            s.lock().unwrap().text_data = Some(data);
            Node::source("text_src", vec![])
        });
        let s = state.clone();
        engine.register_fn(name, move |txt: ImmutableString, _config: Dynamic| -> Node {
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
        // initCam(slot) — default camera (index 0). Returns the slot's
        // source Node (`src(slot)`), matching real hydra.js returning the
        // source object itself - real sketches routinely chain straight off
        // of it (`s0.initCam(0).out()`).
        {
            let s = state.clone();
            engine.register_fn("initCam", move |slot: i64| -> Node {
                if slot >= 100 {
                    let idx = (slot - 100) as usize;
                    s.lock().unwrap().source_requests.push(SourceRequest::InitCam {
                        slot: idx,
                        camera_index: 0,
                    });
                }
                idx_to_source(slot)
            });
        }
        // initCam(slot, camera_index)
        {
            let s = state.clone();
            engine.register_fn("initCam", move |slot: i64, cam: i64| -> Node {
                if slot >= 100 {
                    let idx = (slot - 100) as usize;
                    s.lock().unwrap().source_requests.push(SourceRequest::InitCam {
                        slot: idx,
                        camera_index: cam as u32,
                    });
                }
                idx_to_source(slot)
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
        // of the FFT bins - here they toggle the host's own egui overlay
        // (see `AudioRequest::Show`/`Hide`) instead of drawing into the GL
        // canvas itself.
        {
            let s = state.clone();
            engine.register_fn("show", move |_a: Audio| {
                s.lock().unwrap().audio_requests.push(AudioRequest::Show);
            });
        }
        {
            let s = state.clone();
            engine.register_fn("hide", move |_a: Audio| {
                s.lock().unwrap().audio_requests.push(AudioRequest::Hide);
            });
        }
    }

    // Real-world `hydra-midi` community extension (loadScript-loaded in
    // real hydra.js; no MIDI support exists in hydra-synth's own core at
    // all), ported natively - see src/midi.rs's module doc comment for the
    // faithfulness notes and deliberate scope reductions (merged channels/
    // inputs, `.value(fn)` unsupported).
    #[cfg(feature = "midi")]
    {
        engine.register_fn("note", |n: Dynamic| -> MidiNote {
            MidiNote { note: note_number_from_dynamic(n) }
        });
        engine.register_fn("note", |n: Dynamic, _channel: Dynamic| -> MidiNote {
            MidiNote { note: note_number_from_dynamic(n) }
        });
        engine.register_fn("note", |n: Dynamic, _channel: Dynamic, _input: Dynamic| -> MidiNote {
            MidiNote { note: note_number_from_dynamic(n) }
        });
        engine.register_fn("_note", |n: Dynamic| -> GlslExpr {
            GlslExpr(midi_note_glsl(note_number_from_dynamic(n)))
        });
        engine.register_fn("_note", |n: Dynamic, _channel: Dynamic| -> GlslExpr {
            GlslExpr(midi_note_glsl(note_number_from_dynamic(n)))
        });
        engine.register_fn("_noteVelocity", |n: Dynamic| -> GlslExpr {
            GlslExpr(midi_velocity_glsl(note_number_from_dynamic(n)))
        });
        engine.register_fn("_noteVelocity", |n: Dynamic, _channel: Dynamic| -> GlslExpr {
            GlslExpr(midi_velocity_glsl(note_number_from_dynamic(n)))
        });
        engine.register_fn("velocity", |n: MidiNote| -> GlslExpr {
            GlslExpr(midi_velocity_glsl(n.note))
        });
        {
            let s = state.clone();
            engine.register_fn(
                "adsr",
                move |n: MidiNote, a: Dynamic, d: Dynamic, sus: Dynamic, r: Dynamic| -> GlslExpr {
                    let mut st = s.lock().unwrap();
                    let slot = st.next_midi_envelope_slot;
                    st.next_midi_envelope_slot = (slot + 1) % NUM_MIDI_ENVELOPES;
                    st.midi_requests.push(MidiRequest::AdsrSlot {
                        slot,
                        note: n.note,
                        a: dyn_to_f64(a) as f32,
                        d: dyn_to_f64(d) as f32,
                        s: dyn_to_f64(sus) as f32,
                        r: dyn_to_f64(r) as f32,
                    });
                    GlslExpr(midi_envelope_glsl(slot))
                },
            );
        }

        engine.register_fn("cc", |i: Dynamic| -> MidiCc { MidiCc { index: dyn_to_f64(i) as i64 } });
        engine.register_fn("cc", |i: Dynamic, _channel: Dynamic| -> MidiCc {
            MidiCc { index: dyn_to_f64(i) as i64 }
        });
        engine.register_fn("cc", |i: Dynamic, _channel: Dynamic, _input: Dynamic| -> MidiCc {
            MidiCc { index: dyn_to_f64(i) as i64 }
        });
        engine.register_fn("_cc", |i: Dynamic| -> GlslExpr {
            GlslExpr(midi_cc_glsl(dyn_to_f64(i) as i64))
        });
        engine.register_fn("_cc", |i: Dynamic, _channel: Dynamic| -> GlslExpr {
            GlslExpr(midi_cc_glsl(dyn_to_f64(i) as i64))
        });
        {
            let s = state.clone();
            engine.register_fn("smooth", move |c: MidiCc| -> GlslExpr {
                s.lock().unwrap().midi_requests.push(MidiRequest::SetCcSmooth {
                    index: c.index as usize,
                    factor: crate::midi::DEFAULT_CC_SMOOTH,
                });
                GlslExpr(midi_cc_smoothed_glsl(c.index))
            });
        }
        {
            let s = state.clone();
            engine.register_fn("smooth", move |c: MidiCc, factor: Dynamic| -> GlslExpr {
                s.lock().unwrap().midi_requests.push(MidiRequest::SetCcSmooth {
                    index: c.index as usize,
                    factor: dyn_to_f64(factor) as f32,
                });
                GlslExpr(midi_cc_smoothed_glsl(c.index))
            });
        }

        // aft(note?, channel?, input?)/_aft(...): real hydra-midi unifies
        // channel-wide aftertouch (status 0xD0) and per-note polyphonic
        // aftertouch (0xA0) under one function, distinguished only by
        // whether a note argument was given - see MidiAft's own doc
        // comment. `channel`/`input` are accepted but ignored, same
        // merged-channel treatment `note`/`cc` already get.
        engine.register_fn("aft", || -> MidiAft { MidiAft { note: None } });
        engine.register_fn("aft", |n: Dynamic| -> MidiAft {
            MidiAft { note: Some(note_number_from_dynamic(n)) }
        });
        engine.register_fn("aft", |n: Dynamic, _channel: Dynamic| -> MidiAft {
            MidiAft { note: Some(note_number_from_dynamic(n)) }
        });
        engine.register_fn("aft", |n: Dynamic, _channel: Dynamic, _input: Dynamic| -> MidiAft {
            MidiAft { note: Some(note_number_from_dynamic(n)) }
        });
        engine.register_fn("_aft", || -> GlslExpr { GlslExpr(midi_aft_glsl(None)) });
        engine.register_fn("_aft", |n: Dynamic| -> GlslExpr {
            GlslExpr(midi_aft_glsl(Some(note_number_from_dynamic(n))))
        });
        engine.register_fn("_aft", |n: Dynamic, _channel: Dynamic| -> GlslExpr {
            GlslExpr(midi_aft_glsl(Some(note_number_from_dynamic(n))))
        });

        // .range(lo,hi)/.scale(factor): a generic linear remap/multiply,
        // registered for every reactive-value type this module can hand
        // back, matching real hydra-midi's own `range`/`scale` transforms
        // (which apply the same way regardless of what's upstream in the
        // chain).
        engine.register_fn("range", |e: GlslExpr, lo: Dynamic, hi: Dynamic| -> GlslExpr {
            midi_range(e.0, lo, hi)
        });
        engine.register_fn("range", |n: MidiNote, lo: Dynamic, hi: Dynamic| -> GlslExpr {
            midi_range(midi_note_glsl(n.note), lo, hi)
        });
        engine.register_fn("range", |c: MidiCc, lo: Dynamic, hi: Dynamic| -> GlslExpr {
            midi_range(midi_cc_glsl(c.index), lo, hi)
        });
        engine.register_fn("range", |a: MidiAft, lo: Dynamic, hi: Dynamic| -> GlslExpr {
            midi_range(midi_aft_glsl(a.note), lo, hi)
        });
        engine.register_fn("scale", |e: GlslExpr, factor: Dynamic| -> GlslExpr {
            midi_scale(e.0, factor)
        });
        engine.register_fn("scale", |n: MidiNote, factor: Dynamic| -> GlslExpr {
            midi_scale(midi_note_glsl(n.note), factor)
        });
        engine.register_fn("scale", |c: MidiCc, factor: Dynamic| -> GlslExpr {
            midi_scale(midi_cc_glsl(c.index), factor)
        });
        engine.register_fn("scale", |a: MidiAft, factor: Dynamic| -> GlslExpr {
            midi_scale(midi_aft_glsl(a.note), factor)
        });

        {
            let s = state.clone();
            // Real hydra-midi's start() returns a promise with its own
            // `.show()` (`await midi.start().show()` is the documented
            // idiom); there's no promise/async model here, so this just
            // returns `Midi` again so the same chaining still works,
            // dispatching straight to the no-op `show(Midi)` below.
            engine.register_fn("start", move |_m: Midi| -> Midi {
                s.lock().unwrap().midi_requests.push(MidiRequest::Start);
                Midi
            });
        }
        {
            let s = state.clone();
            engine.register_fn("pause", move |_m: Midi| {
                s.lock().unwrap().midi_requests.push(MidiRequest::Pause);
            });
        }
        // Real hydra-midi's midi.show()/.hide() toggle an on-screen MIDI
        // monitor overlay - here they toggle the host's own egui overlay
        // (see `MidiRequest::Show`/`Hide`) instead. `.channel(n)`/
        // `.input(n)` set script-wide defaults for per-channel/per-input
        // filtering, which this port doesn't do faithfully (see the
        // module doc comment) - accepted and ignored.
        {
            let s = state.clone();
            engine.register_fn("show", move |_m: Midi| {
                s.lock().unwrap().midi_requests.push(MidiRequest::Show);
            });
        }
        {
            let s = state.clone();
            engine.register_fn("hide", move |_m: Midi| {
                s.lock().unwrap().midi_requests.push(MidiRequest::Hide);
            });
        }
        engine.register_fn("channel", |_m: Midi, _n: Dynamic| {});
        engine.register_fn("input", |_m: Midi, _n: Dynamic| {});
    }

    // initVideo/initScreen/initGif (external video/GIF/display capture) and
    // setResolution have no implementation here (no video, GIF, or
    // screen-capture pipeline, and no script-driven canvas resize) - these
    // are no-ops, logged once per call, purely so sketches that call them
    // still evaluate their other effects instead of hard erroring at this
    // line. Real hydra.js returns the source object itself for chaining
    // (`sN.initVideo(url).out(o0)` is a common real-world shape); these
    // return the equivalent Node (`src(idx)`, reading whatever - nothing,
    // in practice - is already in that slot) so such chains keep evaluating
    // too. initImage is a real implementation when the `image_url` feature
    // is enabled (network fetch + decode, see imageload.rs) - the plain
    // no-op stub below is only registered without it.
    #[cfg(feature = "image_url")]
    {
        let s = state.clone();
        engine.register_fn("initImage", move |idx: i64, url: ImmutableString| -> Node {
            if idx >= 100 {
                s.lock().unwrap().source_requests.push(SourceRequest::InitImage {
                    slot: (idx - 100) as usize,
                    url: url.to_string(),
                });
            }
            idx_to_source(idx)
        });
    }
    #[cfg(not(feature = "image_url"))]
    engine.register_fn("initImage", |idx: i64, url: ImmutableString| -> Node {
        log::warn!("initImage({idx}, \"{url}\") ignored: image sources are not supported");
        idx_to_source(idx)
    });
    // initVideo is a real implementation when `video` is enabled - it
    // streams frames from a local file or URL via a standalone `ffmpeg`
    // subprocess (see video.rs), looping indefinitely.
    #[cfg(feature = "video")]
    {
        let s = state.clone();
        engine.register_fn("initVideo", move |idx: i64, url: ImmutableString| -> Node {
            if idx >= 100 {
                s.lock().unwrap().source_requests.push(SourceRequest::InitVideo {
                    slot: (idx - 100) as usize,
                    url: url.to_string(),
                });
            }
            idx_to_source(idx)
        });
    }
    #[cfg(not(feature = "video"))]
    engine.register_fn("initVideo", |idx: i64, url: ImmutableString| -> Node {
        log::warn!("initVideo({idx}, \"{url}\") ignored: video sources are not supported");
        idx_to_source(idx)
    });
    // initGif is a real implementation when `image_url` is enabled - it
    // reuses that feature's fetch pipeline (see imageload.rs), decoding
    // every frame up front and cycling through them by elapsed time once
    // loaded, looping indefinitely like a real animated GIF.
    #[cfg(feature = "image_url")]
    {
        let s = state.clone();
        engine.register_fn("initGif", move |idx: i64, url: ImmutableString| -> Node {
            if idx >= 100 {
                s.lock().unwrap().source_requests.push(SourceRequest::InitGif {
                    slot: (idx - 100) as usize,
                    url: url.to_string(),
                });
            }
            idx_to_source(idx)
        });
    }
    #[cfg(not(feature = "image_url"))]
    engine.register_fn("initGif", |idx: i64, url: ImmutableString| -> Node {
        log::warn!("initGif({idx}, \"{url}\") ignored: GIF sources are not supported");
        idx_to_source(idx)
    });
    // initStream is a real implementation when `stream` is enabled - it
    // connects to another hydra-rust instance's `examples/webrtc_broadcast`
    // over WebRTC (see stream.rs). `url` is reinterpreted as `"host:port"`
    // rather than real hydra.js's session-name argument - this is a
    // hydra-rust-to-hydra-rust feature, not interoperable with (currently
    // broken) real hydra.js sessions, see stream.rs's module doc comment.
    #[cfg(feature = "stream")]
    {
        let s = state.clone();
        engine.register_fn("initStream", move |idx: i64, url: ImmutableString| -> Node {
            if idx >= 100 {
                s.lock().unwrap().source_requests.push(SourceRequest::InitStream {
                    slot: (idx - 100) as usize,
                    addr: url.to_string(),
                });
            }
            idx_to_source(idx)
        });
    }
    #[cfg(not(feature = "stream"))]
    engine.register_fn("initStream", |idx: i64, url: ImmutableString| -> Node {
        log::warn!("initStream({idx}, \"{url}\") ignored: WebRTC/live-stream sources are not supported");
        idx_to_source(idx)
    });
    // broadcastStream/stopBroadcast are hydra-rust-specific - not real
    // hydra.js functions (unlike initStream, which mirrors one), so unlike
    // the exceptions listed in SPEC.md §8 there's no reason to register a
    // no-op fallback without `stream`: a script calling them without the
    // feature enabled just gets the ordinary "Function not found," the same
    // treatment `note()`/`cc()`/`midi.*` get without `midi`.
    #[cfg(feature = "stream")]
    {
        let s = state.clone();
        engine.register_fn("broadcastStream", move |port: i64| {
            s.lock().unwrap().broadcast_request = Some(BroadcastRequest::Start(port.clamp(1, 65535) as u16));
        });
        let s = state.clone();
        engine.register_fn("stopBroadcast", move || {
            s.lock().unwrap().broadcast_request = Some(BroadcastRequest::Stop);
        });
    }
    engine.register_fn("initScreen", |idx: i64| -> Node {
        log::warn!("initScreen({idx}) ignored: screen capture is not supported");
        idx_to_source(idx)
    });
    engine.register_fn("initScreen", |idx: i64, screen: i64| -> Node {
        log::warn!("initScreen({idx}, {screen}) ignored: screen capture is not supported");
        idx_to_source(idx)
    });
    // Real sketches often call this with reactive values (e.g.
    // `setResolution(window.innerWidth, window.innerHeight)`), not just
    // plain numbers - accept `Dynamic`. Only a *statically* known width/
    // height (see `dyn_as_static_u32`) sets a real override; a reactive
    // argument is deliberately treated the same as "not called" (i.e.
    // keep tracking the window size), since that's already the correct
    // behavior for the `window.innerWidth`/`innerHeight` idiom - there's
    // no per-frame callback here to re-evaluate a reactive expression
    // against, the same fundamental limit already documented for MIDI's
    // `.value(fn)`. Clamped to a sane ceiling as a light guard against a
    // copy-pasted/untrusted script requesting an enormous GPU allocation.
    {
        let s = state.clone();
        engine.register_fn("setResolution", move |w: Dynamic, h: Dynamic| {
            if let (Some(w), Some(h)) = (dyn_as_static_u32(&w), dyn_as_static_u32(&h)) {
                s.lock().unwrap().render_resolution =
                    Some((w.clamp(1, 4096), h.clamp(1, 4096)));
            }
        });
    }
    // A widely-copy-pasted community extension adds o0-o3.setNearest()/
    // .setLinear()/.setMode("nearest"|"linear") to toggle a buffer's
    // texture filtering (see `BufferFilter`, applied in renderer.rs).
    {
        let s = state.clone();
        engine.register_fn("setNearest", move |buf: i64| {
            if (0..4).contains(&buf) {
                s.lock().unwrap().buffer_filter[buf as usize] = Some(BufferFilter::Nearest);
            } else {
                log::warn!("o{buf}.setNearest() ignored: buffer index must be 0-3");
            }
        });
    }
    {
        let s = state.clone();
        engine.register_fn("setLinear", move |buf: i64| {
            if (0..4).contains(&buf) {
                s.lock().unwrap().buffer_filter[buf as usize] = Some(BufferFilter::Linear);
            } else {
                log::warn!("o{buf}.setLinear() ignored: buffer index must be 0-3");
            }
        });
    }
    {
        let s = state.clone();
        engine.register_fn("setMode", move |buf: i64, mode: ImmutableString| {
            let parsed = match mode.to_lowercase().as_str() {
                "nearest" => Some(BufferFilter::Nearest),
                "linear" => Some(BufferFilter::Linear),
                _ => {
                    log::warn!("o{buf}.setMode(\"{mode}\") ignored: expected \"nearest\" or \"linear\"");
                    None
                }
            };
            if let Some(mode) = parsed {
                if (0..4).contains(&buf) {
                    s.lock().unwrap().buffer_filter[buf as usize] = Some(mode);
                } else {
                    log::warn!("o{buf}.setMode(\"{mode:?}\") ignored: buffer index must be 0-3");
                }
            }
        });
    }
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
    // `P5`/`new P5(...)` (`new` is already stripped as a bare keyword - see
    // jskeywords.rs) constructs a p5.js instance real sketches use to draw
    // extra overlay graphics alongside the hydra visuals (see README.md's
    // "Known-broken sketches: p5.js dependency") - a whole separate
    // creative-coding framework with no Rust equivalent here. Returning a
    // plain Rhai map (like `hydraText`) rather than hard-erroring means the
    // assignment itself (`let p1 = P5(...)`) still succeeds, and later
    // property reads/writes on it (`p1.canvas`, `p1.width = ...`) are
    // harmless instead of "Variable not found" - real method calls on it
    // (`p1.createCanvas(...)`) still won't exist, same as before.
    engine.register_fn("P5", || -> Map { Map::new() });
    engine.register_fn("P5", |_config: Dynamic| -> Map { Map::new() });
    // A handful of the most commonly-called p5.js instance methods, as
    // no-ops on that same stand-in map - not an attempt at covering p5's
    // whole API (dozens of methods), just the ones seen often enough in
    // the corpus to be worth it.
    engine.register_fn("hide", |_p: Map| {});
    engine.register_fn("show", |_p: Map| {});
    engine.register_fn("textSize", |_p: Map, _size: Dynamic| {});
    engine.register_fn("fill", |_p: Map, _color: Dynamic| {});
    engine.register_fn("fill", |_p: Map, _r: Dynamic, _g: Dynamic, _b: Dynamic| {});
    engine.register_fn("stroke", |_p: Map, _color: Dynamic| {});
    engine.register_fn("stroke", |_p: Map, _r: Dynamic, _g: Dynamic, _b: Dynamic| {});
    engine.register_fn("strokeWeight", |_p: Map, _weight: Dynamic| {});
    // `sN.init({src: ...})` - not a real hydra.js API at all, but a
    // pattern some external platforms/community sketches use to feed a
    // p5.js canvas (or other DOM element) into a hydra source slot. No
    // such canvas-capture pipeline exists here, so this is a no-op that
    // still returns the slot's own source Node for chaining, same
    // treatment as `initImage`/`initVideo` without the `image_url` feature.
    engine.register_fn("init", |idx: i64, _config: Map| -> Node {
        log::warn!("init({idx}, ...) ignored: canvas/DOM element sources are not supported");
        idx_to_source(idx)
    });
    // Real hydra.js's setFunction(descriptor) registers a custom GLSL
    // source/color/combine/combineCoord function from a JS object
    // describing its name/inputs/GLSL body - no such dynamic
    // function-registration or GLSL-embedding exists here, so this is a
    // no-op: whatever function it would have defined simply won't exist
    // (surfacing as an ordinary "missing function" if called), but the
    // rest of the sketch still gets to evaluate.
    engine.register_fn("setFunction", |_descriptor: Map| {
        log::warn!("setFunction(...) ignored: custom GLSL function registration is not supported");
    });
    // Some external VJ/live-coding integrations call a bare `Scene("name")`
    // to switch between named cue banks - not a hydra.js API at all, and
    // with no such integration here, a no-op.
    engine.register_fn("Scene", |name: ImmutableString| {
        log::warn!("Scene(\"{name}\") ignored: external scene/cue integration is not supported");
    });

    engine.register_get("x", |_m: &mut Mouse| -> GlslExpr { GlslExpr("iMouse.x".to_string()) });
    engine.register_get("y", |_m: &mut Mouse| -> GlslExpr { GlslExpr("iMouse.y".to_string()) });
    engine.register_get("innerWidth", |_w: &mut Window| -> GlslExpr {
        GlslExpr("iResolution.x".to_string())
    });
    engine.register_get("innerHeight", |_w: &mut Window| -> GlslExpr {
        GlslExpr("iResolution.y".to_string())
    });
    engine.register_fn("setName", |_pb: Pb, _name: ImmutableString| {});
    engine.register_fn("list", |_pb: Pb| {});

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
    // In a real browser `window` is the global object, so `innerWidth`/
    // `innerHeight` are usable bare, without a `window.` prefix - and real
    // sketches often do exactly that.
    scope.push_constant("innerWidth", GlslExpr("iResolution.x".to_string()));
    scope.push_constant("innerHeight", GlslExpr("iResolution.y".to_string()));
    // Pushed as a regular (non-constant) variable, same reasoning as `a` below:
    // property-getter dispatch on a constant `Mouse`/`Window` isn't worth risking.
    scope.push("mouse", Mouse);
    scope.push("window", Window);
    scope.push("pb", Pb);
    // The `hydra-text.js` community extension (loaded via loadScript(),
    // itself a no-op) exposes a config object real sketches set arbitrary
    // properties on (`hydraText.font = "serif"`, `.lineWidth`, `.fontSize`,
    // ...) before calling the extension's own advanced text-rendering
    // function - which, since the extension never actually loads, still
    // won't exist. A plain Rhai object map accepts any property name with
    // no per-property registration needed, so the assignments themselves
    // at least don't hard-fail the rest of the sketch.
    scope.push("hydraText", Map::new());
    // Pushed as a regular (non-constant) variable, unlike the GlslExpr constants above:
    // Rhai forbids mutable-receiver method calls on constants, and `a.setBins(...)`
    // dispatches as one even though the registered fns take `Audio` by value.
    #[cfg(feature = "audio")]
    scope.push("a", Audio);
    #[cfg(feature = "midi")]
    scope.push("midi", Midi);

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
        render_resolution: patch.render_resolution,
        buffer_filter: patch.buffer_filter,
        #[cfg(any(feature = "webcam", feature = "image_url", feature = "video", feature = "stream"))]
        source_requests: std::mem::take(&mut patch.source_requests),
        #[cfg(feature = "audio")]
        audio_requests: std::mem::take(&mut patch.audio_requests),
        #[cfg(feature = "midi")]
        midi_requests: std::mem::take(&mut patch.midi_requests),
        #[cfg(feature = "stream")]
        broadcast_request: patch.broadcast_request,
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
    if n >= 5 {
        engine.register_fn(name, move |a: Dynamic, b: Dynamic, c: Dynamic, d: Dynamic, e: Dynamic| {
            Node::source(
                name,
                fill_args(&[as_arg(a), as_arg(b), as_arg(c), as_arg(d), as_arg(e)], defaults),
            )
        });
    }
    if n >= 6 {
        engine.register_fn(
            name,
            move |a: Dynamic, b: Dynamic, c: Dynamic, d: Dynamic, e: Dynamic, f: Dynamic| {
                Node::source(
                    name,
                    fill_args(
                        &[as_arg(a), as_arg(b), as_arg(c), as_arg(d), as_arg(e), as_arg(f)],
                        defaults,
                    ),
                )
            },
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- fmt_f ---

    #[test]
    fn fmt_f_appends_point_zero_to_whole_numbers() {
        assert_eq!(fmt_f(1.0), "1.0");
        assert_eq!(fmt_f(-2.0), "-2.0");
        assert_eq!(fmt_f(0.0), "0.0");
    }

    #[test]
    fn fmt_f_leaves_fractional_numbers_alone() {
        assert_eq!(fmt_f(0.1), "0.1");
        assert_eq!(fmt_f(-0.5), "-0.5");
        assert_eq!(fmt_f(60.25), "60.25");
    }

    // --- dyn_as_static_u32 ---

    #[test]
    fn dyn_as_static_u32_accepts_plain_numbers() {
        assert_eq!(dyn_as_static_u32(&Dynamic::from(320_i64)), Some(320));
        assert_eq!(dyn_as_static_u32(&Dynamic::from(240.0_f64)), Some(240));
    }

    #[test]
    fn dyn_as_static_u32_rejects_a_reactive_expression() {
        assert_eq!(dyn_as_static_u32(&Dynamic::from(GlslExpr("iResolution.x".into()))), None);
    }

    // --- fill_args ---

    #[test]
    fn fill_args_appends_missing_defaults() {
        let filled = fill_args(&[Arg::Lit(1.0)], &[1.0, 2.0, 3.0]);
        assert_eq!(filled.len(), 3);
        assert!(matches!(filled[1], Arg::Lit(v) if v == 2.0));
        assert!(matches!(filled[2], Arg::Lit(v) if v == 3.0));
    }

    #[test]
    fn fill_args_leaves_fully_provided_args_alone() {
        let filled = fill_args(&[Arg::Lit(9.0), Arg::Lit(8.0)], &[1.0, 2.0]);
        assert!(matches!(filled[0], Arg::Lit(v) if v == 9.0));
        assert!(matches!(filled[1], Arg::Lit(v) if v == 8.0));
    }

    #[test]
    fn fill_args_handles_more_provided_than_defaults() {
        // shouldn't happen via the registered arity-limited overloads, but
        // fill_args itself must not panic if it ever does
        let filled = fill_args(&[Arg::Lit(1.0), Arg::Lit(2.0), Arg::Lit(3.0)], &[1.0]);
        assert_eq!(filled.len(), 3);
    }

    // --- idx_to_source ---

    #[test]
    fn idx_to_source_below_100_reads_internal_buffer() {
        let node = idx_to_source(2);
        match &node.ops[..] {
            [Op::Source { func, args }] => {
                assert_eq!(*func, "src");
                assert!(matches!(args[0], Arg::Lit(v) if v == 2.0));
            }
            other => panic!("unexpected ops: {other:?}"),
        }
    }

    #[test]
    fn idx_to_source_at_99_is_still_internal_buffer() {
        let node = idx_to_source(99);
        match &node.ops[..] {
            [Op::Source { func, .. }] => assert_eq!(*func, "src"),
            other => panic!("unexpected ops: {other:?}"),
        }
    }

    #[test]
    fn idx_to_source_at_100_switches_to_external_source() {
        let node = idx_to_source(100);
        match &node.ops[..] {
            [Op::Source { func, args }] => {
                assert_eq!(*func, "ext_src");
                assert!(matches!(args[0], Arg::Lit(v) if v == 0.0));
            }
            other => panic!("unexpected ops: {other:?}"),
        }
    }

    #[test]
    fn idx_to_source_103_is_external_source_slot_3() {
        let node = idx_to_source(103);
        match &node.ops[..] {
            [Op::Source { func, args }] => {
                assert_eq!(*func, "ext_src");
                assert!(matches!(args[0], Arg::Lit(v) if v == 3.0));
            }
            other => panic!("unexpected ops: {other:?}"),
        }
    }

    // --- as_node ---

    #[test]
    fn as_node_promotes_a_bare_float_to_a_flat_color() {
        // real hydra.js idiom: `.mult(0.2)` auto-promotes the bare number
        // into a flat-color texture rather than requiring an explicit
        // `solid(...)` chain.
        let node = as_node(Dynamic::from_float(0.2)).unwrap();
        match &node.ops[..] {
            [Op::Source { func, args }] => {
                assert_eq!(*func, "solid");
                assert!(matches!(args[0], Arg::Lit(v) if v == 0.2));
                assert!(matches!(args[1], Arg::Lit(v) if v == 0.2));
                assert!(matches!(args[2], Arg::Lit(v) if v == 0.2));
                assert!(matches!(args[3], Arg::Lit(v) if v == 1.0));
            }
            other => panic!("unexpected ops: {other:?}"),
        }
    }

    #[test]
    fn as_node_still_treats_int_as_a_buffer_index_not_a_color() {
        // o0/s0 etc. are i64 constants - that meaning must take priority
        // over the new float-to-color promotion.
        let node = as_node(Dynamic::from_int(2)).unwrap();
        match &node.ops[..] {
            [Op::Source { func, .. }] => assert_eq!(*func, "src"),
            other => panic!("unexpected ops: {other:?}"),
        }
    }

    #[test]
    fn as_node_passes_through_an_existing_node() {
        let inner = Node::source("noise", vec![]);
        let node = as_node(Dynamic::from(inner)).unwrap();
        match &node.ops[..] {
            [Op::Source { func, .. }] => assert_eq!(*func, "noise"),
            other => panic!("unexpected ops: {other:?}"),
        }
    }

    // --- Pattern::to_glsl ---

    #[test]
    fn pattern_to_glsl_empty_is_zero() {
        assert_eq!(Pattern::from_array(Array::new()).to_glsl(), "0.0");
    }

    #[test]
    fn pattern_to_glsl_single_value_is_a_plain_literal() {
        let arr: Array = vec![Dynamic::from_float(0.5)];
        assert_eq!(Pattern::from_array(arr).to_glsl(), "0.5");
    }

    #[test]
    fn pattern_to_glsl_multi_value_uses_step_function_selection() {
        let arr: Array = vec![Dynamic::from_float(1.0), Dynamic::from_float(2.0)];
        let glsl = Pattern::from_array(arr).to_glsl();
        assert!(glsl.contains("iTime * 1.0 * (iTempo / 60.0)"), "{glsl}");
        assert!(glsl.contains("1.0 * step("), "{glsl}");
        assert!(glsl.contains("2.0 * step("), "{glsl}");
    }

    #[test]
    fn pattern_to_glsl_smooth_interpolates_between_current_and_next_value() {
        let mut p = Pattern::from_array(vec![Dynamic::from_float(1.0), Dynamic::from_float(2.0)]);
        p.smooth = 1.0;
        let glsl = p.to_glsl();
        // linear-interpolation shape: t * (next - curr) + curr, with both
        // curr/next themselves step-selected from the array's values.
        assert!(glsl.contains("min(mod("), "{glsl}");
        assert!(glsl.contains(" - 1.0 / 2.0)"), "{glsl}");
        assert!(glsl.contains("1.0 * step("), "{glsl}");
        assert!(glsl.contains("2.0 * step("), "{glsl}");
    }

    #[test]
    fn pattern_to_glsl_zero_smooth_amount_falls_back_to_stepped() {
        // real hydra.js treats `_smooth == 0` as falsy/off, same as never
        // having called .smooth() at all.
        let mut p = Pattern::from_array(vec![Dynamic::from_float(1.0), Dynamic::from_float(2.0)]);
        p.smooth = 0.0;
        let glsl = p.to_glsl();
        assert!(!glsl.contains("min(mod("), "{glsl}");
    }

    #[test]
    fn pattern_to_glsl_nonzero_offset_appears_in_the_time_expression() {
        let mut p = Pattern::from_array(vec![Dynamic::from_float(1.0), Dynamic::from_float(2.0)]);
        p.offset = 0.25;
        let glsl = p.to_glsl();
        assert!(glsl.contains("+ 0.25"), "{glsl}");
    }

    #[test]
    fn eval_offset_with_no_args_defaults_to_half_a_step() {
        let result = eval("osc(60, [1, 2].offset(), 0).out()").unwrap();
        let glsl = result.shaders[0].as_ref().unwrap();
        assert!(glsl.contains("+ 0.5"), "{glsl}");
    }

    #[test]
    fn eval_offset_reduces_its_argument_modulo_one_step() {
        let result = eval("osc(60, [1, 2].offset(1.25), 0).out()").unwrap();
        let glsl = result.shaders[0].as_ref().unwrap();
        assert!(glsl.contains("+ 0.25"), "{glsl}");
    }

    #[test]
    fn pattern_fit_remaps_values_into_the_given_range() {
        let arr: Array = vec![Dynamic::from_float(0.0), Dynamic::from_float(5.0), Dynamic::from_float(10.0)];
        let p = Pattern::from_array(arr).fit(0.0, 1.0);
        assert_eq!(p.values, vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn pattern_fit_preserves_speed_and_smooth_but_resets_offset() {
        let mut p = Pattern::from_array(vec![Dynamic::from_float(0.0), Dynamic::from_float(10.0)]);
        p.speed = 2.0;
        p.smooth = 1.0;
        p.offset = 0.5;
        let fitted = p.fit(0.0, 1.0);
        assert_eq!(fitted.speed, 2.0);
        assert_eq!(fitted.smooth, 1.0);
        assert_eq!(fitted.offset, 0.0);
    }

    #[test]
    fn pattern_fit_on_a_constant_array_does_not_divide_by_zero() {
        let arr: Array = vec![Dynamic::from_float(3.0), Dynamic::from_float(3.0)];
        let p = Pattern::from_array(arr).fit(2.0, 4.0);
        assert_eq!(p.values, vec![2.0, 2.0]);
    }

    // --- compile_node (GLSL codegen) ---

    #[test]
    fn compile_node_emits_a_plain_source_call() {
        let node = Node::source("osc", vec![Arg::Lit(60.0), Arg::Lit(0.1), Arg::Lit(0.0)]);
        let glsl = compile_node(&node).unwrap();
        assert!(glsl.contains("fn mainImage(") || glsl.contains("void mainImage("), "{glsl}");
        assert!(glsl.contains("osc(st, 60.0, 0.1, 0.0)"), "{glsl}");
    }

    #[test]
    fn compile_node_applies_geo_before_the_source_samples_it() {
        let node = Node::source("osc", vec![]).push_geo("rotate", vec![Arg::Lit(10.0)]);
        let glsl = compile_node(&node).unwrap();
        // the geo transform must run first, producing a new `st`-like var
        // that the source then samples with instead of the original `st`
        assert!(glsl.contains("rotate(st, 10.0)"), "{glsl}");
        assert!(glsl.contains("osc(_st0"), "{glsl}");
    }

    #[test]
    fn compile_node_applies_color_after_the_source() {
        let node = Node::source("osc", vec![]).push_color("invert", vec![Arg::Lit(1.0)]);
        let glsl = compile_node(&node).unwrap();
        let osc_pos = glsl.find("osc(").unwrap();
        let invert_pos = glsl.find("invert(").unwrap();
        assert!(osc_pos < invert_pos, "{glsl}");
    }

    #[test]
    fn compile_node_blend_compiles_the_other_chain_and_feeds_it_in() {
        let a = Node::source("osc", vec![]);
        let b = Node::source("noise", vec![]);
        let node = a.push_blend("add", b, vec![Arg::Lit(1.0)]);
        let glsl = compile_node(&node).unwrap();
        assert!(glsl.contains("osc(st)"), "{glsl}");
        assert!(glsl.contains("noise(st)"), "{glsl}");
        assert!(glsl.contains("add("), "{glsl}");
    }

    #[test]
    fn compile_node_modulate_compiles_the_other_chain_and_feeds_it_in() {
        let a = Node::source("osc", vec![]);
        let b = Node::source("noise", vec![]);
        let node = a.push_modulate("modulate", b, vec![Arg::Lit(0.1)]);
        let glsl = compile_node(&node).unwrap();
        assert!(glsl.contains("noise(st)"), "{glsl}");
        assert!(glsl.contains("modulate("), "{glsl}");
    }

    #[test]
    fn compile_node_src_reads_the_matching_buffer_index() {
        let node = Node::source("src", vec![Arg::Lit(2.0)]);
        let glsl = compile_node(&node).unwrap();
        assert!(glsl.contains("iBuffer2"), "{glsl}");
    }

    #[test]
    fn compile_node_src_clamps_out_of_range_buffer_index() {
        let node = Node::source("src", vec![Arg::Lit(7.0)]);
        let glsl = compile_node(&node).unwrap();
        assert!(glsl.contains("iBuffer3"), "{glsl}");
    }

    #[test]
    fn compile_node_ext_src_reads_the_matching_source_slot() {
        let node = Node::source("ext_src", vec![Arg::Lit(1.0)]);
        let glsl = compile_node(&node).unwrap();
        assert!(glsl.contains("iSource1"), "{glsl}");
    }

    #[test]
    fn compile_node_text_src_reads_the_text_texture() {
        let node = Node::source("text_src", vec![]);
        let glsl = compile_node(&node).unwrap();
        assert!(glsl.contains("iText0"), "{glsl}");
    }

    #[test]
    fn compile_node_rejects_a_chain_with_two_sources() {
        let node = Node { ops: vec![
            Op::Source { func: "osc", args: vec![] },
            Op::Source { func: "noise", args: vec![] },
        ] };
        let err = compile_node(&node).unwrap_err();
        assert!(err.contains("exactly one source"), "{err}");
    }

    #[test]
    fn compile_node_rejects_a_chain_with_no_source() {
        let node = Node { ops: vec![Op::Color { func: "invert", args: vec![] }] };
        let err = compile_node(&node).unwrap_err();
        assert!(err.contains("must start with a source"), "{err}");
    }

    #[test]
    fn compile_node_enforces_max_nesting_depth() {
        fn make_deep_chain(depth: usize) -> Node {
            let base = Node::source("osc", vec![]);
            if depth == 0 {
                return base;
            }
            base.push_blend("add", make_deep_chain(depth - 1), vec![])
        }
        let shallow = make_deep_chain(5);
        assert!(compile_node(&shallow).is_ok());

        let too_deep = make_deep_chain(20);
        let err = compile_node(&too_deep).unwrap_err();
        assert!(err.contains("nesting too deep"), "{err}");
    }

    // --- note_name_to_number (MIDI note names) ---

    #[test]
    #[cfg(feature = "midi")]
    fn note_name_to_number_matches_general_midi_numbering() {
        // C4 = 60 = middle C, standard scientific-pitch-notation/General-
        // MIDI numbering - see note_number_from_dynamic's doc comment for
        // why this (not real hydra-midi's own doc comment, which
        // contradicts its own code) is the source of truth.
        assert_eq!(note_name_to_number("C4"), Some(60));
        assert_eq!(note_name_to_number("A4"), Some(69));
        assert_eq!(note_name_to_number("C0"), Some(12));
    }

    #[test]
    #[cfg(feature = "midi")]
    fn note_name_to_number_handles_sharps_and_flats() {
        assert_eq!(note_name_to_number("C#4"), Some(61));
        assert_eq!(note_name_to_number("Db4"), Some(61));
        assert_eq!(note_name_to_number("Gb3"), Some(54));
    }

    #[test]
    #[cfg(feature = "midi")]
    fn note_name_to_number_is_case_insensitive() {
        assert_eq!(note_name_to_number("c4"), Some(60));
        assert_eq!(note_name_to_number("a#4"), Some(70));
    }

    #[test]
    #[cfg(feature = "midi")]
    fn note_name_to_number_rejects_unrecognized_names() {
        assert_eq!(note_name_to_number("H4"), None);
        assert_eq!(note_name_to_number(""), None);
        assert_eq!(note_name_to_number("C"), None);
    }

    #[test]
    #[cfg(feature = "midi")]
    fn note_number_from_dynamic_passes_bare_numbers_through() {
        assert_eq!(note_number_from_dynamic(Dynamic::from_int(60)), 60);
        assert_eq!(note_number_from_dynamic(Dynamic::from_float(60.0)), 60);
    }
}
