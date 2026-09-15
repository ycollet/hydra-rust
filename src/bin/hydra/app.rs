use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use egui::{Color32, FontId, TextBuffer};
#[cfg(any(feature = "webcam", feature = "image_url", feature = "video"))]
use hydra_rust::eval::SourceRequest;
#[cfg(feature = "audio")]
use hydra_rust::eval::AudioRequest;
use hydra_rust::renderer::{self, RenderUniforms, ShaderRenderer};
#[cfg(feature = "webcam")]
use hydra_rust::source::{CameraStatus, SourceManager, NUM_SOURCES};
#[cfg(all(any(feature = "image_url", feature = "video"), not(feature = "webcam")))]
use hydra_rust::source::NUM_SOURCES;
#[cfg(feature = "audio")]
use hydra_rust::audio::AudioManager;
#[cfg(feature = "image_url")]
use hydra_rust::imageload::ImageManager;
#[cfg(feature = "midi")]
use hydra_rust::eval::MidiRequest;
#[cfg(feature = "midi")]
use hydra_rust::midi::MidiManager;
#[cfg(feature = "video")]
use hydra_rust::video::VideoManager;
use serde::{Deserialize, Serialize};

use crate::highlight::HydraHighlighter;

#[derive(Serialize, Deserialize)]
struct Session {
    code: String,
    tempo: f32,
    font_size: f32,
    text_opacity: f32,
    current_file: Option<PathBuf>,
    /// Scene banks (see `Bank`) - `#[serde(default)]` so a session file
    /// saved before this feature existed still loads (missing field, not
    /// a parse error).
    #[serde(default)]
    banks: Vec<Bank>,
    #[serde(default)]
    current_bank: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            code: String::new(),
            tempo: 120.0,
            font_size: 20.0,
            text_opacity: 0.55,
            current_file: None,
            banks: Vec::new(),
            current_bank: 0,
        }
    }
}

/// A scene bank: a native port of HYDRACTRL's own 4-bank-x-16-slot scene
/// storage (github.com/dxviie/HYDRACTRL). Each slot holds a saved Hydra
/// sketch's source code, `None` when empty - no thumbnail preview (unlike
/// HYDRACTRL's browser-canvas one), since there's no cheap equivalent here
/// and it wasn't requested.
const NUM_BANKS: usize = 4;
const SLOTS_PER_BANK: usize = 16;

#[derive(Serialize, Deserialize, Clone)]
struct Bank {
    #[serde(default = "empty_slots")]
    slots: Vec<Option<String>>,
}

impl Default for Bank {
    fn default() -> Self {
        Self { slots: empty_slots() }
    }
}

fn empty_slots() -> Vec<Option<String>> {
    vec![None; SLOTS_PER_BANK]
}

/// The on-disk `.bhr` ("bank hydra rust") export/import format - matches
/// `Bank`'s own shape plus a version tag for future-proofing (not
/// otherwise enforced yet).
#[derive(Serialize, Deserialize)]
struct BankFile {
    version: u32,
    slots: Vec<Option<String>>,
}

/// The on-disk `.shr` ("slot hydra rust") format: a single saved sketch,
/// versioned the same way `BankFile` is - `-ss`/`-sl`'s counterpart to
/// `-bs`/`-bl`, for sharing or backing up one sketch rather than a whole
/// bank. Not tied to any particular slot index (portable on its own).
#[derive(Serialize, Deserialize)]
struct SlotFile {
    version: u32,
    code: String,
}

fn session_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".hydra-rust.json")
}

impl Session {
    fn load() -> Self {
        std::fs::read_to_string(session_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(session_path(), json);
        }
    }
}

