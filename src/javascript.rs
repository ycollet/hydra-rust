//! Persistent JavaScript execution for shader graphs.
//!
//! Failed evaluations preserve graphs and callbacks, but cannot undo JavaScript global mutations.
//! Evaluations reuse inactive callback slots and protect slots referenced by the previous graph.
//! Libraries run locally without browser, network, or hardware access.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Exception, Function, Object, Runtime};
use serde::{Deserialize, Serialize};

use crate::eval::{
    Arg, BufferFilter, EvalResult, FUNCTIONS, Node, Op, OpKind, RenderMode, compile_node,
};
pub use crate::shader::NUM_JS_VALUES;

const MAX_SCRIPT_BYTES: usize = 1024 * 1024;
const MAX_JOBS: usize = 256;
type Rejections = Rc<RefCell<HashMap<u64, Vec<String>>>>;

#[derive(Clone, Copy, Debug, Default, Serialize)]
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
        let exception = ctx.catch();
        if let Some(object) = exception.clone().into_object()
            && let Ok(message) = object.get::<_, String>("message")
        {
            let stack = object.get::<_, String>("stack").unwrap_or_default();
            return format!("{message}\n{stack}");
        }
        format!("{exception:?}")
    } else {
        error.to_string()
    }
}

impl JavascriptRuntime {
    pub fn new(library_root: PathBuf) -> Result<Self, String> {
        let root = library_root
            .canonicalize()
            .map_err(|e| format!("library root: {e}"))?;
        if !root.is_dir() {
            return Err("library root must be a directory".into());
        }
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(32 * 1024 * 1024);
        runtime.set_max_stack_size(512 * 1024);
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
                    let message = reason
                        .as_object()
                        .and_then(|o| o.get::<_, String>("message").ok())
                        .or_else(|| reason.as_string().and_then(|s| s.to_string().ok()))
                        .unwrap_or_else(|| format!("{reason:?}"));
                    rejected.entry(id).or_default().push(message);
                }
            },
        )));
        let deadline = Arc::new(Mutex::new(Instant::now() + Duration::from_millis(200)));
        let interrupt = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            Instant::now() > *interrupt.lock().unwrap()
        })));
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        context.with(|ctx| {
            let loading = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
            let total = Rc::new(RefCell::new(0usize));
            let load = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, path: String| -> rquickjs::Result<()> {
                    let base = loading
                        .borrow()
                        .last()
                        .and_then(|p| p.parent())
                        .unwrap_or(&root)
                        .to_path_buf();
                    let path = base
                        .join(path)
                        .canonicalize()
                        .map_err(|e| Exception::throw_message(&ctx, &format!("loadScript: {e}")))?;
                    if !path.starts_with(&root) {
                        return Err(Exception::throw_message(
                            &ctx,
                            "loadScript path escapes library root",
                        ));
                    }
                    if loading.borrow().contains(&path) || loading.borrow().len() >= 16 {
                        return Err(Exception::throw_message(
                            &ctx,
                            "loadScript cycle or nesting limit",
                        ));
                    }
                    let metadata = path
                        .metadata()
                        .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
                    if metadata.len() > MAX_SCRIPT_BYTES as u64
                        || *total.borrow() >= 16 * MAX_SCRIPT_BYTES
                    {
                        return Err(Exception::throw_message(
                            &ctx,
                            "loadScript file or runtime byte limit",
                        ));
                    }
                    let code = std::fs::read_to_string(&path)
                        .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
                    if code.len() > MAX_SCRIPT_BYTES {
                        return Err(Exception::throw_message(&ctx, "loadScript file too large"));
                    }
                    *total.borrow_mut() += code.len();
                    loading.borrow_mut().push(path);
                    let result = ctx.eval::<(), _>(code);
                    loading.borrow_mut().pop();
                    result
                },
            )
            .map_err(|e| js_error(&ctx, e))?;
            ctx.globals()
                .set("loadScript", load)
                .map_err(|e| js_error(&ctx, e))?;
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
        })
    }

    fn start(&self) {
        *self.deadline.lock().unwrap() = Instant::now() + Duration::from_millis(200);
    }

    fn run(&self, code: &str) -> Result<String, String> {
        self.context
            .with(|ctx| ctx.eval::<String, _>(code).map_err(|e| js_error(&ctx, e)))
    }

    fn jobs(&self) -> Result<(), String> {
        for _ in 0..MAX_JOBS {
            match self.runtime.execute_pending_job() {
                Ok(false) => {
                    let mut rejections = self.rejections.borrow_mut();
                    let error = rejections
                        .values()
                        .find_map(|messages| messages.first())
                        .map(|reason| format!("unhandled JavaScript rejection: {reason}"));
                    rejections.clear();
                    return error.map_or(Ok(()), Err);
                }
                Ok(true) => {}
                Err(e) => return Err(e.0.with(|ctx| js_error(&ctx, rquickjs::Error::Exception))),
            }
        }
        Err("JavaScript pending job limit exceeded".into())
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
        if self.jobs_blocked {
            return Err("JavaScript job cancellation failed; replace the runtime".into());
        }
        if code.len() > MAX_SCRIPT_BYTES {
            return Err("JavaScript script byte limit exceeded".into());
        }
        self.start();
        self.rejections.borrow_mut().clear();
        self.can_rollback = false;
        self.run("__bridge.begin(); ''")?;
        let result = (|| {
            self.context
                .with(|ctx| ctx.eval::<(), _>(code).map_err(|e| js_error(&ctx, e)))?;
            self.jobs()?;
            let encoded = self.run("__bridge.snapshot()")?;
            if encoded.len() > 4 * MAX_SCRIPT_BYTES {
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
            self.start();
            self.run("__bridge.rollback(); ''")?;
            self.jobs_blocked = self.runtime.is_job_pending();
            self.rejections.borrow_mut().clear();
        }
        result
    }

    pub fn frame(&mut self, input: JavascriptFrame) -> Result<[f32; NUM_JS_VALUES], String> {
        if self.jobs_blocked {
            return Err("JavaScript job cancellation failed; replace the runtime".into());
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
    Ok(EvalResult {
        shaders,
        render_mode,
        text_data: None,
        screencap: snapshot.screencap,
        render_resolution: snapshot.resolution.map(|r| (r[0], r[1])),
        buffer_filter: filters,
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
    })
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
