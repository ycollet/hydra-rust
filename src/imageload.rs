//! Loads a static image from a URL in the background for `initImage()`
//! (see eval.rs), delivering it once as a `SourceFrame` through the same
//! texture-slot upload path webcam frames use (`ShaderRenderer::
//! upload_source`, see renderer.rs). Unlike a camera, a fetched image never
//! changes, so this only ever needs to poll for a single completed frame,
//! not a continuous stream - the resulting `poll()` shape still matches
//! `source::SourceManager::poll` so `app.rs` can treat both the same way.

#[cfg(feature = "image_url")]
mod imp {
    use std::io::Read;
    use std::sync::mpsc::{self, Receiver};
    use std::thread;

    use crate::source::{SourceFrame, NUM_SOURCES};

    #[derive(Default)]
    pub struct ImageManager {
        pending: [Option<Receiver<SourceFrame>>; NUM_SOURCES],
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
            thread::Builder::new()
                .name(format!("hydra-image-{slot}"))
                .spawn(move || match fetch_and_decode(&url) {
                    Ok(frame) => {
                        let _ = tx.send(frame);
                    }
                    Err(e) => log::warn!("initImage: failed to load {url:?}: {e}"),
                })
                .expect("spawn image-load thread");
        }

        /// Returns the freshly loaded frame for `slot` exactly once, the
        /// first time it's ready after `init_image` - `None` before and
        /// after that, since a static image is only ever uploaded once.
        pub fn poll(&mut self, slot: usize) -> Option<SourceFrame> {
            let rx = self.pending.get(slot)?.as_ref()?;
            match rx.try_recv() {
                Ok(frame) => {
                    self.pending[slot] = None;
                    Some(frame)
                }
                Err(_) => None,
            }
        }
    }

    fn fetch_and_decode(url: &str) -> Result<SourceFrame, String> {
        decode_bytes(&fetch_bytes(url)?)
    }

    fn decode_bytes(bytes: &[u8]) -> Result<SourceFrame, String> {
        let img = image::load_from_memory(bytes)
            .map_err(|e| format!("decode error: {e}"))?
            .into_rgb8();
        let (width, height) = img.dimensions();
        Ok(SourceFrame { pixels: img.into_raw(), width, height })
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

        #[test]
        fn decode_bytes_produces_a_source_frame_matching_the_original_image() {
            let mut img = image::RgbImage::new(3, 2);
            for (i, px) in img.pixels_mut().enumerate() {
                *px = image::Rgb([i as u8, (i * 2) as u8, (i * 3) as u8]);
            }
            let mut png_bytes = Vec::new();
            image::DynamicImage::ImageRgb8(img.clone())
                .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
                .unwrap();

            let frame = decode_bytes(&png_bytes).unwrap();
            assert_eq!(frame.width, 3);
            assert_eq!(frame.height, 2);
            assert_eq!(frame.pixels, img.into_raw());
        }

        #[test]
        fn decode_bytes_rejects_non_image_data() {
            assert!(decode_bytes(b"not an image").is_err());
        }

        #[test]
        fn init_image_on_an_out_of_range_slot_does_not_panic() {
            let mut mgr = ImageManager::new();
            mgr.init_image(NUM_SOURCES, "http://example.invalid/x.png".into());
            assert!(mgr.poll(0).is_none());
        }
    }
}

#[cfg(feature = "image_url")]
pub use imp::*;