pub struct HydraApp {
    code: String,
    renderer: Option<ShaderRenderer>,
    error: Option<(String, Instant)>,
    start_time: Instant,
    highlighter: HydraHighlighter,
    tempo: f32,
    font_size: f32,
    text_opacity: f32,
    editor_visible: bool,
    editor_opacity: f32,
    sidebar_open: bool,
    current_file: Option<PathBuf>,
    /// True while a file opened via a CLI argument is sitting in the editor
    /// without having been run yet. Unlike the restored previous session
    /// (the user's own code, already run by them before), a freshly-opened
    /// file could be from an untrusted source (e.g. shared online) -
    /// running it automatically would let it silently call `initCam()`
    /// with no confirmation beyond the OS's one-time, blanket camera
    /// permission. Cleared the first time the user explicitly evaluates.
    pending_confirmation: bool,
    banks: Vec<Bank>,
    current_bank: usize,
    /// The last slot recalled or saved to, for the sidebar's highlight -
    /// purely a UI cue, not otherwise load-bearing.
    active_slot: Option<usize>,
    /// Set from `-bs/--bank-save`; when present, `Alt+X` writes straight
    /// here instead of prompting a save dialog.
    bank_export_path: Option<PathBuf>,
    #[cfg(feature = "webcam")]
    source_manager: SourceManager,
    #[cfg(feature = "audio")]
    audio_manager: AudioManager,
    #[cfg(feature = "image_url")]
    image_manager: ImageManager,
    #[cfg(feature = "midi")]
    midi_manager: MidiManager,
    #[cfg(feature = "video")]
    video_manager: VideoManager,
}

impl HydraApp {
    pub fn new(
        cc: &eframe::CreationContext,
        file_arg: Option<PathBuf>,
        bank_save: Option<PathBuf>,
        bank_load: Option<PathBuf>,
        slot_save: Option<PathBuf>,
        slot_load: Option<PathBuf>,
    ) -> Self {
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);

        let mut session = Session::load();
        if session.banks.len() != NUM_BANKS {
            session.banks = vec![Bank::default(); NUM_BANKS];
        }
        if session.current_bank >= NUM_BANKS {
            session.current_bank = 0;
        }
        let mut app = Self {
            code: session.code,
            renderer: cc.gl.clone().map(ShaderRenderer::new),
            error: None,
            start_time: Instant::now(),
            highlighter: HydraHighlighter::new(),
            tempo: session.tempo,
            font_size: session.font_size,
            text_opacity: session.text_opacity,
            editor_visible: true,
            editor_opacity: 1.0,
            sidebar_open: false,
            current_file: session.current_file,
            pending_confirmation: false,
            banks: session.banks,
            current_bank: session.current_bank,
            active_slot: None,
            bank_export_path: bank_save,
            #[cfg(feature = "webcam")]
            source_manager: SourceManager::new(),
            #[cfg(feature = "audio")]
            audio_manager: AudioManager::new(),
            #[cfg(feature = "image_url")]
            image_manager: ImageManager::new(),
            #[cfg(feature = "midi")]
            midi_manager: MidiManager::new(),
            #[cfg(feature = "video")]
            video_manager: VideoManager::new(),
        };

        if let Some(path) = bank_load {
            match std::fs::read_to_string(&path).map(|s| serde_json::from_str::<BankFile>(&s)) {
                Ok(Ok(file)) => app.banks[app.current_bank].slots = file.slots,
                Ok(Err(e)) => log::warn!("failed to parse bank {}: {e}", path.display()),
                Err(e) => log::warn!("failed to read bank {}: {e}", path.display()),
            }
        }

