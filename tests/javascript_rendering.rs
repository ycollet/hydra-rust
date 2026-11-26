#![cfg(all(feature = "javascript", target_os = "macos"))]

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Arc;

use glow::HasContext;
use hydra_rust::javascript::{JavascriptFrame, JavascriptRuntime, JavascriptSettings};
use hydra_rust::renderer::{RenderUniforms, ShaderRenderer, render_multipass};

#[path = "support/native_gl.rs"]
mod native_gl;
use native_gl::NativeContext;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
}

#[derive(Default, Clone)]
struct GlCalls {
    programs: Vec<u32>,
    textures: usize,
    readbacks: usize,
    force_pending: bool,
    fence_waits: Vec<(u32, u64)>,
}

thread_local! {
    static GL_CALLS: RefCell<GlCalls> = RefCell::default();
}

unsafe extern "C" fn counted_create_program() -> u32 {
    let create: unsafe extern "C" fn() -> u32 = unsafe {
        std::mem::transmute(dlsym(
            (-2_isize) as *mut c_void,
            c"glCreateProgram".as_ptr(),
        ))
    };
    let program = unsafe { create() };
    GL_CALLS.with_borrow_mut(|calls| calls.programs.push(program));
    program
}

unsafe extern "C" fn counted_gen_textures(count: i32, textures: *mut u32) {
    let generate: unsafe extern "C" fn(i32, *mut u32) =
        unsafe { std::mem::transmute(dlsym((-2_isize) as *mut c_void, c"glGenTextures".as_ptr())) };
    GL_CALLS.with_borrow_mut(|calls| calls.textures += count as usize);
    unsafe { generate(count, textures) };
}

unsafe extern "C" fn counted_read_pixels(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    format: u32,
    kind: u32,
    pixels: *mut c_void,
) {
    let read: unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void) =
        unsafe { std::mem::transmute(dlsym((-2_isize) as *mut c_void, c"glReadPixels".as_ptr())) };
    GL_CALLS.with_borrow_mut(|calls| calls.readbacks += 1);
    unsafe { read(x, y, width, height, format, kind, pixels) };
}

unsafe extern "C" fn counted_client_wait_sync(fence: *mut c_void, flags: u32, timeout: u64) -> u32 {
    let pending = GL_CALLS.with_borrow_mut(|calls| {
        calls.fence_waits.push((flags, timeout));
        calls.force_pending
    });
    if pending {
        return glow::TIMEOUT_EXPIRED;
    }
    let wait: unsafe extern "C" fn(*mut c_void, u32, u64) -> u32 = unsafe {
        std::mem::transmute(dlsym(
            (-2_isize) as *mut c_void,
            c"glClientWaitSync".as_ptr(),
        ))
    };
    unsafe { wait(fence, flags, timeout) }
}

