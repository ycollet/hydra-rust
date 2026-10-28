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
Hydra's GLSL output straight into the host's editor window. A small
`egui`/`egui_glow` overlay provides file loading; there is no
`eframe`/winit dependency. It depends on
`hydra-rust` itself with `default-features = false`, matching the plain
library-embedding path the main crate's README documents for Sova.

## Status: v1 (functional, with real limitations)

- **Screenshots**: script `screencap()` is currently desktop-only; the plugin
  logs an explicit warning instead of saving a file.
- **Text**: global `hydraText.fontSize`/`.lineWidth` and per-call overrides
  work as in the desktop app. The bundled Hack font is still used.
- **Script source**: new instances read from the file at the `HYDRA_PLUGIN_SCRIPT`
  environment variable (falls back to a small built-in default sketch if
  unset/unreadable); restored host state uses its saved script snapshot.
  The file's mtime is polled once per frame and
  hot-reloaded on change - edit-and-save in any external editor to
  live-code from inside a DAW. A small overlay (click **"Load menu"** in
  the top-left corner, or press Ctrl/Cmd+E)
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
  by the host instead of a mic. `a.setBins`, `a.setCutoff`, `a.setScale`,
  `a.setSmooth`, and indexed `a.settings[i]` cutoff/scale/smooth controls
  configure that analyzer without requiring microphone capture.
  Audio is still passed through unchanged
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
  window). Channels are preserved as zero-based indices `0`–`15`;
  `midi.channel(n)` and explicit channel arguments filter host events.
  Device selectors see only the routed host input (index `0`, name
  `"host"`), not the external device names hidden by the DAW.
  Pure inline `.value(v => expression)` transforms compile to GLSL;
  callbacks requiring runtime branching or side effects are rejected.
- **Mouse**: wired into `mouse`/`m` reads from the window's actual
  cursor position, normalized to `[0, 1]` with the Y axis flipped to
  match the desktop app's own convention (origin at the bottom-left).
- **Script loader overlay**: click **"Load menu"** in the top-left
  corner to open a small panel (built
  with `egui`, rendered via `egui_glow` on top of the Hydra output)
  showing the currently active script path and a **"Load script..."**
  button that opens a native file-picker dialog (`rfd`) to pick a
  different file; any eval/shader error is shown inline. There's
  no in-window text editor: write/iterate on scripts in the `hydra`
  desktop app or any external editor, then point the plugin at the saved
  file. Close the panel with its title-bar close button; Ctrl+E (Cmd+E
  on macOS) also toggles it, if the host forwards the shortcut.
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

## Loading and editing scripts

1. Write a script in the `hydra` desktop app or any external editor and
   save it to a file.
2. Open the plugin window, click **"Load menu"** in the top-left corner,
   then **"Load script..."** and choose the file.
3. Edit and save that file to hot-reload it in the plugin without
   restarting the host. The panel displays the active path and any
   evaluation or shader-compilation error.

The loader starts closed, but the **"Load menu"** button remains visible
while it is closed. No keyboard shortcut is needed to open or close it.
The plugin loads files only; it does not include an inline code editor.
The `hydra-plugin` standalone binary has the same loader-only interface;
the `hydra` desktop app is the separate application with a code editor.

Alternatively, set `HYDRA_PLUGIN_SCRIPT` before launching the host to
select the initial file. Choosing another file in the loader replaces
that path without changing the environment variable. The selected path,
script source, and window size are saved in the host project. Reopening
the editor or restoring a project uses that source snapshot, even if the
original script is missing or has since changed. Further edits to the
selected file are still hot-reloaded while the editor is open.

## NSM sessions

The `hydra-plugin` standalone executable automatically joins New/Non
Session Manager when launched with `NSM_URL`. Add `hydra-plugin` as an
executable in the manager (ensure it is on `PATH`). It announces, opens
the assigned project, and acknowledges save requests only after writing
`plugin-state.json` inside its assigned session directory. This snapshot
uses the same persistent fields as VST3/CLAP: script path, embedded source,
and window size. Malformed snapshots and save failures are reported to
the manager rather than silently replaced with defaults.

No session switching, dirty-state notifications, or optional-GUI
capabilities are advertised. Invalid/unreachable NSM servers fall back to
normal standalone operation. The upstream nih-plug standalone JACK
backend still uses the fixed name `Hydra`, not NSM's assigned client ID;
stable JACK connection restoration with multiple instances is not
supported. NSM snapshot restoration is independent of the audio backend.

NSM announce/open/save and consecutive real-process restore/save
cycles were verified with a local OSC test manager on macOS, using both
debug and release standalones with the dummy audio backend. Three debug
restart cycles and unmanaged startup passed after the startup fixes.
The restored source and size
survived with the original script file absent. The same restart test
passed for the desktop `hydra` app, including its settings and scene banks.
Unit tests also cover host `Params` serialization, restoration into shared
editor state, legacy size-only state, and corrupt snapshots. A real DAW
VST3/CLAP project round-trip has not been exercised.

The workspace patches both baseview users (the plugin editor and
nih-plug's standalone wrapper) to upstream commit
`3e1297320f6c9feb01993cba7743277b2d645211`, which guards a null `NSWindow`
in `become_first_responder` during macOS window initialization. The patch
uses nih-plug's maintainer's fork as its source because Cargo requires a
different repository URL for git patches; the pinned commit is identical
to upstream. Audio/MIDI mutexes are initialized before processing, and
the audio ring buffer evicts old samples before appending, avoiding the
audio-thread allocations that otherwise abort debug builds. Allocation
checks remain enabled, with a regression test for first callbacks and
full buffers.

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
Interactive testing has since confirmed that loading a script works,
but the shortcut-only loader panel was reported inaccessible. The loader
now has a visible mouse-accessible entry point; interactive confirmation
in the affected host is still pending. The former text-editing overlay
has been removed, and the render viewport is set explicitly every frame
to follow window resizing.
