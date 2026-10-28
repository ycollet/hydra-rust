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
//! New instances read the Hydra script from the `HYDRA_PLUGIN_SCRIPT`
//! environment variable (falling back to a small built-in example sketch if
//! unset or unreadable). Host state restores the selected path and source
//! snapshot instead. The file's mtime is polled once per frame; editing
//! and saving the file in any external editor hot-reloads it in the plugin
//! window, enabling live-coding from inside a DAW. A small overlay ("Load
//! menu" button or Ctrl/Cmd+E - see "Script loader" below) has a "Load script..."
//! button to pick a different file without restarting the host, replacing
//! `script_path` from then on (the env var only sets the *initial* file).
//!
//! ## Audio (v1)
//!
//! The host's audio buffer is downmixed to mono in `process()` (the
//! real-time audio thread) and pushed into a shared
//! [`hydra_rust::audio::AudioBuffer`] - a bounded ring buffer with no FFT/
//! allocation on the push side, safe to call from a real-time callback.
//! The editor's render loop (`HydraGlWindow::on_frame`, a different
//! thread) owns an [`hydra_rust::audio::AudioAnalyzer`] that polls FFT
//! bins out of that buffer once per frame, feeding `a.fft[]` from the
//! host's own audio instead of a microphone - same pipeline the desktop
//! app's `AudioManager` uses internally, just fed by a different
//! producer. Audio itself is still passed through unchanged (this is an
//! analysis tap, not an audio effect).

//! ## MIDI (v1)
//!
//! `MIDI_INPUT` is set to `MidiConfig::MidiCCs`, so the host delivers
//! structured note-on/off, polyphonic/channel pressure, and CC events
//! directly to `process()` (the real-time audio thread) - there's no
//! system-MIDI-port connection here at all (unlike the desktop app's
//! `MidiManager`, which uses `midir` to read a real MIDI input device).
//! Those events are fed into a shared [`hydra_rust::midi::MidiBridge`],
//! polled once per frame on the render thread via
//! [`hydra_rust::midi::MidiPoller`] into `midi_note`/`midi_velocity`/
//! `midi_cc`/etc. - the same `a.fft[]`-style bridge/poller split the
//! audio pipeline above uses. Scripts' own `midi.cc(i).smooth(f)`/
//! `.adsr(...)` calls are honored (applied directly to the bridge after
//! each `eval()`); `midi.start()`/`.pause()`/`.show()`/`.hide()` are
//! accepted but are no-ops here (there's no system MIDI port to start/
//! stop, and no MIDI monitor overlay in this v1 window).
//!
//! ## Resizing (v1)
//!
//! The window starts at a default size but can be resized two ways: (1)
//! dragging the small translucent square handle drawn in the
//! bottom-right corner of the content itself (the same "plugin draws its
//! own resize handle" pattern `nih_plug_vizia::widgets::ResizeHandle`
//! uses, since nih-plug doesn't yet support host-initiated/OS-border
//! plugin-window resizing - see its own `Editor` trait doc comment), or
//! (2), for the standalone binary only, dragging the real OS window
//! border (baseview resizes its own window directly, no in-content
//! handle needed there).
//! Either way, the new size is stored in a shared, host-state-persisted
//! `Arc<AtomicCell<(u32, u32)>>` (`HydraPluginParams::editor_size`) and
//! the host is asked to match via `GuiContext::request_resize()`; if the
//! host rejects the new size, the resize is rolled back.
//!
//! ## In-window editor (v1)
//!
//! An earlier version of this crate drew a full text-editing overlay
//! (type a script directly in the plugin window, no file needed). It
//! was removed: writing/iterating on a script is a much better
//! experience in the standalone desktop app (syntax highlighting, real
//! system clipboard, undo) than a from-scratch mouse/keyboard-driven
//! text box ever was here, and typed text additionally inherited a
//! `baseview` Linux/X11 limitation (its keyboard backend hard-codes a
//! US-QWERTY scan-code table and ignores the system's actual keyboard
//! layout, so e.g. AZERTY users got the wrong characters). The
//! recommended workflow is: live-code/iterate in the standalone app,
//! save the script to a file, then point the plugin at that file (see
//! "Script loader" below) - no plugin restart needed to switch files.
//!
//! ## Script loader (v1)
//!
//! Clicking "Load menu" in the top-left corner opens a small overlay with the
//! currently active script path (or a note that it's the built-in
//! default sketch) and a "Load script..." button that opens a native
//! file-picker dialog (`rfd`, the same crate the desktop app uses) to
//! pick a different `HYDRA_PLUGIN_SCRIPT`-style file; that file is then
//! hot-reloaded/polled exactly like the env-var one. Any eval/shader
//! error (from either source) is shown inline. Close the panel with its
//! title-bar close button, or toggle it with Ctrl+E (Cmd+E on macOS).
//! This only needs mouse
//! input (button clicks) - unlike a text editor, it has no keyboard-
//! layout dependency at all. Built with `egui` (rendered via
//! `egui_glow`'s GL backend on top of our own `render_multipass`
//! output) rather than `nih_plug_egui`/`egui-baseview`, since those own
//! their *entire* window - using them would mean replacing this crate's
//! own `baseview::WindowHandler`/resize/audio/MIDI event-handling with
//! theirs. Instead, `egui::Context::run` is driven by hand, with
//! `baseview`'s mouse events translated into `egui::RawInput` in
//! `on_event`.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Instant, SystemTime};

