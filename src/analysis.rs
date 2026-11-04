//! Audio analysis: FFT over the most recently played audio, and spike detection in
//! three bands. Bass spikes are beats and drive the effects fully; mid (snares, claps)
//! and high (hats, cymbals) spikes give smaller pulses.

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::effects::Drive;

const FFT_LEN: usize = 2048;
pub const BANDS: usize = 24;
/// Fastest allowed hit rate per band (~330 BPM) to avoid double triggers.
const MIN_BEAT_GAP: f32 = 0.18;
/// The auto-gain also never drops below this share of the loudest energy of the last
/// ~LONG_DECAY seconds, so quiet passages of a loud track stay quiet.
const LONG_SHARE: f32 = 0.35;
const LONG_DECAY: f32 = 30.0;
/// Time constant (s) of the auto-gain peak. Slow, so the quiet stretch after a big
/// accent isn't scaled back up to full level within a few seconds.
const PEAK_DECAY: f32 = 15.0;

// A hit is a spike: band energy that comes near the recent peak (HIT), having jumped
// out of a dip (at least RISE times the lowest energy of the last TROUGH_WINDOW). After a
// hit, the level has to fall below REARM before the next one. Held or swelling sounds
// (orchestral low strings, long 808s, sung vowels) wobble a lot from frame to frame but
// never do all three, so they don't trigger.
const HIT: f32 = 0.6;
const REARM: f32 = 0.5;
const RISE: f32 = 2.0;
const TROUGH_WINDOW: f32 = 0.3;

/// One frequency band to watch for spikes.
struct BandSpec {
    lo: f32,
    hi: f32,
    /// Floor for the auto-gain so near-silence isn't amplified into fake hits.
    peak_floor: f32,
    /// Average two frames of energy (bass only: a 43 ms window holds just a few
    /// cycles, so its energy jitters; higher bands have enough cycles and short hits).
    smooth: bool,
    /// Pulse a full-strength hit gives, relative to a bass beat.
    weight: f32,
    /// How fast the band's pulse fades (1/s).
    decay: f32,
}

const BASS: BandSpec = BandSpec {
    lo: 30.0,
    hi: 150.0,
    // Real kicks in normally mastered music reach 0.2-0.8.
    peak_floor: 0.08,
    smooth: true,
    weight: 1.0,
    decay: 7.0,
};
const MID: BandSpec = BandSpec {
    lo: 250.0,
    hi: 2_000.0,
    peak_floor: 0.04,
    smooth: false,
    weight: 0.55,
    decay: 9.0,
};
const HIGH: BandSpec = BandSpec {
    lo: 4_000.0,
    hi: 12_000.0,
    peak_floor: 0.025,
    smooth: false,
    weight: 0.3,
    decay: 12.0,
};

/// Spike detector state for one band.
struct Detector {
    spec: &'static BandSpec,
    /// Unsmoothed energy of the previous frame.
    prev_energy: f32,
    /// Recent (age in seconds, energy), for the trough.
    recent: VecDeque<(f32, f32)>,
    peak: f32,
    long_peak: f32,
    armed: bool,
    since_hit: f32,
    pulse: f32,
    level: f32,
}

impl Detector {
    fn new(spec: &'static BandSpec) -> Self {
        Detector {
            spec,
            prev_energy: 0.0,
            recent: VecDeque::new(),
            peak: spec.peak_floor,
            long_peak: spec.peak_floor,
            armed: true,
            since_hit: 0.0,
            pulse: 0.0,
            level: 0.0,
        }
    }

    fn reset(&mut self) {
        *self = Detector::new(self.spec);
    }

    fn idle(&mut self, dt: f32) {
        self.pulse *= (-dt * self.spec.decay).exp();
        self.level *= (-dt * 4.0).exp();
    }

