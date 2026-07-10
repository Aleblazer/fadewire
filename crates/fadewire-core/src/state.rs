//! Runtime state the daemon persists across restarts — virtual faders'
//! levels and mute state. Kept in a *separate* file
//! (`$XDG_STATE_HOME/fadewire/state.toml`) so the daemon never rewrites the
//! user's hand-edited config (which would destroy comments and formatting).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::ConfigError;

/// Persisted runtime state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub fader: Vec<VirtualLevel>,
}

/// One virtual fader's saved position, keyed by its config label.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VirtualLevel {
    pub label: String,
    /// Current level (0..=100); 0 while muted.
    pub level: i32,
    pub muted: bool,
    /// The level the mute toggle restores.
    pub pre_mute: i32,
}

impl State {
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

    /// Atomic save (write a temp file, then rename over the target) so a
    /// crash mid-write can't leave a corrupt state file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = self.to_toml().map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_toml() {
        let state = State {
            fader: vec![
                VirtualLevel {
                    label: "Everything Else".into(),
                    level: 0,
                    muted: true,
                    pre_mute: 65,
                },
                VirtualLevel {
                    label: "Chat".into(),
                    level: 40,
                    muted: false,
                    pre_mute: 40,
                },
            ],
        };
        let text = state.to_toml().unwrap();
        assert_eq!(State::from_toml(&text).unwrap(), state);
    }

    #[test]
    fn empty_text_is_a_default_state() {
        assert_eq!(State::from_toml("").unwrap(), State::default());
    }
}
