//! Audio output. Decoded PCM is pushed into a shared queue, the device callback plays
//! it and copies every played sample into a "tap" buffer that the analyzer reads.
//! Because we are the ones producing the sound, no system loopback capture is needed.
//!
//! Some outputs (Bluetooth headphones above all) play a sample well after the callback
//! hands it over. The output delay setting makes the clock and the tap lag by that
//! much, so the video and the effects line up with what is heard.
//!
//! A solo range plays only the frequencies in it (band-passed after the tap, so the
//! analyzer still hears everything).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

/// How many mono samples of recently heard audio the analyzer can see: its longest
/// FFT plus what arrives between two drawn frames, even slow ones (~250 ms).
const TAP_LEN: usize = 16_384;
/// Longest output delay that can be set, in milliseconds.
pub const MAX_DELAY_MS: u32 = 1_000;

/// The sync test's metronome: a click every BEAT_SECS, every ACCENT_EVERY-th one
/// higher, so a flash can be matched to its click even with a delay longer than a beat.
pub const BEAT_SECS: f64 = 0.6;
pub const ACCENT_EVERY: u64 = 4;
const CLICK_SECS: f64 = 0.04;

/// Where frame `n` of the metronome is: (beat number, seconds since that beat).
pub fn metronome_beat(n: u64, rate: u32) -> (u64, f64) {
    let t = n as f64 / f64::from(rate);
    let beat = (t / BEAT_SECS) as u64;
    (beat, t - beat as f64 * BEAT_SECS)
}

/// Sample `n` of the metronome: a short decaying tone at each beat, else silence.
fn metronome(n: u64, rate: u32) -> f32 {
    let (beat, since) = metronome_beat(n, rate);
    if since >= CLICK_SECS {
        return 0.0;
    }
    let freq = if beat.is_multiple_of(ACCENT_EVERY) {
        1_760.0
    } else {
        880.0
    };
    let env = (1.0 - since / CLICK_SECS).powi(2);
    (0.5 * env * (std::f64::consts::TAU * freq * since).sin()) as f32
}

/// How long the solo takes to fade in or out, in seconds (no click on the switch).
const SOLO_FADE: f32 = 0.03;

