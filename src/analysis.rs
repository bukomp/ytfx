//! Audio analysis: FFTs over the most recently heard audio, and onset detection.
//!
//! The spectrum up to 12 kHz is cut into 50 Hz slices, each with its own sensitivity
//! (0 = off). Every slice that is on watches for onsets in its own magnitude, on its
//! own: a hit in any of them is a beat. It drives the effects fully and fires the
//! one-shot ones (glitch, invert). By default only 0-150 Hz (kicks) is on. The steady
//! part of the effects follows the auto-gain level of the slices that are on. The
//! three bands (bass below 250 Hz, mid below 4 kHz, high above) only group the slices
//! in the menu.
//!
//! The analysis steps every HOP samples of audio (about 94 times a second), not once
//! per drawn frame, so it behaves the same at any frame rate and when frames are late.

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::ops::RangeInclusive;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::effects::Drive;

/// FFT size for most slices (43 ms at 48 kHz, 23 Hz bins), and the longer one for the
/// slices below LONG_TOP: there a 50 Hz slice is only two short bins wide, and a held
/// sub bass smears into it, so a stomp under a loud sub hardly shows.
const FFT_LEN: usize = 2048;
const FFT_LONG: usize = 4096;
const LONG_TOP: u32 = 100;
/// Audio samples between two analysis steps.
const HOP: usize = 512;
pub const BANDS: usize = 24;
/// Fastest allowed beat rate (~330 BPM), also per slice, to avoid double triggers.
const MIN_BEAT_GAP: f32 = 0.18;
/// How fast the beat pulse fades (1/s).
const PULSE_DECAY: f32 = 7.0;
/// The auto-gain also never drops below this share of the loudest energy of the last
/// ~LONG_DECAY seconds, so quiet passages of a loud track stay quiet.
const LONG_SHARE: f32 = 0.35;
const LONG_DECAY: f32 = 30.0;
/// Time constant (s) of the auto-gain peaks. Slow, so the quiet stretch after a big
/// accent isn't scaled back up to full level within a few seconds.
const PEAK_DECAY: f32 = 15.0;

// A hit is an onset in one slice: how far it rose (in log terms, so a quiet slice
// counts like a loud one) over the quietest of its last RISE_HOPS steps (about the FFT
// window, so a hit counts in full wherever it falls between two steps) jumps above its
// recent normal: more than FLUX_K standard deviations over the mean of the last
// FLUX_WINDOW, and at least FLUX_MIN. Held or swelling sounds (orchestral low strings,
// long 808s, sung vowels) change too slowly to trigger, and a hit still shows when the
// slice never gets quiet between hits (a dense sub bass under the kicks). The slice
// must also be at least LEVEL_MIN of its own recent peak, so tiny onsets in a slice
// that is nearly silent don't count. Nothing fires before WARM_UP of history.
const RISE_HOPS: usize = 4;
const FLUX_K: f32 = 2.0;
const FLUX_MIN: f32 = 1.5;
const FLUX_WINDOW: f32 = 1.0;
const WARM_UP: f32 = 0.5;
const LEVEL_MIN: f32 = 0.4;
/// Slice magnitudes below this count as this, so hiss in empty slices has no flux.
const FLUX_FLOOR: f32 = 1e-3;
/// Nor does a slice far below its own recent peak (a leak at 0.002 doubling).
const FLUX_FLOOR_SHARE: f32 = 0.05;

/// Range of the per-slice sensitivity. 1.0 uses the thresholds above, 0 leaves the
/// slice out, higher values let softer hits through.
pub const SENS_MAX: f32 = 2.0;
pub const SENS_STEP: f32 = 0.1;

/// Width of one frequency slice, and how many there are (0 Hz to 12 kHz).
pub const SLICE_HZ: u32 = 50;
pub const SLICES: usize = 240;
/// Where the bass band ends and the high band starts.
const BASS_TOP: u32 = 250;
const MID_TOP: u32 = 4_000;

/// The hit thresholds for a sensitivity: (FLUX_K, FLUX_MIN, LEVEL_MIN) at 1.0, all
/// lower when more sensitive.
fn thresholds(sens: f32) -> (f32, f32, f32) {
    let s = sens.clamp(0.05, SENS_MAX);
    (FLUX_K / s, FLUX_MIN / s, (LEVEL_MIN / s).min(0.9))
}

/// The bands the menu groups the slices in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Band {
    Bass,
    Mid,
    High,
}

impl Band {
    pub const ALL: [Band; 3] = [Band::Bass, Band::Mid, Band::High];

