use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use glow::HasContext;

use super::RenderSnapshot;
use crate::extended::{MAX_REGIONS, Region};

const BYTES: usize = 8 * 8 * 4 * MAX_REGIONS;
const CAPACITY: usize = 3;

struct Pending {
    buffer: glow::Buffer,
    // Opaque addresses keep snapshots transferable; only the render thread uses these context handles.
    fence: usize,
    channels: [usize; MAX_REGIONS],
    count: usize,
}

impl Pending {
    fn fence(&self) -> glow::Fence {
        glow::NativeFence(std::ptr::with_exposed_provenance_mut(self.fence))
    }
}

#[derive(Default)]
pub(super) struct ReadbackQueue {
    buffers: Vec<glow::Buffer>,
    pending: VecDeque<Pending>,
    last_warning: Option<Instant>,
}

impl ReadbackQueue {
    pub(super) fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            for pending in self.pending.drain(..) {
                gl.delete_sync(pending.fence());
            }
            for buffer in self.buffers.drain(..) {
                gl.delete_buffer(buffer);
            }
        }
        self.last_warning = None;
    }

    fn initialize(&mut self, gl: &glow::Context) -> Result<(), String> {
        if !self.buffers.is_empty() {
            return Ok(());
        }
        for _ in 0..CAPACITY {
            let result = (|| unsafe {
                let buffer = gl.create_buffer()?;
                self.buffers.push(buffer);
                gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(buffer));
                gl.buffer_data_size(glow::PIXEL_PACK_BUFFER, BYTES as i32, glow::STREAM_READ);
                check(gl)
            })();
            if let Err(error) = result {
                self.destroy(gl);
                return Err(error);
            }
        }
        Ok(())
    }

    fn poll(&mut self, gl: &glow::Context) -> Result<Option<[f32; MAX_REGIONS]>, String> {
        let mut latest = None;
        while let Some(pending) = self.pending.front() {
            let status = unsafe { gl.client_wait_sync(pending.fence(), 0, 0) };
            match status {
                glow::TIMEOUT_EXPIRED => break,
                glow::ALREADY_SIGNALED | glow::CONDITION_SATISFIED => {}
                _ => return Err("Asynchronous region readback fence failed".into()),
            }
            let pending = self.pending.pop_front().unwrap();
            let mut pixels = [0; BYTES];
            unsafe {
                gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(pending.buffer));
                gl.get_buffer_sub_data(
                    glow::PIXEL_PACK_BUFFER,
                    0,
                    &mut pixels[..pending.count * 8 * 8 * 4],
                );
                gl.delete_sync(pending.fence());
            }
            check(gl)?;
            latest = Some(reduce(&pixels, &pending.channels[..pending.count]));
        }
        Ok(latest)
    }
}

struct SavedState<'a> {
    gl: &'a glow::Context,
    read: i32,
    draw: i32,
    pack: i32,
    layout: [i32; 4],
    scissor: bool,
}

impl<'a> SavedState<'a> {
    fn new(gl: &'a glow::Context) -> Self {
        unsafe {
            let state = Self {
                gl,
                read: gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING),
                draw: gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING),
                pack: gl.get_parameter_i32(glow::PIXEL_PACK_BUFFER_BINDING),
                layout: [
                    glow::PACK_ALIGNMENT,
                    glow::PACK_ROW_LENGTH,
                    glow::PACK_SKIP_ROWS,
                    glow::PACK_SKIP_PIXELS,
                ]
                .map(|field| gl.get_parameter_i32(field)),
                scissor: gl.is_enabled(glow::SCISSOR_TEST),
            };
            gl.disable(glow::SCISSOR_TEST);
            for (field, value) in [
                glow::PACK_ALIGNMENT,
                glow::PACK_ROW_LENGTH,
                glow::PACK_SKIP_ROWS,
                glow::PACK_SKIP_PIXELS,
            ]
            .into_iter()
            .zip([1, 0, 0, 0])
            {
                gl.pixel_store_i32(field, value);
            }
            state
        }
    }
}

impl Drop for SavedState<'_> {
    fn drop(&mut self) {
        unsafe {
            self.gl.bind_framebuffer(
                glow::READ_FRAMEBUFFER,
                std::num::NonZeroU32::new(self.read as u32).map(glow::NativeFramebuffer),
            );
            self.gl.bind_framebuffer(
                glow::DRAW_FRAMEBUFFER,
                std::num::NonZeroU32::new(self.draw as u32).map(glow::NativeFramebuffer),
            );
            self.gl.bind_buffer(
                glow::PIXEL_PACK_BUFFER,
                std::num::NonZeroU32::new(self.pack as u32).map(glow::NativeBuffer),
            );
            for (field, value) in [
                glow::PACK_ALIGNMENT,
                glow::PACK_ROW_LENGTH,
                glow::PACK_SKIP_ROWS,
                glow::PACK_SKIP_PIXELS,
            ]
            .into_iter()
            .zip(self.layout)
            {
                self.gl.pixel_store_i32(field, value);
            }
            if self.scissor {
                self.gl.enable(glow::SCISSOR_TEST);
            }
        }
    }
}