/// One second-order filter stage (transposed direct form II), in f64: at 50 Hz and
/// 48 kHz the poles sit very close to 1, where f32 gets noisy.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    /// A Butterworth stage at `hz` with quality `q` (RBJ cookbook).
    fn new(high_pass: bool, hz: f64, q: f64, rate: f64) -> Self {
        let w = std::f64::consts::TAU * hz / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b = if high_pass {
            [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0]
        } else {
            [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0]
        };
        Biquad {
            b: b.map(|x| x / a0),
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
        }
    }

    fn run(&self, x: f64, z: &mut [f64; 2]) -> f64 {
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x - self.a[0] * y + z[1];
        z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Stage qualities of an 8th-order Butterworth filter (48 dB per octave).
const BUTTERWORTH_8: [f64; 4] = [0.5098, 0.6013, 0.9000, 2.5629];

/// A band-pass from `lo` to `hi` Hz: a high-pass at `lo` and a low-pass at `hi`, each
/// left out when it would do nothing (0 Hz, or near Nyquist). Keeps one state per
/// channel.
#[derive(Default)]
struct BandPass {
    range: (u32, u32),
    stages: Vec<Biquad>,
    state: Vec<[f64; 2]>,
}

impl BandPass {
    fn set(&mut self, (lo, hi): (u32, u32), rate: u32, channels: usize) {
        if self.range == (lo, hi) && !self.stages.is_empty() {
            return;
        }
        self.range = (lo, hi);
        let rate = f64::from(rate);
        let mut stages = Vec::new();
        if lo > 0 {
            stages.extend(BUTTERWORTH_8.map(|q| Biquad::new(true, f64::from(lo), q, rate)));
        }
        if f64::from(hi) < 0.45 * rate {
            stages.extend(BUTTERWORTH_8.map(|q| Biquad::new(false, f64::from(hi), q, rate)));
        }
        // Keep the state when only the coefficients change: no restart click.
        if stages.len() != self.stages.len() {
            self.state = vec![[0.0; 2]; stages.len() * channels];
        }
        self.stages = stages;
    }

    /// Filter one sample of channel `ch`.
    fn run(&mut self, x: f32, ch: usize) -> f32 {
        let n = self.stages.len();
        let mut y = f64::from(x);
        for (stage, z) in self
            .stages
            .iter()
            .zip(&mut self.state[ch * n..(ch + 1) * n])
        {
            y = stage.run(y, z);
        }
        y as f32
    }
}

/// Played mono samples, newest last, and how many were ever pushed.
#[derive(Default)]
struct Tap {
    samples: VecDeque<f32>,
    total: u64,
}

#[derive(Default)]
pub struct AudioQueue {
    /// Interleaved samples waiting to be played.
    pub samples: VecDeque<f32>,
    /// The decoder hit end of stream.
    pub done: bool,
    /// Total samples the decoder delivered (0 at the end means the stream never opened).
    pub received: u64,
    /// Last error line ffmpeg printed, if the decoder failed.
    pub error: Option<String>,
}

pub struct AudioShared {
    queue: Mutex<AudioQueue>,
    /// Frames actually sent to the device since the last reset; this is the playback clock.
    played: AtomicU64,
    paused: AtomicBool,
    volume: AtomicU32,
    /// Output delay in frames: how long a played frame takes to be heard.
    delay: AtomicU64,
    /// The last TAP_LEN + `delay` played mono samples.
    tap: Mutex<Tap>,
    /// The sync test is on: the device plays the metronome instead of the queue, and
    /// this counts its frames.
    test: AtomicBool,
    test_frames: AtomicU64,
    /// The solo range in Hz, packed as lo << 32 | hi; 0 when off.
    solo: AtomicU64,
}

impl AudioShared {
    fn new() -> Self {
        AudioShared {
            queue: Mutex::new(AudioQueue::default()),
            played: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            volume: AtomicU32::new(0.8f32.to_bits()),
            delay: AtomicU64::new(0),
            tap: Mutex::new(Tap::default()),
            test: AtomicBool::new(false),
            test_frames: AtomicU64::new(0),
            solo: AtomicU64::new(0),
        }
    }

    pub fn queue(&self) -> MutexGuard<'_, AudioQueue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drop all queued audio and restart the clock. Done under the queue lock so the
    /// device callback never sees a half-reset state.
    pub fn reset(&self) {
        let mut q = self.queue();
        q.samples.clear();
        q.done = false;
        q.received = 0;
        q.error = None;
        self.played.store(0, Relaxed);
        drop(q);
        *self.tap.lock().unwrap_or_else(|e| e.into_inner()) = Tap::default();
    }

    /// Frames that have been heard: played, minus the output delay.
    pub fn heard_frames(&self) -> u64 {
        self.played
            .load(Relaxed)
            .saturating_sub(self.delay.load(Relaxed))
    }

    pub fn set_delay_frames(&self, frames: u64) {
        self.delay.store(frames, Relaxed);
    }

    /// Start (from the first click) or stop the sync test. While it runs the queue
    /// waits, so a video holds still and goes on where it was afterwards.
    pub fn set_test(&self, on: bool) {
        if on {
            self.test_frames.store(0, Relaxed);
        }
        self.test.store(on, Relaxed);
    }

    pub fn test_on(&self) -> bool {
        self.test.load(Relaxed)
    }

    /// Metronome frames that have been heard, or None before the first one has.
    pub fn heard_test_frames(&self) -> Option<u64> {
        let sent = self.test_frames.load(Relaxed);
        sent.checked_sub(self.delay.load(Relaxed))
    }

    /// Play only `lo..hi` Hz, or everything with None.
    pub fn set_solo(&self, range: Option<(u32, u32)>) {
        let packed = range
            .filter(|&(lo, hi)| hi > lo)
            .map_or(0, |(lo, hi)| u64::from(lo) << 32 | u64::from(hi));
        self.solo.store(packed, Relaxed);
    }

    pub fn solo(&self) -> Option<(u32, u32)> {
        let packed = self.solo.load(Relaxed);
        (packed != 0).then_some(((packed >> 32) as u32, packed as u32))
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Relaxed)
    }

    pub fn set_paused(&self, p: bool) {
        self.paused.store(p, Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Relaxed))
    }

    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.clamp(0.0, 2.0).to_bits(), Relaxed);
    }

    /// The last TAP_LEN samples that have been heard (see `heard_frames`). Returns
    /// how many samples have been heard in all, up to the last one in `out` (it starts
    /// over from 0 on a reset), so the caller knows how many of them are new.
    pub fn tap_snapshot(&self, out: &mut Vec<f32>) -> u64 {
        out.clear();
        let tap = self.tap.lock().unwrap_or_else(|e| e.into_inner());
        let delay = self.delay.load(Relaxed) as usize;
        let (start, end) = heard_window(tap.samples.len(), delay);
        out.extend(tap.samples.range(start..end));
        tap.total - (tap.samples.len() - end) as u64
    }
}

