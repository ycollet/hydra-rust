use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use glow::{HasContext, PixelUnpackData};

use crate::audio::{AudioFeatures, NUM_FFT_BINS};
use crate::eval::{BufferFilter, RenderMode};
use crate::midi::{NUM_MIDI_CC, NUM_MIDI_ENVELOPES, NUM_MIDI_NOTES, NUM_MIDI_VALUES};
use crate::osc::NUM_OSC_SLOTS;
use crate::shader;
use crate::source::{NUM_SOURCES, SourceFrame};
use crate::text::TextData;

mod region_readback;
pub use region_readback::{read_regions, read_regions_async};

#[derive(Clone, Copy)]
pub struct RenderUniforms {
    pub region_values: [f32; crate::extended::MAX_REGIONS],
    pub time: f32,
    pub resolution: [f32; 2],
    pub mouse: [f32; 2],
    pub beat: f32,
    pub tempo: f32,
    pub phase: f32,
    pub fft: [f32; NUM_FFT_BINS],
    pub javascript_values: [f32; shader::NUM_JS_VALUES],
    pub audio: AudioFeatures,
    pub midi_note: [f32; NUM_MIDI_NOTES],
    pub midi_velocity: [f32; NUM_MIDI_NOTES],
    pub midi_cc: [f32; NUM_MIDI_CC],
    pub midi_cc_smoothed: [f32; NUM_MIDI_CC],
    pub midi_envelope: [f32; NUM_MIDI_ENVELOPES],
    pub midi_aftertouch: [f32; NUM_MIDI_NOTES],
    pub midi_channel_aftertouch: f32,
    pub midi_values: [f32; NUM_MIDI_VALUES],
    pub osc: [f32; NUM_OSC_SLOTS],
}

const NUM_BUFFERS: usize = 4;

#[derive(Clone, Copy)]
struct ProgramState {
    program: glow::Program,
    loc_time: Option<glow::UniformLocation>,
    loc_resolution: Option<glow::UniformLocation>,
    loc_mouse: Option<glow::UniformLocation>,
    loc_beat: Option<glow::UniformLocation>,
    loc_tempo: Option<glow::UniformLocation>,
    loc_phase: Option<glow::UniformLocation>,
    loc_fft: Option<glow::UniformLocation>,
    loc_javascript_values: Option<glow::UniformLocation>,
    loc_audio_rhythm: Option<glow::UniformLocation>,
    loc_midi_note: Option<glow::UniformLocation>,
    loc_midi_velocity: Option<glow::UniformLocation>,
    loc_midi_cc: Option<glow::UniformLocation>,
    loc_midi_cc_smoothed: Option<glow::UniformLocation>,
    loc_midi_envelope: Option<glow::UniformLocation>,
    loc_midi_aftertouch: Option<glow::UniformLocation>,
    loc_midi_channel_aftertouch: Option<glow::UniformLocation>,
    loc_midi_values: Option<glow::UniformLocation>,
    loc_osc: Option<glow::UniformLocation>,
    loc_buffers: [Option<glow::UniformLocation>; NUM_BUFFERS],
    loc_text0: Option<glow::UniformLocation>,
    loc_sources: [Option<glow::UniformLocation>; NUM_SOURCES],
    loc_passes: [Option<glow::UniformLocation>; crate::extended::MAX_PASSES],
    loc_history: [[Option<glow::UniformLocation>; crate::extended::MAX_HISTORY]; NUM_BUFFERS],
    loc_regions: Option<glow::UniformLocation>,
    loc_parameters: Option<glow::UniformLocation>,
}

