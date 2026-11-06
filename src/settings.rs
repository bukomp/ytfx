//! Settings that survive restarts: which effects are on, preset, intensity, sync,
//! the hit sensitivity per 50 Hz slice, the audio output delay, the background fill and the browser's preview pane. Saved to `settings.json` in the
//! config directory whenever one of them changes.

use std::collections::BTreeMap;
use std::fs;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::account::{config_dir, write_private};

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Labels of the enabled effects (see `Fx::label`).
    pub effects: Option<Vec<String>>,
    pub preset: Option<String>,
    pub intensity: Option<f32>,
    pub sync: Option<bool>,
    pub fill: Option<bool>,
    /// The browser's preview pane.
    pub preview: Option<bool>,
    /// Hit sensitivity of the 50 Hz slices that differ from the default, keyed by the
    /// slice's lowest frequency in Hz (see `analysis::default_slice_sensitivity`).
    pub freq_sensitivity: Option<BTreeMap<u32, f32>>,
    /// How long the audio output takes to be heard, in ms (see `audio::MAX_DELAY_MS`).
    pub audio_delay_ms: Option<u32>,
}

/// The saved settings; anything missing or unreadable falls back to the defaults.
pub fn load() -> Settings {
    config_dir()
        .ok()
        .and_then(|dir| fs::read_to_string(dir.join("settings.json")).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<()> {
    write_private(
        &config_dir()?.join("settings.json"),
        &serde_json::to_string_pretty(settings)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_and_unknown_fields_still_load() {
        let s: Settings = serde_json::from_str(r#"{"effects":["zoom","nope"],"extra":1}"#).unwrap();
        assert_eq!(s.effects.unwrap(), ["zoom", "nope"]);
        assert_eq!(s.intensity, None);
        assert_eq!(s.freq_sensitivity, None);
    }

    #[test]
    fn slice_sensitivity_round_trips() {
        let s: Settings =
            serde_json::from_str(r#"{"freq_sensitivity":{"50":1.6,"150":0.4}}"#).unwrap();
        let map = s.freq_sensitivity.clone().unwrap();
        assert_eq!(map[&50], 1.6);
        assert_eq!(map[&150], 0.4);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
