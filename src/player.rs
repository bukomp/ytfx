//! Playback: one ffmpeg process decodes audio into the shared queue, another decodes
//! video into raw RGB frames. Video is slaved to the audio clock.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::audio::{AudioOut, AudioShared};
use crate::youtube::{Stream, StreamInfo};

/// Fresh YouTube stream URLs answer 403 for the first few seconds, so failed decoders are
/// restarted a few times before giving up.
const MAX_ATTEMPTS: u32 = 10;
const RETRY_DELAY: Duration = Duration::from_millis(1500);

fn is_transient(err: &str) -> bool {
    err.contains("403")
}

pub struct Frame {
    pub pts: f64,
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

/// A running ffmpeg process plus its reader thread. Dropping it stops both.
struct Job {
    stop: Arc<AtomicBool>,
    child: Child,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct VideoJob {
    _job: Job,
    rx: Receiver<Frame>,
    /// Set when ffmpeg exits before producing a single frame.
    error: Arc<Mutex<Option<String>>>,
}

pub struct Player {
    pub title: String,
    pub duration: Option<f64>,
    pub aspect: f64,
    video_src: Stream,
    audio_src: Stream,
    shared: Arc<AudioShared>,
    rate: u32,
    channels: u16,
    fps: u32,
    size: (usize, usize),
    /// Stream position where the current decoders started.
    base: f64,
    audio_job: Option<Job>,
    video_job: Option<VideoJob>,
    pending: Option<Frame>,
    pub current: Option<Frame>,
    attempts: u32,
    retry_at: Option<Instant>,
}

impl Player {
    pub fn new(info: StreamInfo, audio: &AudioOut, size: (usize, usize), fps: u32) -> Result<Self> {
        let mut p = Player {
            title: info.title,
            duration: info.duration,
            aspect: info.aspect,
            video_src: info.video,
            audio_src: info.audio,
            shared: audio.shared.clone(),
            rate: audio.rate,
            channels: audio.channels,
            fps,
            size,
            base: 0.0,
            audio_job: None,
            video_job: None,
            pending: None,
            current: None,
            attempts: 0,
            retry_at: None,
        };
        p.shared.set_paused(false);
        p.restart(0.0)?;
        Ok(p)
    }

    fn restart(&mut self, pos: f64) -> Result<()> {
        // Stop old decoders before clearing, so they can't push stale samples afterwards.
        self.audio_job = None;
        self.video_job = None;
        self.shared.reset();
        self.base = pos;
        self.pending = None;
        self.audio_job = Some(spawn_audio(
            &self.audio_src,
            pos,
            self.rate,
            self.channels,
            self.shared.clone(),
        )?);
        self.video_job = Some(spawn_video(&self.video_src, pos, self.size, self.fps)?);
        Ok(())
    }

    /// Current playback position in seconds, driven by what has been heard: what the
    /// sound card has played, minus the output delay.
    pub fn position(&self) -> f64 {
        self.base + self.shared.heard_frames() as f64 / self.rate as f64
    }

    pub fn size(&self) -> (usize, usize) {
        self.size
    }

    pub fn paused(&self) -> bool {
        self.shared.paused()
    }

    pub fn toggle_pause(&self) {
        self.shared.set_paused(!self.paused());
    }

    pub fn seek(&mut self, delta: f64) -> Result<()> {
        let mut target = (self.position() + delta).max(0.0);
        if let Some(d) = self.duration {
            target = target.min((d - 1.0).max(0.0));
        }
        self.restart(target)
    }

    /// Restart only the video decoder at a new output size (terminal resized).
    pub fn resize(&mut self, size: (usize, usize)) -> Result<()> {
        self.size = size;
        self.video_job = None;
        self.pending = None;
        self.video_job = Some(spawn_video(
            &self.video_src,
            self.position(),
            size,
            self.fps,
        )?);
        Ok(())
    }

    /// Advance `current` to the newest frame that is due according to the audio clock.
    pub fn update(&mut self) {
        let Some(vj) = &self.video_job else { return };
        let due = self.position() + 0.5 / self.fps as f64;
        loop {
            if self.pending.is_none() {
                match vj.rx.try_recv() {
                    Ok(f) => self.pending = Some(f),
                    Err(_) => break,
                }
            }
            if self.pending.as_ref().is_some_and(|f| f.pts <= due) {
                self.current = self.pending.take();
            } else {
                break;
            }
        }
    }

    /// `Some(Ok)` when playback reached the end, `Some(Err)` if the stream never opened.
    pub fn finished(&self) -> Option<Result<(), String>> {
        let q = self.shared.queue();
        if !(q.done && q.samples.is_empty()) {
            return None;
        }
        if q.received == 0
            && self.attempts < MAX_ATTEMPTS
            && q.error.as_deref().is_some_and(is_transient)
        {
            return None; // retry_transient() will restart it
        }
        if q.received == 0 {
            let why = q.error.as_deref().unwrap_or("ffmpeg returned no data");
            return Some(Err(format!("could not open the audio stream: {why}")));
        }
        Some(Ok(()))
    }

