//! Loads a static image or an animated GIF from a URL in the background
//! for `initImage()`/`initGif()` (see eval.rs), delivering frames through
//! the same texture-slot upload path webcam frames use (`ShaderRenderer::
//! upload_source`, see renderer.rs). A static image only ever needs
//! uploading once; an animated GIF's frames are all decoded up front
//! (GIFs are small, short loops - no need for a streaming decoder) and
//! then cycled through by elapsed real time, looping indefinitely, the
//! same way a real `<img>`/canvas GIF loops in a browser.

#[cfg(feature = "image_url")]
mod imp {
    use std::io::Read;
    use std::sync::mpsc::{self, Receiver};
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::source::{SourceFrame, NUM_SOURCES};

    enum DecodedMedia {
        Static(SourceFrame),
        /// Each frame paired with its own display duration in milliseconds.
        Animated(Vec<(SourceFrame, u32)>),
    }

    struct AnimatedState {
        frames: Vec<(SourceFrame, u32)>,
        total_duration_ms: u32,
        started_at: Instant,
        last_index: usize,
    }

    #[derive(Default)]
    pub struct ImageManager {
        pending: [Option<Receiver<DecodedMedia>>; NUM_SOURCES],
        animated: [Option<AnimatedState>; NUM_SOURCES],
    }

    impl ImageManager {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn init_image(&mut self, slot: usize, url: String) {
            if slot >= NUM_SOURCES {
                return;
            }
            let (tx, rx) = mpsc::channel();
            self.pending[slot] = Some(rx);
            self.animated[slot] = None;
            thread::Builder::new()
                .name(format!("hydra-image-{slot}"))
                .spawn(move || match fetch_bytes(&url).and_then(|b| decode_static(&b)) {
                    Ok(frame) => {
                        let _ = tx.send(DecodedMedia::Static(frame));
                    }
                    Err(e) => log::warn!("initImage: failed to load {url:?}: {e}"),
                })
                .expect("spawn image-load thread");
        }

        pub fn init_gif(&mut self, slot: usize, url: String) {
            if slot >= NUM_SOURCES {
                return;
            }
            let (tx, rx) = mpsc::channel();
            self.pending[slot] = Some(rx);
            self.animated[slot] = None;
            thread::Builder::new()
                .name(format!("hydra-gif-{slot}"))
                .spawn(move || match fetch_bytes(&url).and_then(|b| decode_gif_frames(&b)) {
                    Ok(frames) => {
                        let _ = tx.send(DecodedMedia::Animated(frames));
                    }
                    Err(e) => log::warn!("initGif: failed to load {url:?}: {e}"),
                })
                .expect("spawn gif-load thread");
        }

        /// Returns a frame to upload for `slot`, if one is newly ready:
        /// either the just-finished static image / first GIF frame, or
        /// (for an already-loaded GIF) the next animation frame once
        /// enough real time has elapsed - `None` otherwise, so callers
        /// don't needlessly re-upload an unchanged frame every render frame.
        pub fn poll(&mut self, slot: usize) -> Option<SourceFrame> {
            if let Some(rx) = self.pending.get(slot).and_then(|r| r.as_ref())
                && let Ok(media) = rx.try_recv()
            {
                self.pending[slot] = None;
                return match media {
                    DecodedMedia::Static(frame) => {
                        self.animated[slot] = None;
                        Some(frame)
                    }
                    DecodedMedia::Animated(frames) => {
                        let total = frames.iter().map(|(_, d)| *d).sum::<u32>().max(1);
                        let first = frames[0].0.clone();
                        self.animated[slot] = Some(AnimatedState {
                            frames,
                            total_duration_ms: total,
                            started_at: Instant::now(),
                            last_index: 0,
                        });
                        Some(first)
                    }
                };
            }

            let state = self.animated.get_mut(slot)?.as_mut()?;
            let idx = frame_index_at(
                state.started_at.elapsed().as_millis() as u32,
                state.total_duration_ms,
                &state.frames,
            );
            if idx == state.last_index {
                return None;
            }
            state.last_index = idx;
            Some(state.frames[idx].0.clone())
        }
    }

    /// Which frame of an animation (each with its own display duration in
    /// `frames`, total `total_duration_ms`) should be showing at
    /// `elapsed_ms` since it started - wrapping around once the whole
    /// animation has looped.
    fn frame_index_at(elapsed_ms: u32, total_duration_ms: u32, frames: &[(SourceFrame, u32)]) -> usize {
        let position = elapsed_ms % total_duration_ms;
        let mut acc = 0u32;
        for (i, (_, delay)) in frames.iter().enumerate() {
            acc += delay;
            if position < acc {
                return i;
            }
        }
        frames.len() - 1
    }

    fn decode_static(bytes: &[u8]) -> Result<SourceFrame, String> {
        let img = image::load_from_memory(bytes)
            .map_err(|e| format!("decode error: {e}"))?
            .into_rgb8();
        let (width, height) = img.dimensions();
        Ok(SourceFrame { pixels: img.into_raw(), width, height })
    }

