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
//! window, enabling live-coding from inside a DAW. There is also an
//! in-window text editor overlay (see "In-window editor" below) for typing
//! code directly without an external file/editor.
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
//! Pressing Ctrl+E (Cmd+E on macOS) toggles a text-editor overlay drawn
//! on top of the rendered output, so a script can be typed and evaluated
//! directly in the plugin window without an external file/editor. It has
//! its own buffer, independent from the `HYDRA_PLUGIN_SCRIPT` file - the
//! two don't overwrite each other, and the overlay has a "Load file"
//! button to copy the currently-active (file or default) script into it
//! as a starting point. Ctrl+Enter (or the "Eval" button) compiles the
//! buffer; any script/shader error is shown inline. The overlay is built
//! with `egui` (rendered via `egui_glow`'s GL backend on top of our own
//! `render_multipass` output) rather than `nih_plug_egui`/`egui-baseview`,
//! since those own their *entire* window - using them would mean
//! replacing this crate's own `baseview::WindowHandler`/resize/audio/MIDI
//! event-handling with theirs. Instead, `egui::Context::run` is driven by
//! hand, with `baseview`'s mouse/keyboard events translated into
//! `egui::RawInput` in `on_event`. There's no system clipboard
//! integration yet (copy/paste stays inside the text field only, no
//! `arboard`/OS clipboard - see `README.md`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use baseview::{
    Event as BvEvent, MouseButton, MouseEvent, Size as BvSize, WindowEvent as BvWindowEvent,
    WindowHandle, WindowOpenOptions, WindowScalePolicy, gl::GlConfig,
};
use crossbeam::atomic::AtomicCell;
use keyboard_types::{Code, Key as KtKey, KeyState, Modifiers as KtModifiers};
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
    /// Whether the in-window text-editor overlay is visible (toggled by
    /// Ctrl/Cmd+E - see the module doc comment's "In-window editor"
    /// section).
    editor_open: bool,
    /// The in-window editor's own script buffer - independent of the
    /// `HYDRA_PLUGIN_SCRIPT` file so neither overwrites the other.
    script_buffer: String,
    /// The most recently hot-reloaded/default file content - what the
    /// editor overlay's "Load file" button copies into `script_buffer`.
    loaded_code: String,
    /// The most recent eval/shader-compile error (from either the file
    /// hot-reload or the in-window editor's "Eval"), shown in the editor
    /// overlay when it's open.
    last_error: Option<String>,
    egui_ctx: egui::Context,
    egui_painter: egui_glow::Painter,
    /// Current keyboard modifier state, tracked across `KeyboardEvent`s
    /// since baseview reports each key press/release individually rather
    /// than a combined modifiers snapshot per-event-type.
    modifiers: KtModifiers,
    /// `egui::Event`s translated from baseview input since the last
    /// `on_frame`, fed into `egui::RawInput` there.
    pending_egui_events: Vec<egui::Event>,
}

