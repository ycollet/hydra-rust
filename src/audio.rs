pub const NUM_FFT_BINS: usize = 8;

/// Audio-clocked onset envelope, estimated beat phase and tempo.
/// Beat and BPM remain zero until repeated, regular onsets establish a tempo.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AudioFeatures {
    pub onset: f32,
    pub beat: f32,
    pub bpm: f32,
}

#[cfg(feature = "fft")]
mod rhythm;

#[cfg(feature = "fft")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BinSettings {
    pub cutoff: f32,
    pub scale: f32,
    pub smooth: f32,
}

#[cfg(feature = "fft")]
impl Default for BinSettings {
    fn default() -> Self {
        Self {
            cutoff: 0.0,
            scale: 1.0,
            smooth: 0.4,
        }
    }
}

#[cfg(feature = "fft")]
#[derive(Debug, Clone, Copy)]
pub enum BinProperty {
    Cutoff,
    Scale,
    Smooth,
}

#[cfg(feature = "fft")]
#[derive(Debug, Clone)]
pub struct AudioSettings {
    pub(crate) bins: usize,
    pub(crate) defaults: BinSettings,
    pub(crate) settings: [BinSettings; NUM_FFT_BINS],
}

#[cfg(feature = "fft")]
impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            bins: 4,
            defaults: BinSettings::default(),
            settings: [BinSettings::default(); NUM_FFT_BINS],
        }
    }
}

#[cfg(feature = "fft")]
impl AudioSettings {
    pub fn set_bins(&mut self, n: usize) {
        self.bins = n.clamp(1, NUM_FFT_BINS);
        self.settings.fill(self.defaults);
    }

    pub fn set(
        &mut self,
        index: Option<usize>,
        property: BinProperty,
        value: f32,
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err("Audio settings must be finite numbers".into());
        }
        let update = |settings: &mut BinSettings| match property {
            BinProperty::Cutoff => settings.cutoff = value.max(0.0),
            BinProperty::Scale => settings.scale = value,
            BinProperty::Smooth => settings.smooth = value.clamp(0.0, 1.0),
        };
        if let Some(index) = index {
            if index >= self.bins {
                return Err(format!(
                    "Audio bin index {index} is outside 0..{}",
                    self.bins
                ));
            }
            update(&mut self.settings[index]);
        } else {
            update(&mut self.defaults);
            for settings in &mut self.settings {
                update(settings);
            }
        }
        Ok(())
    }
}

// The FFT-reactive pipeline (`AudioBuffer`/`AudioAnalyzer`) is split out
// from mic capture (`AudioManager`, below) so a host other than this
// crate's own cpal-based desktop app - e.g. the `hydra-plugin` crate,
// fed by a DAW's own audio callback - can push samples into the exact
// same analysis pipeline without needing `cpal`/microphone access at all.
#[cfg(feature = "fft")]
mod analysis {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use rustfft::{Fft, FftPlanner, num_complex::Complex};

    use super::rhythm::{CAPACITY, PendingAudio, RhythmDetector};
    use super::{AudioFeatures, AudioSettings, BinProperty, NUM_FFT_BINS};

    pub(super) const FFT_SIZE: usize = 1024;

    /// Shared ring buffer that a producer (this crate's own mic-capture
    /// thread, or - for the plugin build - the host's own `process()`
    /// audio callback) pushes raw samples into, and an [`AudioAnalyzer`]
    /// (typically on a different thread, e.g. a render loop) polls FFT
    /// bins out of.
    ///
    /// Splitting the buffer from the analyzer like this means the
    /// producer side never runs any FFT/allocation - only a bounded
    /// `VecDeque` push behind a quick mutex lock - which is what makes it
    /// safe to call `push` directly from a real-time audio thread (e.g.
    /// a VST3/CLAP plugin's `process()`).
    #[derive(Clone)]
    pub struct AudioBuffer {
        pub(super) samples: Arc<Mutex<VecDeque<f32>>>,
        pending: Arc<Mutex<PendingAudio>>,
    }

