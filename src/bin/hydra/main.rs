mod app;
mod highlight;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Hydra")
            .with_maximized(true)
            .with_min_inner_size([400.0, 300.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Hydra",
        options,
        Box::new(|cc| Ok(Box::new(app::HydraApp::new(cc)))),
    )
}
