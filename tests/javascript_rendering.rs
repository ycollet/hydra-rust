#![cfg(all(feature = "javascript", target_os = "macos"))]

use std::ffi::{CString, c_void};
use std::path::PathBuf;
use std::sync::Arc;

use glow::HasContext;
use hydra_rust::javascript::{JavascriptFrame, JavascriptRuntime};
use hydra_rust::renderer::{RenderUniforms, ShaderRenderer, render_multipass};

#[link(name = "OpenGL", kind = "framework")]
unsafe extern "C" {
    fn CGLChoosePixelFormat(
        attributes: *const i32,
        format: *mut *mut c_void,
        count: *mut i32,
    ) -> i32;
    fn CGLDestroyPixelFormat(format: *mut c_void) -> i32;
    fn CGLCreateContext(format: *mut c_void, share: *mut c_void, context: *mut *mut c_void) -> i32;
    fn CGLSetCurrentContext(context: *mut c_void) -> i32;
    fn CGLDestroyContext(context: *mut c_void) -> i32;
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
}

struct NativeContext(*mut c_void);

impl NativeContext {
    fn new() -> Self {
        const OPENGL_PROFILE: i32 = 99;
        const CORE_PROFILE: i32 = 0x3200;
        let attributes = [OPENGL_PROFILE, CORE_PROFILE, 0];
        let mut format = std::ptr::null_mut();
        let mut count = 0;
        let mut context = std::ptr::null_mut();
        unsafe {
            assert_eq!(
                CGLChoosePixelFormat(attributes.as_ptr(), &mut format, &mut count),
                0
            );
            assert!(!format.is_null());
            let result = CGLCreateContext(format, std::ptr::null_mut(), &mut context);
            CGLDestroyPixelFormat(format);
            assert_eq!(result, 0);
            assert_eq!(CGLSetCurrentContext(context), 0);
        }
        Self(context)
    }

    fn gl(&self) -> Arc<glow::Context> {
        Arc::new(unsafe {
            glow::Context::from_loader_function(|symbol| {
                let name = CString::new(symbol).unwrap();
                dlsym((-2_isize) as *mut c_void, name.as_ptr()).cast_const()
            })
        })
    }
}

impl Drop for NativeContext {
    fn drop(&mut self) {
        unsafe {
            CGLSetCurrentContext(std::ptr::null_mut());
            CGLDestroyContext(self.0);
        }
    }
}

fn uniforms(values: [f32; 128]) -> RenderUniforms {
    RenderUniforms {
        time: 0.0,
        resolution: [8.0; 2],
        mouse: [0.0; 2],
        beat: 0.0,
        tempo: 120.0,
        phase: 0.0,
        fft: [0.0; 8],
        javascript_values: values,
        audio: Default::default(),
        midi_note: std::array::from_fn(|_| 0.0),
        midi_velocity: std::array::from_fn(|_| 0.0),
        midi_cc: std::array::from_fn(|_| 0.0),
        midi_cc_smoothed: std::array::from_fn(|_| 0.0),
        midi_envelope: std::array::from_fn(|_| 0.0),
        midi_aftertouch: std::array::from_fn(|_| 0.0),
        midi_channel_aftertouch: 0.0,
        midi_values: std::array::from_fn(|_| 0.0),
        osc: std::array::from_fn(|_| 0.0),
    }
}

#[test]
fn native_rendering_uploads_callbacks_and_preserves_programs_after_compile_failure() {
    let context = NativeContext::new();
    let gl = context.gl();
    let mut renderer = ShaderRenderer::new(gl.clone());
    renderer.ensure_resolution(8, 8);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut js = JavascriptRuntime::new(root.clone()).unwrap();
    let graph = js.eval("solid(()=>time,0,0,1).out()").unwrap();
    assert!(
        renderer
            .compile_buffers_atomic(&graph.shaders, graph.render_mode)
            .is_empty()
    );

    unsafe {
        let texture = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            8,
            8,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        let target = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        assert_eq!(
            gl.check_framebuffer_status(glow::FRAMEBUFFER),
            glow::FRAMEBUFFER_COMPLETE
        );
        gl.viewport(0, 0, 8, 8);
        for time in [0.25, 0.5] {
            let values = js
                .frame(JavascriptFrame {
                    time,
                    ..Default::default()
                })
                .unwrap();
            render_multipass(&gl, &renderer.snapshot(), renderer.ping(), uniforms(values));
            let mut pixel = [0; 4];
            gl.read_pixels(
                4,
                4,
                1,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixel)),
            );
            assert!((pixel[0] as f32 - time * 255.0).abs() <= 1.0, "{pixel:?}");
            assert_eq!(&pixel[1..3], &[0, 0]);
            if time == 0.25 {
                let bad = js
                    .eval(
                        "solid(()=>0.9).out();setFunction({name:'broken',type:'src',\
                         inputs:[],glsl:'return invalid;'});broken().out(o1);render(o1)",
                    )
                    .unwrap();
                assert!(
                    !renderer
                        .compile_buffers_atomic(&bad.shaders, bad.render_mode)
                        .is_empty()
                );
                js.rollback_last_eval().unwrap();
            }
        }
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.delete_framebuffer(target);
        gl.delete_texture(texture);
        assert_eq!(gl.get_error(), glow::NO_ERROR);
    }

    let example_root = root.join("examples/javascript");
    let mut example = JavascriptRuntime::new(example_root.clone()).unwrap();
    let example = example
        .eval(&std::fs::read_to_string(example_root.join("live.js")).unwrap())
        .unwrap();
    assert!(
        renderer
            .compile_buffers_atomic(&example.shaders, example.render_mode)
            .is_empty()
    );
    let override_graph = js
        .eval(
            "setFunction({name:'osc',type:'src',inputs:[],glsl:'return vec4(_st,0,1);'});\
             osc().out()",
        )
        .unwrap();
    assert!(
        renderer
            .compile_buffers_atomic(&override_graph.shaders, override_graph.render_mode)
            .is_empty()
    );
}
