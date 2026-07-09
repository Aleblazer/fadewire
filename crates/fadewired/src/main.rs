//! FadeWire daemon (pre-alpha skeleton).
//!
//! Roadmap for this binary (see docs/architecture.md):
//! 1. hidraw reader for the vendor-page fader report (id 2, up to eight
//!    signed 16-bit LE axes) with poll-based reconnect.
//! 2. PipeWire mixer backend (sinks + per-app streams; libpulse via
//!    pipewire-pulse first, native pipewire-rs later).
//! 3. D-Bus service (`xyz.splitlogic.FadeWire`) for the CLI/GUI.
//! 4. Optional evdev hotkey listener (F13–F24, pass-through).

use anyhow::Result;
use fadewire_core::config::Config;
use std::path::PathBuf;

/// `$XDG_CONFIG_HOME/fadewire/config.toml`, falling back to
/// `~/.config/fadewire/config.toml`.
fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".config")
        });
    base.join("fadewire").join("config.toml")
}

fn main() -> Result<()> {
    let path = config_path();
    let cfg = if path.exists() {
        Config::load(&path)?
    } else {
        eprintln!(
            "fadewired: no config at {} — starting with an empty layout",
            path.display()
        );
        Config::default()
    };

    println!(
        "fadewired {} — {} fader(s) configured",
        env!("CARGO_PKG_VERSION"),
        cfg.fader.len()
    );
    for f in &cfg.fader {
        println!("  · {:?} fader \"{}\" -> {:?}", f.kind, f.label, f.target);
    }
    println!("pre-alpha skeleton: the PipeWire and hidraw backends are the next milestone.");
    Ok(())
}
