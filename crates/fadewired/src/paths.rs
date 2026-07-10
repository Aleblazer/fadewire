//! XDG paths for the daemon's files.

use std::path::PathBuf;

fn xdg(base_var: &str, home_fallback: &[&str]) -> PathBuf {
    std::env::var_os(base_var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            let mut p = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            for part in home_fallback {
                p.push(part);
            }
            p
        })
}

/// `$XDG_CONFIG_HOME/fadewire/config.toml` (usually `~/.config/fadewire/`).
/// User-owned; the daemon only reads it.
pub fn config() -> PathBuf {
    xdg("XDG_CONFIG_HOME", &[".config"]).join("fadewire").join("config.toml")
}

/// `$XDG_STATE_HOME/fadewire/state.toml` (usually `~/.local/state/fadewire/`).
/// Daemon-owned runtime state (virtual fader levels).
pub fn state() -> PathBuf {
    xdg("XDG_STATE_HOME", &[".local", "state"]).join("fadewire").join("state.toml")
}