    fn decode_gif_frames(bytes: &[u8]) -> Result<Vec<(SourceFrame, u32)>, String> {
        use image::codecs::gif::GifDecoder;
        use image::AnimationDecoder;

        let decoder =
            GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| format!("gif decode error: {e}"))?;
        let frames = decoder
            .into_frames()
            .collect_frames()
            .map_err(|e| format!("gif frame decode error: {e}"))?;
        if frames.is_empty() {
            return Err("gif has no frames".to_string());
        }

        Ok(frames
            .into_iter()
            .map(|f| {
                // At least 1ms so a (theoretically) zero-delay frame can't
                // divide-by-zero the total duration or stall frame_index_at.
                let delay_ms = Duration::from(f.delay()).as_millis().max(1) as u32;
                let rgb = image::DynamicImage::ImageRgba8(f.into_buffer()).into_rgb8();
                let (width, height) = rgb.dimensions();
                (SourceFrame { pixels: rgb.into_raw(), width, height }, delay_ms)
            })
            .collect())
    }

    fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
        let response = ureq::get(url)
            .call()
            .map_err(|e| format!("request error: {e}"))?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read error: {e}"))?;
        Ok(bytes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn solid_frame(w: u32, h: u32, v: u8) -> SourceFrame {
            SourceFrame { pixels: vec![v; (w * h * 3) as usize], width: w, height: h }
        }

        #[test]
        fn decode_static_produces_a_source_frame_matching_the_original_image() {
            let mut img = image::RgbImage::new(3, 2);
            for (i, px) in img.pixels_mut().enumerate() {
                *px = image::Rgb([i as u8, (i * 2) as u8, (i * 3) as u8]);
            }
            let mut png_bytes = Vec::new();
            image::DynamicImage::ImageRgb8(img.clone())
                .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
                .unwrap();

            let frame = decode_static(&png_bytes).unwrap();
            assert_eq!(frame.width, 3);
            assert_eq!(frame.height, 2);
            assert_eq!(frame.pixels, img.into_raw());
        }

        #[test]
        fn decode_static_rejects_non_image_data() {
            assert!(decode_static(b"not an image").is_err());
        }

        #[test]
        fn init_image_on_an_out_of_range_slot_does_not_panic() {
            let mut mgr = ImageManager::new();
            mgr.init_image(NUM_SOURCES, "http://example.invalid/x.png".into());
            assert!(mgr.poll(0).is_none());
        }

        #[test]
        fn init_gif_on_an_out_of_range_slot_does_not_panic() {
            let mut mgr = ImageManager::new();
            mgr.init_gif(NUM_SOURCES, "http://example.invalid/x.gif".into());
            assert!(mgr.poll(0).is_none());
        }

        #[test]
        fn frame_index_at_picks_the_frame_covering_the_elapsed_time() {
            let frames =
                vec![(solid_frame(1, 1, 0), 100), (solid_frame(1, 1, 1), 100), (solid_frame(1, 1, 2), 100)];
            assert_eq!(frame_index_at(0, 300, &frames), 0);
            assert_eq!(frame_index_at(99, 300, &frames), 0);
            assert_eq!(frame_index_at(100, 300, &frames), 1);
            assert_eq!(frame_index_at(250, 300, &frames), 2);
        }

        #[test]
        fn frame_index_at_loops_back_to_the_start() {
            let frames = vec![(solid_frame(1, 1, 0), 100), (solid_frame(1, 1, 1), 100)];
            assert_eq!(frame_index_at(200, 200, &frames), 0);
            assert_eq!(frame_index_at(250, 200, &frames), 0);
            assert_eq!(frame_index_at(350, 200, &frames), 1);
        }

        #[test]
        fn decode_gif_frames_produces_every_frame_with_its_delay() {
            // A minimal 2-frame animated GIF (2x2, red then blue, 10ms
            // delay each) built with the `image` crate's own encoder so
            // this test doesn't depend on any external file.
            let mut bytes = Vec::new();
            {
                let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
                for color in [[255u8, 0, 0], [0, 0, 255]] {
                    let mut img = image::RgbaImage::new(2, 2);
                    for px in img.pixels_mut() {
                        *px = image::Rgba([color[0], color[1], color[2], 255]);
                    }
                    let frame = image::Frame::from_parts(
                        img,
                        0,
                        0,
                        image::Delay::from_saturating_duration(std::time::Duration::from_millis(10)),
                    );
                    encoder.encode_frame(frame).unwrap();
                }
            }

            let frames = decode_gif_frames(&bytes).unwrap();
            assert_eq!(frames.len(), 2);
            for (frame, delay_ms) in &frames {
                assert_eq!(frame.width, 2);
                assert_eq!(frame.height, 2);
                assert!(*delay_ms >= 1);
            }
        }
    }
}

#[cfg(feature = "image_url")]
pub use imp::*;
