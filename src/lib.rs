pub mod eval;
pub mod renderer;
pub mod shader;
pub mod source;
mod glsl;
mod text;

pub use eval::{eval, EvalResult, RenderMode};
#[cfg(feature = "webcam")]
pub use eval::SourceRequest;
pub use renderer::{render_multipass, RenderSnapshot, RenderUniforms, ShaderRenderer};
pub use source::SourceFrame;
#[cfg(feature = "webcam")]
pub use source::{CameraInfo, CameraStatus, SourceManager};
pub use text::TextData;
