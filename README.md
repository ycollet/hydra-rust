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
- Toggle editor visibility with `Tab` or `Ctrl+Shift+H`

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
so `a.fft[...]`/`initCam(...)`-style sketches don't fail just because those
functions aren't registered):

```bash
cargo run --release --features webcam,audio --example check_corpus -- sketches
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
- Audio reactivity (`a.fft[]`, `a.setBins`/`setCutoff`/`setScale`/`setSmooth`) and webcam input (`initCam`) require building with the `audio`/`webcam` Cargo features respectively (off by default)

### Stub functions (accepted, but not yet implemented)

These are registered so scripts calling them don't hard-error, but they don't do anything real yet:

| Function | Status |
|----------|--------|
| `initImage(idx, url)` | No-op (returns the source as a chainable value, like real hydra.js) — no image loading/decoding pipeline |
| `initVideo(idx, url)` | No-op (returns the source as a chainable value) — no video file loading pipeline |
| `initGif(idx, url)` | No-op (returns the source as a chainable value) — no GIF loading pipeline |
| `initScreen(idx[, screen])` | No-op (returns the source as a chainable value) — no screen/display capture |
| `setResolution(w, h)` | No-op — canvas resolution isn't script-controllable |
| `a.show()` / `a.hide()` | No-op — no on-screen FFT debug graph to toggle |
| `smooth(amount)` (pattern) | Accepted but not faithful — any non-zero amount just enables the existing on/off smoothing; doesn't reproduce hydra.js's actual interpolation curve |
| `ease(name)` (pattern) | No-op — passes the pattern through unchanged; no easing-curve implementations |
| `fit(lo, hi)` (pattern) | No-op — passes the pattern through unchanged; no value-range remapping |
| `screencap()` | No-op — saving/sharing a screenshot isn't supported |
| `loadScript(url)` | No-op — no dynamic module loading. Some of the most commonly-loaded community extensions' functions are ported natively instead (`spiral`, `turb`, `uturb`, `unoise`, `whitenoise`, `colornoise`, `warp`, `cwarp`, `ncontour`, `pulse`, `pulsetrain`, `hextile`, `concentric`, `brick`, `wave`, `lissa`, `inversion`, `mirrorX`/`mirrorY`/`mirrorX2`/`mirrorY2`/`mirrorWrap`, `colreflect` — see SPEC.md §6); anything else the loaded script would have defined still won't exist |
| `o0-o3.setNearest()` / `.setLinear()` / `.setMode(name)` | No-op — buffers are always sampled with linear filtering; no per-buffer sampler state to switch |
| `pb.setName(name)` / `pb.list()` | No-op — not a hydra.js API at all; boilerplate an external platform injects when a sketch is exported/shared |
| `hydraText.font = ...` / any other property | No-op — the `hydra-text.js` extension's config object (`loadScript`-loaded); accepts assignment to any property, but the extension's own `strokeText`/text-rendering function still won't exist |

## License

[AGPL-3.0-or-later](https://www.gnu.org/licenses/agpl-3.0.html)
