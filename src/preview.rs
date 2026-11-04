//! Thumbnails for the browser's preview pane. ffmpeg fetches and decodes each image on a
//! background thread into raw RGB at a fixed size; a small cache keeps recent ones.

use std::collections::{HashMap, VecDeque};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Decode sizes: 16:9 for video and playlist thumbnails, square for channel avatars.
pub const WIDE: (usize, usize) = (320, 180);
pub const SQUARE: (usize, usize) = (176, 176);
/// Images kept in memory (about 170 KB each).
const CACHE: usize = 64;
/// Fetches running at once.
const MAX_JOBS: usize = 3;
/// How long the selection must stay put before its thumbnail is fetched, so scrolling
/// through a list doesn't start a download per row.
const SETTLE: Duration = Duration::from_millis(120);

pub struct Thumb {
    /// Unique per decoded image, so the graphics renderer can skip re-sending it.
    pub id: u64,
    pub data: Vec<u8>,
    pub w: usize,
    pub h: usize,
}

pub enum Slot {
    Loading,
    Ready(Thumb),
    Failed,
}

#[derive(Default)]
pub struct Thumbs {
    slots: HashMap<String, Slot>,
    /// Finished slots, oldest first, for eviction.
    order: VecDeque<String>,
    jobs: usize,
    /// The URL the pane wants and since when.
    want: Option<(String, Instant)>,
    next_id: u64,
}

impl Thumbs {
    pub fn get(&self, url: &str) -> Option<&Slot> {
        self.slots.get(url)
    }

    /// Called every frame with the image the pane shows (None: no pane). Returns the URL
    /// to fetch now, once the selection has settled and a job slot is free.
    pub fn poll(&mut self, url: Option<&str>, now: Instant) -> Option<String> {
        let Some(url) = url.filter(|u| !self.slots.contains_key(*u)) else {
            self.want = None;
            return None;
        };
        match &self.want {
            Some((u, since)) if u == url => {
                if now.duration_since(*since) < SETTLE || self.jobs >= MAX_JOBS {
                    return None;
                }
                self.want = None;
                self.jobs += 1;
                self.slots.insert(url.to_string(), Slot::Loading);
                Some(url.to_string())
            }
            _ => {
                self.want = Some((url.to_string(), now));
                None
            }
        }
    }

    pub fn done(&mut self, url: String, size: (usize, usize), result: Result<Vec<u8>>) {
        self.jobs = self.jobs.saturating_sub(1);
        let slot = match result {
            Ok(data) => {
                self.next_id += 1;
                Slot::Ready(Thumb {
                    id: self.next_id,
                    data,
                    w: size.0,
                    h: size.1,
                })
            }
            Err(_) => Slot::Failed,
        };
        self.slots.insert(url.clone(), slot);
        self.order.push_back(url);
        while self.order.len() > CACHE {
            if let Some(old) = self.order.pop_front() {
                self.slots.remove(&old);
            }
        }
    }
}

/// Download `url` (already checked by `youtube::image_url`) and decode it to RGB24 of
/// exactly `w` x `h`, letterboxed if its aspect differs.
pub fn fetch(url: &str, (w, h): (usize, usize)) -> Result<Vec<u8>> {
    let fit = format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2"
    );
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
        .args(["-protocol_whitelist", "https,tls,tcp"])
        // Give up on a stalled download (microseconds).
        .args(["-rw_timeout", "8000000"])
        .arg("-i")
        .arg(url)
        .args(["-frames:v", "1", "-vf", &fit])
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .context("could not run ffmpeg")?;
    if !out.status.success() || out.stdout.len() != w * h * 3 {
        bail!("thumbnail decode failed");
    }
    Ok(out.stdout)
}

/// Box-filter an RGB24 image down (or nearest-neighbour up) to `ow` x `oh`.
pub fn resample(data: &[u8], w: usize, h: usize, ow: usize, oh: usize) -> Vec<u8> {
    let mut out = vec![0u8; ow * oh * 3];
    if w == 0 || h == 0 || data.len() < w * h * 3 {
        return out;
    }
    for oy in 0..oh {
        let y0 = oy * h / oh;
        let y1 = ((oy + 1) * h / oh).max(y0 + 1).min(h);
        for ox in 0..ow {
            let x0 = ox * w / ow;
            let x1 = ((ox + 1) * w / ow).max(x0 + 1).min(w);
            let mut sum = [0u32; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 3;
                    for c in 0..3 {
                        sum[c] += data[i + c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (oy * ow + ox) * 3;
            for c in 0..3 {
                out[o + c] = (sum[c] / n) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_waits_for_the_selection_to_settle() {
        let mut t = Thumbs::default();
        let t0 = Instant::now();
        assert_eq!(t.poll(Some("a"), t0), None);
        // Moving on restarts the wait.
        assert_eq!(t.poll(Some("b"), t0 + SETTLE), None);
        assert_eq!(t.poll(Some("b"), t0 + SETTLE * 2), Some("b".into()));
        // Loading: not fetched again.
        assert_eq!(t.poll(Some("b"), t0 + SETTLE * 4), None);
        t.done("b".into(), (1, 1), Ok(vec![1, 2, 3]));
        assert!(matches!(t.get("b"), Some(Slot::Ready(th)) if th.data == [1, 2, 3]));
        assert_eq!(t.poll(Some("b"), t0 + SETTLE * 5), None);
    }

    #[test]
    fn cache_evicts_oldest() {
        let mut t = Thumbs::default();
        for i in 0..=CACHE {
            t.done(i.to_string(), (1, 1), Err(anyhow::anyhow!("x")));
        }
        assert!(t.get("0").is_none());
        assert!(t.get(&CACHE.to_string()).is_some());
    }

    #[test]
    fn resample_averages() {
        // 2x1 black and white -> one grey pixel; 1x1 -> 2x2 copies.
        assert_eq!(
            resample(&[0, 0, 0, 255, 255, 255], 2, 1, 1, 1),
            [127, 127, 127]
        );
        assert_eq!(resample(&[9, 8, 7], 1, 1, 2, 2), [9, 8, 7].repeat(4));
    }
}
