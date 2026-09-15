//! Streams video frames from a local file or URL in the background for
//! `initVideo()` (see eval.rs), delivering them through the same
//! texture-slot upload path webcam/image/GIF frames use (`ShaderRenderer::
//! upload_source`, see renderer.rs).
//!
//! Decoding goes through a standalone `ffmpeg` binary
//! (github.com/nathanbabcock/ffmpeg-sidecar), spawned as a subprocess and
//! read over a pipe - `ffmpeg` is never linked into this binary, so
//! building hydra-rust itself never needs FFmpeg's dev libraries/headers.
//! This does mean `ffmpeg` must be present on `PATH` at runtime; unlike
//! `ffmpeg-sidecar`'s own optional auto-download feature (deliberately
//! not enabled - see `Cargo.toml`), a missing `ffmpeg` just logs one clear
//! warning and leaves the slot empty, the same "no surprise background
//! network/hardware access" treatment already given to the camera/
//! microphone/MIDI features.
//!
//! Unlike a static image (`imageload.rs`), a video is decoded as an
//! unbounded, continuous stream rather than "decode everything up front" -
//! closer in shape to a camera (`source.rs`'s `CameraSlot`): a background
//! thread owns the decode process for as long as the slot is playing, and
//! `poll()` just drains whatever the newest decoded frame is.

#[cfg(feature = "video")]
mod imp {
    use std::sync::{Arc, Mutex};
    use std::thread;

    use ffmpeg_sidecar::child::FfmpegChild;
    use ffmpeg_sidecar::command::FfmpegCommand;

    use crate::source::{SourceFrame, NUM_SOURCES};

    struct VideoSlot {
        child: Arc<Mutex<FfmpegChild>>,
        latest: Arc<Mutex<Option<SourceFrame>>>,
    }

    impl Drop for VideoSlot {
        fn drop(&mut self) {
            // Stop the decode process when the slot is replaced or the
            // manager itself is dropped - otherwise it would keep decoding
            // (and looping, per `-stream_loop -1`) forever in the
            // background with nothing left reading its output.
            let _ = self.child.lock().unwrap().kill();
        }
    }

    #[derive(Default)]
    pub struct VideoManager {
        slots: [Option<VideoSlot>; NUM_SOURCES],
        warned_missing_ffmpeg: bool,
    }

    impl VideoManager {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn init_video(&mut self, slot: usize, url: String) {
            if slot >= NUM_SOURCES {
                return;
            }

            if !ffmpeg_sidecar::command::ffmpeg_is_installed() {
                if !self.warned_missing_ffmpeg {
                    self.warned_missing_ffmpeg = true;
                    log::warn!(
                        "initVideo: `ffmpeg` was not found on PATH - install it \
                         (e.g. `brew install ffmpeg`, `apt install ffmpeg`) to use initVideo()"
                    );
                }
                return;
            }

            // Replace whatever was previously playing in this slot - the
            // outgoing `VideoSlot`'s `Drop` impl stops its ffmpeg process.
            self.slots[slot] = None;

            let mut command = FfmpegCommand::new();
            command
                // Input-side options - must precede `.input(...)`: loop
                // the file indefinitely, and pace output to the video's
                // own real playback rate rather than "as fast as possible"
                // (a `poll()`-per-render-frame consumer wants frames to
                // arrive roughly in real time, not all at once).
                .args(["-stream_loop", "-1", "-re"])
                .input(&url)
                .rawvideo();

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(e) => {
                    log::warn!("initVideo: failed to start ffmpeg for {url:?}: {e}");
                    return;
                }
            };
            let iter = match child.iter() {
                Ok(iter) => iter,
                Err(e) => {
                    log::warn!("initVideo: failed to read ffmpeg output for {url:?}: {e}");
                    let _ = child.kill();
                    return;
                }
            };

            let latest = Arc::new(Mutex::new(None));
            let latest_writer = latest.clone();
            thread::Builder::new()
                .name(format!("hydra-video-{slot}"))
                .spawn(move || {
                    for frame in iter.filter_frames() {
                        let source_frame = SourceFrame {
                            pixels: frame.data,
                            width: frame.width,
                            height: frame.height,
                        };
                        *latest_writer.lock().unwrap() = Some(source_frame);
                    }
                })
                .expect("spawn video-decode thread");

            self.slots[slot] = Some(VideoSlot { child: Arc::new(Mutex::new(child)), latest });
        }

        /// Returns the newest decoded frame for `slot`, if the background
        /// decode thread has produced one since the last call - `None`
        /// otherwise, so callers don't needlessly re-upload an unchanged
        /// frame every render frame.
        pub fn poll(&mut self, slot: usize) -> Option<SourceFrame> {
            self.slots.get(slot)?.as_ref()?.latest.lock().unwrap().take()
        }

        /// Stops every playing video - called on app exit, mirroring
        /// `SourceManager::stop_all` for the camera.
        pub fn stop_all(&mut self) {
            for slot in &mut self.slots {
                *slot = None;
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // These deliberately don't assume whether `ffmpeg` happens to be
        // installed in whatever environment runs the tests - both paths
        // (missing ffmpeg -> warn-and-skip; present but given a bad
        // path/URL -> ffmpeg itself fails quickly) must behave safely.

        #[test]
        fn init_video_on_an_out_of_range_slot_does_not_panic() {
            let mut mgr = VideoManager::new();
            mgr.init_video(NUM_SOURCES, "does-not-matter.mp4".into());
            assert!(mgr.poll(0).is_none());
        }

        #[test]
        fn poll_on_a_never_used_slot_is_none() {
            let mut mgr = VideoManager::new();
            assert!(mgr.poll(0).is_none());
        }

        #[test]
        fn init_video_with_a_bad_path_never_panics_and_eventually_polls_none() {
            let mut mgr = VideoManager::new();
            mgr.init_video(0, "/nonexistent/path/does-not-exist.mp4".into());
            // Whether ffmpeg is installed or not, nothing should ever
            // produce a frame for a file that doesn't exist.
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(mgr.poll(0).is_none());
        }

        #[test]
        fn stop_all_clears_every_slot() {
            let mut mgr = VideoManager::new();
            mgr.init_video(0, "does-not-matter.mp4".into());
            mgr.stop_all();
            assert!(mgr.poll(0).is_none());
        }
    }
}

#[cfg(feature = "video")]
pub use imp::*;
