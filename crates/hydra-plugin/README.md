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
  live-code from inside a DAW. A small overlay (Ctrl/Cmd+E to show/hide)
  has a **"Load script..."** button to switch files without restarting
  the host - see below.
- **Audio**: the host's own audio buffer is downmixed to mono in
  `process()` (the real-time audio thread) and pushed into a
  `hydra_rust::audio::AudioBuffer` - a bounded ring buffer with no
  FFT/allocation on the push side. The editor's render loop owns an
  `AudioAnalyzer` that polls FFT bins out of that buffer once per frame,
  so `a.fft[]` reacts to whatever audio the host feeds the plugin's input
  (e.g. a track routed into it), the same FFT-reactive pipeline the
  desktop app's microphone-based `AudioManager` uses internally - just fed
  by the host instead of a mic. Audio is still passed through unchanged
  (this is an analysis tap, not an audio effect). Tempo/transport sync
  (via nih-plug's host-transport info) is a natural follow-up, not yet
  wired up.
- **MIDI**: `MIDI_INPUT` is `MidiConfig::MidiCCs`, so the host delivers
  note-on/off, polyphonic/channel pressure, and CC events straight to
  `process()` - there's no system-MIDI-port connection here at all (unlike
  the desktop app's `midir`-based `MidiManager`). Those events are pushed
  into a shared `hydra_rust::midi::MidiBridge`, polled once per frame on
  the render thread via `hydra_rust::midi::MidiPoller`, feeding
  `midi_note`/`midi_velocity`/`midi_cc`/`midi_cc_smoothed`/
  `midi_envelope`/`midi_aftertouch`/`midi_channel_aftertouch` - so scripts
  using `note()`/`cc()`/`.adsr(...)`/`.smooth(...)` react to the host's own
  MIDI routing (e.g. a MIDI track feeding this plugin). `midi.start()`/
  `.pause()`/`.show()`/`.hide()` are accepted but are no-ops (there's no
  system MIDI port to start/stop, and no MIDI monitor overlay in this v1
  window).
- **Mouse**: wired into `mouse`/`m` reads from the window's actual
  cursor position, normalized to `[0, 1]` with the Y axis flipped to
  match the desktop app's own convention (origin at the bottom-left).
- **Script loader overlay**: Ctrl/Cmd+E shows/hides a small panel (built
  with `egui`, rendered via `egui_glow` on top of the Hydra output)
  showing the currently active script path and a **"Load script..."**
  button that opens a native file-picker dialog (`rfd`) to pick a
  different file; any eval/shader error is shown inline. There's
  deliberately no in-window text editor here (see "Why no in-window code
  editor?" below) - write/iterate on scripts in the standalone desktop
  app instead, then point the plugin at the saved file.
- **Window is resizable**: drag the bottom-right corner (a small
  translucent square marks the hotspot) to resize the window directly;
  nih-plug doesn't support host-initiated (OS-border) resize of an
  embedded plugin editor yet, so this is the plugin's own
  content-driven resize, the same pattern `nih_plug_vizia`'s
  `ResizeHandle` widget uses. The new size is reported to the host via
  `GuiContext::request_resize()`; if the host rejects it (some DAWs
  don't support runtime editor resizing) the resize is rolled back. In
  the standalone binary, dragging the real OS window border also works
  and keeps the handle in sync. The size is clamped to a minimum of
  256x144 and persisted across host project save/reload.
- **Webcam, OSC**: not exposed to the plugin build at all yet (the
  underlying `hydra-rust` features exist but aren't wired into this crate's
  `Cargo.toml`/editor - straightforward to add the same way `webcam` is
  feature-gated in the main crate).

## Why no in-window code editor?

An earlier version of this crate drew a full text-editing overlay
directly in the plugin window. It was removed: writing/iterating on a
script is a much better experience in the standalone desktop app
(syntax highlighting, real system clipboard, undo) than a from-scratch
mouse/keyboard-driven text box ever was here. It also inherited a
`baseview` Linux/X11 limitation - its keyboard backend hard-codes a
US-QWERTY scan-code table and ignores the system's actual keyboard
layout, so e.g. AZERTY users got the wrong characters typed. The
recommended workflow is: live-code in the standalone app, save the
script to a file, then use the overlay's "Load script..." button to
point the plugin at that file - no plugin restart needed, and no
keyboard-layout dependency since the loader overlay is mouse-only.

## Building

```sh
# From the repo root:
cargo build -p hydra-plugin --release          # compiles the lib (VST3/CLAP) + standalone bin
./target/release/hydra-plugin                  # run as a standalone app
HYDRA_PLUGIN_SCRIPT=/path/to/sketch.hydra ./target/release/hydra-plugin
```

### Producing a real `.vst3`/`.clap` bundle

`cargo build` alone only produces a raw `cdylib` - not something a DAW can
load directly (VST3/CLAP both expect a specific bundle directory layout
plus an `Info.plist`/manifest). This repo wires up nih-plug's own `xtask`
bundler (`xtask/`, `.cargo/config.toml`, `bundler.toml` at the repo root -
a separate, non-default workspace member, same as `hydra-plugin` itself)
to produce that:

```sh
cargo xtask bundle hydra-plugin --release
```

This produces `target/bundled/Hydra.vst3` and `target/bundled/Hydra.clap` -
drop either into your DAW's plugin folder (e.g.
`~/Library/Audio/Plug-Ins/VST3`/`CLAP` on macOS) to load it. The bundle's
display name ("Hydra") comes from the `["hydra-plugin"]` entry in the root
`bundler.toml`.

## Known-good state

The standalone binary has been run and confirmed to open its window and
render continuously (not stall/hang) on macOS with the default built-in
sketch. The audio bridge has been validated directly (pushing a synthetic
tone through `AudioBuffer`/`AudioAnalyzer` produces a clear non-zero FFT
response vs. silence); live host-audio-in-a-real-DAW has not yet been
tested. `cargo xtask bundle hydra-plugin --release` has been run and
verified to produce correctly-structured, validly (ad-hoc) codesigned
`.vst3`/`.clap` bundles on macOS (`codesign --verify --deep --strict`
passes on both, and `Info.plist` reports the expected display name) -
actually loading either in a real DAW has not yet been tested. The MIDI
bridge's unit-level plumbing is covered by `hydra_rust::midi`'s own test
suite (note-on/off, pressure, CC parsing); the standalone binary has been
run and confirmed to recognize a `--midi-input` device option and start
without error - feeding it a real MIDI note/CC stream from inside an
actual DAW host has not yet been tested. The resizable-window code
builds and passes clippy, the standalone binary has been confirmed to
start and run without panicking, and `cargo xtask bundle` still produces
valid, codesigned bundles after the change - actually dragging the
resize handle (and a host accepting/rejecting `request_resize()`) has
not yet been tested interactively in a real DAW or the standalone app.
The in-window editor overlay's build/clippy/bundle are likewise
verified and the standalone binary still starts and runs without
panicking after adding it. After interactive testing on Linux surfaced
two real bugs - the overlay's typed text following `baseview`'s
hard-coded US-QWERTY layout regardless of the system's actual layout
(e.g. AZERTY), and the rendered output not resizing along with the
window (the final screen blit wasn't keeping the GL viewport in sync
with the window's actual size on every platform) - the text-editing
overlay was removed entirely in favor of a mouse-only "Load script..."
file picker (see "Why no in-window code editor?" above), and the
viewport is now set explicitly every frame. Both fixes build/clippy/
bundle clean; re-testing the resize fix and the new loader button on
Linux is still pending.