        let mut loaded_from_file = false;
        if let Some(path) = slot_load {
            match std::fs::read_to_string(&path).map(|s| serde_json::from_str::<SlotFile>(&s)) {
                Ok(Ok(file)) => {
                    app.code = file.code;
                    // Not a `.hydra` file - `Ctrl+S` shouldn't silently
                    // overwrite the `.shr` JSON with plain code text, so
                    // leave `current_file` unset (prompts for a new path).
                    app.current_file = None;
                    loaded_from_file = true;
                }
                Ok(Err(e)) => log::warn!("failed to parse slot {}: {e}", path.display()),
                Err(e) => log::warn!("failed to read slot {}: {e}", path.display()),
            }
        }
        if let Some(path) = file_arg {
            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    app.code = contents;
                    app.current_file = Some(path);
                    loaded_from_file = true;
                }
                Err(e) => log::warn!("failed to read {}: {e}", path.display()),
            }
        }

        if loaded_from_file {
            // Don't silently run a file just because it was opened - it
            // may not be a sketch the user wrote themselves (see the
            // `pending_confirmation` field doc comment). The restored
            // previous session, by contrast, is always the user's own
            // already-run code, so it's fine to resume automatically.
            app.pending_confirmation = !app.code.is_empty();
        } else if !app.code.is_empty() {
            app.evaluate();
        }

        // `-ss/--slot-save`: snapshot whatever code the app is starting
        // with (restored session, `-sl`, or `-i` - in that increasing
        // order of precedence) out to this path once, right at launch.
        // There's no per-slot keybinding to defer to the way `-bs` defers
        // to `Alt+X`, so this just happens immediately instead.
        if let Some(path) = slot_save {
            let file = SlotFile { version: 1, code: app.code.clone() };
            if let Ok(json) = serde_json::to_string_pretty(&file) {
                let _ = std::fs::write(&path, json);
            }
        }

        app
    }

    fn session(&self) -> Session {
        Session {
            code: self.code.clone(),
            tempo: self.tempo,
            font_size: self.font_size,
            text_opacity: self.text_opacity,
            current_file: self.current_file.clone(),
            banks: self.banks.clone(),
            current_bank: self.current_bank,
        }
    }

    /// Loads slot `index`'s saved code (if any) into the editor and
    /// evaluates it immediately - as explicit a user action as `Ctrl+O`'s
    /// `load_file` (which also auto-evaluates with no confirmation gate).
    fn recall_slot(&mut self, index: usize) {
        if let Some(code) = self.banks[self.current_bank].slots[index].clone() {
            self.code = code;
            self.current_file = None;
            self.active_slot = Some(index);
            self.evaluate();
        }
    }

    /// Saves the editor's current code into slot `index` of the active bank.
    fn save_slot(&mut self, index: usize) {
        self.banks[self.current_bank].slots[index] = Some(self.code.clone());
        self.active_slot = Some(index);
    }

    fn cycle_bank(&mut self, delta: i32) {
        self.current_bank = wrap_bank_index(self.current_bank, delta, self.banks.len());
        self.active_slot = None;
    }

    /// Exports the active bank's 16 slots as a `.bhr` file - to
    /// `bank_export_path` (set via `-bs/--bank-save`) if given, otherwise
    /// via an interactive save dialog, mirroring `save_file`.
    fn export_bank(&mut self) {
        let path = self.bank_export_path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("Hydra Bank", &["bhr"])
                .set_file_name("bank.bhr")
                .save_file()
        });
        if let Some(p) = path {
            let file = BankFile { version: 1, slots: self.banks[self.current_bank].slots.clone() };
            if let Ok(json) = serde_json::to_string_pretty(&file) {
                let _ = std::fs::write(&p, json);
            }
        }
    }

    /// Imports a `.bhr` file into the active bank, replacing its 16 slots -
    /// mirrors `load_file`.
    fn import_bank(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Hydra Bank", &["bhr"]).pick_file()
            && let Ok(contents) = std::fs::read_to_string(&p)
            && let Ok(file) = serde_json::from_str::<BankFile>(&contents)
        {
            self.banks[self.current_bank].slots = file.slots;
        }
    }

    fn evaluate(&mut self) {
        self.pending_confirmation = false;
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if self.code.is_empty() {
            let _ = renderer.compile_buffers(
                &[Some(hydra_rust::shader::DEFAULT_SHADER.to_owned()), None, None, None],
                Default::default(),
            );
            self.error = None;
            return;
        }
        match hydra_rust::eval(&self.code) {
            Ok(result) => {
                if let Some(ref td) = result.text_data {
                    renderer.upload_text(td);
                }
                let compile_errors = renderer.compile_buffers(&result.shaders, result.render_mode);
                #[cfg(any(feature = "webcam", feature = "image_url", feature = "video"))]
                for req in &result.source_requests {
                    match req {
                        #[cfg(feature = "webcam")]
                        SourceRequest::InitCam { slot, camera_index } => {
                            self.source_manager.init_cam(*slot, *camera_index);
                        }
                        #[cfg(feature = "image_url")]
                        SourceRequest::InitImage { slot, url } => {
                            self.image_manager.init_image(*slot, url.clone());
                        }
                        #[cfg(feature = "image_url")]
                        SourceRequest::InitGif { slot, url } => {
                            self.image_manager.init_gif(*slot, url.clone());
                        }
                        #[cfg(feature = "video")]
                        SourceRequest::InitVideo { slot, url } => {
                            self.video_manager.init_video(*slot, url.clone());
                        }
                    }
                }
                #[cfg(feature = "audio")]
                {
                    // Only open the microphone once a script is actually
                    // known to use it - either by calling one of the
                    // setters below, or by reading `a.fft[i]` (which
                    // compiles straight to an `iFft[...]` GLSL reference,
                    // with no AudioRequest of its own to detect it by).
                    // Merely having the `audio` feature compiled in must
                    // not, by itself, start capturing audio.
                    let uses_audio = !result.audio_requests.is_empty()
                        || result.shaders.iter().any(|s| s.as_deref().is_some_and(|s| s.contains("iFft[")));
                    if uses_audio {
                        self.audio_manager.ensure_started();
                    }
                    for req in &result.audio_requests {
                        match req {
                            AudioRequest::SetBins(n) => self.audio_manager.set_bins(*n),
                            AudioRequest::SetCutoff(c) => self.audio_manager.set_cutoff(*c),
                            AudioRequest::SetScale(s) => self.audio_manager.set_scale(*s),
                            AudioRequest::SetSmooth(s) => self.audio_manager.set_smooth(*s),
                        }
                    }
                }
                // Unlike audio (which lazily starts on inferred usage - see
                // above), real hydra-midi requires an explicit
                // `midi.start()` call before anything works, so `Start` is
                // just another queued request here, no heuristic needed.
                #[cfg(feature = "midi")]
                for req in &result.midi_requests {
                    match req {
                        MidiRequest::Start => self.midi_manager.ensure_started(),
                        MidiRequest::Pause => self.midi_manager.pause(),
                        MidiRequest::SetCcSmooth { index, factor } => {
                            self.midi_manager.set_cc_smooth(*index, *factor);
                        }
                        MidiRequest::AdsrSlot { slot, note, a, d, s, r } => {
                            self.midi_manager.set_adsr_slot(*slot, *note, *a, *d, *s, *r);
                        }
                    }
                }
                if compile_errors.is_empty() {
                    self.error = None;
                } else {
                    let msg = format!("shader compile error: {}", compile_errors.join("; "));
                    log::error!("{msg}");
                    self.error = Some((msg, Instant::now()));
                }
            }
            Err(e) => {
                log::error!("patch eval error: {e}");
                self.error = Some((e, Instant::now()));
            }
        }
    }

    fn save_file(&mut self) {
        let path = if let Some(ref p) = self.current_file {
            Some(p.clone())
        } else {
            rfd::FileDialog::new()
                .add_filter("Hydra", &["hydra"])
                .set_file_name("patch.hydra")
                .save_file()
        };
        if let Some(p) = path {
            let _ = std::fs::write(&p, &self.code);
            self.current_file = Some(p);
        }
    }

    fn load_file(&mut self) {
        let path = rfd::FileDialog::new()
            .add_filter("Hydra", &["hydra"])
            .pick_file();
        if let Some(p) = path
            && let Ok(contents) = std::fs::read_to_string(&p)
        {
            self.code = contents;
            self.current_file = Some(p);
            self.evaluate();
        }
    }

    fn paint_background(&mut self, ctx: &egui::Context) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };

        ctx.request_repaint();
        let rect = ctx.available_rect();
        let time = self.start_time.elapsed().as_secs_f32();
        let ppp = ctx.pixels_per_point();
        let res_w = (rect.width() * ppp) as u32;
        let res_h = (rect.height() * ppp) as u32;

        let mouse = ctx.input(|i| {
            i.pointer.hover_pos().map_or([0.0, 0.0], |pos| {
                [
                    ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0),
                    (1.0 - (pos.y - rect.min.y) / rect.height()).clamp(0.0, 1.0),
                ]
            })
        });

        renderer.ensure_resolution(res_w, res_h);

        #[cfg(feature = "webcam")]
        for slot in 0..NUM_SOURCES {
            if let Some(frame) = self.source_manager.poll(slot) {
                renderer.upload_source(slot, &frame);
            }
        }
        #[cfg(feature = "image_url")]
        for slot in 0..NUM_SOURCES {
            if let Some(frame) = self.image_manager.poll(slot) {
                renderer.upload_source(slot, &frame);
            }
        }
        #[cfg(feature = "video")]
        for slot in 0..NUM_SOURCES {
            if let Some(frame) = self.video_manager.poll(slot) {
                renderer.upload_source(slot, &frame);
            }
        }

        #[cfg(feature = "audio")]
        let fft = self.audio_manager.poll();
        #[cfg(not(feature = "audio"))]
        let fft = [0.0; hydra_rust::audio::NUM_FFT_BINS];

        #[cfg(feature = "midi")]
        let midi_frame = self.midi_manager.poll();
        #[cfg(not(feature = "midi"))]
        let midi_frame = {
            struct EmptyMidiFrame {
                note: [f32; hydra_rust::midi::NUM_MIDI_NOTES],
                velocity: [f32; hydra_rust::midi::NUM_MIDI_NOTES],
                cc: [f32; hydra_rust::midi::NUM_MIDI_CC],
                cc_smoothed: [f32; hydra_rust::midi::NUM_MIDI_CC],
                envelope: [f32; hydra_rust::midi::NUM_MIDI_ENVELOPES],
            }
            EmptyMidiFrame {
                note: [0.0; hydra_rust::midi::NUM_MIDI_NOTES],
                velocity: [0.0; hydra_rust::midi::NUM_MIDI_NOTES],
                cc: [0.0; hydra_rust::midi::NUM_MIDI_CC],
                cc_smoothed: [0.0; hydra_rust::midi::NUM_MIDI_CC],
                envelope: [0.0; hydra_rust::midi::NUM_MIDI_ENVELOPES],
            }
        };

        let snap = renderer.snapshot();
        let ping = renderer.ping().clone();
        let uniforms = RenderUniforms {
            time,
            resolution: [res_w as f32, res_h as f32],
            mouse,
            beat: time * (self.tempo / 60.0),
            tempo: self.tempo,
            phase: 0.0,
            fft,
            midi_note: midi_frame.note,
            midi_velocity: midi_frame.velocity,
            midi_cc: midi_frame.cc,
            midi_cc_smoothed: midi_frame.cc_smoothed,
            midi_envelope: midi_frame.envelope,
        };

        let cb = eframe::egui_glow::CallbackFn::new(move |_info, painter| {
            renderer::render_multipass(painter.gl(), &snap, &ping, uniforms);
        });

        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(cb),
        });
    }

    fn show_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("options")
            .exact_width(200.0)
            .show(ctx, |ui| {
                ui.heading("Options");
                ui.add_space(8.0);

                ui.label("Tempo (BPM)");
                ui.add(egui::Slider::new(&mut self.tempo, 20.0..=300.0));

                ui.add_space(4.0);
                ui.label("Font size");
                ui.add(egui::Slider::new(&mut self.font_size, 10.0..=40.0));

                ui.add_space(4.0);
                ui.label("Text opacity");
                ui.add(egui::Slider::new(&mut self.text_opacity, 0.0..=1.0));

                ui.add_space(4.0);
                ui.checkbox(&mut self.editor_visible, "Editor visible");

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(4.0);

                #[cfg(feature = "webcam")]
                {
                    ui.horizontal(|ui| {
                        ui.label("Cameras");
                        if ui.small_button("Refresh").clicked() {
                            self.source_manager.refresh_cameras();
                        }
                    });
                    let cameras = self.source_manager.cameras();
                    if cameras.is_empty() {
                        ui.small("No cameras found");
                    } else {
                        for cam in cameras {
                            ui.small(format!("[{}] {}", cam.index, cam.name));
                        }
                    }

                    ui.add_space(4.0);
                    for slot in 0..NUM_SOURCES {
                        match self.source_manager.status(slot) {
                            CameraStatus::Idle => {}
                            CameraStatus::Opening { camera_index } => {
                                ui.small(format!("s{slot}: opening cam {camera_index}..."));
                            }
                            CameraStatus::Active { camera_name, width, height, .. } => {
                                ui.small(format!("s{slot}: {camera_name} ({width}x{height})"));
                            }
                            CameraStatus::Error { message, .. } => {
                                ui.colored_label(
                                    Color32::from_rgb(255, 80, 80),
                                    format!("s{slot}: {message}"),
                                );
                            }
                        }
                    }
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(format!("Bank {} / {NUM_BANKS}", self.current_bank + 1));
                ui.horizontal_wrapped(|ui| {
                    let slots = self.banks[self.current_bank].slots.clone();
                    for (i, slot) in slots.iter().enumerate() {
                        let label = format!("{i:X}");
                        let filled = slot.is_some();
                        let active = self.active_slot == Some(i);
                        let color = if active {
                            Color32::from_rgb(255, 255, 255)
                        } else if filled {
                            Color32::from_rgb(255, 0, 200)
                        } else {
                            Color32::from_gray(90)
                        };
                        if ui
                            .add(egui::Button::new(egui::RichText::new(label).color(color)).small())
                            .clicked()
                        {
                            self.recall_slot(i);
                        }
                    }
                });

                ui.add_space(12.0);
                ui.separator();
                if let Some(ref p) = self.current_file {
                    ui.small(p.file_name().unwrap_or_default().to_string_lossy().to_string());
                }
                ui.add_space(4.0);
                ui.small("Tab — toggle this panel");
                ui.small("Ctrl+Enter — evaluate");
                ui.small("Ctrl+Shift+H — toggle editor");
                ui.small("Ctrl+S — save");
                ui.small("Ctrl+O — open");
                ui.small("Alt+0-9/A-F — recall slot");
                ui.small("Alt+Shift+0-9/A-F — save slot");
                ui.small("Alt+←/→ — cycle bank");
                ui.small("Alt+X / Alt+I — export/import bank");
            });
    }

    /// Persistent (non-fading, unlike `show_error_toast`) banner shown
    /// while a file opened via a CLI argument hasn't been explicitly run
    /// yet - see the `pending_confirmation` field doc comment.
    fn show_pending_confirmation_banner(&self, ctx: &egui::Context) {
        if !self.pending_confirmation {
            return;
        }
        egui::Area::new(egui::Id::new("pending_confirmation_banner"))
            .anchor(egui::Align2::CENTER_TOP, [0.0, 20.0])
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(60, 45, 0, 200))
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(
                                "Loaded from file, not yet run (it may access your camera/mic) - press Ctrl+Enter to run it",
                            )
                            .color(Color32::from_rgb(255, 200, 100))
                            .monospace(),
                        );
                    });
            });
    }

    fn show_error_toast(&mut self, ctx: &egui::Context) {
        let Some((msg, when)) = &self.error else { return };
        let elapsed = when.elapsed().as_secs_f32();
        if elapsed > 5.0 {
            self.error = None;
            return;
        }

        let alpha = if elapsed > 4.0 {
            ((5.0 - elapsed) * 255.0) as u8
        } else {
            255
        };

        let msg = msg.clone();
        egui::Area::new(egui::Id::new("error_toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -20.0])
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(40, 0, 0, alpha / 2))
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&msg)
                                .color(Color32::from_rgba_unmultiplied(255, 80, 80, alpha))
                                .monospace(),
                        );
                    });
            });
    }

    fn show_editor(&mut self, ctx: &egui::Context) {
        let opacity = self.editor_opacity;

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                if opacity <= 0.0 {
                    return;
                }

                let a = |base: u8| -> u8 { (base as f32 * opacity) as u8 };

                let v = ui.visuals_mut();
                v.extreme_bg_color = Color32::TRANSPARENT;
                v.widgets.inactive.bg_fill = Color32::TRANSPARENT;
                v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                v.widgets.hovered.bg_fill = Color32::from_white_alpha(a(20));
                v.widgets.active.bg_fill = Color32::from_white_alpha(a(30));
                v.selection.bg_fill = Color32::from_rgba_unmultiplied(80, 80, 200, a(100));
                v.override_text_color = Some(Color32::from_white_alpha(a(255)));

                let font_id = FontId::monospace(self.font_size);
                let highlighter = &self.highlighter;
                let font_clone = font_id.clone();
                let text_bg_alpha = (self.text_opacity * 255.0 * opacity) as u8;
                let text_bg = Color32::from_black_alpha(text_bg_alpha);
                let mut layouter = move |ui: &egui::Ui, text: &dyn TextBuffer, _wrap_width: f32| {
                    let job = highlighter.layout_job(text.as_str(), &font_clone, text_bg);
                    ui.fonts_mut(|f| f.layout_job(job))
                };

                let available = ui.available_size();
                ui.add_sized(
                    available,
                    egui::TextEdit::multiline(&mut self.code)
                        .font(font_id)
                        .desired_width(f32::INFINITY)
                        .layouter(&mut layouter),
                );
            });
    }
}

