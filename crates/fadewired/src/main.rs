//! FadeWire daemon.
//!
//! Modes:
//!   fadewired               run the drive loop (systemd user service)
//!   fadewired list          print sinks + app streams as FadeWire sees them
//!   fadewired set <fader> <pct>   one-shot apply (verification until D-Bus)
//!
//! Roadmap (see docs/architecture.md): hidraw reader for physical faders,
//! D-Bus service (`xyz.splitlogic.FadeWire`) for the CLI/GUI, evdev hotkeys.

mod engine;
mod pulse;

use anyhow::{bail, Result};
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

fn load_config() -> Result<Config> {
    let path = config_path();
    if path.exists() {
        Ok(Config::load(&path)?)
    } else {
        eprintln!(
            "fadewired: no config at {} — starting with an empty layout \
             (see /usr/share/doc/fadewire/config.example.toml)",
            path.display()
        );
        Ok(Config::default())
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {
            let cfg = load_config()?;
            println!(
                "fadewired {} — {} fader(s), {} category(ies) configured",
                env!("CARGO_PKG_VERSION"),
                cfg.fader.len(),
                cfg.category.len()
            );
            engine::run(cfg)
        }
        Some("list") => engine::list(),
        Some("set") => {
            let (label, pct) = match (args.get(1), args.get(2)) {
                (Some(l), Some(p)) => (l.clone(), p.parse::<u32>()?),
                _ => bail!("usage: fadewired set <fader-label> <percent>"),
            };
            let cfg = load_config()?;
            engine::set_once(&cfg, &label, pct.min(100))
        }
        Some("--version") | Some("version") => {
            println!("fadewired {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => bail!("unknown command \"{other}\" (try: fadewired [list|set|version])"),
    }
}