    impl Default for AudioBuffer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AudioBuffer {
        pub fn new() -> Self {
            let buffer = Self {
                samples: Arc::new(Mutex::new(VecDeque::with_capacity(FFT_SIZE))),
                pending: Arc::new(Mutex::new(PendingAudio::new())),
            };
            // macOS allocates the native mutex on first lock, not in Mutex::new().
            drop(buffer.samples.lock().unwrap());
            drop(buffer.pending.lock().unwrap());
            buffer
        }

        /// Pushes samples, downmixing to mono first if `channels > 1`
        /// (assuming `data` is interleaved, matching `cpal`'s own stream
        /// callback layout). No allocation after construction. Latest FFT
        /// samples and a bounded queue for fixed-hop rhythm analysis are retained.
        pub fn push(&self, data: &[f32], channels: usize) {
            push_samples(&self.samples, data, channels);
            let mut pending = self.pending.lock().unwrap();
            let channels = channels.max(1);
            for frame in data.chunks(channels) {
                pending.push(frame.iter().sum::<f32>() / channels as f32);
            }
        }

        /// Configure the producer's sample rate before pushing audio.
        /// Defaults to 44100 Hz. Changing it clears pending rhythm history.
        pub fn set_sample_rate(&self, sample_rate: u32) -> Result<(), String> {
            if sample_rate == 0 {
                return Err("Audio sample rate must be positive".into());
            }
            let mut pending = self.pending.lock().unwrap();
            if pending.sample_rate != sample_rate {
                pending.samples.clear();
                pending.sample_rate = sample_rate;
                pending.discontinuity = true;
            }
            Ok(())
        }
    }

    /// The FFT analysis side: owns the FFT planner/scratch buffers and
    /// the smoothed-output state, and reads (without ever locking for
    /// longer than a `VecDeque` copy) from an [`AudioBuffer`] a producer
    /// is concurrently pushing into.
    pub struct AudioAnalyzer {
        fft: Arc<dyn Fft<f32>>,
        scratch: Vec<Complex<f32>>,
        /// Hann window, precomputed once - `FFT_SIZE` never changes, so
        /// recomputing `cos()` for all 1024 samples on every `poll()`
        /// call (once per rendered frame) would be pure waste.
        window: [f32; FFT_SIZE],
        smoothed: [f32; NUM_FFT_BINS],
        settings: AudioSettings,
        rhythm: RhythmDetector,
        pending_samples: Vec<f32>,
    }

    impl Default for AudioAnalyzer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AudioAnalyzer {
        pub fn new() -> Self {
            let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);

            Self {
                fft,
                scratch: vec![Complex { re: 0.0, im: 0.0 }; FFT_SIZE],
                window: std::array::from_fn(|i| hann(i, FFT_SIZE)),
                smoothed: [0.0; NUM_FFT_BINS],
                settings: AudioSettings::default(),
                rhythm: RhythmDetector::new(44_100),
                pending_samples: Vec::with_capacity(CAPACITY),
            }
        }

        pub fn set_bins(&mut self, n: usize) {
            self.settings.set_bins(n);
            self.smoothed.fill(0.0);
        }

        pub fn settings(&self) -> &AudioSettings {
            &self.settings
        }

        pub fn set_cutoff(&mut self, c: f32) {
            if let Err(error) = self.settings.set(None, BinProperty::Cutoff, c) {
                log::error!("{error}");
            }
        }

        pub fn set_scale(&mut self, s: f32) {
            if let Err(error) = self.settings.set(None, BinProperty::Scale, s) {
                log::error!("{error}");
            }
        }

        pub fn set_smooth(&mut self, s: f32) {
            if let Err(error) = self.settings.set(None, BinProperty::Smooth, s) {
                log::error!("{error}");
            }
        }

        pub fn set_bin(
            &mut self,
            index: usize,
            property: BinProperty,
            value: f32,
        ) -> Result<(), String> {
            self.settings.set(Some(index), property, value)
        }