    pub fn label(self) -> &'static str {
        match self {
            Band::Bass => "bass",
            Band::Mid => "mid",
            Band::High => "high",
        }
    }

    /// The slices (indices) this band owns.
    pub fn slices(self) -> std::ops::Range<usize> {
        let at = |hz: u32| (hz / SLICE_HZ) as usize;
        match self {
            Band::Bass => 0..at(BASS_TOP),
            Band::Mid => at(BASS_TOP)..at(MID_TOP),
            Band::High => at(MID_TOP)..SLICES,
        }
    }

    pub fn of_slice(i: usize) -> Band {
        Band::ALL
            .into_iter()
            .find(|b| b.slices().contains(&i))
            .unwrap_or(Band::High)
    }

    /// Floor for the level's auto-gain so near-silence isn't amplified: real kicks
    /// in normally mastered music reach 0.2-0.8, higher sounds are quieter.
    fn peak_floor(self) -> f32 {
        match self {
            Band::Bass => 0.08,
            Band::Mid => 0.04,
            Band::High => 0.025,
        }
    }

    /// The same for one slice's magnitude (a full-scale sine reads 1.0), so a slice
    /// that only gets hiss or the spill of a louder sound elsewhere (a snare's attack
    /// reads up to 0.025 in the bass slices; kicks reach 0.2-0.8) isn't scaled up into
    /// hits.
    fn slice_peak_floor(self) -> f32 {
        match self {
            Band::Bass => 0.08,
            Band::Mid => 0.005,
            Band::High => 0.002,
        }
    }
}

/// Lowest frequency (Hz) of slice `i`.
pub fn slice_hz(i: usize) -> u32 {
    i as u32 * SLICE_HZ
}

/// Below this the slices are on by default: kicks. Above it held bass lines start
/// (150-250 Hz), and every mid or high slice that is on fires beats too, so those
/// start off.
const DEFAULT_TOP: u32 = 150;

/// A slice's default sensitivity: 1.0 for the kick slices, else off.
pub fn default_slice_sensitivity(i: usize) -> f32 {
    if slice_hz(i) < DEFAULT_TOP { 1.0 } else { 0.0 }
}

/// The auto-gain level of the slices that are on, for the steady part of the effects.
struct Level {
    peak_floor: f32,
    /// Average two steps of energy when bass is on: a 43 ms window holds just a few
    /// bass cycles, so its energy jitters.
    smooth: bool,
    /// Unsmoothed energy of the previous frame.
    prev_energy: f32,
    peak: f32,
    long_peak: f32,
    level: f32,
}

impl Level {
    fn new() -> Self {
        Level {
            peak_floor: Band::High.peak_floor(),
            smooth: false,
            prev_energy: 0.0,
            peak: 0.0,
            long_peak: 0.0,
            level: 0.0,
        }
    }

    fn reset(&mut self) {
        *self = Level {
            peak_floor: self.peak_floor,
            smooth: self.smooth,
            ..Level::new()
        };
    }

    /// Feed one frame's raw energy.
    fn step(&mut self, raw: f32, dt: f32) {
        let energy = if self.smooth {
            0.5 * (raw + self.prev_energy)
        } else {
            raw
        };
        self.prev_energy = raw;
        self.long_peak = (self.long_peak * (-dt / LONG_DECAY).exp()).max(energy);
        self.peak = (self.peak * (-dt / PEAK_DECAY).exp())
            .max(energy)
            .max(self.peak_floor)
            .max(LONG_SHARE * self.long_peak);
        self.level = (energy / self.peak).clamp(0.0, 1.0);
    }
}

/// Onset state of one slice.
#[derive(Default)]
struct Onset {
    /// Recent (age in seconds, rise), for the threshold.
    recent: VecDeque<(f32, f32)>,
    since_hit: f32,
}

impl Onset {
    /// Feed one hop's rise. Returns the hit strength (0..1) on a hit.
    fn step(&mut self, rise: f32, sens: f32, level: f32, dt: f32) -> Option<f32> {
        // The rises of the last FLUX_WINDOW, before this hop.
        for e in &mut self.recent {
            e.0 += dt;
        }
        while self.recent.front().is_some_and(|e| e.0 > FLUX_WINDOW) {
            self.recent.pop_front();
        }
        let n = self.recent.len() as f32;
        let mean = self.recent.iter().map(|e| e.1).sum::<f32>() / n.max(1.0);
        let var = self
            .recent
            .iter()
            .map(|e| (e.1 - mean).powi(2))
            .sum::<f32>()
            / n.max(1.0);
        let warm = self.recent.front().is_some_and(|e| e.0 >= WARM_UP);
        self.recent.push_back((0.0, rise));

        self.since_hit += dt;
        let (k, min_rise, min_level) = thresholds(sens);
        let threshold = (mean + k * var.sqrt()).max(min_rise);
        let hit = warm && rise > threshold && level > min_level && self.since_hit > MIN_BEAT_GAP;
        if !hit {
            return None;
        }
        self.since_hit = 0.0;
        Some((0.25 * (rise / threshold - 1.0) + 0.5 * level).clamp(0.0, 1.0))
    }
}