impl eframe::App for HydraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.paint_background(ctx);

        // Keep the active slot's persisted content in sync with the
        // editor's live code every frame - otherwise recalling a
        // different slot, cycling banks, opening a different file, or
        // quitting the app would all silently discard an unsaved
        // in-progress edit, since none of those explicitly save back to
        // the slot being left. Cheap: only actually clones/writes when
        // the code has changed since the last frame.
        if let Some(idx) = self.active_slot
            && self.banks[self.current_bank].slots[idx].as_deref() != Some(self.code.as_str())
        {
            self.banks[self.current_bank].slots[idx] = Some(self.code.clone());
        }

        let is_mac = ctx.os().is_mac();
        let cmd = |i: &egui::InputState, key: egui::Key| {
            i.key_pressed(key) && if is_mac { i.modifiers.mac_cmd } else { i.modifiers.ctrl }
        };

        if ctx.input(|i| cmd(i, egui::Key::Enter)) {
            self.evaluate();
        }
        if ctx.input(|i| cmd(i, egui::Key::S)) {
            self.save_file();
        }
        if ctx.input(|i| cmd(i, egui::Key::O)) {
            self.load_file();
        }
        if ctx.input(|i| {
            i.key_pressed(egui::Key::H)
                && i.modifiers.shift
                && if is_mac { i.modifiers.mac_cmd } else { i.modifiers.ctrl }
        }) {
            self.editor_visible = !self.editor_visible;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            self.sidebar_open = !self.sidebar_open;
        }

        // Scene-bank shortcuts (ported from HYDRACTRL's own bank system,
        // github.com/dxviie/HYDRACTRL): Alt+0-9/A-F recalls slot 0-F (hex)
        // in the active bank, Alt+Shift+<same> saves the current code into
        // it instead, Alt+Left/Right cycles between the 4 banks, and
        // Alt+X/Alt+I export/import the active bank as a `.bhr` file.
        if ctx.input(|i| i.modifiers.alt) {
            // Alt/Option is reserved for the bank shortcuts below - on
            // some keyboard layouts, Option(+Shift)+digit is an OS-level
            // dead-key/symbol composition (e.g. producing "≠", "»", ...),
            // which would otherwise also get typed into the code editor
            // as a stray character. Drop any composed text for as long as
            // Alt is held so only the shortcut itself fires.
            ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));

            const HEX_KEYS: [(egui::Key, usize); 16] = [
                (egui::Key::Num0, 0),
                (egui::Key::Num1, 1),
                (egui::Key::Num2, 2),
                (egui::Key::Num3, 3),
                (egui::Key::Num4, 4),
                (egui::Key::Num5, 5),
                (egui::Key::Num6, 6),
                (egui::Key::Num7, 7),
                (egui::Key::Num8, 8),
                (egui::Key::Num9, 9),
                (egui::Key::A, 10),
                (egui::Key::B, 11),
                (egui::Key::C, 12),
                (egui::Key::D, 13),
                (egui::Key::E, 14),
                (egui::Key::F, 15),
            ];
            for (key, idx) in HEX_KEYS {
                let (pressed, shift) = ctx.input(|i| (i.key_pressed(key), i.modifiers.shift));
                if pressed {
                    if shift {
                        self.save_slot(idx);
                    } else {
                        self.recall_slot(idx);
                    }
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                self.cycle_bank(-1);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                self.cycle_bank(1);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::X)) {
                self.export_bank();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::I)) {
                self.import_bank();
            }
        }

        self.editor_opacity = if self.editor_visible { 1.0 } else { 0.0 };

        if self.sidebar_open {
            self.show_sidebar(ctx);
        }

        self.show_editor(ctx);
        self.show_error_toast(ctx);
        self.show_pending_confirmation_banner(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        #[cfg(feature = "webcam")]
        self.source_manager.stop_all();
        #[cfg(feature = "video")]
        self.video_manager.stop_all();
        self.session().save();
    }
}

