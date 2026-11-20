# hydra-rust documentation

Installation, standalone workflows, embedding guidance, compatibility notes,
and an example-driven function reference for hydra-rust. For a short project
overview and quick start, see [README.md](README.md).

[SPEC.md](SPEC.md) describes the language, JavaScript-compatibility
preprocessing, reactive semantics, and shader-chain model. The
[plugin guide](crates/hydra-plugin/README.md) covers VST3/CLAP builds and
the plugin's loader-only interface.

The scripting reference covers core functions and optional Cargo features.
Examples can be evaluated in the standalone editor or through
`hydra_rust::eval()` when the necessary features are enabled. Bare terminating
chains implicitly write to `o0`; an explicit `.out()` is also valid.

## Contents

- [Features](#features)
- [Build](#build)
- [NSM sessions](#nsm-sessions)
- [Audio onset and beat reactivity](#audio-onset-and-beat-reactivity)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Scene banks](#scene-banks)
- [Feature-gated functions](#feature-gated-functions)
- [Example](#example)
- [Used in Sova](#used-in-sova)
- [Differences from browser Hydra](#differences-from-browser-hydra)
- [Custom GLSL functions (`setFunction`)](#custom-glsl-functions-setfunction)
- [Screenshots](#screenshots)
- [Text rendering](#text-rendering)
- [Pattern easing](#pattern-easing)
- [Current limitations](#current-limitations)
- [Testing against a real-world sketch corpus](#testing-against-a-real-world-sketch-corpus)
- [How chains work](#how-chains-work)
- [Sources](#sources)
- [Geometry](#geometry)
- [Color](#color)
- [Blend](#blend)
- [Modulate](#modulate)
- [Other core functions](#other-core-functions)
- [Non compatible Hydra functions](#non-compatible-hydra-functions)
- [Reactive values](#reactive-values)
- [Math functions](#math-functions)
- [Patterns](#patterns)
- [Feature: `webcam`](#feature-webcam)
- [Feature: `audio`](#feature-audio)
- [Feature: `image_url`](#feature-image_url)
- [Feature: `midi`](#feature-midi)
- [Feature: `osc`](#feature-osc)
- [Feature: `video`](#feature-video)
- [Feature: `stream`](#feature-stream)
- [Known gaps and stub functions](#known-gaps-and-stub-functions)

## Features

### Function overview

| Category | Functions |
|----------|-----------|
| **Sources** | `osc`, `noise`, `voronoi`, `shape`, `gradient`, `solid`, `rings`, `checker` |
| **Geometry** | `rotate`, `scale`, `scroll`, `kaleid`, `pixelate`, `repeat`, `scrollX`, `scrollY`, `repeatX`, `repeatY`, `polar`, `cart`, `fold` |
| **Color** | `color`, `invert`, `contrast`, `brightness`, `saturate`, `hue`, `posterize`, `luma`, `colorama`, `shift`, `thresh`, `r`, `g`, `b` |
| **Blend** | `add`, `mult`, `blend`, `diff`, `layer`, `mask`, `sub` |
| **Modulate** | `modulate`, `modulateScale`, `modulateRotate`, `modulateRepeat`, `modulateRepeatX`, `modulateRepeatY`, `modulateKaleid`, `modulateScrollX`, `modulateScrollY`, `modulatePixelate`, `modulateHue` |
| **Other** | `text`, `src`, `initCam`, `fast`, `smooth`, `out`, `render`, `hush` |

### Rendering

- 4-buffer ping-pong architecture (`o0`–`o3`)
- 4 external source slots (`s0`–`s3`) for webcam feeds
- Text rendering via bundled Hack font
- GLSL 330 (OpenGL 3.3 core profile)

### Standalone binary

- Transparent editor overlay over GL visuals
- Syntax highlighting (OneDark theme)
- File save/load (`.hydra` files)
- Session persistence
- Options sidebar (tempo, font size, text opacity)
- Scene banks — 4 banks x 16 slots for instant scene recall, see [Scene banks](#scene-banks)
- Keyboard-driven — see [Keyboard shortcuts](#keyboard-shortcuts)

### Library

- No GUI dependencies when built with `default-features = false`
- Public API: `eval(code) → Result<EvalResult, String>`
- `ShaderRenderer` for OpenGL multipass rendering
- `SourceManager` for webcam capture (4 slots)

## Build

### Installers

Every pushed tag triggers [Release installers](.github/workflows/release.yml).
Once all builds succeed, installers and `SHA256SUMS` are attached to the tag's
[GitHub Release](https://github.com/sova-org/hydra-rust/releases):

| Platform | Installer |
|----------|-----------|
| Debian 12+ (x86-64) | `.deb` |
| Fedora 43+ (x86-64) | `.rpm` |
| macOS 12+ (Intel and Apple Silicon) | Separate `.dmg` files; drag Hydra to Applications |
| Windows (x86-64) | Per-user setup `.exe`, with Start menu entry and uninstaller |

These package the `hydra` desktop app with the default Cargo features (`gui`,
`nsm`), not the VST3/CLAP plugin or optional device/network features. Linux
packages declare their runtime dependencies; install them with `apt install
./file.deb` or `dnf install ./file.rpm`. All platforms require OpenGL 3.3.

Installer versions come from `[package].version` in `Cargo.toml`; update it
before tagging (for example, `git tag v0.1.0 && git push origin v0.1.0`).
Cargo prerelease versions produce prerelease GitHub Releases. The workflow
can be rerun to replace assets on an existing release.

Windows installers are unsigned; macOS apps are ad-hoc signed, not notarized.
Windows SmartScreen or macOS Gatekeeper may require explicit approval.

### From source

```bash
cargo build          # library + standalone binary (gui feature is on by default)
cargo run            # standalone binary
cargo test           # tests
cargo clippy         # lint
```

To embed `hydra-rust` as a library in another project (e.g. a host app driving its own `glow::Context`/window), depend on it with the default `gui` feature turned off — this skips `eframe`/`rfd` and the whole windowing stack (egui, egui_glow, winit, arboard, ...) they pull in, which the library's own `eval`/`renderer`/manager modules never need:

```toml
[dependencies]
hydra-rust = { path = "...", default-features = false, features = ["audio", "webcam"] } # pick whichever device features you need
```

`ShaderRenderer::new` only requires an `Arc<glow::Context>`, so a host can render into its own window/surface without pulling in the `hydra` binary's UI at all.

## NSM sessions

Both `hydra` and `hydra-plugin` automatically join New/Non Session Manager
when the manager launches them with `NSM_URL`. Add the executable to the
manager with its directory on `PATH`. NSM support is enabled by default;
custom minimal desktop builds need `--features gui,nsm`.

The desktop app saves its code, tempo, editor settings, and scene banks to
`session.json` in the manager-assigned client directory, independently of
`~/.hydra-rust.json`. A new managed session starts empty, not from the last
unmanaged session. Ctrl/Cmd+S saves to the assigned session without a file
dialog; Ctrl/Cmd+O imports script contents without making the original
file the save destination. NSM save requests use the same session snapshot.
Normal unmanaged startup and persistence are unchanged.

The plugin standalone saves its embedded script source, selected path,
and window size to `plugin-state.json`; see
[plugin NSM support and JACK limitations](crates/hydra-plugin/README.md#nsm-sessions).
Neither app advertises live session switching, dirty notifications, or an
optional GUI. Invalid/unreachable managers fall back to unmanaged mode;
corrupt session files and failed saves are reported instead of silently
discarding state.

### JACK / PipeWire audio input (Linux desktop app)

Build JACK support explicitly, then select it at runtime:

```sh
# Debian/Ubuntu build dependencies (in addition to the GUI dependencies):
sudo apt install libasound2-dev libjack-jackd2-dev pkg-config
cargo build --release --features jack
./target/release/hydra --jack sketch.hydra
# With PipeWire's JACK compatibility libraries, where a launcher is needed:
pw-jack ./target/release/hydra --jack sketch.hydra
```

`jack` includes the `audio` feature. Without `--jack`, the default audio
backend is unchanged. The flag is rejected on non-Linux platforms or in
builds without `jack`; JACK failures are logged and shown in the editor,
without falling back to another audio backend. A running JACK server (or
PipeWire's JACK compatibility layer) and its `libjack` library are required;
Hydra does not start a server automatically.

Audio capture stays lazy: load and explicitly evaluate an audio-reactive
sketch, for example `osc(10 + a.fft[0] * 100).out()`, before connecting audio.
The JACK client then appears as `hydra_in`, with input ports
`in_0` and `in_1`. Connect a DAW or media player's outputs to these ports
using your JACK/PipeWire patchbay. Hydra downmixes them to mono for FFT
analysis; this desktop input creates no audio outputs and does not
automatically connect to the microphone/system capture ports.

For RaySession/NSM, configure the desktop executable's arguments to include
`--jack` (and launch through `pw-jack` if your setup requires it).
The JACK client uses `<NSM client ID>_in`, preserving the assigned ID's case
and punctuation, with the same input port names. Save the connections using
the session manager's JACK connection-saving facility. On reopening a
session, evaluate the sketch to recreate the ports before expecting
connections to be restored. Multiple managed instances get distinct,
stable names from their NSM IDs. This naming support applies only to
`hydra`; the plugin standalone's upstream JACK naming limitation remains.

## Audio onset and beat reactivity

Build the desktop app with `cargo run --features audio` to use audio input
(or `--features jack` for the Linux JACK backend described above):

```javascript
osc(10 + a.onset * 40)
  .rotate(() => a.beat * 6.28318)
  .brightness(a.onset * 0.3)
  .out()
```

`a.onset` is a decaying transient pulse, `a.beat` is the estimated beat phase
from 0 to 1, and `a.bpm` is the estimated tempo. Beat/tempo remain zero until
regular onsets establish a tempo; this is a basic 60-200 BPM estimate, not
downbeat detection. These signals do not change the patch's global tempo.
The plugin analyzes its host audio through the same pipeline.
See [detection details and embedding API](SPEC.md#audio-onset-and-beat-detection).

## Keyboard shortcuts

The `Enter`-based shortcuts mirror [upstream Hydra](https://github.com/hydra-synth/hydra)'s own CodeMirror keymap (`Ctrl/Cmd+Enter`/`Alt+Enter`/`Ctrl/Cmd+Shift+Enter` for eval line/block/all); `Ctrl/Cmd+Alt+Enter` (eval selection) is specific to this native port, since upstream's `Ctrl/Cmd+Shift+Enter` slot was needed for "eval all" instead.

| Shortcut | Action |
|----------|--------|
| `Ctrl/Cmd + Enter` | Evaluate only the line the cursor is on |
| `Alt + Enter` | Evaluate the current "block": the contiguous run of non-blank lines around the cursor, stopping at the first blank line (or file boundary) above/below |
| `Ctrl/Cmd + Shift + Enter` | Evaluate the whole sketch |
| `Ctrl/Cmd + Alt + Enter` | Evaluate only the selected text in the editor (falls back to the whole sketch if there's no selection) |
| `Ctrl/Cmd + S` | Save the current sketch to a `.hydra` file |
| `Ctrl/Cmd + O` | Open a `.hydra` file |
| `Ctrl/Cmd + P` or `Ctrl/Cmd + Shift + S` | Save a screenshot of the current render (no sidebar/editor) as `hydra_screenshot_DD_MM_YYYY.png` in the current directory |
| `Ctrl/Cmd + /` | Toggle `//` line comments on the selected lines (or the cursor's current line, with no selection) |
| `Ctrl/Cmd + Shift + F` | Format code: breaks a single-line method chain (e.g. `osc(60).rotate(0.1).out()`) onto one call per line. Always reformats the whole script, matching upstream (an already multi-line chain is left untouched) |
| `Ctrl/Cmd + Shift + H` | Toggle editor visibility (hide the code overlay, keep the visuals running) |
| `Tab` | Toggle the options sidebar — tempo/font/text-opacity, camera status, and the scene-bank grid (see below) |
| `Alt/⌥ + 0`-`9` / `A`-`F` | Recall slot `0`-`F` (hex) in the active bank — loads and immediately evaluates its saved code |
| `Alt/⌥ + Shift + 0`-`9` / `A`-`F` | Save the editor's current code into *that* slot (the recalled slot's own content is auto-saved continuously as you type — this is for copying the current code into a *different*/new slot) |
| `Alt/⌥ + ←` / `→` | Cycle to the previous/next scene bank |
| `Alt/⌥ + X` | Export the active bank (16 slots) as a `.bhr` file |
| `Alt/⌥ + I` | Import a `.bhr` file into the active bank, replacing its 16 slots |

## Scene banks

A native port of [HYDRACTRL](https://github.com/dxviie/HYDRACTRL)'s own scene-bank system: **4 banks x 16 slots** (64 scenes total) for instant recall during a live-coding performance. Each slot holds a saved sketch's source code (no thumbnail preview, unlike HYDRACTRL's browser-canvas one — there's no cheap equivalent in a native GL app). Banks persist across restarts as part of the usual session file (`~/.hydra-rust.json`); a single bank (16 slots) can also be exported/imported as its own portable `.bhr` ("bank hydra rust") JSON file. See [Keyboard shortcuts](#keyboard-shortcuts) above for the `Alt`-based recall/save/cycle/export/import shortcuts (there's no MIDI-controller bank-switch mapping to defer to here, unlike HYDRACTRL, so cycling banks is always available).

Once you've recalled or saved-to a slot, that slot keeps itself in sync with whatever you type from then on — switching to another slot, cycling banks, opening a different file, or quitting the app never silently discards an in-progress edit.

Slots can also be clicked directly in the sidebar (`Tab` to show it) — the current bank number and a `0`-`F` slot grid are displayed there (white = the last slot you touched, magenta = filled, gray = empty).

Five CLI flags round out the workflow (in addition to the existing bare positional argument, e.g. `hydra some.hydra`, which still works):

```bash
hydra -i mysketch.hydra           # --input: load a sketch at startup (same as the bare positional arg)
hydra -bl mybank.bhr              # --bank-load: preload a .bhr file into the active bank at startup
hydra -bs mybank.bhr              # --bank-save: Alt+X writes straight to this path instead of prompting
hydra -bl mybank.bhr -bs mybank.bhr  # load and keep exporting to the same file
hydra -sl mysketch.shr             # --slot-load: load a .shr file's code at startup (like -i, JSON-wrapped)
hydra -ss mysketch.shr             # --slot-save: snapshot the starting code out to this path once, at launch
```

`.shr` ("slot hydra rust") is the single-sketch counterpart to `.bhr`: just `{"version": 1, "code": "..."}`, versus `.bhr`'s 16-element `slots` array. It's mostly a convenience/consistency format — a plain `.hydra` file already does the "one saved sketch" job just fine — but it shares `.bhr`'s versioned-JSON envelope (useful if either format ever needs extra metadata later) and gives `-ss`/`-sl` a natural single-sketch analogue to `-bs`/`-bl`. Unlike `-bs` (which pre-fills `Alt+X`'s target for later), `-ss` has no keybinding to defer to, so it writes immediately at launch instead.

## Feature-gated functions

Optional device and network functions require their matching Cargo features:

```bash
cargo run --features webcam,audio,image_url,midi,osc,video,stream --bin hydra
```

See the [webcam](#feature-webcam), [audio](#feature-audio),
[image/GIF](#feature-image_url), [MIDI](#feature-midi), [OSC](#feature-osc),
[video](#feature-video), and [streaming](#feature-stream) references below
for commands, parameters, examples, and dependencies. `fft` provides the
audio scripting API for host-fed samples without microphone capture;
`jack` adds the Linux backend described under
[JACK / PipeWire audio input](#jack--pipewire-audio-input-linux-desktop-app).
Without the matching feature, functions are unavailable or explicitly
stubbed as described in [the stub-function table](#stub-functions-accepted-but-not-yet-implemented).

## Example

```
osc(60.0, 0.1, time * 0.5)
  .kaleid(4.0)
  .color(1.0, 0.5, 0.3)
  .modulate(noise(3.0, 0.1), 0.02)
  .rotate(0.0, 0.1)
  .out()
```

Built-in globals available as GLSL expressions: `time`, `beat`, `tempo`, `phase`, `mouseX`, `mouseY`.

## Used in Sova

hydra-rust is the visual engine of [Sova](https://github.com/Bubobubobubobubo/Sova), a polyglot live coding sequencer for music and visuals. In Sova, hydra shaders sync to the musical clock via `beat`, `tempo`, and `phase`, and can be shared across peers in multiplayer sessions. A host like Sova can depend on `hydra-rust` with `default-features = false` (see [Build](#build)) to use `eval`/`ShaderRenderer` directly inside its own window, without the standalone app's `eframe`-based UI.

## Differences from browser Hydra

- **Rhai, not JavaScript.** A preprocessing layer accepts supported JavaScript-style syntax, including selected arrow-function forms, but there is no JavaScript runtime. Reactive expressions such as `osc(60.0, sin(time))` compile to GLSL; see [SPEC.md](SPEC.md) for supported syntax.
- **Expression variables.** `time`, `beat`, `tempo`, `phase` are GLSL uniform references, not callback functions.
- **Mouse input.** `mouseX` and `mouseY` are normalized to `[0, 1]`.
- **No `speed` global.** Animation speed is per-source, controlled through function arguments.
- **GLSL 330.** Targets OpenGL 3.3 core profile.

## Custom GLSL functions (`setFunction`)

`setFunction({name, type, inputs, glsl})` defines a new source/coord/color/
combine/combineCoord function at runtime, the same way real hydra.js does —
useful for pasting in a shader from a community extension or tutorial
without editing hydra-rust itself. Must be its own statement (not chained
off of, e.g. `setFunction({...}).out()`); define it, then call the new
function by name afterwards:

```js
setFunction({
  name: 'gradient2',
  type: 'src',
  inputs: [{type: 'float', name: 'speed', default: 0}],
  glsl: `return vec4(sin(speed*time), st, 1.0);`
});
gradient2(0.5).out();
```

Every input is treated as a plain `float` regardless of its declared
`type`; `time`/`resolution` in the `glsl` body are rewritten to this
project's own `iTime`/`iResolution` uniform names. See `SPEC.md` §12 for
the full details and limitations.

## Screenshots

Call `screencap()` in a script to save the next rendered frame, without the
editor/sidebar, using the same PNG path as the screenshot keyboard shortcuts:
`hydra_screenshot_DD_MM_YYYY.png` in the current directory, with a numeric
suffix when needed. Multiple calls in one evaluation produce one capture;
evaluating the script again requests another. Failed evaluations or shader
compilation do not queue a capture.

This is desktop-only for now. The plugin logs a warning, and library hosts
can handle `EvalResult.screencap` themselves; `eval()` never writes a file.

## Text rendering

`text("hello")` draws filled text; `strokeText("hello")` draws hollow
outlines; `fillStrokeText("hello")` and `strokeFillText("hello")` draw
both. All use the bundled Hack font and one shared text texture (the last
text call in an evaluation wins).

`hydraText.fontSize` and `.lineWidth` set defaults for subsequent text calls
in the current evaluation. Each call can override either property without
changing those defaults:

```javascript
hydraText.fontSize = "50%"
hydraText.lineWidth = "4%"
strokeText("hello", {fontSize: "100px", lineWidth: 3}).out()
```

`fontSize` accepts `"auto"` (default), a percentage of the auto-fit size,
or a number/numeric string with optional `px`. `lineWidth` accepts pixels
or a percentage of the resolved font size (default `"2%"`); zero disables
the stroke, and fractional widths are supported. Pixels refer to the
512x512 text texture, not the window. Oversized text is clipped; font sizes
must resolve to more than zero and at most 4096 pixels.

Invalid or non-static settings log a warning and use the built-in default
for that property. Defaults reset between evaluations. Outlines use an
approximate distance field, with bounded processing cost even for wide
strokes. Fill and stroke remain white, so both drawing orders have the
same result. Arbitrary font selection and other styling properties remain
accepted but ignored.

## Pattern easing

`[0, 1].ease('easeInOutCubic')` uses Hydra's named interpolation curves:
`linear`, `sin`, and `easeIn`/`easeOut`/`easeInOut` with `Quad`, `Cubic`,
`Quart`, or `Quint`. Calling `.ease()` selects `linear`; a recognized
curve resets the smoothing window to `1`. A later `.smooth(amount)`
changes that window, and `.fit()` preserves the curve. Unknown names
log a warning and leave the pattern unchanged.

## Current limitations

- Max nesting depth of 16
- Audio reactivity, webcam input, image-URL loading, MIDI input, video playback, and WebRTC streaming each require building with their own Cargo feature (off by default) — see [Feature-gated functions](#feature-gated-functions) above
- `initVideo` (the `video` feature) additionally requires a standalone `ffmpeg` binary on `PATH` at runtime — it's spawned as a subprocess, not linked into this binary, so building hydra-rust itself never needs FFmpeg's dev libraries. Missing it just logs a warning rather than failing. `initStream`/`broadcastStream` (the `stream` feature) share this same `ffmpeg` requirement.
- `initStream`/`broadcastStream` are hydra-rust-to-hydra-rust only — not interoperable with real hydra.js's own (currently broken) `initStream`/`pb.setName()` — and connect directly by IP:port with no NAT traversal, so both instances need to be reachable from each other directly (typically the same LAN). `broadcastStream` supports any number of simultaneous viewers, sharing one `ffmpeg` encode between them.
- `broadcastStream` downscales anything wider than 1280px before encoding, regardless of the actual window/display resolution — realtime software VP8 encoding at a Retina display's true (2x+) framebuffer resolution can't keep up otherwise (found via real testing, not just a theoretical cap).

### Stub functions (accepted, but not yet implemented)

These are registered so scripts calling them don't hard-error, but they don't do anything real yet:

| Function | Status |
|----------|--------|
| `initImage(idx, url)` | Real implementation behind the `image_url` feature: fetches the URL and decodes it (PNG/JPEG/GIF/WebP) in the background, then uploads it to the source slot once it's ready, through the same texture path webcam frames use. Without that feature, it's a no-op (returns the source as a chainable value, like real hydra.js) |
| `initVideo(idx, url)` | Real implementation behind the `video` feature: streams frames from a local file or URL via a standalone `ffmpeg` subprocess (not linked into this binary — `ffmpeg` must be on `PATH` at runtime; missing it logs one clear warning rather than failing), looping indefinitely. Without that feature, it's a no-op (returns the source as a chainable value) |
| `initGif(idx, url)` | Real implementation behind the `image_url` feature: fetches the URL, decodes every frame up front, and cycles through them by elapsed time once loaded, looping indefinitely — same texture path webcam frames use. Without that feature, it's a no-op (returns the source as a chainable value) |
| `initStream(idx, url)` | Real implementation behind the `stream` feature: connects to another hydra-rust instance over WebRTC (see [Feature: `stream`](#feature-stream)) — not real hydra.js's own (currently broken) `initStream`. Without that feature, it's a no-op (returns the source as a chainable value) |
| `initScreen(idx[, screen])` | No-op (returns the source as a chainable value) — no screen/display capture |
| `sN.init({src: ...})` | No-op (returns the slot's source as a chainable value) — not a real hydra.js API at all, but a pattern some external platforms use to feed a p5.js canvas/DOM element into a source slot; no such capture pipeline exists here |
| `P5(...)` / `new P5(...)` | Returns a plain settable map (like `hydraText`) rather than hard-erroring, so an assignment (`let p1 = P5(...)`) and later property reads/writes on it still work — p5.js is a whole separate creative-coding framework with no Rust equivalent here. A handful of its most commonly-called instance methods (`hide`, `show`, `textSize`, `fill`, `stroke`, `strokeWeight`, `noStroke`, `noFill`, `textFont`, `color`, `text`) are additionally registered as no-ops on that map; the rest of its (large) API isn't, and unregistered numeric properties (`p1.frameCount`, `mouseX`, ...) read as `()`, which arithmetic (`+`/`-`/`*`/`/`) treats as `0` rather than hard-failing. p5.js's global-mode `color(r,g,b[,a])` constructor (no `p1.` prefix) is also registered separately from hydra's own `.color()` chain method |
| `new THREE.PerspectiveCamera(...)` / `new THREE.WebGLRenderer(...)` / `renderer.setSize(...)` | No-ops (return a settable map) — three.js setup boilerplate. `THREE` itself is never a real module — `let THREE = await import(url)` degrades to a plain string once `await`/`import` are stripped as bare keywords — so these are registered against that string receiver too. No 3D rendering pipeline exists here |
| `document.createElement(...)` / `.getElementById(...)` / `.querySelector(...)` / `.getElementsByTagName(...)` | Each returns a plain settable map (like `P5(...)`) rather than hard-erroring — real sketches commonly build an offscreen `<canvas>`/`<img>` element this way, usually to feed a p5.js overlay or draw text via a 2D canvas context; no DOM exists here to back it. `.getContext("2d")` on the resulting map returns another settable map with its own common methods (`fillText`, `fillRect`, `clearRect`, `strokeText`, `save`/`restore`, `translate`/`scale`, `measureText`, `createLinearGradient`) registered as no-ops; properties like `.fillStyle`/`.font` need no registration, same as `hydraText` |
| `canvas.setRelativeSize(...)` / `.setAlign(...)` / `.setLinear()` / `.setNearest()` | No-op — a top-level `canvas` config object the `hyper-hydra` community extension exposes, not a real hydra.js API at all |
| `Scene(name)` | No-op — not a hydra.js API at all; some external VJ/live-coding integrations use it to switch named cue banks |
| `loadScript(url)` | No-op — no dynamic module loading. Some of the most commonly-loaded community extensions' functions are ported natively instead (`spiral`, `turb`, `uturb`, `unoise`, `whitenoise`, `colornoise`, `warp`, `cwarp`, `ncontour`, `pulse`, `pulsetrain`, `hextile`, `concentric`, `brick`, `wave`, `lissa`, `inversion`, `mirrorX`/`mirrorY`/`mirrorX2`/`mirrorY2`/`mirrorWrap`, `mandeloffs`, `colreflect` — see SPEC.md §6); anything else the loaded script would have defined still won't exist |
| `pb.setName(name)` / `pb.list()` | No-op — not a hydra.js API at all; boilerplate an external platform injects when a sketch is exported/shared |
| `hydraText.font = ...` / other styling properties | Font selection and other styling remain no-ops; `fontSize` and `lineWidth` are implemented globally and per call (see Text rendering above) |

## Testing against a real-world sketch corpus

`examples/check_corpus.rs` is a fast conformance-testing harness: it walks a
directory of `.hydra` files and evaluates each one directly through
`hydra_rust::eval()` — no window, no GL context — so tens of thousands of
sketches check in seconds. It buckets failures by cause (missing function,
syntax error, etc.) and writes the full per-file failure list to JSON.

The sketches themselves are **not** part of this repository (they're
third-party, user-submitted, CC BY-NC-SA-licensed content — see
`sketches/` in `.gitignore`). Fetch a fresh copy of the public
[hydra-synth](https://hydra.ojack.xyz/) sketch database instead:

```bash
python3 scripts/fetch_corpus.py          # downloads into ./sketches
```

Then run the harness against it (built with `--release` and every feature,
so `a.fft[...]`/`initCam(...)`/`initImage(...)`/`note(...)`-style sketches
don't fail just because those functions aren't registered):

```bash
cargo run --release --features webcam,audio,image_url,midi,osc,video,stream --example check_corpus -- sketches
```

This prints an ok/failed count and the top failure buckets, and writes
`check_corpus_failures.json` (path, error message per failing file) for
deeper digging.

### Known-broken sketches: buffers beyond `o0`-`o3`

hydra-rust only implements 4 output buffers (`o0`-`o3`), matching real
hydra.js's own limit. Some sketches in the public corpus reference a higher
buffer index (`o4` up through a clearly-typo'd `o10987654`) — these are
errors in the original sketch itself, not a hydra-rust conformance gap;
real hydra.js would reject them too. Regenerate this list from a fresh
`check_corpus_failures.json` (see above) yourself:

```bash
python3 -c "
import json, re
data = json.load(open('check_corpus_failures.json'))
for e in data:
    if re.match(r'Variable not found: o\d+ \(', e['error']):
        print(e['file'], '-', e['error'])
"
```

<details>
<summary>73 affected sketches (snapshot from this session)</summary>

- `o4` (21): `43ehBRbYLii1rgGr`, `5Y6r4TLGs4XKYwhx`, `6zFiWCIoFUL7n9zu`, `ASb5VAew6t067Uio`, `AjFg5nRpovNBAj9D`, `E4G3DklYF1VpyfMT`, `FxkxjQsCeFFrWsdg`, `LNfi2yCRY09SxR64`, `QslDk59HczJhaGWU`, `V2fAjkgS8zhheRvV`, `VFUf3eU67CTCUfYU`, `Y7Sqy1P4KF3CLxUU`, `YugaBL74Pw7UAvlb`, `aodWvyME0xI3jB7m`, `fEIXp0g0ZwQEy9Rc`, `gtwk9SGqsA4GODJd`, `jQX9cpluGcoqpQQA`, `jRZpwk5wxZzLT1J5`, `lJ6hhdXr33ilhfBF`, `lQVEtvWCB3EqXWp3`, `utYYjFUXRO9Z5Aut`
- `o5` (15): `1wZaiXko9Q8rZ1xq`, `Ad4xNLD6ulqEmE49`, `Kx101mGtz5DURR5S`, `MdFbiECQ8QehBQqM`, `NXkmjf7HlaBHRudY`, `OpJyLigtwsL4cvWf`, `QsWyj8GGPOTvQGhO`, `WJiPmKwkFxJzR9r7`, `cvV5LkExVeKRWE6U`, `eT1yrdX0o8AuZeUz`, `grROnRapw2HQmIZG`, `lb8bQm84w1odpl0O`, `mV8VYA3ocxtMZhPA`, `meKGY0Moy4fEVWSG`, `woXLgY6LNG1guPuk`
- `o6` (1): `bQYkAZjS9nqonZZG`
- `o7` (4): `UhzTt8jwuXe5SoIm`, `mZTRwQ6OMVqNedC8`, `n2XHJwUdnVbJWlHQ`, `sKtOcGCST9KO4xRG`
- `o8` (1): `ZbUc4DqjLaNZqbO9`
- `o9` (13): `6G6CLqvkidogMUdd`, `8itoqbZZAUjcAeIW`, `E12OtehM9mjervDh`, `GfMCIDzf3w4hoh1o`, `GkITmLwMSDjeTwN6`, `MJYf0b50mx4wXtdl`, `Nn8PFsdthS2IKAUi`, `X4F3usrzrUug9dw6`, `cT3MMWRcVaXIkMhu`, `dZXW1u8WtF9rCcGX`, `gHgaqS49pXTGPKUA`, `gklcdmy2g6XHihCT`, `hg9GlnmsDsO1tPX9`
- `o01` (1): `nRRv1dSvh9H2Smdd`
- `o10` (6): `3QaV3GPfXuVTXVdl`, `Z00AluW5X9YIH7MO`, `Z4yRrk1zPnLmlYzD`, `gS2SkOwfTkWWcCYX`, `gUGuVXXtTKPXIOVH`, `zQstjilmm0sIguBf`
- `o15` (2): `LSNsUz1ZztltknzU`, `xocvis4HiEP5IbtC`
- `o23` (1): `PtNdKcbmywLb9dXQ`
- `o33` (1): `KiLRvow6fWplbsV8`
- `o50` (1): `ZOhOb9d4CM8GIkD0`
- `o80` (1): `cwkEVTIJ8xudVoXM`
- `o90` (1): `BGqMcfJ3zcEh77lH`
- `o000` (2): `izVQdfNptnfPO2UD`, `octWoCHdMUIiHrHh`
- `o100` (1): `rT0hWBYtmopYe4AL`
- `o10987654` (1): `EGrgeAl4On3Rxv2D`

</details>

### Known-broken sketches: p5.js dependency

hydra-rust has no JavaScript engine and doesn't load, execute, or manage
any external JS libraries. Some sketches instantiate
[p5.js](https://p5js.org/) directly (`p1 = new P5()` and similar) to draw
extra overlay graphics alongside the hydra visuals — this is a whole
separate creative-coding framework with no Rust equivalent here, so these
sketches can't work regardless of any hydra-rust conformance fixes.
Regenerate this list from a fresh `check_corpus_failures.json` (see above)
yourself:

```bash
python3 -c "
import json
data = json.load(open('check_corpus_failures.json'))
for e in data:
    if 'P5' in e['error'] or \"'p5'\" in e['error']:
        print(e['file'], '-', e['error'])
"
```

<details>
<summary>11 affected sketches (snapshot from this session)</summary>

`C5VZw1nFAp9CHIJp`, `Tgcly25jfyE8j63N`, `W7uusYNDDH2eHGzB`, `WGu8wEalERTIjqyz`, `YmrdxMAsWx7rNw8O`, `lDGDJnvXdpq57VBZ`, `q5Lye9Lf1oJHnXIy`, `qYAgIWCEtjZ47ZuR`, `uqWfXFRF9cBAfZCY`, `wnJi3Zs4i6Ym7Y3e`, `5GT1XJqVnxbPhaJj`

</details>

## How chains work

A hydra-rust script is one or more **chains**. A chain starts with exactly one **source**
function (`osc`, `noise`, `shape`, `src`, ...), followed by zero or more geometry/color/
blend/modulate operations, and ends (implicitly or via `.out()`) by writing to one of four
output buffers `o0`-`o3`:

```rust
osc(60, 0.1, 0)
  .kaleid(4)
  .color(1, 0.5, 0.3)
  .modulate(noise(3, 0.1), 0.02)
  .rotate(0, 0.1)
  .out()
```

Trailing parameters may be omitted and take their listed default. A parameter can be a plain
number, a *reactive value* (below), or an *array-literal pattern* (below) instead — all three
compile down to a GLSL expression the same way.

## Sources

Start a chain — no chain to call these on.

| Function | Parameters (defaults) | Description | Example |
|---|---|---|---|
| `osc` | `frequency=60, sync=0.1, offset=0` | RGB sine oscillator, offset per channel | `osc(60, 0.1, 0).out()` |
| `noise` | `scale=10, offset=0.1` | 3D simplex noise, animated via `offset * iTime` | `noise(10, 0.1).out()` |
| `voronoi` | `scale=5, speed=0.3, blending=0.3` | Animated voronoi cell pattern | `voronoi(5, 0.3, 0.3).out()` |
| `shape` | `sides=3, radius=0.3, smoothing=0.01` | Regular polygon / circle | `shape(6, 0.4, 0.01).out()` |
| `gradient` | `speed=0` | Diagonal gradient, blue channel animated | `gradient(0.2).out()` |
| `solid` | `r=0, g=0, b=0, a=1` | Flat color | `solid(1, 0, 0, 1).out()` |
| `rings` | `freq=8, speed=0.1` | Concentric animated rings | `rings(8, 0.1).out()` |
| `checker` | `cols=4, rows=4` | Checkerboard | `checker(8, 8).out()` |
| `src` | `idx` | Reads buffer/source `idx` as a chain-starting source — `idx` `0`-`3` reads output buffer `o0`-`o3`'s previous frame, `idx` `100`-`103` (i.e. `s0`-`s3`) reads an external source slot (camera/image/GIF/video, see the feature sections below) | `src(0).scale(1.2).out()` |
| `text` | `content: string, config?: map` | Rasterizes a string (bundled Hack font) into one shared text texture; last call wins. `config.fontSize` overrides `hydraText.fontSize`: `"auto"` (default), percent of auto-fit size, or pixels (number/numeric string, optional `px`) | `text("hydra-rust", {fontSize: "50%"}).out()` |
| `strokeText` / `fillStrokeText` / `strokeFillText` | `content: string, config?: map` | Hollow outlines / filled outlines / filled outlines. `config.lineWidth` overrides `hydraText.lineWidth`: pixels or percent of resolved font size (default `"2%"`). Fractional widths work; zero disables strokes. Approximate distance field, white fill/stroke; other styling remains ignored | `strokeText("hi", {fontSize: 100, lineWidth: 3}).out()` |

Text settings are static and measured in the shared 512x512 texture, not
window pixels. Global defaults apply to subsequent calls and reset between
evaluations; per-call overrides do not change them. Font sizes must resolve
to `(0, 4096]` pixels; oversized text clips. Invalid/non-static values log a
warning and use that property's built-in default. An invalid config object
warns and uses global settings. Arbitrary font selection remains deferred.

The following are ported natively from popular real-world `loadScript()`-loaded community
extensions (`loadScript` itself is a permanent no-op — see [Known gaps](#known-gaps-and-stub-functions)).
Unless noted otherwise, from `metagrowing/extra-shaders-for-hydra` (AGPL-3.0, Thomas Jourdan):

| Function | Parameters (defaults) | Description | Example |
|---|---|---|---|
| `spiral` | `a=1, b=5, thickness=0.1` | Archimedean-spiral band pattern | `spiral(1, 5, 0.1).out()` |
| `turb` | `scale=10, offset=0.1, octaves=3` | Fractional Brownian motion (turbulence) | `turb(10, 0.1, 3).out()` |
| `uturb` | `scale=10, offset=0.1, octaves=3` | `turb`, normalized to `[0, 1]` | `uturb(10, 0.1, 3).out()` |
| `unoise` | `scale=10, offset=0.1` | `noise`, normalized to `[0, 1]` | `unoise(10, 0.1).out()` |
| `whitenoise` | `size=10, dynamic=0` | Grayscale hash noise, blocky at `size` | `whitenoise(20, 0).out()` |
| `colornoise` | `size=10, dynamic=0` | `whitenoise`, independent per channel | `colornoise(20, 0).out()` |
| `warp` | `scalei=10, offset=0.1, octaves=2, octavesinner=3, scale=1` | Domain-warped turbulence | `warp(10, 0.1, 2, 3, 1).out()` |
| `cwarp` | `scalei=10, offset=0.1, octaves=2, octavesinner=3, scale=1, focus=0.5` | `warp` with radial focus falloff | `cwarp(10, 0.1, 2, 3, 1, 0.5).out()` |
| `ncontour` | `thresh=0.5, smooth=0.1, octaves=3, scale=5, speed=0.5, step=2` | Contour lines from layered noise | `ncontour(0.5, 0.1, 3, 5, 0.5, 2).out()` |
| `pulse` | `edge=0.5, width=0.05, epsilon=0.001` | Single vertical pulse band | `pulse(0.5, 0.05, 0.001).out()` |
| `pulsetrain` | `train=3, edge=0.5, width=0.05, epsilon=0.001` | Repeated pulse bands | `pulsetrain(3, 0.5, 0.05, 0.001).out()` |
| `hextile` | `tiles=10` | Hexagonal tiling | `hextile(10).out()` |
| `concentric` | `scale=100, centerX=0.5, centerY=0.5` | Concentric rings from a center point | `concentric(100, 0.5, 0.5).out()` |
| `brick` | `width=0.25, height=0.08, gap=0.01` | Brick/masonry pattern | `brick(0.25, 0.08, 0.01).out()` |
| `wave` | `time=0, frequ=10, loops=3, thick=0.025` | Layered horizontal sine wave | `wave(0, 10, 3, 0.025).out()` |
| `lissa` | `time=0, frequ=10, loops=3, thick=0.025` | Polar-coordinate Lissajous/harmonograph curve | `lissa(0, 10, 3, 0.025).out()` |

## Geometry

Transform the sampling coordinate (`st`) before the source samples it. Call on a chain.

| Function | Parameters (defaults) | Description | Example |
|---|---|---|---|
| `rotate` | `angle=10, speed=0` | Rotate around center, optionally spinning over time | `osc().rotate(45, 0.1).out()` |
| `scale` | `amount=1.5, xMult=1, yMult=1, offsetX=0.5, offsetY=0.5` | Scale around a pivot point | `osc().scale(1.5, 1, 1, 0.5, 0.5).out()` |
| `scroll` | `scrollX=0.5, scrollY=0.5, speedX=0, speedY=0` | Offset + animated scroll, wraps | `osc().scroll(0.1, 0.1, 0.1, 0).out()` |
| `scrollX` | `scroll=0.5, speed=0` | Single-axis (X) scroll | `osc().scrollX(0.2, 0.1).out()` |
| `scrollY` | `scroll=0.5, speed=0` | Single-axis (Y) scroll | `osc().scrollY(0.2, 0.1).out()` |
| `kaleid` | `sides=4` | Kaleidoscope mirror-fold | `osc().kaleid(8).out()` |
| `pixelate` | `pixelX=20, pixelY=20` | Snap to a coarse pixel grid | `osc().pixelate(20, 20).out()` |
| `repeat` | `repeatX=3, repeatY=3, offsetX=0, offsetY=0` | Tile with per-row/column offset | `shape().repeat(3, 3, 0, 0).out()` |
| `repeatX` | `reps=3, offset=0` | Single-axis (X) tiling | `shape().repeatX(3, 0).out()` |
| `repeatY` | `reps=3, offset=0` | Single-axis (Y) tiling | `shape().repeatY(3, 0).out()` |
| `polar` | *(none)* | Cartesian → polar coordinates | `osc().polar().out()` |
| `cart` | *(none)* | Polar → Cartesian coordinates | `osc().polar().cart().out()` |
| `fold` | `amount=1` | Mirror-fold coordinates | `osc().fold(1).out()` |

The following are ported from `geikha/hyper-hydra` (MIT):

| Function | Parameters (defaults) | Description | Example |
|---|---|---|---|
| `inversion` | *(none)* | Circle inversion (`st /= dot(st, st)`) | `osc().inversion().out()` |
| `mirrorX` | `pos=0, coverage=1` | Mirror-fold around `pos` (X axis) | `osc().mirrorX(0, 1).out()` |
| `mirrorY` | `pos=0, coverage=1` | Mirror-fold around `pos` (Y axis) | `osc().mirrorY(0, 1).out()` |
| `mirrorX2` | `pos=0, coverage=1` | `mirrorX` variant, unflipped half | `osc().mirrorX2(0, 1).out()` |
| `mirrorY2` | `pos=0, coverage=1` | `mirrorY` variant, unflipped half | `osc().mirrorY2(0, 1).out()` |
| `mirrorWrap` | *(none)* | Fold coordinates into `[-1, 1]` then reflect | `osc().mirrorWrap().out()` |

## Color

Recolor a chain's output. Call on a chain.

| Function | Parameters (defaults) | Description | Example |
|---|---|---|---|
| `color` | `r=1, g=1, b=1, a=1` | Multiply/recolor (sign-dependent blend with input) | `osc().color(1, 0.5, 0.2, 1).out()` |
| `invert` | `amount=1` | Invert toward `1 - color` | `osc().invert(1).out()` |
| `contrast` | `amount=1.6` | Push away from/toward mid-gray | `osc().contrast(2).out()` |
| `brightness` | `amount=0.4` | Additive brightness | `osc().brightness(0.2).out()` |
| `saturate` | `amount=2` | Scale distance from luminance | `osc().saturate(3).out()` |
| `hue` | `amount=0.4` | Rotate hue (via HSV round-trip) | `osc().hue(0.5).out()` |
| `posterize` | `bins=3, gamma=0.6` | Quantize levels | `osc().posterize(4, 0.6).out()` |
| `luma` | `threshold=0.5, tolerance=0.1` | Luminance-keyed alpha | `osc().luma(0.5, 0.1).out()` |
| `colorama` | `amount=0.005` | Cycle hue, wrap channels | `osc().colorama(0.01).out()` |
| `shift` | `r=0.5, g=0, b=0, a=0` | Per-channel fractional shift | `osc().shift(0.5, 0.1, 0, 0).out()` |
| `thresh` | `threshold=0.5, tolerance=0.04` | Hard luminance threshold | `osc().thresh(0.5, 0.04).out()` |
| `r` | `scale=1, offset=0` | Isolate + scale the red channel (broadcast to all 4 output channels) | `osc().r(1, 0).out()` |
| `g` | `scale=1, offset=0` | Isolate + scale the green channel (broadcast) | `osc().g(1, 0).out()` |
| `b` | `scale=1, offset=0` | Isolate + scale the blue channel (broadcast) | `osc().b(1, 0).out()` |
| `a` | `scale=1, offset=0` | Isolate + scale the alpha channel (broadcast) | `osc().a(1, 0).out()` |
| `sum` | `scaleR=1, scaleG=1, scaleB=1, scaleA=1` | Weighted grayscale sum of channels, alpha preserved | `osc().sum(1, 1, 1, 1).out()` |

## Blend

Combine a chain with another chain. `other` may be another source/chain, a buffer/source
constant (`o0`-`o3`/`s0`-`s3`, auto-converted to a `src(...)` read), or a plain number
(auto-converted to a flat `solid(v, v, v, 1)` color).

| Function | Parameters (defaults) | Description | Example |
|---|---|---|---|
| `add` | `other, amount=1` | Additive blend | `osc(30, 0.1, 0).add(shape(4, 0.5, 0.1), 0.5).out()` |
| `mult` | `other, amount=1` | Multiplicative blend | `osc().mult(noise(), 1).out()` |
| `blend` | `other, amount=0.5` | Linear cross-fade | `osc().blend(shape(), 0.5).out()` |
| `diff` | `other` | Absolute difference | `osc().diff(shape()).out()` |
| `layer` | `other` | Alpha-composite `other` over the chain | `solid(0, 0, 0, 1).layer(shape(3, 0.4, 0.1)).out()` |
| `mask` | `other` | Use `other`'s luminance as an alpha mask | `osc().mask(shape(4, 0.4, 0.1)).out()` |
| `sub` | `other, amount=1` | Subtractive blend | `osc().sub(noise(), 0.5).out()` |
| `colreflect` | `other, amount=1` | Cross product of the two chains' RGB (ported from `metagrowing/extra-shaders-for-hydra`, AGPL-3.0) | `osc().colreflect(noise(), 1).out()` |

## Modulate

Use another chain's color output to distort a chain's coordinates. Same `other` rules as Blend.

| Function | Parameters (defaults) | Description | Example |
|---|---|---|---|
| `modulate` | `other, amount=0.1` | Offset `st` by `other`'s RG channels | `osc(60, 0.1, 0).modulate(noise(3, 0.1), 0.02).out()` |
| `modulateScale` | `other, multiple=1, offset=1` | Scale by `other`'s RG channels | `osc().modulateScale(noise(), 1, 1).out()` |
| `modulateRotate` | `other, multiple=1, offset=0` | Rotate by `other`'s red channel | `osc().modulateRotate(noise(), 1, 0).out()` |
| `modulateRepeat` | `other, repeatX=3, repeatY=3, offsetX=0.5, offsetY=0.5` | Tile with `other`-driven offset | `osc().modulateRepeat(noise(), 3, 3, 0.5, 0.5).out()` |
| `modulateRepeatX` | `other, reps=3, offset=0.5` | Single-axis (X) version | `osc().modulateRepeatX(noise(), 3, 0.5).out()` |
| `modulateRepeatY` | `other, reps=3, offset=0.5` | Single-axis (Y) version | `osc().modulateRepeatY(noise(), 3, 0.5).out()` |
| `modulateKaleid` | `other, sides=4` | Kaleidoscope with `other`-driven radius | `osc().modulateKaleid(noise(), 4).out()` |
| `modulateScrollX` | `other, scroll=0.5, speed=0` | Scroll (X) driven by `other`'s red channel | `osc().modulateScrollX(noise(), 0.5, 0).out()` |
| `modulateScrollY` | `other, scroll=0.5, speed=0` | Scroll (Y) driven by `other`'s red channel | `osc().modulateScrollY(noise(), 0.5, 0).out()` |
| `modulatePixelate` | `other, multiple=10, offset=3` | Pixelation grid driven by `other` | `osc().modulatePixelate(noise(), 10, 3).out()` |
| `modulateHue` | `other, amount=1` | Hue-shift-like distortion from `other`'s G/B difference | `osc().modulateHue(noise(), 1).out()` |

## Other core functions

| Function | Parameters | Description | Example |
|---|---|---|---|
| `out` | *(none)* or `bufIdx` | Writes the chain to buffer `o0`, or `bufIdx` (`o0`-`o3`) if given | `osc().out(); shape().out(o1)` |
| `render` | *(none)* or `bufIdx` | Sets the display mode: all 4 buffers in a 2x2 grid, or just buffer `bufIdx` | `osc().out(); render()` |
| `hush` | *(none)* | Clears all four buffers and resets `o0` to solid black | `hush()` |
| `screencap` | *(none)* | Requests one desktop PNG of the next rendered frame, without UI, using the screenshot-shortcut filename. Calls coalesce per evaluation; failed evaluation/shader compilation does not queue a capture. Plugin logs a warning; embedding hosts can handle `EvalResult.screencap` | `osc().out(); screencap()` |
| `random` | *(none)*, or 0-2 ignored extra args | A one-shot pseudo-random `f64` in `[0, 1)`, computed once at script-eval time (not per-frame) — the target of `Math.random()` | `osc(random() * 100, 0.1, 0).out()` |
| `setResolution` | `w, h` (must be static numbers, clamped to `1..=4096`) | Overrides the render buffers' own resolution, independent of the window size — the display still stretches to fill the window (the classic low-res/pixelation trick). A reactive argument (e.g. `window.innerWidth`) is treated as "no override" instead, since there's no per-frame callback to re-evaluate it against. Sticky — persists across evaluations that don't call it again | `setResolution(320, 240)` |
| `o0`-`o3`.`setNearest()` | *(none)* | Sets that buffer's texture sampling to nearest-neighbor (blocky) instead of linear. Sticky, like `setResolution` | `o0.setNearest()` |
| `o0`-`o3`.`setLinear()` | *(none)* | Sets that buffer's texture sampling back to linear (the default) | `o0.setLinear()` |
| `o0`-`o3`.`setMode(name)` | `name: "nearest"` or `"linear"` | Same as `setNearest`/`setLinear`, chosen by string. An unrecognized name is logged and ignored | `o0.setMode("nearest")` |

## Non compatible Hydra functions

These are hydra-rust extensions, not standard browser Hydra functions. Scripts
using them need modification or an equivalent extension to run in hydra.js.
This section documents the seamless-tiling extension; it is not an exhaustive
list of differences. For unsupported browser functions, see
[Known gaps and stub functions](#known-gaps-and-stub-functions).

### `.tileable(width=0.15)`

Makes the **complete preceding chain** repeat seamlessly in X and Y by blending
opposite edges, including corners. Works with procedural sources, feedback,
external textures, and chains containing blends or modulation. Available in
the library, standalone application, and plugin without additional features.

```rust
noise(5)
  .color(1, 0.5, 0.2)
  .tileable(0.15)
  .out()
```

`width` controls the overlap in normalized source coordinates and is clamped
to `[0.001, 0.5]`, including for reactive values and patterns. Larger values
produce wider transitions and more noticeable blending. The source coordinates
are slightly rescaled and inset to overlap opposite edges; this changes the
framing, and recognizable features may ghost in the transition regions.
RGB and alpha are blended together.

Use `.tileable()` with no argument for the default width. Place it after all
distortions and compositing: operations applied afterward can break tiling.
To preview a grid of the resulting tile:

```rust
noise(5).color(1, 0.5, 0.2).tileable(0.15).repeat(3, 3).out()
```

The operation evaluates the preceding graph four times per pixel, so complex
graphs cost more and nested `.tileable()` calls multiply that cost. It makes
each animation frame spatially tileable, not temporally looping. It is not a
periodic-noise generator and does not preserve the original image unchanged.

## Reactive values

Bare identifiers, not function calls — referencing one splices a live GLSL expression into
the compiled shader. Usable anywhere a plain number is expected.

| Identifier | GLSL expression | Notes |
|---|---|---|
| `time` | `iTime` | The only *reassignable* one (`time = 0`) — rebinds the local script variable for the rest of this evaluation only; the live `iTime` uniform is unaffected |
| `beat` | `iBeat` | `time * (tempo / 60)`, computed by the host |
| `tempo` | `iTempo` | Host-controlled BPM |
| `phase` | `iPhase` | Host-controlled |
| `mouseX`, `mouseY` | `iMouse.x`, `iMouse.y` | Normalized `[0, 1]` |
| `mouse.x`, `mouse.y` | `iMouse.x`, `iMouse.y` | Same values, object-property form |
| `width`, `height` | `iResolution.x`, `iResolution.y` | Canvas resolution in pixels |
| `innerWidth`, `innerHeight` / `window.innerWidth`, `window.innerHeight` | `iResolution.x`, `iResolution.y` | Same values, DOM-style form |
| `a.fft[i]` | `iFft[i]` | **`fft` feature** (included by `audio`) — see [below](#feature-audio) |

Example:

```rust
osc(60, 0.1, time).rotate(0, 0.1).scale(1.5, 1, 1, mouse.x, mouse.y).out()
```

## Math functions

`sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `abs`, `fract`, `floor`, `ceil`, `sqrt`, `sign`,
`exp`, `log` — plus two-argument `pow`, `min`, `max`, and a two-argument `atan(y, x)`
(`Math.atan2`-style). Each works on a reactive value (producing a live GLSL expression) or on
a plain number (evaluated immediately in Rust, baked into the shader as a literal).

```rust
osc(60, 0.1, sin(time) * 0.5 + 1).out()
```

## Patterns

An array literal (`[1, 2, 3]`) can be used almost anywhere a plain number is expected. It
compiles to a step function that cycles through the values once per beat, one value per
`1.0`-wide step.

| Modifier | Effect |
|---|---|
| `.fast(speed)` | Multiplies the cycle rate (default `1`) |
| `.smooth()` / `.smooth(amount)` | Linearly interpolates between the current and next value over an `amount`-beat-wide window (default `1`) |
| `.offset(amount)` | Shifts the pattern's phase (default `0.5`) |
| `.ease(name='linear')` | Named interpolation: `linear`, `sin`, or `easeIn`/`easeOut`/`easeInOut` with `Quad`, `Cubic`, `Quart`, or `Quint`. Resets smoothing to `1`; unknown names log a warning and leave the pattern unchanged |
| `.fit(lo, hi)` | Remaps the array's own `[min, max]` into `[lo, hi]` (default `0, 1`); resets `.offset()` but keeps `.fast()`/`.smooth()` |
| `.reverse()` | Returns the reversed array for chaining, matching JS's `Array.prototype.reverse()` (non-mutating) |

```rust
osc(60, 0.1, [0, 1, 2].fast(2).smooth()).out()
shape([3, 4, 5].fit(2, 6).offset(0.25)).out()
```

## Feature: `webcam`

```bash
cargo run --features webcam --bin hydra
```

| Function | Description | Example |
|---|---|---|
| `initCam(slot)` | Starts the default camera (index 0), populating source slot `s0`-`s3` | `s0.initCam().out()` |
| `initCam(slot, cameraIndex)` | Starts a specific camera by index | `s0.initCam(1).out()` |

`slot` must be a source (`s0`-`s3`); calling `initCam()` on an output
(`o0`-`o3`, e.g. `o0.initCam()`) is a common typo and now raises an eval
error instead of silently doing nothing.

Captured frames are flipped vertically before upload on every platform, to
correct for `nokhwa`'s decoded row order (top-down on both the AVFoundation/
macOS and V4L2/Linux backends we've tested) versus the source-sampling
shader's Y-flip (`1.0 - st.y`, eval.rs), which expects bottom-up rows.

## Feature: `audio`

```bash
cargo run --features audio --bin hydra
```

The microphone only opens once a script actually uses one of these (a setter, or reading
`a.fft[i]`) — never just because the feature is compiled in.

| Function | Description | Example |
|---|---|---|
| `a.fft[i]` | A frequency bin's magnitude (roughly `0`-`1`, shaped by the setters below); usable as any numeric argument | `osc(60, 0.1, a.fft[0]).out()` |
| `a.setBins(n)` | How many bins to reduce the spectrum into (default `4`) | `a.setBins(8)` |
| `a.setCutoff(c)` | Zeroes out bin values below this noise-floor threshold (default `0`) | `a.setCutoff(0.15)` |
| `a.setScale(s)` | Multiplies every bin's value (default `1`) | `a.setScale(2)` |
| `a.setSmooth(s)` | Exponential smoothing between frames, `0`-`1` (default `0.4`) | `a.setSmooth(0.8)` |
| `a.settings[i].cutoff` / `.scale` / `.smooth` | Reads or sets one active bin's static numeric setting | `a.settings[0].scale = 3` |
| `a.show()` / `a.hide()` | Shows/hides a small on-screen bar-graph overlay of the current FFT bins | `a.show()` |

`a.settings.length` returns the active bin count (maximum 8). Global
setters update every bin; `setBins` resets settings to current global
defaults and clears smoothing history. The `fft` feature alone registers
this API for host-fed audio; only `audio` enables microphone capture.
The desktop and plugin evaluate against their current settings so reads
remain correct between evaluations. Embedding hosts can do the same via
`eval_with_audio_settings(code, analyzer.settings())`.

Cutoff, gain, and FFT normalization retain hydra-rust's behavior rather than
Hydra's Meyda loudness units. For transient and beat signals, see
[Audio onset and beat reactivity](#audio-onset-and-beat-reactivity).

## Feature: `image_url`

```bash
cargo run --features image_url --bin hydra
```

| Function | Description | Example |
|---|---|---|
| `initImage(slot, url)` | Fetches and decodes a still image (PNG/JPEG/GIF/WebP) in the background, uploading it to a source slot once ready. Since it never changes, it's uploaded exactly once | `s0.initImage("https://example.com/pic.png").out()` |
| `initGif(slot, url)` | Fetches an animated GIF, decodes every frame up front, and cycles through them by elapsed real time once loaded — looping indefinitely, like a real `<img>` GIF | `s0.initGif("https://example.com/anim.gif").out()` |

Without this feature, both are no-ops (logged once) that still return the slot's source node
for chaining, so a script that calls them keeps working even in a build without `image_url`.

## Feature: `midi`

```bash
cargo run --features midi --bin hydra
```

A native port of the [hydra-midi](https://github.com/arnoson/hydra-midi) community
extension's API — real hydra.js has no MIDI support in its own core. Note names use standard
scientific-pitch-notation/General-MIDI numbering (`"C4"` is 60, middle C).
Channels are zero-based (`0`–`15`), inputs accept a zero-based index or an
exact port name, and `'*'` selects any channel/input (the default).
Notes and CC indices also support `'*'` to read the latest matching event.

| Function | Description | Example |
|---|---|---|
| `note(nameOrNumber[, channel[, input]])` | A chainable gate: `1` while the note is held, `0` otherwise (not velocity) | `osc().invert(note(60, 0, 'Keys')).out()` |
| `note(...).velocity()` | The note's last velocity, `0`-`1` (`0` once released) | `osc(60, 0.1, note("C4").velocity()).out()` |
| `note(...).adsr(a, d, s, r)` | An ADSR envelope (`a`/`d`/`r` in milliseconds, `s` a `0`-`1` sustain level), keyed to that note's on/off events, multiplied by the velocity captured at trigger time | `solid(1, 0, 0, note(60).adsr(50, 100, 0.7, 300)).out()` |
| `cc(index[, channel[, input]])` | A chainable, raw CC value normalized to `0`-`1` | `osc().rotate(cc(1)).out()` |
| `cc(...).smooth(factor=0.01)` | Exponential slew (temporal smoothing) of the CC value | `osc(60, 0.1, cc(1).smooth(0.2)).out()` |
| `aft()` | Channel-wide aftertouch (MIDI channel pressure), normalized to `0`-`1` | `osc().brightness(aft()).out()` |
| `aft(nameOrNumber[, channel])` | Per-note polyphonic aftertouch (key pressure) for that note, normalized to `0`-`1` — passing a note is what distinguishes this from channel-wide `aft()`, matching real hydra-midi | `osc(60, 0.1, aft(60)).out()` |
| `.range(lo, hi)` (on `note`/`cc`/`aft`/`.velocity()`) | Linearly remaps a `0`-`1` value into `[lo, hi]` | `osc().rotate(cc(1).range(0, 6.28)).out()` |
| `.scale(factor)` (on `note`/`cc`/`aft`/`.velocity()`) | Multiplies a value | `osc(1, 1, note(60).scale(0.5)).out()` |
| `.value(v => expression)` | Compiles a pure inline numeric transform into live GLSL, including captured constants and shader math | `osc(cc(1).value(v => v * 20 + time)).out()` |
| `_note(nameOrNumber[, channel])` | Plain (non-chainable) equivalent of `note(...)`, for use inside a `()=>` wrapper | `osc(1, 1, _note(60) * 0.5).out()` |
| `_cc(index[, channel])` | Plain (non-chainable) equivalent of `cc(...)` | `osc(1, 1, _cc(1) * 6.28).out()` |
| `_noteVelocity(nameOrNumber[, channel])` | Plain (non-chainable) equivalent of `note(...).velocity()` | `osc(1, 1, _noteVelocity(60)).out()` |
| `_aft([nameOrNumber[, channel]])` | Plain (non-chainable) equivalent of `aft(...)` | `osc(1, 1, _aft(60) * 0.5).out()` |
| `midi.start()` | Connects to every available MIDI input device — required before anything above reacts to input. Returns `midi` again, so `midi.start().show()` still parses | `midi.start()` |
| `midi.pause()` | Disconnects from all MIDI input devices | `midi.pause()` |
| `midi.show()` / `.hide()` | Shows/hides an on-screen overlay listing currently-held notes (with velocity) and non-zero CC values — a "current state" snapshot rather than real hydra-midi's own scrolling raw-message log | `midi.show()` |
| `midi.channel(n)` / `.input(n)` | Returns a reusable selector without changing global defaults; explicit function arguments override it | `midi.input('Keys').channel(0).cc(74)` |
| `midi.start({channel, input})` | Sets defaults for values created in this evaluation and starts input; other options are rejected | `midi.start({channel: 0, input: 'Keys'})` |

Selectors also apply to aftertouch, `_note`/`_cc`/`_noteVelocity`/`_aft`,
smoothing, and ADSR triggers. There are 64 filtered/wildcard value slots
and 16 envelope slots per evaluated script; excess allocations fail
explicitly. Plugins preserve MIDI channels but expose only one routed
input, index `0`, named `"host"`, not the original external devices.

`.value()` is symbolic, not a per-frame CPU callback. It accepts pure
single-parameter expression callbacks, not blocks, assignments, runtime
branching, arbitrary function calls, or side effects. Put CC smoothing
before `.value()`; smoothing an arbitrary transformed result is unsupported.

Filtered value and envelope slots are assigned per evaluation. Reevaluate
all affected output buffers together, rather than leaving older shaders
running with slot IDs that a partial evaluation could reuse.
Run with `RUST_LOG=info` to see connected device names; port indices follow
the system's enumeration order.

`.value()` accepts captured constants, `time`, and numeric math functions
(`sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `abs`, `fract`, `floor`,
`ceil`, `sqrt`, `sign`, `exp`, `log`, `pow`, `min`, `max`, and supported
`Math.*` aliases). It also works after `.smooth()`, `.adsr()`, `.range()`,
or `.scale()`.

## Feature: `osc`

```bash
cargo run --features osc --bin hydra
```

A native UDP OSC listener, modeled on the [hydra-osc](https://github.com/ojack/hydra-osc)
example (a browser/WebSocket bridge to a local UDP `osc-js` server) and atom-hydra's more
complete [`osc-loader.js`](https://github.com/hydra-synth/atom-hydra/blob/master/lib/osc-loader.js)
(a Node child-process UDP bridge, default port `57101` — matched here). Exposed as `_osc`
rather than `osc`: real hydra.js's `osc(freq, sync, offset)` is already the sine-oscillator
source function, so a bare `osc` identifier would collide with it.

Real hydra-osc/atom-hydra expose an `EventEmitter`-style `_osc.on(address, callback)`, running
an arbitrary per-message JS callback. hydra-rust scripts compile to one static GLSL expression
per chain rather than running arbitrary per-message callbacks. Instead,
`_osc.get(address[, argIndex])` returns a reactive value read directly into a GLSL uniform,
the same value-not-callback shape `note()`/`cc()` already use for MIDI. Each distinct
`_osc.get(...)` call site claims one slot from a small fixed pool (32 total), assigned
round-robin at eval time — unlike MIDI's 0-127 note/CC numbers, OSC addresses are arbitrary
strings with no natural small index to key a uniform array on directly. OSC bundles are
applied message-by-message, in order; their own timetags aren't specially honored.

| Function | Description | Example |
|---|---|---|
| `_osc.get(address)` | The most recent first argument (index `0`) received at that OSC address, or `0` if none has arrived yet | `osc(60, 0.1, _osc.get("/hue")).out()` |
| `_osc.get(address, argIndex)` | The most recent `argIndex`'th argument (0-based) received at that address | `osc().rotate(_osc.get("/xyz", 1)).out()` |
| `.range(lo, hi)` (on `_osc.get(...)`) | Linearly remaps a value into `[lo, hi]`, same generic transform MIDI's `note`/`cc`/`aft` chains use | `osc().rotate(_osc.get("/x").range(0, 6.28)).out()` |
| `.scale(factor)` (on `_osc.get(...)`) | Multiplies a value | `osc(1, 1, _osc.get("/x").scale(0.5)).out()` |
| `_osc.start()` | Binds a UDP socket on the default port (`57101`) and starts listening — required before `.get()` reacts to anything. Returns `_osc` again, so `_osc.start().show()` still parses | `_osc.start()` |
| `_osc.start(port)` | Same, on an explicit port | `_osc.start(9000)` |
| `_osc.pause()` | Stops listening and releases the UDP socket | `_osc.pause()` |
| `_osc.show()` / `.hide()` | Shows/hides an on-screen overlay listing every OSC address received so far and its latest arguments — a "current state" snapshot, the same treatment `midi.show()` gives its own monitor | `_osc.show()` |

## Feature: `video`

```bash
cargo run --features video --bin hydra
```

Requires a standalone `ffmpeg` binary on `PATH` at runtime (`brew install ffmpeg`,
`apt install ffmpeg`, ...) — it's spawned as a subprocess via
[ffmpeg-sidecar](https://github.com/nathanbabcock/ffmpeg-sidecar), never linked into this
binary, so `cargo build` itself never needs FFmpeg's dev libraries. If `ffmpeg` isn't found,
`initVideo` logs one warning and leaves the slot empty rather than failing to build or run.

| Function | Description | Example |
|---|---|---|
| `initVideo(slot, url)` | Streams frames from a local video file or a remote URL, looping indefinitely, uploading each new frame to a source slot as it's decoded | `s0.initVideo("/path/to/clip.mp4").out()` |

Without this feature, `initVideo` is a no-op (logged once) that still returns the slot's
source node for chaining.

## Feature: `stream`

```bash
cargo run --features stream --bin hydra
```

Streams video between **hydra-rust instances** over WebRTC — not real hydra.js's own
`initStream`/`pb.setName()`, which is itself currently broken in the live hydra.js editor (its
signaling server hasn't been touched since 2024) and uses a bespoke, undocumented protocol with
nothing to interoperate with. `initStream` receives; `broadcastStream` sends this sketch's own
rendered output (whatever `render()` currently displays) to the next thing that connects.

Connects directly by IP:port — a single TCP connection carries one SDP offer/answer exchange, no
relay server, no STUN/TURN. This means no NAT traversal: both instances need to be directly
reachable from each other, which typically means the same LAN or the same machine. Requires a
standalone `ffmpeg` binary on `PATH` at runtime, same as `video` — it does the actual VP8
encode/decode in both directions (`webrtc-rs`, the WebRTC crate used here, only handles
transport/RTP relaying, never codec work itself). A broadcast frame wider than 1280px is
downscaled before encoding — found necessary via real end-to-end testing: a Retina display's
actual framebuffer resolution is far more pixels than realtime software VP8 encoding can keep up
with otherwise.

| Function | Description | Example |
|---|---|---|
| `s0.initStream("host:port")` | Connects to a broadcaster listening at that address and streams its video into a source slot | `s0.initStream("192.168.1.20:9000").out()` |
| `broadcastStream(port)` | Broadcasts this sketch's own rendered output to any number of `initStream` connections on `port`, all sharing a single `ffmpeg` encode; a no-op if already broadcasting | `broadcastStream(9000)` |
| `stopBroadcast()` | Stops broadcasting, disconnecting every connected viewer | `stopBroadcast()` |

The simplest way to try it is two runnable `.hydra` scripts talking to each other directly (edit
the address in the receiving one first):
```bash
cargo run --features stream --bin hydra -- examples/stream_broadcast.hydra  # broadcasts on :9000
cargo run --features stream --bin hydra -- examples/stream_basic.hydra      # bare initStream().out()
cargo run --features stream --bin hydra -- examples/stream_vj.hydra         # kaleid/modulate/layer on top
```

Two companion CLI examples are also available for testing/debugging without a full GUI app — a
"server" that broadcasts a webcam/file/test pattern (not a live sketch), and a "client" that
receives and reports frame stats headlessly:
```bash
cargo run --features stream --example webrtc_broadcast -- 9000
cargo run --features stream --example webrtc_receive -- <broadcaster-ip>:9000 --save frame.ppm
```

Without this feature, `initStream` is a no-op (logged once) that still returns the slot's source
node for chaining; `broadcastStream`/`stopBroadcast` aren't real hydra.js functions, so without
`stream` they're simply not registered at all (a plain "Function not found").

To broadcast a webcam, file, or test pattern with the companion CLI:

```bash
cargo run --features stream --example webrtc_broadcast -- 9000
# macOS webcam: append --format avfoundation --input 0
# Linux webcam: append --format v4l2 --input /dev/video0
# Video file: append --input clip.mp4
```

## Known gaps and stub functions

Some accepted functions do nothing or only partially implement browser
Hydra/community-extension behavior. See the
[stub-function table](#stub-functions-accepted-but-not-yet-implemented)
for the status and rationale of `initScreen`, `P5(...)`, `Scene(...)`,
`loadScript`, browser DOM helpers, and feature-disabled source initializers.
SPEC.md §9 describes these gaps in the language-spec context.

`setFunction(...)` is **not** a stub: it registers dynamic custom GLSL
functions. See [Custom GLSL functions](#custom-glsl-functions-setfunction)
and SPEC.md §12.
