//! Loads images and animated GIFs with bounded downloads, decoded storage, and background workers.

#[cfg(feature = "image_url")]
mod imp {
    use std::borrow::Cow;
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::source::{NUM_SOURCES, SourceFrame};

    const MAX_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;
    const MAX_MEDIA_BYTES: usize = 64 * 1024 * 1024;
    const MAX_SHARED_BYTES: usize = 256 * 1024 * 1024;
    const MAX_GIF_FRAMES: usize = 500;
    const LOAD_WORKERS: usize = 2;

    #[derive(Clone, Copy)]
    struct DecodeLimits {
        media_bytes: usize,
        frames: usize,
        decoder_bytes: u64,
    }

    impl Default for DecodeLimits {
        fn default() -> Self {
            Self {
                media_bytes: MAX_MEDIA_BYTES,
                frames: MAX_GIF_FRAMES,
                decoder_bytes: MAX_MEDIA_BYTES as u64,
            }
        }
    }

    impl DecodeLimits {
        fn image_limits(self) -> image::Limits {
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(self.decoder_bytes);
            limits
        }
    }

    struct MediaBudget {
        used: AtomicUsize,
        limit: usize,
    }

    impl Default for MediaBudget {
        fn default() -> Self {
            Self {
                used: AtomicUsize::new(0),
                limit: MAX_SHARED_BYTES,
            }
        }
    }

    struct Reservation {
        budget: Arc<MediaBudget>,
        bytes: usize,
    }

    impl Reservation {
        fn new(budget: Arc<MediaBudget>) -> Self {
            Self { budget, bytes: 0 }
        }

        fn grow(&mut self, bytes: usize, limit: usize) -> Result<(), String> {
            let total = self
                .bytes
                .checked_add(bytes)
                .filter(|total| *total <= limit)
                .ok_or_else(|| format!("Decoded media exceeds its {limit} byte limit"))?;
            let mut used = self.budget.used.load(Ordering::Acquire);
            loop {
                let next = used
                    .checked_add(bytes)
                    .filter(|next| *next <= self.budget.limit)
                    .ok_or_else(|| {
                        format!(
                            "Decoded media exceeds the shared {} byte limit",
                            self.budget.limit
                        )
                    })?;
                match self.budget.used.compare_exchange_weak(
                    used,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(current) => used = current,
                }
            }
            self.bytes = total;
            Ok(())
        }
    }

    impl Drop for Reservation {
        fn drop(&mut self) {
            self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
        }
    }

    enum DecodedMedia {
        Static(SourceFrame),
        /// Each frame paired with its own display duration in milliseconds.
        Animated(Vec<(SourceFrame, u32)>),
    }

    struct LoadedMedia {
        media: DecodedMedia,
        reservation: Reservation,
    }

    struct PendingLoad {
        receiver: Receiver<Result<LoadedMedia, String>>,
        cancelled: Arc<AtomicBool>,
    }

    impl Drop for PendingLoad {
        fn drop(&mut self) {
            self.cancelled.store(true, Ordering::Release);
        }
    }

    struct LoadJob {
        slot: usize,
        url: String,
        gif: bool,
        cancelled: Arc<AtomicBool>,
        sender: SyncSender<Result<LoadedMedia, String>>,
        budget: Arc<MediaBudget>,
    }

    impl LoadJob {
        fn run(self) {
            let result = (|| {
                check_cancelled(&self.cancelled)?;
                let bytes = fetch_bytes(&self.url, &self.cancelled)?;
                let mut reservation = Reservation::new(self.budget);
                let limits = DecodeLimits::default();
                let media = if self.gif {
                    DecodedMedia::Animated(decode_gif_frames(
                        &bytes,
                        limits,
                        &self.cancelled,
                        &mut reservation,
                    )?)
                } else {
                    DecodedMedia::Static(decode_static(
                        &bytes,
                        limits,
                        &self.cancelled,
                        &mut reservation,
                    )?)
                };
                check_cancelled(&self.cancelled)?;
                Ok(LoadedMedia { media, reservation })
            })();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            let result = result.map_err(|error: String| {
                let function = if self.gif { "initGif" } else { "initImage" };
                let error = format!("{function}: failed to load {:?}: {error}", self.url);
                log::warn!("{error}");
                error
            });
            // Replaced requests drop their receiver and release any decoded storage here.
            let _ = self.sender.send(result);
        }
    }

    #[derive(Default)]
    struct JobState {
        jobs: [Option<LoadJob>; NUM_SOURCES],
        stopped: bool,
        next_slot: usize,
    }