    /// Feed one frame's raw band energy. Returns the hit strength (0..1) on a hit.
    fn step(&mut self, raw: f32, dt: f32) -> Option<f32> {
        let spec = self.spec;
        let energy = if spec.smooth {
            0.5 * (raw + self.prev_energy)
        } else {
            raw
        };
        self.prev_energy = raw;

        // Auto-gain: level is energy relative to the recent peak.
        self.long_peak = (self.long_peak * (-dt / LONG_DECAY).exp()).max(energy);
        self.peak = (self.peak * (-dt / PEAK_DECAY).exp())
            .max(energy)
            .max(spec.peak_floor)
            .max(LONG_SHARE * self.long_peak);
        self.level = (energy / self.peak).clamp(0.0, 1.0);

        for e in &mut self.recent {
            e.0 += dt;
        }
        while self.recent.front().is_some_and(|e| e.0 > TROUGH_WINDOW) {
            self.recent.pop_front();
        }
        let trough = self
            .recent
            .iter()
            .map(|e| e.1)
            .fold(f32::INFINITY, f32::min);
        let rise = if trough.is_finite() {
            energy / trough.max(spec.peak_floor * 0.05)
        } else {
            0.0
        };
        self.recent.push_back((0.0, energy));

        self.since_hit += dt;
        self.pulse *= (-dt * spec.decay).exp();
        if self.level < REARM {
            self.armed = true;
        }
        let hit = self.armed && self.level > HIT && rise > RISE && self.since_hit > MIN_BEAT_GAP;
        if !hit {
            return None;
        }
        let strength =
            (0.5 * (rise - RISE) / 4.0 + 0.5 * (self.level - HIT) / (1.0 - HIT)).clamp(0.0, 1.0);
        self.armed = false;
        self.since_hit = 0.0;
        self.pulse = self.pulse.max(spec.weight * (0.65 + 0.35 * strength));
        Some(strength)
    }
}

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex<f32>>,
    rate: f32,
    bass: Detector,
    mid: Detector,
    high: Detector,
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
            rate: rate as f32,
            bass: Detector::new(&BASS),
            mid: Detector::new(&MID),
            high: Detector::new(&HIGH),
            bands: [0.0; BANDS],
        }
    }

    pub fn reset(&mut self) {
        self.bass.reset();
        self.mid.reset();
        self.high.reset();
        self.bands = [0.0; BANDS];
    }

    /// The strongest pulse of the three bands.
    fn pulse(&self) -> f32 {
        self.bass.pulse.max(self.mid.pulse).max(self.high.pulse)
    }

    /// Called while paused: let everything fade out.
    pub fn idle(&mut self, dt: f32) -> Drive {
        self.bass.idle(dt);
        self.mid.idle(dt);
        self.high.idle(dt);
        for b in &mut self.bands {
            *b = (*b - dt * 1.5).max(0.0);
        }
        Drive {
            pulse: self.pulse(),
            level: self.bass.level,
            beat: false,
            strength: 0.0,
        }
    }

    /// RMS-style energy of the bins between `lo` and `hi` Hz.
    fn band_energy(&self, lo: f32, hi: f32, norm: f32, bin_hz: f32) -> f32 {
        let lo = ((lo / bin_hz) as usize).max(1);
        let hi = ((hi / bin_hz).ceil() as usize).min(FFT_LEN / 2 - 1);
        (lo..=hi)
            .map(|k| (self.buf[k].norm() * norm).powi(2))
            .sum::<f32>()
            .sqrt()
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
        let bass = self.band_energy(BASS.lo, BASS.hi, norm, bin_hz);
        let mid = self.band_energy(MID.lo, MID.hi, norm, bin_hz);
        // Bands above Nyquist (low sample rates) just read silence.
        let high = self.band_energy(HIGH.lo, HIGH.hi, norm, bin_hz);

        // Only bass hits count as beats: they also fire the one-shot effects (glitch,
        // invert). Mid and high hits only raise the pulse.
        let beat = self.bass.step(bass, dt);
        self.mid.step(mid, dt);
        self.high.step(high, dt);

        self.update_bands(norm, bin_hz, dt);
        Drive {
            pulse: self.pulse(),
            level: self.bass.level,
            beat: beat.is_some(),
            strength: beat.unwrap_or(0.0),
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

    /// A held bass line: legato note changes, tremolo and slow swells, but no hits.
    fn bass_line(secs: f32) -> Vec<f32> {
        let mut phase = 0.0f32;
        (0..(RATE as f32 * secs) as usize)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let freq = [55.0, 65.4, 49.0, 73.4][(t / 1.3) as usize % 4];
                phase += 2.0 * PI * freq / RATE as f32;
                let swell = 0.35 + 0.15 * (2.0 * PI * 0.25 * t).sin();
                let tremolo = 1.0 + 0.3 * (2.0 * PI * 7.0 * t).sin();
                let fade_in = (t / 0.8).min(1.0);
                swell * tremolo * fade_in * phase.sin()
            })
            .collect()
    }

    #[test]
    fn held_bass_lines_are_not_beats() {
        // Only the line's entry from silence may count.
        let beats = beat_times(&bass_line(10.0));
        assert!(beats.iter().all(|&t| t < 1.0), "beats: {beats:?}");
    }

    #[test]
    fn kicks_over_a_bass_line_still_hit() {
        let line = bass_line(8.0);
        let signal: Vec<f32> = line
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let since_kick = (i as f32 / RATE as f32) % 0.5;
                let kick = (2.0 * PI * 55.0 * since_kick).sin() * (-since_kick * 18.0).exp();
                0.4 * b + 0.9 * kick
            })
            .collect();
        let beats = beat_times(&signal);
        assert!(beats.len() >= 13, "beats: {beats:?}");
    }

    /// Feed a signal like `beat_times`; returns (bass beats, mid hits, high hits, max pulse).
    fn band_hits(signal: &[f32]) -> (usize, usize, usize, f32) {
        let mut a = Analyzer::new(RATE);
        let step = RATE as usize / FPS;
        let (mut beats, mut mids, mut highs, mut max_pulse) = (0, 0, 0, 0.0f32);
        let mut end = step;
        while end <= signal.len() {
            let start = end.saturating_sub(4096);
            let (m, h) = (a.mid.since_hit, a.high.since_hit);
            let d = a.update(&signal[start..end], 1.0 / FPS as f32);
            beats += d.beat as usize;
            mids += (a.mid.since_hit < m) as usize;
            highs += (a.high.since_hit < h) as usize;
            max_pulse = max_pulse.max(d.pulse);
            end += step;
        }
        (beats, mids, highs, max_pulse)
    }

    /// Hits every 0.5 s. A snare is a decaying tone pair in the mid band; a hat is
    /// differenced noise (rises 6 dB/oct, so most of its energy sits up high).
    fn bursts(secs: f32, decay: f32, hat: bool) -> Vec<f32> {
        let mut noise = crate::effects::Rng::new(11);
        let mut prev = 0.0f32;
        (0..(RATE as f32 * secs) as usize)
            .map(|i| {
                let since = (i as f32 / RATE as f32) % 0.5;
                let x = if hat {
                    let n = noise.signed();
                    let y = n - prev;
                    prev = n;
                    0.5 * y
                } else {
                    let tone = |f: f32| (2.0 * PI * f * since).sin();
                    0.6 * tone(400.0) + 0.4 * tone(1_000.0)
                };
                0.6 * x * (-since * decay).exp()
            })
            .collect()
    }

    #[test]
    fn snares_give_a_mid_pulse() {
        let (beats, mids, _, pulse) = band_hits(&bursts(6.0, 25.0, false));
        assert!(mids >= 9, "mid hits: {mids}");
        assert!(beats <= 1, "snares counted as beats: {beats}");
        assert!(
            (MID.weight * 0.6..=MID.weight + 0.01).contains(&pulse),
            "pulse {pulse}"
        );
    }

    #[test]
    fn hats_give_a_small_high_pulse() {
        let (beats, _, highs, pulse) = band_hits(&bursts(6.0, 60.0, true));
        assert!(highs >= 9, "high hits: {highs}");
        assert_eq!(beats, 0);
        // Hats also leak into the mid band, which may lift the pulse to the mid weight.
        assert!(pulse <= MID.weight + 0.01, "pulse {pulse}");
        assert!(pulse >= HIGH.weight * 0.6, "pulse {pulse}");
    }

    #[test]
    fn hiss_gives_no_pulse() {
        let mut noise = crate::effects::Rng::new(3);
        let hiss: Vec<f32> = (0..RATE as usize * 4)
            .map(|_| 0.01 * noise.signed())
            .collect();
        let (beats, mids, highs, pulse) = band_hits(&hiss);
        assert_eq!((beats, mids, highs), (0, 0, 0));
        assert!(pulse < 0.01, "pulse {pulse}");
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
        let secs = mono.len() as f32 / RATE as f32;
        println!("{} beats in {secs:.1}s", beats.len());
        // Beats per 10 s window, to see where they cluster.
        let windows: Vec<usize> = (0..(secs / 10.0).ceil() as usize)
            .map(|w| beats.iter().filter(|&&t| (t / 10.0) as usize == w).count())
            .collect();
        println!("per 10 s: {windows:?}");
    }
}