/// One 50 Hz slice: its sensitivity, magnitude, auto-gain peak and onsets.
struct Slice {
    sens: f32,
    peak_floor: f32,
    /// Magnitude in this hop, and in the RISE_HOPS before it (oldest first).
    mag: f32,
    before: VecDeque<f32>,
    /// Its recent peak magnitude.
    peak: f32,
    onset: Onset,
    /// How lit its last hit still is, for the menu.
    glow: f32,
}

impl Slice {
    fn new(i: usize) -> Self {
        let peak_floor = Band::of_slice(i).slice_peak_floor();
        Slice {
            sens: default_slice_sensitivity(i),
            peak_floor,
            mag: 0.0,
            before: VecDeque::with_capacity(RISE_HOPS + 1),
            peak: peak_floor,
            onset: Onset::default(),
            glow: 0.0,
        }
    }

    /// Back to silence, keeping the sensitivity.
    fn reset(&mut self) {
        *self = Slice {
            sens: self.sens,
            peak_floor: self.peak_floor,
            peak: self.peak_floor,
            ..Slice::new(0)
        };
    }

    /// Feed this hop's magnitude; returns the hit strength on an onset.
    fn step(&mut self, mag: f32, dt: f32) -> Option<f32> {
        self.before.push_back(self.mag);
        if self.before.len() > RISE_HOPS {
            self.before.pop_front();
        }
        self.mag = mag;
        self.glow *= (-dt * PULSE_DECAY).exp();
        self.peak = (self.peak * (-dt / PEAK_DECAY).exp())
            .max(mag)
            .max(self.peak_floor);
        if self.sens <= 0.0 {
            self.onset.recent.clear();
            return None;
        }
        // How far it rose (in log terms) over the quietest of the last RISE_HOPS.
        let floor = FLUX_FLOOR.max(FLUX_FLOOR_SHARE * self.peak);
        let low = self.before.iter().copied().fold(f32::MAX, f32::min);
        let rise = (mag.max(floor).ln() - low.max(floor).ln()).max(0.0);
        let level = (mag / self.peak).clamp(0.0, 1.0);
        let strength = self.onset.step(rise, self.sens, level, dt)?;
        self.glow = self.glow.max(0.65 + 0.35 * strength);
        Some(strength)
    }
}

/// A Hann-windowed FFT of the latest `len` samples.
struct Spectrum {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex<f32>>,
    bin_hz: f32,
}

impl Spectrum {
    fn new(len: usize, rate: f32) -> Self {
        Spectrum {
            fft: FftPlanner::new().plan_fft_forward(len),
            window: (0..len)
                .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (len - 1) as f32).cos())
                .collect(),
            buf: vec![Complex::default(); len],
            bin_hz: rate / len as f32,
        }
    }

    /// Transform the last samples of `samples` (zero-padded in front if short).
    fn run(&mut self, samples: &[f32]) {
        let len = self.buf.len();
        let tail = &samples[samples.len().saturating_sub(len)..];
        let pad = len - tail.len();
        for (i, c) in self.buf.iter_mut().enumerate() {
            let s = if i < pad { 0.0 } else { tail[i - pad] };
            *c = Complex::new(s * self.window[i], 0.0);
        }
        self.fft.process(&mut self.buf);
    }

    /// Magnitude of bin `k`, scaled so a full-scale sine reads ~1.0 (the Hann
    /// window halves the amplitude).
    fn mag(&self, k: usize) -> f32 {
        self.buf[k].norm() * 4.0 / self.buf.len() as f32
    }

    /// The bins (1..len/2) with the slice each falls in.
    fn bins(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let bin_hz = self.bin_hz;
        (1..self.buf.len() / 2).map(move |k| (k, (k as f32 * bin_hz / SLICE_HZ as f32) as usize))
    }
}

pub struct Analyzer {
    rate: f32,
    /// FFT_LEN for most slices, FFT_LONG for the ones below LONG_TOP.
    short: Spectrum,
    long: Spectrum,
    slices: Vec<Slice>,
    level: Level,
    /// The beat pulse and the time since the last beat.
    pulse: f32,
    since_beat: f32,
    /// Samples to go until the next hop.
    to_hop: usize,
    /// Level of each slice for the menu, 0..1, like `bands`.
    pub slice_levels: [f32; SLICES],
    /// Spectrum for the HUD, 0..1 per log-spaced band.
    pub bands: [f32; BANDS],
}

