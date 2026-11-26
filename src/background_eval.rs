//! Bounded background evaluation with host-controlled graph acceptance.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::EvalResult;

#[derive(Clone)]
pub struct Cancellation {
    generation: Arc<AtomicU64>,
    request: u64,
    #[cfg(test)]
    progress: Option<Arc<std::sync::Barrier>>,
}

impl Cancellation {
    fn new(generation: Arc<AtomicU64>, request: u64) -> Self {
        Self {
            generation,
            request,
            #[cfg(test)]
            progress: None,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.generation.load(Ordering::Acquire) != self.request
    }

    pub(crate) fn check_progress(&self) -> bool {
        #[cfg(test)]
        if let Some(progress) = &self.progress {
            progress.wait();
            progress.wait();
        }
        self.is_cancelled()
    }
}

#[cfg(feature = "javascript")]
#[derive(Clone)]
pub struct JavascriptConfig {
    pub library_root: std::path::PathBuf,
    pub settings: crate::javascript_settings::JavascriptSettings,
}

pub struct EvalRequest {
    pub code: String,
    #[cfg(feature = "fft")]
    pub audio_settings: crate::audio::AudioSettings,
    #[cfg(feature = "javascript")]
    pub javascript: Option<JavascriptConfig>,
    #[cfg(feature = "javascript")]
    pub filename: Option<std::path::PathBuf>,
    #[cfg(feature = "javascript")]
    pub module: bool,
}

impl EvalRequest {
    pub fn rhai(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            #[cfg(feature = "fft")]
            audio_settings: Default::default(),
            #[cfg(feature = "javascript")]
            javascript: None,
            #[cfg(feature = "javascript")]
            filename: None,
            #[cfg(feature = "javascript")]
            module: false,
        }
    }
}

pub enum Completion {
    Evaluated {
        id: u64,
        javascript: bool,
        uses_audio: bool,
        #[cfg(feature = "javascript")]
        runtime_config: Option<JavascriptConfig>,
        result: Result<Box<EvalResult>, String>,
    },
    #[cfg(feature = "javascript")]
    Reset {
        result: Result<JavascriptConfig, String>,
    },
    Failed(String),
}

enum Command {
    Evaluate(EvalRequest),
    #[cfg(feature = "javascript")]
    Reset(JavascriptConfig),
}

#[cfg(feature = "javascript")]
pub struct FrameCompletion {
    pub values: Result<[f32; crate::shader::NUM_JS_VALUES], String>,
    pub uses_audio: bool,
}

#[derive(Default)]
struct State {
    command: Option<(u64, Command)>,
    completion: Option<Completion>,
    decision: Option<(u64, bool)>,
    #[cfg(feature = "javascript")]
    frame: Option<(u64, crate::javascript::JavascriptFrame)>,
    #[cfg(feature = "javascript")]
    frame_completion: Option<(u64, FrameCompletion)>,
    stopped: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    generation: Arc<AtomicU64>,
}

pub struct BackgroundEvaluator {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Default for BackgroundEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundEvaluator {
    pub fn new() -> Self {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("hydra-evaluation".into())
            .spawn(move || {
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&worker)));
                if outcome.is_err() {
                    let mut state = worker.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.stopped = true;
                    state.completion =
                        Some(Completion::Failed("Evaluation worker panicked".into()));
                }
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                let mut state = shared.state.lock().unwrap();
                state.stopped = true;
                state.completion = Some(Completion::Failed(format!(
                    "Cannot start evaluation worker: {error}"
                )));
                None
            }
        };
        Self { shared, thread }
    }

    fn send(&self, command: Command) -> u64 {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if !state.stopped {
            state.command = Some((id, command));
            state.completion = None;
            #[cfg(feature = "javascript")]
            {
                state.frame = None;
                state.frame_completion = None;
            }
            self.shared.wake.notify_one();
        } else if state.completion.is_none() {
            state.completion = Some(Completion::Failed("Evaluation worker stopped".into()));
        }
        id
    }

    pub fn submit(&self, request: EvalRequest) -> u64 {
        self.send(Command::Evaluate(request))
    }

    #[cfg(feature = "javascript")]
    pub fn reset(&self, config: JavascriptConfig) {
        self.send(Command::Reset(config));
    }

    pub fn poll(&self) -> Option<Completion> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .completion
            .take()
    }

    /// Reject failed GPU compilations before applying any host-side requests.
    pub fn resolve(&self, id: u64, accepted: bool) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .decision = Some((id, accepted));
        self.shared.wake.notify_one();
    }

    #[cfg(feature = "javascript")]
    pub fn frame(&self, input: crate::javascript::JavascriptFrame) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.stopped {
            state.frame = Some((self.shared.generation.load(Ordering::Acquire), input));
            self.shared.wake.notify_one();
        }
    }

    #[cfg(feature = "javascript")]
    pub fn poll_frame(&self) -> Option<FrameCompletion> {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.frame_completion.take().and_then(|(id, result)| {
            (id == self.shared.generation.load(Ordering::Acquire)).then_some(result)
        })
    }
}

