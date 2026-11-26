# hydra-rust documentation

Installation, standalone workflows, embedding guidance, compatibility notes,
and an example-driven function reference for hydra-rust. For a short project
overview and quick start, see [README.md](README.md).

[SPEC.md](SPEC.md) describes the language, JavaScript-compatibility
preprocessing, reactive semantics, and shader-chain model. The
[plugin guide](crates/hydra-plugin/README.md) covers VST3/CLAP builds and
the plugin's loader-only interface.

The scripting reference describes the default Rhai backend and optional Cargo features.
See [JavaScript scripting](#javascript-scripting) for the separate interpreter backend.
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
- [JavaScript scripting](#javascript-scripting)
- [Example](#example)
- [Used in Sova](#used-in-sova)
- [Differences from browser Hydra](#differences-from-browser-hydra)
- [Custom GLSL functions (`setFunction`)](#custom-glsl-functions-setfunction)
- [Screenshots](#screenshots)
- [Text rendering](#text-rendering)
- [Pattern easing](#pattern-easing)
- [Runtime and resource limits](#runtime-and-resource-limits)
- [Performance measurements](#performance-measurements)
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

`.shr` ("slot hydra rust") stores one sketch; `.bhr` stores a bank of 16 scenes.
Version 2 exports include each scene's scripting language.
For example, a slot can contain `{"version":2,"code":"osc(10).out()","language":"javascript"}`.
Recalling a scene selects its stored language before evaluation.
Legacy session scenes inherit the session's saved language.
Legacy imported files default to Rhai unless a command-line JavaScript selection overrides their missing language.
Explicit language metadata takes precedence over that command-line selection.
For example, a tagged Rhai slot remains Rhai when loaded with `--javascript`.
Current builds read both export versions; older builds cannot read version 2 exports.
The `-ss` option writes immediately at launch.
The `-bs` option instead sets the destination for later bank exports.

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

## JavaScript scripting

The optional `javascript` feature embeds QuickJS alongside the existing Rhai backend.
The application still uses its native window and OpenGL renderer.
Browser APIs, external JavaScript runtimes, and embedded browser engines are not included.
The plugin continues using Rhai.

### Starting JavaScript mode

- Build with `javascript` and select the backend with `--javascript`.
- Use `--library-root` to choose the directory containing trusted local libraries.
- Explicitly evaluate the opened sketch with **Ctrl/Cmd + Shift + Enter**.

```bash
cargo run --features javascript -- --javascript \
  --library-root examples/javascript examples/javascript/live.js
```

New sessions use the application's working directory unless `--library-root` specifies another directory.
Restored sessions reuse their saved library root.
The options sidebar also offers a JavaScript checkbox.
Evaluate a sketch after changing the checkbox to apply the selected language.
The session stores the selected language and library root.
JavaScript variables do not persist across application restarts.
Scene banks store source text and its scripting language.
Recalling a scene automatically selects its saved language.
Scenes also preserve whether the editor explicitly selected module evaluation.
For example, a saved `.mjs` scene keeps module scope after recall.
The built-in code formatter remains Rhai-only.

### Local libraries

`loadScript(path)` executes a local JavaScript file in the current interpreter.
Libraries can define functions, extend prototypes, and register shaders.
Loading is synchronous; network URLs and browser script loading are unsupported.
Use `loadScript()` for ordinary scripts and `import` for modules.
CommonJS `require()` remains unsupported.
Only files inside the configured library root are allowed.

```javascript
loadScript("libraries/helpers.js");
osc(20).tintCycle(0.5).out();
```

The example library extends Hydra chains and JavaScript arrays.
For example, `[0, 1, 2].scaled(0.5)` returns `[0, 0.5, 1]`.
See [helpers.js](examples/javascript/libraries/helpers.js) and [live.js](examples/javascript/live.js).

[Hyper-Hydra](https://github.com/geikha/hyper-hydra)'s arrays and fractals modules can load unchanged from local files.
This does not imply compatibility with every Hyper-Hydra extension.

```javascript
loadScript("hydra-arrays.js");
loadScript("hydra-fractals.js");
osc([10, 20].add(5)).mirrorX().out();
```

### Local modules

JavaScript sketches can import local `.js` and `.mjs` modules.
Relative imports resolve beside the importing file.
Scenes without an associated file resolve entry-point imports from the library root.
Imported files must remain inside the configured library root, including after resolving symbolic links.
Package names, network URLs, and import attributes remain unsupported.
For example, `import "./libraries/pulse.mjs"` is local; `import "some-package"` is not supported.

```javascript
import { pulse } from "./libraries/pulse.mjs";
osc(20).brightness(() => pulse(time) * 0.2).out();
```

See [pulse.mjs](examples/javascript/libraries/pulse.mjs) for the exported helper.
Run the complete [module example](examples/javascript/module.mjs):

```bash
cargo run --features javascript -- --javascript \
  --library-root examples/javascript examples/javascript/module.mjs
```

Files ending in `.mjs` always use module evaluation.
The interpreter also recognizes module syntax in `.js` sketches.
The sidebar's **Evaluate as a module** checkbox can force module evaluation without a file extension.
For example, select it before saving a scene containing module-scoped `const` declarations.
Ordinary scripts retain their existing global-variable behavior.
Module variables stay inside their module.
For example, use `globalThis.counter = 0` to share a counter between modules.
Modules can await promises that complete within the bounded job queue.
For example, `const value = await Promise.resolve(1)` works in a module.
Unresolved promises report an error rather than blocking the application indefinitely.
Reset the interpreter before retrying a module with unfinished asynchronous work.

Imported modules remain cached until interpreter reset.
Reevaluating a sketch does not reread its cached dependencies.
For example, reset the interpreter after editing `pulse.mjs`, then evaluate the sketch again.

### Logging and library errors

`console.log`, `console.info`, `console.debug`, `console.warn`, and `console.error` use native application logging.
Library errors include their source paths and line numbers.
For example, an exception can identify `libraries/helpers.js:12`.

```javascript
console.log("Library loaded.");
```

Enable informational messages with the existing logging environment variable:

```bash
RUST_LOG=hydra_rust::javascript=info cargo run --features javascript -- --javascript
```

### Runtime settings and recovery

The options sidebar offers **JavaScript runtime limits** and **Reset JavaScript interpreter**.
Applied settings persist in ordinary sessions and session-manager projects.
Draft changes remain unapplied until confirmation.

| Setting | Default | Allowed range |
|---------|---------|---------------|
| JavaScript execution time | 200 milliseconds | 1 to 2,000 milliseconds |
| Heap memory | 32 mebibytes | 8 to 256 mebibytes |
| Stack memory | 512 kibibytes | 128 to 2,048 kibibytes |
| Sketch source size | 1,024 kibibytes | 1 to 4,096 kibibytes |
| Individual library size | 1,024 kibibytes | 1 to 4,096 kibibytes |
| Total loaded library text | 16 mebibytes | 1 to 64 mebibytes |
| Pending promise jobs per call | 256 | 1 to 4,096 |

One mebibyte equals 1,048,576 bytes; one kibibyte equals 1,024 bytes.
The sidebar abbreviates these units as MiB and KiB.
An individual library cannot exceed the total loaded-text limit.
Classic libraries and imported modules share the loaded-text budget.
A 256-kibibyte sketch limit also caps serialized graph data at one mebibyte.
Invalid settings report an error and preserve the existing interpreter.

- Open the options sidebar with **Tab**.
- Expand **JavaScript runtime limits** and adjust the desired values.
- Select **Apply limits and reset**, then confirm **Reset now**.
- Evaluate the sketch again.

For example, select a 50-millisecond execution limit without disabling memory or promise-job limits.
Reset clears JavaScript variables, loaded libraries, cached modules, and callback state.
It does not delete scene banks or execute the editor's code automatically.
Current shaders keep rendering with their last callback values until a successful evaluation replaces them.
For example, use **Reset JavaScript interpreter** after heap exhaustion, then evaluate a corrected sketch.

### Hyper-Hydra compatibility

This table describes local loading in the standalone application's JavaScript backend.
The plugin's Rhai backend does not execute these JavaScript files.
Source reviews and execution checks use revision [e48aacb](https://github.com/geikha/hyper-hydra/tree/e48aacbe775cd6731c50cc327ed86cd7e7beab6c).
Future upstream changes can affect compatibility.

**Verified examples** means representative scripts executed and produced valid shaders, not every exported function.
**Candidate** means the source appears suitable, but execution remains unverified.
**Partial candidate** means only helpers without missing browser dependencies appear suitable.
Other statuses describe known blockers from the source review.
Loading a file successfully does not prove that its functions work.

| Extension | Status | Evidence or blocker |
|-----------|--------|---------------------|
| `hydra-abbreviations.js` | Verified examples | `bright`, `rot`, and chained `o()` aliases execute and compile. |
| `hydra-arithmetics.js` | Verified examples | Scalar addition, trigonometric functions, ranges, and texture comparisons execute and compile. |
| `hydra-arrays.js` | Verified examples | Array extensions work, including arithmetic and arrays used as animated shader arguments. |
| `hydra-blend.js` | Verified examples with a known failure | `screen` compiles without unused shaders. `phoenix` still fails because its scalar assignments contain vectors. |
| `hydra-canvas.js` | Unsupported browser APIs | Changes browser canvas styles and parent-element layout. |
| `hydra-colorspaces.js` | Verified examples | Hue offsets, color-space multipliers, and `hWith` graph copies execute and compile. |
| `hydra-convolutions.js` | Verified examples | `sharpen` and `sharpenLuma` compile with native output and source samplers. |
| `hydra-debug.js` | Unsupported browser APIs | Creates browser interface elements and loads remote dependencies. |
| `hydra-fractals.js` | Verified examples | Mirror and inversion shaders work through dynamic registration. |
| `hydra-gif.js` | Unsupported browser APIs | Requires browser image decoding, canvases, timers, and source internals. |
| `hydra-glsl.js` | Verified examples | Source, coordinate, color, hue/saturation/value, combination, and modulation snippets execute and compile. |
| `hydra-gradientmap.js` | Unsupported browser APIs | Requires `sampler2D` inputs, browser canvases, and browser Hydra source objects. |
| `hydra-mouse.js` | Unsupported browser APIs | Attaches browser pointer listeners and reads canvas dimensions. Native `mouse` remains available. |
| `hydra-outputs.js` | Unsupported graphics internals | Directly manipulates `regl` textures, framebuffers, and browser Hydra output objects. |
| `hydra-pip.js` | Unsupported browser APIs | Requires canvas video capture and browser picture-in-picture support. |
| `hydra-pixels.js` | Unsupported graphics internals | Reads pixels through `regl` and browser Hydra framebuffer objects. |
| `hydra-src.js` | Partial candidate | Buffer masking helpers appear suitable. Dimension-aware helpers require browser canvas and source metadata. |
| `hydra-swizzle.js` | Verified examples | The complete registry loads. Chained `bgra` and `r` properties execute and compile. |
| `hydra-tap.js` | Unsupported browser APIs | Requires browser keyboard listeners and timers. |
| `hydra-text.js` | Unsupported browser APIs | Draws through browser canvases and creates browser Hydra source objects. |
| `hydra-vec4.js` | Verified examples | Nested `vec2`, `vec3`, and `vec4` helpers generate valid solid-color shaders. |
| `hydra-wrap.js` | Unsupported graphics internals | Recreates `regl` framebuffers and modifies browser Hydra output objects. |

`regl` is Hydra's browser graphics library.
The standalone provides a detection marker, not its graphics methods.
Native equivalents do not make the corresponding JavaScript extension compatible.
For example, native `o0.setNearest()` does not provide the framebuffer internals required by `hydra-outputs.js`.

Keep local copies inside the configured library root.
For example, `loadScript("hydra-fractals.js")` loads the local file without `await`.
Remote URLs remain unsupported.

#### Reproducing compatibility checks

The optional upstream checks require Git, a local checkout, and `glslangValidator`.
They do not download libraries during execution.
They verify the checkout revision and reject tracked source changes.
Each example executes the unchanged library and validates its complete generated fragment shader.
The blend check also confirms the known `phoenix` failure.

```bash
git clone https://github.com/geikha/hyper-hydra libraries/hyper-hydra
git -C libraries/hyper-hydra checkout e48aacbe775cd6731c50cc327ed86cd7e7beab6c
HYDRA_HYPER_HYDRA_ROOT="$PWD/libraries/hyper-hydra" \
  cargo test --no-default-features --features javascript \
  --test hyper_hydra -- --ignored
```

Native rendering checks also exercise actual output and source sampler bindings.
For example, a custom sampler reads an existing green output and an uploaded red source.

### Live JavaScript callbacks

JavaScript state remains available between evaluations within the same runtime.
Numeric function arguments execute each frame and feed shader uniforms.
They do not run once per pixel.
An optional `update` function executes each frame before parameter callbacks.
Define shader graphs during sketch evaluation, not inside frame callbacks or their promise jobs.

```javascript
var pulse = 0;
update = () => { pulse = 0.5 + 0.5 * Math.sin(time); };
osc(20).brightness(() => pulse * 0.2).out();
```

Use callbacks when parameters should follow changing JavaScript values.
For example, `.rotate(() => time * 0.1)` animates instead of capturing the evaluation-time value.
JavaScript supports genuine functions and closures; it does not use Rhai's syntax-rewriting pipeline.
Callbacks can read `time`, `beat`, `tempo`, `phase`, `mouse`, `width`, and `height`.
For example, `.brightness(() => mouse.x * 0.2)` follows the horizontal pointer position.
Mouse coordinates range from zero to one.
With the `audio` feature, callbacks can also read `a.fft`, `a.onset`, `a.beat`, and `a.bpm`.
For example, `.scale(() => 1 + a.fft[0])` follows the first audio frequency band.

### Native feature parity

JavaScript shares the native shader compiler, text renderer, and device managers.
Device actions and buffer clearing apply only after successful evaluation and shader compilation.
Create device signals while evaluating the sketch, not inside frame callbacks.
Missing Cargo features produce explicit errors.

#### Shader and text helpers

`.tileable(width)` uses the existing native edge-blending implementation.
The default width is `0.15`; callbacks and patterns also work.

```javascript
osc(10).tileable(() => 0.15).repeat(3, 3).out();
```

`text`, `strokeText`, `fillStrokeText`, and `strokeFillText` use the bundled native font.
Their configuration matches the [native text helpers](#text-rendering).
`hydraText` supplies global defaults; per-call settings override them.
All text nodes share one texture, so the last text call wins.

```javascript
hydraText.fontSize = "50%";
strokeText("Hydra", { lineWidth: "3px" }).out();
```

### Native rendering extensions

These opt-in extensions also support the plugin's Rhai renderer, except region controls and OSC output.
Ordinary Hydra parameters and previous-frame output sampling retain their existing behavior.
Extended sketches do not necessarily run in browser Hydra.

#### Example sketches

Each example isolates one extension.
The neighborhood examples keep the original image in `o0` and display the processed image from `o1`.
Change `render(o1)` to `render(o0)` to compare them.

| Extension | Rhai script | JavaScript script | Visible result |
|-----------|-------------|-------------------|----------------|
| Per-pixel parameters | [Rhai](examples/extensions/per_pixel_parameters.hydra) | [JavaScript](examples/javascript/extensions/per_pixel_parameters.js) | Stripe frequency varies across the image |
| Blur | [Rhai](examples/extensions/blur.hydra) | [JavaScript](examples/javascript/extensions/blur.js) | Softer stripe boundaries |
| Sharpen | [Rhai](examples/extensions/sharpen.hydra) | [JavaScript](examples/javascript/extensions/sharpen.js) | Stronger noise details |
| Sobel | [Rhai](examples/extensions/sobel.hydra) | [JavaScript](examples/javascript/extensions/sobel.js) | Bright polygon outlines |
| Emboss | [Rhai](examples/extensions/emboss.hydra) | [JavaScript](examples/javascript/extensions/emboss.js) | Noise resembles a shaded surface |
| Dilation | [Rhai](examples/extensions/dilate.hydra) | [JavaScript](examples/javascript/extensions/dilate.js) | White dots grow |
| Erosion | [Rhai](examples/extensions/erode.hydra) | [JavaScript](examples/javascript/extensions/erode.js) | White dots shrink |
| History | [Rhai](examples/extensions/history.hydra) | [JavaScript](examples/javascript/extensions/history.js) | A rotating triangle leaves a delayed echo |
| Region control | [Rhai](examples/extensions/region.hydra) | [JavaScript](examples/javascript/extensions/region.js) | Image intensity changes stripe frequency |
| Region OSC output | [Rhai](examples/extensions/region_osc.hydra) | [JavaScript](examples/javascript/extensions/region_osc.js) | Changing intensity also reaches an OSC receiver |

- Open a Rhai example in the standalone application or plugin.
- Use the standalone application for region examples.
- Select **JavaScript** when opening a `.js` example.
- Evaluate the sketch with **Ctrl/Cmd + Shift + Enter** in the standalone application.

```bash
cargo run --bin hydra -- examples/extensions/blur.hydra
cargo run --bin hydra --features javascript -- --javascript examples/javascript/extensions/blur.js
```

The OSC example requires the `osc` feature and a receiver listening on `127.0.0.1:9000`.
It sends one float at `/hydra/brightness`, at most 30 times per second.
Replace the destination and address to match your receiver.

```bash
cargo run --bin hydra --features osc -- examples/extensions/region_osc.hydra
```

Reset the JavaScript interpreter before switching examples to discard retained outputs and region bindings.
Reevaluate the new sketch after reset.
For example, reset before switching from `region_osc.js` to `blur.js`.

#### Per-pixel float parameters

A texture argument supplies its red channel at each pixel.
Texture arithmetic preserves values above one until rendering stores them.
JavaScript cannot overload multiplication, so use `.mul(number)` instead.

```javascript
osc(noise(2).mul(40)).out();
```

The equivalent Rhai expression uses multiplication.

```javascript
osc(noise(2) * 40).out();
```

#### Neighborhood effects

| Method | Operation | Example |
|--------|-----------|---------|
| `.blur(radius)` | Average nine nearby samples | `osc(10).blur(1).out();` |
| `.sharpen(radius)` | Subtract the local average from twice the center | `osc(10).sharpen(1).out();` |
| `.sobel(radius)` | Detect intensity edges | `osc(10).sobel(1).out();` |
| `.emboss(radius)` | Shade diagonal intensity changes | `osc(10).emboss(1).out();` |
| `.dilate(radius)` | Keep each channel's local maximum | `osc(10).dilate(1).out();` |
| `.erode(radius)` | Keep each channel's local minimum | `osc(10).erode(1).out();` |

Radius controls sample spacing in pixels, from zero through 32.
Its default is one pixel.
JavaScript requires a fixed number; Rhai also accepts shader expressions and patterns.
Each effect adds two intermediate passes.
Intermediate textures use eight-bit red, green, blue, and alpha channels, matching native output storage.
Values clamp to zero through one between passes.
The renderer limits each graph replacement to 32 passes.
Nested graphs also retain the existing 16-level limit.

JavaScript libraries can replace ordinary effect names.
Use native-prefixed aliases when a library supplies its own effect.

```javascript
osc(10).nativeSharpen(1).nativeBlur(2).out();
```

#### Output history

`history(output,delay)` reads a completed output from an earlier frame.
Choose a fixed delay from one through eight frames.
JavaScript defaults to one frame; Rhai requires both arguments.
`setHistoryLimit(limit)` lowers the per-output cap in either language.
The default cap is eight; values outside one through eight produce errors.
Delay one uses ordinary feedback textures.
Longer delays allocate a ring of textures only for requested outputs.

```javascript
history(o0,4).blend(osc(10),0.1).out(o0);
```

For example, choose a four-frame cap before declaring delayed feedback.

```javascript
setHistoryLimit(4);
history(o0,4).blend(osc(10),0.1).out();
```

New history starts transparent black.
Clearing an output also clears its history.
Resizing clears history and intermediate textures.
Successful recompilation preserves history when its depth remains unchanged.
Failed compilation preserves the previous graph and history.

Extended plans have a 256 MiB texture budget, including the eight ordinary feedback textures.
Device texture dimensions and available texture units impose additional limits.
Unsupported plans and resizing report errors rather than silently reducing history.
For example, eight 1920-by-1080 history textures require approximately 63 MiB for one output.
The ordinary feedback textures require another 63 MiB at that resolution.
Temporary shader replacement resources can increase peak memory usage.

#### Image-region controls and OSC output

Region controls run only in the standalone application.
`region(output,x,y,width,height,channel)` selects a normalized output rectangle.
The default rectangle covers the whole output.
Channels zero through three select red, green, blue, and alpha.
Channel four selects luminance and is the default.
Up to sixteen regions can coexist.
JavaScript returns a callback signal supporting the existing `.scale()` and `.range()` helpers.

```javascript
const brightness = region(o0,0,0,1,1,4);
osc(brightness.range(5,40)).out();
```

Rhai returns a shader expression.
Rhai requires all six arguments.

```javascript
let brightness = region(o0,0.0,0.0,1.0,1.0,4);
osc(brightness * 40.0).out();
```

Each region averages an eight-by-eight downsample, not every original pixel.
Values become available on the next frame.
Reading pixels can stall rendering, so keep region counts and output rates modest.
Regions must be declared during evaluation, not inside frame callbacks.

`sendRegion` sends an unmodified region signal through OSC when the `osc` Cargo feature is enabled.
Destinations require a numeric IP address and a nonzero port; hostnames are unsupported.
Each binding accepts one through 60 messages per second.
JavaScript defaults to 30; Rhai requires the rate argument.
No packet is sent before the first completed region sample.

```javascript
const level = region(o0);
sendRegion(level,"127.0.0.1:9000","/brightness",30);
```

JavaScript `stopRegionOutput()` removes all output bindings.
Rhai reevaluation replaces its region declarations and bindings.
For example, evaluate `osc(10).out()` in Rhai to stop region output.
JavaScript interpreter reset discards declarations; the displayed graph remains until successful replacement.
Reset also stops OSC region output.
GPU pixel sorting, waveform audio output, and direct control-voltage hardware output remain deferred.

### Native device controls

#### Source initialization

These methods return their source object and queue native source requests.
They do not create browser cameras, canvases, or media elements.

| Method | Required feature | Example |
|--------|------------------|---------|
| `initCam(index=0)` | `webcam` | `s0.initCam(0); src(s0).out();` |
| `initImage(url)` | `image_url` | `s0.initImage("https://example.com/picture.png"); src(s0).out();` |
| `initGif(url)` | `image_url` | `s0.initGif("https://example.com/animation.gif"); src(s0).out();` |
| `initVideo(location)` | `video` | `s0.initVideo("clip.mp4"); src(s0).out();` |
| `initStream(address)` | `stream` | `s0.initStream("127.0.0.1:9000"); src(s0).out();` |

Locations use the same rules as their native source managers.
Image and animated-image loading use network URLs; JavaScript library loading remains local-only.
For example, camera capture requires a build with `javascript,webcam`.
Browser-only `initScreen` and generic canvas initialization remain unsupported.

#### Audio controls

The `fft` feature exposes audio settings; `audio` additionally enables microphone capture.
Global setters include `setBins`, `setCutoff`, `setScale`, and `setSmooth`.
`a.bins[index]` exposes native `cutoff`, `scale`, and `smooth` properties.
The standalone application synchronizes these settings with its audio manager before evaluation.
`a.show()` and `a.hide()` control the native audio overlay.

```javascript
a.setBins(4).setSmooth(0.5);
a.bins[0].scale = 2;
osc(() => 10 + a.fft[0] * 20).out();
```

#### MIDI and OSC controls

Musical Instrument Digital Interface (MIDI) controls require the `midi` feature.
Open Sound Control (OSC) controls require the `osc` feature.
MIDI methods include `start`, `pause`, `show`, and `hide`.
These controls accept no arguments.
Value methods include `midi.note`, `midi.velocity`, `midi.cc`, and `midi.aft`.
They accept an index, optional zero-based channel, and optional input index or exact input name.
Use `"*"` or omit a selector to match any value.
Signals support `.range(min,max)` and `.scale(factor)`.
Controller signals additionally support `.smooth(factor=0.01)`.
Note and velocity signals support `.adsr(attack,decay,sustain,release)`.
Attack, decay, and release use milliseconds; sustain ranges from zero to one.
Numeric note indices work; note names, reusable selectors, and Rhai's global MIDI aliases remain unsupported.

```javascript
midi.start();
var level = midi.cc(74, 0, "Keys").smooth(0.1).range(0, 0.5);
osc(20).brightness(level).out();
```

`_osc.start(port=57101)`, `pause`, `show`, and `hide` control the native listener and overlay.
`_osc.get(address,argIndex=0)` returns a signal with `.range()` and `.scale()`.
Bindings remain stable across evaluations.
The interpreter allows 64 MIDI values, 16 MIDI envelopes, and 32 OSC values before reset.
At most 256 native requests can accompany an evaluation.

```javascript
_osc.start(57101);
var level = _osc.get("/level", 0).scale(0.5);
osc(20).brightness(level).out();
```

#### Buffer clearing

`o0.clear()` through `o3.clear()` clear both native feedback textures once.
Clearing preserves the buffer's shader and sampling settings.
The application applies clearing after any requested render-size change.
Existing shaders continue drawing on subsequent frames.
For example, use `solid(0,0,0,1).out(o0)` to keep an output black.
Browser framebuffer objects and direct `regl` access remain unsupported.

```javascript
o0.clear().setLinear();
src(o0).blend(osc(10), 0.1).out(o0);
```

### Dynamic shader functions

Libraries can call `setFunction()` at runtime to add chain operations and sources.
Hydra's `_st`, `_c0`, and `_c1` shader argument names are supported.
The native `st`, `c0`, and `c1` names also remain available.
Custom definitions can replace built-in shader functions.
Names beginning with underscores work.
The bridge maps function and input names to safe internal shader identifiers.
Snippets can declare local `st`, `c0`, and `c1` without duplicating native aliases.

```javascript
setFunction({
  name: "diagonal",
  type: "src",
  inputs: [{ name: "frequency", type: "float", default: 12 }],
  glsl: "return vec4(vec3(sin((_st.x + _st.y) * frequency)), 1.0);"
});
diagonal(20).out();
```

Only custom functions used by an output graph enter that output's shader.
Unused invalid shader bodies no longer block unrelated functions.
Using an invalid shader still reports an error and preserves the previous output.
For example, `hydra-blend.js`'s `screen` works even though its unused `phoenix` shader remains invalid.
Internal shader names are not a public library interface.
Compose registered operations through graph chains instead of calling their internal shader names.

| Shader registry limit | Maximum |
|-----------------------|---------|
| Registered functions | 4,096 |
| Combined shader-body text | 4 mebibytes |
| One shader body | 64 kibibytes |
| Inputs per function | 32 |
| Function or input name | 128 characters |

These bounds supplement the configured heap, execution, and serialized-graph limits.
For example, loading swizzle definitions does not remove memory protection.
GLSL is OpenGL's shader language.
`texture2D` calls map to the native GLSL `texture` function.
Custom inputs accept `float` or `sampler2D`.
Samplers require native outputs `o0` through `o3`, or sources `s0` through `s3`.
Texture callbacks, arbitrary graph nodes, and browser texture objects remain unsupported as sampler inputs.

```javascript
setFunction({
  name: "_sample",
  type: "src",
  inputs: [{ name: "_tex", type: "sampler2D", default: o0 }],
  glsl: "return texture2D(_tex, _st);"
});
_sample(o1).out();
```

Direct samplers use raw native texture coordinates.
Use `src(s0)` when a graph needs the usual native source orientation.

#### Graph metadata

Nodes expose `transforms` records containing function metadata and original arguments.
Extensions can copy those arrays and reuse the graph prototype.
This adapter supports Hyper-Hydra's colorspace cloning helpers.
It does not provide browser source, canvas, or framebuffer objects.

```javascript
var original = osc(10);
var copy = Object.assign({}, original);
Object.setPrototypeOf(copy, Object.getPrototypeOf(original));
copy.transforms = Array.from(original.transforms);
copy.invert().out();
```

### Scope and limitations

The initial goal is local JavaScript libraries, not unrestricted browser Hydra compatibility.
Some Hyper-Hydra extensions modify JavaScript prototypes or register shaders.
Others directly access browser canvases, `regl`, or WebGL framebuffer internals.
Those browser integrations remain unsupported and report errors.
For example, loading a canvas-dependent extension cannot provide browser text rendering.

Script execution enforces the configured runtime limits.
At most 128 callback slots can protect previous graphs and build the next graph together.
Unused slots become available on later evaluations.
For example, repeatedly evaluating `osc(() => time).out()` reuses inactive slots.

Evaluation and shader compilation errors preserve the previous graph and its callbacks.
JavaScript global mutations cannot be undone.
For example, assigning `counter = 2` before throwing an error still changes `counter`.
Callback errors pause JavaScript updates and appear in the editor.
Evaluate a corrected sketch to resume.
Reset the interpreter if resource exhaustion prevents normal recovery.
JavaScript runs on the interface thread, never on the plugin's real-time audio thread.

Native helpers do not make browser-dependent extensions compatible without adaptation.
For example, use native `text("Hydra")` instead of constructing a browser canvas.

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

- **Two scripting backends.** Rhai remains the default. The optional [JavaScript backend](#javascript-scripting) supports local libraries, not browser APIs.
- **Expression variables.** Rhai's `time`, `beat`, `tempo`, and `phase` reference GLSL uniforms. JavaScript reads numeric frame values.
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

## Runtime and resource limits

### Background script execution

Each host uses a dedicated worker for script evaluation and preprocessing.
The standalone application also runs JavaScript frame callbacks on its worker.
The application renders the last completed callback values while new work runs.
This can add callback latency and coalesce updates during slow execution.

Bounded slots retain the latest pending request instead of accumulating edits.
Each worker holds one pending command, one pending frame, and one completion slot for each kind.
New evaluations cancel superseded interpreter work and discard stale results.
Cancellation remains cooperative; preprocessing and native calls can finish before noticing it.
Graphics compilation and resource changes stay on the rendering context.
Only successful compilation applies device actions and buffer clearing.
Rejected evaluations preserve the previous graph and restore its callbacks.
Existing JavaScript global-variable rollback limitations still apply.
For example, submitting a corrected sketch supersedes an older request without replacing the graph with stale code.

Runtime resets also run on the worker and preserve rendering until reevaluation succeeds.
Successful resets clear interpreter state and suspend callbacks until evaluation.
Failed resets preserve the interpreter and its settings.
Shutdown cancels interpreter work but can wait for an unfinished native call.
For example, closing the application during a native library read can wait for that read.

### Rhai execution

Rhai evaluation stops after 1,000,000 operations or 200 milliseconds of interpreter execution.
For example, `while true {}` reports an error instead of freezing the application indefinitely.
These limits do not interrupt preprocessing, native functions, or graphics-driver compilation.
JavaScript retains its separate [runtime settings](#runtime-settings-and-recovery).

### Audio and MIDI queues

Audio callbacks publish samples without locking analysis or rendering state.
Musical Instrument Digital Interface (MIDI) callbacks use the same bounded-queue approach.
The audio queue holds 65,536 samples.
The MIDI queue holds 4,096 events.
Both queues keep the newest data when full.

An audio overrun resets beat history.
A MIDI overrun clears held notes and releases envelopes before applying retained events.
For example, newer controller changes still reach `midiCC(1)` after an overrun.
Overruns produce warnings in the application logs.
These queues avoid renderer-lock stalls but do not guarantee a fixed maximum callback duration.

### Rendering and capture

Reevaluation reuses unchanged shader programs and compatible intermediate textures.
For example, changing the source color in `solid(0.2,0.3,0.4,1).blur(1).out()` recompiles only the affected pass.
Failed compilation preserves the previous graph and its feedback history.

Shader generation allows at most 256 estimated source evaluations per pass.
Each `.tileable()` multiplies its input's estimate by four.
Blend inputs and texture-driven parameters also contribute.
For example, four nested `.tileable(0.15)` calls around `noise(2)` reach the limit.
A fifth call reports an error before graphics compilation.
This estimate does not bound arbitrary loops inside custom shader functions.
No automatic tileable texture conversion changes accepted sketches' precision.

Adjacent neighborhood effects reuse the previous effect's rounded texture.
For example, four chained `.blur(1)` calls now require five intermediate passes instead of eight.
Intervening operations still receive their own pass.
For example, `.blur(1).invert().blur(1)` keeps the pass containing `invert()`.

Region controls collect their samples into one small texture before reading it.
Each region still uses an eight-by-eight sample grid.
All sampled regions share one graphics readback.
For example, sixteen `region(o0,0,0,1,1,0)` controls require one readback instead of sixteen.

### Region sampling frequency and latency

`setRegionSampling(rate, asynchronous)` configures standalone region sampling in either scripting language.
The rate accepts zero or values from one to 240 samples per second.
Zero samples every rendered frame.
JavaScript defaults the second argument to `false`; Rhai requires both arguments.
The default remains `setRegionSampling(0, false)`.
For example, `setRegionSampling(10, false)` samples ten times per second while rendering continues normally.

Sampling and Open Sound Control transmission use separate rate limits.
Controls hold the latest completed value between samples.
For example, `setRegionSampling(10, false)` can feed a sender configured for thirty messages per second.
Actual sampling cannot exceed the rendered frame rate.

Set the second argument to `true` to request asynchronous readback.
For example, `setRegionSampling(10, true)` requests ten nonblocking readbacks per second.
Three graphics transfer buffers bound pending work.
The renderer checks completion without waiting and never reads an unfinished transfer buffer.
It skips new requests when all buffers remain busy and logs a warning.
Graphics drivers can still introduce internal submission stalls.
This mode avoids explicit completion waits, not every possible driver stall.

Asynchronous samples arrive in later render callbacks.
Compared with synchronous sampling, controls normally gain at least one additional rendered frame of latency.
Heavy graphics work can delay them longer.
For example, use the default synchronous mode when exact previous-frame control timing matters.
Reevaluation, resizing, and buffer clearing discard queued samples from the old rendering state.
The plugin still rejects region controls, as documented above.

### Broadcast capture

Broadcast capture starts only when viewers connect.
Frames wider than 1280 pixels shrink on the graphics card before transfer to the processor.
For example, a 1920-by-1080 broadcast transfers a 1280-by-720 frame.
Screenshots retain the full window resolution.

Blocking encoder writes and connection exchanges use dedicated workers instead of asynchronous runtime workers.
At most four handshakes run simultaneously.
Each handshake has a ten-second deadline, including connection negotiation.
For example, a stalled connection cannot occupy every asynchronous network worker.

Each connected viewer has a queue holding at most 256 encoded video packets.
A full queue disconnects that viewer and produces a warning.
Other viewers continue independently.
For example, one slow receiver cannot hold up delivery to three faster receivers.
The broadcaster disconnects instead of dropping arbitrary packets from encoded frames.

An encoder pipe stalled for ten seconds triggers a logged restart.
Stopping broadcasting or losing every viewer terminates the encoder.
For example, closing the final receiving window stops both capture demand and encoding.
Shutdown cancels asynchronous work, but operating-system thread cleanup has no strict real-time duration guarantee.

### Images and animated GIFs

Media limits use mebibytes (MiB), where one MiB equals 1,048,576 bytes.

| Resource | Limit |
|---|---|
| Downloaded bytes per request | 32 MiB |
| Decoded red, green, and blue pixel storage per image or animation | 64 MiB |
| Shared decoded pixel storage per image manager | 256 MiB |
| Frames per animated Graphics Interchange Format (GIF) file | 500 |
| Concurrent loading workers per image manager | 2 |
| Queued requests per source slot | 1 |

Oversized media produces a visible application error and a log warning.
For example, `s0.initGif("https://example.com/large.gif")` fails when its decoded frames exceed 64 MiB.
GIF decoding processes frames individually instead of collecting all intermediate color buffers first.
Decoders also receive a 64 MiB allocation limit.
Some codecs treat internal allocation limits as best effort.
Downloads, temporary decoding buffers, frame uploads, and graphics textures remain separate from the shared decoded-storage budget.

New requests replace queued requests and cancel older loads for the same slot.
Cancellation checks run between download reads and GIF frames.
A running native decoder may finish before noticing cancellation.
For example, evaluating `s0.initImage("https://example.com/new.png")` cancels an older pending load for `s0`.
Requests use a 20-second timeout and a two-second socket-read timeout.

## Performance measurements

### Repeatable release benchmarks

The `performance` example measures scripting, analysis, graph replacement, rendering, and readback.
It never starts microphones, cameras, system MIDI ports, network sessions, or audio hosts.
Processor workloads run without a window.
The optional offscreen graphics workloads currently require macOS.

- Run commands from the repository root.
- Use a release build, consistent power settings, and the same compiler and features.
- Avoid simultaneous builds, profilers, and other heavy workloads.
- Save reports under `target/` to keep generated results outside version control.
- Repeat each run before drawing conclusions.

For example, record processor timings and Rust allocations:

```bash
cargo run --release --no-default-features --features fft,midi,javascript \
  --example performance -- --output target/performance-cpu.json --label baseline
```

On macOS, include offscreen graphics measurements:

```bash
cargo run --release --no-default-features --features fft,midi,javascript \
  --example performance -- --gpu --samples 15 --iterations 20 \
  --output target/performance-gpu.json --label baseline
```

Omitted features appear in the report's `skipped` list.
For example, removing `javascript` skips callback workloads without silently reporting zero time.
Invalid arguments, unsupported graphics platforms, and workload failures exit unsuccessfully.
Debug builds refuse measurement.
For example, `--samples 0` reports an error.

Reports include the base revision, source changes, compiler, processor, enabled workload features, and graphics device.
Keep the matching source changes when measuring an uncommitted build.
For example, compare `graph/unchanged_blur` with `graph/change_one_pass` in reports from the same machine.

### Interpreting results

Each workload warms up with three operations.
The default run measures fifteen batches of twenty operations.
Times include operation cleanup.
The report records the nearest-rank median and 95th percentile of batch means.
These are not individual-operation tail latencies or confidence intervals.
For example, one slow operation can disappear inside a twenty-operation average.

Allocation counts use a separate measurement pass.
They count successful Rust allocation and reallocation calls on the benchmark thread.
Requested bytes sum allocation sizes, including full reallocation requests.
They exclude interpreter-native, graphics-driver, and other-thread allocations.
They do not measure retained or peak memory.
The counting allocator remains installed during timings, with counting disabled.
For example, zero reported Rust allocations does not prove a graphics operation allocates no driver memory.

| Timing kind | Meaning | Example |
|---|---|---|
| `cpu_wall` | Calling-thread elapsed time, including operating-system scheduling | Rhai evaluation |
| `gpu_forced_completion_wall` | Submission and completion, including processor work and driver waits | Rendering followed by `glFinish()` |
| Application frame rate | Not measured by this harness | A live window with vertical synchronization |

Graphics workloads render at 1280 by 720 pixels.
Region workloads include rendering and synchronous readback for one or sixteen regions.
Capture workloads include rendering and a 640-by-360 pixel transfer.
For example, compare `readback/render_16_regions` with `render/noise`, not with a live application's frame rate.

Graph workloads exclude scripting and alternate between two prebuilt graphs.
The changed-graph workload modifies one intermediate pass.
Driver shader caches remain warm.
For example, these results do not predict first-launch shader compilation.

Audio producer measurements deliberately fill the bounded queue before timing.
Normal analysis measurements push 800 samples and poll the analyzer.
For example, `audio/push_512_full_queue` measures overflow replacement, not an entire host audio callback.
MIDI workloads poll sixty-four wildcard bindings across one or eight synthetic inputs.
JavaScript workloads retain either zero or 128 reachable callbacks.

### Initial local measurements

This run used Apple M4 Max, macOS 27.2.1, and Rust 1.100.0.
The date was November 26, 2026.
The base revision was `12cd0cb`, with the new benchmark harness and documentation changes.
The command above used `--gpu`, fifteen samples, and twenty iterations.
These illustrative results are not performance guarantees or automatic regression thresholds.

| Workload | Median microseconds | 95th-percentile batch mean | Rust allocation calls per operation |
|---|---:|---:|---:|
| Simple Rhai evaluation | 151.50 | 267.53 | 3303 |
| Preprocessing, 22,814 bytes | 2960.31 | 3088.74 | 80673 |
| Push 512 samples into a full queue | 1.77 | 2.35 | 0 |
| Idle audio poll | 2.81 | 3.20 | 3 |
| Push and analyze 800 samples | 16.56 | 20.06 | 6.2 |
| MIDI, sixty-four bindings, one input | 60.79 | 63.92 | 0 |
| MIDI, sixty-four bindings, eight inputs | 533.06 | 600.02 | 0 |
| JavaScript, zero callbacks | 15.53 | 16.81 | 14 |
| JavaScript, 128 callbacks | 85.82 | 90.74 | 14 |
| Render noise | 158.74 | 416.93 | 0 |
| Render one blur | 196.96 | 201.75 | 0 |
| Render four blurs | 446.44 | 719.47 | 0 |
| Render two nested tileable operations | 697.47 | 947.49 | 0 |
| Replace an unchanged graph | 0.28 | 0.36 | 6 |
| Replace one changed pass | 5068.30 | 5197.14 | 167 |
| Render and sample one region | 189.63 | 398.70 | 0 |
| Render and sample sixteen regions | 604.41 | 699.71 | 0 |
| Render and capture 640 by 360 pixels | 456.46 | 551.90 | 1 |

Fractional allocation counts represent calls averaged across a batch.
For example, rhythm-analysis steps can allocate in some operations but not others.

After effect-pass compaction, the same local setup measured four blurs at approximately 277 microseconds.
That run used five intermediate passes instead of eight.
The earlier median was approximately 446 microseconds.
For example, rerun `render/blur_4` before comparing other machines or graphics drivers.
Scheduling differences affect these measurements; this comparison is not a guaranteed speedup.

### Representative hardware profiling

These live checks remain pending.
Synthetic benchmarks cannot establish audio deadlines, camera throughput, network behavior, or interface responsiveness.
Do not open hardware devices or network listeners on shared machines without permission.

- Record the revision, operating system, processor, graphics driver, display scale, power mode, and build command.
- Record the host name and version, plugin format, audio interface, sample rate, and block size.
- Use the same sketch, window size, and test duration for baseline and candidate builds.
- Warm up for thirty seconds, then record at least sixty seconds.
- Repeat three times and save raw profiler data alongside the results.

For example, compare the same 1280-by-720 sketch before and after a change, using identical host settings.

| Scenario | Procedure | Required measurements |
|---|---|---|
| Plugin audio | Feed audio and MIDI while resizing, maximizing, and reloading sketches | Callback duration distribution, maximum duration, missed deadlines, host underruns |
| Interface stalls | Type, evaluate, resize, and recover from invalid scripts | Main-thread stalls, event-to-paint delay, frame intervals |
| Camera capture | Use one camera, then multiple supported source slots | Delivered and uploaded frames, frame age, processor use, memory growth |
| Broadcast viewers | Compare zero, one, and four permitted local-network viewers | Capture rate, delivered frame rate, encoder use, memory, slow-viewer effects |
| Linux display systems | Repeat window and plugin checks under X11 and Wayland where supported | Window behavior, frame intervals, driver details, host differences |

For audio, calculate the callback deadline as block frames divided by sample rate.
For example, 128 frames at 48,000 samples per second allow approximately 2.67 milliseconds.
Measure the entire host callback, not just queue publication.
Record host underruns separately from audio-analysis queue overruns.

Use the host's deadline monitor or external profiling tools without logging or allocating inside the audio callback.
For example, use Instruments on macOS or `perf` on Linux to identify blocking call stacks.
Statistical sampling alone cannot prove that every callback met its deadline.
If the host lacks callback-duration tracing, report that field as unavailable.

Use a presentation-aware trace for live frame rates.
For example, a sixty-frame-per-second display has approximately 16.67 milliseconds between presentations.
Do not derive live frame rates by inverting the offscreen benchmark times.

Use an externally visible clock to measure camera frame age.
For example, film a running millisecond counter and compare capture with displayed output.
Record camera permissions and requested format before testing.

Use the existing stream examples for permitted viewer tests:

```bash
cargo run --release --features stream --bin hydra -- examples/stream_broadcast.hydra
cargo run --release --features stream --bin hydra -- examples/stream_basic.hydra
```

Verify matching addresses and ports before connecting viewers.
For example, compare zero viewers against four viewers on the same permitted local network.
Confirm that the broadcaster stops capturing when the final viewer disconnects.

Mark hardware profiling complete only after saving results from real audio hosts, cameras, viewers, and Linux display sessions.
For example, an offscreen macOS run cannot satisfy the Linux profiling task.

## Current limitations

- Max nesting depth of 16
- Audio reactivity, webcam input, image-URL loading, MIDI input, video playback, and WebRTC streaming each require building with their own Cargo feature (off by default) — see [Feature-gated functions](#feature-gated-functions) above
- `initVideo` (the `video` feature) additionally requires a standalone `ffmpeg` binary on `PATH` at runtime — it's spawned as a subprocess, not linked into this binary, so building hydra-rust itself never needs FFmpeg's dev libraries. Missing it just logs a warning rather than failing. `initStream`/`broadcastStream` (the `stream` feature) share this same `ffmpeg` requirement.
- `initStream`/`broadcastStream` are hydra-rust-to-hydra-rust only — not interoperable with real hydra.js's own (currently broken) `initStream`/`pb.setName()` — and connect directly by IP:port with no NAT traversal, so both instances need to be reachable from each other directly (typically the same LAN). `broadcastStream` supports any number of simultaneous viewers, sharing one `ffmpeg` encode between them.
- `broadcastStream` captures only for connected viewers. It downscales frames wider than 1280 pixels before reading them from the graphics card.

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

The Rhai backend does not execute external JavaScript libraries.
The optional JavaScript backend still does not provide browser drawing APIs.
Some sketches instantiate
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
extensions (`loadScript` is a no-op in Rhai; JavaScript can load local files).
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

See [media limits](#images-and-animated-gifs) for download, storage, and frame limits.

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
transport/RTP relaying, never codec work itself).
Broadcast capture runs only while viewers connect.
Frames wider than 1280 pixels shrink on the graphics card before capture and encoding.
For example, a 1920-by-1080 window broadcasts at 1280-by-720.

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

In Rhai, some accepted functions do nothing or only partially implement browser
Hydra/community-extension behavior. See the
[stub-function table](#stub-functions-accepted-but-not-yet-implemented)
for the status and rationale of `initScreen`, `P5(...)`, `Scene(...)`,
`loadScript`, browser DOM helpers, and feature-disabled source initializers.
SPEC.md §9 describes these gaps in the language-spec context.
JavaScript rejects unsupported browser APIs instead of providing these Rhai stubs.

`setFunction(...)` is **not** a stub: it registers dynamic custom GLSL
functions. See [Custom GLSL functions](#custom-glsl-functions-setfunction)
and SPEC.md §12.
