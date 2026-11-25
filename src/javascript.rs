//! Persistent JavaScript execution for shader graphs.
//!
//! Failed evaluations preserve graphs and callbacks, but cannot undo JavaScript global mutations.
//! Evaluations reuse inactive callback slots and protect slots referenced by the previous graph.
//! Native device requests apply only after the host accepts the compiled graph.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::context::EvalOptions;
use rquickjs::{Context, Ctx, Exception, Function, Module, Object, Persistent, Runtime};
use serde::{Deserialize, Serialize};

use crate::eval::{
    Arg, BufferFilter, EvalResult, FUNCTIONS, Node, Op, OpKind, RenderMode, compile_node,
};
pub use crate::javascript_settings::JavascriptSettings;
pub use crate::shader::NUM_JS_VALUES;

mod library;
mod native;
use library::Libraries;
type Rejections = Rc<RefCell<HashMap<u64, Vec<String>>>>;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct JavascriptFrame {
    pub time: f32,
    pub resolution: [f32; 2],
    pub mouse: [f32; 2],
    pub beat: f32,
    pub tempo: f32,
    pub phase: f32,
    pub fft: [f32; 8],
    pub audio_onset: f32,
    pub audio_beat: f32,
    pub audio_bpm: f32,
    #[cfg(feature = "midi")]
    #[serde(serialize_with = "serialize_array")]
    pub midi_values: [f32; crate::midi::NUM_MIDI_VALUES],
    #[cfg(feature = "midi")]
    pub midi_envelope: [f32; crate::midi::NUM_MIDI_ENVELOPES],
    #[cfg(feature = "osc")]
    pub osc: [f32; crate::osc::NUM_OSC_SLOTS],
}

#[cfg(feature = "midi")]
fn serialize_array<S: serde::Serializer, const N: usize>(
    values: &[f32; N],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    values.as_slice().serialize(serializer)
}

impl Default for JavascriptFrame {
    fn default() -> Self {
        Self {
            time: 0.0,
            resolution: [0.0; 2],
            mouse: [0.0; 2],
            beat: 0.0,
            tempo: 0.0,
            phase: 0.0,
            fft: [0.0; 8],
            audio_onset: 0.0,
            audio_beat: 0.0,
            audio_bpm: 0.0,
            #[cfg(feature = "midi")]
            midi_values: [0.0; crate::midi::NUM_MIDI_VALUES],
            #[cfg(feature = "midi")]
            midi_envelope: [0.0; crate::midi::NUM_MIDI_ENVELOPES],
            #[cfg(feature = "osc")]
            osc: [0.0; crate::osc::NUM_OSC_SLOTS],
        }
    }
}

pub struct JavascriptRuntime {
    context: Context,
    runtime: Runtime,
    deadline: Arc<Mutex<Instant>>,
    uses_audio: bool,
    previous_audio: bool,
    can_rollback: bool,
    rejections: Rejections,
    jobs_blocked: bool,
    library_root: PathBuf,
    settings: JavascriptSettings,
    evaluations: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    buffers: [Option<Graph>; 4],
    definitions: Vec<Definition>,
    render: Option<usize>,
    all: bool,
    resolution: Option<[u32; 2]>,
    filters: [Option<String>; 4],
    screencap: bool,
    slots: usize,
    audio: bool,
    text: Option<TextRequest>,
    clear: [bool; 4],
    commands: Vec<native::Command>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextRequest {
    content: String,
    mode: String,
    font_size: String,
    line_width: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    name: String,
    kind: String,
    inputs: Vec<Input>,
    glsl: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    name: String,
    default: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Graph {
    ops: Vec<Operation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    name: String,
    args: Vec<Value>,
    other: Option<Box<Graph>>,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "value", deny_unknown_fields)]
enum Value {
    #[serde(rename = "number")]
    Number(f64),
    #[serde(rename = "callback")]
    Callback(usize),
}

fn js_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if error.is_exception() {
        exception_text(ctx.catch())
    } else {
        error.to_string()
    }
}

fn exception_text(exception: rquickjs::Value<'_>) -> String {
    if let Some(object) = exception.clone().into_object()
        && let Ok(message) = object.get::<_, String>("message")
    {
        let stack = object.get::<_, String>("stack").unwrap_or_default();
        return format!("{message}\n{stack}");
    }
    format!("{exception:?}")
}