impl ProgramState {
    #[inline]
    fn upload_uniforms(
        &self,
        gl: &glow::Context,
        u: &RenderUniforms,
        parameter_values: &[f32; crate::parameters::MAX_PARAMETERS],
    ) {
        unsafe {
            if let Some(ref loc) = self.loc_parameters {
                gl.uniform_1_f32_slice(Some(loc), parameter_values);
            }

            if let Some(ref loc) = self.loc_time {
                gl.uniform_1_f32(Some(loc), u.time);
            }
            if let Some(ref loc) = self.loc_beat {
                gl.uniform_1_f32(Some(loc), u.beat);
            }
            if let Some(ref loc) = self.loc_tempo {
                gl.uniform_1_f32(Some(loc), u.tempo);
            }
            if let Some(ref loc) = self.loc_phase {
                gl.uniform_1_f32(Some(loc), u.phase);
            }
            if let Some(ref loc) = self.loc_fft {
                gl.uniform_1_f32_slice(Some(loc), &u.fft);
            }
            if let Some(ref loc) = self.loc_javascript_values {
                gl.uniform_1_f32_slice(Some(loc), &u.javascript_values);
            }
            if let Some(ref loc) = self.loc_audio_rhythm {
                gl.uniform_3_f32(Some(loc), u.audio.onset, u.audio.beat, u.audio.bpm);
            }
            if let Some(ref loc) = self.loc_midi_note {
                gl.uniform_1_f32_slice(Some(loc), &u.midi_note);
            }
            if let Some(ref loc) = self.loc_midi_velocity {
                gl.uniform_1_f32_slice(Some(loc), &u.midi_velocity);
            }
            if let Some(ref loc) = self.loc_midi_cc {
                gl.uniform_1_f32_slice(Some(loc), &u.midi_cc);
            }
            if let Some(ref loc) = self.loc_midi_cc_smoothed {
                gl.uniform_1_f32_slice(Some(loc), &u.midi_cc_smoothed);
            }
            if let Some(ref loc) = self.loc_midi_envelope {
                gl.uniform_1_f32_slice(Some(loc), &u.midi_envelope);
            }
            if let Some(ref loc) = self.loc_midi_aftertouch {
                gl.uniform_1_f32_slice(Some(loc), &u.midi_aftertouch);
            }
            if let Some(ref loc) = self.loc_midi_channel_aftertouch {
                gl.uniform_1_f32(Some(loc), u.midi_channel_aftertouch);
            }
            if let Some(ref loc) = self.loc_midi_values {
                gl.uniform_1_f32_slice(Some(loc), &u.midi_values);
            }
            if let Some(ref loc) = self.loc_osc {
                gl.uniform_1_f32_slice(Some(loc), &u.osc);
            }
            if let Some(loc) = &self.loc_regions {
                gl.uniform_1_f32_slice(Some(loc), &u.region_values);
            }
            if let Some(ref loc) = self.loc_resolution {
                gl.uniform_2_f32(Some(loc), u.resolution[0], u.resolution[1]);
            }
            if let Some(ref loc) = self.loc_mouse {
                gl.uniform_2_f32(Some(loc), u.mouse[0], u.mouse[1]);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct BufferTarget {
    fbo: [glow::Framebuffer; 2],
    texture: [glow::Texture; 2],
}

#[derive(Clone)]
pub struct RenderSnapshot {
    parameter_values: [f32; crate::parameters::MAX_PARAMETERS],
    targets: [BufferTarget; NUM_BUFFERS],
    programs: [Option<ProgramState>; NUM_BUFFERS],
    display: ProgramState,
    vao: glow::VertexArray,
    render_mode: RenderMode,
    text_texture: Option<glow::Texture>,
    source_textures: [Option<glow::Texture>; NUM_SOURCES],
    /// Each source texture's last-uploaded (width, height), so
    /// `upload_source` can stream same-size frames in place via
    /// `tex_sub_image_2d` instead of reallocating GPU storage on every
    /// single incoming camera frame (~30Hz per active source).
    source_dims: [Option<(u32, u32)>; NUM_SOURCES],
    extended: Arc<ExtendedTargets>,
    capture: Option<(SingleTarget, [u32; 2])>,
}

#[derive(Clone, Copy)]
struct SingleTarget {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
}

impl SingleTarget {
    fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_texture(self.texture);
        }
    }
}

#[derive(Default)]
struct ExtendedTargets {
    passes: Vec<(ProgramState, SingleTarget)>,
    history: [Vec<SingleTarget>; NUM_BUFFERS],
    cursor: [AtomicUsize; NUM_BUFFERS],
    region: Option<SingleTarget>,
    readback_queue: std::sync::Mutex<region_readback::ReadbackQueue>,
}

impl ExtendedTargets {
    fn destroy(&self, gl: &glow::Context) {
        self.readback_queue.lock().unwrap().destroy(gl);
        unsafe {
            for (program, target) in &self.passes {
                gl.delete_program(program.program);
                target.destroy(gl);
            }
            for target in self.history.iter().flatten() {
                target.destroy(gl);
            }
            if let Some(target) = self.region {
                target.destroy(gl);
            }
        }
    }
}

pub struct ShaderRenderer {
    snapshot: RenderSnapshot,
    vbo: glow::Buffer,
    gl: Arc<glow::Context>,
    resolution: [u32; 2],
    ping: Arc<AtomicBool>,
    vertex_shader: glow::Shader,
    shader_sources: [Option<String>; NUM_BUFFERS],
    pass_sources: Vec<String>,
    parameters: Vec<crate::parameters::Parameter>,
}

struct StagedResources {
    gl: Arc<glow::Context>,
    programs: Vec<glow::Program>,
    targets: Vec<SingleTarget>,
}

impl StagedResources {
    fn program(&mut self, vertex: glow::Shader, source: &str) -> Result<ProgramState, String> {
        let program = compile_program(&self.gl, vertex, &shader::fragment_source(source))?;
        self.programs.push(program);
        Ok(resolve_program_state(&self.gl, program))
    }

    fn target(&mut self, resolution: [u32; 2]) -> Result<SingleTarget, String> {
        let target = ShaderRenderer::create_single(&self.gl, resolution)?;
        self.targets.push(target);
        Ok(target)
    }

    fn commit(mut self) {
        self.programs.clear();
        self.targets.clear();
    }
}

impl Drop for StagedResources {
    fn drop(&mut self) {
        unsafe {
            for program in &self.programs {
                self.gl.delete_program(*program);
            }
            for target in &self.targets {
                target.destroy(&self.gl);
            }
        }
    }
}

impl Drop for ShaderRenderer {
    fn drop(&mut self) {
        unsafe {
            self.snapshot.extended.destroy(&self.gl);
            for target in &self.snapshot.targets {
                for fbo in &target.fbo {
                    self.gl.delete_framebuffer(*fbo);
                }
                for tex in &target.texture {
                    self.gl.delete_texture(*tex);
                }
            }
            for prog in self.snapshot.programs.iter().flatten() {
                self.gl.delete_program(prog.program);
            }
            self.gl.delete_program(self.snapshot.display.program);
            self.gl.delete_shader(self.vertex_shader);
            if let Some((target, _)) = self.snapshot.capture {
                target.destroy(&self.gl);
            }
            if let Some(tex) = self.snapshot.text_texture {
                self.gl.delete_texture(tex);
            }
            for tex in self.snapshot.source_textures.iter().flatten() {
                self.gl.delete_texture(*tex);
            }
            self.gl.delete_vertex_array(self.snapshot.vao);
            self.gl.delete_buffer(self.vbo);
        }
    }
}

impl ShaderRenderer {
    pub fn new(gl: Arc<glow::Context>) -> Self {
        let (vao, vbo) = create_fullscreen_quad(&gl);
        let resolution = [512, 512];
        let targets = std::array::from_fn(|_| create_buffer_target(&gl, resolution));
        let vertex_shader = compile_vertex_shader(&gl).expect("vertex shader must compile");
        let display_program =
            compile_program(&gl, vertex_shader, &shader::display_fragment_source())
                .expect("display shader must compile");
        let display = resolve_program_state(&gl, display_program);

        let default_program = compile_program(
            &gl,
            vertex_shader,
            &shader::fragment_source(shader::DEFAULT_SHADER),
        )
        .expect("default shader must compile");
        let slot0 = Some(resolve_program_state(&gl, default_program));

        Self {
            snapshot: RenderSnapshot {
                parameter_values: [0.0; crate::parameters::MAX_PARAMETERS],
                targets,
                programs: [slot0, None, None, None],
                display,
                vao,
                render_mode: RenderMode::Single0,
                text_texture: None,
                source_textures: [None; NUM_SOURCES],
                source_dims: [None; NUM_SOURCES],
                extended: Arc::new(ExtendedTargets::default()),
                capture: None,
            },
            vbo,
            gl,
            resolution,
            ping: Arc::new(AtomicBool::new(false)),
            vertex_shader,
            shader_sources: [Some(shader::DEFAULT_SHADER.into()), None, None, None],
            pass_sources: Vec::new(),
            parameters: Vec::new(),
        }
    }

    pub fn ping(&self) -> &Arc<AtomicBool> {
        &self.ping
    }

    pub fn snapshot(&self) -> RenderSnapshot {
        self.snapshot.clone()
    }

    pub fn resolution(&self) -> [u32; 2] {
        self.resolution
    }

    pub fn prepare_capture(&mut self, resolution: [u32; 2]) -> Result<(), String> {
        if self
            .snapshot
            .capture
            .is_some_and(|(_, size)| size == resolution)
        {
            return Ok(());
        }
        let maximum = unsafe { self.gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) } as u32;
        if resolution.contains(&0) || resolution.iter().any(|dimension| *dimension > maximum) {
            return Err("Capture dimensions exceed the graphics device limit".into());
        }
        let target = Self::create_single(&self.gl, resolution)?;
        if let Some((previous, _)) = self.snapshot.capture.replace((target, resolution)) {
            previous.destroy(&self.gl);
        }
        Ok(())
    }

    pub fn clear_buffers(&mut self, buffers: [bool; NUM_BUFFERS]) {
        if !buffers.iter().any(|clear| *clear) {
            return;
        }
        self.snapshot
            .extended
            .readback_queue
            .lock()
            .unwrap()
            .destroy(&self.gl);
        unsafe {
            let framebuffer = self.gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
            let scissor = self.gl.is_enabled(glow::SCISSOR_TEST);
            let mut mask = [0; 4];
            self.gl
                .get_parameter_i32_slice(glow::COLOR_WRITEMASK, &mut mask);
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.color_mask(true, true, true, true);
            for (index, clear) in buffers.iter().enumerate() {
                if *clear {
                    for fbo in self.snapshot.targets[index].fbo.into_iter().chain(
                        self.snapshot.extended.history[index]
                            .iter()
                            .map(|target| target.fbo),
                    ) {
                        self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(fbo));
                        self.gl.clear_buffer_f32_slice(glow::COLOR, 0, &[0.0; 4]);
                    }
                }
            }
            self.gl.bind_framebuffer(
                glow::DRAW_FRAMEBUFFER,
                std::num::NonZeroU32::new(framebuffer as u32).map(glow::NativeFramebuffer),
            );
            self.gl
                .color_mask(mask[0] != 0, mask[1] != 0, mask[2] != 0, mask[3] != 0);
            if scissor {
                self.gl.enable(glow::SCISSOR_TEST);
            }
        }
    }

