//! OS media controls: the keyboard's play/pause, next and previous keys, headphone
//! buttons (AirPods, wired remotes, Bluetooth headsets) and the system's "Now Playing"
//! widget. macOS uses MPRemoteCommandCenter, Linux uses MPRIS (D-Bus). Also watches the
//! audio output device, so unplugging headphones can pause playback.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait};

/// What the OS asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    /// Seek by this many seconds (negative: back).
    SeekBy(f64),
    /// Jump to this many seconds into the video.
    SeekTo(f64),
    /// The default audio output device changed (headphones plugged in or out).
    OutputChanged,
}

/// What the "Now Playing" widget shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub cover: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    Stopped,
    Playing(f64),
    Paused(f64),
}

/// How often the progress shown by the OS is corrected while nothing else changes.
const PROGRESS_EVERY: Duration = Duration::from_secs(5);
/// How often the audio output device is checked.
const OUTPUT_EVERY: Duration = Duration::from_secs(1);

pub struct Media {
    #[cfg(unix)]
    controls: Option<souvlaki::MediaControls>,
    rx: Receiver<Command>,
    track: Option<Track>,
    state: State,
    sent_at: Instant,
}

impl Media {
    /// Connect to the OS media controls. Never fails: without them ytfx just doesn't get
    /// the keys (the error says why).
    pub fn new() -> (Self, Option<String>) {
        let (tx, rx) = channel();
        watch_output(tx.clone());
        let (controls, error) = match attach(tx) {
            Ok(c) => (Some(c), None),
            Err(e) => (None, Some(e)),
        };
        #[cfg(not(unix))]
        let _ = controls;
        let media = Media {
            #[cfg(unix)]
            controls,
            rx,
            track: None,
            state: State::Stopped,
            sent_at: Instant::now(),
        };
        (media, error)
    }

    /// The commands that came in since the last call. Call it often from the main thread:
    /// on macOS it also runs the main run loop, which is where the commands arrive.
    pub fn poll(&mut self) -> Vec<Command> {
        #[cfg(target_os = "macos")]
        if self.controls.is_some() {
            macos::pump_run_loop();
        }
        self.rx.try_iter().collect()
    }

    /// Show `track` as playing (`None`: nothing is).
    pub fn set_track(&mut self, track: Option<Track>) {
        if self.track == track {
            return;
        }
        self.track = track;
        #[cfg(unix)]
        if let Some(c) = self.controls.as_mut() {
            let t = self.track.as_ref();
            let _ = c.set_metadata(souvlaki::MediaMetadata {
                title: t.map(|t| t.title.as_str()),
                artist: t.map(|t| t.artist.as_str()).filter(|a| !a.is_empty()),
                album: None,
                cover_url: t.and_then(|t| t.cover.as_deref()),
                duration: t
                    .and_then(|t| t.duration)
                    .filter(|d| d.is_finite() && *d > 0.0)
                    .map(Duration::from_secs_f64),
            });
        }
        // New metadata resets the progress the OS shows.
        self.send_state(self.state);
    }

    /// Tell the OS whether something plays and where it is. Cheap to call every frame:
    /// it only passes on changes, seeks and a periodic progress correction.
    pub fn set_state(&mut self, state: State) {
        let elapsed = self.sent_at.elapsed();
        let changed = match (self.state, state) {
            (State::Playing(old), State::Playing(new)) => {
                let expected = old + elapsed.as_secs_f64();
                (new - expected).abs() > 1.5 || elapsed >= PROGRESS_EVERY
            }
            (State::Paused(old), State::Paused(new)) => (new - old).abs() > 0.5,
            (old, new) => std::mem::discriminant(&old) != std::mem::discriminant(&new),
        };
        if changed {
            self.send_state(state);
        }
    }

    fn send_state(&mut self, state: State) {
        self.state = state;
        self.sent_at = Instant::now();
        #[cfg(unix)]
        if let Some(c) = self.controls.as_mut() {
            use souvlaki::{MediaPlayback, MediaPosition};
            let at = |s: f64| Some(MediaPosition(Duration::from_secs_f64(s.max(0.0))));
            let _ = c.set_playback(match state {
                State::Stopped => MediaPlayback::Stopped,
                State::Playing(s) => MediaPlayback::Playing { progress: at(s) },
                State::Paused(s) => MediaPlayback::Paused { progress: at(s) },
            });
        }
    }
}

#[cfg(unix)]
fn attach(tx: Sender<Command>) -> Result<souvlaki::MediaControls, String> {
    use souvlaki::{MediaControlEvent as E, MediaControls, PlatformConfig, SeekDirection};

    let mut controls = MediaControls::new(PlatformConfig {
        display_name: "ytfx",
        dbus_name: "ytfx",
        hwnd: None,
    })
    .map_err(|e| format!("media keys unavailable: {e:?}"))?;
    controls
        .attach(move |event| {
            let sign = |d: SeekDirection| match d {
                SeekDirection::Forward => 1.0,
                SeekDirection::Backward => -1.0,
            };
            let cmd = match event {
                E::Play => Command::Play,
                E::Pause => Command::Pause,
                E::Toggle => Command::Toggle,
                E::Next => Command::Next,
                E::Previous => Command::Previous,
                E::Stop => Command::Stop,
                E::Seek(d) => Command::SeekBy(sign(d) * 10.0),
                E::SeekBy(d, by) => Command::SeekBy(sign(d) * by.as_secs_f64()),
                E::SetPosition(p) => Command::SeekTo(p.0.as_secs_f64()),
                _ => return,
            };
            let _ = tx.send(cmd);
        })
        .map_err(|e| format!("media keys unavailable: {e:?}"))?;
    Ok(controls)
}

#[cfg(not(unix))]
fn attach(_tx: Sender<Command>) -> Result<(), String> {
    Err("media keys are not supported on this system".into())
}

/// Report when the default output device changes, from a background thread (asking the
/// audio system can take a moment).
fn watch_output(tx: Sender<Command>) {
    thread::spawn(move || {
        let current = || {
            cpal::default_host()
                .default_output_device()
                .and_then(|d| d.name().ok())
        };
        let mut last = current();
        loop {
            thread::sleep(OUTPUT_EVERY);
            let now = current();
            if now != last {
                last = now;
                if tx.send(Command::OutputChanged).is_err() {
                    return;
                }
            }
        }
    });
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
    }

    /// Handle whatever is waiting on the main run loop (the remote command handlers run
    /// there), without blocking.
    pub fn pump_run_loop() {
        // SAFETY: called from the main thread; a zero timeout returns right away.
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, 0);
        }
    }
}