fn requires_module(ctx: &Ctx<'_>, code: &str, filename: &Path) -> Result<bool, String> {
    let code = CString::new(code).map_err(|error| error.to_string())?;
    let filename =
        CString::new(filename.to_string_lossy().as_bytes()).map_err(|error| error.to_string())?;
    let flags = rquickjs::qjs::JS_EVAL_TYPE_GLOBAL
        | rquickjs::qjs::JS_EVAL_FLAG_STRICT
        | rquickjs::qjs::JS_EVAL_FLAG_COMPILE_ONLY;
    // Compile in the existing limited context without executing or creating another interpreter.
    let failed = unsafe {
        let value = rquickjs::qjs::JS_Eval(
            ctx.as_raw().as_ptr(),
            code.as_ptr(),
            code.as_bytes().len() as _,
            filename.as_ptr(),
            flags as _,
        );
        let failed = rquickjs::qjs::JS_IsException(value);
        rquickjs::qjs::JS_FreeValue(ctx.as_raw().as_ptr(), value);
        failed
    };
    if failed {
        let exception = ctx.catch();
        if exception
            .as_object()
            .and_then(|object| object.get::<_, String>("name").ok())
            .as_deref()
            == Some("SyntaxError")
        {
            Ok(true)
        } else {
            Err(exception_text(exception))
        }
    } else {
        Ok(false)
    }
}

impl JavascriptRuntime {
    pub fn new(library_root: PathBuf) -> Result<Self, String> {
        Self::with_settings(library_root, JavascriptSettings::default())
    }

