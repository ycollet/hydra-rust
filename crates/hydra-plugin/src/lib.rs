//! Hydra-rust as a VST3/CLAP/standalone plugin.
//!
//! This is a "bring your own GUI" nih-plug plugin: instead of using
//! nih-plug's egui/iced/vizia integrations, it opens a plain `baseview`
//! window with a raw OpenGL context (via `glow`, the same crate
//! `hydra-rust`'s own renderer uses) and drives `hydra_rust::renderer`
//! directly - no adapter layer, no second GL binding crate.
//!
//! ## Script source (v1)
//!
//! The Hydra script is read from the file path in the `HYDRA_PLUGIN_SCRIPT`
//! environment variable (falling back to a small built-in example sketch if
//! unset or unreadable). The file's mtime is polled once per frame; editing
//! and saving the file in any external editor hot-reloads it in the plugin
//! window, enabling live-coding from inside a DAW. There is currently no
//! in-window text editor - see `README.md` in this crate for known
//! limitations and the plan for a future in-window editor / parameter-based
//! script loading.
//!
//! ## Audio (v1)
//!
//! Audio is passed through unchanged; the host's audio buffers are not yet
//! fed into Hydra's FFT-reactive (`a.fft[]`) pipeline. See `README.md`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use baseview::{WindowHandle, WindowOpenOptions, WindowScalePolicy, gl::GlConfig};
use nih_plug::prelude::*;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use hydra_rust::eval::RenderMode;
use hydra_rust::renderer::{self, RenderUniforms, ShaderRenderer};

const DEFAULT_SKETCH: &str = "osc(60, 0.1, 1.2).out()";

fn script_path_from_env() -> Option<PathBuf> {
    std::env::var_os("HYDRA_PLUGIN_SCRIPT").map(PathBuf::from)
}

fn load_script(path: &Option<PathBuf>) -> (String, Option<SystemTime>) {
    match path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(code) => {
                let mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok();
                (code, mtime)
            }
            Err(err) => {
                log::warn!(
                    "hydra-plugin: failed to read HYDRA_PLUGIN_SCRIPT ({}): {err} - using built-in default sketch",
                    p.display()
                );
                (DEFAULT_SKETCH.to_owned(), None)
            }
        },
        None => (DEFAULT_SKETCH.to_owned(), None),
    }
}

/// The plugin's OpenGL-backed editor window.
struct HydraGlWindow {
    gl: Arc<glow::Context>,
    renderer: ShaderRenderer,
    start: Instant,
    script_path: Option<PathBuf>,
    last_mtime: Option<SystemTime>,
    render_mode: RenderMode,
}

impl HydraGlWindow {
    fn new(window: &mut baseview::Window<'_>) -> Self {
        let gl_context = window
            .gl_context()
            .expect("failed to get baseview gl context");

        // `ShaderRenderer::new` and `compile_buffers` both allocate/compile
        // real GL objects (VAO, programs), so both must run while the
        // context is current - it must NOT be made current->not-current
        // around just the VAO creation like the first version of this
        // code did (that failed with "Unable to create VertexArray
        // object"/would have failed identically on the first shader
        // compile too).
        let script_path = script_path_from_env();
        let (code, mtime) = load_script(&script_path);

        let (gl, renderer, render_mode) = unsafe {
            gl_context.make_current();
            #[allow(clippy::arc_with_non_send_sync)]
            let gl = Arc::new(glow::Context::from_loader_function(|s| {
                gl_context.get_proc_address(s)
            }));
            let mut renderer = ShaderRenderer::new(gl.clone());
            let render_mode = Self::compile(&mut renderer, &code);
            gl_context.make_not_current();
            (gl, renderer, render_mode)
        };

        Self {
            gl,
            renderer,
            start: Instant::now(),
            script_path,
            last_mtime: mtime,
            render_mode,
        }
    }