    #[derive(Default)]
    struct JobQueue {
        state: Mutex<JobState>,
        available: Condvar,
    }

    impl JobQueue {
        fn submit(&self, job: LoadJob) {
            let slot = job.slot;
            self.state.lock().unwrap().jobs[slot] = Some(job);
            self.available.notify_one();
        }

        fn take(&self) -> Option<LoadJob> {
            let mut state = self.state.lock().unwrap();
            loop {
                if state.stopped {
                    return None;
                }
                for offset in 0..NUM_SOURCES {
                    let slot = (state.next_slot + offset) % NUM_SOURCES;
                    if let Some(job) = state.jobs[slot].take() {
                        state.next_slot = (slot + 1) % NUM_SOURCES;
                        return Some(job);
                    }
                }
                state = self.available.wait(state).unwrap();
            }
        }

        fn stop(&self) {
            let mut state = self.state.lock().unwrap();
            state.stopped = true;
            for job in &mut state.jobs {
                job.take();
            }
            self.available.notify_all();
        }
    }

    struct Loader {
        queue: Arc<JobQueue>,
    }

    impl Loader {
        fn new() -> Result<Self, String> {
            let loader = Self {
                queue: Arc::new(JobQueue::default()),
            };
            for index in 0..LOAD_WORKERS {
                let queue = loader.queue.clone();
                thread::Builder::new()
                    .name(format!("hydra-media-{index}"))
                    .spawn(move || {
                        while let Some(job) = queue.take() {
                            job.run();
                        }
                    })
                    .map_err(|error| format!("Could not start media loader: {error}"))?;
            }
            Ok(loader)
        }
    }

    impl Drop for Loader {
        fn drop(&mut self) {
            self.queue.stop();
        }
    }

    struct AnimatedState {
        frames: Vec<(SourceFrame, u32)>,
        total_duration_ms: u64,
        started_at: Instant,
        last_index: usize,
        _reservation: Reservation,
    }

    #[derive(Default)]
    pub struct ImageManager {
        pending: [Option<PendingLoad>; NUM_SOURCES],
        animated: [Option<AnimatedState>; NUM_SOURCES],
        loader: Option<Loader>,
        budget: Arc<MediaBudget>,
        errors: [Option<String>; NUM_SOURCES],
    }

    impl ImageManager {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn init_image(&mut self, slot: usize, url: String) {
            self.init(slot, url, false);
        }

        pub fn init_gif(&mut self, slot: usize, url: String) {
            self.init(slot, url, true);
        }

        fn init(&mut self, slot: usize, url: String, gif: bool) {
            if slot >= NUM_SOURCES {
                log::warn!("Media source slot {slot} is out of range");
                return;
            }
            self.pending[slot] = None;
            self.animated[slot] = None;
            self.errors[slot] = None;
            if self.loader.is_none() {
                match Loader::new() {
                    Ok(loader) => self.loader = Some(loader),
                    Err(error) => {
                        log::warn!("{error}");
                        self.errors[slot] = Some(error);
                        return;
                    }
                }
            }
            let (sender, receiver) = mpsc::sync_channel(1);
            let cancelled = Arc::new(AtomicBool::new(false));
            self.pending[slot] = Some(PendingLoad {
                receiver,
                cancelled: cancelled.clone(),
            });
            self.loader.as_ref().unwrap().queue.submit(LoadJob {
                slot,
                url,
                gif,
                cancelled,
                sender,
                budget: self.budget.clone(),
            });
        }

        pub fn take_error(&mut self) -> Option<String> {
            self.errors.iter_mut().find_map(Option::take)
        }

        /// Returns a frame to upload for `slot`, if one is newly ready:
        /// either the just-finished static image / first GIF frame, or
        /// (for an already-loaded GIF) the next animation frame once
        /// enough real time has elapsed - `None` otherwise, so callers
        /// don't needlessly re-upload an unchanged frame every render frame.
        pub fn poll(&mut self, slot: usize) -> Option<SourceFrame> {
            self.poll_frame(slot).map(Cow::into_owned)
        }

        /// Borrows newly ready frames to avoid copying decoded animation pixels.
        pub fn poll_with<T>(
            &mut self,
            slot: usize,
            consume: impl FnOnce(&SourceFrame) -> T,
        ) -> Option<T> {
            self.poll_frame(slot).map(|frame| consume(&frame))
        }