impl Analyzer {
    pub fn new(rate: u32) -> Self {
        let rate = rate as f32;
        let mut a = Analyzer {
            rate,
            short: Spectrum::new(FFT_LEN, rate),
            long: Spectrum::new(FFT_LONG, rate),
            slices: (0..SLICES).map(Slice::new).collect(),
            level: Level::new(),
            pulse: 0.0,
            since_beat: 0.0,
            to_hop: HOP,
            slice_levels: [0.0; SLICES],
            bands: [0.0; BANDS],
        };
        a.rebuild();
        a
    }

    /// Recompute the level's gain floor from the slices that are on.
    fn rebuild(&mut self) {
        let lowest = (0..SLICES).find(|&i| self.slices[i].sens > 0.0);
        let band = lowest.map_or(Band::High, Band::of_slice);
        self.level.peak_floor = band.peak_floor();
        self.level.smooth = band == Band::Bass;
    }

    /// How lit the last hit of slice `i` still is, 0..1.
    pub fn slice_glow(&self, i: usize) -> f32 {
        self.slices.get(i).map_or(0.0, |s| s.glow.clamp(0.0, 1.0))
    }

    /// The beat pulse, 0..1: jumps on a beat and fades.
    pub fn pulse(&self) -> f32 {
        self.pulse.clamp(0.0, 1.0)
    }

    pub fn slice_sensitivity(&self, i: usize) -> f32 {
        self.slices.get(i).map_or(0.0, |s| s.sens)
    }

    /// Set a slice's sensitivity, clamped to 0..=SENS_MAX and rounded to SENS_STEP.
    /// Returns whether it changed.
    pub fn set_slice_sensitivity(&mut self, i: usize, value: f32) -> bool {
        let v = if value.is_finite() {
            (value.clamp(0.0, SENS_MAX) / SENS_STEP).round() * SENS_STEP
        } else {
            default_slice_sensitivity(i)
        };
        match self.slices.get_mut(i) {
            Some(s) if s.sens != v => s.sens = v,
            _ => return false,
        }
        self.rebuild();
        true
    }

    /// Move every slice in `slices` by `steps` SENS_STEPs, down to off or up to
    /// SENS_MAX. Returns whether any changed.
    pub fn nudge_slices(&mut self, slices: RangeInclusive<usize>, steps: f32) -> bool {
        let mut changed = false;
        for i in slices {
            let v = self.slice_sensitivity(i) + steps * SENS_STEP;
            changed |= self.set_slice_sensitivity(i, v);
        }
        changed
    }

    /// Set every slice in `slices` to `value(i)`. Returns whether any changed.
    pub fn set_slices(
        &mut self,
        slices: RangeInclusive<usize>,
        value: impl Fn(usize) -> f32,
    ) -> bool {
        let mut changed = false;
        for i in slices {
            changed |= self.set_slice_sensitivity(i, value(i));
        }
        changed
    }

    /// Back to the default slices.
    pub fn reset_sensitivity(&mut self) {
        for (i, s) in self.slices.iter_mut().enumerate() {
            s.sens = default_slice_sensitivity(i);
        }
        self.rebuild();
    }

    pub fn reset(&mut self) {
        self.level.reset();
        self.slices.iter_mut().for_each(Slice::reset);
        self.pulse = 0.0;
        self.to_hop = HOP;
        self.bands = [0.0; BANDS];
        self.slice_levels = [0.0; SLICES];
    }

    /// Called while paused: let everything fade out.
    pub fn idle(&mut self, dt: f32) -> Drive {
        self.level.level *= (-dt * 4.0).exp();
        self.pulse *= (-dt * PULSE_DECAY).exp();
        for s in &mut self.slices {
            s.glow *= (-dt * PULSE_DECAY).exp();
        }
        for b in self.bands.iter_mut().chain(&mut self.slice_levels) {
            *b = (*b - dt * 1.5).max(0.0);
        }
        Drive {
            pulse: self.pulse,
            level: self.level.level,
            beat: false,
            strength: 0.0,
        }
    }

    /// Feed the latest heard audio: `samples` ends with the newest one, and its last
    /// `new` samples arrived since the previous call. The analysis runs once per HOP
    /// samples of audio, however often this is called.
    pub fn update(&mut self, samples: &[f32], new: usize) -> Drive {
        let new = new.min(samples.len());
        let mut end = samples.len() - new;
        let mut left = new;
        let mut beat: Option<f32> = None;
        let mut ran = false;
        while self.to_hop <= left {
            end += self.to_hop;
            left -= self.to_hop;
            self.to_hop = HOP;
            ran = true;
            if let Some(s) = self.hop(&samples[..end]) {
                beat = Some(beat.map_or(s, |b: f32| b.max(s)));
            }
        }
        self.to_hop -= left;
        if ran {
            self.update_bands(new as f32 / self.rate);
        }
        Drive {
            pulse: self.pulse,
            level: self.level.level,
            beat: beat.is_some(),
            strength: beat.unwrap_or(0.0),
        }
    }

