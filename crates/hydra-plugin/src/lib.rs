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

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use baseview::{
    Event as BvEvent, MouseButton, MouseEvent, Size as BvSize, WindowEvent as BvWindowEvent,
    WindowHandle, WindowOpenOptions, WindowScalePolicy, gl::GlConfig,
};
use crossbeam::atomic::AtomicCell;
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

        let (gl, renderer, render_mode) = unsafe {
            gl_context.make_current();
            #[allow(clippy::arc_with_non_send_sync)]
            let gl = Arc::new(glow::Context::from_loader_function(|s| {
                gl_context.get_proc_address(s)
            }));
            let mut renderer = ShaderRenderer::new(gl.clone());
            let render_mode = Self::compile(&mut renderer, &code, &midi_bridge);
            gl_context.make_not_current();
            (gl, renderer, render_mode)
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
        }
    }

    /// Compiles `code` into `renderer`'s buffers, logging (not panicking)
    /// on a script error and leaving whatever previously compiled, so a
    /// typo while live-coding doesn't blank the editor window. Also
    /// applies any `midi.*` requests the script generated (CC smoothing,
    /// ADSR envelope slots) directly to `midi_bridge` - see the module
    /// doc comment for which requests are honored here.
    fn compile(renderer: &mut ShaderRenderer, code: &str, midi_bridge: &MidiBridge) -> RenderMode {
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
            self.render_mode = Self::compile(&mut self.renderer, &code, &self.midi_bridge);
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

        let midi_frame = self.midi_poller.poll();
        let uniforms = RenderUniforms {
            time: self.start.elapsed().as_secs_f32(),
            resolution: [w as f32, h as f32],
            // TODO: wire real mouse position from baseview's `on_event`
            // (see `HydraGlWindow::on_event` below) into `mouse`/`m` reads.
            mouse: [0.0, 0.0],
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
            BvEvent::Mouse(MouseEvent::CursorMoved { position, .. }) => {
                self.cursor_pos = (position.x as f32, position.y as f32);
                if let Some((start_cursor, start_size)) = self.resize_drag {
                    let dx = self.cursor_pos.0 - start_cursor.0;
                    let dy = self.cursor_pos.1 - start_cursor.1;
                    let new_w = (start_size.0 as f32 + dx).round() as u32;
                    let new_h = (start_size.1 as f32 + dy).round() as u32;
                    self.apply_resize(window, new_w, new_h);
                }
            }
            BvEvent::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                ..
            }) => {
                if self.in_resize_handle(self.cursor_pos) {
                    self.resize_drag = Some((self.cursor_pos, self.editor_size.load()));
                }
            }
            BvEvent::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                ..
            }) => {
                self.resize_drag = None;
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