/// Wraps `current + delta` into `0..n`, in either direction - `-1` from `0`
/// lands on `n - 1`, matching how `Alt+Left`/`Alt+Right` should cycle
/// through the (fixed-size) bank list.
fn wrap_bank_index(current: usize, delta: i32, n: usize) -> usize {
    (current as i32 + delta).rem_euclid(n as i32) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_bank_index_moves_forward_and_back() {
        assert_eq!(wrap_bank_index(0, 1, 4), 1);
        assert_eq!(wrap_bank_index(1, -1, 4), 0);
    }

    #[test]
    fn wrap_bank_index_wraps_at_both_ends() {
        assert_eq!(wrap_bank_index(0, -1, 4), 3);
        assert_eq!(wrap_bank_index(3, 1, 4), 0);
    }

    #[test]
    fn bank_file_round_trips_through_json() {
        let file = BankFile {
            version: 1,
            slots: vec![Some("osc(60).out()".to_string()), None, None],
        };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: BankFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.slots, file.slots);
    }

    #[test]
    fn slot_file_round_trips_through_json() {
        let file = SlotFile { version: 1, code: "osc(60).out()".to_string() };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: SlotFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.code, file.code);
    }

    #[test]
    fn bank_defaults_to_sixteen_empty_slots() {
        let bank = Bank::default();
        assert_eq!(bank.slots.len(), SLOTS_PER_BANK);
        assert!(bank.slots.iter().all(Option::is_none));
    }

    #[test]
    fn session_without_bank_fields_still_deserializes() {
        // a session file saved before this feature existed has neither
        // `banks` nor `current_bank` - `#[serde(default)]` must keep it
        // loading instead of failing outright.
        let old_json = r#"{
            "code": "osc(60).out()",
            "tempo": 120.0,
            "font_size": 20.0,
            "text_opacity": 0.55,
            "current_file": null
        }"#;
        let session: Session = serde_json::from_str(old_json).unwrap();
        assert!(session.banks.is_empty());
        assert_eq!(session.current_bank, 0);
    }

    #[test]
    fn bank_with_missing_slots_field_defaults_to_empty() {
        let bank: Bank = serde_json::from_str("{}").unwrap();
        assert_eq!(bank.slots.len(), SLOTS_PER_BANK);
    }
}
