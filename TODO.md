# HydraPy feature ideas

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
