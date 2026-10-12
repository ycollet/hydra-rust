# hydra-plugin

Hydra-rust packaged as a VST3/CLAP/standalone audio-plugin-host target, via
[nih-plug](https://github.com/robbert-vdh/nih-plug). This is a **separate
workspace member crate** - it does not affect the main `hydra-rust` library
or the `hydra` desktop app, and is not built by a plain `cargo build`/`cargo
test` run from the repo root (see `default-members` in the root
`Cargo.toml`).

## Why this is a plugin at all

VST3/CLAP are audio-plugin APIs, but a plugin's editor window can render
with a toolkit of its own choosing - including raw OpenGL. This crate uses
nih-plug's "bring your own GUI" support (`baseview` + a raw `glow` context,
the same `glow` version `hydra-rust`'s own `ShaderRenderer` uses) to render
Hydra's GLSL output straight into the host's editor window, with **no**
`eframe`/`egui`/winit dependency pulled in at all - it depends on
`hydra-rust` itself with `default-features = false`, matching the plain
library-embedding path the main crate's README documents for Sova.

## Status: v1 (functional, with real limitations)

- **Script source**: read from the file at the `HYDRA_PLUGIN_SCRIPT`
  environment variable (falls back to a small built-in default sketch if
  unset/unreadable). The file's mtime is polled once per frame and
  hot-reloaded on change - edit-and-save in any external editor to
  live-code from inside a DAW. There is **no in-window text editor yet**.
- **Audio**: passed through unchanged. The host's audio buffer is **not**
  yet fed into hydra-rust's FFT-reactive (`a.fft[]`) pipeline - this is the
  single biggest piece of remaining work to make the plugin feel like a
  "real" Hydra instance inside a DAW (tempo/transport sync is a natural
  follow-up once this is wired up, via nih-plug's host-transport info).
- **Mouse**: not yet wired into `mouse`/`m` reads (always `[0, 0]`).
- **Window is fixed-size** (`1024x576`) - not resizable yet.
- **MIDI, webcam, OSC**: not exposed to the plugin build at all yet (the
  underlying `hydra-rust` features exist but aren't wired into this crate's
  `Cargo.toml`/editor - straightforward to add the same way `webcam` is
  feature-gated in the main crate).

## Building

```sh
# From the repo root:
cargo build -p hydra-plugin --release          # compiles the lib (VST3/CLAP) + standalone bin
./target/release/hydra-plugin                  # run as a standalone app
HYDRA_PLUGIN_SCRIPT=/path/to/sketch.hydra ./target/release/hydra-plugin
```

nih-plug doesn't publish a `cargo install`-able bundler on crates.io; the
usual way to produce an actual loadable `.vst3`/`.clap` bundle (correct
bundle layout + `Info.plist`/manifest) is nih-plug's own `xtask` bundler. To
set that up in this repo, vendor (or depend on, as a dev-dependency)
`nih_plug_xtask` the same way nih-plug's own example plugins do - not yet
wired up here since the standalone build is enough to validate the
rendering path (which is the part reused by the real plugin formats too).

## Known-good state

The standalone binary has been run and confirmed to open its window and
render continuously (not stall/hang) on macOS with the default built-in
sketch. VST3/CLAP bundle-level loading in a real DAW has not yet been
tested - only `cargo build`'s compile-time checks (`nih_export_clap!`/
`nih_export_vst3!`) confirm the trait impls are well-formed.
