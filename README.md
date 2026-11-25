# hydra-rust

A native Rust port of [Hydra](https://hydra.ojack.xyz/), the live-codable video
synthesizer created by [Olivia Jack](https://ojack.xyz/).
It compiles Rhai scripts to GLSL and renders them through OpenGL 3.3.
An optional JavaScript interpreter supports local libraries and live callbacks.

Use it as a standalone live-coding application, embed its GUI-independent
library, or build its VST3/CLAP plugin. Features include four feedback buffers,
text rendering, scene banks, and optional audio/MIDI/OSC reactivity, webcam,
image, video, and streaming inputs. Originally extracted from
[Sova](https://github.com/Bubobubobubobubo/Sova).

## Quick start

Download an installer from [GitHub Releases](https://github.com/sova-org/hydra-rust/releases),
or build and run from source:

```bash
cargo run
```

Paste a sketch into the editor and press **Ctrl/Cmd + Shift + Enter**:

```javascript
osc(60, 0.1, 0)
  .kaleid(4)
  .color(1, 0.5, 0.3)
  .out()
```

Device, network, and JavaScript features are opt-in when building from source.

## Documentation

- [User guide and function reference](DOCUMENTATION.md): installation, JavaScript,
  shortcuts, scenes, sessions, optional features, embedding, and limitations.
- [Language specification](SPEC.md): scripting syntax, reactive values, and compatibility.
- [Plugin guide](crates/hydra-plugin/README.md): VST3/CLAP and plugin standalone usage.
- [License: AGPL-3.0-or-later](https://www.gnu.org/licenses/agpl-3.0.html)
