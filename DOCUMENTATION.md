# hydra-rust function documentation

A complete, example-driven reference for every function `hydra_rust::eval()` registers —
core functions (always available) and feature-gated ones (`webcam`, `audio`, `image_url`,
`midi`, `video`), plus the standalone `hydra` binary's scene-bank system. Each entry lists
its parameters (with defaults) and a small runnable example.

This complements [SPEC.md](SPEC.md), which documents the *language* (the JS-compatibility
preprocessing pipeline, reactive-value semantics, chain/buffer model, patterns) rather than
enumerating every function with examples. See SPEC.md if you want the "why" behind a
behavior; see this document if you want "what does function X take and how do I call it."

Every example is a complete, valid hydra-rust script (verified against `eval()`) that can be
pasted into the standalone binary's editor or passed to the library's `eval()` function
directly. Most omit `.out()` where a bare terminating source/chain would implicitly write to
`o0` (see SPEC.md §2) — but calling `.out()` explicitly is always fine too.

## Contents

- [How chains work](#how-chains-work)
- [Sources](#sources)
- [Geometry](#geometry)
- [Color](#color)
- [Blend](#blend)
- [Modulate](#modulate)
- [Other core functions](#other-core-functions)
- [Reactive values](#reactive-values)
- [Math functions](#math-functions)
- [Patterns](#patterns)
- [Feature: `webcam`](#feature-webcam)
- [Feature: `audio`](#feature-audio)
- [Feature: `image_url`](#feature-image_url)
- [Feature: `midi`](#feature-midi)
- [Feature: `video`](#feature-video)
- [Feature: `stream`](#feature-stream)
- [Scene banks (standalone binary)](#scene-banks-standalone-binary)
- [Known gaps and stub functions](#known-gaps-and-stub-functions)

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
| `text` | `content: string` | Rasterizes a string (bundled Hack font) into a shared text texture and returns a chain reading it. Only one call's content is visible at a time (last one wins per evaluation). Accepts an optional second `config` argument for parity with real hydra.js — accepted and ignored | `text("hydra-rust").out()` |
| `strokeText` / `fillStrokeText` / `strokeFillText` | `content: string` | Aliases of `text(...)` — the `hydra-text.js` extension's stroke/outline variants; not faithful (no separate stroke/outline rendering mode exists here, they render exactly like `text`) | `strokeText("hi").out()` |

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
| `random` | *(none)*, or 0-2 ignored extra args | A one-shot pseudo-random `f64` in `[0, 1)`, computed once at script-eval time (not per-frame) — the target of `Math.random()` | `osc(random() * 100, 0.1, 0).out()` |
| `setResolution` | `w, h` (must be static numbers, clamped to `1..=4096`) | Overrides the render buffers' own resolution, independent of the window size — the display still stretches to fill the window (the classic low-res/pixelation trick). A reactive argument (e.g. `window.innerWidth`) is treated as "no override" instead, since there's no per-frame callback to re-evaluate it against. Sticky — persists across evaluations that don't call it again | `setResolution(320, 240)` |
| `o0`-`o3`.`setNearest()` | *(none)* | Sets that buffer's texture sampling to nearest-neighbor (blocky) instead of linear. Sticky, like `setResolution` | `o0.setNearest()` |
| `o0`-`o3`.`setLinear()` | *(none)* | Sets that buffer's texture sampling back to linear (the default) | `o0.setLinear()` |
| `o0`-`o3`.`setMode(name)` | `name: "nearest"` or `"linear"` | Same as `setNearest`/`setLinear`, chosen by string. An unrecognized name is logged and ignored | `o0.setMode("nearest")` |

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
| `a.fft[i]` | `iFft[i]` | **`audio` feature only** — see [below](#feature-audio) |

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
| `.ease(name)` | Accepted (any curve name), but only toggles smoothing on if not already smoothed — always linear, no named easing curves |
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
| `a.show()` / `a.hide()` | Shows/hides a small on-screen bar-graph overlay of the current FFT bins | `a.show()` |

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
scientific-pitch-notation/General-MIDI numbering (`"C4"` is 60, middle C). All MIDI channels
and input devices are merged into one rather than filtered separately.

| Function | Description | Example |
|---|---|---|
| `note(nameOrNumber[, channel])` | A chainable gate: `1` while the note is held, `0` otherwise (not velocity). `channel` is accepted but ignored | `osc().invert(note(60)).out()` |
| `note(...).velocity()` | The note's last velocity, `0`-`1` (`0` once released) | `osc(60, 0.1, note("C4").velocity()).out()` |
| `note(...).adsr(a, d, s, r)` | An ADSR envelope (`a`/`d`/`r` in milliseconds, `s` a `0`-`1` sustain level), keyed to that note's on/off events, multiplied by the velocity captured at trigger time | `solid(1, 0, 0, note(60).adsr(50, 100, 0.7, 300)).out()` |
| `cc(index[, channel])` | A chainable, raw CC value normalized to `0`-`1` | `osc().rotate(cc(1)).out()` |
| `cc(...).smooth(factor=0.01)` | Exponential slew (temporal smoothing) of the CC value | `osc(60, 0.1, cc(1).smooth(0.2)).out()` |
| `.range(lo, hi)` (on `note`/`cc`/`.velocity()`) | Linearly remaps a `0`-`1` value into `[lo, hi]` | `osc().rotate(cc(1).range(0, 6.28)).out()` |
| `.scale(factor)` (on `note`/`cc`/`.velocity()`) | Multiplies a value | `osc(1, 1, note(60).scale(0.5)).out()` |
| `_note(nameOrNumber[, channel])` | Plain (non-chainable) equivalent of `note(...)`, for use inside a `()=>` wrapper | `osc(1, 1, _note(60) * 0.5).out()` |
| `_cc(index[, channel])` | Plain (non-chainable) equivalent of `cc(...)` | `osc(1, 1, _cc(1) * 6.28).out()` |
| `_noteVelocity(nameOrNumber[, channel])` | Plain (non-chainable) equivalent of `note(...).velocity()` | `osc(1, 1, _noteVelocity(60)).out()` |
| `midi.start()` | Connects to every available MIDI input device — required before anything above reacts to input. Returns `midi` again, so `midi.start().show()` still parses | `midi.start()` |
| `midi.pause()` | Disconnects from all MIDI input devices | `midi.pause()` |
| `midi.show()` / `.hide()` | Shows/hides an on-screen overlay listing currently-held notes (with velocity) and non-zero CC values — a "current state" snapshot rather than real hydra-midi's own scrolling raw-message log | `midi.show()` |
| `midi.channel(n)` / `.input(n)` | Accepted, logged, ignored — channels/inputs are merged (see above) | `midi.channel(0)` |

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
| `broadcastStream(port)` | Broadcasts this sketch's own rendered output to the next `initStream` connection on `port`. One viewer at a time; a no-op if already broadcasting | `broadcastStream(9000)` |
| `stopBroadcast()` | Stops broadcasting | `stopBroadcast()` |

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

## Scene banks (standalone binary)

Not a scripting-language feature — a `hydra` binary UI/workflow feature for live performance,
a native port of [HYDRACTRL](https://github.com/dxviie/HYDRACTRL)'s bank system: **4 banks x
16 slots** (64 scenes total), each slot holding a saved sketch's source code. Banks persist
across restarts in the session file (`~/.hydra-rust.json`); a single bank can also be
exported/imported as a `.bhr` file, and a single slot's code as a `.shr` file.

| Shortcut | Action |
|---|---|
| `Alt/⌥ + 0`-`9` / `A`-`F` | Recall slot `0`-`F` (hex) in the active bank — loads and immediately evaluates its saved code |
| `Alt/⌥ + Shift + 0`-`9` / `A`-`F` | Save the editor's current code into that slot |
| `Alt/⌥ + ←` / `→` | Cycle to the previous/next scene bank |
| `Alt/⌥ + X` | Export the active bank (16 slots) as a `.bhr` file |
| `Alt/⌥ + I` | Import a `.bhr` file into the active bank, replacing its 16 slots |

The recalled/saved-to slot then auto-persists from the live editor every frame, so switching
slots, cycling banks, opening a different file, or quitting never silently discards an
in-progress edit. Slots can also be clicked directly in the sidebar (`Tab` to show it).

CLI flags:

```bash
hydra -i mysketch.hydra              # --input: load a sketch at startup
hydra -bl mybank.bhr                 # --bank-load: preload a .bhr file into the active bank
hydra -bs mybank.bhr                 # --bank-save: Alt+X writes straight to this path
hydra -sl mysketch.shr               # --slot-load: load a .shr file's code at startup
hydra -ss mysketch.shr               # --slot-save: snapshot the starting code to this path once, at launch
```

See [README.md](README.md#keyboard-shortcuts) for the full keyboard-shortcut table (including
non-bank shortcuts like evaluate/save/open) and [README.md](README.md#scene-banks) for more
detail on the `.bhr`/`.shr` formats.

## Known gaps and stub functions

A handful of functions are registered (so a script calling them doesn't hard-error) but don't
do anything real, or only partially implement real hydra.js/community-extension behavior —
`initScreen`, `P5(...)`, `setFunction`, `Scene(...)`,
`loadScript`, `ease` (non-linear curves), MIDI aftertouch, `.value(fn)`, and a few others. See
[README.md's stub-function table](README.md#stub-functions-accepted-but-not-yet-implemented)
for the complete, currently-accurate list with rationale for each, and SPEC.md §9 for the
same list in the language-spec context.
