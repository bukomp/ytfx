//! Where each video was left off, kept in `resume.json` in the config directory. A video
//! played on its own picks up from there; videos in a playlist, feed, queue or radio
//! always start from the beginning (main.rs decides which is which).

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::account::{config_dir, write_private};
use crate::youtube;

/// Most videos remembered; the ones left longest ago are dropped.
const LIMIT: usize = 300;
/// Stopped before this many seconds: nothing worth resuming.
const MIN_POS: f64 = 10.0;
/// Stopped this close to the end: counts as watched, starts over next time.
const END_MARGIN: f64 = 15.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Spot {
    id: String,
    /// Seconds into the video.
    pos: f64,
    /// When it was left, in seconds since the Unix epoch.
    at: u64,
}

#[derive(Default)]
pub struct Resume {
    /// Most recent first.
    spots: Vec<Spot>,
}

impl Resume {
    /// The saved positions; a missing or unreadable file gives none.
    pub fn load() -> Self {
        let spots = config_dir()
            .ok()
            .and_then(|dir| fs::read_to_string(dir.join("resume.json")).ok())
            .and_then(|text| serde_json::from_str::<Vec<Spot>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|s| youtube::is_video_id(&s.id) && s.pos.is_finite() && s.pos > 0.0)
            .take(LIMIT)
            .collect();
        Resume { spots }
    }

    pub fn save(&self) -> Result<()> {
        write_private(
            &config_dir()?.join("resume.json"),
            &serde_json::to_string(&self.spots)?,
        )
    }

    /// Where `id` was left off, if anywhere.
    pub fn get(&self, id: &str) -> Option<f64> {
        self.spots.iter().find(|s| s.id == id).map(|s| s.pos)
    }

    /// Remember that `id` stopped at `pos`. Near the start or the end (or with no known
    /// length, like live streams) it is forgotten instead. Returns whether anything changed.
    pub fn note(&mut self, id: &str, pos: f64, duration: Option<f64>) -> bool {
        if !youtube::is_video_id(id) {
            return false;
        }
        let keep = duration.is_some_and(|d| pos >= MIN_POS && pos < d - END_MARGIN);
        let old = self.spots.iter().position(|s| s.id == id);
        if !keep {
            return old.map(|i| self.spots.remove(i)).is_some();
        }
        if let Some(i) = old {
            self.spots.remove(i);
        }
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        self.spots.insert(
            0,
            Spot {
                id: id.to_string(),
                pos,
                at,
            },
        );
        self.spots.truncate(LIMIT);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "dQw4w9WgXcQ";

    #[test]
    fn keeps_the_middle_and_forgets_the_ends() {
        let mut r = Resume::default();
        assert!(r.note(ID, 95.0, Some(200.0)));
        assert_eq!(r.get(ID), Some(95.0));
        assert!(r.note(ID, 120.0, Some(200.0)));
        assert_eq!(r.get(ID), Some(120.0));
        assert_eq!(r.spots.len(), 1);
        // Watched to the end: next time starts over.
        assert!(r.note(ID, 190.0, Some(200.0)));
        assert_eq!(r.get(ID), None);
        // Barely started, or a live stream with no length.
        assert!(!r.note(ID, 4.0, Some(200.0)));
        assert!(!r.note(ID, 50.0, None));
        assert_eq!(r.get(ID), None);
    }

    #[test]
    fn ignores_bad_ids_and_drops_the_oldest() {
        let mut r = Resume::default();
        assert!(!r.note("not an id", 50.0, Some(200.0)));
        for i in 0..LIMIT + 5 {
            r.note(&format!("{i:0>11}"), 50.0, Some(200.0));
        }
        assert_eq!(r.spots.len(), LIMIT);
        assert_eq!(r.get(&format!("{:0>11}", 0)), None);
        assert_eq!(r.get(&format!("{:0>11}", LIMIT + 4)), Some(50.0));
    }
}
