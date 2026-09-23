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

```bash
cargo build          # library only
cargo run            # standalone binary
cargo test           # tests
cargo clippy         # lint
```

## Keyboard shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl/Cmd + Enter` | Evaluate the current sketch |
| `Ctrl/Cmd + S` | Save the current sketch to a `.hydra` file |
| `Ctrl/Cmd + O` | Open a `.hydra` file |
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

The functions below only exist when the library is built with the matching Cargo feature (`cargo build --features <name>`, or comma-separated for several: `--features webcam,audio,image_url,midi,video,stream`). Without the feature, calling one of these fails with a plain "Function not found" error. See [SPEC.md](SPEC.md) for the complete function reference, including the ~48 always-available core functions.

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

| Function | Description | Example |
|----------|-------------|---------|
| `a.fft[i]` | A frequency bin's magnitude (roughly `0`-`1`, shaped by the setters below); usable as any numeric argument | `osc(60, 0.1, a.fft[0]).out()` |
| `a.setBins(n)` | How many bins to reduce the spectrum into (default `4`) | `a.setBins(8)` |
| `a.setCutoff(c)` | Zeroes out bin values below this noise-floor threshold (default `0`) | `a.setCutoff(0.15)` |
| `a.setScale(s)` | Multiplies every bin's value (default `1`) | `a.setScale(2)` |
| `a.setSmooth(s)` | Exponential smoothing between frames, `0`-`1` (default `0.4`) | `a.setSmooth(0.8)` |
| `a.show()` / `a.hide()` | Shows/hides a small on-screen bar-graph overlay of the current FFT bins — an egui overlay standing in for real hydra.js's own on-screen debug graph, since nothing else here draws directly into the GL canvas | `a.show()` |

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