/// The part of a tap of `len` samples, newest last, that has been heard with `delay`
/// samples of output delay: up to TAP_LEN samples ending `delay` before the newest.
fn heard_window(len: usize, delay: usize) -> (usize, usize) {
    let end = len.saturating_sub(delay);
    (end.saturating_sub(TAP_LEN), end)
}

pub struct AudioOut {
    _stream: cpal::Stream,
    pub shared: Arc<AudioShared>,
    pub rate: u32,
    pub channels: u16,
}

impl AudioOut {
    pub fn open() -> Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| anyhow!("no audio output device"))?;
        let supported = device
            .default_output_config()
            .context("no usable audio output config")?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let shared = Arc::new(AudioShared::new());

        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, shared.clone())?,
            SampleFormat::I16 => build::<i16>(&device, &config, shared.clone())?,
            SampleFormat::U16 => build::<u16>(&device, &config, shared.clone())?,
            other => bail!("unsupported audio sample format {other:?}"),
        };
        stream.play().context("could not start audio output")?;

        Ok(AudioOut {
            _stream: stream,
            shared,
            rate: config.sample_rate.0,
            channels: config.channels,
        })
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<AudioShared>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let rate = config.sample_rate.0;
    let mut scratch: Vec<f32> = Vec::new();
    let mut band = BandPass::default();
    // How much of the solo filter is heard, 0..1.
    let mut wet = 0.0f32;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            scratch.clear();
            scratch.resize(data.len(), 0.0);

            let mut got = 0;
            if shared.test_on() {
                let frames = data.len() / channels;
                let start = shared.test_frames.fetch_add(frames as u64, Relaxed);
                for (f, frame) in scratch.chunks_exact_mut(channels).enumerate() {
                    frame.fill(metronome(start + f as u64, rate));
                }
            } else if !shared.paused() {
                let mut q = shared.queue();
                got = data.len().min(q.samples.len()) / channels * channels;
                for (dst, s) in scratch.iter_mut().zip(q.samples.drain(..got)) {
                    *dst = s;
                }
                shared.played.fetch_add((got / channels) as u64, Relaxed);
            }

            // Tap the pre-volume, unfiltered signal, so beat detection depends on
            // neither the volume knob nor the solo. Never skipped: a gap would be a
            // jump in the signal, which reads as a hit. The UI holds the lock only to
            // copy it out.
            if got > 0 {
                let mut tap = shared.tap.lock().unwrap_or_else(|e| e.into_inner());
                for frame in scratch[..got].chunks_exact(channels) {
                    tap.samples
                        .push_back(frame.iter().sum::<f32>() / channels as f32);
                }
                tap.total += (got / channels) as u64;
                let keep = TAP_LEN + shared.delay.load(Relaxed) as usize;
                let excess = tap.samples.len().saturating_sub(keep);
                tap.samples.drain(..excess);
            }

            // The solo fades in and out; while it fades out, the last range stays.
            let solo = shared.solo().filter(|_| !shared.test_on());
            if let Some(range) = solo {
                band.set(range, rate, channels);
            }
            let target = if solo.is_some() { 1.0 } else { 0.0 };
            let fade_step = 1.0 / (SOLO_FADE * rate as f32);
            if target > 0.0 || wet > 0.0 {
                for frame in scratch.chunks_exact_mut(channels) {
                    wet = if target > wet {
                        (wet + fade_step).min(1.0)
                    } else {
                        (wet - fade_step).max(0.0)
                    };
                    for (ch, x) in frame.iter_mut().enumerate() {
                        let y = band.run(*x, ch);
                        *x += (y - *x) * wet;
                    }
                }
            }

            let vol = shared.volume();
            for (dst, s) in data.iter_mut().zip(&scratch) {
                *dst = T::from_sample(*s * vol);
            }
        },
        // Can't print inside the TUI; a broken stream shows up as the clock stalling.
        |_err| {},
        None,
    )?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heard_window_lags_by_the_delay() {
        assert_eq!(heard_window(TAP_LEN, 0), (0, TAP_LEN));
        assert_eq!(heard_window(TAP_LEN + 500, 500), (0, TAP_LEN));
        assert_eq!(
            heard_window(TAP_LEN + 10_000, 500),
            (9_500, TAP_LEN + 9_500)
        );
        // Not enough heard yet: what there is, or nothing.
        assert_eq!(heard_window(300, 500), (0, 0));
        assert_eq!(heard_window(800, 500), (0, 300));
    }

    #[test]
    fn metronome_clicks_on_the_beat() {
        let rate = 48_000;
        let at = |secs: f64| (secs * f64::from(rate)) as u64;
        assert_eq!(metronome_beat(at(0.0), rate).0, 0);
        assert_eq!(metronome_beat(at(BEAT_SECS * 5.0 + 0.01), rate).0, 5);
        // Sound right after a beat, silence between clicks.
        let loud = (0..at(CLICK_SECS))
            .map(|n| metronome(n, rate).abs())
            .fold(0.0, f32::max);
        assert!(loud > 0.3, "{loud}");
        let quiet = (at(0.1)..at(0.5)).all(|n| metronome(n, rate) == 0.0);
        assert!(quiet);
    }

    #[test]
    fn test_frames_are_heard_after_the_delay() {
        let a = AudioShared::new();
        a.set_test(true);
        a.set_delay_frames(1_000);
        a.test_frames.store(600, Relaxed);
        assert_eq!(a.heard_test_frames(), None);
        a.test_frames.store(1_600, Relaxed);
        assert_eq!(a.heard_test_frames(), Some(600));
        a.set_test(false);
        assert!(!a.test_on());
    }

    /// RMS of a sine at `hz` after the band-pass, once it settled.
    fn through(range: (u32, u32), hz: f32) -> f32 {
        let rate = 48_000;
        let mut band = BandPass::default();
        band.set(range, rate, 1);
        let out: Vec<f32> = (0..rate)
            .map(|n| {
                band.run(
                    (std::f32::consts::TAU * hz * n as f32 / rate as f32).sin(),
                    0,
                )
            })
            .collect();
        let tail = &out[rate as usize / 2..];
        (tail.iter().map(|y| y * y).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn solo_plays_only_its_range() {
        let full = std::f32::consts::FRAC_1_SQRT_2;
        // Inside: about unchanged. An octave out: below -40 dB.
        for hz in [280.0, 400.0] {
            let rms = through((200, 500), hz);
            assert!((rms / full - 1.0).abs() < 0.1, "{hz} Hz: {rms}");
        }
        for hz in [100.0, 1_000.0] {
            assert!(through((200, 500), hz) < 0.01 * full, "{hz} Hz leaked");
        }
        // From 0 Hz: only a low-pass. Up to 12 kHz at 48 kHz: only a high-pass.
        assert!(through((0, 150), 60.0) > 0.9 * full);
        assert!(through((0, 150), 600.0) < 0.01 * full);
        assert!(through((4_000, 12_000), 9_000.0) > 0.9 * full);
        assert!(through((4_000, 12_000), 1_000.0) < 0.01 * full);
        // Both a 50 Hz-wide low range and a narrow one stay stable.
        assert!(through((50, 100), 70.0) > 0.5 * full);
        assert!(through((50, 100), 400.0) < 0.01 * full);
    }

    #[test]
    fn solo_range_round_trips() {
        let a = AudioShared::new();
        assert_eq!(a.solo(), None);
        a.set_solo(Some((50, 350)));
        assert_eq!(a.solo(), Some((50, 350)));
        a.set_solo(Some((300, 300)));
        assert_eq!(a.solo(), None);
        a.set_solo(Some((0, 150)));
        assert_eq!(a.solo(), Some((0, 150)));
        a.set_solo(None);
        assert_eq!(a.solo(), None);
    }

    #[test]
    fn tap_counts_heard_samples() {
        let a = AudioShared::new();
        {
            let mut tap = a.tap.lock().unwrap();
            tap.samples.extend((0..1_000).map(|i| i as f32));
            tap.total = 50_000;
        }
        let mut out = Vec::new();
        assert_eq!(a.tap_snapshot(&mut out), 50_000);
        assert_eq!(out.last(), Some(&999.0));
        a.set_delay_frames(100);
        assert_eq!(a.tap_snapshot(&mut out), 49_900);
        assert_eq!(out.last(), Some(&899.0));
        a.reset();
        assert_eq!(a.tap_snapshot(&mut out), 0);
        assert!(out.is_empty());
    }

    #[test]
    fn heard_frames_lag_by_the_delay() {
        let a = AudioShared::new();
        a.played.store(48_000, Relaxed);
        assert_eq!(a.heard_frames(), 48_000);
        a.set_delay_frames(9_600);
        assert_eq!(a.heard_frames(), 38_400);
        a.set_delay_frames(60_000);
        assert_eq!(a.heard_frames(), 0);
    }
}
