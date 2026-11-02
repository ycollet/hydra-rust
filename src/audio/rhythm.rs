use std::collections::VecDeque;

use rustfft::num_complex::Complex;

use super::AudioFeatures;
use super::analysis::FFT_SIZE as SIZE;

pub(super) const CAPACITY: usize = 65_536;
const HOP: usize = 256;

pub(super) struct PendingAudio {
    pub samples: VecDeque<f32>,
    pub sample_rate: u32,
    pub discontinuity: bool,
}

impl PendingAudio {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(CAPACITY),
            sample_rate: 44_100,
            discontinuity: false,
        }
    }

    pub fn push(&mut self, sample: f32) {
        if self.samples.len() == CAPACITY {
            self.samples.pop_front();
            self.discontinuity = true;
        }
        self.samples.push_back(sample);
    }
}

pub(super) struct RhythmDetector {
    window: [f32; SIZE],
    filled: usize,
    previous: [f32; SIZE / 2],
    mean_flux: f32,
    time: f64,
    last_onset: Option<f64>,
    intervals: VecDeque<f64>,
    sample_rate: u32,
    pub features: AudioFeatures,
}

impl RhythmDetector {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            window: [0.0; SIZE],
            filled: 0,
            previous: [0.0; SIZE / 2],
            mean_flux: 0.0,
            time: 0.0,
            last_onset: None,
            intervals: VecDeque::with_capacity(8),
            sample_rate,
            features: AudioFeatures::default(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn process(
        &mut self,
        samples: &[f32],
        fft: &dyn rustfft::Fft<f32>,
        scratch: &mut [Complex<f32>],
        hann: &[f32; SIZE],
    ) {
        let decay = (-1.0 / (0.12 * self.sample_rate as f32)).exp();
        for &sample in samples {
            self.time += 1.0 / self.sample_rate as f64;
            self.features.onset *= decay;
            self.window[self.filled] = sample;
            self.filled += 1;
            if self.filled == SIZE {
                for ((slot, &value), &weight) in scratch.iter_mut().zip(&self.window).zip(hann) {
                    *slot = Complex::new(value * weight, 0.0);
                }
                fft.process(scratch);
                self.analyze(&scratch[..SIZE / 2]);
                self.window.copy_within(HOP.., 0);
                self.filled = SIZE - HOP;
            }
        }
        if let Some(last) = self.last_onset {
            if self.time - last > 2.0 {
                self.intervals.clear();
                self.features.bpm = 0.0;
            }
            self.features.beat = if self.features.bpm > 0.0 {
                ((self.time - last) * self.features.bpm as f64 / 60.0).rem_euclid(1.0) as f32
            } else {
                0.0
            };
        }
    }

    fn analyze(&mut self, spectrum: &[Complex<f32>]) {
        let mut flux = 0.0;
        for (previous, value) in self.previous.iter_mut().zip(spectrum) {
            let magnitude = value.norm() / SIZE as f32;
            flux += (magnitude - *previous).max(0.0);
            *previous = magnitude;
        }
        let threshold = (self.mean_flux * 2.5).max(0.01);
        let dt = HOP as f32 / self.sample_rate as f32;
        self.mean_flux += (flux - self.mean_flux) * (1.0 - (-dt / 0.5).exp());
        if flux <= threshold || self.last_onset.is_some_and(|last| self.time - last < 0.2) {
            return;
        }

        self.features.onset = 1.0;
        if let Some(last) = self.last_onset {
            let interval = self.time - last;
            let tolerance = HOP as f64 / self.sample_rate as f64;
            if (0.3 - tolerance..=1.0 + tolerance).contains(&interval) {
                if self.intervals.len() == 8 {
                    self.intervals.pop_front();
                }
                self.intervals.push_back(interval.clamp(0.3, 1.0));
                if self.intervals.len() >= 3 {
                    let mut sorted = [0.0; 8];
                    for (dst, &src) in sorted.iter_mut().zip(&self.intervals) {
                        *dst = src;
                    }
                    let sorted = &mut sorted[..self.intervals.len()];
                    sorted.sort_by(f64::total_cmp);
                    let median = sorted[sorted.len() / 2];
                    // Irregular transients are onsets, not evidence of a stable beat.
                    self.features.bpm = if sorted.iter().all(|i| (i - median).abs() < median * 0.2)
                    {
                        (60.0 / median) as f32
                    } else {
                        0.0
                    };
                }
            } else {
                self.intervals.clear();
                self.features.bpm = 0.0;
            }
        }
        self.last_onset = Some(self.time);
    }
}