    pub fn ensure_resolution(&mut self, width: u32, height: u32) {
        if self.resolution == [width, height] || width == 0 || height == 0 {
            return;
        }
        self.resolution = [width, height];
        self.snapshot
            .extended
            .readback_queue
            .lock()
            .unwrap()
            .destroy(&self.gl);
        unsafe {
            for target in &self.snapshot.targets {
                for i in 0..2 {
                    self.gl
                        .bind_texture(glow::TEXTURE_2D, Some(target.texture[i]));
                    self.gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        width as i32,
                        height as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        PixelUnpackData::Slice(None),
                    );
                }
            }
            self.gl.bind_texture(glow::TEXTURE_2D, None);
        }
        for target in self.snapshot.extended.history.iter().flatten().chain(
            self.snapshot
                .extended
                .passes
                .iter()
                .map(|(_, target)| target),
        ) {
            Self::resize_single(&self.gl, *target, self.resolution);
        }
        for cursor in &self.snapshot.extended.cursor {
            cursor.store(0, Ordering::Relaxed);
        }
    }

    pub fn ensure_resolution_checked(&mut self, width: u32, height: u32) -> Result<(), String> {
        let plan = crate::extended::RenderPlan {
            passes: vec![String::new(); self.snapshot.extended.passes.len()],
            history: std::array::from_fn(|index| self.snapshot.extended.history[index].len()),
            ..Default::default()
        };
        Self::validate_plan(&plan, [width, height])?;
        let maximum = unsafe { self.gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) } as u32;
        if width == 0 || height == 0 || width > maximum || height > maximum {
            return Err("Render dimensions exceed the graphics device limit".into());
        }
        let previous = self.resolution;
        self.ensure_resolution(width, height);
        if previous != [width, height] {
            let error = unsafe { self.gl.get_error() };
            if error != glow::NO_ERROR {
                self.resolution = previous;
                return Err(format!(
                    "Render texture resizing failed: OpenGL error {error:#x}"
                ));
            }
        }
        Ok(())
    }

    /// `o0-o3.setNearest()`/`.setLinear()`/`.setMode(...)` - sets a
    /// buffer's texture sampling mode. Applies to both of that buffer's
    /// ping-pong textures (`texture[0]`/`[1]`, sampled interchangeably
    /// frame-to-frame). A texture's filter parameter is independent of
    /// its image data, so this survives later `ensure_resolution` resizes
    /// without needing to be reapplied there.
    pub fn set_buffer_filter(&mut self, buf: usize, mode: BufferFilter) {
        if buf >= NUM_BUFFERS {
            return;
        }
        let f = match mode {
            BufferFilter::Linear => glow::LINEAR,
            BufferFilter::Nearest => glow::NEAREST,
        } as i32;
        unsafe {
            for tex in self.snapshot.targets[buf].texture {
                self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                self.gl
                    .tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, f);
                self.gl
                    .tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, f);
            }
            self.gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    fn validate_plan(
        plan: &crate::extended::RenderPlan,
        resolution: [u32; 2],
    ) -> Result<(), String> {
        if plan.passes.len() > crate::extended::MAX_PASSES
            || !(1..=crate::extended::MAX_HISTORY).contains(&plan.history_limit)
            || plan.history.iter().any(|count| *count > plan.history_limit)
        {
            return Err("Render plan exceeds pass or history limits".into());
        }
        if !plan.passes.is_empty() || plan.history.iter().any(|count| *count > 0) {
            let count = 8 + plan.passes.len() + plan.history.iter().sum::<usize>();
            let bytes = (resolution[0] as usize)
                .checked_mul(resolution[1] as usize)
                .and_then(|pixels| pixels.checked_mul(4))
                .and_then(|bytes| bytes.checked_mul(count))
                .ok_or("Extended texture allocation overflow")?;
            if bytes > crate::extended::MAX_TEXTURE_BYTES {
                return Err("Extended render textures exceed the 256 MiB allocation limit".into());
            }
        }
        Ok(())
    }

    fn create_single(gl: &glow::Context, resolution: [u32; 2]) -> Result<SingleTarget, String> {
        unsafe {
            let texture = gl.create_texture()?;
            let fbo = match gl.create_framebuffer() {
                Ok(fbo) => fbo,
                Err(error) => {
                    gl.delete_texture(texture);
                    return Err(error);
                }
            };
            let target = SingleTarget { texture, fbo };
            let previous = gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            set_texture_defaults(gl);
            Self::resize_single(gl, target, resolution);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::DRAW_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            let complete =
                gl.check_framebuffer_status(glow::DRAW_FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
            let error = gl.get_error();
            gl.bind_framebuffer(
                glow::DRAW_FRAMEBUFFER,
                std::num::NonZeroU32::new(previous as u32).map(glow::NativeFramebuffer),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            if !complete || error != glow::NO_ERROR {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(texture);
                return Err("Could not allocate an extended render texture".into());
            }
            Ok(target)
        }
    }

    fn resize_single(gl: &glow::Context, target: SingleTarget, resolution: [u32; 2]) {
        unsafe {
            let previous = gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
            let scissor = gl.is_enabled(glow::SCISSOR_TEST);
            let mut mask = [0; 4];
            gl.get_parameter_i32_slice(glow::COLOR_WRITEMASK, &mut mask);
            gl.bind_texture(glow::TEXTURE_2D, Some(target.texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                resolution[0] as i32,
                resolution[1] as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                PixelUnpackData::Slice(None),
            );
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(target.fbo));
            gl.framebuffer_texture_2d(
                glow::DRAW_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(target.texture),
                0,
            );
            gl.disable(glow::SCISSOR_TEST);
            gl.color_mask(true, true, true, true);
            gl.clear_buffer_f32_slice(glow::COLOR, 0, &[0.0; 4]);
            gl.color_mask(mask[0] != 0, mask[1] != 0, mask[2] != 0, mask[3] != 0);
            if scissor {
                gl.enable(glow::SCISSOR_TEST);
            }
            gl.bind_framebuffer(
                glow::DRAW_FRAMEBUFFER,
                std::num::NonZeroU32::new(previous as u32).map(glow::NativeFramebuffer),
            );
        }
    }

    pub fn upload_text(&mut self, data: &TextData) {
        unsafe {
            let tex = self.snapshot.text_texture.unwrap_or_else(|| {
                let tex = self.gl.create_texture().expect("create text texture");
                self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                set_texture_defaults(&self.gl);
                tex
            });

            self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                data.width as i32,
                data.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                PixelUnpackData::Slice(Some(&data.pixels)),
            );
            self.gl.bind_texture(glow::TEXTURE_2D, None);
            self.snapshot.text_texture = Some(tex);
        }
    }

    pub fn upload_source(&mut self, slot: usize, frame: &SourceFrame) {
        if slot >= NUM_SOURCES {
            return;
        }
        unsafe {
            let tex = self.snapshot.source_textures[slot].unwrap_or_else(|| {
                let tex = self.gl.create_texture().expect("create source texture");
                self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                set_texture_defaults(&self.gl);
                tex
            });

            self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            let dims = (frame.width, frame.height);
            if self.snapshot.source_dims[slot] == Some(dims) {
                // Same size as last frame: stream into the existing
                // storage rather than reallocating it from scratch.
                self.gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    frame.width as i32,
                    frame.height as i32,
                    glow::RGB,
                    glow::UNSIGNED_BYTE,
                    PixelUnpackData::Slice(Some(&frame.pixels)),
                );
            } else {
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGB8 as i32,
                    frame.width as i32,
                    frame.height as i32,
                    0,
                    glow::RGB,
                    glow::UNSIGNED_BYTE,
                    PixelUnpackData::Slice(Some(&frame.pixels)),
                );
                self.snapshot.source_dims[slot] = Some(dims);
            }
            self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
            self.snapshot.source_textures[slot] = Some(tex);
        }
    }

    /// Compiles each buffer's shader, swapping in the new program on
    /// success (the previous one, if any, is deleted either way - a
    /// compile failure just means that buffer keeps rendering nothing
    /// rather than a broken program). Returns one error string per buffer
    /// that failed to compile, so the caller can surface it - `eval()`
    /// only validates the Rhai/GLSL-codegen side, so a GLSL body ported
    /// into `library.glsl` with a real syntax mistake would otherwise fail
    /// completely silently here.
    pub fn compile_buffers(
        &mut self,
        shaders: &[Option<String>; NUM_BUFFERS],
        render_mode: RenderMode,
    ) -> Vec<String> {
        self.snapshot.extended.destroy(&self.gl);
        self.snapshot.extended = Arc::new(ExtendedTargets::default());
        self.pass_sources.clear();
        let mut errors = Vec::new();
        for (i, shader_src) in shaders.iter().enumerate() {
            if self.shader_sources[i] == *shader_src {
                continue;
            }
            if let Some(old) = self.snapshot.programs[i].take() {
                unsafe { self.gl.delete_program(old.program) };
            }
            self.shader_sources[i] = None;
            if let Some(code) = shader_src {
                let full_src = shader::fragment_source(code);
                match compile_program(&self.gl, self.vertex_shader, &full_src) {
                    Ok(program) => {
                        self.snapshot.programs[i] = Some(resolve_program_state(&self.gl, program));
                        self.shader_sources[i] = Some(code.clone());
                    }
                    Err(e) => errors.push(format!("buffer {i}: {e}")),
                }
            }
        }
        self.snapshot.render_mode = render_mode;
        errors
    }

    /// Keeps the current programs when any replacement fails to compile.
    pub fn compile_buffers_atomic(
        &mut self,
        shaders: &[Option<String>; NUM_BUFFERS],
        render_mode: RenderMode,
    ) -> Vec<String> {
        self.compile_plan_atomic(
            shaders,
            render_mode,
            &crate::extended::RenderPlan::default(),
            false,
        )
    }

    pub fn compile_result_atomic(&mut self, result: &crate::eval::EvalResult) -> Vec<String> {
        if let Err(error) = crate::parameters::validate(&result.parameters) {
            return vec![error];
        }
        if let Err(error) = result.region_sampling.validate() {
            return vec![error];
        }
        if result.regions.len() > crate::extended::MAX_REGIONS {
            return vec!["Image region limit exceeded".into()];
        }
        for region in &result.regions {
            if let Err(error) = region.validate() {
                return vec![error];
            }
        }
        let errors = self.compile_plan_atomic(
            &result.shaders,
            result.render_mode,
            &result.render_plan,
            !result.regions.is_empty(),
        );
        if errors.is_empty() {
            self.snapshot.parameter_values = [0.0; crate::parameters::MAX_PARAMETERS];
            for (index, parameter) in result.parameters.iter().enumerate() {
                self.snapshot.parameter_values[index] = parameter.default;
            }
            self.parameters = result.parameters.clone();
        }
        errors
    }

    pub fn set_parameter_values(
        &mut self,
        values: &crate::parameters::ParameterValues,
    ) -> Result<(), String> {
        self.snapshot.parameter_values = values.uniforms(&self.parameters)?;
        Ok(())
    }

    pub fn compile_result_with_parameters_atomic(
        &mut self,
        result: &crate::eval::EvalResult,
        values: &crate::parameters::ParameterValues,
    ) -> Vec<String> {
        let uniforms = match values.uniforms(&result.parameters) {
            Ok(uniforms) => uniforms,
            Err(error) => return vec![error],
        };
        let errors = self.compile_result_atomic(result);
        if errors.is_empty() {
            self.snapshot.parameter_values = uniforms;
        }
        errors
    }

    fn compile_plan_atomic(
        &mut self,
        shaders: &[Option<String>; NUM_BUFFERS],
        render_mode: RenderMode,
        plan: &crate::extended::RenderPlan,
        regions: bool,
    ) -> Vec<String> {
        if let Err(error) = Self::validate_plan(plan, self.resolution) {
            return vec![error];
        }
        let mut extended = ExtendedTargets::default();
        let mut resources = StagedResources {
            gl: Arc::clone(&self.gl),
            programs: Vec::new(),
            targets: Vec::new(),
        };
        let mut programs = std::array::from_fn::<_, NUM_BUFFERS, _>(|_| None);
        let mut errors = Vec::new();
        for (i, source) in shaders.iter().enumerate() {
            if let Some(source) = source {
                if self.shader_sources[i].as_ref() == Some(source) {
                    programs[i] = self.snapshot.programs[i];
                    continue;
                }
                match resources.program(self.vertex_shader, source) {
                    Ok(program) => programs[i] = Some(program),
                    Err(error) => errors.push(format!("buffer {i}: {error}")),
                }
            }
        }
        for (index, source) in plan.passes.iter().enumerate() {
            let previous = self.snapshot.extended.passes.get(index);
            let program = if self.pass_sources.get(index) == Some(source) {
                Ok(previous.expect("cached pass has a target").0)
            } else {
                resources.program(self.vertex_shader, source)
            };
            let program = match program {
                Ok(program) => program,
                Err(error) => {
                    errors.push(format!("automatic pass {index}: {error}"));
                    continue;
                }
            };
            let target = match previous {
                Some((_, target)) => Ok(*target),
                None => resources.target(self.resolution),
            };
            match target {
                Ok(target) => extended.passes.push((program, target)),
                Err(error) => errors.push(error),
            }
        }
        let max_units =
            unsafe { self.gl.get_parameter_i32(glow::MAX_TEXTURE_IMAGE_UNITS) } as usize;
        for program in programs
            .iter()
            .flatten()
            .chain(extended.passes.iter().map(|(program, _)| program))
        {
            let extra = program
                .loc_passes
                .iter()
                .filter(|loc| loc.is_some())
                .count()
                + program
                    .loc_history
                    .iter()
                    .flatten()
                    .filter(|loc| loc.is_some())
                    .count();
            if extra + 9 > max_units {
                errors.push("Extended shader exceeds available texture units".into());
            }
            for (index, location) in program.loc_passes.iter().enumerate() {
                if location.is_some() && index >= plan.passes.len() {
                    errors.push("Missing intermediate render pass".into());
                }
            }
            for (output, locations) in program.loc_history.iter().enumerate() {
                if locations
                    .iter()
                    .enumerate()
                    .any(|(index, loc)| loc.is_some() && index + 1 > plan.history[output])
                {
                    errors.push("Missing output history storage".into());
                }
            }
        }
        for (index, (program, _)) in extended.passes.iter().enumerate() {
            if program.loc_passes[index..].iter().any(Option::is_some) {
                errors.push("Automatic render passes must only sample earlier passes".into());
            }
        }
        if regions {
            let target = match self.snapshot.extended.region {
                Some(target) => Ok(target),
                None => resources.target([8 * crate::extended::MAX_REGIONS as u32, 8]),
            };
            match target {
                Ok(target) => extended.region = Some(target),
                Err(error) => errors.push(error),
            }
        }
        if !errors.is_empty() {
            return errors;
        }
        for (output, count) in plan.history.iter().enumerate() {
            if self.snapshot.extended.history[output].len() == *count {
                continue;
            }
            for _ in 0..*count {
                match resources.target(self.resolution) {
                    Ok(target) => extended.history[output].push(target),
                    Err(error) => errors.push(error),
                }
            }
        }
        if !errors.is_empty() {
            return errors;
        }
        let previous = &self.snapshot.extended;
        previous.readback_queue.lock().unwrap().destroy(&self.gl);
        for (output, count) in plan.history.iter().enumerate() {
            if previous.history[output].len() == *count {
                extended.history[output] = previous.history[output].clone();
                extended.cursor[output].store(
                    previous.cursor[output].load(Ordering::Relaxed),
                    Ordering::Relaxed,
                );
            } else {
                for target in &previous.history[output] {
                    unsafe {
                        self.gl.delete_texture(target.texture);
                        self.gl.delete_framebuffer(target.fbo);
                    }
                }
            }
        }
        for (index, (program, target)) in previous.passes.iter().enumerate() {
            unsafe {
                if extended.passes.get(index).map(|(state, _)| state.program)
                    != Some(program.program)
                {
                    self.gl.delete_program(program.program);
                }
                if index >= extended.passes.len() {
                    self.gl.delete_texture(target.texture);
                    self.gl.delete_framebuffer(target.fbo);
                }
            }
        }
        if let Some(target) = previous.region.filter(|_| !regions) {
            target.destroy(&self.gl);
        }
        for (index, state) in self.snapshot.programs.iter().enumerate() {
            if let Some(state) = state
                && programs[index].map(|program| program.program) != Some(state.program)
            {
                unsafe { self.gl.delete_program(state.program) };
            }
        }
        self.snapshot.programs = programs;
        self.snapshot.render_mode = render_mode;
        self.snapshot.extended = Arc::new(extended);
        self.shader_sources = shaders.clone();
        self.pass_sources = plan.passes.clone();
        resources.commit();
        errors
    }
}

