//! Audio output. Decoded PCM is pushed into a shared queue, the device callback plays
//! it and copies every played sample into a "tap" buffer that the analyzer reads.
//! Because we are the ones producing the sound, no system loopback capture is needed.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

/// How many mono samples of recently played audio the analyzer can see.
const TAP_LEN: usize = 4096;

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
    tap: Mutex<VecDeque<f32>>,
}

impl AudioShared {
    fn new() -> Self {
        AudioShared {
            queue: Mutex::new(AudioQueue::default()),
            played: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            volume: AtomicU32::new(0.8f32.to_bits()),
            tap: Mutex::new(VecDeque::with_capacity(TAP_LEN)),
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
        self.tap.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn played_frames(&self) -> u64 {
        self.played.load(Relaxed)
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

    pub fn tap_snapshot(&self, out: &mut Vec<f32>) {
        out.clear();
        out.extend(self.tap.lock().unwrap_or_else(|e| e.into_inner()).iter());
    }
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
    let mut scratch: Vec<f32> = Vec::new();

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            scratch.clear();
            scratch.resize(data.len(), 0.0);

            let mut got = 0;
            if !shared.paused() {
                let mut q = shared.queue();
                got = data.len().min(q.samples.len()) / channels * channels;
                for (dst, s) in scratch.iter_mut().zip(q.samples.drain(..got)) {
                    *dst = s;
                }
                shared.played.fetch_add((got / channels) as u64, Relaxed);
            }

            let vol = shared.volume();
            for (dst, s) in data.iter_mut().zip(&scratch) {
                *dst = T::from_sample(*s * vol);
            }

            // Tap the pre-volume signal so beat detection doesn't depend on the volume knob.
            if got > 0
                && let Ok(mut tap) = shared.tap.try_lock()
            {
                for frame in scratch[..got].chunks_exact(channels) {
                    tap.push_back(frame.iter().sum::<f32>() / channels as f32);
                }
                let excess = tap.len().saturating_sub(TAP_LEN);
                tap.drain(..excess);
            }
        },
        // Can't print inside the TUI; a broken stream shows up as the clock stalling.
        |_err| {},
        None,
    )?;
    Ok(stream)
}
