//! Player settings that survive restarts: which effects are on, preset, intensity, bass
//! sync and the background fill. Saved to `settings.json` in the config directory
//! whenever one of them changes.

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
    }
}
