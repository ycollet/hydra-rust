# hydra-rust language specification

This document specifies the scripting language accepted by `hydra_rust::eval()` —
the dialect of [Rhai](https://rhai.rs/) that compiles to GLSL fragment shaders,
and the JS-compatibility layer that lets real [hydra-synth](https://hydra.ojack.xyz/)
sketches run largely unmodified. It describes the *current implementation*
(`src/eval.rs`, `src/library.glsl`, and the preprocessing passes in `src/*.rs`),
not an aspirational design — anything marked "stub" or "not implemented" is a
known gap, not an oversight.

## 1. Execution model

A patch is a single string of source code, evaluated by `eval(code: &str) ->
Result<EvalResult, String>`. Evaluation has three stages:

1. **JS-compatibility preprocessing** (§4) — a fixed pipeline of textual
   rewrites turns common real-hydra.js idioms into valid Rhai before the
   script is parsed at all.
2. **Rhai evaluation** — the rewritten source runs on a fresh `rhai::Engine`
   with hydra's functions and reactive globals registered (§2, §3, §5). This
   builds an in-memory `Node` tree (an AST of chained operations) per output
   buffer; it does **not** touch the GPU.
3. **GLSL codegen** — each buffer's `Node` tree is compiled to a
   `mainImage(out vec4 c, in vec2 st)` GLSL function (`compile_node` in
   `eval.rs`), wrapped in a fixed preamble/uniform block (`shader.rs`) and the
   GLSL function library (`library.glsl`, generated into `src/glsl.rs` from
   `build.rs`). The caller (`ShaderRenderer` / the `hydra` binary) compiles and
   runs this shader via OpenGL 3.3 core.

A fresh `Engine` and `Scope` are created on **every** call to `eval()` — there
is no persistent script state across evaluations. Anything a script needs
must be declared within that same string.

## 2. Program structure

A chain is one source function followed by zero or more geometry/color/blend/
modulate operations, terminated (implicitly or explicitly) by `.out(bufIdx)`:

```
osc(60.0, 0.1, time * 0.5)
  .kaleid(4.0)
  .color(1.0, 0.5, 0.3)
  .modulate(noise(3.0, 0.1), 0.02)
  .rotate(0.0, 0.1)
  .out()
```

- **Exactly one source** must start a chain (`chain must start with a source`
  if missing); a chain may not contain a second source (`chain must have
  exactly one source`).
- **`.out()`** writes to buffer `o0`; **`.out(o1)`** (or any of `o0`-`o3`)
  targets that buffer explicitly.
- If a script's final expression is a bare `Node` (no explicit `.out(...)`)
  *and* no buffer has been written yet, it's written to `o0` implicitly.
- **`render()`** displays all 4 buffers in a 2x2 grid; **`render(n)`**
  displays only buffer `n`. Default display mode is buffer `0`.
- **`hush()`** clears all four buffers and resets `o0` to solid black.
- Multiple statements are just Rhai statements, one per chain (see §4.4 for
  how missing `;` between them is handled).
- **Nesting limit:** a chain passed as another chain's "other" operand
  (`.modulate(otherChain, ...)`) recurses through `compile_node`; total
  recursion depth is capped at 16 (`MAX_DEPTH`), erroring `nesting too deep
  (max 16)` beyond that.

## 3. Reactive values

These identifiers are pushed into every script's `Scope` before evaluation.
Referencing one splices the given GLSL expression directly into the compiled
shader — they are **not** callback functions like in JS; there's no need to
wrap them in `() => ...` (see §4.6, arrow-stripping, for what happens if a
script does anyway).

| Identifier | GLSL expression | Notes |
|---|---|---|
| `time` | `iTime` | **Reassignable** (`time = 0`), unlike the others below. Reassignment only rebinds the local script variable for the rest of *this* evaluation — the live `iTime` uniform keeps incrementing regardless. |
| `beat` | `iBeat` | Constant. Computed by the host as `time * (tempo / 60)`. |
| `tempo` | `iTempo` | Constant. Host-controlled (BPM), not settable from a script. |
| `phase` | `iPhase` | Constant. Host-controlled. |
| `mouseX`, `mouseY` | `iMouse.x`, `iMouse.y` | Constants, normalized `[0, 1]`. |
| `mouse.x`, `mouse.y` | `iMouse.x`, `iMouse.y` | Same values, object-property form matching real hydra.js. `mouse` itself is a marker value, not reassignable in any useful way. |
| `width`, `height` | `iResolution.x`, `iResolution.y` | Constants — canvas resolution in pixels. |
| `window.innerWidth`, `window.innerHeight` | `iResolution.x`, `iResolution.y` | Same values, DOM-style form some sketches use instead. |
| `a.fft[i]` | `iFft[i]` | **`audio` feature only.** `i` may be an integer literal (clamped to `[0, NUM_FFT_BINS-1]`, currently 8 bins) or itself a reactive `GlslExpr`, in which case the index becomes `int(mod(expr, 8.0))` in GLSL. |

Arithmetic between these values and plain numbers works via operator
overloads registered on the underlying `GlslExpr` wrapper type (`+ - * / %`,
unary `-`), e.g. `time * 0.5 + 1` is valid and produces `(((iTime * 0.5)) +
1.0)`. `%` compiles to GLSL's `mod()` (GLSL's native `%` is integer-only;
everything here is float-typed).

### Math functions

`sin cos tan asin acos atan abs fract floor ceil sqrt sign exp log` are
registered as free functions. Each has two families of overloads:

- Taking a `GlslExpr` (or a plain number combined with one via the operators
  above) → returns a `GlslExpr` wrapping the equivalent GLSL call, e.g.
  `sin(time)` → GLSL `sin(iTime)`.
- Taking a plain `f64`/`i64` → evaluated immediately in Rust, returns a plain
  number (e.g. `sin(4)` → `-0.7568...`, a static value baked into the shader
  as a literal, not a live expression).

`pow`, `min`, `max` are two-argument versions of the same pattern; `atan`
additionally has a two-argument GLSL overload (`atan(y, x)`, matching JS's
`Math.atan2`).

## 4. The JS-compatibility layer

Real hydra.js sketches are JavaScript; hydra-rust's DSL is Rhai. Rather than
implementing a JS parser, `eval()` runs the source through a fixed pipeline
of small, targeted textual rewrites (`src/eval.rs::eval`, in this order) that
turn the common JS idioms found in real sketches into valid Rhai, before any
of it is parsed. Each pass is its own module with its own unit tests; all are
string/comment-aware (via the shared scanner in `src/srcscan.rs`, which masks
out `"..."`, `'...'`, `` `...` `` string bodies and `//`/`/* */` comments so
none of the passes below ever rewrite something inside a string or comment).

Pipeline order (each step's output feeds the next):

1. **`whitespace::normalize_whitespace`** — real sketches, often copy-pasted
   from a web page or word processor, sometimes contain non-ASCII Unicode
   whitespace (non-breaking spaces, en/em spaces, ideographic space, ...).
   Rust's `char::is_whitespace()` (used throughout the other passes here)
   already treats these as whitespace, but Rhai's lexer only recognizes
   ASCII space/tab/CR/LF — anything else in code position is a hard
   "Unexpected '<char>'" lex error, even though the character is visually
   indistinguishable from a normal space. Maps every such character to a
   plain ASCII space. Runs first, unconditionally (no string/comment
   masking needed): even inside a string, one of these renders identically
   to a regular space, so there's no reason to preserve the distinction.
2. **`quotes::rewrite_single_quoted_strings`** — JS treats `'...'` and
   `"..."` identically as general strings. Rhai's `'x'` syntax is a
   *character* literal (exactly one character); any real multi-character
   JS string written with single quotes (`.ease('sin')`) would otherwise fail
   to lex at all. Rewrites `'...'` → `"..."` (unescaping `\'`, escaping
   embedded `"`). Runs early because every later pass's string/comment
   masking depends on a consistent double-quoted view of the source.
3. **`numlit::insert_leading_zero`** — JS allows a decimal point with no
   leading digit (`.1`); Rhai's number grammar requires `0.1`. Inserts a `0`
   before any `.` immediately followed by a digit, unless the character
   immediately before it is itself a digit (so `1.5` is untouched).
4. **`jskeywords::rewrite_keywords`** — `var`/`null`/`new`/`await`/`async`/
   `import` are all Rhai-*reserved* keywords with no meaning registered to
   them; any bare appearance is already an unconditional hard syntax error,
   so substituting them can only help or be neutral. `var` → `let` (same
   declaration semantics here); `null` → `()` (Rhai's unit value); `new`,
   `await`, `async`, `import` are dropped entirely (`new Foo()` becomes a
   plain `Foo()` call — still fails gracefully with "Function not found" if
   `Foo` isn't registered, rather than a hard parse stop). Also rewrites JS
   strict (in)equality `===`/`!==` ("not a valid operator... Should it be
   '=='?") to `==`/`!=` — Rhai's equality already compares by value and
   type here.
5. **`jsfunctions::rewrite_function_decls`** — rewrites JS *named* function
   declarations (`function name(a, b=1) { ... }`) into Rhai's own,
   similarly-shaped function syntax (`fn name(a, b) { ... }` — spelled
   `fn`, and without default parameter values). Real sketches sometimes
   define small helpers this way (easing curves, custom math). Anonymous
   `function(...) { ... }` expressions are left alone, since they're often
   used as closures capturing outer-scope variables, which Rhai's
   `fn`-defined functions can't do.
6. **`iife::unwrap_iife`** — real sketches sometimes wrap their entire body
   in an immediately-invoked function expression to load an extension
   library first: `(() => { BODY })()` (`async`/`await` already stripped
   by step 3), optionally followed by a promise `.then(...)`/`.catch(...)`/
   `.finally(...)` handler chain. Rhai has no `=>` closure syntax at all, so
   this was a hard parse failure. Matches only the *zero-parameter* form
   (`(hydra) => {...}` is left alone, since unwrapping would leave `hydra`
   unbound in the body) and reduces the whole construct — handler chain
   included — to a bare Rhai block `{ BODY }`, evaluated as a normal
   statement sequence.
7. **`autolet::insert_missing_let`** — JS creates a variable implicitly on
   first assignment (`speed = 0.8`); Rhai requires `let`. Inserts `let `
   before the first bare assignment to any name not already known (built-in
   globals from §3, or a name this same pass already declared earlier in the
   script). Only fires at paren/bracket depth 0 — `let` is a statement, and
   isn't valid inside a function call's argument list or an array literal,
   where JS allows assignment-as-expression (`foo(x = 5)`).
8. **`mathjs::rewrite_math`** — rewrites `Math.<method>(...)` to the bare
   function name for every method with a registered equivalent (§3's math
   function list, plus `atan2`→`atan`), and `Math.PI` to a numeric literal.
   `Math.random()` is deliberately left untouched — there's no
   GLSL-expression equivalent for true randomness.
9. **`argtrunc::truncate_extra_args`** — JS silently ignores extra arguments
   beyond a function's declared parameters; Rhai has no such leniency and
   errors "Function not found" if no overload matches the arity. Truncates
   each known hydra function's call-site argument list down to its real
   maximum (bracket/string-aware, and recursive so nested calls are
   truncated too). The per-function max-argument table lives in
   `argtrunc.rs::MAX_ARGS`; for blend/modulate-kind functions it counts the
   leading "other" operand as one of the arguments (e.g.
   `modulate(other, amount)` → max 2).
10. **`patcall::rewrite_pattern_calls`** — real sketches often store a reusable
   value as `pat = ()=>expr` and invoke it later as `pat()`, JS-callback
   style. Since step 7 reduces such definitions to a plain value (not a
   callable), a later zero-argument call to a name bound this way is
   rewritten to a bare reference (`pat()` → `pat`). Must run before arrow-
   stripping, since it needs to see the `()=>` marker to know which names
   qualify. Only the exact zero-arg-arrow-then-zero-arg-call shape is
   rewritten; a call with any arguments is left alone.
11. **`arrow::strip_zero_arg_arrows`** — strips the `()=>` wrapper JS uses to
   mark a value as per-frame-dynamic (`rotate(()=>time*0.1)` →
   `rotate(time*0.1)`). This is safe here because hydra-rust's reactive
   values (§3) are already "dynamic" without a wrapper — they compile
   straight into live GLSL uniform expressions. Also strips the
   destructured-parameter form real hydra.js per-frame callbacks use
   (`invert(({time})=>Math.sin(time)*3)` → `invert(Math.sin(time)*3)`,
   likewise for multiple properties, `({time,mouse})=>...`) — the
   destructured names are simply dropped, since they already resolve
   correctly in the body whenever they match one of hydra-rust's own
   globals. Deliberately **not** rewritten: multi-param or bare-identifier
   arrows (`(a,b)=>...`, `x=>...`, used for pattern/sequencer callbacks —
   unsupported) and block-bodied arrows (`()=>{ ... }` — don't reduce to a
   single expression).
12. **`ternary::rewrite_ternaries`** — rewrites JS ternaries (`cond ? a : b`)
   into Rhai's `if`/`else` expression form (`if cond { a } else { b }`,
   valid since Rhai's `if`/`else` blocks evaluate to their last statement's
   value). Rhai has no `?:` operator at all ("Unknown operator: '?'").
   Boundaries are found structurally — a top-level `,`/`;`/bare `=` marks
   where a branch starts or ends, and brackets are recursed into — rather
   than via full expression-grammar parsing; nested/chained ternaries
   (`a?b:c?d:e`, right-associative) are handled via recursion on the
   extracted branches.
13. **`asi::insert_missing_semicolons`** — JS has automatic semicolon
   insertion; Rhai doesn't. Real multi-buffer sketches routinely put each
   statement on its own line with no `;` (`osc(10).out(o0)\nosc(20).out(o1)`).
   Inserts `;` at line breaks that are genuine statement boundaries (bracket
   depth 0, and neither the end of the current line nor the start of the
   next one looks like a continuation — an operator, a trailing comma/open
   bracket, or a leading `.`/closing bracket/operator on the next line).
   Runs last so it sees the final structural shape of the code.

None of these passes attempt full JS parsing; each targets one specific,
empirically-observed idiom (found by running the pipeline against a corpus
of ~46,600 real, publicly-shared hydra sketches — see `examples/check_corpus.rs`)
and is scoped narrowly enough to avoid false positives on the surrounding
code. What's still unhandled — `await`/`async`/`new`/`var`/`null` (reserved
Rhai keywords colliding with real JS syntax, mostly from module-loading code
outside the scope of a visual patch DSL), `function(...) {...}` expression
syntax (same "block body doesn't reduce to an expression" limitation as
multi-statement arrows), and `a.settings[i].cutoff = ...` (a bigger,
not-yet-designed indexable-config feature) — is left as syntax or semantic
errors from `eval()`, surfaced to the caller as `Err(String)`.

## 5. Buffers, sources, and text

- **`o0`-`o3`** (Rhai constants `0`-`3`): the four output buffers, each
  double-buffered (ping-pong) so a buffer can read its own previous frame.
- **`s0`-`s3`** (Rhai constants `100`-`103`): four external source slots,
  populated by camera capture (`initCam`, behind the `webcam` feature).
- **`src(idx)`**: reads a buffer or source as a chain-starting `Node`. `idx <
  100` reads buffer `idx`'s previous frame (`texture(iBufferN, st)`); `idx >=
  100` reads external source slot `idx - 100` (`texture(iSourceN, ...)`,
  vertically flipped and alpha forced to `1.0`, since camera frames are RGB).
  Any of `o0`-`o3`/`s0`-`s3` used **directly** as the second argument to a
  blend/modulate function (`.modulate(s0, 0.5)`) is implicitly converted the
  same way — real hydra.js treats these as first-class chainable objects;
  here it's a convenience conversion (`as_node` in `eval.rs`) rather than a
  real object model.
- **`text("...")`**: rasterizes a string (via the bundled Hack font,
  `src/text.rs`) into a single shared text texture (`iText0`) and returns a
  chain reading it. Only one `text()` call's content is visible at a time
  (last one wins within an evaluation).
- **`initCam(slot)` / `initCam(slot, cameraIndex)`** (`webcam` feature only):
  requests slot `slot` (must be `s0`-`s3`, i.e. `>= 100`) be filled from
  camera `cameraIndex` (default `0`). This only queues a request in the
  returned `EvalResult`; actual capture happens on the host side
  (`SourceManager`), asynchronously, outside of `eval()`.

## 6. Function reference

Every entry below is a Rhai function taking a chain (or, for sources, no
chain) plus the listed parameters in order; trailing parameters may be
omitted and take their default. Parameter values may be plain numbers, a
reactive expression (§3), or a pattern (§7). GLSL semantics are in
`library.glsl`; only a short description is given here.

### Sources (start a chain)

| Function | Parameters (defaults) | Description |
|---|---|---|
| `osc(frequency=60, sync=0.1, offset=0)` | | RGB sine oscillator, offset per channel |
| `noise(scale=10, offset=0.1)` | | 3D simplex noise, animated via `offset * iTime` |
| `voronoi(scale=5, speed=0.3, blending=0.3)` | | Animated voronoi cell pattern |
| `shape(sides=3, radius=0.3, smoothing=0.01)` | | Regular polygon / circle |
| `gradient(speed=0)` | | Diagonal gradient, blue channel animated |
| `solid(r=0, g=0, b=0, a=1)` | | Flat color |
| `rings(freq=8, speed=0.1)` | | Concentric animated rings |
| `checker(cols=4, rows=4)` | | Checkerboard |
| `src(idx)` | | Read buffer/source `idx` (see §5) |
| `text("...")` | | Rasterized text (see §5) |

### Geometry (transform `st` before the source samples it)

| Function | Parameters (defaults) | Description |
|---|---|---|
| `rotate(angle=10, speed=0)` | | Rotate around center, optionally spinning over time |
| `scale(amount=1.5, xMult=1, yMult=1, offsetX=0.5, offsetY=0.5)` | | Scale around a pivot point |
| `scroll(scrollX=0.5, scrollY=0.5, speedX=0, speedY=0)` | | Offset + animated scroll, wraps |
| `scrollX(scroll=0.5, speed=0)` / `scrollY(...)` | | Single-axis scroll |
| `kaleid(sides=4)` | | Kaleidoscope mirror-fold |
| `pixelate(pixelX=20, pixelY=20)` | | Snap to a coarse pixel grid |
| `repeat(repeatX=3, repeatY=3, offsetX=0, offsetY=0)` | | Tile with per-row/column offset |
| `repeatX(reps=3, offset=0)` / `repeatY(...)` | | Single-axis tiling |
| `polar()` | | Cartesian → polar coordinates |
| `cart()` | | Polar → Cartesian coordinates |
| `fold(amount=1)` | | Mirror-fold coordinates |

### Color

| Function | Parameters (defaults) | Description |
|---|---|---|
| `color(r=1, g=1, b=1, a=1)` | | Multiply/recolor (sign-dependent blend with input) |
| `invert(amount=1)` | | Invert toward `1 - color` |
| `contrast(amount=1.6)` | | Push away from/toward mid-gray |
| `brightness(amount=0.4)` | | Additive brightness |
| `saturate(amount=2)` | | Scale distance from luminance |
| `hue(amount=0.4)` | | Rotate hue (via HSV round-trip) |
| `posterize(bins=3, gamma=0.6)` | | Quantize levels |
| `luma(threshold=0.5, tolerance=0.1)` | | Luminance-keyed alpha |
| `colorama(amount=0.005)` | | Cycle hue, wrap channels |
| `shift(r=0.5, g=0, b=0, a=0)` | | Per-channel fractional shift |
| `thresh(threshold=0.5, tolerance=0.04)` | | Hard luminance threshold |
| `r(scale=1, offset=0)` / `g(...)` / `b(...)` | | Isolate + scale one channel (broadcast to all 4 output channels) |
| `a(scale=1, offset=0)` | | Isolate + scale the alpha channel (broadcast) |
| `sum(scaleR=1, scaleG=1, scaleB=1, scaleA=1)` | | Weighted grayscale sum of channels, alpha preserved |

### Blend (combine with another chain)

| Function | Parameters (defaults) | Description |
|---|---|---|
| `add(other, amount=1)` | | Additive blend |
| `mult(other, amount=1)` | | Multiplicative blend |
| `blend(other, amount=0.5)` | | Linear cross-fade |
| `diff(other)` | | Absolute difference |
| `layer(other)` | | Alpha-composite `other` over the chain |
| `mask(other)` | | Use `other`'s luminance as an alpha mask |
| `sub(other, amount=1)` | | Subtractive blend |

### Modulate (use another chain's color to distort coordinates)

| Function | Parameters (defaults) | Description |
|---|---|---|
| `modulate(other, amount=0.1)` | | Offset `st` by `other`'s RG channels |
| `modulateScale(other, multiple=1, offset=1)` | | Scale by `other`'s RG channels |
| `modulateRotate(other, multiple=1, offset=0)` | | Rotate by `other`'s red channel |
| `modulateRepeat(other, repeatX=3, repeatY=3, offsetX=0.5, offsetY=0.5)` | | Tile with `other`-driven offset |
| `modulateRepeatX(other, reps=3, offset=0.5)` / `modulateRepeatY(...)` | | Single-axis version |
| `modulateKaleid(other, sides=4)` | | Kaleidoscope with `other`-driven radius |
| `modulateScrollX(other, scroll=0.5, speed=0)` / `modulateScrollY(...)` | | Scroll driven by `other`'s red channel |
| `modulatePixelate(other, multiple=10, offset=3)` | | Pixelation grid driven by `other` |
| `modulateHue(other, amount=1)` | | Hue-shift-like distortion from `other`'s G/B difference |

### Other

| Function | Description |
|---|---|
| `out()` / `out(bufIdx)` | Write chain to buffer `o0`, or `bufIdx` (see §2) |
| `render()` / `render(bufIdx)` | Display mode (see §2) |
| `hush()` | Clear all buffers (see §2) |

## 7. Patterns

An array literal (`[1, 2, 3]`) can be used almost anywhere a plain number
is expected. It compiles to a `mod(iTime * speed, N)`-indexed step function
in GLSL that cycles through the values once per unit of `iTime`, one value
per `1.0`-wide step (see `Pattern::to_glsl` in `eval.rs`).

| Modifier | Effect |
|---|---|
| `.fast(speed)` | Multiplies the cycle rate (default `1`) |
| `.smooth()` / `.smooth(amount)` | Cross-fades between consecutive values instead of stepping. **`amount` is accepted but not faithful** — any value just turns smoothing fully on; there's no partial-interpolation curve. |
| `.offset(amount)` | Shifts the pattern's phase |
| `.ease(name)` | **Stub.** Accepted (any curve name), but the pattern passes through unchanged — no easing-curve implementations exist. |
| `.fit(lo, hi)` | **Stub.** Accepted, but the pattern passes through unchanged — no value-range remapping exists. |

`.reverse()` (Rhai's own array method, overridden here) returns the
reversed array for chaining (`[0,1].reverse().smooth()`), matching JS's
`Array.prototype.reverse()` rather than Rhai's built-in (which mutates in
place and returns nothing).

## 8. Feature flags

Two Cargo features gate optional hardware/dependency-heavy functionality,
both off by default:

| Feature | Deps | Enables |
|---|---|---|
| `webcam` | `nokhwa` | `initCam(slot[, cameraIndex])`, populating `s0`-`s3` from a physical camera |
| `audio` | `cpal`, `rustfft` | `a.fft[i]`, `a.setBins(n)`, `a.setCutoff(c)`, `a.setScale(s)`, `a.setSmooth(s)`, `a.show()`/`a.hide()` (no-op) |

Calling a feature-gated function in a build without that feature produces a
plain "Function not found" error from `eval()` — there is no separate
"feature not compiled in" error path.

## 9. Known gaps and stub functions

These are registered (so a script calling them doesn't hard-error) but do
**not** do anything real:

| Function | Status |
|---|---|
| `initImage(idx, url)` | No-op — no image loading/decoding pipeline |
| `initVideo(idx, url)` | No-op — no video file loading pipeline |
| `initScreen(idx[, screen])` | No-op — no screen/display capture |
| `setResolution(w, h)` | No-op — canvas resolution isn't script-controllable |
| `screencap()` | No-op — saving/sharing a screenshot isn't supported |
| `a.show()` / `a.hide()` | No-op — no on-screen FFT debug graph to toggle |
| `smooth(amount)` (pattern) | See §7 |
| `ease(name)` / `fit(lo, hi)` (pattern) | See §7 |
| `Math.random()` | Not rewritten — no GLSL-expression equivalent for real randomness |

Not registered at all, and not silently tolerated: `a.settings[i].cutoff =
...` (real hydra.js exposes indexable, mutable per-bin audio config; this
would need a materially different, indexable settings type — bigger than
the flat `setBins`/`setCutoff`/etc. setters here), JS ternaries, multi-param
or block-bodied arrow functions, `var`/`new`/`async`/`await`/`null` as used
in JS module-loading or class-based code.

## 10. Standalone binary

`cargo run --bin hydra [-- <path-to-sketch.hydra>]` opens the editor with
that file loaded (falling back to the previous session's sketch, with a
logged warning, if the path can't be read). Patch evaluation errors from
`eval()` are shown as an in-app toast and also logged via `log::error!`
("patch eval error: ..."), so `RUST_LOG=error` (or lower) surfaces them on
the console for scripted workflows. See `README.md` for the full list of
editor keybindings and build instructions.