A native port of the [hydra-midi](https://github.com/arnoson/hydra-midi) community extension's API — real hydra.js has no MIDI support in its own core at all. Note names use standard scientific-pitch-notation/General-MIDI numbering (`"C4"` is 60, middle C). All MIDI channels and input devices are merged into one rather than filtered separately.

| Function | Description | Example |
|----------|-------------|---------|
| `note(nameOrNumber[, channel])` | A gate: `1` while the note is held, `0` otherwise (not velocity) | `osc().invert(note(60)).out()` |
| `note(...).velocity()` | The note's last velocity, `0`-`1` (`0` once released) | `osc(60, 0.1, note("C4").velocity()).out()` |
| `note(...).adsr(a, d, s, r)` | An ADSR envelope (`a`/`d`/`r` in milliseconds, `s` a `0`-`1` sustain level) triggered by the note's on/off events | `solid(1, 0, note(60).adsr(50, 100, 0.7, 300)).out()` |
| `cc(index[, channel])` | A CC controller's value, normalized to `0`-`1` | `osc().rotate(cc(1)).out()` |
| `cc(...).smooth(factor=0.01)` | Exponential slew of a CC value between frames | `osc(cc(1).smooth(0.2)).out()` |
| `aft()` | Channel-wide aftertouch (channel pressure), normalized to `0`-`1` | `osc().brightness(aft()).out()` |
| `aft(nameOrNumber[, channel])` | Per-note polyphonic aftertouch (key pressure) for that note, normalized to `0`-`1` — passing a note is what distinguishes this from channel-wide `aft()` | `osc(60, 0.1, aft(60)).out()` |
| `.range(lo, hi)` (on `note`/`cc`/`aft`/`.velocity()`) | Remaps a `0`-`1` value into `[lo, hi]` | `osc().rotate(cc(1).range(0, 6.28)).out()` |
| `.scale(factor)` (on `note`/`cc`/`aft`/`.velocity()`) | Multiplies a value | `osc(1, 1, note(60).scale(0.5)).out()` |
| `_note(...)` / `_cc(...)` / `_noteVelocity(...)` / `_aft(...)` | Plain (non-chainable) equivalents, for use inside a `()=>` wrapper | `osc(1, 1, _note(60) * 0.5).out()` |
| `midi.start()` | Connects to every available MIDI input device (required before any of the above reacts to anything) | `midi.start()` |
| `midi.pause()` | Disconnects from all MIDI input devices | `midi.pause()` |
| `midi.show()` / `.hide()` | Shows/hides an on-screen overlay listing currently-held notes (with velocity) and non-zero CC values — a "current state" snapshot rather than real hydra-midi's own scrolling raw-message log, simpler to implement and just as useful for confirming a controller is connected | `midi.show()` |
| `midi.channel(n)` / `.input(n)` | Accepted, logged, ignored — channels/inputs are merged (see above) | `midi.channel(0)` |

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
cargo run --release --features webcam,audio,image_url,midi,video,stream --example check_corpus -- sketches
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

hydra-rust is the visual engine of [Sova](https://github.com/Bubobubobubobubo/Sova), a polyglot live coding sequencer for music and visuals. In Sova, hydra shaders sync to the musical clock via `beat`, `tempo`, and `phase`, and can be shared across peers in multiplayer sessions.

## Differences from browser Hydra

- **Rhai, not JavaScript.** No closures or arrow functions. Write `osc(60.0, sin(time))` directly.
- **Expression variables.** `time`, `beat`, `tempo`, `phase` are GLSL uniform references, not callback functions.
- **Mouse input.** `mouseX` and `mouseY` are normalized to `[0, 1]`.
- **No `speed` global.** Animation speed is per-source, controlled through function arguments.
- **GLSL 330.** Targets OpenGL 3.3 core profile.

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
| `P5(...)` / `new P5(...)` | Returns a plain settable map (like `hydraText`) rather than hard-erroring, so an assignment (`let p1 = P5(...)`) and later property reads/writes on it still work — p5.js is a whole separate creative-coding framework with no Rust equivalent here. A handful of its most commonly-called instance methods (`hide`, `show`, `textSize`, `fill`) are additionally registered as no-ops on that map; the rest of its (large) API isn't |
| `setFunction(descriptor)` | No-op — real hydra.js registers a custom GLSL source/color/combine function from a JS descriptor object + GLSL string; no dynamic function-registration or GLSL-embedding pipeline exists here |
| `Scene(name)` | No-op — not a hydra.js API at all; some external VJ/live-coding integrations use it to switch named cue banks |
| `.value(fn)` (MIDI `note`/`cc`, `midi` feature) | Not implemented — each chain compiles to a static GLSL expression once, so there's nowhere to run an arbitrary per-frame Rhai closure the way a real per-frame JS callback would; calling it cleanly fails as "Function not found" |
| `midi.channel(n)` / `.input(n)` | Accepted, logged, ignored — MIDI channels and input devices are all merged into one rather than faithfully filtered (see SPEC.md §6.1) |
| `ease(name)` (pattern) | Accepted but not faithful — `smooth()` still interpolates linearly regardless of the named curve; no non-linear easing curves are implemented |
| `screencap()` | No-op — saving/sharing a screenshot isn't supported |
| `loadScript(url)` | No-op — no dynamic module loading. Some of the most commonly-loaded community extensions' functions are ported natively instead (`spiral`, `turb`, `uturb`, `unoise`, `whitenoise`, `colornoise`, `warp`, `cwarp`, `ncontour`, `pulse`, `pulsetrain`, `hextile`, `concentric`, `brick`, `wave`, `lissa`, `inversion`, `mirrorX`/`mirrorY`/`mirrorX2`/`mirrorY2`/`mirrorWrap`, `colreflect` — see SPEC.md §6); anything else the loaded script would have defined still won't exist |
| `pb.setName(name)` / `pb.list()` | No-op — not a hydra.js API at all; boilerplate an external platform injects when a sketch is exported/shared |
| `hydraText.font = ...` / any other property | No-op — the `hydra-text.js` extension's config object (`loadScript`-loaded); accepts assignment to any property. `strokeText`/`fillStrokeText`/`strokeFillText` themselves are aliased to `text()`'s own rendering (not faithful - no separate stroke/outline mode) |

## License

[AGPL-3.0-or-later](https://www.gnu.org/licenses/agpl-3.0.html)