impl HydraGlWindow {
    fn new(
        window: &mut baseview::Window<'_>,
        audio_buffer: AudioBuffer,
        midi_bridge: MidiBridge,
        gui_context: Arc<dyn GuiContext>,
        editor_size: Arc<AtomicCell<(u32, u32)>>,
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
        let script_path = script_path_from_env();
        let (code, mtime) = load_script(&script_path);

        let (gl, renderer, render_mode, last_error, egui_painter) = unsafe {
            gl_context.make_current();
            #[allow(clippy::arc_with_non_send_sync)]
            let gl = Arc::new(glow::Context::from_loader_function(|s| {
                gl_context.get_proc_address(s)
            }));
            let mut renderer = ShaderRenderer::new(gl.clone());
            let (render_mode, last_error) = Self::compile(&mut renderer, &code, &midi_bridge);
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
            last_mtime: mtime,
            render_mode,
            audio_buffer,
            audio_analyzer: AudioAnalyzer::new(),
            midi_bridge,
            midi_poller,
            gui_context,
            editor_size,
            cursor_pos: (0.0, 0.0),
            resize_drag: None,
            editor_open: false,
            script_buffer: code.clone(),
            loaded_code: code,
            last_error,
            egui_ctx: egui::Context::default(),
            egui_painter,
            modifiers: KtModifiers::empty(),
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
    /// display in the in-window editor overlay, regardless of whether
    /// the code came from the overlay or the hot-reloaded file).
    fn compile(
        renderer: &mut ShaderRenderer,
        code: &str,
        midi_bridge: &MidiBridge,
    ) -> (RenderMode, Option<String>) {
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
                for req in &result.midi_requests {
                    match *req {
                        MidiRequest::SetCcSmooth { index, factor } => {
                            midi_bridge.set_cc_smooth(index, factor);
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
        let Some(path) = &self.script_path else {
            return;
        };
        let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) else {
            return;
        };
        if Some(mtime) != self.last_mtime {
            self.last_mtime = Some(mtime);
            let code = std::fs::read_to_string(path).unwrap_or_default();
            let (render_mode, error) = Self::compile(&mut self.renderer, &code, &self.midi_bridge);
            self.render_mode = render_mode;
            self.last_error = error;
            self.loaded_code = code;
        }
    }

    /// Evaluates the in-window editor's own buffer (independent of the
    /// hot-reloaded file) and updates `last_error` for display.
    fn eval_editor_buffer(&mut self) {
        let (render_mode, error) =
            Self::compile(&mut self.renderer, &self.script_buffer, &self.midi_bridge);
        self.render_mode = render_mode;
        self.last_error = error;
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

    /// When open, runs one `egui` frame from the input accumulated in
    /// `on_event` since the last call, draws the editor window, and
    /// paints it on top of whatever `render_multipass` already drew this
    /// frame via `egui_glow`.
    fn draw_editor_overlay(&mut self, w: u32, h: u32) {
        if !self.editor_open {
            // Still drop any input that arrived while the overlay was
            // closed (e.g. the toggle keypress itself) so it isn't
            // replayed stale the next time it's opened.
            self.pending_egui_events.clear();
            return;
        }

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w as f32, h as f32),
            )),
            time: Some(self.start.elapsed().as_secs_f64()),
            modifiers: kt_modifiers_to_egui(self.modifiers),
            events: std::mem::take(&mut self.pending_egui_events),
            ..Default::default()
        };

        let mut do_eval = false;
        let mut do_load = false;
        let script_buffer = &mut self.script_buffer;
        let last_error = self.last_error.as_deref();

        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            egui::Window::new("Hydra Editor")
                .default_pos([16.0, 16.0])
                .default_size([w as f32 * 0.6, h as f32 * 0.6])
                .show(ctx, |ui| {
                    ui.label("Ctrl/Cmd+E to close - Ctrl/Cmd+Enter or \"Eval\" to run.");
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(script_buffer)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(16),
                        );
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Eval").clicked() {
                            do_eval = true;
                        }
                        if ui
                            .button("Load file")
                            .on_hover_text("Copy the currently active HYDRA_PLUGIN_SCRIPT (or default sketch) into the buffer above")
                            .clicked()
                        {
                            do_load = true;
                        }
                    });
                    if ctx.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command) {
                        do_eval = true;
                    }
                    if let Some(err) = last_error {
                        ui.colored_label(egui::Color32::from_rgb(230, 80, 80), err);
                    }
                });
        });

        if do_load {
            self.script_buffer = self.loaded_code.clone();
        }
        if do_eval {
            self.eval_editor_buffer();
        }

        let clipped_primitives = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        self.egui_painter.paint_and_update_textures(
            [w, h],
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
        unsafe {
            use glow::HasContext as _;
            self.gl.viewport(0, 0, w as i32, h as i32);
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
                modifiers,
            }) => {
                self.cursor_pos = (position.x as f32, position.y as f32);
                self.modifiers = modifiers;
                if let Some((start_cursor, start_size)) = self.resize_drag {
                    let dx = self.cursor_pos.0 - start_cursor.0;
                    let dy = self.cursor_pos.1 - start_cursor.1;
                    let new_w = (start_size.0 as f32 + dx).round() as u32;
                    let new_h = (start_size.1 as f32 + dy).round() as u32;
                    self.apply_resize(window, new_w, new_h);
                }
                if self.editor_open {
                    self.pending_egui_events
                        .push(egui::Event::PointerMoved(egui::pos2(
                            self.cursor_pos.0,
                            self.cursor_pos.1,
                        )));
                }
            }
            BvEvent::Mouse(MouseEvent::ButtonPressed { button, modifiers }) => {
                self.modifiers = modifiers;
                if button == MouseButton::Left && self.in_resize_handle(self.cursor_pos) {
                    self.resize_drag = Some((self.cursor_pos, self.editor_size.load()));
                }
                if self.editor_open {
                    if let Some(b) = bv_button_to_egui(button) {
                        self.pending_egui_events.push(egui::Event::PointerButton {
                            pos: egui::pos2(self.cursor_pos.0, self.cursor_pos.1),
                            button: b,
                            pressed: true,
                            modifiers: kt_modifiers_to_egui(modifiers),
                        });
                    }
                }
            }
            BvEvent::Mouse(MouseEvent::ButtonReleased { button, modifiers }) => {
                self.modifiers = modifiers;
                if button == MouseButton::Left {
                    self.resize_drag = None;
                }
                if self.editor_open {
                    if let Some(b) = bv_button_to_egui(button) {
                        self.pending_egui_events.push(egui::Event::PointerButton {
                            pos: egui::pos2(self.cursor_pos.0, self.cursor_pos.1),
                            button: b,
                            pressed: false,
                            modifiers: kt_modifiers_to_egui(modifiers),
                        });
                    }
                }
            }
            BvEvent::Mouse(MouseEvent::WheelScrolled { delta, modifiers }) => {
                self.modifiers = modifiers;
                if self.editor_open {
                    let delta = match delta {
                        baseview::ScrollDelta::Lines { x, y } => egui::vec2(x, y) * 24.0,
                        baseview::ScrollDelta::Pixels { x, y } => egui::vec2(x, y),
                    };
                    self.pending_egui_events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta,
                        modifiers: kt_modifiers_to_egui(modifiers),
                    });
                }
            }
            BvEvent::Keyboard(kb_event) => {
                self.modifiers = kb_event.modifiers;
                let is_toggle = kb_event.code == Code::KeyE
                    && kb_event.state == KeyState::Down
                    && (kb_event.modifiers.contains(KtModifiers::CONTROL)
                        || kb_event.modifiers.contains(KtModifiers::META));
                if is_toggle {
                    self.editor_open = !self.editor_open;
                } else if self.editor_open {
                    let pressed = kb_event.state == KeyState::Down;
                    let modifiers = kt_modifiers_to_egui(kb_event.modifiers);
                    if let Some(key) = kt_code_to_egui_key(kb_event.code) {
                        self.pending_egui_events.push(egui::Event::Key {
                            key,
                            physical_key: Some(key),
                            pressed,
                            repeat: false,
                            modifiers,
                        });
                    }
                    // Only plain character presses (no Ctrl/Cmd chord,
                    // which are shortcuts, not text) are forwarded as
                    // text input - mirrors how every other egui backend
                    // integration splits `Key`/`Text` events.
                    if pressed
                        && !modifiers.ctrl
                        && !modifiers.command
                        && !modifiers.mac_cmd
                    {
                        if let KtKey::Character(s) = &kb_event.key {
                            self.pending_egui_events
                                .push(egui::Event::Text(s.clone()));
                        }
                    }
                }
            }
            // Fires when the OS window itself is resized directly (only
            // reachable in the standalone binary - a parented/embedded
            // plugin view's size is instead driven by `apply_resize`
            // above, in response to our own in-content handle).
            BvEvent::Window(BvWindowEvent::Resized(info)) => {
                let logical = info.logical_size();
                self.editor_size.store((
                    logical.width.round() as u32,
                    logical.height.round() as u32,
                ));
            }
            _ => {}
        }

        baseview::EventStatus::Captured
    }
}