    pub fn with_settings(
        library_root: PathBuf,
        settings: JavascriptSettings,
    ) -> Result<Self, String> {
        settings.validate()?;
        let root = library_root
            .canonicalize()
            .map_err(|e| format!("library root: {e}"))?;
        if !root.is_dir() {
            return Err("library root must be a directory".into());
        }
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(settings.memory_mib * 1024 * 1024);
        runtime.set_max_stack_size(settings.stack_kib * 1024);
        let libraries = Libraries::new(root.clone(), settings.clone());
        runtime.set_loader(libraries.clone(), libraries.clone());
        let rejections: Rejections = Rc::new(RefCell::new(HashMap::new()));
        let tracked = rejections.clone();
        runtime.set_host_promise_rejection_tracker(Some(Box::new(
            move |_ctx, promise, reason, handled| {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                promise.hash(&mut hash);
                let id = hash.finish();
                let mut rejected = tracked.borrow_mut();
                if handled {
                    if let Some(messages) = rejected.get_mut(&id) {
                        messages.pop();
                        if messages.is_empty() {
                            rejected.remove(&id);
                        }
                    }
                } else {
                    let mut message = reason
                        .as_object()
                        .and_then(|o| o.get::<_, String>("message").ok())
                        .or_else(|| reason.as_string().and_then(|s| s.to_string().ok()))
                        .unwrap_or_else(|| format!("{reason:?}"));
                    if let Some(stack) = reason
                        .as_object()
                        .and_then(|object| object.get::<_, String>("stack").ok())
                    {
                        message.push('\n');
                        message.push_str(&stack);
                    }
                    rejected.entry(id).or_default().push(message);
                }
            },
        )));
        let deadline = Arc::new(Mutex::new(
            Instant::now() + Duration::from_millis(settings.execution_ms),
        ));
        let interrupt = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            Instant::now() > *interrupt.lock().unwrap()
        })));
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        context.with(|ctx| {
            let load = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, path: String| -> rquickjs::Result<()> {
                    let base = libraries
                        .loading
                        .borrow()
                        .last()
                        .and_then(|p| p.parent())
                        .unwrap_or(&libraries.root)
                        .to_path_buf();
                    let path = libraries.resolve_path(&base, &path).map_err(|error| {
                        Exception::throw_message(&ctx, &format!("loadScript: {error}"))
                    })?;
                    if libraries.loading.borrow().contains(&path)
                        || libraries.loading.borrow().len() >= 16
                    {
                        return Err(Exception::throw_message(
                            &ctx,
                            "loadScript cycle or nesting limit",
                        ));
                    }
                    let code = libraries.read(&path).map_err(|error| {
                        Exception::throw_message(&ctx, &format!("loadScript: {error}"))
                    })?;
                    let mut options = EvalOptions::default();
                    options.filename = Some(path.to_string_lossy().into_owned());
                    libraries.loading.borrow_mut().push(path);
                    let result = ctx.eval_with_options::<(), _>(code, options);
                    libraries.loading.borrow_mut().pop();
                    result
                },
            )
            .map_err(|e| js_error(&ctx, e))?;
            ctx.globals()
                .set("loadScript", load)
                .map_err(|e| js_error(&ctx, e))?;
            let console = Function::new(
                ctx.clone(),
                |ctx: Ctx<'_>, level: String, message: String| {
                    match level.as_str() {
                        "debug" => log::debug!(target: "hydra_rust::javascript", "{message}"),
                        "info" => log::info!(target: "hydra_rust::javascript", "{message}"),
                        "warn" => log::warn!(target: "hydra_rust::javascript", "{message}"),
                        "error" => log::error!(target: "hydra_rust::javascript", "{message}"),
                        _ => {
                            return Err(Exception::throw_message(
                                &ctx,
                                "Unsupported console level",
                            ));
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|error| js_error(&ctx, error))?;
            ctx.globals()
                .set("__consoleLog", console)
                .map_err(|error| js_error(&ctx, error))?;
            let metadata: Vec<_> = FUNCTIONS
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "name": m.name,
                        "kind": kind_name(m.kind),
                        "defaults": m.defaults,
                    })
                })
                .collect();
            ctx.eval::<(), _>(format!(
                "globalThis.__metadata = {};",
                serde_json::to_string(&metadata).unwrap()
            ))
            .map_err(|e| js_error(&ctx, e))?;
            ctx.eval::<(), _>(format!("globalThis.__features = {};", serde_json::json!({
                "webcam": cfg!(feature = "webcam"), "image_url": cfg!(feature = "image_url"),
                "video": cfg!(feature = "video"), "stream": cfg!(feature = "stream"),
                "fft": cfg!(feature = "fft"), "midi": cfg!(feature = "midi"), "osc": cfg!(feature = "osc")
            }))).map_err(|e| js_error(&ctx, e))?;
            #[cfg(feature = "fft")]
            ctx.eval::<(), _>(format!("globalThis.__audioSettings = {};", native::audio_settings(&crate::audio::AudioSettings::default())))
                .map_err(|e| js_error(&ctx, e))?;
            ctx.eval::<(), _>(include_str!("javascript/bridge.js"))
                .map_err(|e| js_error(&ctx, e))
        })?;
        Ok(Self {
            context,
            runtime,
            deadline,
            uses_audio: false,
            previous_audio: false,
            can_rollback: false,
            rejections,
            jobs_blocked: false,
            library_root: root,
            settings,
            evaluations: 0,
        })
    }

    fn start(&self) {
        *self.deadline.lock().unwrap() =
            Instant::now() + Duration::from_millis(self.settings.execution_ms);
    }

    fn run(&self, code: &str) -> Result<String, String> {
        self.context
            .with(|ctx| ctx.eval::<String, _>(code).map_err(|e| js_error(&ctx, e)))
    }

    fn jobs(&self) -> Result<(), String> {
        for _ in 0..self.settings.pending_jobs {
            match self.runtime.execute_pending_job() {
                Ok(false) => return self.rejection_error(),
                Ok(true) => {}
                Err(e) => return Err(e.0.with(|ctx| js_error(&ctx, rquickjs::Error::Exception))),
            }
        }
        if self.runtime.is_job_pending() {
            Err("JavaScript pending job limit exceeded".into())
        } else {
            self.rejection_error()
        }
    }

    fn rejection_error(&self) -> Result<(), String> {
        let mut rejections = self.rejections.borrow_mut();
        let error = rejections
            .values()
            .find_map(|messages| messages.first())
            .map(|reason| format!("unhandled JavaScript rejection: {reason}"));
        rejections.clear();
        error.map_or(Ok(()), Err)
    }

    fn cancel_jobs(&self) {
        *self.deadline.lock().unwrap() = Instant::now() - Duration::from_secs(1);
        for _ in 0..4096 {
            match self.runtime.execute_pending_job() {
                Ok(false) => break,
                Ok(true) => {}
                Err(error) => error.0.with(|ctx| {
                    ctx.catch();
                }),
            }
        }
        self.rejections.borrow_mut().clear();
    }

    pub fn eval(&mut self, code: &str) -> Result<EvalResult, String> {
        self.eval_named(code, &self.library_root.join("sketch.js"))
    }

    #[cfg(feature = "fft")]
    pub fn sync_audio_settings(
        &self,
        settings: &crate::audio::AudioSettings,
    ) -> Result<(), String> {
        self.start();
        self.run(&format!(
            "__bridge.setAudioSettings({}); ''",
            native::audio_settings(settings)
        ))
        .map(|_| ())
    }

    pub fn eval_named(&mut self, code: &str, filename: &Path) -> Result<EvalResult, String> {
        self.eval_source(code, filename, false)
    }

    pub fn eval_module_named(&mut self, code: &str, filename: &Path) -> Result<EvalResult, String> {
        self.eval_source(code, filename, true)
    }

    fn eval_source(
        &mut self,
        code: &str,
        filename: &Path,
        force_module: bool,
    ) -> Result<EvalResult, String> {
        if self.jobs_blocked {
            return Err(
                "JavaScript has unfinished asynchronous work; reset the interpreter".into(),
            );
        }
        if code.len() > self.settings.script_kib * 1024 {
            return Err("JavaScript script byte limit exceeded".into());
        }
        self.start();
        self.rejections.borrow_mut().clear();
        self.can_rollback = false;
        self.run("__bridge.begin(); ''")?;
        let mut module_promise = None;
        let result = (|| {
            let filename = if filename.is_absolute() {
                filename.to_path_buf()
            } else {
                self.library_root.join(filename)
            };
            let is_module = force_module
                || filename
                    .extension()
                    .is_some_and(|extension| extension == "mjs")
                || self
                    .context
                    .with(|ctx| requires_module(&ctx, code, &filename))?;
            self.evaluations = self
                .evaluations
                .checked_add(1)
                .ok_or("Evaluation count limit exceeded")?;
            let promise = self.context.with(|ctx| {
                if is_module {
                    let name = format!("{}#evaluation-{}", filename.display(), self.evaluations);
                    Module::evaluate(ctx.clone(), name, code)
                        .map(|promise| Some(Persistent::save(&ctx, promise)))
                        .map_err(|error| js_error(&ctx, error))
                } else {
                    let mut options = EvalOptions::default();
                    options.filename = Some(filename.to_string_lossy().into_owned());
                    ctx.eval_with_options::<(), _>(code, options)
                        .map(|()| None)
                        .map_err(|error| js_error(&ctx, error))
                }
            })?;
            module_promise = promise.clone();
            self.jobs()?;
            if let Some(promise) = promise {
                let resolved = self.context.with(|ctx| {
                    match promise
                        .restore(&ctx)
                        .map_err(|error| js_error(&ctx, error))?
                        .result::<()>()
                    {
                        Some(result) => {
                            result.map(|()| true).map_err(|error| js_error(&ctx, error))
                        }
                        None => Ok(false),
                    }
                })?;
                if !resolved {
                    self.jobs_blocked = true;
                    return Err(
                        "Local module evaluation left an unresolved promise; reset the interpreter"
                            .into(),
                    );
                }
            }
            let encoded = self.run("__bridge.snapshot()")?;
            if encoded.len() > 4 * self.settings.script_kib * 1024 {
                return Err("JavaScript graph byte limit exceeded".into());
            }
            let snapshot: Snapshot = serde_json::from_str(&encoded)
                .map_err(|e| format!("invalid JavaScript graph: {e}"))?;
            let result = compile_snapshot(&snapshot)?;
            self.run("__bridge.commit(); ''")?;
            self.previous_audio = self.uses_audio;
            self.uses_audio = snapshot.audio;
            self.can_rollback = true;
            Ok(result)
        })();
        if result.is_err() {
            self.cancel_jobs();
            if let Some(promise) = module_promise {
                self.jobs_blocked |= self.context.with(|ctx| {
                    promise
                        .restore(&ctx)
                        .map(|promise| promise.state() == rquickjs::promise::PromiseState::Pending)
                        .map_err(|error| js_error(&ctx, error))
                })?;
            }
            self.start();
            self.run("__bridge.rollback(); ''")?;
            self.jobs_blocked |= self.runtime.is_job_pending();
            self.rejections.borrow_mut().clear();
        }
        result
    }

    pub fn frame(&mut self, input: JavascriptFrame) -> Result<[f32; NUM_JS_VALUES], String> {
        if self.jobs_blocked {
            return Err(
                "JavaScript has unfinished asynchronous work; reset the interpreter".into(),
            );
        }
        if [
            input.time,
            input.beat,
            input.tempo,
            input.phase,
            input.audio_onset,
            input.audio_beat,
            input.audio_bpm,
        ]
        .iter()
        .chain(input.resolution.iter())
        .chain(input.mouse.iter())
        .chain(input.fft.iter())
        .any(|v| !v.is_finite())
        {
            return Err("JavaScript frame inputs must be finite".into());
        }
        #[cfg(feature = "midi")]
        if input
            .midi_values
            .iter()
            .chain(input.midi_envelope.iter())
            .any(|v| !v.is_finite())
        {
            return Err("JavaScript MIDI frame inputs must be finite".into());
        }
        #[cfg(feature = "osc")]
        if input.osc.iter().any(|v| !v.is_finite()) {
            return Err("JavaScript OSC frame inputs must be finite".into());
        }
        self.start();
        let result = (|| {
            let input = serde_json::to_string(&input).map_err(|e| e.to_string())?;
            let output = self.context.with(|ctx| {
                let call = || -> rquickjs::Result<String> {
                    let bridge: Object = ctx.globals().get("__bridge")?;
                    let frame: Function = bridge.get("frame")?;
                    let input = ctx.json_parse(input)?;
                    frame.call((input,))
                };
                call().map_err(|e| js_error(&ctx, e))
            })?;
            self.jobs()?;
            let values: Vec<f32> = serde_json::from_str(&output).map_err(|e| e.to_string())?;
            if values.len() != NUM_JS_VALUES || values.iter().any(|v| !v.is_finite()) {
                return Err("JavaScript callback must return a finite float".into());
            }
            self.uses_audio |= self.context.with(|ctx| {
                let call = || -> rquickjs::Result<bool> {
                    let bridge: Object = ctx.globals().get("__bridge")?;
                    let audio: Function = bridge.get("audio")?;
                    audio.call(())
                };
                call().map_err(|e| js_error(&ctx, e))
            })?;
            values
                .try_into()
                .map_err(|_| "invalid callback value count".into())
        })();
        if result.is_err() {
            self.cancel_jobs();
            self.jobs_blocked = self.runtime.is_job_pending();
        }
        self.start();
        self.run("__bridge.finishFrame(); ''")?;
        result
    }

    pub fn uses_audio(&self) -> bool {
        self.uses_audio
    }

    pub fn reset(&mut self, settings: JavascriptSettings) -> Result<(), String> {
        *self = Self::with_settings(self.library_root.clone(), settings)?;
        Ok(())
    }

    pub fn library_root(&self) -> &Path {
        &self.library_root
    }

    /// Call only when shader validation fails before installing any new shader.
    pub fn rollback_last_eval(&mut self) -> Result<(), String> {
        if !self.can_rollback {
            return Err("no JavaScript evaluation available to roll back".into());
        }
        self.start();
        self.run("__bridge.rollback(); ''")?;
        self.uses_audio = self.previous_audio;
        self.can_rollback = false;
        Ok(())
    }
}

