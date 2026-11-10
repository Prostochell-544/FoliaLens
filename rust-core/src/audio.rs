//! Bioacoustic DSP: log-Mel spectrogram extraction for on-device sound classification.
//!
//! Everything that does not depend on the signal — periodic Hann window, FFT plan,
//! sparse triangular Mel filterbank, colormap LUT — is computed once in
//! [`MelSpectrogramExtractor::new`]. [`MelSpectrogramExtractor::compute`] then runs the
//! STFT with three small per-call buffers and no per-frame allocations.
//!
//! Output layout of `out_mel_floats` is mel-major (`[n_mels][n_frames]`, row = Mel band
//! from low to high frequency, column = time frame), i.e. a ready `[1, n_mels, n_frames]`
//! model input. The optional RGBA image is `n_frames` wide and `n_mels` tall with low
//! frequencies at the bottom, matching `Bitmap.Config.ARGB_8888` byte order for
//! `Bitmap.copyPixelsFromBuffer`.

use std::f64::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use crate::AtlasError;

/// Power floor before the log: `10·log10(1e-10) = -100 dB`.
const POWER_FLOOR: f32 = 1e-10;
/// Upper bound for the synthetic test signal to keep allocations reasonable.
const MAX_SYNTH_DURATION_SEC: f32 = 60.0;

/// Frequency → Mel mapping used to place the triangular filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MelScale {
    /// Slaney (Auditory Toolbox) scale with Slaney area normalization —
    /// identical to `librosa.filters.mel` defaults.
    Slaney,
    /// HTK scale (`2595·log10(1 + f/700)`) without normalization —
    /// identical to `torchaudio.transforms.MelSpectrogram` defaults.
    Htk,
}

/// Perceptually uniform colormap for the UI spectrogram bitmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colormap {
    Magma,
    Viridis,
}

/// Spectrogram parameters. Defaults are tuned for bird song at 48 kHz.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub n_fft: usize,
    pub hop_length: usize,
    pub n_mels: usize,
    pub f_min: f32,
    pub f_max: f32,
    pub mel_scale: MelScale,
    /// Dynamic range kept below the loudest bin before scaling to `0.0..=1.0`
    /// (as `librosa.power_to_db(top_db=80)`). `None` scales between the global min
    /// and max dB, which lets the −100 dB floor of silent bins dominate the range.
    pub top_db: Option<f32>,
    pub colormap: Colormap,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            n_fft: 2048,
            hop_length: 512,
            n_mels: 96,
            f_min: 150.0,
            f_max: 15_000.0,
            mel_scale: MelScale::Slaney,
            top_db: Some(80.0),
            colormap: Colormap::Magma,
        }
    }
}

impl AudioConfig {
    fn validate(&self) -> Result<(), AtlasError> {
        let nyquist = self.sample_rate as f32 / 2.0;
        let valid = self.sample_rate > 0
            && (16..=1 << 16).contains(&self.n_fft)
            && self.hop_length > 0
            && self.n_mels > 0
            && self.n_mels <= self.n_fft / 2
            && self.f_min.is_finite()
            && self.f_max.is_finite()
            && self.f_min >= 0.0
            && self.f_min < self.f_max
            && self.f_max <= nyquist
            && self.top_db.is_none_or(|db| db.is_finite() && db > 0.0);
        if valid {
            Ok(())
        } else {
            Err(AtlasError::BufferError)
        }
    }
}

/// Non-zero span of one triangular filter inside `filter_weights`.
#[derive(Debug, Clone, Copy)]
struct MelFilter {
    start_bin: usize,
    weights_offset: usize,
    len: usize,
}

/// Sparse Mel filterbank: only the non-zero support of each triangle is stored
/// (a few hundred weights in total instead of `n_mels × (n_fft/2 + 1)`).
struct MelFilterbank {
    filters: Vec<MelFilter>,
    weights: Vec<f32>,
    center_hz: Vec<f32>,
}