fn set_texture_defaults(gl: &glow::Context) {
    unsafe {
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
    }
}

fn resolve_program_state(gl: &glow::Context, program: glow::Program) -> ProgramState {
    unsafe {
        ProgramState {
            program,
            loc_time: gl.get_uniform_location(program, "iTime"),
            loc_resolution: gl.get_uniform_location(program, "iResolution"),
            loc_mouse: gl.get_uniform_location(program, "iMouse"),
            loc_beat: gl.get_uniform_location(program, "iBeat"),
            loc_tempo: gl.get_uniform_location(program, "iTempo"),
            loc_phase: gl.get_uniform_location(program, "iPhase"),
            loc_fft: gl.get_uniform_location(program, "iFft"),
            loc_javascript_values: gl.get_uniform_location(program, "iJsValues"),
            loc_audio_rhythm: gl.get_uniform_location(program, "iAudioRhythm"),
            loc_midi_note: gl.get_uniform_location(program, "iMidiNote"),
            loc_midi_velocity: gl.get_uniform_location(program, "iMidiVelocity"),
            loc_midi_cc: gl.get_uniform_location(program, "iMidiCC"),
            loc_midi_cc_smoothed: gl.get_uniform_location(program, "iMidiCCSmoothed"),
            loc_midi_envelope: gl.get_uniform_location(program, "iMidiEnvelope"),
            loc_midi_aftertouch: gl.get_uniform_location(program, "iMidiAftertouch"),
            loc_midi_channel_aftertouch: gl.get_uniform_location(program, "iMidiChannelAftertouch"),
            loc_midi_values: gl.get_uniform_location(program, "iMidiValue"),
            loc_osc: gl.get_uniform_location(program, "iOsc"),
            loc_buffers: [
                gl.get_uniform_location(program, "iBuffer0"),
                gl.get_uniform_location(program, "iBuffer1"),
                gl.get_uniform_location(program, "iBuffer2"),
                gl.get_uniform_location(program, "iBuffer3"),
            ],
            loc_text0: gl.get_uniform_location(program, "iText0"),
            loc_sources: [
                gl.get_uniform_location(program, "iSource0"),
                gl.get_uniform_location(program, "iSource1"),
                gl.get_uniform_location(program, "iSource2"),
                gl.get_uniform_location(program, "iSource3"),
            ],
            loc_passes: std::array::from_fn(|index| {
                gl.get_uniform_location(program, &format!("iPass{index}"))
            }),
            loc_history: std::array::from_fn(|output| {
                std::array::from_fn(|delay| {
                    gl.get_uniform_location(program, &format!("iHistory{output}_{}", delay + 1))
                })
            }),
            loc_regions: gl.get_uniform_location(program, "iRegionValues"),
            loc_parameters: gl.get_uniform_location(program, "iParameterValues"),
        }
    }
}

