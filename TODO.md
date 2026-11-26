# Hydra development ideas and tasks

Potential additions inspired by [HydraPy](https://github.com/JosiahAlanLee/hydrapy),
compared with standard browser-based [Hydra](https://github.com/hydra-synth/hydra-synth).
This is an inspiration list, not an audit of which features hydra-rust already supports
or a committed implementation roadmap.

## Highest-interest candidates

- [x] **Per-pixel parameters:** textures can drive ordinary float parameters through their red channel.
  Example: use `osc(noise(2) * 40)` in Rhai or `osc(noise(2).mul(40))` in JavaScript.
- [x] **Automatic multipass effects:** support blur, sharpen, Sobel edges, emboss, dilation, and erosion.
  Example: `osc(10).blur(1).out()` uses intermediate native rendering passes.
- [x] **Multi-frame delays:** allocate output history on demand, with eight frames maximum per output.
  Example: `history(o0,4).blend(osc(10),0.1).out()` creates delayed feedback.
- [x] **Image-region controls:** sample normalized regions and optionally send their intensity through Open Sound Control (OSC).
  Example: `sendRegion(region(o0),"127.0.0.1:9000","/brightness",30)` sends a JavaScript region signal.
- [ ] **GPU pixel sorting:** add a separately bounded multipass sorting effect.
- [ ] **Image-to-audio feedback:** convert image strips into audio waveforms.
- [ ] **Direct control-voltage output:** connect region signals to suitable hardware through an explicit native device backend.

These extensions preserve ordinary Hydra sketches but require hydra-rust when used.
See [native rendering extensions](DOCUMENTATION.md#native-rendering-extensions) for limits and examples.

## Remaining performance work

The first five performance priorities are implemented.
These tasks cover remaining audit findings and follow-up optimizations.
Preserve ordinary Hydra visuals, feedback behavior, and input timing.
See [runtime and resource limits](DOCUMENTATION.md#runtime-and-resource-limits) for the completed protections.

### Higher-priority improvements

- [x] **Shader work budgets:** estimate repeated shader work and reject excessive nesting before graphics compilation.
  Limit each pass to 256 estimated source evaluations without introducing precision-changing intermediate textures.
  Example: bound the cost of `noise(2).tileable(0.15).tileable(0.15).out()`.
- [x] **Background evaluation:** move processor-side script evaluation and preprocessing off the interface thread.
  Cancel superseded requests, reject stale results, and keep graphics operations on a valid context.
  Example: editing a slow sketch should not block keyboard input or replace a newer successful graph.
- [x] **Broadcast workers:** isolate blocking encoder writes and connection handshakes from asynchronous runtime workers.
  Bound each viewer queue to 256 packets and disconnect slow viewers instead of dropping encoded packets.
  Limit handshakes to four concurrent workers with ten-second deadlines.
  Example: a slow viewer should not delay other viewers or new connections.
- [x] **Asynchronous region readback:** queue graphics readbacks without explicit completion waits.
  Three pending buffers bound work, and controls retain the latest completed sample.
  Asynchronous mode remains opt-in because it adds frame latency.
  Example: `setRegionSampling(0,true)` samples every frame without waiting for unfinished transfers.
- [x] **Region sampling frequency:** configure sampling independently of rendering and Open Sound Control transmission.
  Example: `setRegionSampling(10,false)` samples ten times per second while rendering continues.
- [x] **Effect pass compaction:** remove copy-only passes between consecutive effects.
  Preserve existing color rounding, feedback, and output ordering.
  Example: four chained `.blur(1)` calls should avoid copying each intermediate result into another identical texture.

### Smaller optimizations

- [x] **Audio analysis buffers:** reuse frequency-analysis workspace and buffers for converting samples into intensity values.
  Reuse unchanged analysis results while preserving smoothing, settings changes, and sample-based beat timing.
  Example: polling an idle audio buffer should not allocate temporary analysis arrays.
- [x] **Filtered MIDI polling:** cache stable Musical Instrument Digital Interface (MIDI) binding matches instead of repeatedly scanning every channel.
  Invalidate cached matches when bindings or input metadata change.
  Example: compare polling costs for sixty-four wildcard bindings across one and eight inputs.
- [x] **Rhai preprocessing:** add fast paths for unchanged syntax and reduce temporary strings.
  Example: preprocess a 22-kilobyte sketch without creating thousands of short-lived string copies.
- [x] **Animated frame handoff:** avoid cloning full decoded Graphics Interchange Format (GIF) frames whenever the animation advances.
  Keep existing source-loading interfaces and upload only changed frames.
  Example: reuse decoded pixel storage when uploading the next full-resolution animation frame.
- [x] **JavaScript frame bridge:** reduce temporary strings and arrays exchanged with the interpreter.
  Preserve callback errors, value validation, and existing frame behavior.
  Example: process 128 callbacks without repeatedly allocating serialization buffers.
- [x] **Syntax highlighting:** reuse regular-expression group names instead of creating strings for every token.
  Example: highlighting a thousand-line sketch should not rebuild the same group names for each match.

### Performance measurements

- [x] **Repeatable benchmarks:** record release-build timings and allocations for evaluation, graph replacement, analysis, and readback.
  Separate processor timings, forced graphics completion, and real application frame rates.
  Example: compare unchanged graph replacement with a change affecting only one intermediate pass.
  Run `cargo run --release --no-default-features --features fft,midi,javascript --example performance -- --gpu`.
  The graphics workloads currently require macOS; omit `--gpu` for portable processor measurements.
- [ ] **Representative hardware profiling:** measure plugin audio deadlines, interface stalls, camera capture, and simultaneous viewers.
  Include Linux display systems and real digital audio workstation hosts.
  Example: resize the plugin while processing audio and check for callback overruns.
  The profiling procedure is ready, but live host, camera, viewer, and Linux measurements remain pending.

See [performance measurements](DOCUMENTATION.md#performance-measurements) for commands, methodology, initial results, and the hardware checklist.

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