fn hz_to_mel(hz: f64, scale: MelScale) -> f64 {
    match scale {
        MelScale::Htk => 2595.0 * (1.0 + hz / 700.0).log10(),
        MelScale::Slaney => {
            const F_SP: f64 = 200.0 / 3.0;
            const MIN_LOG_HZ: f64 = 1000.0;
            let min_log_mel = MIN_LOG_HZ / F_SP;
            let logstep = 6.4f64.ln() / 27.0;
            if hz >= MIN_LOG_HZ {
                min_log_mel + (hz / MIN_LOG_HZ).ln() / logstep
            } else {
                hz / F_SP
            }
        }
    }
}

fn mel_to_hz(mel: f64, scale: MelScale) -> f64 {
    match scale {
        MelScale::Htk => 700.0 * (10f64.powf(mel / 2595.0) - 1.0),
        MelScale::Slaney => {
            const F_SP: f64 = 200.0 / 3.0;
            const MIN_LOG_HZ: f64 = 1000.0;
            let min_log_mel = MIN_LOG_HZ / F_SP;
            let logstep = 6.4f64.ln() / 27.0;
            if mel >= min_log_mel {
                MIN_LOG_HZ * (logstep * (mel - min_log_mel)).exp()
            } else {
                F_SP * mel
            }
        }
    }
}

fn build_mel_filterbank(cfg: &AudioConfig) -> MelFilterbank {
    let n_bins = cfg.n_fft / 2 + 1;
    let bin_hz = cfg.sample_rate as f64 / cfg.n_fft as f64;
    let mel_lo = hz_to_mel(cfg.f_min as f64, cfg.mel_scale);
    let mel_hi = hz_to_mel(cfg.f_max as f64, cfg.mel_scale);
    let edges: Vec<f64> = (0..cfg.n_mels + 2)
        .map(|i| {
            let mel = mel_lo + (mel_hi - mel_lo) * i as f64 / (cfg.n_mels + 1) as f64;
            mel_to_hz(mel, cfg.mel_scale)
        })
        .collect();

    let mut filters = Vec::with_capacity(cfg.n_mels);
    let mut weights = Vec::new();
    let mut center_hz = Vec::with_capacity(cfg.n_mels);

    for band in edges.windows(3) {
        let (lo, center, hi) = (band[0], band[1], band[2]);
        let norm = match cfg.mel_scale {
            MelScale::Slaney => 2.0 / (hi - lo),
            MelScale::Htk => 1.0,
        };
        let weights_offset = weights.len();
        let mut start_bin = None;
        for k in 0..n_bins {
            let f = k as f64 * bin_hz;
            let rising = (f - lo) / (center - lo);
            let falling = (hi - f) / (hi - center);
            let w = rising.min(falling);
            if w > 0.0 {
                start_bin.get_or_insert(k);
                weights.push((w * norm) as f32);
            } else if start_bin.is_some() {
                break; // Triangle support is contiguous.
            }
        }
        filters.push(MelFilter {
            start_bin: start_bin.unwrap_or(0),
            weights_offset,
            len: weights.len() - weights_offset,
        });
        center_hz.push(center as f32);
    }

    MelFilterbank {
        filters,
        weights,
        center_hz,
    }
}

/// Periodic Hann window (`scipy.signal.get_window("hann", n, fftbins=True)`).
fn periodic_hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()) as f32)
        .collect()
}

// Ten evenly spaced samples of the matplotlib colormaps (identical to R's
// `viridisLite::magma(10)` / `viridis(10)`), linearly interpolated to 256 entries.
const MAGMA_STOPS: [[u8; 3]; 10] = [
    [0x00, 0x00, 0x04],
    [0x18, 0x0F, 0x3E],
    [0x45, 0x10, 0x77],
    [0x72, 0x1F, 0x81],
    [0x9F, 0x2F, 0x7F],
    [0xCD, 0x40, 0x71],
    [0xF1, 0x60, 0x5D],
    [0xFD, 0x95, 0x67],
    [0xFE, 0xC9, 0x8D],
    [0xFC, 0xFD, 0xBF],
];
const VIRIDIS_STOPS: [[u8; 3]; 10] = [
    [0x44, 0x01, 0x54],
    [0x48, 0x28, 0x78],
    [0x3E, 0x4A, 0x89],
    [0x31, 0x68, 0x8E],
    [0x26, 0x82, 0x8E],
    [0x1F, 0x9E, 0x89],
    [0x35, 0xB7, 0x79],
    [0x6D, 0xCD, 0x59],
    [0xB4, 0xDE, 0x2C],
    [0xFD, 0xE7, 0x25],
];