fn create_buffer_target(gl: &glow::Context, resolution: [u32; 2]) -> BufferTarget {
    unsafe {
        let mut textures: [Option<glow::Texture>; 2] = [None, None];
        let mut fbos: [Option<glow::Framebuffer>; 2] = [None, None];

        for i in 0..2 {
            let tex = gl.create_texture().expect("create texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                resolution[0] as i32,
                resolution[1] as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                PixelUnpackData::Slice(None),
            );
            set_texture_defaults(gl);

            let fbo = gl.create_framebuffer().expect("create FBO");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );

            textures[i] = Some(tex);
            fbos[i] = Some(fbo);
        }

        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.bind_texture(glow::TEXTURE_2D, None);

        BufferTarget {
            fbo: [fbos[0].unwrap(), fbos[1].unwrap()],
            texture: [textures[0].unwrap(), textures[1].unwrap()],
        }
    }
}

pub fn render_multipass(
    gl: &glow::Context,
    snap: &RenderSnapshot,
    ping: &AtomicBool,
    u: RenderUniforms,
) {
    let write = ping.load(Ordering::Relaxed) as usize;
    let read = 1 - write;

    unsafe {
        let saved_fbo = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
        let mut saved_vp = [0_i32; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut saved_vp);
        let blend_was_on = gl.is_enabled(glow::BLEND);

        gl.disable(glow::BLEND);

        let res_w = u.resolution[0] as i32;
        let res_h = u.resolution[1] as i32;

        let passes = snap
            .extended
            .passes
            .iter()
            .map(|(program, target)| (program, target.fbo));
        let outputs = snap
            .programs
            .iter()
            .enumerate()
            .filter_map(|(index, program)| {
                program
                    .as_ref()
                    .map(|program| (program, snap.targets[index].fbo[write]))
            });
        for (p, target) in passes.chain(outputs) {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target));
            gl.viewport(0, 0, res_w, res_h);
            gl.use_program(Some(p.program));
            p.upload_uniforms(gl, &u, &snap.parameter_values);

            for (j, loc) in p.loc_buffers.iter().enumerate() {
                if let Some(loc) = loc {
                    gl.active_texture(glow::TEXTURE0 + j as u32);
                    gl.bind_texture(glow::TEXTURE_2D, Some(snap.targets[j].texture[read]));
                    gl.uniform_1_i32(Some(loc), j as i32);
                }
            }

            if let (Some(tex), Some(loc)) = (snap.text_texture, &p.loc_text0) {
                gl.active_texture(glow::TEXTURE0 + NUM_BUFFERS as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.uniform_1_i32(Some(loc), NUM_BUFFERS as i32);
            }

            for (j, tex) in snap.source_textures.iter().enumerate() {
                if let (Some(tex), Some(loc)) = (tex, &p.loc_sources[j]) {
                    let unit = (NUM_BUFFERS + 1 + j) as u32;
                    gl.active_texture(glow::TEXTURE0 + unit);
                    gl.bind_texture(glow::TEXTURE_2D, Some(*tex));
                    gl.uniform_1_i32(Some(loc), unit as i32);
                }
            }
            let mut unit = 9;
            for (index, loc) in p.loc_passes.iter().enumerate() {
                if let Some(loc) = loc {
                    let texture = snap.extended.passes[index].1.texture;
                    gl.active_texture(glow::TEXTURE0 + unit);
                    gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                    gl.uniform_1_i32(Some(loc), unit as i32);
                    unit += 1;
                }
            }
            for (output, locations) in p.loc_history.iter().enumerate() {
                let history = &snap.extended.history[output];
                for (index, location) in locations.iter().enumerate() {
                    if let Some(location) = location {
                        let cursor = snap.extended.cursor[output].load(Ordering::Relaxed);
                        let texture = history[(cursor + history.len()
                            - (index + 1) % history.len())
                            % history.len()]
                        .texture;
                        gl.active_texture(glow::TEXTURE0 + unit);
                        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                        gl.uniform_1_i32(Some(location), unit as i32);
                        unit += 1;
                    }
                }
            }
            gl.bind_vertex_array(Some(snap.vao));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }

        let scissor_was_on = gl.is_enabled(glow::SCISSOR_TEST);
        gl.disable(glow::SCISSOR_TEST);
        for (output, history) in snap.extended.history.iter().enumerate() {
            if history.is_empty() {
                continue;
            }
            let cursor = snap.extended.cursor[output].load(Ordering::Relaxed);
            gl.bind_framebuffer(
                glow::READ_FRAMEBUFFER,
                Some(snap.targets[output].fbo[write]),
            );
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(history[cursor].fbo));
            gl.blit_framebuffer(
                0,
                0,
                res_w,
                res_h,
                0,
                0,
                res_w,
                res_h,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            snap.extended.cursor[output].store((cursor + 1) % history.len(), Ordering::Relaxed);
        }

        if scissor_was_on {
            gl.enable(glow::SCISSOR_TEST);
        }

        ping.store(write == 0, Ordering::Relaxed);

        let restored_fbo = std::num::NonZeroU32::new(saved_fbo as u32).map(glow::NativeFramebuffer);
        gl.bind_framebuffer(glow::FRAMEBUFFER, restored_fbo);
        gl.viewport(saved_vp[0], saved_vp[1], saved_vp[2], saved_vp[3]);

        let d = &snap.display;
        gl.use_program(Some(d.program));

        if let Some(ref loc) = d.loc_resolution {
            gl.uniform_2_f32(Some(loc), u.resolution[0], u.resolution[1]);
        }

        match snap.render_mode {
            RenderMode::Single0 => {
                draw_display_buffer(gl, snap, d, write, 0);
            }
            RenderMode::Single(n) => {
                draw_display_buffer(gl, snap, d, write, n.min(NUM_BUFFERS - 1));
            }
            RenderMode::All => {
                let hw = saved_vp[2] / 2;
                let hh = saved_vp[3] / 2;
                // Quadrant positions paired with the buffer index hydra-synth's own
                // renderAll shader puts there. That shader derives the quadrant from
                // `floor(2*x)*2 + floor(2*(1-y))`, which works out to a diagonal
                // o0/o2/o1/o3 assignment (top-right is o2, bottom-left is o1) rather
                // than the row-major o0/o1/o2/o3 order it might look like at a glance.
                for ((vx, vy), buf_idx) in [(0, hh), (hw, hh), (0, 0), (hw, 0)]
                    .iter()
                    .zip([0, 2, 1, 3])
                {
                    gl.viewport(saved_vp[0] + vx, saved_vp[1] + vy, hw, hh);
                    draw_display_buffer(gl, snap, d, write, buf_idx);
                }
                gl.viewport(saved_vp[0], saved_vp[1], saved_vp[2], saved_vp[3]);
            }
        }

        gl.bind_vertex_array(None);
        gl.use_program(None);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, None);

        if blend_was_on {
            gl.enable(glow::BLEND);
        }
    }
}