    /// One analysis step on the audio ending at the end of `samples`. Returns the
    /// beat strength on a beat.
    fn hop(&mut self, samples: &[f32]) -> Option<f32> {
        let dt = HOP as f32 / self.rate;
        self.short.run(samples);
        self.long.run(samples);
        let long_slices = (LONG_TOP / SLICE_HZ) as usize;
        // Slices above Nyquist (low sample rates) have no bins and read silence.
        let mut mags = [0.0f32; SLICES];
        for (spectrum, keep) in [
            (&self.short, long_slices..SLICES),
            (&self.long, 0..long_slices),
        ] {
            for (k, slice) in spectrum.bins() {
                if keep.contains(&slice) {
                    mags[slice] += spectrum.mag(k).powi(2);
                }
            }
        }
        let mags = mags.map(f32::sqrt);

        // The level: the slices that are on, each weighted by its sensitivity.
        let top = self.slices.iter().map(|s| s.sens).fold(0.0f32, f32::max);
        let energy = if top > 0.0 {
            self.slices
                .iter()
                .zip(&mags)
                .map(|(s, m)| (m * s.sens / top).powi(2))
                .sum::<f32>()
                .sqrt()
        } else {
            0.0
        };
        self.level.step(energy, dt);

        // A hit in any slice that is on is a beat (one per MIN_BEAT_GAP).
        let mut best: Option<f32> = None;
        for (s, m) in self.slices.iter_mut().zip(mags) {
            if let Some(strength) = s.step(m, dt) {
                best = Some(best.map_or(strength, |b| b.max(strength)));
            }
        }
        self.since_beat += dt;
        self.pulse *= (-dt * PULSE_DECAY).exp();
        let beat = best.filter(|_| self.since_beat > MIN_BEAT_GAP)?;
        self.since_beat = 0.0;
        self.pulse = self.pulse.max(0.65 + 0.35 * beat);
        Some(beat)
    }