fn build_colormap_lut(map: Colormap) -> [[u8; 4]; 256] {
    let stops = match map {
        Colormap::Magma => &MAGMA_STOPS,
        Colormap::Viridis => &VIRIDIS_STOPS,
    };
    let segments = (stops.len() - 1) as f32;
    let mut lut = [[0u8; 4]; 256];
    for (i, px) in lut.iter_mut().enumerate() {
        let pos = i as f32 / 255.0 * segments;
        let idx = (pos.floor() as usize).min(stops.len() - 2);
        let frac = pos - idx as f32;
        let (a, b) = (stops[idx], stops[idx + 1]);
        for c in 0..3 {
            let v = a[c] as f32 + (b[c] as f32 - a[c] as f32) * frac;
            px[c] = v.round().clamp(0.0, 255.0) as u8;
        }
        px[3] = 255;
    }
    lut
}

/// Precomputed STFT → log-Mel pipeline. Immutable after construction, so one instance
/// can be shared across threads (`Send + Sync`).
pub struct MelSpectrogramExtractor {
    config: AudioConfig,
    window: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    filterbank: MelFilterbank,
    colormap_lut: [[u8; 4]; 256],
}

impl MelSpectrogramExtractor {
    /// Builds the window, FFT plan, filterbank and colormap.
    ///
    /// Returns [`AtlasError::BufferError`] if the configuration is inconsistent
    /// (e.g. `f_max` above Nyquist, zero hop, more Mel bands than FFT bins allow).
    pub fn new(config: AudioConfig) -> Result<Self, AtlasError> {
        config.validate()?;
        let window = periodic_hann(config.n_fft);
        let fft = FftPlanner::<f32>::new().plan_fft_forward(config.n_fft);
        let filterbank = build_mel_filterbank(&config);
        let colormap_lut = build_colormap_lut(config.colormap);
        Ok(Self {
            config,
            window,
            fft,
            filterbank,
            colormap_lut,
        })
    }

    pub fn config(&self) -> &AudioConfig {
        &self.config
    }

    /// Center frequency (Hz) of every Mel band, low to high.
    pub fn mel_center_frequencies(&self) -> &[f32] {
        &self.filterbank.center_hz
    }

    /// Number of STFT frames for `num_samples` samples (no centering/padding;
    /// a signal shorter than `n_fft` yields one zero-padded frame).
    pub fn num_frames(&self, num_samples: usize) -> usize {
        num_samples.saturating_sub(self.config.n_fft) / self.config.hop_length + 1
    }

    /// Number of `f32` values (and RGBA pixels) produced for `num_samples` samples.
    pub fn output_len(&self, num_samples: usize) -> Option<usize> {
        self.config.n_mels.checked_mul(self.num_frames(num_samples))
    }

    /// Maps a normalized value in `0.0..=1.0` to an RGBA8888 colormap pixel.
    pub fn colormap_rgba(&self, value: f32) -> [u8; 4] {
        let idx = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as usize;
        self.colormap_lut[idx.min(255)]
    }

