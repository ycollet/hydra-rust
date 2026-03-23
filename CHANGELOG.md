# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Webcam support via `nokhwa`: `initCam(s0)` to capture, `src(s0)` to use as texture source
- 4 external source slots (`s0`–`s3`) for camera feeds
- Floating error toast that auto-dismisses after 5 seconds (no longer steals editor focus)
- Editor toggle with Ctrl+Shift+H

## [0.0.1] - 2026-03-23

### Added

- Hydra visual engine extracted from [Sova](https://github.com/sova-org/sova) as a standalone Rust library
- Rhai scripting → AST → GLSL codegen pipeline with 48 built-in functions (sources, geometry, color, blend, modulate)
- OpenGL multipass renderer with 4-buffer ping-pong architecture
- Text rasterization via bundled Hack font
- Standalone `hydra` binary with full-screen transparent editor over GL visuals
- Syntax highlighting (OneDark theme) for Hydra code
- Options sidebar (Tab to toggle): tempo, font size, text opacity, editor visibility
- File save/load (Ctrl+S / Ctrl+O) with `.hydra` extension and native OS file picker
- Session auto-persistence across launches (`~/.hydra-rust.json`)
