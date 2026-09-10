pub mod audio;
pub mod eval;
pub mod renderer;
pub mod shader;
pub mod source;
mod argtrunc;
mod arrow;
mod asi;
mod autolet;
mod glsl;
mod iife;
mod jsfunctions;
mod jskeywords;
mod mathjs;
mod numlit;
mod patcall;
mod quotes;
mod srcscan;
mod ternary;
mod text;
mod whitespace;

pub use eval::{eval, EvalResult, RenderMode};
#[cfg(feature = "webcam")]
pub use eval::SourceRequest;
#[cfg(feature = "audio")]
pub use eval::AudioRequest;
pub use renderer::{render_multipass, RenderSnapshot, RenderUniforms, ShaderRenderer};
pub use source::SourceFrame;
#[cfg(feature = "webcam")]
pub use source::{CameraInfo, CameraStatus, SourceManager};
#[cfg(feature = "audio")]
pub use audio::AudioManager;
pub use text::TextData;