impl Drop for BackgroundEvaluator {
    fn drop(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.stopped = true;
            state.command = None;
        }
        self.shared.wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(shared: &Shared) {
    #[cfg(feature = "javascript")]
    let mut runtime: Option<crate::javascript::JavascriptRuntime> = None;
    #[cfg(feature = "javascript")]
    let mut callbacks_paused = true;
    let mut awaiting: Option<(u64, bool)> = None;
    loop {
        let mut state = shared.state.lock().unwrap();
        if state.stopped {
            return;
        }
        if let Some((id, javascript)) = awaiting {
            let decision = state.decision.take().filter(|(request, _)| *request == id);
            if decision.is_some() || id != shared.generation.load(Ordering::Acquire) {
                let accepted = decision.is_some_and(|(_, accepted)| accepted);
                awaiting = None;
                drop(state);
                #[cfg(feature = "javascript")]
                if accepted {
                    callbacks_paused = !javascript;
                }
                #[cfg(feature = "javascript")]
                if javascript
                    && !accepted
                    && let Some(runtime) = &mut runtime
                    && let Err(error) = runtime.rollback_last_eval()
                {
                    let mut state = shared.state.lock().unwrap();
                    state.stopped = true;
                    state.completion = Some(Completion::Failed(format!(
                        "JavaScript rollback failed: {error}"
                    )));
                    return;
                }
                #[cfg(not(feature = "javascript"))]
                let _ = (javascript, accepted);
                continue;
            }
            drop(shared.wake.wait(state).unwrap());
            continue;
        }
        if let Some((id, command)) = state.command.take() {
            drop(state);
            let cancellation = Cancellation::new(shared.generation.clone(), id);
            let completion = match command {
                Command::Evaluate(request) => {
                    #[cfg(feature = "javascript")]
                    let javascript = request.javascript.is_some();
                    #[cfg(not(feature = "javascript"))]
                    let javascript = false;
                    let result = {
                        #[cfg(feature = "javascript")]
                        if let Some(config) = request.javascript {
                            (|| {
                                if runtime.is_none() {
                                    runtime =
                                        Some(crate::javascript::JavascriptRuntime::with_settings(
                                            config.library_root,
                                            config.settings,
                                        )?);
                                }
                                let runtime = runtime.as_mut().unwrap();
                                #[cfg(feature = "fft")]
                                runtime.sync_audio_settings(&request.audio_settings)?;
                                runtime.set_cancellation(Some(cancellation.clone()));
                                let filename = request
                                    .filename
                                    .unwrap_or_else(|| runtime.library_root().join("sketch.js"));
                                let code = if request.code.is_empty() {
                                    "hush()"
                                } else {
                                    &request.code
                                };
                                let result = if request.module {
                                    runtime.eval_module_named(code, &filename)
                                } else {
                                    runtime.eval_named(code, &filename)
                                };
                                runtime.set_cancellation(None);
                                result
                            })()
                        } else {
                            crate::eval::eval_cancellable(
                                &request.code,
                                #[cfg(feature = "fft")]
                                request.audio_settings,
                                cancellation.clone(),
                            )
                        }
                        #[cfg(not(feature = "javascript"))]
                        crate::eval::eval_cancellable(
                            &request.code,
                            #[cfg(feature = "fft")]
                            request.audio_settings,
                            cancellation.clone(),
                        )
                    };
                    #[cfg(feature = "javascript")]
                    let uses_audio = javascript && runtime.as_ref().is_some_and(|r| r.uses_audio());
                    #[cfg(not(feature = "javascript"))]
                    let uses_audio = false;
                    if result.is_ok() {
                        awaiting = Some((id, javascript));
                    }
                    Completion::Evaluated {
                        id,
                        javascript,
                        uses_audio,
                        #[cfg(feature = "javascript")]
                        runtime_config: runtime.as_ref().map(|runtime| JavascriptConfig {
                            library_root: runtime.library_root().to_path_buf(),
                            settings: runtime.settings().clone(),
                        }),
                        result: result.map(Box::new),
                    }
                }
                #[cfg(feature = "javascript")]
                Command::Reset(config) => {
                    let result = crate::javascript::JavascriptRuntime::with_settings(
                        config.library_root.clone(),
                        config.settings.clone(),
                    )
                    .map(|new| {
                        let config = JavascriptConfig {
                            library_root: new.library_root().to_path_buf(),
                            settings: config.settings,
                        };
                        runtime = Some(new);
                        callbacks_paused = true;
                        config
                    });
                    Completion::Reset { result }
                }
            };
            let mut state = shared.state.lock().unwrap();
            if !cancellation.is_cancelled() {
                state.completion = Some(completion);
            }
            continue;
        }
        #[cfg(feature = "javascript")]
        if let Some((id, input)) = state.frame.take() {
            drop(state);
            if !callbacks_paused && let Some(runtime) = &mut runtime {
                let cancellation = Cancellation::new(shared.generation.clone(), id);
                runtime.set_cancellation(Some(cancellation.clone()));
                let values = runtime.frame(input);
                runtime.set_cancellation(None);
                let failed = values.is_err();
                let completion = FrameCompletion {
                    values,
                    uses_audio: runtime.uses_audio(),
                };
                if !cancellation.is_cancelled() {
                    callbacks_paused = failed;
                    shared.state.lock().unwrap().frame_completion = Some((id, completion));
                }
            }
            continue;
        }
        drop(shared.wake.wait(state).unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait<T>(mut poll: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(value) = poll() {
                return value;
            }
            assert!(Instant::now() < deadline, "worker did not respond");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn evaluated(worker: &BackgroundEvaluator, expected: u64) -> EvalResult {
        match wait(|| worker.poll()) {
            Completion::Evaluated { id, result, .. } => {
                assert_eq!(id, expected);
                *result.unwrap()
            }
            _ => panic!("unexpected worker completion"),
        }
    }

    #[test]
    fn latest_request_replaces_pending_work_and_unaccepted_results() {
        let worker = BackgroundEvaluator::new();
        let first = worker.submit(EvalRequest::rhai("solid(0.1).out()"));
        evaluated(&worker, first);
        // A missing decision must not prevent a newer request from running.
        for value in 0..100 {
            worker.submit(EvalRequest::rhai(format!("solid({value}).out()")));
        }
        let latest = worker.submit(EvalRequest::rhai("osc(23).out()"));
        let graph = evaluated(&worker, latest);
        assert!(graph.shaders[0].as_ref().unwrap().contains("23.0"));
        worker.resolve(latest, true);
        assert!(worker.poll().is_none());
    }

    #[test]
    fn errors_propagate_and_do_not_stop_the_worker() {
        let worker = BackgroundEvaluator::new();
        let bad = worker.submit(EvalRequest::rhai("missing_function()"));
        match wait(|| worker.poll()) {
            Completion::Evaluated { id, result, .. } => {
                assert_eq!(id, bad);
                assert!(result.is_err());
            }
            _ => panic!("unexpected worker completion"),
        }
        let good = worker.submit(EvalRequest::rhai("osc().out()"));
        evaluated(&worker, good);
        worker.resolve(good, true);
    }

    #[test]
    fn rhai_honors_cancelled_generation_before_preprocessing() {
        let generation = Arc::new(AtomicU64::new(2));
        let token = Cancellation::new(generation, 1);
        let error = crate::eval::eval_cancellable(
            "osc().out()",
            #[cfg(feature = "fft")]
            Default::default(),
            token,
        )
        .err()
        .unwrap();
        assert!(error.contains("cancelled"));
    }

    #[test]
    fn rhai_progress_interrupts_a_running_interpreter() {
        let generation = Arc::new(AtomicU64::new(1));
        let changed = generation.clone();
        let progress = Arc::new(std::sync::Barrier::new(2));
        let notified = progress.clone();
        let interrupter = std::thread::spawn(move || {
            notified.wait();
            changed.store(2, Ordering::Release);
            notified.wait();
        });
        let mut token = Cancellation::new(generation, 1);
        token.progress = Some(progress);
        let result = crate::eval::eval_cancellable(
            "while true {}",
            #[cfg(feature = "fft")]
            Default::default(),
            token,
        );
        interrupter.join().unwrap();
        let error = result.err().unwrap();
        assert!(error.contains("cancelled"), "{error}");
    }

    #[cfg(feature = "javascript")]
    fn js(code: &str) -> EvalRequest {
        let mut request = EvalRequest::rhai(code);
        request.javascript = Some(JavascriptConfig {
            library_root: std::env::current_dir().unwrap(),
            settings: crate::javascript_settings::JavascriptSettings {
                execution_ms: 2000,
                ..Default::default()
            },
        });
        request
    }

    #[cfg(feature = "javascript")]
    fn wait_started(worker: &BackgroundEvaluator) {
        wait(|| {
            worker
                .shared
                .state
                .lock()
                .unwrap()
                .command
                .is_none()
                .then_some(())
        });
        std::thread::sleep(Duration::from_millis(20));
    }

    #[cfg(feature = "javascript")]
    fn frame(worker: &BackgroundEvaluator) -> [f32; crate::shader::NUM_JS_VALUES] {
        worker.frame(crate::javascript::JavascriptFrame::default());
        wait(|| worker.poll_frame()).values.unwrap()
    }

    #[cfg(feature = "javascript")]
    #[test]
    fn superseding_running_javascript_interrupts_it_and_preserves_state() {
        let worker = BackgroundEvaluator::new();
        worker.submit(js("globalThis.saved=42;while(true){}"));
        wait_started(&worker);
        let start = Instant::now();
        let latest = worker.submit(js("solid(()=>saved).out()"));
        evaluated(&worker, latest);
        assert!(start.elapsed() < Duration::from_secs(1));
        worker.resolve(latest, true);
        assert_eq!(frame(&worker)[0], 42.0);
    }

    #[cfg(feature = "javascript")]
    #[test]
    fn rejection_and_supersession_restore_committed_callbacks() {
        let worker = BackgroundEvaluator::new();
        let first = worker.submit(js("solid(()=>11).out()"));
        evaluated(&worker, first);
        worker.resolve(first, true);
        assert_eq!(frame(&worker)[0], 11.0);

        let candidate = worker.submit(js("solid(()=>99).out()"));
        evaluated(&worker, candidate);
        worker.resolve(candidate, false);
        assert_eq!(frame(&worker)[0], 11.0);

        let candidate = worker.submit(js("solid(()=>77).out()"));
        evaluated(&worker, candidate);
        let next = worker.submit(js("globalThis.afterSupersession=true"));
        evaluated(&worker, next);
        worker.resolve(next, true);
        assert_eq!(frame(&worker)[0], 11.0);
    }

    #[cfg(feature = "javascript")]
    #[test]
    fn slow_frames_do_not_block_submission_and_errors_pause_callbacks() {
        let worker = BackgroundEvaluator::new();
        let id = worker.submit(js("solid(()=>{while(true){}}).out()"));
        evaluated(&worker, id);
        worker.resolve(id, true);
        worker.frame(crate::javascript::JavascriptFrame::default());
        std::thread::sleep(Duration::from_millis(20));
        let start = Instant::now();
        let next = worker.submit(js("solid(()=>7).out()"));
        let graph = evaluated(&worker, next);
        let slot: usize = graph.shaders[0]
            .as_ref()
            .unwrap()
            .split("iJsValues[")
            .nth(1)
            .unwrap()
            .split(']')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        worker.resolve(next, true);
        assert_eq!(frame(&worker)[slot], 7.0);

        let id = worker.submit(js("solid(()=>{throw Error('frame failure')}).out()"));
        evaluated(&worker, id);
        worker.resolve(id, true);
        worker.frame(crate::javascript::JavascriptFrame::default());
        let failure = wait(|| worker.poll_frame()).values.unwrap_err();
        assert!(failure.contains("frame failure"));
        worker.frame(crate::javascript::JavascriptFrame::default());
        std::thread::sleep(Duration::from_millis(10));
        assert!(worker.poll_frame().is_none());
    }

    #[cfg(feature = "javascript")]
    #[test]
    fn cancelled_frames_resume_after_a_rejected_evaluation() {
        let worker = BackgroundEvaluator::new();
        let id = worker.submit(js(
            "solid(()=>{if(time===0){while(true){}}return time}).out()",
        ));
        evaluated(&worker, id);
        worker.resolve(id, true);
        worker.frame(crate::javascript::JavascriptFrame::default());
        std::thread::sleep(Duration::from_millis(20));
        worker.submit(js("throw Error('rejected evaluation')"));
        match wait(|| worker.poll()) {
            Completion::Evaluated {
                result: Err(error), ..
            } => {
                assert!(error.contains("rejected evaluation"));
            }
            _ => panic!("unexpected worker completion"),
        }
        worker.frame(crate::javascript::JavascriptFrame {
            time: 7.0,
            ..Default::default()
        });
        assert_eq!(wait(|| worker.poll_frame()).values.unwrap()[0], 7.0);
    }

    #[cfg(feature = "javascript")]
    #[test]
    fn shutdown_interrupts_running_javascript_and_joins_the_worker() {
        let worker = BackgroundEvaluator::new();
        worker.submit(js("while(true){}"));
        wait_started(&worker);
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[cfg(feature = "javascript")]
    #[test]
    fn reset_reports_errors_and_replaces_state_only_on_success() {
        let worker = BackgroundEvaluator::new();
        let id = worker.submit(js("globalThis.saved=42;solid(()=>saved).out()"));
        evaluated(&worker, id);
        worker.resolve(id, true);
        let mut config = js("").javascript.unwrap();
        config.settings.execution_ms = 0;
        worker.reset(config);
        assert!(matches!(
            wait(|| worker.poll()),
            Completion::Reset { result: Err(_) }
        ));
        assert_eq!(frame(&worker)[0], 42.0);

        let mut config = js("").javascript.unwrap();
        config.settings.execution_ms = 50;
        worker.reset(config);
        match wait(|| worker.poll()) {
            Completion::Reset { result: Ok(config) } => {
                assert_eq!(config.settings.execution_ms, 50);
                assert!(config.library_root.is_absolute());
            }
            _ => panic!("unexpected reset completion"),
        }
        let id = worker.submit(js(
            "if(typeof saved!=='undefined')throw Error('reset failed');solid(()=>3).out()",
        ));
        evaluated(&worker, id);
        worker.resolve(id, true);
        assert_eq!(frame(&worker)[0], 3.0);
    }

    #[cfg(all(feature = "javascript", feature = "fft"))]
    #[test]
    fn javascript_receives_host_audio_settings_and_returns_requests_without_applying_them() {
        let worker = BackgroundEvaluator::new();
        let mut request = js(
            "if(a.bins[5].scale!==3)throw Error('audio sync');a.setScale(9);solid(()=>a.fft[0]).out()",
        );
        request.audio_settings.set_bins(6);
        request
            .audio_settings
            .set(Some(5), crate::audio::BinProperty::Scale, 3.0)
            .unwrap();
        let id = worker.submit(request);
        let graph = evaluated(&worker, id);
        assert!(
            graph
                .audio_requests
                .iter()
                .any(|request| matches!(request, crate::AudioRequest::SetScale(9.0)))
        );
        worker.resolve(id, true);
        worker.frame(crate::javascript::JavascriptFrame {
            fft: [0.25; 8],
            ..Default::default()
        });
        let completion = wait(|| worker.poll_frame());
        assert!(completion.uses_audio);
        assert_eq!(completion.values.unwrap()[0], 0.25);
    }
}