use baseview::{
    gl::GlConfig, Event as BvEvent, MouseButton, MouseEvent, Size as BvSize,
    WindowEvent as BvWindowEvent, WindowHandle, WindowOpenOptions, WindowScalePolicy,
};
use crossbeam::atomic::AtomicCell;
use keyboard_types::{Code, KeyState, Modifiers as KtModifiers};
use nih_plug::prelude::*;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use hydra_rust::audio::{AudioAnalyzer, AudioBuffer};
use hydra_rust::eval::{MidiRequest, RenderMode};
use hydra_rust::midi::{MidiBridge, MidiPoller};
use hydra_rust::renderer::{self, RenderUniforms, ShaderRenderer};

const DEFAULT_SKETCH: &str = "osc(60, 0.1, 1.2).out()";

/// Logical-pixel size of the triangular corner handle a user drags to
/// resize the window (see the module doc comment's "Resizing" section).
const RESIZE_HANDLE_SIZE: f32 = 16.0;
const MIN_EDITOR_WIDTH: u32 = 256;
const MIN_EDITOR_HEIGHT: u32 = 144;
const DEFAULT_EDITOR_WIDTH: u32 = 1024;
const DEFAULT_EDITOR_HEIGHT: u32 = 576;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct ScriptState {
    path: Option<PathBuf>,
    source: String,
}

impl Default for ScriptState {
    fn default() -> Self {
        let path = script_path_from_env();
        let (source, _) = load_script(&path);
        Self { path, source }
    }
}

struct StandaloneSession {
    client: hydra_rust::nsm::NsmClient,
    params: Arc<HydraPluginParams>,
}

static STANDALONE_SESSION: OnceLock<StandaloneSession> = OnceLock::new();

/// Start the standalone wrapper, restoring an NSM snapshot before opening audio or the editor.
pub fn run_standalone() -> std::io::Result<bool> {
    // CLI help should not wait for a session-manager handshake.
    if !std::env::args().any(|arg| arg == "--help" || arg == "-h" || arg == "--version") {
        if let Some(client) = hydra_rust::nsm::NsmClient::from_env("Hydra") {
            let params = Arc::new(HydraPluginParams::default());
            let state = hydra_rust::nsm::load::<std::collections::BTreeMap<String, String>>(
                &client.project_dir.join("plugin-state.json"),
            );
            let restored = state
                .and_then(|state| params.restore_session(&state))
                .and_then(|()| std::fs::create_dir_all(&client.project_dir));
            match restored {
                Ok(()) => {}
                Err(err) => {
                    client.reply(
                        "/nsm/client/open",
                        Err(std::io::Error::other(err.to_string())),
                    )?;
                    return Err(err);
                }
            }
            client.reply("/nsm/client/open", Ok(()))?;
            STANDALONE_SESSION
                .set(StandaloneSession { client, params })
                .map_err(|_| std::io::Error::other("Standalone session already initialized"))?;
        }
    }
    Ok(nih_plug::nih_export_standalone::<HydraPlugin>())
}

