//! Standalone binary entry point, built via nih-plug's `standalone` feature
//! (enabled in this crate's `Cargo.toml`). This is what gives
//! `hydra-plugin` its "Standalone" build target alongside VST3/CLAP, using
//! the exact same `HydraPlugin` code as the plugin formats.
use hydra_plugin::HydraPlugin;
use nih_plug::nih_export_standalone;

fn main() {
    nih_export_standalone::<HydraPlugin>();
}