        fn poll_frame(&mut self, slot: usize) -> Option<Cow<'_, SourceFrame>> {
            if let Some(pending) = self.pending.get(slot).and_then(Option::as_ref) {
                let result = match pending.receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => {
                        let error = "Media loader stopped before returning a result".to_string();
                        log::warn!("{error}");
                        Some(Err(error))
                    }
                };
                if let Some(result) = result {
                    self.pending[slot] = None;
                    let LoadedMedia { media, reservation } = match result {
                        Ok(media) => media,
                        Err(error) => {
                            self.errors[slot] = Some(error);
                            return None;
                        }
                    };
                    return match media {
                        DecodedMedia::Static(frame) => {
                            self.animated[slot] = None;
                            Some(Cow::Owned(frame))
                        }
                        DecodedMedia::Animated(frames) => {
                            let total = frames.iter().map(|(_, d)| *d as u64).sum::<u64>().max(1);
                            self.animated[slot] = Some(AnimatedState {
                                frames,
                                total_duration_ms: total,
                                started_at: Instant::now(),
                                last_index: 0,
                                _reservation: reservation,
                            });
                            Some(Cow::Borrowed(
                                &self.animated[slot].as_ref().unwrap().frames[0].0,
                            ))
                        }
                    };
                }
            }