fn script_loader_ui(
    ctx: &egui::Context,
    overlay_open: &mut bool,
    script_label: &str,
    last_error: Option<&str>,
) -> bool {
    let mut do_pick_file = false;
    if *overlay_open {
        egui::Window::new("Hydra")
            .open(overlay_open)
            .default_pos([16.0, 16.0])
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("Ctrl/Cmd+E to toggle this panel.");
                ui.label(script_label);
                if ui
                    .button("Load script...")
                    .on_hover_text(
                        "Pick a Hydra script file to hot-reload from (replaces \
                         HYDRA_PLUGIN_SCRIPT; saved with the host project)",
                    )
                    .clicked()
                {
                    do_pick_file = true;
                }
                if let Some(err) = last_error {
                    ui.colored_label(egui::Color32::from_rgb(230, 80, 80), err);
                }
            });
    } else {
        egui::Area::new(egui::Id::new("script_loader_toggle"))
            .fixed_pos([16.0, 16.0])
            .movable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if ui.button("Load menu").clicked() {
                        *overlay_open = true;
                    }
                });
            });
    }
    do_pick_file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_audio_and_midi_callbacks_and_full_buffers_do_not_allocate() {
        let audio = AudioBuffer::new();
        let midi = MidiBridge::new();
        midi.register_input(1, "Keys");
        midi.set_filtered_adsr_slot(
            0,
            60,
            hydra_rust::midi::MidiFilter {
                channel: Some(2),
                input: hydra_rust::midi::InputFilter::Name("Keys".into()),
            },
            [0.0, 0.0, 1.0, 0.0],
        );
        std::thread::spawn(move || {
            let samples = [0.25; 4096];
            assert_no_alloc::assert_no_alloc(|| {
                for size in [512, 512, 512, 4096] {
                    audio.push(&samples[..size], 1);
                }
                midi.note_on(60, 0.8);
                midi.poly_pressure(60, 0.4);
                midi.control_change(1, 0.5);
                midi.channel_pressure(0.3);
                midi.note_off(60);
                midi.note_on_from(1, 2, 60, 0.8);
                midi.poly_pressure_from(1, 2, 60, 0.4);
                midi.control_change_from(1, 2, 1, 0.5);
                midi.channel_pressure_from(1, 2, 0.3);
                midi.note_off_from(1, 2, 60);
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn host_state_restores_script_source_path_and_size_without_the_original_file() {
        let params = HydraPluginParams::default();
        let script = ScriptState {
            path: Some(PathBuf::from("/nonexistent/hydra-state-test.hydra")),
            source: "osc(12, 0.2, 0.7).rotate(0.5).out()".into(),
        };
        *params.script.write().unwrap() = script.clone();
        params.editor_size.store((1280, 720));
        // This is the same Params persistence path used by the CLAP/VST3 wrappers.
        let serialized = serde_json::to_vec(&params.serialize_fields()).unwrap();
        let restored = HydraPluginParams::default();
        let editor_script = restored.script.clone();
        let editor_size = restored.editor_size.clone();
        restored
            .restore_session(&serde_json::from_slice(&serialized).unwrap())
            .unwrap();
        assert_eq!(*editor_script.read().unwrap(), script);
        assert_eq!(editor_size.load(), (1280, 720));
        assert!(hydra_rust::eval(&editor_script.read().unwrap().source).is_ok());

        // A second restore also updates already-shared editor state.
        *params.script.write().unwrap() = ScriptState {
            path: None,
            source: "noise(3).out()".into(),
        };
        restored.deserialize_fields(&params.serialize_fields());
        assert_eq!(
            *editor_script.read().unwrap(),
            *params.script.read().unwrap()
        );
    }

    #[test]
    fn legacy_state_loads_and_corrupt_nsm_state_is_rejected_without_partial_restore() {
        let params = HydraPluginParams::default();
        let original_script = params.script.read().unwrap().clone();
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("editor-size".into(), "[800,450]".into());
        params.restore_session(&fields).unwrap();
        assert_eq!(params.editor_size.load(), (800, 450));
        assert_eq!(*params.script.read().unwrap(), original_script);
        fields.insert("editor-size".into(), "[1600,900]".into());
        fields.insert("script".into(), "invalid JSON".into());
        assert!(params.restore_session(&fields).is_err());
        assert_eq!(params.editor_size.load(), (800, 450));
    }

    fn frame(
        ctx: &egui::Context,
        open: &mut bool,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, bool) {
        let mut pick_file = false;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1024.0, 576.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                pick_file = script_loader_ui(
                    ctx,
                    open,
                    "Script: example.hydra",
                    Some("Example script error"),
                );
            },
        );
        (output, pick_file)
    }

    fn text_rect(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
        fn find(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Rect> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find(shape, label))
                }
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, label))
    }

    fn click(ctx: &egui::Context, open: &mut bool, pos: egui::Pos2) -> (egui::FullOutput, bool) {
        frame(
            ctx,
            open,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            ctx,
            open,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
    }

    #[test]
    fn loader_is_accessible_without_keyboard_at_standard_and_high_dpi() {
        for scale in [1.0, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let mut open = false;
            frame(&ctx, &mut open, vec![]);
            let (output, pick_file) = frame(&ctx, &mut open, vec![]);
            assert!(!pick_file);
            assert!(text_rect(&output, "Load script...").is_none());
            let menu_pos = text_rect(&output, "Load menu").unwrap().center();
            assert!(!click(&ctx, &mut open, menu_pos).1);
            assert!(open);

            frame(&ctx, &mut open, vec![]);
            let (output, _) = frame(&ctx, &mut open, vec![]);
            assert!(text_rect(&output, "Script: example.hydra").is_some());
            assert!(text_rect(&output, "Example script error").is_some());
            let load_pos = text_rect(&output, "Load script...").unwrap().center();
            assert!(click(&ctx, &mut open, load_pos).1);

            let window_rect = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("Hydra")))
                .unwrap();
            click(
                &ctx,
                &mut open,
                egui::pos2(window_rect.right() - 14.0, window_rect.top() + 14.0),
            );
            assert!(!open);
            frame(&ctx, &mut open, vec![]);
            let (output, _) = frame(&ctx, &mut open, vec![]);
            assert!(text_rect(&output, "Load menu").is_some());
            assert!(text_rect(&output, "Load script...").is_none());
        }
    }
}

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
    script_state: Arc<RwLock<ScriptState>>,
    applied_script: ScriptState,
    last_mtime: Option<SystemTime>,
    render_mode: RenderMode,
    audio_buffer: AudioBuffer,
    audio_analyzer: AudioAnalyzer,
    midi_bridge: MidiBridge,
    midi_poller: MidiPoller,
    gui_context: Arc<dyn GuiContext>,
    editor_size: Arc<AtomicCell<(u32, u32)>>,
    /// The logical-pixel cursor position last reported by baseview -
    /// needed to hit-test the resize handle on a button press (baseview
    /// doesn't include the position in `ButtonPressed` itself).
    cursor_pos: (f32, f32),
    /// `Some` for as long as the resize handle is being dragged - the
    /// cursor position and editor size at drag-start, so the new size is
    /// computed as an absolute delta (avoids accumulating rounding error
    /// across many small `CursorMoved` events).
    resize_drag: Option<((f32, f32), (u32, u32))>,
    /// Whether the script-loader overlay is visible (opened by the
    /// "Load menu" button or Ctrl/Cmd+E - see "Script loader"
    /// section).
    overlay_open: bool,
    /// The window's logical-to-physical pixel scale factor (1.0 on a
    /// standard-DPI display, e.g. 2.0 on a macOS Retina display),
    /// updated from `WindowEvent::Resized`'s `WindowInfo` whenever it
    /// changes. `editor_size` (and everything derived from it, like
    /// `ensure_resolution`/`RenderUniforms::resolution`) stays in
    /// logical pixels, but the GL viewport must be set in *physical*
    /// pixels (`editor_size * editor_scale`) to cover the whole
    /// window's actual framebuffer - using logical pixels there only
    /// fills the bottom-left quarter of the screen on a 2x-scaled
    /// display (see `on_frame`).
    editor_scale: f64,
    /// The most recent eval/shader-compile error (from the hot-reloaded
    /// script), shown in the overlay when it's open.
    last_error: Option<String>,
    egui_ctx: egui::Context,
    egui_painter: egui_glow::Painter,
    /// `egui::Event`s translated from baseview mouse input since the
    /// last `on_frame`, fed into `egui::RawInput` there. No keyboard
    /// text input is translated - the overlay is mouse-only (a label and
    /// a button), see the module doc comment's "In-window editor"
    /// section for why.
    pending_egui_events: Vec<egui::Event>,
}