        pub fn poll(&mut self, buffer: &AudioBuffer) -> [f32; NUM_FFT_BINS] {
            self.pending_samples.clear();
            {
                let mut pending = buffer.pending.lock().unwrap();
                if pending.discontinuity || pending.sample_rate != self.rhythm.sample_rate() {
                    if pending.discontinuity && pending.samples.len() == CAPACITY {
                        log::warn!("Audio rhythm analysis overrun; resetting beat history");
                    }
                    self.rhythm = RhythmDetector::new(pending.sample_rate);
                    pending.discontinuity = false;
                }
                self.pending_samples.extend(pending.samples.drain(..));
            }
            self.rhythm.process(
                &self.pending_samples,
                self.fft.as_ref(),
                &mut self.scratch,
                &self.window,
            );
            {
                let buf = buffer.samples.lock().unwrap();
                let len = buf.len();
                let offset = FFT_SIZE.saturating_sub(len);
                for (i, slot) in self.scratch.iter_mut().enumerate() {
                    let sample = if i < offset {
                        0.0
                    } else {
                        *buf.get(i - offset).unwrap_or(&0.0)
                    };
                    *slot = Complex {
                        re: sample * self.window[i],
                        im: 0.0,
                    };
                }
            }

            self.fft.process(&mut self.scratch);

            let usable = FFT_SIZE / 2;
            let magnitudes: Vec<f32> = self.scratch[..usable]
                .iter()
                .map(|c| c.norm() / FFT_SIZE as f32)
                .collect();

            let bins = reduce_to_bins(&magnitudes, self.settings.bins);

            let mut out = [0.0f32; NUM_FFT_BINS];
            for i in 0..self.settings.bins {
                let settings = self.settings.settings[i];
                let raw = bins[i];
                let gated = if raw < settings.cutoff {
                    0.0
                } else {
                    raw * settings.scale
                };
                self.smoothed[i] =
                    self.smoothed[i] * settings.smooth + gated * (1.0 - settings.smooth);
                out[i] = self.smoothed[i];
            }
            out
        }

