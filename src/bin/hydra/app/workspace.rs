use std::sync::Arc;

use super::{HydraApp, ParameterValues, egui, renderer};

impl HydraApp {
    pub(super) fn show_performance_workspace(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Performance");
        if self.parameters.is_empty() {
            ui.small("Use parameter(\"name\",value,min,max) in your script.");
        }
        let mut changed = false;
        for parameter in &self.parameters {
            let mut value = self
                .parameter_values
                .get(&parameter.name)
                .unwrap_or(parameter.default);
            ui.label(&parameter.name);
            if ui
                .add(egui::Slider::new(&mut value, parameter.min..=parameter.max))
                .changed()
            {
                match self.parameter_values.set(parameter, value) {
                    Ok(()) => changed = true,
                    Err(error) => {
                        Self::report_error(&mut self.error, error);
                    }
                }
            }
        }
        if !self.parameters.is_empty() && ui.button("Reset parameters").clicked() {
            match ParameterValues::reconcile(&self.parameters, &ParameterValues::default()) {
                Ok(values) => {
                    self.parameter_values = values;
                    changed = true;
                }
                Err(error) => {
                    Self::report_error(&mut self.error, error);
                }
            }
        }
        if changed {
            if let Some(renderer) = &mut self.renderer
                && let Err(error) = renderer.set_parameter_values(&self.parameter_values)
            {
                Self::report_error(&mut self.error, error);
            }
            if self.parameter_preset.is_none()
                && let Some(index) = self.active_slot
            {
                self.save_slot(index);
            }
        }
        ui.small("Shift-click a bank slot to save a parameter preset.");
        ui.collapsing("Output previews", |ui| {
            let Some(renderer) = &self.renderer else {
                return;
            };
            let snapshot = Arc::new(renderer.snapshot());
            let ping = renderer.ping().clone();
            let [width, height] = renderer.resolution().map(|value| value as f32);
            for output in 0..4 {
                ui.label(format!("o{output}"));
                let scale = (ui.available_width() / width).min(140.0 / height);
                let size = egui::vec2(width * scale, height * scale);
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let snapshot = snapshot.clone();
                let ping = ping.clone();
                ui.painter().add(egui::PaintCallback {
                    rect,
                    callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
                        let viewport = info.viewport_in_pixels();
                        if let Err(error) = renderer::render_output_preview(
                            painter.gl(),
                            &snapshot,
                            &ping,
                            output,
                            [
                                viewport.left_px,
                                viewport.from_bottom_px,
                                viewport.width_px,
                                viewport.height_px,
                            ],
                        ) {
                            log::error!("{error}");
                        }
                    })),
                });
            }
        });
    }
}