impl Drop for JavascriptRuntime {
    fn drop(&mut self) {
        self.runtime.set_host_promise_rejection_tracker(None);
        self.rejections.borrow_mut().clear();
    }
}

fn kind_name(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Source => "src",
        OpKind::Geo => "coord",
        OpKind::Color => "color",
        OpKind::Blend => "combine",
        OpKind::Modulate => "combineCoord",
    }
}

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with("gl_")
}

fn hydra_uniforms(body: &str, inputs: &[Input]) -> String {
    let chars: Vec<_> = body.chars().collect();
    let regions = crate::srcscan::classify(&chars);
    let mut output = String::new();
    let mut i = 0;
    while i < chars.len() {
        if regions[i] == crate::srcscan::Region::Code
            && (chars[i].is_ascii_alphabetic() || chars[i] == '_')
        {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let field = chars[..start].iter().rev().find(|c| !c.is_whitespace()) == Some(&'.');
            let parameter = inputs.iter().any(|input| input.name == word);
            output.push_str(match (word.as_str(), field || parameter) {
                ("time", false) => "iTime",
                ("resolution", false) => "iResolution",
                _ => &word,
            });
        } else {
            output.push(chars[i]);
            i += 1;
        }
    }
    output
}

fn compile_snapshot(snapshot: &Snapshot) -> Result<EvalResult, String> {
    if snapshot.slots > NUM_JS_VALUES || snapshot.definitions.len() > 256 {
        return Err("JavaScript graph resource limit".into());
    }
    let mut definitions: HashMap<String, (String, Vec<f64>, String)> = FUNCTIONS
        .iter()
        .map(|m| {
            (
                m.name.to_string(),
                (kind_name(m.kind).into(), m.defaults.to_vec(), m.name.into()),
            )
        })
        .collect();
    definitions.insert("src".into(), ("src".into(), vec![0.0], "src".into()));
    definitions.insert(
        "ext_src".into(),
        ("src".into(), vec![0.0], "ext_src".into()),
    );
    definitions.insert("text_src".into(), ("src".into(), vec![], "text_src".into()));
    let mut glsl = String::new();
    for d in &snapshot.definitions {
        if !identifier(&d.name) || d.inputs.len() > 32 || d.glsl.len() > 64 * 1024 {
            return Err("invalid custom function name or size".into());
        }
        let (ret, fixed, aliases) = match d.kind.as_str() {
            "src" => ("vec4", "vec2 st", "vec2 _st = st;"),
            "coord" => ("vec2", "vec2 st", "vec2 _st = st;"),
            "color" => ("vec4", "vec4 c0", "vec4 _c0 = c0;"),
            "combine" => ("vec4", "vec4 c0, vec4 c1", "vec4 _c0 = c0; vec4 _c1 = c1;"),
            "combineCoord" => ("vec2", "vec2 st, vec4 c0", "vec2 _st = st; vec4 _c0 = c0;"),
            _ => return Err("unsupported custom function type".into()),
        };
        let mut names = std::collections::HashSet::new();
        for input in &d.inputs {
            if !identifier(&input.name)
                || ["st", "c0", "c1"].contains(&input.name.as_str())
                || !names.insert(&input.name)
                || !input.default.is_finite()
            {
                return Err("invalid custom function input".into());
            }
        }
        let extra = d
            .inputs
            .iter()
            .map(|i| format!(", float {}", i.name))
            .collect::<String>();
        glsl.push_str(&format!(
            "{ret} {}({fixed}{extra}) {{\n{aliases}\n{}\n}}\n",
            format_args!("hydraJs_{}", d.name),
            hydra_uniforms(&d.glsl, &d.inputs)
        ));
        definitions.insert(
            d.name.clone(),
            (
                d.kind.clone(),
                d.inputs.iter().map(|i| i.default).collect(),
                format!("hydraJs_{}", d.name),
            ),
        );
    }
    let mut budget = 4096;
    let mut shaders = [None, None, None, None];
    for (i, graph) in snapshot.buffers.iter().enumerate() {
        if let Some(graph) = graph {
            shaders[i] = Some(format!(
                "{glsl}{}",
                compile_node(&graph_node(
                    graph,
                    &definitions,
                    snapshot.slots,
                    0,
                    &mut budget
                )?)?
            ));
        }
    }
    let mut filters = [None; 4];
    for (i, filter) in snapshot.filters.iter().enumerate() {
        filters[i] = match filter.as_deref() {
            None => None,
            Some("linear") => Some(BufferFilter::Linear),
            Some("nearest") => Some(BufferFilter::Nearest),
            _ => return Err("invalid buffer filter".into()),
        };
    }
    let render_mode = if snapshot.all {
        RenderMode::All
    } else {
        match snapshot.render {
            None | Some(0) => RenderMode::Single0,
            Some(i @ 1..=3) => RenderMode::Single(i),
            _ => return Err("invalid render buffer".into()),
        }
    };
    let mut result = EvalResult {
        shaders,
        render_mode,
        text_data: snapshot
            .text
            .as_ref()
            .map(|request| {
                if request.content.len() > 4096 {
                    return Err("Text content byte limit exceeded".to_string());
                }
                let mode = match request.mode.as_str() {
                    "fill" => crate::text::TextMode::Fill,
                    "stroke" => crate::text::TextMode::Stroke,
                    "fillStroke" => crate::text::TextMode::FillStroke,
                    _ => return Err("Invalid text mode".into()),
                };
                let config = crate::text::TextConfig {
                    font_size: crate::text::parse_size(&request.font_size, "fontSize")?,
                    line_width: crate::text::parse_size(&request.line_width, "lineWidth")?,
                };
                Ok(crate::text::rasterize(&request.content, mode, &config))
            })
            .transpose()?,
        screencap: snapshot.screencap,
        render_resolution: snapshot.resolution.map(|r| (r[0], r[1])),
        buffer_filter: filters,
        buffer_clear: snapshot.clear,
        #[cfg(any(
            feature = "webcam",
            feature = "image_url",
            feature = "video",
            feature = "stream"
        ))]
        source_requests: Vec::new(),
        #[cfg(feature = "fft")]
        audio_requests: Vec::new(),
        #[cfg(feature = "midi")]
        midi_requests: Vec::new(),
        #[cfg(feature = "osc")]
        osc_requests: Vec::new(),
        #[cfg(feature = "stream")]
        broadcast_request: None,
    };
    native::apply(&snapshot.commands, &mut result)?;
    Ok(result)
}