        /// Features updated by the most recent poll, independent of FFT-bin settings.
        pub fn features(&self) -> AudioFeatures {
            self.rhythm.features
        }
    }

    fn hann(i: usize, n: usize) -> f32 {
        0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0)).cos()
    }

    /// Averages the magnitude spectrum into `num_bins` logarithmically-spaced buckets,
    /// since audio-reactive visuals care about relative octaves, not raw linear FFT bins.
    fn reduce_to_bins(spectrum: &[f32], num_bins: usize) -> Vec<f32> {
        let usable = spectrum.len();
        let log_max = (usable as f32).ln();
        let mut bins = vec![0.0f32; num_bins];
        for (b, out) in bins.iter_mut().enumerate() {
            let start = (((b as f32) / num_bins as f32) * log_max).exp().max(1.0) as usize;
            let end = (((b + 1) as f32 / num_bins as f32) * log_max)
                .exp()
                .max(1.0) as usize;
            let start = start.min(usable.saturating_sub(1));
            let end = end.max(start + 1).min(usable);
            let slice = &spectrum[start..end];
            *out = if slice.is_empty() {
                0.0
            } else {
                slice.iter().sum::<f32>() / slice.len() as f32
            };
        }
        bins
    }

    pub(super) fn push_samples(samples: &Arc<Mutex<VecDeque<f32>>>, data: &[f32], channels: usize) {
        let mut buf = samples.lock().unwrap();
        let channels = channels.max(1);
        for frame in data.chunks(channels) {
            if buf.len() == FFT_SIZE {
                buf.pop_front();
            }
            let sample = if channels == 1 {
                frame[0]
            } else {
                frame.iter().sum::<f32>() / channels as f32
            };
            buf.push_back(sample);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn click_track(sample_rate: u32, seconds: f32) -> Vec<f32> {
            let mut samples = vec![0.0; (sample_rate as f32 * seconds) as usize];
            for beat in 0..7 {
                let index = (sample_rate as f64 * (0.1 + beat as f64 * 0.5)).round() as usize;
                if index < samples.len() {
                    samples[index] = 1.0;
                }
            }
            samples
        }

        fn analyze_chunks(samples: &[f32], sample_rate: u32, chunk: usize) -> AudioFeatures {
            let buffer = AudioBuffer::new();
            buffer.set_sample_rate(sample_rate).unwrap();
            let mut analyzer = AudioAnalyzer::new();
            for samples in samples.chunks(chunk) {
                buffer.push(samples, 1);
                analyzer.poll(&buffer);
            }
            analyzer.features()
        }

        #[test]
        fn rhythm_uses_audio_sample_rate_and_not_render_poll_frequency() {
            for rate in [44_100, 48_000, 96_000] {
                let samples = click_track(rate, 3.25);
                let fast = analyze_chunks(&samples, rate, 128);
                let slow = analyze_chunks(&samples, rate, 4096);
                assert_eq!(fast, slow);
                assert!((fast.bpm - 120.0).abs() < 2.0, "{rate}: {fast:?}");
                assert!((0.2..0.4).contains(&fast.beat), "{rate}: {fast:?}");
                assert!(fast.onset > 0.0 && fast.onset < 0.5, "{fast:?}");
            }
        }

        #[test]
        fn tempo_range_endpoints_allow_for_fft_hop_quantization() {
            for bpm in [60.0, 200.0] {
                let mut samples = vec![0.0; 48_000 * 8];
                for beat in 0..8 {
                    let index = ((0.1 + beat as f64 * 60.0 / bpm) * 48_000.0) as usize;
                    samples[index] = 1.0;
                }
                let end = ((0.25 + 7.0 * 60.0 / bpm) * 48_000.0) as usize;
                let features = analyze_chunks(&samples[..end], 48_000, 512);
                assert!(
                    (features.bpm as f64 - bpm).abs() < 2.0,
                    "{bpm}: {features:?}"
                );
            }
        }

        #[test]
        fn silence_and_sustained_tone_do_not_create_a_beat() {
            let silence = analyze_chunks(&vec![0.0; 96_000], 48_000, 512);
            assert_eq!(silence, AudioFeatures::default());
            let tone: Vec<_> = (0..96_000)
                .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5)
                .collect();
            let features = analyze_chunks(&tone, 48_000, 512);
            assert_eq!(features.bpm, 0.0, "{features:?}");
            assert!(features.onset < 0.01, "{features:?}");
        }

        #[test]
        fn irregular_onsets_do_not_establish_a_tempo() {
            let mut samples = vec![0.0; 48_000 * 4];
            for time in [0.1, 0.45, 1.3, 1.8, 2.75, 3.1, 3.65] {
                samples[(time * 48_000.0) as usize] = 1.0;
            }
            let features = analyze_chunks(&samples, 48_000, 512);
            assert_eq!(features.bpm, 0.0, "{features:?}");
            assert_eq!(features.beat, 0.0, "{features:?}");
            assert!(features.onset > 0.0, "{features:?}");
        }

        #[test]
        fn onset_decays_and_tempo_expires_without_new_transients() {
            let buffer = AudioBuffer::new();
            let mut analyzer = AudioAnalyzer::new();
            for chunk in click_track(44_100, 3.15).chunks(512) {
                buffer.push(chunk, 1);
                analyzer.poll(&buffer);
            }
            let features = analyzer.features();
            assert!(features.onset > 0.4, "{features:?}");
            assert!(features.bpm > 0.0);
            analyzer.poll(&buffer);
            assert_eq!(analyzer.features(), features);
            for _ in 0..200 {
                buffer.push(&[0.0; 512], 1);
                analyzer.poll(&buffer);
            }
            assert_eq!(analyzer.features().bpm, 0.0);
            assert_eq!(analyzer.features().beat, 0.0);
            assert!(analyzer.features().onset < 0.001);
        }

        #[test]
        fn rhythm_history_resets_on_sample_rate_change_or_overrun() {
            let buffer = AudioBuffer::new();
            let mut analyzer = AudioAnalyzer::new();
            for chunk in click_track(44_100, 3.15).chunks(512) {
                buffer.push(chunk, 1);
                analyzer.poll(&buffer);
            }
            assert!(analyzer.features().bpm > 0.0);
            assert!(buffer.set_sample_rate(0).is_err());
            buffer.set_sample_rate(48_000).unwrap();
            analyzer.poll(&buffer);
            assert_eq!(analyzer.features(), AudioFeatures::default());
            buffer.push(&vec![0.0; CAPACITY + 1], 1);
            analyzer.poll(&buffer);
            assert_eq!(analyzer.features(), AudioFeatures::default());
            assert_eq!(buffer.pending.lock().unwrap().samples.capacity(), CAPACITY);
        }

        #[test]
        fn per_bin_settings_affect_only_the_selected_frequency_band() {
            let buffer = AudioBuffer::new();
            let samples: Vec<_> = (0..FFT_SIZE).map(|i| (i as f32 * 0.17).sin()).collect();
            buffer.push(&samples, 1);
            let mut analyzer = AudioAnalyzer::new();
            analyzer.set_smooth(0.0);
            let baseline = analyzer.poll(&buffer);
            assert!(baseline[1] > 0.0);
            analyzer.set_bin(1, BinProperty::Scale, 3.0).unwrap();
            let scaled = analyzer.poll(&buffer);
            assert!((scaled[1] - baseline[1] * 3.0).abs() < 1e-6);
            for i in [0, 2, 3] {
                assert_eq!(scaled[i], baseline[i]);
            }
            analyzer.set_bin(1, BinProperty::Smooth, 1.0).unwrap();
            analyzer.set_bin(1, BinProperty::Cutoff, 100.0).unwrap();
            assert_eq!(analyzer.poll(&buffer)[1], scaled[1]);
            analyzer.set_smooth(0.0);
            assert_eq!(analyzer.poll(&buffer)[1], 0.0);
            analyzer.set_bins(4);
            assert_eq!(analyzer.poll(&buffer)[1], baseline[1]);
        }

        #[test]
        fn pushes_keep_only_the_newest_samples_without_growing_the_buffer() {
            let buffer = AudioBuffer::new();
            let capacity = buffer.samples.lock().unwrap().capacity();
            let data: Vec<f32> = (0..FFT_SIZE * 3).map(|i| i as f32).collect();
            for chunk in data.chunks(512) {
                buffer.push(chunk, 1);
            }
            let samples = buffer.samples.lock().unwrap();
            assert_eq!(samples.capacity(), capacity);
            assert_eq!(samples.len(), FFT_SIZE);
            assert!(
                samples
                    .iter()
                    .copied()
                    .eq(data[FFT_SIZE * 2..].iter().copied())
            );
        }

        #[test]
        fn oversized_multichannel_push_preserves_downmix_and_partial_frames() {
            let buffer = AudioBuffer::new();
            let capacity = buffer.samples.lock().unwrap().capacity();
            let data: Vec<f32> = (0..FFT_SIZE * 6 + 1).map(|i| i as f32).collect();
            buffer.push(&data, 2);
            let expected: Vec<f32> = data
                .chunks(2)
                .map(|frame| frame.iter().sum::<f32>() / 2.0)
                .collect();
            let samples = buffer.samples.lock().unwrap();
            assert_eq!(samples.capacity(), capacity);
            assert_eq!(samples.len(), FFT_SIZE);
            assert!(
                samples
                    .iter()
                    .copied()
                    .eq(expected[expected.len() - FFT_SIZE..].iter().copied())
            );
        }

        #[test]
        fn empty_and_zero_channel_pushes_keep_mono_behavior() {
            let buffer = AudioBuffer::new();
            buffer.push(&[], 0);
            buffer.push(&[1.0, -0.0, 3.0], 0);
            let samples = buffer.samples.lock().unwrap();
            assert_eq!(
                samples.iter().copied().collect::<Vec<_>>(),
                vec![1.0, -0.0, 3.0]
            );
            assert!(samples[1].is_sign_negative());
        }
    }
}