pub fn render_output_preview(
    gl: &glow::Context,
    snapshot: &RenderSnapshot,
    ping: &AtomicBool,
    output: usize,
    viewport: [i32; 4],
) -> Result<(), String> {
    if output >= NUM_BUFFERS || viewport[2] <= 0 || viewport[3] <= 0 {
        return Err("Invalid output preview index or viewport".into());
    }
    unsafe {
        let mut previous_viewport = [0; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut previous_viewport);
        let program = gl.get_parameter_i32(glow::CURRENT_PROGRAM);
        let vao = gl.get_parameter_i32(glow::VERTEX_ARRAY_BINDING);
        let active = gl.get_parameter_i32(glow::ACTIVE_TEXTURE);
        gl.active_texture(glow::TEXTURE0);
        let texture = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D);
        let sampler = gl.get_parameter_i32(glow::SAMPLER_BINDING);
        let blend = gl.is_enabled(glow::BLEND);
        gl.disable(glow::BLEND);
        gl.bind_sampler(0, None);
        gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        gl.bind_vertex_array(Some(snapshot.vao));
        gl.use_program(Some(snapshot.display.program));
        let latest = 1 - ping.load(Ordering::Relaxed) as usize;
        draw_display_buffer(gl, snapshot, &snapshot.display, latest, output);
        gl.bind_texture(
            glow::TEXTURE_2D,
            std::num::NonZeroU32::new(texture as u32).map(glow::NativeTexture),
        );
        gl.bind_sampler(
            0,
            std::num::NonZeroU32::new(sampler as u32).map(glow::NativeSampler),
        );
        gl.active_texture(active as u32);
        gl.bind_vertex_array(std::num::NonZeroU32::new(vao as u32).map(glow::NativeVertexArray));
        gl.use_program(std::num::NonZeroU32::new(program as u32).map(glow::NativeProgram));
        gl.viewport(
            previous_viewport[0],
            previous_viewport[1],
            previous_viewport[2],
            previous_viewport[3],
        );
        if blend {
            gl.enable(glow::BLEND);
        }
    }
    Ok(())
}

