use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use egui::{Color32, FontId, TextBuffer};
use hydra_rust::eval::SourceRequest;
use hydra_rust::renderer::{self, RenderUniforms, ShaderRenderer};
use hydra_rust::source::{CameraStatus, SourceManager, NUM_SOURCES};
use serde::{Deserialize, Serialize};

use crate::highlight::HydraHighlighter;

#[derive(Serialize, Deserialize)]
struct Session {
    code: String,
    tempo: f32,
    font_size: f32,
    text_opacity: f32,
    current_file: Option<PathBuf>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            code: String::new(),
            tempo: 120.0,
            font_size: 20.0,
            text_opacity: 0.55,
            current_file: None,
        }
    }
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
    source_manager: SourceManager,
}

impl HydraApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);

        let session = Session::load();
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
            source_manager: SourceManager::new(),
        };
        if !app.code.is_empty() {
            app.evaluate();
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
        }
    }

    fn evaluate(&mut self) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if self.code.is_empty() {
            renderer.compile_buffers(
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
                renderer.compile_buffers(&result.shaders, result.render_mode);
                for req in &result.source_requests {
                    match req {
                        SourceRequest::InitCam { slot, camera_index } => {
                            self.source_manager.init_cam(*slot, *camera_index);
                        }
                    }
                }
                self.error = None;
            }
            Err(e) => self.error = Some((e, Instant::now())),
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

        for slot in 0..NUM_SOURCES {
            if let Some(frame) = self.source_manager.poll(slot) {
                renderer.upload_source(slot, &frame);
            }
        }

        let snap = renderer.snapshot();
        let ping = renderer.ping().clone();
        let uniforms = RenderUniforms {
            time,
            resolution: [res_w as f32, res_h as f32],
            mouse,
            beat: time * (self.tempo / 60.0),
            tempo: self.tempo,
            phase: 0.0,
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

        self.editor_opacity = if self.editor_visible { 1.0 } else { 0.0 };

        if self.sidebar_open {
            self.show_sidebar(ctx);
        }

        self.show_editor(ctx);
        self.show_error_toast(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        self.source_manager.stop_all();
        self.session().save();
    }
}