/// Converts a `keyboard_types::Modifiers` snapshot into `egui::Modifiers`.
/// `command`/`mac_cmd` follow platform convention (Cmd on macOS, Ctrl
/// elsewhere), matching every other egui backend integration.
fn kt_modifiers_to_egui(m: KtModifiers) -> egui::Modifiers {
    let mac_cmd = cfg!(target_os = "macos") && m.contains(KtModifiers::META);
    let command = if cfg!(target_os = "macos") {
        m.contains(KtModifiers::META)
    } else {
        m.contains(KtModifiers::CONTROL)
    };
    egui::Modifiers {
        alt: m.contains(KtModifiers::ALT),
        ctrl: m.contains(KtModifiers::CONTROL),
        shift: m.contains(KtModifiers::SHIFT),
        mac_cmd,
        command,
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

/// Maps a physical key position to an `egui::Key` - only the subset
/// needed for text editing (letters, digits, navigation, punctuation);
/// anything else (media keys, function keys, etc.) is ignored.
fn kt_code_to_egui_key(code: Code) -> Option<egui::Key> {
    use egui::Key as EK;
    Some(match code {
        Code::ArrowDown => EK::ArrowDown,
        Code::ArrowLeft => EK::ArrowLeft,
        Code::ArrowRight => EK::ArrowRight,
        Code::ArrowUp => EK::ArrowUp,
        Code::Escape => EK::Escape,
        Code::Tab => EK::Tab,
        Code::Backspace => EK::Backspace,
        Code::Enter | Code::NumpadEnter => EK::Enter,
        Code::Space => EK::Space,
        Code::Insert => EK::Insert,
        Code::Delete => EK::Delete,
        Code::Home => EK::Home,
        Code::End => EK::End,
        Code::PageUp => EK::PageUp,
        Code::PageDown => EK::PageDown,
        Code::Minus => EK::Minus,
        Code::Equal => EK::Equals,
        Code::Comma => EK::Comma,
        Code::Period => EK::Period,
        Code::Semicolon => EK::Semicolon,
        Code::Quote => EK::Quote,
        Code::Backquote => EK::Backtick,
        Code::BracketLeft => EK::OpenBracket,
        Code::BracketRight => EK::CloseBracket,
        Code::Backslash => EK::Backslash,
        Code::Slash => EK::Slash,
        Code::Digit0 => EK::Num0,
        Code::Digit1 => EK::Num1,
        Code::Digit2 => EK::Num2,
        Code::Digit3 => EK::Num3,
        Code::Digit4 => EK::Num4,
        Code::Digit5 => EK::Num5,
        Code::Digit6 => EK::Num6,
        Code::Digit7 => EK::Num7,
        Code::Digit8 => EK::Num8,
        Code::Digit9 => EK::Num9,
        Code::KeyA => EK::A,
        Code::KeyB => EK::B,
        Code::KeyC => EK::C,
        Code::KeyD => EK::D,
        Code::KeyE => EK::E,
        Code::KeyF => EK::F,
        Code::KeyG => EK::G,
        Code::KeyH => EK::H,
        Code::KeyI => EK::I,
        Code::KeyJ => EK::J,
        Code::KeyK => EK::K,
        Code::KeyL => EK::L,
        Code::KeyM => EK::M,
        Code::KeyN => EK::N,
        Code::KeyO => EK::O,
        Code::KeyP => EK::P,
        Code::KeyQ => EK::Q,
        Code::KeyR => EK::R,
        Code::KeyS => EK::S,
        Code::KeyT => EK::T,
        Code::KeyU => EK::U,
        Code::KeyV => EK::V,
        Code::KeyW => EK::W,
        Code::KeyX => EK::X,
        Code::KeyY => EK::Y,
        Code::KeyZ => EK::Z,
        _ => return None,
    })
}

struct HydraEditor {
    audio_buffer: AudioBuffer,
    midi_bridge: MidiBridge,
    editor_size: Arc<AtomicCell<(u32, u32)>>,
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
}

impl Default for HydraPluginParams {
    fn default() -> Self {
        Self {
            editor_size: Arc::new(AtomicCell::new((DEFAULT_EDITOR_WIDTH, DEFAULT_EDITOR_HEIGHT))),
        }
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
            params: Arc::new(HydraPluginParams::default()),
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
                NoteEvent::NoteOn { note, velocity, .. } => {
                    self.midi_bridge.note_on(note, velocity);
                }
                NoteEvent::NoteOff { note, velocity, .. } => {
                    // A real note-off always clears the note regardless of
                    // its (released) velocity - `MidiBridge::note_on`'s
                    // own velocity<=0 shorthand only matters for the raw
                    // system-MIDI-port path.
                    let _ = velocity;
                    self.midi_bridge.note_off(note);
                }
                NoteEvent::PolyPressure { note, pressure, .. } => {
                    self.midi_bridge.poly_pressure(note, pressure);
                }
                NoteEvent::MidiCC { cc, value, .. } => {
                    self.midi_bridge.control_change(cc, value);
                }
                NoteEvent::MidiChannelPressure { pressure, .. } => {
                    self.midi_bridge.channel_pressure(pressure);
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