    /// Compiles `code` into `renderer`'s buffers, logging (not panicking)
    /// on a script error and leaving whatever previously compiled, so a
    /// typo while live-coding doesn't blank the editor window.
    fn compile(renderer: &mut ShaderRenderer, code: &str) -> RenderMode {
        match hydra_rust::eval(code) {
            Ok(result) => {
                if let Some(ref td) = result.text_data {
                    renderer.upload_text(td);
                }
                let errs = renderer.compile_buffers(&result.shaders, result.render_mode);
                if !errs.is_empty() {
                    log::warn!("hydra-plugin: shader compile error(s): {errs:?}");
                }
                for (i, filter) in result.buffer_filter.iter().enumerate() {
                    if let Some(mode) = filter {
                        renderer.set_buffer_filter(i, *mode);
                    }
                }
                result.render_mode
            }
            Err(err) => {
                log::warn!("hydra-plugin: script eval error: {err}");
                Default::default()
            }
        }
    }

    /// Re-reads the script file if its mtime changed since the last check
    /// (cheap: a single `stat` per frame), recompiling only on an actual
    /// change - this is what gives the live-coding hot-reload behavior.
    fn maybe_reload(&mut self) {
        let Some(path) = &self.script_path else {
            return;
        };
        let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) else {
            return;
        };
        if Some(mtime) != self.last_mtime {
            self.last_mtime = Some(mtime);
            let code = std::fs::read_to_string(path).unwrap_or_default();
            self.render_mode = Self::compile(&mut self.renderer, &code);
        }
    }
}

impl baseview::WindowHandler for HydraGlWindow {
    fn on_frame(&mut self, window: &mut baseview::Window) {
        let gl_context = window
            .gl_context()
            .expect("failed to get baseview gl context");

        unsafe {
            gl_context.make_current();
        }

        // Recompile (if needed) only once the context is current, since
        // this may call `compile_buffers`, which issues real GL calls.
        self.maybe_reload();

        // TODO: track the actual baseview window size; nih-plug's
        // `Editor::size()` reports a fixed logical size for now (see
        // `HydraEditor::size`), so this stays in sync with that until a
        // resizable-window TODO below is picked up.
        let (w, h) = (HYDRA_EDITOR_WIDTH, HYDRA_EDITOR_HEIGHT);
        self.renderer.ensure_resolution(w, h);

        let uniforms = RenderUniforms {
            time: self.start.elapsed().as_secs_f32(),
            resolution: [w as f32, h as f32],
            // TODO: wire real mouse position from baseview's `on_event`
            // (see `HydraGlWindow::on_event` below) into `mouse`/`m` reads.
            mouse: [0.0, 0.0],
            beat: 0.0,
            tempo: 120.0,
            phase: 0.0,
            // TODO: feed the host's audio buffer (from `process()`) into
            // hydra-rust's FFT analysis instead of zeros, so `a.fft[]`
            // reacts to the host's audio like the desktop app's mic input
            // does.
            fft: [0.0; hydra_rust::audio::NUM_FFT_BINS],
            midi_note: [0.0; hydra_rust::midi::NUM_MIDI_NOTES],
            midi_velocity: [0.0; hydra_rust::midi::NUM_MIDI_NOTES],
            midi_cc: [0.0; hydra_rust::midi::NUM_MIDI_CC],
            midi_cc_smoothed: [0.0; hydra_rust::midi::NUM_MIDI_CC],
            midi_envelope: [0.0; hydra_rust::midi::NUM_MIDI_ENVELOPES],
            midi_aftertouch: [0.0; hydra_rust::midi::NUM_MIDI_NOTES],
            midi_channel_aftertouch: 0.0,
            osc: [0.0; hydra_rust::osc::NUM_OSC_SLOTS],
        };

        let snap = self.renderer.snapshot();
        let ping = self.renderer.ping().clone();
        renderer::render_multipass(&self.gl, &snap, &ping, uniforms);
        let _ = self.render_mode;

        unsafe {
            gl_context.swap_buffers();
            gl_context.make_not_current();
        }
    }

    fn on_event(
        &mut self,
        _window: &mut baseview::Window,
        _event: baseview::Event,
    ) -> baseview::EventStatus {
        baseview::EventStatus::Captured
    }
}

const HYDRA_EDITOR_WIDTH: u32 = 1024;
const HYDRA_EDITOR_HEIGHT: u32 = 576;

struct HydraEditor;

