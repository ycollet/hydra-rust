# hydra-rust

A Rust port of [Hydra](https://hydra.ojack.xyz/) — the live-codable video synthesizer created by [Olivia Jack](https://ojack.xyz/). Takes [Rhai](https://rhai.rs/) scripts, compiles them to GLSL shaders, and renders them via OpenGL. The core is a library with zero GUI dependencies, suitable for embedding. A standalone binary is included for testing and standalone use. Originally extracted from [Sova](https://github.com/Bubobubobubobubo/Sova), the polyglot live coding sequencer.

See [SPEC.md](SPEC.md) for the full language specification (reactive values, function reference, the JS-compatibility layer, feature flags, and known gaps).

## Features

### 48 built-in functions

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
- Scene banks — 4 banks x 16 slots for instant scene recall, see [below](#scene-banks)
- Keyboard-driven — see [Keyboard shortcuts](#keyboard-shortcuts)

### Library

- Zero GUI dependencies — only `rhai`, `glow`, and `ab_glyph`
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

The functions below only exist when the library is built with the matching Cargo feature (`cargo build --features <name>`, or comma-separated for several: `--features webcam,audio,image_url,midi,osc,video,stream`). Without the feature, calling one of these fails with a plain "Function not found" error. See [SPEC.md](SPEC.md) for the complete function reference, including the ~48 always-available core functions.

### `webcam` — camera input

```bash
cargo run --features webcam --bin hydra
```

| Function | Description | Example |
|----------|-------------|---------|
| `initCam(slot)` | Starts the default camera (index 0), populating source slot `s0`-`s3` | `s0.initCam().out()` |
| `initCam(slot, cameraIndex)` | Starts a specific camera by index | `s0.initCam(1).out()` |

### `audio` — microphone FFT reactivity

```bash
cargo run --features audio --bin hydra
```

The microphone is only opened once a script actually uses one of these (calls a setter, or reads `a.fft[i]`) — never just because the feature is compiled in.
The scripting API also exists with `fft` alone for host-fed audio, as used
by the plugin, without opening a microphone.

| Function | Description | Example |
|----------|-------------|---------|
| `a.fft[i]` | A frequency bin's magnitude (roughly `0`-`1`, shaped by the setters below); usable as any numeric argument | `osc(60, 0.1, a.fft[0]).out()` |
| `a.setBins(n)` | How many bins to reduce the spectrum into (default `4`) | `a.setBins(8)` |
| `a.setCutoff(c)` | Zeroes out bin values below this noise-floor threshold (default `0`) | `a.setCutoff(0.15)` |
| `a.setScale(s)` | Multiplies every bin's value (default `1`) | `a.setScale(2)` |
| `a.setSmooth(s)` | Exponential smoothing between frames, `0`-`1` (default `0.4`) | `a.setSmooth(0.8)` |
| `a.settings[i].cutoff` / `.scale` / `.smooth` | Read or change one active bin's settings; static numeric values only | `a.settings[0].scale = 3` |
| `a.show()` / `a.hide()` | Shows/hides a small on-screen bar-graph overlay of the current FFT bins — an egui overlay standing in for real hydra.js's own on-screen debug graph, since nothing else here draws directly into the GL canvas | `a.show()` |

`a.settings.length` is the active bin count (maximum 8). Global setters
update all bins; `a.setBins(n)` resets the per-bin settings to the current
global defaults and clears smoothing history. Cutoff, gain, and FFT
normalization retain hydra-rust's existing behavior, not Hydra's Meyda
loudness units. Embedding hosts can use `eval_with_audio_settings()` with
their analyzer's `settings()` to preserve config reads between evaluations;
the desktop app and plugin do this automatically.

### `image_url` — load an image or animated GIF from a URL

```bash
cargo run --features image_url --bin hydra
```

| Function | Description | Example |
|----------|-------------|---------|
| `initImage(slot, url)` | Fetches and decodes a still image (PNG/JPEG/GIF/WebP) in the background, uploading it to a source slot once it's ready | `s0.initImage("https://example.com/pic.png").out()` |
| `initGif(slot, url)` | Fetches an animated GIF, decodes every frame up front, and cycles through them by elapsed time once loaded — looping indefinitely, like a real `<img>` GIF | `s0.initGif("https://example.com/anim.gif").out()` |

### `midi` — MIDI input

```bash
cargo run --features midi --bin hydra
```

A native port of the [hydra-midi](https://github.com/arnoson/hydra-midi) community extension's API — real hydra.js has no MIDI support in its own core at all. Note names use standard scientific-pitch-notation/General-MIDI numbering (`"C4"` is 60, middle C). Channels are zero-based (`0`–`15`); inputs accept a zero-based port index or an exact name. `'*'` selects any channel/input (the default), and any note or CC index.

| Function | Description | Example |
|----------|-------------|---------|
| `note(nameOrNumber[, channel[, input]])` | A gate: `1` while the note is held, `0` otherwise (not velocity) | `osc().invert(note(60, 0, 'Keys')).out()` |
| `note(...).velocity()` | The note's last velocity, `0`-`1` (`0` once released) | `osc(60, 0.1, note("C4").velocity()).out()` |
| `note(...).adsr(a, d, s, r)` | An ADSR envelope (`a`/`d`/`r` in milliseconds, `s` a `0`-`1` sustain level) triggered by the note's on/off events | `solid(1, 0, note(60).adsr(50, 100, 0.7, 300)).out()` |
| `cc(index[, channel[, input]])` | A CC controller's value, normalized to `0`-`1` | `osc().rotate(cc(1)).out()` |
| `cc(...).smooth(factor=0.01)` | Exponential slew of a CC value between frames | `osc(cc(1).smooth(0.2)).out()` |
| `aft()` | Channel-wide aftertouch (channel pressure), normalized to `0`-`1` | `osc().brightness(aft()).out()` |
| `aft(nameOrNumber[, channel])` | Per-note polyphonic aftertouch (key pressure) for that note, normalized to `0`-`1` — passing a note is what distinguishes this from channel-wide `aft()` | `osc(60, 0.1, aft(60)).out()` |
| `.range(lo, hi)` (on `note`/`cc`/`aft`/`.velocity()`) | Remaps a `0`-`1` value into `[lo, hi]` | `osc().rotate(cc(1).range(0, 6.28)).out()` |
| `.scale(factor)` (on `note`/`cc`/`aft`/`.velocity()`) | Multiplies a value | `osc(1, 1, note(60).scale(0.5)).out()` |
| `.value(v => expression)` | Compiles a pure inline numeric transform into live GLSL; also works after `.smooth()`, `.adsr()`, `.range()`, or `.scale()` | `osc(cc(1).value(v => v * 20 + time)).out()` |
| `_note(...)` / `_cc(...)` / `_noteVelocity(...)` / `_aft(...)` | Plain (non-chainable) equivalents, for use inside a `()=>` wrapper | `osc(1, 1, _note(60) * 0.5).out()` |
| `midi.start()` | Connects to every available MIDI input device (required before any of the above reacts to anything) | `midi.start()` |
| `midi.start({channel, input})` | Sets defaults for values created in this evaluation, then starts input | `midi.start({channel: 0, input: 'Keys'})` |
| `midi.pause()` | Disconnects from all MIDI input devices | `midi.pause()` |
| `midi.show()` / `.hide()` | Shows/hides an on-screen overlay listing currently-held notes (with velocity) and non-zero CC values — a "current state" snapshot rather than real hydra-midi's own scrolling raw-message log, simpler to implement and just as useful for confirming a controller is connected | `midi.show()` |
| `midi.channel(n)` / `.input(indexOrName)` | Returns a reusable selector without changing global defaults; supports chained `note`, `cc`, and `aft` calls with explicit overrides | `midi.input('Keys').channel(0).cc(74)` |

Filtering also applies to the `_note`/`_cc`/`_noteVelocity`/`_aft` forms,
aftertouch, CC smoothing, and ADSR triggers. There are 64 filtered/wildcard
value slots and 16 envelope slots per evaluated script; exceeding either
limit is an error. Wildcard values follow the latest matching event.
Like envelope slots, filtered value slots are assigned per evaluation:
reevaluate all affected output buffers together, rather than leaving older
shaders running with slot IDs that a partial evaluation could reuse.
Run with `RUST_LOG=info` to see connected device names; port indices follow
the system's enumeration order. The plugin sees one host-routed input,
index `0`, named `"host"`, and preserves event channels.

`.value()` accepts one-parameter expression callbacks, captured constants,
`time`, and numeric math functions (`sin`, `cos`, `tan`, `asin`, `acos`,
`atan`, `abs`, `fract`, `floor`, `ceil`, `sqrt`, `sign`, `exp`, `log`,
`pow`, `min`, `max`, including supported `Math.*` aliases). It does not run JS/Rhai
callbacks every frame: blocks, assignments, runtime branching, arbitrary
function calls, and side effects are rejected explicitly. Smoothing a
transformed `.value()` result is not supported; smooth the CC first.

### `osc` — OSC (Open Sound Control) input

```bash
cargo run --features osc --bin hydra
```

A native UDP OSC listener, modeled on the [hydra-osc](https://github.com/ojack/hydra-osc) example and atom-hydra's more complete [`osc-loader.js`](https://github.com/hydra-synth/atom-hydra/blob/master/lib/osc-loader.js) (default port `57101`, matching atom-hydra's own). Exposed as `_osc` rather than `osc` — real hydra.js's `osc(freq, sync, offset)` is already the sine-oscillator source function, so a bare `osc` would collide with it. Unlike real hydra-osc/atom-hydra's `_osc.on(address, callback)` event style, values are read reactively instead: compiled shader expressions do not run arbitrary per-message callbacks.

| Function | Description | Example |
|----------|-------------|---------|
| `_osc.get(address)` | The most recent first argument received at that OSC address, or `0` if none has arrived yet | `osc(60, 0.1, _osc.get("/hue")).out()` |
| `_osc.get(address, argIndex)` | The most recent `argIndex`'th argument (0-based) at that address | `osc().rotate(_osc.get("/xyz", 1)).out()` |
| `.range(lo, hi)` / `.scale(factor)` (on `_osc.get(...)`) | Same generic remap/multiply MIDI's `note`/`cc`/`aft` chains use | `osc().rotate(_osc.get("/x").range(0, 6.28)).out()` |
| `_osc.start()` / `_osc.start(port)` | Binds a UDP socket and starts listening (default port `57101`) — required before `.get()` reacts to anything | `_osc.start(9000)` |
| `_osc.pause()` | Stops listening and releases the UDP socket | `_osc.pause()` |
| `_osc.show()` / `.hide()` | Shows/hides an on-screen overlay listing every address received so far and its latest arguments | `_osc.show()` |

Each distinct `_osc.get(...)` call site claims one slot from a small fixed pool (32), assigned round-robin at eval time — OSC addresses are arbitrary strings, unlike MIDI's 0-127 note/CC numbers, so there's no natural small index to key a uniform array on directly.

### `video` — play a video file or URL as a source

```bash
cargo run --features video --bin hydra
```

Requires a standalone `ffmpeg` binary on `PATH` at runtime (`brew install ffmpeg`, `apt install ffmpeg`, ...) — it's spawned as a subprocess via [ffmpeg-sidecar](https://github.com/nathanbabcock/ffmpeg-sidecar), never linked into this binary, so `cargo build` itself never needs FFmpeg's dev libraries. If `ffmpeg` isn't found, `initVideo` logs one warning and leaves the slot empty rather than failing to build or run.

| Function | Description | Example |
|----------|-------------|---------|
| `initVideo(slot, url)` | Streams frames from a local video file or a remote URL, looping indefinitely, uploading each new frame to a source slot as it's decoded | `s0.initVideo("/path/to/clip.mp4").out()` |

### `stream` — stream video between hydra-rust instances over WebRTC

```bash
cargo run --features stream --bin hydra
```

**Not** real hydra.js's own `initStream`/`pb.setName()` — that feature is itself currently broken in the live hydra.js editor (its signaling server hasn't been touched since 2024), and its wire protocol is bespoke and undocumented. This is a **hydra-rust-to-hydra-rust** feature instead. Connects directly by IP:port — no relay server, no STUN/TURN (deliberately; see `src/stream.rs`'s module doc comment) — so it's built for two instances on the same LAN/room, not across separate NATs on the open internet. Requires `ffmpeg` on `PATH` at runtime, same as `video` (also spawned as a subprocess, also never linked in). A broadcast frame wider than 1280px is downscaled before encoding (found necessary via real testing — a Retina display's actual framebuffer resolution is far more pixels than realtime VP8 encoding can keep up with otherwise).

| Function | Description | Example |
|----------|-------------|---------|
| `s0.initStream("host:port")` | Connects to a broadcaster listening at that address and streams its video into a source slot | `s0.initStream("192.168.1.20:9000").out()` |
| `broadcastStream(port)` | Broadcasts *this sketch's own rendered output* — whatever `render()` currently displays — to anything that connects on `port`. Any number of viewers can connect, all sharing a single `ffmpeg` encode (only the WebRTC transport is per-viewer). No-op if already broadcasting (call `stopBroadcast()` first to change ports) | `broadcastStream(9000)` |
| `stopBroadcast()` | Stops broadcasting, disconnecting every connected viewer | `stopBroadcast()` |

The simplest way to try it is two runnable `.hydra` scripts talking to each other directly — no separate tools needed (edit the address in the receiving one first):
```bash
cargo run --features stream --bin hydra -- examples/stream_broadcast.hydra   # broadcasts its own visuals on :9000
cargo run --features stream --bin hydra -- examples/stream_basic.hydra      # receives them - bare initStream().out()
cargo run --features stream --bin hydra -- examples/stream_vj.hydra         # or with kaleid/modulate/layer on top
```

Two companion CLI examples are also available, useful for testing/debugging without a full GUI app — a "server" that broadcasts a webcam/file/test pattern (not a live sketch), and a "client" that receives and reports frame stats headlessly:
```bash
cargo run --features stream --example webrtc_broadcast -- 9000
# Or broadcast a real webcam/file: --format avfoundation --input 0  (macOS)
#                                   --format v4l2 --input /dev/video0  (Linux)
#                                   --input clip.mp4  (any file)

cargo run --features stream --example webrtc_receive -- <broadcaster's-ip>:9000 --save frame.ppm
```

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

- **Rhai, not JavaScript.** No closures or arrow functions. Write `osc(60.0, sin(time))` directly.
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
| `initStream(idx, url)` | Real implementation behind the `stream` feature: connects to another hydra-rust instance over WebRTC (see the `stream` section above) — not real hydra.js's own (currently broken) `initStream`. Without that feature, it's a no-op (returns the source as a chainable value) |
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

## License

[AGPL-3.0-or-later](https://www.gnu.org/licenses/agpl-3.0.html)
