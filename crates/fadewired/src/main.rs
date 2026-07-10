//! FadeWire daemon.
//!
//! Modes:
//!   fadewired               run the drive loop (systemd user service)
//!   fadewired list          print sinks + app streams as FadeWire sees them
//!   fadewired set <fader> <pct>   one-shot apply (works without the daemon)
//!   fadewired watch         dump raw fader axis values as they arrive
//!
//! While running, the daemon serves `xyz.splitlogic.FadeWire` on the session
//! bus — the `fadewire` CLI is the client. Next up: evdev hotkeys, GUI.

mod dbus;
mod engine;
mod hid;
mod paths;
mod pulse;

use anyhow::{bail, Result};
use fadewire_core::config::Config;

fn load_config() -> Result<Config> {
    let path = paths::config();
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
        Some("watch") => engine::watch(),
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
        Some(other) => bail!("unknown command \"{other}\" (try: fadewired [list|watch|set|version])"),
    }
}