    /// The menu's slice meters (what the detection reads) and the HUD's spectrum.
    fn update_bands(&mut self, dt: f32) {
        let db = |m: f32| ((20.0 * (m + 1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
        for (level, s) in self.slice_levels.iter_mut().zip(&self.slices) {
            *level = db(s.mag).max(*level - dt * 1.5);
        }

        let spectrum = &self.short;
        let bin_hz = spectrum.bin_hz;
        let f_lo = 30.0f32;
        let f_hi = (self.rate * 0.5).min(16_000.0);
        let ratio = f_hi / f_lo;
        let max_bin = FFT_LEN / 2;
        for (b, band) in self.bands.iter_mut().enumerate() {
            let a = f_lo * ratio.powf(b as f32 / BANDS as f32);
            let z = f_lo * ratio.powf((b + 1) as f32 / BANDS as f32);
            let k0 = ((a / bin_hz) as usize).clamp(1, max_bin - 1);
            let k1 = ((z / bin_hz) as usize).clamp(k0 + 1, max_bin);
            let m = (k0..k1).map(|k| spectrum.mag(k)).fold(0.0f32, f32::max);
            *band = db(m).max(*band - dt * 1.5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const FPS: usize = 30;
    /// How much audio each call sees, like the app's tap.
    const TAP: usize = 8_192;

    /// Feed a mono signal the way the app does: one snapshot of the tap per render tick.
    fn beat_times(signal: &[f32]) -> Vec<f32> {
        let mut a = Analyzer::new(RATE);
        let step = RATE as usize / FPS;
        let mut beats = Vec::new();
        let mut end = step;
        while end <= signal.len() {
            let start = end.saturating_sub(TAP);
            if a.update(&signal[start..end], step).beat {
                beats.push(end as f32 / RATE as f32);
            }
            end += step;
        }
        beats
    }

    /// Beat times (s) of `signal` through `a`, fed like `beat_times`.
    fn beats_in(a: &mut Analyzer, signal: &[f32]) -> Vec<f32> {
        let step = RATE as usize / FPS;
        let mut beats = Vec::new();
        let mut end = step;
        while end <= signal.len() {
            if a.update(&signal[end.saturating_sub(TAP)..end], step).beat {
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
    fn beats_do_not_depend_on_the_frame_rate() {
        // The same kicks, fed at a steady 30 or 60 fps and at uneven frame times
        // (late frames, as when drawing is slow), give the same beats.
        let signal = kicks(8.0);
        let run = |steps: &mut dyn FnMut() -> usize| {
            let mut a = Analyzer::new(RATE);
            let (mut end, mut last, mut beats) = (0, 0, Vec::new());
            loop {
                end += steps();
                if end > signal.len() {
                    return beats;
                }
                if a.update(&signal[end.saturating_sub(TAP)..end], end - last)
                    .beat
                {
                    beats.push(end as f32 / RATE as f32);
                }
                last = end;
            }
        };
        let steady = run(&mut || 1_600);
        let fast = run(&mut || 800);
        let mut rng = crate::effects::Rng::new(9);
        let uneven = run(&mut || 500 + (rng.signed().abs() * 4_000.0) as usize);
        assert!(steady.len() >= 12, "beats: {steady:?}");
        for other in [&fast, &uneven] {
            assert_eq!(other.len(), steady.len(), "{steady:?} vs {other:?}");
            for (a, b) in steady.iter().zip(other) {
                // Reported at the frame that saw it: within one (uneven) frame.
                assert!((a - b).abs() < 0.11, "{steady:?} vs {other:?}");
            }
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
        line_of(&[55.0, 65.4, 49.0, 73.4], secs)
    }

    /// A held line through `notes` (1.3 s each), with tremolo and slow swells.
    fn line_of(notes: &[f32], secs: f32) -> Vec<f32> {
        let mut phase = 0.0f32;
        (0..(RATE as f32 * secs) as usize)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let freq = notes[(t / 1.3) as usize % notes.len()];
                phase += 2.0 * PI * freq / RATE as f32;
                let swell = 0.35 + 0.15 * (2.0 * PI * 0.25 * t).sin();
                let tremolo = 1.0 + 0.3 * (2.0 * PI * 7.0 * t).sin();
                let fade_in = (t / 0.8).min(1.0);
                swell * tremolo * fade_in * phase.sin()
            })
            .collect()
    }

    #[test]
    fn held_notes_are_not_beats() {
        // Swells and tremolo never count: one note held, every bass slice on. Only
        // its entry from silence may.
        let held = line_of(&[73.4], 10.0);
        let mut a = Analyzer::new(RATE);
        a.set_slices(0..=4, |_| 1.0);
        let beats = beats_in(&mut a, &held);
        assert!(beats.iter().all(|&t| t < 1.0), "beats: {beats:?}");
        // Notes that stay in the one slice that is on (50-100 Hz) don't count either.
        let line = line_of(&[65.4, 73.4, 82.4, 77.8], 10.0);
        let (beats, _) = beats_with(&line, |i| if i == 1 { 1.0 } else { 0.0 });
        assert!(beats <= 1, "beats: {beats}");
    }

    #[test]
    fn notes_moving_between_slices_are_hits_there() {
        // Slices fire on their own: a note that moves into a slice that is on is a
        // real rise there (55 -> 65 -> 49 -> 73 Hz crosses 50 Hz on each change).
        let (beats, _) = beats_with(&bass_line(10.0), default_slice_sensitivity);
        assert!((3..=8).contains(&beats), "beats: {beats}");
        // With only the slice it leaves on, it doesn't fire.
        let (beats, _) = beats_with(&line_of(&[73.4, 49.0], 10.0), |i| {
            if i == 1 { 1.0 } else { 0.0 }
        });
        assert!(beats <= 1, "beats: {beats}");
    }

    /// Kicks at 160 BPM over a loud sub bass that never dips between them, so the
    /// band level stays high all the time (the case of dense, bass-heavy mixes).
    #[test]
    fn kicks_over_a_sustained_sub_still_hit() {
        let period = 0.375;
        let secs = 8.0;
        let mut phase = 0.0f32;
        let signal: Vec<f32> = (0..(RATE as f32 * secs) as usize)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                phase += 2.0 * PI * 42.0 / RATE as f32;
                let sub = (0.55 + 0.1 * (2.0 * PI * 0.5 * t).sin()) * phase.sin();
                let since = t % period;
                let kick = (2.0 * PI * (60.0 + 90.0 * (-since * 30.0).exp()) * since).sin()
                    * (-since * 14.0).exp();
                0.6 * sub + 0.7 * kick
            })
            .collect();
        let beats = beat_times(&signal);
        // The first second is spent filling the threshold history.
        let kicks = ((secs - 1.0) / period) as usize;
        let hit = beats.iter().filter(|&&t| t > 1.0).count();
        assert!(hit + 2 >= kicks, "{hit} of {kicks}: {beats:?}");
        for pair in beats.windows(2) {
            let gap = pair[1] - pair[0];
            let n = (gap / period).round();
            assert!(
                n >= 1.0 && (gap - n * period).abs() < 0.05,
                "off-beat hit: {beats:?}"
            );
        }
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

    /// Feed a signal like `beat_times` with slice `i` at `sens(i)`; returns (beats,
    /// max pulse).
    fn beats_with(signal: &[f32], sens: impl Fn(usize) -> f32) -> (usize, f32) {
        let mut a = Analyzer::new(RATE);
        a.set_slices(0..=SLICES - 1, sens);
        let step = RATE as usize / FPS;
        let (mut beats, mut max_pulse) = (0, 0.0f32);
        let mut end = step;
        while end <= signal.len() {
            let start = end.saturating_sub(TAP);
            let d = a.update(&signal[start..end], step);
            beats += d.beat as usize;
            max_pulse = max_pulse.max(d.pulse);
            end += step;
        }
        (beats, max_pulse)
    }

    /// 1.0 for the slices of `band`, else off.
    fn only(band: Band) -> impl Fn(usize) -> f32 {
        move |i| if band.slices().contains(&i) { 1.0 } else { 0.0 }
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
    fn snares_fire_beats_when_mid_is_on() {
        let snares = bursts(6.0, 25.0, false);
        let (beats, _) = beats_with(&snares, default_slice_sensitivity);
        assert!(beats <= 1, "snares were beats with only bass on: {beats}");
        let (beats, pulse) = beats_with(&snares, only(Band::Mid));
        assert!(beats >= 9, "beats: {beats}");
        assert!(pulse >= 0.65, "pulse {pulse}");
    }

    #[test]
    fn hats_fire_beats_when_high_is_on() {
        let hats = bursts(6.0, 60.0, true);
        assert_eq!(beats_with(&hats, default_slice_sensitivity).0, 0);
        let (beats, _) = beats_with(&hats, only(Band::High));
        assert!(beats >= 9, "beats: {beats}");
    }

    #[test]
    fn one_slice_fires_on_its_own() {
        // A 1 kHz blip every 0.5 s: only the 1000-1050 Hz slice is on.
        let blips: Vec<f32> = (0..RATE as usize * 6)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let since = t % 0.5;
                0.5 * (2.0 * PI * 1_025.0 * t).sin() * (-since * 30.0).exp()
            })
            .collect();
        let (beats, _) = beats_with(&blips, |i| if i == 20 { 1.0 } else { 0.0 });
        assert!(beats >= 9, "beats: {beats}");
        let (beats, _) = beats_with(&blips, |i| if i == 60 { 1.0 } else { 0.0 });
        assert_eq!(beats, 0, "a slice elsewhere fired");

        // Only the slice that hit lights up in the menu.
        let mut a = Analyzer::new(RATE);
        a.set_slices(
            0..=SLICES - 1,
            |i| if i == 20 || i == 60 { 1.0 } else { 0.0 },
        );
        let mut end = RATE as usize / FPS;
        let mut lit = false;
        while end <= blips.len() {
            let d = a.update(&blips[end.saturating_sub(TAP)..end], RATE as usize / FPS);
            if d.beat {
                lit = true;
                assert!(a.slice_glow(20) >= 0.65, "glow {}", a.slice_glow(20));
                assert_eq!(a.slice_glow(60), 0.0);
                assert_eq!(a.slice_glow(19), 0.0);
            }
            end += RATE as usize / FPS;
        }
        assert!(lit);
    }

    #[test]
    fn hiss_gives_no_pulse() {
        let mut noise = crate::effects::Rng::new(3);
        let hiss: Vec<f32> = (0..RATE as usize * 4)
            .map(|_| 0.01 * noise.signed())
            .collect();
        for sens in [1.0, SENS_MAX] {
            let (beats, pulse) = beats_with(&hiss, |_| sens);
            assert_eq!(beats, 0, "at {sens}");
            assert!(pulse < 0.01, "pulse {pulse}");
        }
    }

    /// Set the slices of `band` that are on by default to `sens`.
    fn set_band(a: &mut Analyzer, band: Band, sens: f32) {
        for i in band.slices() {
            if default_slice_sensitivity(i) > 0.0 {
                a.set_slice_sensitivity(i, sens);
            }
        }
    }

    /// Bass beats in `signal` with the bass band at `sens`.
    fn beats_at(signal: &[f32], sens: f32) -> usize {
        let mut a = Analyzer::new(RATE);
        set_band(&mut a, Band::Bass, sens);
        count_beats(&mut a, signal)
    }

    fn count_beats(a: &mut Analyzer, signal: &[f32]) -> usize {
        let step = RATE as usize / FPS;
        let mut beats = 0;
        let mut end = step;
        while end <= signal.len() {
            let start = end.saturating_sub(TAP);
            beats += a.update(&signal[start..end], step).beat as usize;
            end += step;
        }
        beats
    }

    /// Soft kicks (slow attack, long tail) over a bass line: borderline at 1.0.
    fn soft_kicks(secs: f32) -> Vec<f32> {
        bass_line(secs)
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let since = (i as f32 / RATE as f32) % 0.5;
                let env = (since * 40.0).min(1.0) * (-since * 6.0).exp();
                0.6 * b + 0.35 * env * (2.0 * PI * 55.0 * since).sin()
            })
            .collect()
    }

    #[test]
    fn sensitivity_scales_hits() {
        let signal = soft_kicks(8.0);
        let (low, mid, high) = (
            beats_at(&signal, 0.3),
            beats_at(&signal, 1.0),
            beats_at(&signal, SENS_MAX),
        );
        assert!(low <= mid && mid <= high, "{low} {mid} {high}");
        assert!(
            high > low,
            "sensitivity changed nothing: {low} {mid} {high}"
        );
        assert_eq!(beats_at(&signal, 0.0), 0);
    }

    #[test]
    fn full_sensitivity_still_ignores_silence() {
        assert_eq!(beats_at(&vec![0.0; RATE as usize * 4], SENS_MAX), 0);
    }

    #[test]
    fn sensitivity_is_clamped_and_kept_on_reset() {
        let mut a = Analyzer::new(RATE);
        assert!(a.set_slice_sensitivity(10, 9.0));
        // Off by default, so -1 (clamped to off) changes nothing.
        assert!(!a.set_slice_sensitivity(100, -1.0));
        assert!(!a.set_slice_sensitivity(1, f32::NAN));
        assert!(!a.set_slice_sensitivity(SLICES, 1.0));
        a.reset();
        assert_eq!(a.slice_sensitivity(10), SENS_MAX);
        assert_eq!(a.slice_sensitivity(100), 0.0);
        assert_eq!(a.slice_sensitivity(1), 1.0);
        assert_eq!(thresholds(1.0), (FLUX_K, FLUX_MIN, LEVEL_MIN));
    }

    #[test]
    fn default_slices_match_the_bands() {
        let on = |hz: u32| default_slice_sensitivity((hz / SLICE_HZ) as usize) > 0.0;
        for hz in [0, 50, 100] {
            assert!(on(hz), "{hz} Hz should be on");
        }
        for hz in [150, 200, 250, 1_000, 4_000, 11_950] {
            assert!(!on(hz), "{hz} Hz should be off");
        }
        assert_eq!(Band::Bass.slices(), 0..5);
        assert_eq!(Band::High.slices().end, SLICES);
    }

    /// 55 Hz kicks every 0.5 s over a little noise.
    fn kicks(secs: f32) -> Vec<f32> {
        let mut noise = crate::effects::Rng::new(7);
        (0..(RATE as f32 * secs) as usize)
            .map(|i| {
                let since = (i as f32 / RATE as f32) % 0.5;
                let kick = (2.0 * PI * 55.0 * since).sin() * (-since * 18.0).exp();
                0.8 * kick + 0.05 * noise.signed()
            })
            .collect()
    }

    #[test]
    fn kicks_need_their_slices() {
        let signal = kicks(8.0);
        let mut a = Analyzer::new(RATE);
        for i in Band::Bass.slices() {
            a.set_slice_sensitivity(i, 0.0);
        }
        assert_eq!(count_beats(&mut a, &signal), 0);

        // 50-100 Hz alone still catches a 55 Hz kick.
        let mut a = Analyzer::new(RATE);
        for i in Band::Bass.slices() {
            a.set_slice_sensitivity(i, if i == 1 { 1.0 } else { 0.0 });
        }
        let beats = count_beats(&mut a, &signal);
        assert!(beats >= 10, "beats: {beats}");
    }

    #[test]
    fn nudge_moves_a_range_down_to_off() {
        let mut a = Analyzer::new(RATE);
        // Bass at 1.0 except the off slices 3 and 4.
        assert!(a.nudge_slices(0..=4, 2.0));
        let bass = |a: &Analyzer| -> Vec<f32> {
            Band::Bass
                .slices()
                .map(|i| (a.slice_sensitivity(i) * 10.0).round() / 10.0)
                .collect()
        };
        assert_eq!(bass(&a), [1.2, 1.2, 1.2, 0.2, 0.2]);
        for _ in 0..30 {
            a.nudge_slices(0..=4, -1.0);
        }
        assert_eq!(bass(&a), [0.0; 5]);
        assert!(!a.nudge_slices(0..=4, -1.0));
        // Past the end is ignored.
        assert!(a.set_slices(SLICES - 1..=SLICES + 3, |_| 0.5));
        assert_eq!(a.slice_sensitivity(SLICES - 1), 0.5);
        a.reset_sensitivity();
        assert_eq!(a.slice_sensitivity(1), 1.0);
    }

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
        let (all, _) = beats_with(&mono, |_| 1.0);
        println!("{all} beats with every slice on");
        // Beats per 10 s window, to see where they cluster.
        let windows: Vec<usize> = (0..(secs / 10.0).ceil() as usize)
            .map(|w| beats.iter().filter(|&&t| (t / 10.0) as usize == w).count())
            .collect();
        println!("per 10 s: {windows:?}");
    }
}