impl HydraGlWindow {
    fn new(
        window: &mut baseview::Window<'_>,
        audio_buffer: AudioBuffer,
        midi_bridge: MidiBridge,
        gui_context: Arc<dyn GuiContext>,
        editor_size: Arc<AtomicCell<(u32, u32)>>,
        script_state: Arc<RwLock<ScriptState>>,
    ) -> Self {
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
        let applied_script = script_state.read().expect("Poisoned script state").clone();
        let script_path = applied_script.path.clone();
        let code = &applied_script.source;
        let mtime = script_path
            .as_ref()
            .and_then(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());

        let mut audio_analyzer = AudioAnalyzer::new();
        let (gl, renderer, render_mode, last_error, egui_painter) = unsafe {
            gl_context.make_current();
            #[allow(clippy::arc_with_non_send_sync)]
            let gl = Arc::new(glow::Context::from_loader_function(|s| {
                gl_context.get_proc_address(s)
            }));
            let mut renderer = ShaderRenderer::new(gl.clone());
            let (render_mode, last_error) =
                Self::compile(&mut renderer, code, &midi_bridge, &mut audio_analyzer);
            // `egui_glow::Painter` needs its own `glow::Context` handle
            // (a different `glow` crate version than `hydra_rust`'s
            // renderer uses) - both just wrap the same underlying GL
            // function pointers, so two independent handles are fine.
            #[allow(clippy::arc_with_non_send_sync)]
            let egui_gl = Arc::new(egui_glow::glow::Context::from_loader_function(|s| {
                gl_context.get_proc_address(s)
            }));
            let egui_painter = egui_glow::Painter::new(egui_gl, "", None, false)
                .expect("failed to create egui_glow painter");
            gl_context.make_not_current();
            (gl, renderer, render_mode, last_error, egui_painter)
        };

        let midi_poller = MidiPoller::new(midi_bridge.clone());

        Self {
            gl,
            renderer,
            start: Instant::now(),
            script_path,
            script_state,
            applied_script,
            last_mtime: mtime,
            render_mode,
            audio_buffer,
            audio_analyzer,
            midi_bridge,
            midi_poller,
            gui_context,
            editor_size,
            cursor_pos: (0.0, 0.0),
            resize_drag: None,
            overlay_open: false,
            // `baseview` opens the window at scale 1.0 as a placeholder
            // (even under `SystemScaleFactor`) and corrects it via a
            // `Resized` event almost immediately once the real display
            // scale is known - see the field doc comment.
            editor_scale: 1.0,
            last_error,
            egui_ctx: egui::Context::default(),
            egui_painter,
            pending_egui_events: Vec::new(),
        }
    }