pub fn read_capture(
    gl: &glow::Context,
    snapshot: &RenderSnapshot,
    source_resolution: [u32; 2],
) -> Result<(u32, u32, Vec<u8>), String> {
    if source_resolution.contains(&0)
        || source_resolution
            .iter()
            .any(|dimension| *dimension > i32::MAX as u32)
    {
        return Err("Capture requires valid render dimensions".into());
    }
    let (target, [width, height]) = snapshot.capture.ok_or("Capture target is not configured")?;
    let mut pixels = vec![0; width as usize * height as usize * 3];
    unsafe {
        let read = gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING);
        let draw = gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
        let alignment = gl.get_parameter_i32(glow::PACK_ALIGNMENT);
        let scissor = gl.is_enabled(glow::SCISSOR_TEST);
        gl.disable(glow::SCISSOR_TEST);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(target.fbo));
        gl.blit_framebuffer(
            0,
            0,
            source_resolution[0] as i32,
            source_resolution[1] as i32,
            0,
            0,
            width as i32,
            height as i32,
            glow::COLOR_BUFFER_BIT,
            glow::LINEAR,
        );
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(target.fbo));
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
        gl.read_pixels(
            0,
            0,
            width as i32,
            height as i32,
            glow::RGB,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
        gl.bind_framebuffer(
            glow::READ_FRAMEBUFFER,
            std::num::NonZeroU32::new(read as u32).map(glow::NativeFramebuffer),
        );
        gl.bind_framebuffer(
            glow::DRAW_FRAMEBUFFER,
            std::num::NonZeroU32::new(draw as u32).map(glow::NativeFramebuffer),
        );
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, alignment);
        if scissor {
            gl.enable(glow::SCISSOR_TEST);
        }
        let error = gl.get_error();
        if error != glow::NO_ERROR {
            return Err(format!("Capture readback failed: OpenGL error {error:#x}"));
        }
    }
    Ok((width, height, pixels))
}

