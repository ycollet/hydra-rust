//! Standalone binary entry point, built via nih-plug's `standalone` feature
//! (enabled in this crate's `Cargo.toml`). This is what gives
//! `hydra-plugin` its "Standalone" build target alongside VST3/CLAP, using
//! the exact same `HydraPlugin` code as the plugin formats.
fn main() -> std::process::ExitCode {
    match hydra_plugin::run_standalone() {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(err) => {
            eprintln!("hydra-plugin: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}