#[cfg(feature = "fft")]
pub use analysis::{AudioAnalyzer, AudioBuffer};

#[cfg(feature = "audio")]
mod capture {
    use std::sync::mpsc;
    use std::thread;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use super::NUM_FFT_BINS;
    use super::analysis::{AudioAnalyzer, AudioBuffer};

    enum Command {
        Shutdown,
    }

    #[derive(Clone, Default)]
    enum Backend {
        #[default]
        Default,
        Jack(String),
    }

    pub struct AudioManager {
        command_tx: Option<mpsc::Sender<Command>>,
        error_rx: Option<mpsc::Receiver<String>>,
        backend: Backend,
        buffer: AudioBuffer,
        analyzer: AudioAnalyzer,
    }

    impl Default for AudioManager {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AudioManager {
        /// Doesn't touch the microphone - see `ensure_started`. Constructing
        /// this (e.g. once at app startup) must not, by itself, start
        /// capturing audio; only a script that actually uses `a.*` should.
        pub fn new() -> Self {
            Self {
                command_tx: None,
                error_rx: None,
                backend: Backend::Default,
                buffer: AudioBuffer::new(),
                analyzer: AudioAnalyzer::new(),
            }
        }

        /// Select JACK without opening an audio device. CPAL appends `_in`
        /// to this name; managed callers should pass the NSM client ID.
        pub fn with_jack(client_name: &str) -> std::io::Result<Self> {
            if !cfg!(all(feature = "jack", target_os = "linux")) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "JACK input requires a Linux build with --features jack",
                ));
            }
            if client_name.is_empty() || client_name.contains('\0') {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "JACK client name must be nonempty and contain no NUL bytes",
                ));
            }
            let mut manager = Self::new();
            manager.backend = Backend::Jack(client_name.to_owned());
            Ok(manager)
        }

        /// Opens the selected input stream, if it isn't already
        /// open. Idempotent - safe to call on every frame. Call this only
        /// once a script is known to actually reference `a.*` (checking for
        /// an `AudioRequest` or `iFft` in its compiled output), not
        /// unconditionally at startup: capturing audio is a real privacy-
        /// sensitive action a script shouldn't get "for free" just because
        /// the `audio` feature happens to be compiled in.
        pub fn ensure_started(&mut self) {
            if self.command_tx.is_some() {
                return;
            }
            let (command_tx, command_rx) = mpsc::channel();
            let (error_tx, error_rx) = mpsc::sync_channel(1);
            let buffer = self.buffer.clone();
            let backend = self.backend.clone();
            thread::Builder::new()
                .name("hydra-audio".into())
                .spawn(move || {
                    if let Err(error) = capture_thread(buffer, backend, command_rx, &error_tx) {
                        report_error(&error_tx, error);
                    }
                })
                .expect("spawn audio capture thread");
            self.command_tx = Some(command_tx);
            self.error_rx = Some(error_rx);
        }

        pub fn take_error(&self) -> Option<String> {
            self.error_rx.as_ref()?.try_recv().ok()
        }

        pub fn set_bins(&mut self, n: usize) {
            self.analyzer.set_bins(n);
        }

        pub fn settings(&self) -> &super::AudioSettings {
            self.analyzer.settings()
        }

        pub fn set_cutoff(&mut self, c: f32) {
            self.analyzer.set_cutoff(c);
        }

        pub fn set_scale(&mut self, s: f32) {
            self.analyzer.set_scale(s);
        }

        pub fn set_smooth(&mut self, s: f32) {
            self.analyzer.set_smooth(s);
        }

        pub fn set_bin(
            &mut self,
            index: usize,
            property: super::BinProperty,
            value: f32,
        ) -> Result<(), String> {
            self.analyzer.set_bin(index, property, value)
        }

        pub fn poll(&mut self) -> [f32; NUM_FFT_BINS] {
            self.analyzer.poll(&self.buffer)
        }

        pub fn features(&self) -> super::AudioFeatures {
            self.analyzer.features()
        }
    }

    impl Drop for AudioManager {
        fn drop(&mut self) {
            if let Some(tx) = &self.command_tx {
                let _ = tx.send(Command::Shutdown);
            }
        }
    }

    fn report_error(error_tx: &mpsc::SyncSender<String>, error: String) {
        log::error!("{error}");
        // Keep the first pending error without blocking the audio callback.
        let _ = error_tx.try_send(error);
    }

    #[cfg(all(feature = "jack", target_os = "linux"))]
    fn jack_input(client_name: &str) -> Result<cpal::Device, String> {
        // CPAL 0.15's Host::input_device_with_name() reuses its cached default
        // device, so construct the named device directly instead.
        let device = cpal::platform::JackDevice::default_input_device(client_name, false, false)
            .map_err(|error| format!("Failed to open JACK input: {error}"))?;
        let expected_name = format!("{client_name}_in");
        let actual_name = device.name().map_err(|error| error.to_string())?;
        if actual_name != expected_name {
            return Err(format!(
                "JACK client name {expected_name:?} is unavailable (assigned {actual_name:?})"
            ));
        }
        Ok(device.into())
    }

    #[cfg(not(all(feature = "jack", target_os = "linux")))]
    fn jack_input(_client_name: &str) -> Result<cpal::Device, String> {
        Err("JACK input requires a Linux build with --features jack".into())
    }

    fn capture_thread(
        buffer: AudioBuffer,
        backend: Backend,
        command_rx: mpsc::Receiver<Command>,
        error_tx: &mpsc::SyncSender<String>,
    ) -> Result<(), String> {
        // The cpal Stream must be built and dropped on the thread that owns it: it
        // wraps non-Send platform handles on some backends, so it stays parked here
        // for its whole lifetime rather than being handed back to the caller.
        let device = match backend {
            Backend::Default => cpal::default_host()
                .default_input_device()
                .ok_or("No default audio input device found")?,
            Backend::Jack(client_name) => jack_input(&client_name)?,
        };
        let config = device
            .default_input_config()
            .map_err(|error| format!("Failed to get audio input config: {error}"))?;

        let channels = config.channels() as usize;
        let sample_format = config.sample_format();
        let stream_config = config.config();
        buffer.set_sample_rate(stream_config.sample_rate.0)?;
        let stream_error_tx = error_tx.clone();
        let err_fn = move |error| {
            report_error(
                &stream_error_tx,
                format!("Audio input stream error: {error}"),
            );
        };

        let stream_result = match sample_format {
            cpal::SampleFormat::F32 => {
                let buffer = buffer.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &_| buffer.push(data, channels),
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let buffer = buffer.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &_| {
                        let floats: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        buffer.push(&floats, channels)
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let buffer = buffer.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _: &_| {
                        let floats: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0))
                            .collect();
                        buffer.push(&floats, channels)
                    },
                    err_fn,
                    None,
                )
            }
            other => {
                return Err(format!("Unsupported audio sample format: {other:?}"));
            }
        };

        let stream = stream_result
            .map_err(|error| format!("Failed to build audio input stream: {error}"))?;
        stream
            .play()
            .map_err(|error| format!("Failed to start audio input stream: {error}"))?;

        let _ = command_rx.recv();
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[cfg(all(feature = "jack", target_os = "linux"))]
        #[test]
        fn jack_selection_is_lazy_and_preserves_the_nsm_name() {
            let manager = AudioManager::with_jack("Hydra.nABC").unwrap();
            assert!(matches!(manager.backend, Backend::Jack(ref name) if name == "Hydra.nABC"));
            assert!(manager.command_tx.is_none());
            assert!(manager.take_error().is_none());
            for name in ["", "Hydra\0invalid"] {
                assert_eq!(
                    AudioManager::with_jack(name).err().unwrap().kind(),
                    std::io::ErrorKind::InvalidInput
                );
            }
        }

        #[cfg(not(all(feature = "jack", target_os = "linux")))]
        #[test]
        fn unsupported_jack_selection_is_an_error() {
            assert_eq!(
                AudioManager::with_jack("hydra").err().unwrap().kind(),
                std::io::ErrorKind::Unsupported
            );
        }

        #[test]
        fn capture_errors_reach_the_ui_without_blocking() {
            let (tx, rx) = mpsc::sync_channel(1);
            let mut manager = AudioManager::new();
            manager.error_rx = Some(rx);
            report_error(&tx, "JACK server unavailable".into());
            report_error(&tx, "Another error".into());
            assert_eq!(
                manager.take_error().as_deref(),
                Some("JACK server unavailable")
            );
            assert!(manager.take_error().is_none());
        }
    }
}

#[cfg(feature = "audio")]
pub use capture::AudioManager;