    /// Computes the normalized log-Mel spectrogram of `pcm_samples`.
    ///
    /// * `out_mel_floats` — at least `n_mels · n_frames` values; receives the
    ///   mel-major spectrogram scaled to `0.0..=1.0`.
    /// * `out_ui_rgba` — optional, at least `n_mels · n_frames · 4` bytes; receives a
    ///   colormapped image (width `n_frames`, height `n_mels`, high frequencies on top).
    ///
    /// Non-finite input samples are treated as silence. Returns `(n_mels, n_frames)`.
    pub fn compute(
        &self,
        pcm_samples: &[f32],
        out_mel_floats: &mut [f32],
        out_ui_rgba: Option<&mut [u8]>,
    ) -> Result<(usize, usize), AtlasError> {
        if pcm_samples.is_empty() {
            return Err(AtlasError::BufferError);
        }
        let n_mels = self.config.n_mels;
        let n_frames = self.num_frames(pcm_samples.len());
        let total = self
            .output_len(pcm_samples.len())
            .ok_or(AtlasError::BufferError)?;
        if out_mel_floats.len() < total {
            return Err(AtlasError::BufferError);
        }
        if let Some(rgba) = out_ui_rgba.as_deref() {
            let needed = total.checked_mul(4).ok_or(AtlasError::BufferError)?;
            if rgba.len() < needed {
                return Err(AtlasError::BufferError);
            }
        }

        let n_fft = self.config.n_fft;
        let hop = self.config.hop_length;
        let n_bins = n_fft / 2 + 1;
        let mel = &mut out_mel_floats[..total];

        let mut frame = vec![Complex32::new(0.0, 0.0); n_fft];
        let mut scratch = vec![Complex32::new(0.0, 0.0); self.fft.get_inplace_scratch_len()];
        let mut power = vec![0.0f32; n_bins];
        let mut max_db = f32::NEG_INFINITY;
        let mut min_db = f32::INFINITY;

        for t in 0..n_frames {
            let start = t * hop;
            let available = pcm_samples.len().saturating_sub(start).min(n_fft);
            let src = &pcm_samples[start.min(pcm_samples.len())..][..available];
            for ((dst, &s), &w) in frame.iter_mut().zip(src).zip(&self.window) {
                let s = if s.is_finite() { s } else { 0.0 };
                *dst = Complex32::new(s * w, 0.0);
            }
            for dst in &mut frame[available..] {
                *dst = Complex32::new(0.0, 0.0);
            }

            self.fft.process_with_scratch(&mut frame, &mut scratch);

            for (p, c) in power.iter_mut().zip(&frame[..n_bins]) {
                *p = c.re * c.re + c.im * c.im;
            }

            for (m, filt) in self.filterbank.filters.iter().enumerate() {
                let weights =
                    &self.filterbank.weights[filt.weights_offset..filt.weights_offset + filt.len];
                let bins = &power[filt.start_bin..filt.start_bin + filt.len];
                let energy: f32 = weights.iter().zip(bins).map(|(w, p)| w * p).sum();
                let db = 10.0 * energy.max(POWER_FLOOR).log10();
                mel[m * n_frames + t] = db;
                max_db = max_db.max(db);
                min_db = min_db.min(db);
            }
        }

        let floor = match self.config.top_db {
            Some(top_db) => (max_db - top_db).max(min_db),
            None => min_db,
        };
        let range = max_db - floor;
        if range > f32::EPSILON {
            let inv = 1.0 / range;
            for v in mel.iter_mut() {
                *v = ((*v - floor) * inv).clamp(0.0, 1.0);
            }
        } else {
            mel.fill(0.0);
        }

        if let Some(rgba) = out_ui_rgba {
            let row_bytes = n_frames * 4;
            for (row, dst_row) in rgba[..total * 4].chunks_exact_mut(row_bytes).enumerate() {
                // Image row 0 is the top → highest Mel band.
                let band = n_mels - 1 - row;
                let src_row = &mel[band * n_frames..(band + 1) * n_frames];
                for (px, &v) in dst_row.chunks_exact_mut(4).zip(src_row) {
                    px.copy_from_slice(&self.colormap_rgba(v));
                }
            }
        }

        Ok((n_mels, n_frames))
    }
}

/// Minimal deterministic PRNG for reproducible background noise.
struct XorShift32(u32);

