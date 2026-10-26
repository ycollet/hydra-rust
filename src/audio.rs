pub const NUM_FFT_BINS: usize = 8;

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

    use super::NUM_FFT_BINS;

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
            };
            // macOS allocates the native mutex on first lock, not in Mutex::new().
            drop(buffer.samples.lock().unwrap());
            buffer
        }

        /// Pushes samples, downmixing to mono first if `channels > 1`
        /// (assuming `data` is interleaved, matching `cpal`'s own stream
        /// callback layout). No allocation after construction; the deque
        /// never exceeds its preallocated `FFT_SIZE` capacity.
        pub fn push(&self, data: &[f32], channels: usize) {
            push_samples(&self.samples, data, channels);
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
        num_bins: usize,
        cutoff: f32,
        scale: f32,
        smooth: f32,
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
                num_bins: 4,
                cutoff: 0.0,
                scale: 1.0,
                smooth: 0.4,
            }
        }

        pub fn set_bins(&mut self, n: usize) {
            self.num_bins = n.clamp(1, NUM_FFT_BINS);
        }

        pub fn set_cutoff(&mut self, c: f32) {
            self.cutoff = c.max(0.0);
        }

        pub fn set_scale(&mut self, s: f32) {
            self.scale = s;
        }

        pub fn set_smooth(&mut self, s: f32) {
            self.smooth = s.clamp(0.0, 1.0);
        }

        pub fn poll(&mut self, buffer: &AudioBuffer) -> [f32; NUM_FFT_BINS] {
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

            let bins = reduce_to_bins(&magnitudes, self.num_bins);

            let mut out = [0.0f32; NUM_FFT_BINS];
            for i in 0..self.num_bins {
                let raw = bins[i];
                let gated = if raw < self.cutoff {
                    0.0
                } else {
                    raw * self.scale
                };
                self.smoothed[i] = self.smoothed[i] * self.smooth + gated * (1.0 - self.smooth);
                out[i] = self.smoothed[i];
            }
            out
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

    pub struct AudioManager {
        command_tx: Option<mpsc::Sender<Command>>,
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
                buffer: AudioBuffer::new(),
                analyzer: AudioAnalyzer::new(),
            }
        }

        /// Opens the default microphone input stream, if it isn't already
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
            let buffer = self.buffer.clone();
            thread::Builder::new()
                .name("hydra-audio".into())
                .spawn(move || capture_thread(buffer, command_rx))
                .expect("spawn audio capture thread");
            self.command_tx = Some(command_tx);
        }

        pub fn set_bins(&mut self, n: usize) {
            self.analyzer.set_bins(n);
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

        pub fn poll(&mut self) -> [f32; NUM_FFT_BINS] {
            self.analyzer.poll(&self.buffer)
        }
    }

    impl Drop for AudioManager {
        fn drop(&mut self) {
            if let Some(tx) = &self.command_tx {
                let _ = tx.send(Command::Shutdown);
            }
        }
    }

    fn capture_thread(buffer: AudioBuffer, command_rx: mpsc::Receiver<Command>) {
        // The cpal Stream must be built and dropped on the thread that owns it: it
        // wraps non-Send platform handles on some backends, so it stays parked here
        // for its whole lifetime rather than being handed back to the caller.
        let host = cpal::default_host();
        let Some(device) = host.default_input_device() else {
            log::warn!("no default audio input device found");
            let _ = command_rx.recv();
            return;
        };
        let config = match device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("failed to get default audio input config: {e}");
                let _ = command_rx.recv();
                return;
            }
        };

        let channels = config.channels() as usize;
        let sample_format = config.sample_format();
        let stream_config = config.config();
        let err_fn = |err| log::warn!("audio input stream error: {err}");

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
                log::warn!("unsupported audio sample format: {other:?}");
                let _ = command_rx.recv();
                return;
            }
        };

        let stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                log::warn!("failed to build audio input stream: {e}");
                let _ = command_rx.recv();
                return;
            }
        };

        if let Err(e) = stream.play() {
            log::warn!("failed to start audio input stream: {e}");
        }

        let _ = command_rx.recv();
    }
}

#[cfg(feature = "audio")]
pub use capture::AudioManager;