    /// Compiles `code` into `renderer`'s buffers, logging (not panicking)
    /// on a script error and leaving whatever previously compiled, so a
    /// typo while live-coding doesn't blank the editor window. Also
    /// applies any `midi.*` requests the script generated (CC smoothing,
    /// ADSR envelope slots) directly to `midi_bridge` - see the module
    /// doc comment for which requests are honored here. Returns the
    /// compiled `RenderMode` plus any eval/shader-compile error text (for
    /// display in the script-loader overlay).
    fn compile(
        renderer: &mut ShaderRenderer,
        code: &str,
        midi_bridge: &MidiBridge,
        audio_analyzer: &mut AudioAnalyzer,
    ) -> (RenderMode, Option<String>) {
        match hydra_rust::eval_with_audio_settings(code, audio_analyzer.settings()) {
            Ok(result) => {
                if result.screencap {
                    log::warn!("hydra-plugin: screencap() is only supported by the desktop app");
                }
                if let Some(ref td) = result.text_data {
                    renderer.upload_text(td);
                }
                let mut errs = renderer.compile_buffers(&result.shaders, result.render_mode);
                if !errs.is_empty() {
                    log::warn!("hydra-plugin: shader compile error(s): {errs:?}");
                }
                for (i, filter) in result.buffer_filter.iter().enumerate() {
                    if let Some(mode) = filter {
                        renderer.set_buffer_filter(i, *mode);
                    }
                }
                for req in &result.audio_requests {
                    use hydra_rust::AudioRequest;
                    match *req {
                        AudioRequest::SetBins(n) => audio_analyzer.set_bins(n),
                        AudioRequest::SetCutoff(value) => audio_analyzer.set_cutoff(value),
                        AudioRequest::SetScale(value) => audio_analyzer.set_scale(value),
                        AudioRequest::SetSmooth(value) => audio_analyzer.set_smooth(value),
                        AudioRequest::SetBin {
                            index,
                            property,
                            value,
                        } => {
                            if let Err(error) = audio_analyzer.set_bin(index, property, value) {
                                log::error!("hydra-plugin: {error}");
                                errs.push(error);
                            }
                        }
                        AudioRequest::Show | AudioRequest::Hide => {}
                    }
                }
                for req in &result.midi_requests {
                    match *req {
                        MidiRequest::SetCcSmooth { index, factor } => {
                            midi_bridge.set_cc_smooth(index, factor);
                        }
                        MidiRequest::Bind { slot, ref binding } => {
                            if let Err(error) = midi_bridge.bind(slot, binding.clone()) {
                                log::error!("hydra-plugin: {error}");
                                errs.push(error);
                            }
                        }
                        MidiRequest::FilteredAdsrSlot {
                            slot,
                            note,
                            ref filter,
                            adsr,
                        } => {
                            midi_bridge.set_filtered_adsr_slot(slot, note, filter.clone(), adsr);
                        }
                        MidiRequest::AdsrSlot {
                            slot,
                            note,
                            a,
                            d,
                            s,
                            r,
                        } => {
                            midi_bridge.set_adsr_slot(slot, note, a, d, s, r);
                        }
                        // The host (not this process) always owns MIDI
                        // routing for a plugin - there's no system MIDI
                        // port to start/pause - and there's no MIDI
                        // monitor overlay in this v1 window either.
                        MidiRequest::Start
                        | MidiRequest::Pause
                        | MidiRequest::Show
                        | MidiRequest::Hide => {}
                    }
                }
                let error = if errs.is_empty() {
                    None
                } else {
                    Some(errs.join("\n"))
                };
                (result.render_mode, error)
            }
            Err(err) => {
                log::warn!("hydra-plugin: script eval error: {err}");
                (Default::default(), Some(err))
            }
        }
    }

    /// Re-reads the script file if its mtime changed since the last check
    /// (cheap: a single `stat` per frame), recompiling only on an actual
    /// change - this is what gives the live-coding hot-reload behavior.
    fn maybe_reload(&mut self) {
        let restored = {
            let state = self.script_state.read().expect("Poisoned script state");
            (self.applied_script != *state).then(|| state.clone())
        };
        if let Some(state) = restored {
            self.script_path = state.path.clone();
            self.last_mtime = self
                .script_path
                .as_ref()
                .and_then(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
            let (mode, error) = Self::compile(
                &mut self.renderer,
                &state.source,
                &self.midi_bridge,
                &mut self.audio_analyzer,
            );
            self.render_mode = mode;
            self.last_error = error;
            self.applied_script = state;
        }
        let Some(path) = &self.script_path else {
            return;
        };
        let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) else {
            return;
        };
        if Some(mtime) != self.last_mtime {
            let code = match std::fs::read_to_string(path) {
                Ok(code) => code,
                Err(err) => {
                    let message = format!("Cannot read {}: {err}", path.display());
                    if self.last_error.as_ref() != Some(&message) {
                        log::warn!("hydra-plugin: {message}");
                    }
                    self.last_error = Some(message);
                    return;
                }
            };
            self.last_mtime = Some(mtime);
            self.applied_script = ScriptState {
                path: Some(path.clone()),
                source: code.clone(),
            };
            *self.script_state.write().expect("Poisoned script state") =
                self.applied_script.clone();
            let (render_mode, error) = Self::compile(
                &mut self.renderer,
                &code,
                &self.midi_bridge,
                &mut self.audio_analyzer,
            );
            self.render_mode = render_mode;
            self.last_error = error;
        }
    }

