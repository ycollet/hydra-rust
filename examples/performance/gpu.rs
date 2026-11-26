use std::sync::Arc;

use glow::HasContext;
use hydra_rust::renderer::{
    RenderUniforms, ShaderRenderer, read_capture, read_regions, render_multipass,
};

use super::{Measurement, Options, black_box, measure, native_gl::NativeContext};

const SIZE: [u32; 2] = [1280, 720];

struct OutputTarget {
    gl: Arc<glow::Context>,
    texture: glow::Texture,
    framebuffer: glow::Framebuffer,
}

impl OutputTarget {
    fn new(gl: Arc<glow::Context>) -> Result<Self, String> {
        unsafe {
            let texture = gl.create_texture()?;
            let framebuffer = match gl.create_framebuffer() {
                Ok(framebuffer) => framebuffer,
                Err(error) => {
                    gl.delete_texture(texture);
                    return Err(error);
                }
            };
            let target = Self {
                gl,
                texture,
                framebuffer,
            };
            target.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            target.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                SIZE[0] as i32,
                SIZE[1] as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            target
                .gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            target.gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            if target.gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                return Err("Offscreen benchmark framebuffer is incomplete".into());
            }
            target.gl.viewport(0, 0, SIZE[0] as i32, SIZE[1] as i32);
            check(&target.gl)?;
            Ok(target)
        }
    }
}

impl Drop for OutputTarget {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_framebuffer(self.framebuffer);
            self.gl.delete_texture(self.texture);
        }
    }
}

fn check(gl: &glow::Context) -> Result<(), String> {
    let error = unsafe { gl.get_error() };
    if error == glow::NO_ERROR {
        Ok(())
    } else {
        Err(format!("OpenGL benchmark failed: {error:#x}"))
    }
}

fn compile(renderer: &mut ShaderRenderer, result: &hydra_rust::EvalResult) -> Result<(), String> {
    let errors = renderer.compile_result_atomic(result);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn uniforms() -> RenderUniforms {
    RenderUniforms {
        resolution: SIZE.map(|dimension| dimension as f32),
        time: 1.0,
        mouse: [0.0; 2],
        beat: 0.0,
        tempo: 120.0,
        phase: 0.0,
        fft: [0.0; 8],
        audio: Default::default(),
        region_values: [0.0; hydra_rust::extended::MAX_REGIONS],
        javascript_values: [0.0; 128],
        midi_note: [0.0; 128],
        midi_velocity: [0.0; 128],
        midi_cc: [0.0; 128],
        midi_cc_smoothed: [0.0; 128],
        midi_envelope: [0.0; 16],
        midi_aftertouch: [0.0; 128],
        midi_channel_aftertouch: 0.0,
        midi_values: [0.0; hydra_rust::midi::NUM_MIDI_VALUES],
        osc: [0.0; hydra_rust::osc::NUM_OSC_SLOTS],
    }
}

pub(super) fn run(
    results: &mut Vec<Measurement>,
    options: &Options,
) -> Result<serde_json::Value, String> {
    let context = NativeContext::new()?;
    let gl = context.gl_with_loader(|_| None);
    let mut renderer = ShaderRenderer::new(gl.clone());
    renderer.ensure_resolution_checked(SIZE[0], SIZE[1])?;
    let _output = OutputTarget::new(gl.clone())?;
    let metadata = unsafe {
        serde_json::json!({
            "vendor": gl.get_parameter_string(glow::VENDOR),
            "renderer": gl.get_parameter_string(glow::RENDERER),
            "version": gl.get_parameter_string(glow::VERSION),
            "resolution": SIZE,
            "context": "macOS offscreen CGL; not a window or application FPS",
        })
    };
    let frame = uniforms();
    for (name, script) in [
        ("noise", "noise(2).out()"),
        ("blur_1", "noise(2).blur(1).out()"),
        ("blur_4", "noise(2).blur(1).blur(1).blur(1).blur(1).out()"),
        ("tileable_2", "noise(2).tileable(0.15).tileable(0.15).out()"),
    ] {
        let graph = hydra_rust::eval(script)?;
        compile(&mut renderer, &graph)?;
        let snapshot = renderer.snapshot();
        unsafe { gl.finish() };
        measure(
            results,
            options,
            &format!("render/{name}"),
            "gpu_forced_completion_wall",
            || {
                render_multipass(&gl, &snapshot, renderer.ping(), frame);
                unsafe { gl.finish() };
                check(&gl)
            },
        )?;
    }
    let first = hydra_rust::eval("solid(0.2,0.3,0.4,1).blur(1).out()")?;
    let second = hydra_rust::eval("solid(0.6,0.3,0.4,1).blur(1).out()")?;
    compile(&mut renderer, &first)?;
    unsafe { gl.finish() };
    measure(
        results,
        options,
        "graph/unchanged_blur",
        "gpu_forced_completion_wall",
        || {
            compile(&mut renderer, &first)?;
            unsafe { gl.finish() };
            check(&gl)
        },
    )?;
    let mut alternate = false;
    measure(
        results,
        options,
        "graph/change_one_pass",
        "gpu_forced_completion_wall",
        || {
            alternate = !alternate;
            compile(&mut renderer, if alternate { &second } else { &first })?;
            unsafe { gl.finish() };
            check(&gl)
        },
    )?;
    for count in [1, 16] {
        let mut script = "noise(2).out();".to_string();
        for _ in 0..count {
            script.push_str("region(o0,0.0,0.0,1.0,1.0,0);");
        }
        let graph = hydra_rust::eval(&script)?;
        compile(&mut renderer, &graph)?;
        let snapshot = renderer.snapshot();
        unsafe { gl.finish() };
        measure(
            results,
            options,
            &format!("readback/render_{count}_regions"),
            "gpu_forced_completion_wall",
            || {
                render_multipass(&gl, &snapshot, renderer.ping(), frame);
                black_box(read_regions(
                    &gl,
                    &snapshot,
                    renderer.ping(),
                    SIZE,
                    &graph.regions,
                )?);
                unsafe { gl.finish() };
                check(&gl)
            },
        )?;
    }
    let graph = hydra_rust::eval("noise(2).out()")?;
    compile(&mut renderer, &graph)?;
    renderer.prepare_capture([640, 360])?;
    let snapshot = renderer.snapshot();
    unsafe { gl.finish() };
    measure(
        results,
        options,
        "readback/render_capture_640x360",
        "gpu_forced_completion_wall",
        || {
            render_multipass(&gl, &snapshot, renderer.ping(), frame);
            black_box(read_capture(&gl, &snapshot, SIZE)?);
            unsafe { gl.finish() };
            check(&gl)
        },
    )?;
    Ok(metadata)
}
