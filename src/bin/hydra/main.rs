mod app;
mod highlight;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init();

    let file_arg = std::env::args().nth(1).map(PathBuf::from);

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
        Box::new(move |cc| Ok(Box::new(app::HydraApp::new(cc, file_arg)))),
    )
}
