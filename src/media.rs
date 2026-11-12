//! OS media controls: the keyboard's play/pause, next and previous keys, headphone
//! buttons (AirPods, wired remotes, Bluetooth headsets) and the system's "Now Playing"
//! widget. macOS uses MPRemoteCommandCenter, Linux uses MPRIS (D-Bus). Also watches the
//! audio output device, so unplugging headphones can pause playback.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait};

#[cfg(target_os = "macos")]
use macos::Backend;
#[cfg(all(unix, not(target_os = "macos")))]
use mpris::Backend;

/// What the OS asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    /// Seek by this many seconds (negative: back). MPRIS only: on macOS, skip buttons
    /// would replace next / previous in the Now Playing widget.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
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
    backend: Option<Backend>,
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
        #[cfg(unix)]
        let (backend, error) = match Backend::attach(tx) {
            Ok(b) => (Some(b), None),
            Err(e) => (None, Some(format!("media keys unavailable: {e}"))),
        };
        #[cfg(not(unix))]
        let error = {
            drop(tx);
            Some("media keys are not supported on this system".to_string())
        };
        let media = Media {
            #[cfg(unix)]
            backend,
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
        #[cfg(unix)]
        if let Some(b) = self.backend.as_mut() {
            b.pump();
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
        if let Some(b) = self.backend.as_mut() {
            b.set_track(self.track.as_ref());
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
        if let Some(b) = self.backend.as_mut() {
            b.set_state(state);
        }
    }
}

/// A duration the OS can show: known, finite and positive.
fn shown_duration(t: &Track) -> Option<f64> {
    t.duration.filter(|d| d.is_finite() && *d > 0.0)
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

/// macOS: MPRemoteCommandCenter for the commands, MPNowPlayingInfoCenter for the widget.
/// The command handlers run on the main run loop, which `pump` runs.
#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::thread;

    use block2::RcBlock;
    use objc2::AllocAnyThread;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSImage;
    use objc2_core_foundation::CGSize;
    use objc2_foundation::{NSData, NSMutableDictionary, NSNumber, NSString, NSURL};
    use objc2_media_player::{
        MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyArtist,
        MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle,
        MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
        MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState, MPRemoteCommand,
        MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
    };

    use super::{Command, State, Track, shown_duration};

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
    }

    pub struct Backend {
        /// Each command with the handler token it returned, to remove them again.
        targets: Vec<(Retained<MPRemoteCommand>, Retained<AnyObject>)>,
        track: Option<Track>,
        state: State,
        artwork: Option<Retained<MPMediaItemArtwork>>,
        /// Downloaded cover images, by URL (downloads run on another thread).
        covers_tx: Sender<(String, Vec<u8>)>,
        covers_rx: Receiver<(String, Vec<u8>)>,
    }

    impl Backend {
        pub fn attach(tx: Sender<Command>) -> Result<Self, String> {
            let mut targets = Vec::new();
            // SAFETY: plain MediaPlayer calls on the main thread (main.rs creates this
            // there). The handlers only send into a channel and report success.
            unsafe {
                let center = MPRemoteCommandCenter::sharedCommandCenter();
                let mut add =
                    |cmd: Retained<MPRemoteCommand>,
                     make: fn(NonNull<MPRemoteCommandEvent>) -> Command| {
                        let tx = tx.clone();
                        let handler = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| {
                            let _ = tx.send(make(event));
                            MPRemoteCommandHandlerStatus::Success
                        });
                        cmd.setEnabled(true);
                        let target = cmd.addTargetWithHandler(&handler);
                        targets.push((cmd, target));
                    };
                add(center.togglePlayPauseCommand(), |_| Command::Toggle);
                add(center.playCommand(), |_| Command::Play);
                add(center.pauseCommand(), |_| Command::Pause);
                add(center.stopCommand(), |_| Command::Stop);
                add(center.nextTrackCommand(), |_| Command::Next);
                add(center.previousTrackCommand(), |_| Command::Previous);
                add(
                    center.changePlaybackPositionCommand().into_super(),
                    |event| {
                        // SAFETY: this command only ever sends position events.
                        let event = event.cast::<MPChangePlaybackPositionCommandEvent>();
                        Command::SeekTo(event.as_ref().positionTime())
                    },
                );
            }
            let (covers_tx, covers_rx) = channel();
            Ok(Backend {
                targets,
                track: None,
                state: State::Stopped,
                artwork: None,
                covers_tx,
                covers_rx,
            })
        }

        /// Run whatever waits on the main run loop (the command handlers), without
        /// blocking, and show covers that finished downloading.
        pub fn pump(&mut self) {
            // SAFETY: called from the main thread; a zero timeout returns right away.
            unsafe {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, 0);
            }
            let wanted = self.track.as_ref().and_then(|t| t.cover.clone());
            let Some((_, bytes)) = self
                .covers_rx
                .try_iter()
                .filter(|(url, _)| Some(url) == wanted.as_ref())
                .last()
            else {
                return;
            };
            let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&bytes));
            if let Some(image) = image {
                let size = image.size();
                let handler = RcBlock::new(move |_: CGSize| NonNull::from(&*image));
                // SAFETY: the handler returns an image the block itself keeps alive.
                self.artwork = Some(unsafe {
                    MPMediaItemArtwork::initWithBoundsSize_requestHandler(
                        MPMediaItemArtwork::alloc(),
                        size,
                        &handler,
                    )
                });
                self.publish();
            }
        }

        pub fn set_track(&mut self, track: Option<&Track>) {
            self.track = track.cloned();
            self.artwork = None;
            if let Some(url) = self.track.as_ref().and_then(|t| t.cover.clone()) {
                let tx = self.covers_tx.clone();
                thread::spawn(move || {
                    let bytes = NSURL::URLWithString(&NSString::from_str(&url))
                        .and_then(|u| NSData::dataWithContentsOfURL(&u))
                        .map(|d| d.to_vec());
                    if let Some(bytes) = bytes {
                        let _ = tx.send((url, bytes));
                    }
                });
            }
            self.publish();
        }

        pub fn set_state(&mut self, state: State) {
            self.state = state;
            self.publish();
        }

        /// Hand the whole current picture to the Now Playing center.
        fn publish(&self) {
            // SAFETY: MediaPlayer's now-playing center, called from the main thread with
            // objects of the types its keys document.
            unsafe {
                let center = MPNowPlayingInfoCenter::defaultCenter();
                let (playback, pos, rate) = match self.state {
                    State::Stopped => (MPNowPlayingPlaybackState::Stopped, 0.0, 0.0),
                    State::Playing(p) => (MPNowPlayingPlaybackState::Playing, p, 1.0),
                    State::Paused(p) => (MPNowPlayingPlaybackState::Paused, p, 0.0),
                };
                let Some(track) = &self.track else {
                    center.setNowPlayingInfo(None);
                    center.setPlaybackState(playback);
                    return;
                };
                let info = NSMutableDictionary::<NSString, AnyObject>::new();
                let text =
                    |s: &str| Retained::into_super(Retained::into_super(NSString::from_str(s)));
                let number = |n: f64| {
                    Retained::into_super(Retained::into_super(Retained::into_super(
                        NSNumber::new_f64(n),
                    )))
                };
                info.insert(MPMediaItemPropertyTitle, &text(&track.title));
                if !track.artist.is_empty() {
                    info.insert(MPMediaItemPropertyArtist, &text(&track.artist));
                }
                if let Some(d) = shown_duration(track) {
                    info.insert(MPMediaItemPropertyPlaybackDuration, &number(d));
                }
                info.insert(
                    MPNowPlayingInfoPropertyElapsedPlaybackTime,
                    &number(pos.max(0.0)),
                );
                info.insert(MPNowPlayingInfoPropertyPlaybackRate, &number(rate));
                if let Some(art) = &self.artwork {
                    info.insert(
                        MPMediaItemPropertyArtwork,
                        &Retained::into_super(Retained::into_super(art.clone())),
                    );
                }
                center.setNowPlayingInfo(Some(&info));
                center.setPlaybackState(playback);
            }
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            // SAFETY: removes the handlers `attach` added, on the main thread.
            unsafe {
                for (cmd, target) in &self.targets {
                    cmd.removeTarget(Some(target));
                    cmd.setEnabled(false);
                }
                MPNowPlayingInfoCenter::defaultCenter().setNowPlayingInfo(None);
            }
        }
    }
}

