#![cfg(all(feature = "javascript", target_os = "macos"))]

use std::ffi::{CString, c_void};
use std::path::PathBuf;
use std::sync::Arc;

use glow::HasContext;
use hydra_rust::javascript::{JavascriptFrame, JavascriptRuntime, JavascriptSettings};
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
        region_values: [0.0; hydra_rust::extended::MAX_REGIONS],
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
fn extension_examples_compile_in_both_scripting_languages() {
    let context = NativeContext::new();
    let gl = context.gl();
    let mut renderer = ShaderRenderer::new(gl);
    renderer.ensure_resolution_checked(8, 8).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut examples = std::fs::read_dir(root.join("examples/extensions"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "hydra")
        })
        .collect::<Vec<_>>();
    examples.sort();
    assert!(!examples.is_empty());
    for path in examples {
        let name = path.file_stem().unwrap().to_str().unwrap();
        if name == "region_osc" && !cfg!(feature = "osc") {
            continue;
        }
        let native = hydra_rust::eval(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let errors = renderer.compile_result_atomic(&native);
        assert!(errors.is_empty(), "{}: {errors:?}", path.display());
        let path = root.join(format!("examples/javascript/extensions/{name}.js"));
        let mut js = JavascriptRuntime::new(root.clone()).unwrap();
        let graph = js
            .eval(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let errors = renderer.compile_result_atomic(&graph);
        assert!(errors.is_empty(), "{}: {errors:?}", path.display());
        js.frame(JavascriptFrame {
            time: 1.0,
            ..Default::default()
        })
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }
}

#[test]
fn extended_gpu_passes_history_and_region_readback() {
    let context = NativeContext::new();
    let gl = context.gl();
    let mut renderer = ShaderRenderer::new(gl.clone());
    renderer.ensure_resolution_checked(8, 8).unwrap();
    let mut js = JavascriptRuntime::new(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
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
        gl.viewport(0, 0, 8, 8);
        let draw = |renderer: &ShaderRenderer, values: [f32; 128], size: f32| {
            let mut frame = uniforms(values);
            frame.resolution = [size; 2];
            render_multipass(&gl, &renderer.snapshot(), renderer.ping(), frame);
            let mut pixels = [0; 8 * 8 * 4];
            gl.read_pixels(
                0,
                0,
                8,
                8,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            assert_eq!(gl.get_error(), glow::NO_ERROR);
            pixels
        };
        for (effect, left, right) in [
            ("blur", 85, 170),
            ("sharpen", 0, 255),
            ("sobel", 255, 255),
            ("emboss", 255, 255),
            ("dilate", 255, 255),
            ("erode", 0, 0),
        ] {
            js.reset(JavascriptSettings::default()).unwrap();
            let result=js.eval(&format!("setFunction({{name:'stepImage',type:'src',inputs:[],glsl:'return vec4(vec3(step(0.5,st.x)),1.0);'}});stepImage().{effect}(1).out()")).unwrap();
            assert_eq!(result.render_plan.passes.len(), 2);
            assert!(
                renderer.compile_result_atomic(&result).is_empty(),
                "{effect}"
            );
            let pixels = draw(&renderer, [0.0; 128], 8.0);
            for (x, expected) in [(3, left), (4, right)] {
                assert!(
                    (pixels[(4 * 8 + x) * 4] as i32 - expected).abs() <= 1,
                    "{effect}: {pixels:?}"
                );
            }
        }
        js.reset(JavascriptSettings::default()).unwrap();
        let spatial=js.eval("setFunction({name:'rampImage',type:'src',inputs:[],glsl:'return vec4(st.x,0.0,0.0,1.0);'});solid(rampImage(),0,0,1).out()").unwrap();
        assert!(renderer.compile_result_atomic(&spatial).is_empty());
        let pixels = draw(&renderer, [0.0; 128], 8.0);
        assert!((pixels[4 * 8 * 4] as i32 - 16).abs() <= 1);
        assert!((pixels[(4 * 8 + 7) * 4] as i32 - 239).abs() <= 1);

        js.reset(JavascriptSettings::default()).unwrap();
        let history=js.eval("solid(()=>time,0,0,1).out();history(o0,3).out(o1);history(o0,8).out(o2);render(o1)").unwrap();
        assert!(renderer.compile_result_atomic(&history).is_empty());
        for frame in 0..12 {
            if frame == 5 {
                assert!(renderer.compile_result_atomic(&history).is_empty());
            }
            if frame == 6 {
                let mut invalid = js.eval("").unwrap();
                invalid
                    .render_plan
                    .passes
                    .push("void mainImage(out vec4 c,in vec2 st){c=invalid;}".into());
                assert!(!renderer.compile_result_atomic(&invalid).is_empty());
                assert!(renderer.ensure_resolution_checked(4096, 4096).is_err());
            }
            let values = js
                .frame(JavascriptFrame {
                    time: (frame + 1) as f32 / 20.0,
                    ..Default::default()
                })
                .unwrap();
            let pixels = draw(&renderer, values, 8.0);
            let expected = if frame < 3 {
                0.0
            } else {
                (frame - 2) as f32 / 20.0 * 255.0
            };
            assert!(
                (pixels[(4 * 8 + 4) * 4] as f32 - expected).abs() <= 1.0,
                "frame {frame}"
            );
        }
        let eight = js.eval("render(o2)").unwrap();
        assert!(renderer.compile_result_atomic(&eight).is_empty());
        let pixels = draw(&renderer, [0.0; 128], 8.0);
        assert!((pixels[(4 * 8 + 4) * 4] as i32 - 64).abs() <= 1);
        renderer.clear_buffers([true, false, false, false]);
        let pixels = draw(&renderer, [0.0; 128], 8.0);
        assert_eq!(pixels[(4 * 8 + 4) * 4], 0);
        renderer.ensure_resolution_checked(16, 16).unwrap();
        let pixels = draw(&renderer, [0.0; 128], 16.0);
        assert_eq!(pixels[(4 * 8 + 4) * 4], 0);

        js.reset(JavascriptSettings::default()).unwrap();
        let regions = js
            .eval("solid(0.25,0.5,0.75,1).out();region(o0,0,0,1,1,0);region(o0,0,0,1,1,4)")
            .unwrap();
        assert!(renderer.compile_result_atomic(&regions).is_empty());
        draw(&renderer, [0.0; 128], 16.0);
        let read = gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING);
        let write = gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(0, 0, 0, 0);
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, 8);
        let values = hydra_rust::renderer::read_regions(
            &gl,
            &renderer.snapshot(),
            renderer.ping(),
            [16, 16],
            &regions.regions,
        )
        .unwrap();
        assert!((values[0] - 0.25).abs() < 0.005);
        assert!((values[1] - (0.25 * 0.2126 + 0.5 * 0.7152 + 0.75 * 0.0722)).abs() < 0.005);
        assert_eq!(gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING), read);
        assert_eq!(gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING), write);
        assert_eq!(gl.get_parameter_i32(glow::PACK_ALIGNMENT), 8);
        assert!(gl.is_enabled(glow::SCISSOR_TEST));
        gl.disable(glow::SCISSOR_TEST);
        gl.delete_framebuffer(target);
        gl.delete_texture(texture);
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
                        "o0.clear();solid(()=>0.9).out();setFunction({name:'broken',type:'src',\
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
        js.reset(JavascriptSettings::default()).unwrap();
        let mut retained = [0.0; 128];
        retained[0] = 0.5;
        render_multipass(
            &gl,
            &renderer.snapshot(),
            renderer.ping(),
            uniforms(retained),
        );
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
        assert!((pixel[0] as i32 - 128).abs() <= 1, "{pixel:?}");
        let replacement = js.eval("solid(()=>0.75,0,0,1).out()").unwrap();
        assert!(
            renderer
                .compile_buffers_atomic(&replacement.shaders, replacement.render_mode)
                .is_empty()
        );
        let values = js.frame(JavascriptFrame::default()).unwrap();
        render_multipass(&gl, &renderer.snapshot(), renderer.ping(), uniforms(values));
        gl.read_pixels(
            4,
            4,
            1,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixel)),
        );
        assert!((pixel[0] as i32 - 191).abs() <= 1, "{pixel:?}");

        js.reset(JavascriptSettings::default()).unwrap();
        let filled = js
            .eval("solid(1,0,0,1).out();solid(0,1,0,1).out(o1)")
            .unwrap();
        assert!(
            renderer
                .compile_buffers_atomic(&filled.shaders, filled.render_mode)
                .is_empty()
        );
        for _ in 0..2 {
            render_multipass(
                &gl,
                &renderer.snapshot(),
                renderer.ping(),
                uniforms([0.0; 128]),
            );
        }
        let feedback = js.eval("src(o0).out();src(o1).out(o1);o0.clear()").unwrap();
        assert!(
            renderer
                .compile_buffers_atomic(&feedback.shaders, feedback.render_mode)
                .is_empty()
        );
        let draw = gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
        let read = gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING);
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(0, 0, 0, 0);
        gl.color_mask(false, false, false, false);
        renderer.clear_buffers(feedback.buffer_clear);
        assert_eq!(gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING), draw);
        assert_eq!(gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING), read);
        assert!(gl.is_enabled(glow::SCISSOR_TEST));
        let mut mask = [1; 4];
        gl.get_parameter_i32_slice(glow::COLOR_WRITEMASK, &mut mask);
        assert_eq!(mask, [0; 4]);
        gl.disable(glow::SCISSOR_TEST);
        gl.color_mask(true, true, true, true);
        for _ in 0..2 {
            render_multipass(
                &gl,
                &renderer.snapshot(),
                renderer.ping(),
                uniforms([0.0; 128]),
            );
            gl.read_pixels(
                4,
                4,
                1,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixel)),
            );
            assert_eq!(&pixel[..3], &[0, 0, 0]);
        }
        let retained = js.eval("render(o1)").unwrap();
        assert!(
            renderer
                .compile_buffers_atomic(&retained.shaders, retained.render_mode)
                .is_empty()
        );
        render_multipass(
            &gl,
            &renderer.snapshot(),
            renderer.ping(),
            uniforms([0.0; 128]),
        );
        gl.read_pixels(
            4,
            4,
            1,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixel)),
        );
        assert_eq!(&pixel[..3], &[0, 255, 0]);
        let sampler = js.eval(
            "setFunction({name:'_sample',type:'src',inputs:[{name:'_tex',type:'sampler2D',default:o1}],\
             glsl:'return texture2D(_tex,_st);'});_sample().out();render(o0)"
        ).unwrap();
        assert!(
            renderer
                .compile_buffers_atomic(&sampler.shaders, sampler.render_mode)
                .is_empty()
        );
        render_multipass(
            &gl,
            &renderer.snapshot(),
            renderer.ping(),
            uniforms([0.0; 128]),
        );
        gl.read_pixels(
            4,
            4,
            1,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixel)),
        );
        assert_eq!(&pixel[..3], &[0, 255, 0]);
        renderer.upload_source(
            2,
            &hydra_rust::source::SourceFrame {
                pixels: [255, 0, 0].repeat(4),
                width: 2,
                height: 2,
            },
        );
        let source = js.eval("_sample(s2).out()").unwrap();
        assert!(
            renderer
                .compile_buffers_atomic(&source.shaders, source.render_mode)
                .is_empty()
        );
        render_multipass(
            &gl,
            &renderer.snapshot(),
            renderer.ping(),
            uniforms([0.0; 128]),
        );
        gl.read_pixels(
            4,
            4,
            1,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixel)),
        );
        assert_eq!(&pixel[..3], &[255, 0, 0]);
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
    for code in [
        "osc(10).tileable(()=>0.15).out()",
        "strokeText('Hydra',{lineWidth:'3px'}).out()",
        "setFunction({name:'_localCoord',type:'coord',glsl:'vec2 st = _st * 2.0; return st;'});osc(10)._localCoord().out()",
        "setFunction({name:'_localColor',type:'color',glsl:'vec4 c0 = _c0 * 0.5; return c0;'});osc(10)._localColor().out()",
        "setFunction({name:'_loopColor',type:'color',glsl:'for(int c0=0;c0<1;c0++) {} return c0;'});osc(10)._loopColor().out()",
        "setFunction({name:'_unusedBroken',type:'src',glsl:'return invalid;'});osc(10).out()",
        "setFunction({name:'_fields',type:'src',inputs:[{name:'_in',type:'float',default:0.25}],\
         glsl:'struct Data { float _in; float time; }; Data data; data._in=0.4; data.time=0.1; return vec4(_in+data. /* field */ _in+data.time);'});_fields().out()",
    ] {
        let graph = JavascriptRuntime::new(root.clone())
            .unwrap()
            .eval(code)
            .unwrap();
        assert!(
            renderer
                .compile_buffers_atomic(&graph.shaders, graph.render_mode)
                .is_empty()
        );
        if let Some(text) = &graph.text_data {
            renderer.upload_text(text);
        }
    }
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
    let module_path = example_root.join("module.mjs");
    let mut module = JavascriptRuntime::new(example_root).unwrap();
    let module_graph = module
        .eval_named(
            &std::fs::read_to_string(&module_path).unwrap(),
            &module_path,
        )
        .unwrap();
    assert!(
        renderer
            .compile_buffers_atomic(&module_graph.shaders, module_graph.render_mode)
            .is_empty()
    );
    assert!(
        module
            .frame(JavascriptFrame {
                time: 0.5,
                ..Default::default()
            })
            .unwrap()
            .iter()
            .any(|value| *value > 0.0)
    );
}