fn check(gl: &glow::Context) -> Result<(), String> {
    let error = unsafe { gl.get_error() };
    if error == glow::NO_ERROR {
        Ok(())
    } else {
        Err(format!("Region readback failed: OpenGL error {error:#x}"))
    }
}

fn validate(resolution: [u32; 2], regions: &[Region]) -> Result<(), String> {
    if regions.len() > MAX_REGIONS {
        return Err("Image region limit exceeded".into());
    }
    for region in regions {
        region.validate()?;
    }
    if resolution.contains(&0)
        || resolution
            .iter()
            .any(|dimension| *dimension > i32::MAX as u32)
    {
        return Err("Region readback requires valid render dimensions".into());
    }
    Ok(())
}

fn blit(
    gl: &glow::Context,
    snapshot: &RenderSnapshot,
    ping: &AtomicBool,
    resolution: [u32; 2],
    regions: &[Region],
) -> Result<(), String> {
    let target = snapshot
        .extended
        .region
        .ok_or("Region readback target is not configured")?;
    let latest = 1 - ping.load(Ordering::Relaxed) as usize;
    unsafe {
        for (slot, region) in regions.iter().enumerate() {
            let [x, y, width, height] = region.rect;
            gl.bind_framebuffer(
                glow::READ_FRAMEBUFFER,
                Some(snapshot.targets[region.output].fbo[latest]),
            );
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(target.fbo));
            gl.blit_framebuffer(
                (x * resolution[0] as f32).floor() as i32,
                (y * resolution[1] as f32).floor() as i32,
                ((x + width) * resolution[0] as f32).ceil() as i32,
                ((y + height) * resolution[1] as f32).ceil() as i32,
                slot as i32 * 8,
                0,
                (slot as i32 + 1) * 8,
                8,
                glow::COLOR_BUFFER_BIT,
                glow::LINEAR,
            );
        }
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(target.fbo));
    }
    check(gl)
}

fn reduce(pixels: &[u8; BYTES], channels: &[usize]) -> [f32; MAX_REGIONS] {
    let mut values = [0.0; MAX_REGIONS];
    let width = 8 * channels.len();
    for (slot, channel) in channels.iter().enumerate() {
        let total: f32 = (0..8)
            .flat_map(|row| {
                let start = (row * width + slot * 8) * 4;
                pixels[start..start + 8 * 4].as_chunks::<4>().0.iter()
            })
            .map(|pixel| {
                if *channel == 4 {
                    (pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722)
                        / 255.0
                } else {
                    pixel[*channel] as f32 / 255.0
                }
            })
            .sum();
        values[slot] = total / 64.0;
    }
    values
}

pub fn read_regions(
    gl: &glow::Context,
    snapshot: &RenderSnapshot,
    ping: &AtomicBool,
    resolution: [u32; 2],
    regions: &[Region],
) -> Result<[f32; MAX_REGIONS], String> {
    if regions.is_empty() {
        return Ok([0.0; MAX_REGIONS]);
    }
    validate(resolution, regions)?;
    let _state = SavedState::new(gl);
    blit(gl, snapshot, ping, resolution, regions)?;
    let mut pixels = [0; BYTES];
    unsafe {
        gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
        gl.read_pixels(
            0,
            0,
            regions.len() as i32 * 8,
            8,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels[..regions.len() * 8 * 8 * 4])),
        );
    }
    check(gl)?;
    let channels: [usize; MAX_REGIONS] =
        std::array::from_fn(|index| regions.get(index).map_or(0, |region| region.channel));
    Ok(reduce(&pixels, &channels[..regions.len()]))
}

pub fn read_regions_async(
    gl: &glow::Context,
    snapshot: &RenderSnapshot,
    ping: &AtomicBool,
    resolution: [u32; 2],
    regions: &[Region],
    submit: bool,
) -> Result<Option<[f32; MAX_REGIONS]>, String> {
    validate(resolution, regions)?;
    let _state = SavedState::new(gl);
    let mut queue = snapshot.extended.readback_queue.lock().unwrap();
    let latest = match queue.poll(gl) {
        Ok(values) => values,
        Err(error) => {
            queue.destroy(gl);
            return Err(error);
        }
    };
    if !submit || regions.is_empty() {
        return Ok(latest);
    }
    queue.initialize(gl)?;
    let buffer = queue.buffers.iter().copied().find(|buffer| {
        !queue
            .pending
            .iter()
            .any(|pending| pending.buffer == *buffer)
    });
    let Some(buffer) = buffer else {
        let now = Instant::now();
        if queue
            .last_warning
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
        {
            log::warn!(
                "Region readback queue full; keeping completed values and skipping new samples"
            );
            queue.last_warning = Some(now);
        }
        return Ok(latest);
    };
    blit(gl, snapshot, ping, resolution, regions)?;
    unsafe {
        gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(buffer));
        gl.read_pixels(
            0,
            0,
            regions.len() as i32 * 8,
            8,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::BufferOffset(0),
        );
        check(gl)?;
        let fence = gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0)?;
        queue.pending.push_back(Pending {
            buffer,
            fence: fence.0.expose_provenance(),
            channels: std::array::from_fn(|index| {
                regions.get(index).map_or(0, |region| region.channel)
            }),
            count: regions.len(),
        });
        gl.flush();
    }

    check(gl)?;
    Ok(latest)
}