/// Linux and the BSDs: MPRIS over D-Bus, through souvlaki.
#[cfg(all(unix, not(target_os = "macos")))]
mod mpris {
    use std::sync::mpsc::Sender;
    use std::time::Duration;

    use souvlaki::{
        MediaControlEvent as E, MediaControls, MediaMetadata, MediaPlayback, MediaPosition,
        PlatformConfig, SeekDirection,
    };

    use super::{Command, State, Track, shown_duration};

    pub struct Backend(MediaControls);

    impl Backend {
        pub fn attach(tx: Sender<Command>) -> Result<Self, String> {
            let mut controls = MediaControls::new(PlatformConfig {
                display_name: "ytfx",
                dbus_name: "ytfx",
                hwnd: None,
            })
            .map_err(|e| format!("{e:?}"))?;
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
                .map_err(|e| format!("{e:?}"))?;
            Ok(Backend(controls))
        }

        /// D-Bus messages are handled on souvlaki's own thread.
        pub fn pump(&mut self) {}

        pub fn set_track(&mut self, t: Option<&Track>) {
            let _ = self.0.set_metadata(MediaMetadata {
                title: t.map(|t| t.title.as_str()),
                artist: t.map(|t| t.artist.as_str()).filter(|a| !a.is_empty()),
                album: None,
                cover_url: t.and_then(|t| t.cover.as_deref()),
                duration: t.and_then(shown_duration).map(Duration::from_secs_f64),
            });
        }

        pub fn set_state(&mut self, state: State) {
            let at = |s: f64| Some(MediaPosition(Duration::from_secs_f64(s.max(0.0))));
            let _ = self.0.set_playback(match state {
                State::Stopped => MediaPlayback::Stopped,
                State::Playing(s) => MediaPlayback::Playing { progress: at(s) },
                State::Paused(s) => MediaPlayback::Paused { progress: at(s) },
            });
        }
    }
}