    /// Restart decoders that failed with a transient error, after a short delay.
    pub fn retry_transient(&mut self) -> Result<()> {
        let audio_failed = {
            let q = self.shared.queue();
            q.done && q.received == 0 && q.error.as_deref().is_some_and(is_transient)
        };
        let video_failed = self.video_error().is_some_and(|e| is_transient(&e));
        if !(audio_failed || video_failed) || self.attempts >= MAX_ATTEMPTS {
            self.retry_at = None;
            return Ok(());
        }
        match self.retry_at {
            None => self.retry_at = Some(Instant::now() + RETRY_DELAY),
            Some(at) if Instant::now() >= at => {
                self.retry_at = None;
                self.attempts += 1;
                if audio_failed {
                    self.restart(self.base)?;
                } else {
                    self.resize(self.size)?;
                }
            }
            Some(_) => {}
        }
        Ok(())
    }

    /// Waiting to restart a decoder that hit a transient error.
    pub fn retrying(&self) -> bool {
        self.retry_at.is_some()
    }

    /// Why the video decoder produced nothing, if it failed.
    pub fn video_error(&self) -> Option<String> {
        let vj = self.video_job.as_ref()?;
        vj.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.audio_job = None;
        self.video_job = None;
        self.shared.reset();
    }
}

fn ffmpeg(src: &Stream, start: f64) -> Command {
    let mut cmd = Command::new("ffmpeg");
    // -nostdin is essential: otherwise ffmpeg reads our keypresses from the terminal.
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    // Playlists (HLS/DASH) may point ffmpeg at further URLs; keep those to plain web protocols.
    cmd.args(["-protocol_whitelist", "http,https,tls,tcp,crypto"]);
    cmd.args([
        "-reconnect",
        "1",
        "-reconnect_streamed",
        "1",
        "-reconnect_delay_max",
        "4",
    ]);
    if !src.headers.is_empty() {
        let headers: String = src
            .headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect();
        cmd.arg("-headers").arg(headers);
    }
    if start > 0.0 {
        cmd.arg("-ss").arg(format!("{start:.3}"));
    }
    cmd.arg("-i").arg(&src.url);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Drain ffmpeg's stderr on a thread; the handle yields its last non-empty line.
fn last_stderr_line(child: &mut Child) -> thread::JoinHandle<Option<String>> {
    let stderr = child.stderr.take();
    thread::spawn(move || {
        let mut last = None;
        for line in BufReader::new(stderr?).lines().map_while(|l| l.ok()) {
            let line: String = line.chars().filter(|c| !c.is_control()).collect();
            if !line.trim().is_empty() {
                last = Some(line.trim().to_string());
            }
        }
        last
    })
}

fn spawn_audio(
    src: &Stream,
    start: f64,
    rate: u32,
    channels: u16,
    shared: Arc<AudioShared>,
) -> Result<Job> {
    let mut cmd = ffmpeg(src, start);
    cmd.args([
        "-vn",
        "-sn",
        "-ac",
        &channels.to_string(),
        "-ar",
        &rate.to_string(),
        "-f",
        "f32le",
        "-",
    ]);
    let mut child = cmd
        .spawn()
        .context("could not start ffmpeg (is it installed?)")?;
    let mut out = child.stdout.take().context("ffmpeg stdout")?;
    let stderr = last_stderr_line(&mut child);

    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    // Stay about one second ahead of the device.
    let max_queued = rate as usize * channels as usize;

    thread::spawn(move || {
        let mut bytes = vec![0u8; 16 * 1024];
        let mut samples = Vec::with_capacity(bytes.len() / 4);
        while out.read_exact(&mut bytes).is_ok() {
            samples.clear();
            samples.extend(
                bytes
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            );
            loop {
                {
                    let mut q = shared.queue();
                    if st.load(Relaxed) {
                        return;
                    }
                    if q.samples.len() < max_queued {
                        q.samples.extend(&samples);
                        q.received += samples.len() as u64;
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        let error = stderr.join().ok().flatten();
        let mut q = shared.queue();
        if !st.load(Relaxed) {
            q.done = true;
            q.error = error;
        }
    });

    Ok(Job { stop, child })
}

fn spawn_video(src: &Stream, start: f64, (w, h): (usize, usize), fps: u32) -> Result<VideoJob> {
    let mut cmd = ffmpeg(src, start);
    let filter = format!(
        "fps={fps},scale={w}:{h}:force_original_aspect_ratio=decrease:flags=bilinear,\
         pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,format=rgb24"
    );
    cmd.args(["-an", "-sn", "-vf", &filter, "-f", "rawvideo", "-"]);
    let mut child = cmd
        .spawn()
        .context("could not start ffmpeg (is it installed?)")?;
    let mut out = child.stdout.take().context("ffmpeg stdout")?;
    let stderr = last_stderr_line(&mut child);
    let error = Arc::new(Mutex::new(None));
    let error_slot = error.clone();

    // Small bound: the decoder blocks when it gets ahead, which keeps memory flat.
    let (tx, rx) = sync_channel::<Frame>(8);
    thread::spawn(move || {
        for i in 0u64.. {
            let mut data = vec![0u8; w * h * 3];
            if out.read_exact(&mut data).is_err() {
                if i == 0 {
                    let why = stderr.join().ok().flatten();
                    *error_slot.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(why.unwrap_or_else(|| "ffmpeg returned no frames".into()));
                }
                return;
            }
            let frame = Frame {
                pts: start + i as f64 / fps as f64,
                w,
                h,
                data,
            };
            if tx.send(frame).is_err() {
                return;
            }
        }
    });

    Ok(VideoJob {
        _job: Job {
            stop: Arc::new(AtomicBool::new(false)),
            child,
        },
        rx,
        error,
    })
}