            let state = self.animated.get_mut(slot)?.as_mut()?;
            let idx = frame_index_at(
                state.started_at.elapsed().as_millis() as u64,
                state.total_duration_ms,
                &state.frames,
            );
            if idx == state.last_index {
                return None;
            }
            state.last_index = idx;
            Some(Cow::Borrowed(&state.frames[idx].0))
        }
    }

    /// Which frame of an animation (each with its own display duration in
    /// `frames`, total `total_duration_ms`) should be showing at
    /// `elapsed_ms` since it started - wrapping around once the whole
    /// animation has looped.
    fn frame_index_at(
        elapsed_ms: u64,
        total_duration_ms: u64,
        frames: &[(SourceFrame, u32)],
    ) -> usize {
        let position = elapsed_ms % total_duration_ms;
        let mut acc = 0u64;
        for (i, (_, delay)) in frames.iter().enumerate() {
            acc += *delay as u64;
            if position < acc {
                return i;
            }
        }
        frames.len() - 1
    }

    fn checked_rgb_bytes(width: u32, height: u32) -> Result<usize, String> {
        (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(3))
            .filter(|bytes| *bytes > 0)
            .ok_or_else(|| "Invalid decoded image dimensions".into())
    }

    fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
        if cancelled.load(Ordering::Acquire) {
            Err("Media load cancelled".into())
        } else {
            Ok(())
        }
    }

    fn decode_static(
        bytes: &[u8],
        limits: DecodeLimits,
        cancelled: &AtomicBool,
        reservation: &mut Reservation,
    ) -> Result<SourceFrame, String> {
        use image::ImageDecoder;
        check_cancelled(cancelled)?;
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| format!("Image format error: {error}"))?;
        reader.limits(limits.image_limits());
        let decoder = reader
            .into_decoder()
            .map_err(|error| format!("Image decode error: {error}"))?;
        if decoder.total_bytes() > limits.decoder_bytes {
            return Err("Image exceeds the decoder memory limit".into());
        }
        let (width, height) = decoder.dimensions();
        reservation.grow(checked_rgb_bytes(width, height)?, limits.media_bytes)?;
        let img = image::DynamicImage::from_decoder(decoder)
            .map_err(|error| format!("Image decode error: {error}"))?
            .into_rgb8();
        check_cancelled(cancelled)?;
        let (width, height) = img.dimensions();
        Ok(SourceFrame {
            pixels: img.into_raw(),
            width,
            height,
        })
    }

    fn decode_gif_frames(
        bytes: &[u8],
        limits: DecodeLimits,
        cancelled: &AtomicBool,
        reservation: &mut Reservation,
    ) -> Result<Vec<(SourceFrame, u32)>, String> {
        use image::codecs::gif::GifDecoder;
        use image::{AnimationDecoder, ImageDecoder};

        check_cancelled(cancelled)?;
        let mut decoder = GifDecoder::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("gif decode error: {e}"))?;
        decoder
            .set_limits(limits.image_limits())
            .map_err(|error| format!("GIF decoder limit: {error}"))?;
        if decoder.total_bytes() > limits.decoder_bytes {
            return Err("GIF exceeds the decoder memory limit".into());
        }
        let mut frames = Vec::new();
        let mut iterator = decoder.into_frames();
        loop {
            check_cancelled(cancelled)?;
            let Some(frame) = iterator.next() else {
                break;
            };
            if frames.len() >= limits.frames {
                return Err(format!("GIF exceeds its {} frame limit", limits.frames));
            }
            let frame = frame.map_err(|error| format!("GIF frame decode error: {error}"))?;
            let (width, height) = frame.buffer().dimensions();
            reservation.grow(checked_rgb_bytes(width, height)?, limits.media_bytes)?;
            let delay = u32::try_from(Duration::from(frame.delay()).as_millis().max(1))
                .map_err(|_| "GIF frame delay exceeds the supported duration")?;
            let rgb = image::DynamicImage::ImageRgba8(frame.into_buffer()).into_rgb8();
            frames.push((
                SourceFrame {
                    pixels: rgb.into_raw(),
                    width,
                    height,
                },
                delay,
            ));
        }
        if frames.is_empty() {
            return Err("gif has no frames".to_string());
        }

        Ok(frames)
    }

    fn read_bounded(
        mut reader: impl Read,
        limit: usize,
        cancelled: &AtomicBool,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            check_cancelled(cancelled)?;
            let count = reader
                .read(&mut chunk)
                .map_err(|error| format!("Media read error: {error}"))?;
            if count == 0 {
                return Ok(bytes);
            }
            if bytes
                .len()
                .checked_add(count)
                .is_none_or(|total| total > limit)
            {
                return Err(format!("Media download exceeds its {limit} byte limit"));
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
    }

    fn fetch_bytes(url: &str, cancelled: &AtomicBool) -> Result<Vec<u8>, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(2))
            .build();
        let response = agent
            .get(url)
            .call()
            .map_err(|e| format!("request error: {e}"))?;
        read_bounded(response.into_reader(), MAX_DOWNLOAD_BYTES, cancelled)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn decode_static(bytes: &[u8]) -> Result<SourceFrame, String> {
            super::decode_static(
                bytes,
                DecodeLimits::default(),
                &AtomicBool::new(false),
                &mut Reservation::new(Arc::new(MediaBudget::default())),
            )
        }

        fn decode_gif_frames(bytes: &[u8]) -> Result<Vec<(SourceFrame, u32)>, String> {
            super::decode_gif_frames(
                bytes,
                DecodeLimits::default(),
                &AtomicBool::new(false),
                &mut Reservation::new(Arc::new(MediaBudget::default())),
            )
        }

        fn gif_bytes(count: usize) -> Vec<u8> {
            let mut bytes = Vec::new();
            {
                let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
                for index in 0..count {
                    encoder
                        .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                            2,
                            2,
                            image::Rgba([index as u8, 0, 0, 255]),
                        )))
                        .unwrap();
                }
            }
            bytes
        }

        #[test]
        fn downloads_stop_at_the_byte_limit_and_between_cancelled_reads() {
            let cancelled = AtomicBool::new(false);
            assert_eq!(read_bounded(&b"1234"[..], 4, &cancelled).unwrap(), b"1234");
            assert!(
                read_bounded(&b"12345"[..], 4, &cancelled)
                    .unwrap_err()
                    .contains("download exceeds")
            );
            struct CancellingReader<'a> {
                cancelled: &'a AtomicBool,
                reads: usize,
            }
            impl Read for CancellingReader<'_> {
                fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                    self.reads += 1;
                    self.cancelled.store(true, Ordering::Release);
                    output[0] = 1;
                    Ok(1)
                }
            }
            let mut reader = CancellingReader {
                cancelled: &cancelled,
                reads: 0,
            };
            assert!(
                read_bounded(&mut reader, 10, &cancelled)
                    .unwrap_err()
                    .contains("cancelled")
            );
            assert_eq!(reader.reads, 1);
        }

        #[test]
        fn gif_decoding_enforces_frame_and_storage_limits_and_releases_reservations() {
            let bytes = gif_bytes(2);
            let budget = Arc::new(MediaBudget::default());
            let cancelled = AtomicBool::new(false);
            let limits = DecodeLimits {
                media_bytes: 12,
                ..Default::default()
            };
            {
                let mut reservation = Reservation::new(budget.clone());
                assert!(
                    super::decode_gif_frames(&bytes, limits, &cancelled, &mut reservation)
                        .err()
                        .unwrap()
                        .contains("Decoded media exceeds")
                );
                assert_eq!(budget.used.load(Ordering::Acquire), 12);
            }
            assert_eq!(budget.used.load(Ordering::Acquire), 0);
            let limits = DecodeLimits {
                frames: 1,
                ..Default::default()
            };
            assert!(
                super::decode_gif_frames(
                    &bytes,
                    limits,
                    &cancelled,
                    &mut Reservation::new(budget.clone())
                )
                .err()
                .unwrap()
                .contains("frame limit")
            );
            assert_eq!(budget.used.load(Ordering::Acquire), 0);
            cancelled.store(true, Ordering::Release);
            assert!(
                super::decode_gif_frames(
                    &bytes,
                    DecodeLimits::default(),
                    &cancelled,
                    &mut Reservation::new(budget.clone())
                )
                .err()
                .unwrap()
                .contains("cancelled")
            );
            assert_eq!(budget.used.load(Ordering::Acquire), 0);
        }

        #[test]
        fn decoded_storage_budget_is_shared_until_media_is_released() {
            let budget = Arc::new(MediaBudget {
                used: AtomicUsize::new(0),
                limit: 12,
            });
            let bytes = gif_bytes(1);
            let mut first = Reservation::new(budget.clone());
            let frames = super::decode_gif_frames(
                &bytes,
                DecodeLimits::default(),
                &AtomicBool::new(false),
                &mut first,
            )
            .unwrap();
            let mut second = Reservation::new(budget.clone());
            assert!(
                second
                    .grow(12, MAX_MEDIA_BYTES)
                    .unwrap_err()
                    .contains("shared")
            );
            drop(frames);
            drop(first);
            second.grow(12, MAX_MEDIA_BYTES).unwrap();
            assert_eq!(budget.used.load(Ordering::Acquire), 12);
            drop(second);
            assert_eq!(budget.used.load(Ordering::Acquire), 0);
        }

        #[test]
        fn replacements_cancel_previous_loads_and_keep_only_one_queued_job_per_slot() {
            let queue = Arc::new(JobQueue::default());
            let mut manager = ImageManager {
                loader: Some(Loader {
                    queue: queue.clone(),
                }),
                ..Default::default()
            };
            manager.init_image(0, "first".into());
            let first = manager.pending[0].as_ref().unwrap().cancelled.clone();
            for index in 0..1000 {
                manager.init_image(0, index.to_string());
            }
            assert!(first.load(Ordering::Acquire));
            assert_eq!(queue.state.lock().unwrap().jobs.iter().flatten().count(), 1);
            assert_eq!(queue.take().unwrap().url, "999");
            for slot in 0..NUM_SOURCES {
                manager.init_gif(slot, slot.to_string());
            }
            assert_eq!(
                queue.state.lock().unwrap().jobs.iter().flatten().count(),
                NUM_SOURCES
            );
            let cancelled = manager.pending[0].as_ref().unwrap().cancelled.clone();
            drop(manager);
            assert!(cancelled.load(Ordering::Acquire));
            assert!(queue.take().is_none());
        }

        #[test]
        fn completed_animation_keeps_its_budget_and_errors_are_available_to_the_app() {
            let queue = Arc::new(JobQueue::default());
            let mut manager = ImageManager {
                loader: Some(Loader { queue }),
                ..Default::default()
            };
            let (sender, receiver) = mpsc::sync_channel(1);
            let mut reservation = Reservation::new(manager.budget.clone());
            reservation.grow(12, MAX_MEDIA_BYTES).unwrap();
            manager.pending[0] = Some(PendingLoad {
                receiver,
                cancelled: Arc::new(AtomicBool::new(false)),
            });
            sender
                .send(Ok(LoadedMedia {
                    media: DecodedMedia::Animated(vec![(solid_frame(2, 2, 1), 10)]),
                    reservation,
                }))
                .ok()
                .unwrap();
            assert!(manager.poll(0).is_some());
            assert!(manager.poll(0).is_none());
            assert_eq!(manager.budget.used.load(Ordering::Acquire), 12);
            manager.init_image(0, "replacement".into());
            assert_eq!(manager.budget.used.load(Ordering::Acquire), 0);
            let (sender, receiver) = mpsc::sync_channel(1);
            manager.pending[0] = Some(PendingLoad {
                receiver,
                cancelled: Arc::new(AtomicBool::new(false)),
            });
            sender.send(Err("Oversized media".into())).ok().unwrap();
            assert!(manager.poll(0).is_none());
            assert!(manager.pending[0].is_none());
            assert_eq!(manager.take_error().as_deref(), Some("Oversized media"));
            assert!(manager.take_error().is_none());
        }

        #[test]
        fn borrowed_animation_frames_reuse_pixels_and_skip_unchanged_uploads() {
            let mut manager = ImageManager::new();
            let frames = vec![
                (solid_frame(2, 2, 1), 3_600_000),
                (solid_frame(2, 2, 2), 3_600_000),
            ];
            let first = frames[0].0.pixels.as_ptr();
            let second = frames[1].0.pixels.as_ptr();
            let mut reservation = Reservation::new(manager.budget.clone());
            reservation.grow(24, MAX_MEDIA_BYTES).unwrap();
            let (sender, receiver) = mpsc::sync_channel(1);
            manager.pending[0] = Some(PendingLoad {
                receiver,
                cancelled: Arc::new(AtomicBool::new(false)),
            });
            sender
                .send(Ok(LoadedMedia {
                    media: DecodedMedia::Animated(frames),
                    reservation,
                }))
                .ok()
                .unwrap();
            assert_eq!(
                manager.poll_with(0, |frame| frame.pixels.as_ptr()),
                Some(first)
            );
            assert!(
                manager
                    .poll_with(0, |_| panic!("unchanged frame uploaded"))
                    .is_none()
            );
            manager.animated[0].as_mut().unwrap().started_at =
                Instant::now() - Duration::from_secs(3601);
            assert_eq!(
                manager.poll_with(0, |frame| frame.pixels.as_ptr()),
                Some(second)
            );
            assert!(
                manager
                    .poll_with(0, |_| panic!("unchanged frame uploaded"))
                    .is_none()
            );
            manager.animated[0].as_mut().unwrap().started_at =
                Instant::now() - Duration::from_secs(7201);
            assert_eq!(
                manager.poll_with(0, |frame| frame.pixels.as_ptr()),
                Some(first)
            );
            assert_eq!(manager.budget.used.load(Ordering::Acquire), 24);
        }

        #[test]
        fn owned_static_frames_move_without_copying_pixels() {
            let mut manager = ImageManager::new();
            let frame = solid_frame(2, 2, 1);
            let pixels = frame.pixels.as_ptr();
            let (sender, receiver) = mpsc::sync_channel(1);
            manager.pending[0] = Some(PendingLoad {
                receiver,
                cancelled: Arc::new(AtomicBool::new(false)),
            });
            sender
                .send(Ok(LoadedMedia {
                    media: DecodedMedia::Static(frame),
                    reservation: Reservation::new(manager.budget.clone()),
                }))
                .ok()
                .unwrap();
            let frame = manager.poll(0).unwrap();
            assert_eq!(frame.pixels.as_ptr(), pixels);
            assert!(manager.poll(0).is_none());
        }

        fn solid_frame(w: u32, h: u32, v: u8) -> SourceFrame {
            SourceFrame {
                pixels: vec![v; (w * h * 3) as usize],
                width: w,
                height: h,
            }
        }

        #[test]
        fn decode_static_produces_a_source_frame_matching_the_original_image() {
            let mut img = image::RgbImage::new(3, 2);
            for (i, px) in img.pixels_mut().enumerate() {
                *px = image::Rgb([i as u8, (i * 2) as u8, (i * 3) as u8]);
            }
            let mut png_bytes = Vec::new();
            image::DynamicImage::ImageRgb8(img.clone())
                .write_to(
                    &mut std::io::Cursor::new(&mut png_bytes),
                    image::ImageFormat::Png,
                )
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
        fn static_decode_rejects_oversized_pixels_before_reserving_storage() {
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgb8(image::RgbImage::new(2, 2))
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .unwrap();
            let budget = Arc::new(MediaBudget::default());
            let cancelled = AtomicBool::new(false);
            for limits in [
                DecodeLimits {
                    media_bytes: 11,
                    ..Default::default()
                },
                DecodeLimits {
                    decoder_bytes: 1,
                    ..Default::default()
                },
            ] {
                let mut reservation = Reservation::new(budget.clone());
                assert!(
                    super::decode_static(&bytes, limits, &cancelled, &mut reservation).is_err()
                );
                assert_eq!(budget.used.load(Ordering::Acquire), 0);
            }
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
            let frames = vec![
                (solid_frame(1, 1, 0), 100),
                (solid_frame(1, 1, 1), 100),
                (solid_frame(1, 1, 2), 100),
            ];
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
                        image::Delay::from_saturating_duration(std::time::Duration::from_millis(
                            10,
                        )),
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