impl Editor for HydraEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        _context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let window = baseview::Window::open_parented(
            &ParentWindowHandleAdapter(parent),
            WindowOpenOptions {
                title: String::from("Hydra"),
                size: baseview::Size::new(
                    HYDRA_EDITOR_WIDTH as f64,
                    HYDRA_EDITOR_HEIGHT as f64,
                ),
                scale: WindowScalePolicy::SystemScaleFactor,
                gl_config: Some(GlConfig {
                    version: (3, 2),
                    red_bits: 8,
                    blue_bits: 8,
                    green_bits: 8,
                    alpha_bits: 8,
                    depth_bits: 0,
                    stencil_bits: 0,
                    samples: None,
                    srgb: true,
                    double_buffer: true,
                    vsync: false,
                    ..Default::default()
                }),
            },
            move |window: &mut baseview::Window<'_>| -> HydraGlWindow { HydraGlWindow::new(window) },
        );

        Box::new(HydraEditorHandle { window })
    }

    fn size(&self) -> (u32, u32) {
        (HYDRA_EDITOR_WIDTH, HYDRA_EDITOR_HEIGHT)
    }

    fn set_scale_factor(&self, _factor: f32) -> bool {
        false
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {}
    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}
    fn param_values_changed(&self) {}
}

struct HydraEditorHandle {
    #[allow(dead_code)]
    window: WindowHandle,
}

// SAFETY: matches nih-plug's own `byo_gui_gl` example - the enum stored
// within `WindowHandle` contains raw pointers that are not `Send` by
// default, but the handle itself is only ever touched on the GUI thread
// that created it; nih-plug only requires `Send` so the `Box<dyn Any +
// Send>` bound type-checks when the host drops the editor from its own
// thread.
unsafe impl Send for HydraEditorHandle {}

/// `baseview`'s pinned git rev here uses a different `raw-window-handle`
/// version than nih-plug's, so the parent handle needs adapting - same
/// approach as nih-plug's own `byo_gui_gl` example.
struct ParentWindowHandleAdapter(ParentWindowHandle);

unsafe impl HasRawWindowHandle for ParentWindowHandleAdapter {
    fn raw_window_handle(&self) -> RawWindowHandle {
        match self.0 {
            ParentWindowHandle::X11Window(window) => {
                let mut handle = raw_window_handle::XcbWindowHandle::empty();
                handle.window = window;
                RawWindowHandle::Xcb(handle)
            }
            ParentWindowHandle::AppKitNsView(ns_view) => {
                let mut handle = raw_window_handle::AppKitWindowHandle::empty();
                handle.ns_view = ns_view;
                RawWindowHandle::AppKit(handle)
            }
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                let mut handle = raw_window_handle::Win32WindowHandle::empty();
                handle.hwnd = hwnd;
                RawWindowHandle::Win32(handle)
            }
        }
    }
}

#[derive(Params, Default)]
struct HydraPluginParams {}

pub struct HydraPlugin {
    params: Arc<HydraPluginParams>,
}

impl Default for HydraPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(HydraPluginParams::default()),
        }
    }
}

impl Plugin for HydraPlugin {
    const NAME: &'static str = "Hydra";
    const VENDOR: &'static str = "hydra-rust";
    const URL: &'static str = "https://github.com/sova-org/hydra-rust";
    const EMAIL: &'static str = "info@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // A visual synth plugin with no audio processing of its own (v1) -
    // audio is passed through so it can still sit on an audio track.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(HydraEditor))
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // v1: pass audio through unchanged. See the module-level docs and
        // README.md for the plan to feed this into hydra-rust's FFT
        // pipeline.
        ProcessStatus::Normal
    }
}

impl ClapPlugin for HydraPlugin {
    const CLAP_ID: &'static str = "org.sova.hydra-rust";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Hydra visual synthesizer, rendered into the host window");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Utility];
}

impl Vst3Plugin for HydraPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"HydraRustVisual0";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Tools];
}

nih_export_clap!(HydraPlugin);
// This macro's own expansion (`vst3_com::vtable`) triggers the
// `semicolon_in_expressions_from_non_local_macros` future-incompat lint -
// it's inside nih-plug's/vst3-sys's own macro definitions, not anything
// under this crate's control.
#[allow(semicolon_in_expressions_from_non_local_macros)]
mod vst3_export {
    use super::HydraPlugin;
    use nih_plug::nih_export_vst3;
    nih_export_vst3!(HydraPlugin);
}
