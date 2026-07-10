//! FadeWire control CLI — talks to the running daemon over D-Bus.
//!
//! This is the scripting surface: bind these in your compositor
//! (Hyprland `bind = , F13, exec, fadewire nudge Chat 5`), call them from
//! scripts, or build bar modules on them.

use anyhow::{bail, Context, Result};

#[zbus::proxy(
    interface = "xyz.splitlogic.FadeWire1",
    default_service = "xyz.splitlogic.FadeWire",
    default_path = "/xyz/splitlogic/FadeWire"
)]
trait FadeWire {
    fn status(&self) -> zbus::Result<(bool, String)>;
    fn faders(&self) -> zbus::Result<Vec<(u32, String, String, i32, String)>>;
    fn set_level(&self, fader: &str, pct: u32) -> zbus::Result<u32>;
    fn nudge(&self, fader: &str, delta: i32) -> zbus::Result<u32>;
    fn toggle_mute(&self, fader: &str) -> zbus::Result<bool>;
}

fn proxy() -> Result<FadeWireProxyBlocking<'static>> {
    let conn = zbus::blocking::Connection::session()
        .context("connecting to the session bus")?;
    FadeWireProxyBlocking::new(&conn)
        .context("is fadewired running? (systemctl --user status fadewire)")
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("status") => {
            let (connected, message) = proxy()?.status().context("querying the daemon")?;
            println!("{} — {message}", if connected { "connected" } else { "disconnected" });
        }
        Some("list") => {
            let rows = proxy()?.faders().context("querying the daemon")?;
            if rows.is_empty() {
                println!("no faders configured (~/.config/fadewire/config.toml)");
            }
            for (i, label, kind, level, target) in rows {
                let level = if level < 0 { "--".to_string() } else { format!("{level}%") };
                println!("[{i}] {label:<20} {kind:<9} {level:>4}  {target}");
            }
        }
        Some("set") => {
            let (label, pct) = two_args(&args, "set <fader> <percent>")?;
            let applied = proxy()?.set_level(&label, pct.parse::<u32>()?.min(100))?;
            println!("{label} = {applied}%");
        }
        Some("nudge") => {
            let (label, delta) = two_args(&args, "nudge <fader> <±percent>")?;
            let applied = proxy()?.nudge(&label, delta.parse::<i32>()?)?;
            println!("{label} = {applied}%");
        }
        Some("mute") => {
            let label = args.get(1).cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: fadewire mute <fader>"))?;
            let muted = proxy()?.toggle_mute(&label)?;
            println!("{label} {}", if muted { "muted" } else { "unmuted" });
        }
        Some("--version") | Some("version") => {
            println!("fadewire {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            println!("fadewire {} — control CLI for fadewired", env!("CARGO_PKG_VERSION"));
            println!();
            println!("usage:");
            println!("  fadewire status                  fader-device connection state");
            println!("  fadewire list                    faders, levels, and targets");
            println!("  fadewire set <fader> <pct>       set a virtual fader's level");
            println!("  fadewire nudge <fader> <±pct>    step a virtual fader (bind me!)");
            println!("  fadewire mute <fader>            toggle a virtual fader's mute");
        }
    }
    Ok(())
}

fn two_args(args: &[String], usage: &str) -> Result<(String, String)> {
    match (args.get(1), args.get(2)) {
        (Some(a), Some(b)) => Ok((a.clone(), b.clone())),
        _ => bail!("usage: fadewire {usage}"),
    }
}