impl NativeContext {
    fn gl(&self) -> Arc<glow::Context> {
        self.gl_with_loader(|symbol| {
            Some(match symbol {
                "glCreateProgram" => counted_create_program as *const () as *const c_void,
                "glGenTextures" => counted_gen_textures as *const () as *const c_void,
                "glReadPixels" => counted_read_pixels as *const () as *const c_void,
                "glClientWaitSync" => counted_client_wait_sync as *const () as *const c_void,
                _ => return None,
            })
        })
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
fn graph_replacement_reuses_programs_and_targets_and_discards_only_failed_staging() {
    let context = NativeContext::new().unwrap();
    let gl = context.gl();
    let mut renderer = ShaderRenderer::new(gl.clone());
    renderer.ensure_resolution_checked(8, 8).unwrap();
    let graph = hydra_rust::eval("solid(0.2,0.3,0.4,1.0).blur(1.0).out(o0);").unwrap();
    assert!(renderer.compile_result_atomic(&graph).is_empty());
    let before = GL_CALLS.with_borrow(Clone::clone);
    assert!(renderer.compile_result_atomic(&graph).is_empty());
    let after = GL_CALLS.with_borrow(Clone::clone);
    assert_eq!(after.programs, before.programs);
    assert_eq!(after.textures, before.textures);

    let changed = hydra_rust::eval("solid(0.6,0.3,0.4,1.0).blur(1.0).out(o0);").unwrap();
    assert!(renderer.compile_result_atomic(&changed).is_empty());
    let partial = GL_CALLS.with_borrow(Clone::clone);
    assert_eq!(partial.programs.len(), before.programs.len() + 1);
    assert_eq!(partial.textures, before.textures);
    let mut invalid = hydra_rust::eval("solid(0.6,0.3,0.4,1.0).blur(1.0).out(o0);").unwrap();
    invalid.shaders[1] = Some("void mainImage(out vec4 c,in vec2 st){c=vec4(1.0);}".into());
    invalid.render_plan.passes[0] = "void mainImage(out vec4 c,in vec2 st){c=invalid;}".into();
    assert!(!renderer.compile_result_atomic(&invalid).is_empty());
    let failed = GL_CALLS.with_borrow(Clone::clone);
    assert_eq!(failed.programs.len(), partial.programs.len() + 1);
    unsafe {
        assert!(!gl.is_program(glow::NativeProgram(
            std::num::NonZeroU32::new(*failed.programs.last().unwrap()).unwrap()
        )));
    }
    assert!(renderer.compile_result_atomic(&changed).is_empty());
    assert_eq!(
        GL_CALLS.with_borrow(|calls| calls.programs.len()),
        failed.programs.len()
    );
    renderer.ensure_resolution_checked(16, 16).unwrap();
    assert!(renderer.compile_result_atomic(&changed).is_empty());
    assert_eq!(
        GL_CALLS.with_borrow(|calls| calls.programs.len()),
        failed.programs.len()
    );
    assert_eq!(unsafe { gl.get_error() }, glow::NO_ERROR);
}

#[test]
fn extension_examples_compile_in_both_scripting_languages() {
    let context = NativeContext::new().unwrap();
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
    let context = NativeContext::new().unwrap();
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
        for (compact, reference) in [
            (
                "noise(5).blur(1.25).blur(0.75).blur(2).blur(1).out()",
                "noise(5).blur(1.25).color(1,1,1,1).blur(0.75).color(1,1,1,1).blur(2).color(1,1,1,1).blur(1).out()",
            ),
            (
                "noise(5).blur(1.25).sharpen(0.75).emboss(2).erode(1).out()",
                "noise(5).blur(1.25).color(1,1,1,1).sharpen(0.75).color(1,1,1,1).emboss(2).color(1,1,1,1).erode(1).out()",
            ),
        ] {
            js.reset(JavascriptSettings::default()).unwrap();
            let graph = js.eval(compact).unwrap();
            assert_eq!(graph.render_plan.passes.len(), 5);
            assert!(renderer.compile_result_atomic(&graph).is_empty());
            let expected = draw(&renderer, [0.0; 128], 8.0);
            js.reset(JavascriptSettings::default()).unwrap();
            let graph = js.eval(reference).unwrap();
            assert_eq!(graph.render_plan.passes.len(), 8);
            assert!(renderer.compile_result_atomic(&graph).is_empty());
            assert_eq!(draw(&renderer, [0.0; 128], 8.0), expected);
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
            .eval("solid(()=>time,0.5,0.75,1).out();region(o0,0,0,1,1,0);region(o0,0,0,1,1,4)")
            .unwrap();
        assert!(renderer.compile_result_atomic(&regions).is_empty());
        draw(&renderer, [0.25; 128], 16.0);
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
        let pack = gl.create_buffer().unwrap();
        gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(pack));
        gl.buffer_data_size(glow::PIXEL_PACK_BUFFER, 1024, glow::STREAM_READ);
        gl.pixel_store_i32(glow::PACK_ROW_LENGTH, 20);
        gl.pixel_store_i32(glow::PACK_SKIP_ROWS, 1);
        gl.pixel_store_i32(glow::PACK_SKIP_PIXELS, 2);
        GL_CALLS.with_borrow_mut(|calls| calls.force_pending = true);
        let before = GL_CALLS.with_borrow(|calls| calls.readbacks);
        for index in 0..4 {
            if index < 3 {
                gl.disable(glow::SCISSOR_TEST);
                let mut frame = uniforms([0.25 + index as f32 * 0.25; 128]);
                frame.resolution = [16.0; 2];
                render_multipass(&gl, &renderer.snapshot(), renderer.ping(), frame);
                gl.enable(glow::SCISSOR_TEST);
            }
            assert!(
                hydra_rust::renderer::read_regions_async(
                    &gl,
                    &renderer.snapshot(),
                    renderer.ping(),
                    [16, 16],
                    &regions.regions,
                    true
                )
                .unwrap()
                .is_none()
            );
            assert_eq!(
                gl.get_parameter_i32(glow::PIXEL_PACK_BUFFER_BINDING),
                pack.0.get() as i32
            );
            assert_eq!(gl.get_parameter_i32(glow::PACK_ROW_LENGTH), 20);
            assert_eq!(gl.get_parameter_i32(glow::PACK_SKIP_ROWS), 1);
            assert_eq!(gl.get_parameter_i32(glow::PACK_SKIP_PIXELS), 2);
        }
        assert_eq!(GL_CALLS.with_borrow(|calls| calls.readbacks), before + 3);
        let mut invalid = hydra_rust::eval("solid(0,0,0,1).out()").unwrap();
        invalid.shaders[0] = Some("void mainImage(out vec4 c,in vec2 st){c=invalid;}".into());
        assert!(!renderer.compile_result_atomic(&invalid).is_empty());
        gl.finish();
        GL_CALLS.with_borrow_mut(|calls| calls.force_pending = false);
        let completed = hydra_rust::renderer::read_regions_async(
            &gl,
            &renderer.snapshot(),
            renderer.ping(),
            [16, 16],
            &regions.regions,
            false,
        )
        .unwrap()
        .unwrap();
        assert!((completed[0] - 0.75).abs() < 0.005);
        assert!((completed[1] - (0.75 * 0.2126 + 0.5 * 0.7152 + 0.75 * 0.0722)).abs() < 0.005);
        assert!(GL_CALLS.with_borrow(|calls| {
            calls
                .fence_waits
                .iter()
                .all(|(flags, timeout)| *flags == 0 && *timeout == 0)
        }));
        assert_eq!(GL_CALLS.with_borrow(|calls| calls.readbacks), before + 3);
        assert!(
            hydra_rust::renderer::read_regions_async(
                &gl,
                &renderer.snapshot(),
                renderer.ping(),
                [16, 16],
                &regions.regions,
                true
            )
            .unwrap()
            .is_none()
        );
        assert!(renderer.compile_result_atomic(&regions).is_empty());
        gl.finish();
        assert!(
            hydra_rust::renderer::read_regions_async(
                &gl,
                &renderer.snapshot(),
                renderer.ping(),
                [16, 16],
                &regions.regions,
                false
            )
            .unwrap()
            .is_none()
        );
        gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
        gl.delete_buffer(pack);
        for field in [
            glow::PACK_ROW_LENGTH,
            glow::PACK_SKIP_ROWS,
            glow::PACK_SKIP_PIXELS,
        ] {
            gl.pixel_store_i32(field, 0);
        }
        gl.disable(glow::SCISSOR_TEST);
        js.reset(JavascriptSettings::default()).unwrap();
        let mut script = "setFunction({name:'atlasRamp',type:'src',inputs:[],glsl:'return vec4(st.x,0.5,0.75,1.0);'});atlasRamp().out();".to_string();
        for slot in 0..16 {
            script.push_str(&format!(
                "region(o0,{},0,0.25,1,{});",
                (slot % 4) as f32 / 4.0,
                slot % 5
            ));
        }
        let atlas = js.eval(&script).unwrap();
        assert!(renderer.compile_result_atomic(&atlas).is_empty());
        draw(&renderer, [0.0; 128], 16.0);
        let before = GL_CALLS.with_borrow(|calls| calls.readbacks);
        let values = hydra_rust::renderer::read_regions(
            &gl,
            &renderer.snapshot(),
            renderer.ping(),
            [16, 16],
            &atlas.regions,
        )
        .unwrap();
        assert_eq!(GL_CALLS.with_borrow(|calls| calls.readbacks), before + 1);
        for (slot, value) in values.iter().enumerate() {
            let red = ((slot % 4) as f32 + 0.5) / 4.0;
            let expected = match slot % 5 {
                0 => red,
                1 => 0.5,
                2 => 0.75,
                3 => 1.0,
                _ => red * 0.2126 + 0.5 * 0.7152 + 0.75 * 0.0722,
            };
            assert!(
                (value - expected).abs() < 0.005,
                "region {slot}: {value} versus {expected}"
            );
        }
        renderer.prepare_capture([4, 3]).unwrap();
        let textures = GL_CALLS.with_borrow(|calls| calls.textures);
        renderer.prepare_capture([4, 3]).unwrap();
        assert_eq!(GL_CALLS.with_borrow(|calls| calls.textures), textures);
        gl.enable(glow::SCISSOR_TEST);
        let readbacks = GL_CALLS.with_borrow(|calls| calls.readbacks);
        let (width, height, pixels) =
            hydra_rust::renderer::read_capture(&gl, &renderer.snapshot(), [8, 8]).unwrap();
        assert_eq!((width, height, pixels.len()), (4, 3, 36));
        assert!((pixels[0] as i32 - 32).abs() <= 1);
        assert!((pixels[9] as i32 - 223).abs() <= 1);
        assert_eq!(GL_CALLS.with_borrow(|calls| calls.readbacks), readbacks + 1);
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
    let context = NativeContext::new().unwrap();
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