    /// Opens a native file-picker dialog and, if a file was chosen,
    /// switches `script_path` to it and clears `last_mtime` so
    /// `maybe_reload` picks it up (and compiles it) on the very next
    /// frame.
    fn pick_script_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Hydra script", &["hydra", "rhai", "txt"])
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(source) => {
                    *self.script_state.write().expect("Poisoned script state") = ScriptState {
                        path: Some(path),
                        source,
                    };
                }
                Err(err) => {
                    let message = format!("Cannot read {}: {err}", path.display());
                    log::warn!("hydra-plugin: {message}");
                    self.last_error = Some(message);
                }
            }
        }
    }

    /// Whether `pos` (logical window coordinates) falls within the
    /// resize-handle region drawn in the bottom-right corner (see
    /// `draw_resize_handle`).
    fn in_resize_handle(&self, pos: (f32, f32)) -> bool {
        let (w, h) = self.editor_size.load();
        pos.0 >= w as f32 - RESIZE_HANDLE_SIZE && pos.1 >= h as f32 - RESIZE_HANDLE_SIZE
    }

    /// Resizes the baseview window itself, updates the shared
    /// host-visible size, and asks the host to match via
    /// `request_resize()` - rolling the resize back if the host refuses
    /// it (mirrors `nih_plug_vizia`'s own resize-handle widget).
    fn apply_resize(&mut self, window: &mut baseview::Window, width: u32, height: u32) {
        let width = width.max(MIN_EDITOR_WIDTH);
        let height = height.max(MIN_EDITOR_HEIGHT);
        let previous = self.editor_size.load();
        if (width, height) == previous {
            return;
        }

        self.editor_size.store((width, height));
        window.resize(BvSize::new(width as f64, height as f64));
        if !self.gui_context.request_resize() {
            self.editor_size.store(previous);
            window.resize(BvSize::new(previous.0 as f64, previous.1 as f64));
        }
    }

    /// Draws a small translucent square over the bottom-right corner as a
    /// visible/grabbable resize-handle affordance - drawn directly with
    /// `glow` (scissored clear) rather than extra geometry/shader setup,
    /// since it's just an indicator, not part of the Hydra-rendered
    /// output itself.
    fn draw_resize_handle(&self, w: u32, h: u32) {
        use glow::HasContext as _;
        let size = (RESIZE_HANDLE_SIZE as i32 * 2).min(w as i32).min(h as i32);
        if size <= 0 {
            return;
        }
        unsafe {
            self.gl.enable(glow::SCISSOR_TEST);
            self.gl.scissor(w as i32 - size, 0, size, size);
            self.gl.clear_color(1.0, 1.0, 1.0, 0.35);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
            self.gl.disable(glow::SCISSOR_TEST);
        }
    }

    /// Runs one `egui` frame from the pointer input
    /// accumulated in `on_event` since the last call, draws the script
    /// loader overlay, and paints it on top of whatever `render_multipass`
    /// already drew this frame via `egui_glow`.
    fn draw_editor_overlay(&mut self, w: u32, h: u32) {
        self.egui_ctx.set_pixels_per_point(self.editor_scale as f32);
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w as f32, h as f32),
            )),
            time: Some(self.start.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.pending_egui_events),
            ..Default::default()
        };

        let script_label = match &self.script_path {
            Some(p) => format!("Script: {}", p.display()),
            None => "Script: (built-in default sketch - no file loaded)".to_string(),
        };
        let mut do_pick_file = false;

        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            do_pick_file = script_loader_ui(
                ctx,
                &mut self.overlay_open,
                &script_label,
                self.last_error.as_deref(),
            );
        });

        if do_pick_file {
            self.pick_script_file();
        }

        let clipped_primitives = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        self.egui_painter.paint_and_update_textures(
            [
                (w as f64 * self.editor_scale).round() as u32,
                (h as f64 * self.editor_scale).round() as u32,
            ],
            full_output.pixels_per_point,
            &clipped_primitives,
            &full_output.textures_delta,
        );
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
        if let Some(session) = STANDALONE_SESSION.get() {
            if let Err(err) = session.client.poll_save(|| {
                hydra_rust::nsm::save(
                    &session.client.project_dir.join("plugin-state.json"),
                    &session.params.serialize_fields(),
                )
            }) {
                log::error!("hydra-plugin: NSM communication failed: {err}");
                self.last_error = Some(format!("NSM: {err}"));
            }
        }

        let (w, h) = self.editor_size.load();
        self.renderer.ensure_resolution(w, h);
        // `render_multipass`'s final screen blit reuses whatever GL
        // viewport is already set (it only touches it for per-buffer FBO
        // passes); on at least some platforms/backends resizing the
        // window alone doesn't implicitly update it, so the drawn output
        // would stay pinned at the old size (visibly cropped/offset)
        // even though `editor_size`/the window itself are already
        // correct. Setting it explicitly every frame keeps it in sync
        // regardless of platform.
        //
        // This must be in *physical* pixels, not `editor_size`'s logical
        // ones - on a 2x-scaled (e.g. Retina) display the real
        // framebuffer is twice as wide/tall as `editor_size`, so a
        // viewport of `(w, h)` only covers its bottom-left quarter
        // (OpenGL's origin is bottom-left), leaving the rest black.
        let (phys_w, phys_h) = (
            (w as f64 * self.editor_scale).round() as i32,
            (h as f64 * self.editor_scale).round() as i32,
        );
        unsafe {
            use glow::HasContext as _;
            self.gl.viewport(0, 0, phys_w, phys_h);
        }

        let midi_frame = self.midi_poller.poll();
        let uniforms = RenderUniforms {
            time: self.start.elapsed().as_secs_f32(),
            resolution: [w as f32, h as f32],
            mouse: [
                (self.cursor_pos.0 / w as f32).clamp(0.0, 1.0),
                (1.0 - self.cursor_pos.1 / h as f32).clamp(0.0, 1.0),
            ],
            beat: 0.0,
            tempo: 120.0,
            phase: 0.0,
            // Fed from the host's own audio buffer, pushed in
            // `HydraPlugin::process()` (the real-time audio thread) into
            // `self.audio_buffer`; polling/FFT only ever happens here, on
            // the render thread.
            fft: self.audio_analyzer.poll(&self.audio_buffer),
            // Fed from the host's own MIDI events, pushed in
            // `HydraPlugin::process()` into `self.midi_bridge`; polling
            // (CC smoothing, envelope timing) only ever happens here.
            midi_note: midi_frame.note,
            midi_velocity: midi_frame.velocity,
            midi_cc: midi_frame.cc,
            midi_cc_smoothed: midi_frame.cc_smoothed,
            midi_envelope: midi_frame.envelope,
            midi_aftertouch: midi_frame.aftertouch,
            midi_channel_aftertouch: midi_frame.channel_aftertouch,
            midi_values: midi_frame.values,
            osc: [0.0; hydra_rust::osc::NUM_OSC_SLOTS],
        };

        let snap = self.renderer.snapshot();
        let ping = self.renderer.ping().clone();
        renderer::render_multipass(&self.gl, &snap, &ping, uniforms);
        let _ = self.render_mode;

        self.draw_resize_handle(w, h);
        self.draw_editor_overlay(w, h);

        unsafe {
            gl_context.swap_buffers();
            gl_context.make_not_current();
        }
    }

    fn on_event(
        &mut self,
        window: &mut baseview::Window,
        event: baseview::Event,
    ) -> baseview::EventStatus {
        match event {
            BvEvent::Mouse(MouseEvent::CursorMoved {
                position,
                modifiers: _,
            }) => {
                self.cursor_pos = (position.x as f32, position.y as f32);
                if let Some((start_cursor, start_size)) = self.resize_drag {
                    let dx = self.cursor_pos.0 - start_cursor.0;
                    let dy = self.cursor_pos.1 - start_cursor.1;
                    let new_w = (start_size.0 as f32 + dx).round() as u32;
                    let new_h = (start_size.1 as f32 + dy).round() as u32;
                    self.apply_resize(window, new_w, new_h);
                }
                self.pending_egui_events
                    .push(egui::Event::PointerMoved(egui::pos2(
                        self.cursor_pos.0,
                        self.cursor_pos.1,
                    )));
            }
            BvEvent::Mouse(MouseEvent::ButtonPressed {
                button,
                modifiers: _,
            }) => {
                if button == MouseButton::Left && self.in_resize_handle(self.cursor_pos) {
                    self.resize_drag = Some((self.cursor_pos, self.editor_size.load()));
                }
                if let Some(b) = bv_button_to_egui(button) {
                    self.pending_egui_events.push(egui::Event::PointerButton {
                        pos: egui::pos2(self.cursor_pos.0, self.cursor_pos.1),
                        button: b,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            BvEvent::Mouse(MouseEvent::ButtonReleased {
                button,
                modifiers: _,
            }) => {
                if button == MouseButton::Left {
                    self.resize_drag = None;
                }
                if let Some(b) = bv_button_to_egui(button) {
                    self.pending_egui_events.push(egui::Event::PointerButton {
                        pos: egui::pos2(self.cursor_pos.0, self.cursor_pos.1),
                        button: b,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            // The script-loader overlay has no scrollable content, so
            // scroll events don't need translating.
            BvEvent::Keyboard(kb_event) => {
                // Physical-key-position based (`Code`, not the layout-
                // mapped `Key`), so this works the same regardless of
                // keyboard layout - no text input is translated here at
                // all (see the module doc comment's "In-window editor"
                // section for why).
                let is_toggle = kb_event.code == Code::KeyE
                    && kb_event.state == KeyState::Down
                    && (kb_event.modifiers.contains(KtModifiers::CONTROL)
                        || kb_event.modifiers.contains(KtModifiers::META));
                if is_toggle {
                    self.overlay_open = !self.overlay_open;
                }
            }
            // Fires when the OS window itself is resized directly (only
            // reachable in the standalone binary - a parented/embedded
            // plugin view's size is instead driven by `apply_resize`
            // above, in response to our own in-content handle).
            BvEvent::Window(BvWindowEvent::Resized(info)) => {
                let logical = info.logical_size();
                self.editor_size
                    .store((logical.width.round() as u32, logical.height.round() as u32));
                self.editor_scale = info.scale();
            }
            _ => {}
        }

        baseview::EventStatus::Captured
    }
}

fn bv_button_to_egui(button: MouseButton) -> Option<egui::PointerButton> {
    match button {
        MouseButton::Left => Some(egui::PointerButton::Primary),
        MouseButton::Right => Some(egui::PointerButton::Secondary),
        MouseButton::Middle => Some(egui::PointerButton::Middle),
        MouseButton::Back => Some(egui::PointerButton::Extra1),
        MouseButton::Forward => Some(egui::PointerButton::Extra2),
        MouseButton::Other(_) => None,
    }
}

struct HydraEditor {
    audio_buffer: AudioBuffer,
    midi_bridge: MidiBridge,
    editor_size: Arc<AtomicCell<(u32, u32)>>,
    script_state: Arc<RwLock<ScriptState>>,
}

impl Editor for HydraEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let audio_buffer = self.audio_buffer.clone();
        let midi_bridge = self.midi_bridge.clone();
        let editor_size = self.editor_size.clone();
        let script_state = self.script_state.clone();
        let (initial_w, initial_h) = editor_size.load();
        let window = baseview::Window::open_parented(
            &ParentWindowHandleAdapter(parent),
            WindowOpenOptions {
                title: String::from("Hydra"),
                size: baseview::Size::new(initial_w as f64, initial_h as f64),
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
            move |window: &mut baseview::Window<'_>| -> HydraGlWindow {
                HydraGlWindow::new(
                    window,
                    audio_buffer.clone(),
                    midi_bridge.clone(),
                    context.clone(),
                    editor_size.clone(),
                    script_state.clone(),
                )
            },
        );

        Box::new(HydraEditorHandle { window })
    }

    fn size(&self) -> (u32, u32) {
        self.editor_size.load()
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

#[derive(Params)]
struct HydraPluginParams {
    /// The editor window's last size, in logical pixels - persisted
    /// across host project save/reload via `AtomicCell`'s built-in
    /// `PersistentField` impl (see the module doc comment's "Resizing"
    /// section), so a resized window doesn't reset on the next load.
    #[persist = "editor-size"]
    editor_size: Arc<AtomicCell<(u32, u32)>>,
    #[persist = "script"]
    script: Arc<RwLock<ScriptState>>,
}

impl Default for HydraPluginParams {
    fn default() -> Self {
        Self {
            editor_size: Arc::new(AtomicCell::new((
                DEFAULT_EDITOR_WIDTH,
                DEFAULT_EDITOR_HEIGHT,
            ))),
            script: Arc::new(RwLock::new(ScriptState::default())),
        }
    }
}

impl HydraPluginParams {
    fn restore_session(
        &self,
        fields: &std::collections::BTreeMap<String, String>,
    ) -> std::io::Result<()> {
        // NIH-plug logs malformed fields and skips them. NSM instead needs an error reply.
        if let Some(script) = fields.get("script") {
            serde_json::from_str::<ScriptState>(script).map_err(std::io::Error::other)?;
        }
        if let Some(size) = fields.get("editor-size") {
            serde_json::from_str::<(u32, u32)>(size).map_err(std::io::Error::other)?;
        }
        self.deserialize_fields(fields);
        Ok(())
    }
}

pub struct HydraPlugin {
    params: Arc<HydraPluginParams>,
    audio_buffer: AudioBuffer,
    midi_bridge: MidiBridge,
    /// Reused across `process()` calls to downmix the host's planar
    /// channel buffers into mono before pushing into `audio_buffer` -
    /// preallocated to `max_buffer_size` in `initialize()` so pushing
    /// into it in `process()` (the real-time audio thread) never
    /// reallocates.
    mono_scratch: Vec<f32>,
}

impl Default for HydraPlugin {
    fn default() -> Self {
        Self {
            params: STANDALONE_SESSION
                .get()
                .map(|session| session.params.clone())
                .unwrap_or_else(|| Arc::new(HydraPluginParams::default())),
            audio_buffer: AudioBuffer::new(),
            midi_bridge: MidiBridge::new(),
            mono_scratch: Vec::new(),
        }
    }
}

impl Plugin for HydraPlugin {
    const NAME: &'static str = "Hydra";
    const VENDOR: &'static str = "hydra-rust";
    const URL: &'static str = "https://github.com/sova-org/hydra-rust";
    const EMAIL: &'static str = "info@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // A visual synth plugin: audio is only analyzed (fed into the
    // FFT-reactive `a.fft[]` pipeline, see `process()`), never modified -
    // it's passed through unchanged so the plugin can sit on a regular
    // audio track and still "see" that track's audio.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(HydraEditor {
            audio_buffer: self.audio_buffer.clone(),
            midi_bridge: self.midi_bridge.clone(),
            editor_size: self.params.editor_size.clone(),
            script_state: self.params.script.clone(),
        }))
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.mono_scratch = Vec::with_capacity(buffer_config.max_buffer_size as usize);
        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Drain every MIDI event the host delivered for this block and
        // apply it directly to `midi_bridge` - `MidiBridge`'s own methods
        // just lock a `Mutex` and write plain floats/array slots (same
        // real-time-safety trade-off `audio_buffer.push()` below already
        // makes - no heap allocation either way), so this is safe to call
        // from `process()`.
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn {
                    channel,
                    note,
                    velocity,
                    ..
                } => {
                    self.midi_bridge.note_on_from(0, channel, note, velocity);
                }
                NoteEvent::NoteOff {
                    channel,
                    note,
                    velocity,
                    ..
                } => {
                    // A real note-off always clears the note regardless of
                    // its (released) velocity - `MidiBridge::note_on`'s
                    // own velocity<=0 shorthand only matters for the raw
                    // system-MIDI-port path.
                    let _ = velocity;
                    self.midi_bridge.note_off_from(0, channel, note);
                }
                NoteEvent::PolyPressure {
                    channel,
                    note,
                    pressure,
                    ..
                } => {
                    self.midi_bridge
                        .poly_pressure_from(0, channel, note, pressure);
                }
                NoteEvent::MidiCC {
                    channel, cc, value, ..
                } => {
                    self.midi_bridge.control_change_from(0, channel, cc, value);
                }
                NoteEvent::MidiChannelPressure {
                    channel, pressure, ..
                } => {
                    self.midi_bridge.channel_pressure_from(0, channel, pressure);
                }
                _ => {}
            }
        }

        // v1: audio is passed through unchanged - this is purely an
        // analysis tap. Downmix the host's planar channel buffers to mono
        // and push into `audio_buffer`; the editor's render loop (a
        // different thread) is what actually runs the FFT on this data
        // (see `HydraGlWindow::on_frame`), so this stays a cheap, no-alloc
        // `Vec`/`VecDeque` push, safe for the real-time audio thread.
        let num_samples = buffer.samples();
        let channel_data = buffer.as_slice();
        let num_channels = channel_data.len().max(1);

        self.mono_scratch.clear();
        for i in 0..num_samples {
            let sum: f32 = channel_data.iter().map(|ch| ch[i]).sum();
            self.mono_scratch.push(sum / num_channels as f32);
        }
        self.audio_buffer.push(&self.mono_scratch, 1);

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
