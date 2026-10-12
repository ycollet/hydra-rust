//! `cargo xtask bundle hydra-plugin --release` builds an actual loadable
//! `.vst3`/`.clap` bundle (correct bundle layout + manifest) from the
//! `hydra-plugin` crate, using nih-plug's own bundler. See `bundler.toml`
//! at the repo root for the bundle's display name.
fn main() -> nih_plug_xtask::Result<()> {
    nih_plug_xtask::main()
}
