//! The play queue: a list of videos with a current position. In radio mode it is refilled
//! from YouTube Mixes of recently played videos, shuffled, never repeating a video.

use std::collections::HashSet;

use crate::effects::Rng;
use crate::youtube::{Entry, Kind};

/// Refill radio when fewer than this many videos are left after the current one.
pub const RADIO_LOW: usize = 3;
/// Played videos to keep before the current one; older ones are dropped.
const HISTORY: usize = 100;

#[derive(Default)]
pub struct Queue {
    pub items: Vec<Entry>,
    pub pos: usize,
    pub radio: bool,
    seen: HashSet<String>,
}

impl Queue {
    /// Replace the queue with the videos of `entries`, starting at `entries[start]`.
    pub fn set(&mut self, entries: &[Entry], start: usize) {
        let start_id = entries.get(start).map(|e| e.target.clone());
        self.items = entries
            .iter()
            .filter(|e| e.kind == Kind::Video)
            .cloned()
            .collect();
        self.pos = start_id
            .and_then(|id| self.items.iter().position(|e| e.target == id))
            .unwrap_or(0);
        self.radio = false;
        self.seen.clear();
    }

    /// Start radio: `seed` plays (or keeps playing), followed by the shuffled `mix`.
    pub fn start_radio(&mut self, seed: Entry, mix: Vec<Entry>, rng: &mut Rng) {
        self.items.clear();
        self.pos = 0;
        self.seen.clear();
        self.seen.insert(seed.target.clone());
        self.items.push(seed);
        self.radio = true;
        self.extend_radio(mix, rng);
    }

    /// Append the videos of `mix` that radio hasn't queued yet, shuffled. Returns how many.
    pub fn extend_radio(&mut self, mix: Vec<Entry>, rng: &mut Rng) -> usize {
        let mut fresh: Vec<Entry> = mix
            .into_iter()
            .filter(|e| e.kind == Kind::Video && self.seen.insert(e.target.clone()))
            .collect();
        shuffle(&mut fresh, rng);
        let added = fresh.len();
        self.items.extend(fresh);
        if self.pos > HISTORY * 2 {
            let drop = self.pos - HISTORY;
            self.items.drain(..drop);
            self.pos -= drop;
        }
        added
    }

    /// A random recently played video to seed the next Mix from.
    pub fn radio_seed(&self, rng: &mut Rng) -> Option<&Entry> {
        let first = self.pos.saturating_sub(4);
        let recent = self.items.get(first..=self.pos)?;
        recent.get(rng.below(recent.len()))
    }

    pub fn current(&self) -> Option<&Entry> {
        self.items.get(self.pos)
    }

    pub fn peek_next(&self) -> Option<&Entry> {
        self.items.get(self.pos + 1)
    }

    /// Videos queued after the current one.
    pub fn remaining(&self) -> usize {
        self.items.len().saturating_sub(self.pos + 1)
    }

    pub fn advance(&mut self) -> Option<&Entry> {
        if self.pos + 1 < self.items.len() {
            self.pos += 1;
            self.current()
        } else {
            None
        }
    }

    pub fn back(&mut self) -> Option<&Entry> {
        self.pos = self.pos.checked_sub(1)?;
        self.current()
    }

    pub fn jump(&mut self, i: usize) -> Option<&Entry> {
        if i < self.items.len() {
            self.pos = i;
        }
        self.items.get(i)
    }

    pub fn push(&mut self, entry: Entry) {
        self.seen.insert(entry.target.clone());
        self.items.push(entry);
    }

    /// Remove an item. The current one can go only while nothing is `playing`; the
    /// position then moves to the video after it (or the new last one).
    pub fn remove(&mut self, i: usize, playing: bool) -> bool {
        if i >= self.items.len() || (playing && i == self.pos) {
            return false;
        }
        self.items.remove(i);
        if i < self.pos {
            self.pos -= 1;
        }
        self.pos = self.pos.min(self.items.len().saturating_sub(1));
        true
    }

    /// Shuffle the whole queue and start from the top.
    pub fn shuffle_all(&mut self, rng: &mut Rng) {
        shuffle(&mut self.items, rng);
        self.pos = 0;
    }

    /// Shuffle everything after the current video.
    pub fn shuffle_rest(&mut self, rng: &mut Rng) {
        if let Some(rest) = self.items.get_mut(self.pos + 1..) {
            shuffle(rest, rng);
        }
    }
}

fn shuffle<T>(items: &mut [T], rng: &mut Rng) {
    for i in (1..items.len()).rev() {
        items.swap(i, rng.below(i + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str) -> Entry {
        Entry {
            kind: Kind::Video,
            target: id.into(),
            title: id.into(),
            channel: String::new(),
            duration: None,
            views: None,
            meta: Default::default(),
        }
    }

    fn ids(q: &Queue) -> Vec<&str> {
        q.items.iter().map(|e| e.target.as_str()).collect()
    }

    #[test]
    fn set_skips_non_videos_and_keeps_the_selection() {
        let mut list = vec![video("a"), video("b"), video("c")];
        list.insert(
            1,
            Entry {
                kind: Kind::Playlist,
                ..video("p")
            },
        );
        let mut q = Queue::default();
        q.set(&list, 2);
        assert_eq!(ids(&q), ["a", "b", "c"]);
        assert_eq!(q.current().unwrap().target, "b");
        assert_eq!(q.advance().unwrap().target, "c");
        assert!(q.advance().is_none());
        assert_eq!(q.back().unwrap().target, "b");
    }

    #[test]
    fn radio_never_repeats_and_keeps_the_seed_first() {
        let mut rng = Rng::new(7);
        let mut q = Queue::default();
        let mix: Vec<Entry> = ["s", "a", "b", "c", "d"].map(video).into();
        q.start_radio(video("s"), mix, &mut rng);
        assert!(q.radio);
        assert_eq!(q.items[0].target, "s");
        assert_eq!(q.items.len(), 5);

        let again: Vec<Entry> = ["a", "b", "e", "s", "f"].map(video).into();
        assert_eq!(q.extend_radio(again, &mut rng), 2);
        let mut all = ids(&q);
        all.sort();
        all.dedup();
        assert_eq!(all.len(), q.items.len());
    }

    #[test]
    fn remove_keeps_the_current_item() {
        let mut q = Queue::default();
        q.set(&["a", "b", "c"].map(video), 1);
        assert!(!q.remove(1, true));
        assert!(q.remove(0, true));
        assert_eq!(q.current().unwrap().target, "b");
        assert_eq!(q.remaining(), 1);
    }

    #[test]
    fn remove_takes_the_current_item_when_idle() {
        let mut q = Queue::default();
        q.set(&["a", "b"].map(video), 0);
        assert!(q.remove(0, false));
        assert_eq!(q.current().unwrap().target, "b");
        assert!(q.remove(0, false));
        assert!(q.items.is_empty());
        assert_eq!(q.pos, 0);
    }
}