fn graph_node(
    graph: &Graph,
    definitions: &HashMap<String, (String, Vec<f64>, String)>,
    slots: usize,
    depth: usize,
    budget: &mut usize,
) -> Result<Node, String> {
    if depth >= 16 || graph.ops.is_empty() || graph.ops.len() > *budget {
        return Err("JavaScript graph depth or operation limit".into());
    }
    *budget -= graph.ops.len();
    let mut ops = Vec::new();
    for operation in &graph.ops {
        if operation.name == "tileable" {
            if !ops.is_empty() || operation.args.len() != 1 {
                return Err("Invalid tileable graph".into());
            }
            let width = match &operation.args[0] {
                Value::Number(n) if n.is_finite() && (*n as f32).is_finite() => Arg::Lit(*n),
                Value::Callback(i) if *i < slots => Arg::Expr(format!("iJsValues[{i}]")),
                _ => return Err("Invalid tileable width".into()),
            };
            let other = graph_node(
                operation.other.as_ref().ok_or("Missing tileable graph")?,
                definitions,
                slots,
                depth + 1,
                budget,
            )?;
            ops.push(Op::Tileable { other, width });
            continue;
        }
        let (kind, defaults, shader_name) = definitions
            .get(&operation.name)
            .ok_or("unknown graph function")?;
        if operation.args.len() != defaults.len() {
            return Err("invalid graph argument count".into());
        }
        let args = operation
            .args
            .iter()
            .map(|v| match v {
                Value::Number(n) if n.is_finite() && (*n as f32).is_finite() => Ok(Arg::Lit(*n)),
                Value::Callback(i) if *i < slots => Ok(Arg::Expr(format!("iJsValues[{i}]"))),
                _ => Err("invalid graph argument".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let func = shader_name.clone().into();
        let other = operation
            .other
            .as_ref()
            .map(|g| graph_node(g, definitions, slots, depth + 1, budget))
            .transpose()?;
        let op = match (kind.as_str(), other) {
            ("src", None) => Op::Source { func, args },
            ("coord", None) => Op::Geo { func, args },
            ("color", None) => Op::Color { func, args },
            ("combine", Some(other)) => Op::Blend { func, other, args },
            ("combineCoord", Some(other)) => Op::Modulate { func, other, args },
            _ => return Err("invalid graph texture operand".into()),
        };
        ops.push(op);
    }
    Ok(Node { ops })
}
