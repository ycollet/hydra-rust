# Hydra development ideas and tasks

Potential additions inspired by [HydraPy](https://github.com/JosiahAlanLee/hydrapy),
compared with standard browser-based [Hydra](https://github.com/hydra-synth/hydra-synth).
This is an inspiration list, not an audit of which features hydra-rust already supports
or a committed implementation roadmap.

## Highest-interest candidates

- [ ] **Per-pixel parameters:** let textures drive ordinary float parameters, not
  just dedicated `modulate*` operators. For example, `osc(noise(2) * 40)` produces
  spatially varying frequency.
- [ ] **Automatic multipass effects:** introduce intermediate rendering passes for
  neighbourhood operations such as blur, sharpen, Sobel edges, emboss,
  dilation/erosion and GPU pixel sorting.
- [ ] **Multi-frame delays:** retain output history in ring buffers for temporal
  echoes, frame holds, motion extraction and delayed feedback.
- [ ] **Image-to-audio/CV feedback:** read image strips as audio waveforms and image
  regions as control signals, allowing visuals to drive sound and modular hardware.

## Additional ideas

- [ ] **Modulation toolkit:** reusable LFO waveforms, attack/decay envelopes,
  sample-and-hold, smooth random drift, clocks and composable signal arithmetic.
  Standard Hydra can approximate much of this with JavaScript callbacks; HydraPy
  supplies dedicated building blocks.
- [ ] **Feedback simulations:** Gray-Scott reaction-diffusion, Life-like cellular
  automata, Physarum/slime-mould agents and neural cellular automata that grow and
  repair learned textures.
- [ ] **Paint and print tools:** transparent-dye compositing, ink bleeding, brush
  dabs, charcoal, paper textures, mathematical marbling, halftone printing and
  image quilting.
- [ ] **Analog-video and film effects:** Rutt-Etra-style scan-line displacement,
  composite-video colour bleed, VHS tracking errors, tape dropouts, film-stock
  grading, projector weave, scratches and frame-rate holds.
- [ ] **Procedural 3D and materials:** texture mapping onto planes, spheres and
  cylinders; composable 3D signed-distance fields and ray marching; recursive
  fractal folds; normal maps, relighting, environment mapping and normal-driven
  refraction-like distortion.
- [ ] **Hyperbolic geometry:** hyperbolic kaleidoscopes and transformations for
  Poincare-disk or band-model tilings.
- [ ] **Vector output:** XY path synthesis for oscilloscope-style output and
  path export for plotters.
- [ ] **Hardware integration:** integrated CV input/output through DC-coupled
  interfaces, OSC input/output, MIDI controls and clock, Ableton Link, and
  JACK/PipeWire routing.
- [ ] **Audiovisual tracker:** sequence visual parameters and musical/CV lanes
  from one score, with VCV Rack integration and Program/Auxiliary displays,
  following HydraPy's Gorgon approach.
- [ ] **Offline production:** CPU and desktop GLSL rendering, headless rendering,
  frame-accurate file-audio analysis, optical flow and video
  export with hardware encoding.
- [x] **Live beat/onset detection:** shared desktop/plugin analysis exposes
  `a.onset`, estimated beat phase `a.beat` and estimated tempo `a.bpm`.
  This is a basic regular-onset tempo estimate, not musical downbeat detection.
- [ ] **Standalone WebGL export:** export supported extended operators without
  depending on the upstream Hydra runtime.
- [ ] **Performance workspace:** named parameter sliders, modulation controls,
  presets, output previews, export jobs and hot reload that preserves feedback
  history.
- [ ] **Timeline automation:** keyed parameter tracks in seconds or beats.

## Caveats

- HydraPy is not a drop-in superset of Hydra. Its compatibility guide lists
  missing upstream features such as `initScreen` and `setFunction`.
- Backend support varies. Some Python-computed sources need baking for HTML
  export, and live hardware connections do not carry into standalone HTML.
- Gorgon is in development. Physical hardware paths and end-to-end A/V latency
  still require workstation validation according to the project's README.
- These are additions provided by HydraPy itself; some can also be assembled
  in standard Hydra using JavaScript, external libraries or extensions.

## References

- [Compatibility](https://github.com/JosiahAlanLee/hydrapy/blob/main/docs/getting-started.md#whats-different-from-hydra)
- [Signals, feedback and multipass architecture](https://github.com/JosiahAlanLee/hydrapy/blob/main/docs/patching.md)
- [Operator reference](https://github.com/JosiahAlanLee/hydrapy/blob/main/docs/operators.md)
- [Simulations, vectors and analysis](https://github.com/JosiahAlanLee/hydrapy/blob/main/docs/ecosystem.md)
- [Film, print and drawn sound](https://github.com/JosiahAlanLee/hydrapy/blob/main/docs/film-and-sound.md)
- [3D and material tools](https://github.com/JosiahAlanLee/hydrapy/blob/main/docs/DEMOSCENE.md)
- [Hardware integration](https://github.com/JosiahAlanLee/hydrapy/blob/main/docs/live-io.md)
- [Rendering and export](https://github.com/JosiahAlanLee/hydrapy/blob/main/docs/rendering-and-export.md)
- [Studio](https://github.com/JosiahAlanLee/hydrapy/blob/main/docs/studio.md)
- [Gorgon overview](https://github.com/JosiahAlanLee/hydrapy#gorgon-a-cv-and-av-tracker)

## JavaScript support

These tasks improve the optional JavaScript backend while keeping native rendering and local library loading.
See the [Hyper-Hydra compatibility table](DOCUMENTATION.md#hyper-hydra-compatibility) for current blockers.
Remote loading and full browser emulation remain outside this task list.
JavaScript execution in the plugin also remains outside this scope.

### Hyper-Hydra compatibility priorities

- [x] **Shader identifiers:** accept underscore-prefixed names while generating safe shader identifiers.
  Example: register `_glsl_ext_NODE_0` from `hydra-glsl.js`.
- [x] **Shader parameter aliases:** avoid redeclarations of `st`, `c0`, and `c1` in extension snippets.
  Example: compile `.glslCoord("uv * 2.0")` after loading `hydra-glsl.js`.
- [x] **Large shader registries:** support more registered functions while preserving memory and compilation limits.
  Example: load `hydra-swizzle.js` without exceeding the current 256-definition limit.
- [x] **Unused shader definitions:** compile only custom functions needed by the current graphs.
  Example: an unused, incompatible `phoenix` shader should not block other `hydra-blend.js` modes.
- [x] **Texture sampler inputs:** bind `sampler2D` parameters to native output and source textures.
  Example: register a convolution from `hydra-convolutions.js` with `o0` as its texture input.
- [x] **Graph metadata adapters:** support extension helpers that inspect or copy browser Hydra graph metadata.
  Example: adapt `hydra-colorspaces.js` helpers that copy `transforms`.
- [x] **Compatibility checks:** exercise representative helpers and compile their generated shaders.
  Example: check `hydra-abbreviations.js` with `osc(10).bright(0.2).out()` and check vector helpers from `hydra-vec4.js`.
- [x] **Compatibility tracking:** record checked upstream revisions and update the documentation after each supported extension.
  Example: move `hydra-glsl.js` from blocked to verified examples only after its helpers execute and compile.

### Native feature parity

- [x] **Native shader helpers:** expose existing native helpers through JavaScript rather than duplicating their implementations.
  Example: support `osc(10).tileable(0.15).out()` and native `text("Hydra")`.
- [x] **Source initialization:** bridge existing source managers without introducing browser camera or canvas objects.
  Example: support `s0.initCam()` when the application includes the `webcam` feature.
- [x] **Device controls:** expose native audio setters, Musical Instrument Digital Interface controls, and Open Sound Control controls.
  Example: support `a.setBins(4)` when the application includes the `audio` feature.
- [x] **Native buffer operations:** adapt selected framebuffer helpers without exposing browser graphics internals.
  Example: provide native `o0.clear()` for an adapted subset of `hydra-outputs.js`.

### Runtime and editing improvements

- [x] **Interpreter recovery:** add an explicit reset action with clear consequences for JavaScript variables and libraries.
  Example: recover from heap exhaustion without restarting the application.
- [x] **Library diagnostics:** provide native console logging and errors containing library paths and line numbers.
  Example: report `libraries/helpers.js:12` when a library throws an error.
- [x] **Scene languages:** store the scripting language alongside each saved scene.
  Example: recall a Rhai scene while JavaScript mode is selected without interpreting it as JavaScript.
- [x] **Local modules:** support local module imports under the existing library-root restrictions.
  Example: load `import { pulse } from "./helpers.js";` without allowing network access.
- [x] **Runtime settings:** expose bounded execution, memory, and library-size settings without removing protective limits.
  Example: adjust the script timeout while retaining memory and pending-job limits.
