//! Bass analysis: FFT over the most recently played audio, bass energy level with
//! automatic gain, and kick/beat detection via spectral flux in the bass band.

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::effects::Drive;

const FFT_LEN: usize = 2048;
pub const BANDS: usize = 24;
const BASS_LO: f32 = 30.0;
const BASS_HI: f32 = 150.0;
/// Number of past flux values for the adaptive threshold (~1.5 s at 30 fps).
const HISTORY: usize = 48;
/// Fastest allowed beat rate (~330 BPM) to avoid double triggers.
const MIN_BEAT_GAP: f32 = 0.18;
/// Floor for the auto-gain so near-silence isn't amplified into fake beats.
const PEAK_FLOOR: f32 = 0.02;
/// How fast the beat pulse fades (1/s).
const PULSE_DECAY: f32 = 7.0;

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex<f32>>,
    prev_mag: Vec<f32>,
    rate: f32,
    flux_hist: VecDeque<f32>,
    peak: f32,
    since_beat: f32,
    pulse: f32,
    level: f32,
    /// Spectrum for the HUD, 0..1 per log-spaced band.
    pub bands: [f32; BANDS],
}

impl Analyzer {
    pub fn new(rate: u32) -> Self {
        let window = (0..FFT_LEN)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (FFT_LEN - 1) as f32).cos())
            .collect();
        Analyzer {
            fft: FftPlanner::new().plan_fft_forward(FFT_LEN),
            window,
            buf: vec![Complex::default(); FFT_LEN],
            prev_mag: vec![0.0; FFT_LEN / 2],
            rate: rate as f32,
            flux_hist: VecDeque::with_capacity(HISTORY + 1),
            peak: PEAK_FLOOR,
            since_beat: 0.0,
            pulse: 0.0,
            level: 0.0,
            bands: [0.0; BANDS],
        }
    }

    pub fn reset(&mut self) {
        self.prev_mag.fill(0.0);
        self.flux_hist.clear();
        self.peak = PEAK_FLOOR;
        self.pulse = 0.0;
        self.level = 0.0;
        self.bands = [0.0; BANDS];
    }

    /// Called while paused: let everything fade out.
    pub fn idle(&mut self, dt: f32) -> Drive {
        self.pulse *= (-dt * PULSE_DECAY).exp();
        self.level *= (-dt * 4.0).exp();
        for b in &mut self.bands {
            *b = (*b - dt * 1.5).max(0.0);
        }
        Drive {
            pulse: self.pulse,
            level: self.level,
            beat: false,
            strength: 0.0,
        }
    }

    pub fn update(&mut self, samples: &[f32], dt: f32) -> Drive {
        let tail = &samples[samples.len().saturating_sub(FFT_LEN)..];
        let pad = FFT_LEN - tail.len();
        for (i, c) in self.buf.iter_mut().enumerate() {
            let s = if i < pad { 0.0 } else { tail[i - pad] };
            *c = Complex::new(s * self.window[i], 0.0);
        }
        self.fft.process(&mut self.buf);

        // Scale so a full-scale sine reads ~1.0 (Hann window halves the amplitude).
        let norm = 4.0 / FFT_LEN as f32;
        let bin_hz = self.rate / FFT_LEN as f32;
        let lo = ((BASS_LO / bin_hz) as usize).max(1);
        let hi = ((BASS_HI / bin_hz).ceil() as usize).min(FFT_LEN / 2 - 1);

        let (mut energy, mut flux) = (0.0f32, 0.0f32);
        for k in lo..=hi {
            let m = self.buf[k].norm() * norm;
            energy += m * m;
            flux += (m - self.prev_mag[k]).max(0.0);
            self.prev_mag[k] = m;
        }
        let energy = energy.sqrt();

        // Auto-gain: level is bass energy relative to the recent peak.
        self.peak = (self.peak * (-dt / 4.0).exp()).max(energy).max(PEAK_FLOOR);
        self.level = (energy / self.peak).clamp(0.0, 1.0);

        // Beat = bass flux well above its recent average.
        let n = self.flux_hist.len() as f32;
        let mean = if n > 0.0 {
            self.flux_hist.iter().sum::<f32>() / n
        } else {
            0.0
        };
        let var = if n > 0.0 {
            self.flux_hist
                .iter()
                .map(|f| (f - mean).powi(2))
                .sum::<f32>()
                / n
        } else {
            0.0
        };
        let std = var.sqrt();

        self.since_beat += dt;
        self.pulse *= (-dt * PULSE_DECAY).exp();
        let beat = self.flux_hist.len() >= 8
            && flux > mean + 1.5 * std
            && flux > mean * 1.4
            && self.level > 0.35
            && self.since_beat > MIN_BEAT_GAP;
        let strength = if beat {
            ((flux / mean.max(1e-6) - 1.4) / 2.5).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if beat {
            self.since_beat = 0.0;
            self.pulse = self.pulse.max(0.65 + 0.35 * strength);
        }

        self.flux_hist.push_back(flux);
        if self.flux_hist.len() > HISTORY {
            self.flux_hist.pop_front();
        }

        self.update_bands(norm, bin_hz, dt);
        Drive {
            pulse: self.pulse,
            level: self.level,
            beat,
            strength,
        }
    }

    fn update_bands(&mut self, norm: f32, bin_hz: f32, dt: f32) {
        let f_lo = 30.0f32;
        let f_hi = (self.rate * 0.5).min(16_000.0);
        let ratio = f_hi / f_lo;
        let max_bin = FFT_LEN / 2;
        for (b, band) in self.bands.iter_mut().enumerate() {
            let a = f_lo * ratio.powf(b as f32 / BANDS as f32);
            let z = f_lo * ratio.powf((b + 1) as f32 / BANDS as f32);
            let k0 = ((a / bin_hz) as usize).min(max_bin - 1);
            let k1 = ((z / bin_hz) as usize).clamp(k0 + 1, max_bin);
            let m = self.buf[k0..k1]
                .iter()
                .map(|c| c.norm() * norm)
                .fold(0.0f32, f32::max);
            let v = ((20.0 * (m + 1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
            *band = v.max(*band - dt * 1.5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const FPS: usize = 30;

    /// Feed a mono signal the way the app does: one snapshot of the tap per render tick.
    fn beat_times(signal: &[f32]) -> Vec<f32> {
        let mut a = Analyzer::new(RATE);
        let step = RATE as usize / FPS;
        let mut beats = Vec::new();
        let mut end = step;
        while end <= signal.len() {
            let start = end.saturating_sub(4096);
            if a.update(&signal[start..end], 1.0 / FPS as f32).beat {
                beats.push(end as f32 / RATE as f32);
            }
            end += step;
        }
        beats
    }

    #[test]
    fn detects_kicks_at_120_bpm() {
        let mut noise = crate::effects::Rng::new(7);
        let signal: Vec<f32> = (0..RATE as usize * 8)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let since_kick = t % 0.5;
                let kick = (2.0 * PI * 55.0 * since_kick).sin() * (-since_kick * 18.0).exp();
                0.8 * kick + 0.05 * noise.signed()
            })
            .collect();
        let beats = beat_times(&signal);
        // 16 kicks in 8 s; the first few are spent filling the threshold history.
        assert!((12..=16).contains(&beats.len()), "beats: {beats:?}");
        for pair in beats.windows(2) {
            let gap = pair[1] - pair[0];
            assert!((gap - 0.5).abs() < 0.05, "uneven beat gap {gap}: {beats:?}");
        }
    }

    #[test]
    fn silence_and_hiss_have_no_beats() {
        assert!(beat_times(&vec![0.0; RATE as usize * 4]).is_empty());
        let mut noise = crate::effects::Rng::new(3);
        let hiss: Vec<f32> = (0..RATE as usize * 4)
            .map(|_| 0.01 * noise.signed())
            .collect();
        assert!(beat_times(&hiss).is_empty());
    }

    /// Run on real audio: `YTFX_PCM=file.f32 cargo test real_audio -- --ignored --nocapture`
    /// where the file is 48 kHz stereo f32le (e.g. from `ffmpeg ... -ac 2 -ar 48000 -f f32le`).
    #[test]
    #[ignore]
    fn real_audio() {
        let path = std::env::var("YTFX_PCM").expect("set YTFX_PCM");
        let bytes = std::fs::read(path).unwrap();
        let mono: Vec<f32> = bytes
            .chunks_exact(8)
            .map(|b| {
                let l = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let r = f32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                (l + r) * 0.5
            })
            .collect();
        let beats = beat_times(&mono);
        println!(
            "{} beats in {:.1}s: {beats:?}",
            beats.len(),
            mono.len() as f32 / RATE as f32
        );
    }
}