fn draw_display_buffer(
    gl: &glow::Context,
    snap: &RenderSnapshot,
    d: &ProgramState,
    write: usize,
    buf_idx: usize,
) {
    unsafe {
        if let Some(ref loc) = d.loc_buffers[0] {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(snap.targets[buf_idx].texture[write]));
            gl.uniform_1_i32(Some(loc), 0);
        }
        gl.bind_vertex_array(Some(snap.vao));
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
    }
}

fn create_fullscreen_quad(gl: &glow::Context) -> (glow::VertexArray, glow::Buffer) {
    #[rustfmt::skip]
    let vertices: [f32; 8] = [
        -1.0, -1.0,
         1.0, -1.0,
        -1.0,  1.0,
         1.0,  1.0,
    ];

    unsafe {
        let vao = gl.create_vertex_array().expect("create VAO");
        let vbo = gl.create_buffer().expect("create VBO");

        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            as_u8_slice(&vertices),
            glow::STATIC_DRAW,
        );
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
        gl.bind_vertex_array(None);

        (vao, vbo)
    }
}

fn as_u8_slice(data: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) }
}

fn compile_vertex_shader(gl: &glow::Context) -> Result<glow::Shader, String> {
    unsafe {
        let vert = gl
            .create_shader(glow::VERTEX_SHADER)
            .map_err(|e| e.to_string())?;
        gl.shader_source(vert, shader::vertex_source());
        gl.compile_shader(vert);
        if !gl.get_shader_compile_status(vert) {
            let log = gl.get_shader_info_log(vert);
            gl.delete_shader(vert);
            return Err(format!("vertex shader: {log}"));
        }
        Ok(vert)
    }
}

fn compile_program(
    gl: &glow::Context,
    vert: glow::Shader,
    frag_src: &str,
) -> Result<glow::Program, String> {
    unsafe {
        let frag = gl
            .create_shader(glow::FRAGMENT_SHADER)
            .map_err(|e| e.to_string())?;
        gl.shader_source(frag, frag_src);
        gl.compile_shader(frag);
        if !gl.get_shader_compile_status(frag) {
            let log = gl.get_shader_info_log(frag);
            gl.delete_shader(frag);
            return Err(format!("fragment shader: {log}"));
        }

        let program = match gl.create_program() {
            Ok(program) => program,
            Err(error) => {
                gl.delete_shader(frag);
                return Err(error);
            }
        };
        gl.attach_shader(program, vert);
        gl.attach_shader(program, frag);
        gl.link_program(program);
        gl.detach_shader(program, vert);
        gl.detach_shader(program, frag);
        gl.delete_shader(frag);

        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(format!("link: {log}"));
        }

        Ok(program)
    }
}
