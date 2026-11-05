//! Watch history: the videos ytfx played, newest first. Kept locally (works without a
//! YouTube account) in `history.json` in the config directory.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::account::{config_dir, write_private};
use crate::youtube::{self, Entry, Kind, Meta};

/// Most videos remembered; older ones are dropped.
const LIMIT: usize = 500;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Record {
    id: String,
    title: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    thumbnail: Option<String>,
    /// When it last started playing, in seconds since the Unix epoch.
    #[serde(default)]
    at: u64,
}

#[derive(Default)]
pub struct History {
    records: Vec<Record>,
}

impl History {
    /// The saved history; a missing or unreadable file gives an empty one.
    pub fn load() -> Self {
        let records = config_dir()
            .ok()
            .and_then(|dir| fs::read_to_string(dir.join("history.json")).ok())
            .and_then(|text| serde_json::from_str::<Vec<Record>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|r| youtube::is_video_id(&r.id))
            .map(|mut r| {
                // The file may have been edited: keep its text off the terminal's escape
                // sequences, and only ever fetch thumbnails from YouTube.
                r.title = youtube::clean(&r.title);
                r.channel = youtube::clean(&r.channel);
                r.thumbnail = r.thumbnail.as_deref().and_then(youtube::image_url);
                r
            })
            .take(LIMIT)
            .collect();
        History { records }
    }

    pub fn save(&self) -> Result<()> {
        write_private(
            &config_dir()?.join("history.json"),
            &serde_json::to_string(&self.records)?,
        )
    }

    /// Put `entry` at the top, moving it there if it was played before.
    pub fn record(&mut self, entry: &Entry) {
        if entry.kind != Kind::Video || !youtube::is_video_id(&entry.target) {
            return;
        }
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        self.records.retain(|r| r.id != entry.target);
        self.records.insert(
            0,
            Record {
                id: entry.target.clone(),
                title: entry.title.clone(),
                channel: entry.channel.clone(),
                duration: entry.duration,
                thumbnail: entry.meta.thumbnail.clone(),
                at,
            },
        );
        self.records.truncate(LIMIT);
    }

    /// Forget the video with this id. Returns whether it was there.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.records.len();
        self.records.retain(|r| r.id != id);
        self.records.len() != before
    }

    /// The history as list rows, newest first.
    pub fn entries(&self) -> Vec<Entry> {
        self.records
            .iter()
            .map(|r| Entry {
                kind: Kind::Video,
                target: r.id.clone(),
                title: r.title.clone(),
                channel: r.channel.clone(),
                duration: r.duration,
                views: None,
                meta: Meta {
                    thumbnail: r.thumbnail.clone(),
                    ..Default::default()
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str) -> Entry {
        Entry {
            kind: Kind::Video,
            target: id.into(),
            title: format!("title {id}"),
            channel: "chan".into(),
            duration: Some(60.0),
            views: None,
            meta: Default::default(),
        }
    }

    #[test]
    fn newest_first_without_duplicates() {
        let mut h = History::default();
        for id in ["aaaaaaaaaaa", "bbbbbbbbbbb", "aaaaaaaaaaa"] {
            h.record(&video(id));
        }
        let ids: Vec<_> = h.entries().into_iter().map(|e| e.target).collect();
        assert_eq!(ids, ["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    }

    #[test]
    fn caps_length_and_skips_non_videos() {
        let mut h = History::default();
        for i in 0..LIMIT + 5 {
            h.record(&video(&format!("{i:0>11}")));
        }
        assert_eq!(h.entries().len(), LIMIT);
        let mut list = video("PLxxxxxxxxxx");
        list.kind = Kind::Playlist;
        h.record(&list);
        assert_eq!(h.entries()[0].target, format!("{:0>11}", LIMIT + 4));
    }

    #[test]
    fn remove_forgets_a_video() {
        let mut h = History::default();
        h.record(&video("aaaaaaaaaaa"));
        assert!(h.remove("aaaaaaaaaaa"));
        assert!(!h.remove("aaaaaaaaaaa"));
        assert!(h.entries().is_empty());
    }

    #[test]
    fn records_round_trip_through_json() {
        let mut h = History::default();
        h.record(&video("aaaaaaaaaaa"));
        let text = serde_json::to_string(&h.records).unwrap();
        let back: Vec<Record> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, h.records);
    }
}