impl XorShift32 {
    /// Uniform sample in `-1.0..=1.0`.
    fn next_signed(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x as f64 / u32::MAX as f64 * 2.0 - 1.0
    }
}

/// Synthesizes a nightingale-like trill for testing the audio path without a microphone.
///
/// Repeating 85 ms syllables separated by 35 ms gaps; each syllable is an exponential
/// up-sweep 2.2 → 5.8 kHz with slight vibrato, a smooth bell-shaped amplitude envelope
/// and a second harmonic at −11 dB. The phrase crescendos with short fade-in/out, over
/// a −48 dBFS white-noise floor (fixed seed, so output is deterministic). Components
/// above 0.45·`sample_rate` are omitted to avoid aliasing.
///
/// Returns an empty vector for a zero sample rate or non-positive/non-finite duration;
/// durations above 60 s are clamped.
pub fn generate_synthetic_bird_chirp(sample_rate: u32, duration_sec: f32) -> Vec<f32> {
    if sample_rate == 0 || !duration_sec.is_finite() || duration_sec <= 0.0 {
        return Vec::new();
    }
    const F_START: f64 = 2_200.0;
    const F_END: f64 = 5_800.0;
    const SYLLABLE_SEC: f64 = 0.085;
    const PERIOD_SEC: f64 = 0.120;
    const VIBRATO_HZ: f64 = 38.0;
    const VIBRATO_DEPTH: f64 = 0.015;
    const HARMONIC_GAIN: f64 = 0.28;
    const NOISE_GAIN: f64 = 0.004;
    const AMPLITUDE: f64 = 0.7;
    const FADE_SEC: f64 = 0.03;

    let sr = sample_rate as f64;
    let duration = duration_sec.min(MAX_SYNTH_DURATION_SEC) as f64;
    let n = (duration * sr).round() as usize;
    let alias_limit = 0.45 * sr;
    let sweep_ratio = F_END / F_START;

    let mut rng = XorShift32(0x9E37_79B9);
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(n);

    for i in 0..n {
        let t = i as f64 / sr;
        let t_in = t % PERIOD_SEC;
        let mut tone = 0.0;
        if t_in < SYLLABLE_SEC {
            let p = t_in / SYLLABLE_SEC;
            let vibrato = 1.0 + VIBRATO_DEPTH * (2.0 * PI * VIBRATO_HZ * t).sin();
            let f = F_START * sweep_ratio.powf(p) * vibrato;
            phase = (phase + 2.0 * PI * f / sr) % (2.0 * PI);
            let envelope = (PI * p).sin().powi(2);
            if f < alias_limit {
                tone += phase.sin();
            }
            if 2.0 * f < alias_limit {
                tone += HARMONIC_GAIN * (2.0 * phase).sin();
            }
            tone *= envelope;
        } else {
            phase = 0.0;
        }
        let crescendo = 0.55 + 0.45 * (t / duration);
        let fade = (t / FADE_SEC).min((duration - t) / FADE_SEC).clamp(0.0, 1.0);
        let noise = NOISE_GAIN * rng.next_signed();
        out.push((AMPLITUDE * tone * crescendo * fade + noise) as f32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sample_rate: u32, n: usize, amp: f32) -> Vec<f32> {
        // Phase in f64: f32 loses ~1e-2 rad at the ~75k rad reached after 3 s at 4 kHz.
        (0..n)
            .map(|i| {
                let phase = 2.0 * PI * freq as f64 * i as f64 / sample_rate as f64;
                amp * phase.sin() as f32
            })
            .collect()
    }

    /// Mel band with the highest mean normalized energy over all frames.
    fn loudest_band(mel: &[f32], n_mels: usize, n_frames: usize) -> usize {
        (0..n_mels)
            .map(|m| {
                let row = &mel[m * n_frames..(m + 1) * n_frames];
                (m, row.iter().sum::<f32>() / n_frames as f32)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(m, _)| m)
            .unwrap()
    }

    #[test]
    fn default_config_matches_spec() {
        let c = AudioConfig::default();
        assert_eq!(
            (c.sample_rate, c.n_fft, c.hop_length, c.n_mels),
            (48_000, 2048, 512, 96)
        );
        assert_eq!((c.f_min, c.f_max), (150.0, 15_000.0));
    }

    #[test]
    fn num_frames_formula() {
        let ex = MelSpectrogramExtractor::new(AudioConfig::default()).unwrap();
        assert_eq!(ex.num_frames(144_000), 278);
        assert_eq!(ex.num_frames(2048), 1);
        assert_eq!(ex.num_frames(2048 + 511), 1);
        assert_eq!(ex.num_frames(2048 + 512), 2);
        assert_eq!(ex.num_frames(100), 1);
        assert_eq!(ex.num_frames(0), 1);
    }

    #[test]
    fn window_and_filterbank_are_well_formed() {
        let ex = MelSpectrogramExtractor::new(AudioConfig::default()).unwrap();
        assert_eq!(ex.window.len(), 2048);
        assert_eq!(ex.window[0], 0.0);
        assert!((ex.window[1024] - 1.0).abs() < 1e-6, "periodic Hann peaks at N/2");

        let fb = &ex.filterbank;
        assert_eq!(fb.filters.len(), 96);
        for (m, f) in fb.filters.iter().enumerate() {
            assert!(f.len > 0, "Mel band {m} has no FFT bins");
            assert!(f.start_bin + f.len <= 1025);
        }
        assert!(fb.weights.len() < 96 * 1025 / 10, "filterbank should be sparse");
        assert!(fb.center_hz.windows(2).all(|w| w[0] < w[1]));
        assert!(fb.center_hz[0] > 150.0 && fb.center_hz[95] < 15_000.0);
    }

    #[test]
    fn rejects_invalid_config() {
        let bad = [
            AudioConfig { f_max: 30_000.0, ..Default::default() },
            AudioConfig { hop_length: 0, ..Default::default() },
            AudioConfig { n_mels: 0, ..Default::default() },
            AudioConfig { f_min: 16_000.0, ..Default::default() },
            AudioConfig { sample_rate: 0, ..Default::default() },
            AudioConfig { top_db: Some(f32::NAN), ..Default::default() },
        ];
        for cfg in bad {
            assert!(MelSpectrogramExtractor::new(cfg.clone()).is_err(), "{cfg:?}");
        }
    }

    #[test]
    fn sine_4khz_peaks_in_matching_mel_band() {
        let ex = MelSpectrogramExtractor::new(AudioConfig::default()).unwrap();
        let pcm = sine(4000.0, 48_000, 144_000, 0.5);
        let n_frames = ex.num_frames(pcm.len());
        let n_mels = 96;
        let mut mel = vec![-1.0f32; n_mels * n_frames];
        let mut rgba = vec![0u8; n_mels * n_frames * 4];

        let dims = ex.compute(&pcm, &mut mel, Some(&mut rgba)).unwrap();
        assert_eq!(dims, (96, 278));

        assert!(mel.iter().all(|v| (0.0..=1.0).contains(v)));
        assert_eq!(mel.iter().copied().fold(f32::MIN, f32::max), 1.0);

        let centers = ex.mel_center_frequencies();
        let expected = centers
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - 4000.0).abs().total_cmp(&(b.1 - 4000.0).abs()))
            .map(|(m, _)| m)
            .unwrap();
        let peak = loudest_band(&mel, n_mels, n_frames);
        assert!(
            peak.abs_diff(expected) <= 1 && (centers[peak] - 4000.0).abs() < 200.0,
            "peak band {peak} ({} Hz), expected {expected} ({} Hz)",
            centers[peak],
            centers[expected]
        );

        // Every pixel opaque and equal to the colormap of the vertically flipped band.
        assert!(rgba.chunks_exact(4).all(|px| px[3] == 255));
        for row in 0..n_mels {
            let band = n_mels - 1 - row;
            for t in (0..n_frames).step_by(37) {
                let px = &rgba[(row * n_frames + t) * 4..][..4];
                assert_eq!(px, ex.colormap_rgba(mel[band * n_frames + t]));
            }
        }
        // The 4 kHz line sits at image row n_mels-1-peak and is the brightest one.
        let row_of_peak = n_mels - 1 - peak;
        let mid = n_frames / 2;
        assert_eq!(&rgba[(row_of_peak * n_frames + mid) * 4..][..4], ex.colormap_rgba(1.0));
    }

    #[test]
    fn mel_only_mode_and_buffer_checks() {
        let ex = MelSpectrogramExtractor::new(AudioConfig::default()).unwrap();
        let pcm = sine(1000.0, 48_000, 48_000, 0.3);
        let total = ex.output_len(pcm.len()).unwrap();
        let mut mel = vec![0.0f32; total];
        assert!(ex.compute(&pcm, &mut mel, None).is_ok());

        let mut short_mel = vec![0.0f32; total - 1];
        assert!(ex.compute(&pcm, &mut short_mel, None).is_err());
        let mut short_rgba = vec![0u8; total * 4 - 1];
        assert!(ex.compute(&pcm, &mut mel, Some(&mut short_rgba)).is_err());
        assert!(ex.compute(&[], &mut mel, None).is_err());
    }

    #[test]
    fn silence_and_garbage_input_are_safe() {
        let ex = MelSpectrogramExtractor::new(AudioConfig::default()).unwrap();
        let mut mel = vec![0.5f32; ex.output_len(4096).unwrap()];
        ex.compute(&vec![0.0; 4096], &mut mel, None).unwrap();
        assert!(mel.iter().all(|&v| v == 0.0));

        let mut noisy = sine(3000.0, 48_000, 4096, 0.5);
        noisy[10] = f32::NAN;
        noisy[20] = f32::INFINITY;
        ex.compute(&noisy, &mut mel, None).unwrap();
        assert!(mel.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));

        // Shorter than n_fft → one zero-padded frame.
        let mut one = vec![0.0f32; 96];
        assert_eq!(ex.compute(&noisy[..500], &mut one, None).unwrap(), (96, 1));
    }

    #[test]
    fn synthetic_chirp_is_bounded_and_in_band() {
        let pcm = generate_synthetic_bird_chirp(48_000, 3.0);
        assert_eq!(pcm.len(), 144_000);
        assert!(pcm.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert_eq!(pcm, generate_synthetic_bird_chirp(48_000, 3.0), "deterministic");

        let ex = MelSpectrogramExtractor::new(AudioConfig::default()).unwrap();
        let n_frames = ex.num_frames(pcm.len());
        let mut mel = vec![0.0f32; 96 * n_frames];
        ex.compute(&pcm, &mut mel, None).unwrap();
        let peak_hz = ex.mel_center_frequencies()[loudest_band(&mel, 96, n_frames)];
        assert!((2_000.0..=6_200.0).contains(&peak_hz), "chirp energy peaks at {peak_hz} Hz");

        assert!(generate_synthetic_bird_chirp(0, 1.0).is_empty());
        assert!(generate_synthetic_bird_chirp(48_000, -1.0).is_empty());
        assert!(generate_synthetic_bird_chirp(48_000, f32::NAN).is_empty());
        assert_eq!(generate_synthetic_bird_chirp(8_000, 0.5).len(), 4_000);
    }

    #[test]
    fn colormaps_are_opaque_and_monotonic_in_brightness() {
        for map in [Colormap::Magma, Colormap::Viridis] {
            let lut = build_colormap_lut(map);
            assert!(lut.iter().all(|px| px[3] == 255));
            let luma = |px: &[u8; 4]| 299 * px[0] as u32 + 587 * px[1] as u32 + 114 * px[2] as u32;
            assert!(luma(&lut[255]) > luma(&lut[128]) && luma(&lut[128]) > luma(&lut[0]));
        }
    }
}
