//! The daemon's configuration model, stored as TOML at
//! `$XDG_CONFIG_HOME/fadewire/config.toml` (usually `~/.config/fadewire/`).
//!
//! See `docs/config.example.toml` for a worked example.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::calibration::Calibration;

/// Top-level config: an ordered list of faders.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub fader: Vec<FaderConfig>,
}

/// One fader: where its level comes from and what it drives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FaderConfig {
    pub label: String,
    pub kind: FaderKind,
    /// HID report axis (0..=7) for a physical fader; ignored for virtual.
    pub axis: i32,
    /// Maximum volume % this fader reaches at the top of its throw.
    pub max_percent: i32,
    pub calibration: Calibration,
    pub target: Target,
}

impl Default for FaderConfig {
    fn default() -> Self {
        Self {
            label: String::new(),
            kind: FaderKind::Physical,
            axis: 0,
            max_percent: 100,
            calibration: Calibration::default(),
            target: Target::default(),
        }
    }
}

/// Whether the fader is driven by hardware or by the UI/hotkeys/CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaderKind {
    #[default]
    Physical,
    Virtual,
}

/// What a fader controls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Target {
    /// An output device (PipeWire sink), matched by a substring of its
    /// description or node name.
    Sink { name_match: String },
    /// One application's streams, matched on `application.process.binary`
    /// (or the Flatpak application id).
    App { binary: String },
    /// A named group of apps that move together.
    Category { name: String },
    /// Every stream not assigned to a category and not targeted directly by
    /// another fader ("Everything Else").
    Unassigned,
}

impl Default for Target {
    fn default() -> Self {
        Target::Unassigned
    }
}

impl Config {
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
        Self::from_toml(&text).map_err(ConfigError::Parse)
    }
}

/// Why a config failed to load.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "reading config: {e}"),
            ConfigError::Parse(e) => write!(f, "parsing config: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::Taper;

    #[test]
    fn example_config_parses() {
        let cfg = Config::from_toml(include_str!("../../../docs/config.example.toml"))
            .expect("docs/config.example.toml must stay valid");
        assert_eq!(cfg.fader.len(), 4);

        let game = &cfg.fader[0];
        assert_eq!(game.label, "Game");
        assert_eq!(game.kind, FaderKind::Physical);
        assert_eq!(game.calibration.taper, Taper::Linear);
        assert_eq!(game.calibration.mute_raw, 10);
        assert!(matches!(&game.target, Target::Sink { name_match } if name_match.contains("Game")));

        assert!(matches!(&cfg.fader[1].target, Target::App { binary } if binary == "discord"));
        assert!(matches!(&cfg.fader[2].target, Target::Category { name } if name == "Music"));
        assert_eq!(cfg.fader[3].kind, FaderKind::Virtual);
        assert!(matches!(&cfg.fader[3].target, Target::Unassigned));
    }

    #[test]
    fn roundtrips_through_toml() {
        let cfg = Config {
            fader: vec![
                FaderConfig {
                    label: "Chat".into(),
                    kind: FaderKind::Virtual,
                    axis: -1,
                    max_percent: 80,
                    target: Target::App {
                        binary: "discord".into(),
                    },
                    ..FaderConfig::default()
                },
                FaderConfig::default(),
            ],
        };
        let text = cfg.to_toml().unwrap();
        let back = Config::from_toml(&text).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn missing_fields_get_defaults() {
        let cfg = Config::from_toml(
            r#"
            [[fader]]
            label = "Minimal"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.fader[0].max_percent, 100);
        assert_eq!(cfg.fader[0].kind, FaderKind::Physical);
        assert_eq!(cfg.fader[0].target, Target::Unassigned);
        assert_eq!(cfg.fader[0].calibration, Calibration::default());
    }
}
