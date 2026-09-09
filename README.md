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
| `initImage(idx, url)` | No-op — no image loading/decoding pipeline |
| `initVideo(idx, url)` | No-op — no video file loading pipeline |
| `initScreen(idx[, screen])` | No-op — no screen/display capture |
| `setResolution(w, h)` | No-op — canvas resolution isn't script-controllable |
| `a.show()` / `a.hide()` | No-op — no on-screen FFT debug graph to toggle |
| `smooth(amount)` (pattern) | Accepted but not faithful — any non-zero amount just enables the existing on/off smoothing; doesn't reproduce hydra.js's actual interpolation curve |
| `ease(name)` (pattern) | No-op — passes the pattern through unchanged; no easing-curve implementations |
| `fit(lo, hi)` (pattern) | No-op — passes the pattern through unchanged; no value-range remapping |
| `screencap()` | No-op — saving/sharing a screenshot isn't supported |
| `Math.random()` | Not rewritten at all (unlike other `Math.*` methods) — no GLSL-expression equivalent for real randomness |

## License

[AGPL-3.0-or-later](https://www.gnu.org/licenses/agpl-3.0.html)
