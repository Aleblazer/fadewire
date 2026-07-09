//! The daemon's drive loop: snapshot the audio world once a second, resolve
//! each fader's targets, and push levels to nodes it hasn't driven yet.
//!
//! Volumes are only written when a node is *newly seen* for a fader (or the
//! fader's level changes), never re-asserted every tick — so a manual tweak
//! in pavucontrol isn't fought, while a freshly launched app still snaps to
//! its fader's level (the "catch new apps" behaviour of the Windows app).
//!
//! Event subscriptions (instead of polling) come with the native PipeWire
//! backend; 1 Hz polling matches the Windows sibling and is plenty.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::Result;
use fadewire_core::config::{Config, FaderKind};
use fadewire_core::mixer::resolve_targets;

use crate::pulse::Pulse;

/// Nodes a fader has already driven: (fader index, is_stream, node index).
type Driven = HashSet<(usize, bool, u32)>;

pub fn run(cfg: Config) -> Result<()> {
    let mut pulse = Pulse::connect()?;

    // Virtual faders start at their persisted value; physical faders stay
    // inactive until the hidraw milestone gives them a position.
    let levels: Vec<Option<u32>> = cfg
        .fader
        .iter()
        .map(|f| match f.kind {
            FaderKind::Virtual => Some(f.value.clamp(0, 100) as u32),
            FaderKind::Physical => None,
        })
        .collect();

    let active = levels.iter().flatten().count();
    println!(
        "fadewired: connected — driving {active} virtual fader(s); physical faders await the hidraw milestone"
    );

    let mut driven: Driven = HashSet::new();
    loop {
        tick(&cfg, &levels, &mut pulse, &mut driven)?;
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn tick(cfg: &Config, levels: &[Option<u32>], pulse: &mut Pulse, driven: &mut Driven) -> Result<()> {
    let world = pulse.snapshot()?;

    // Forget nodes that vanished so a restarted app gets re-driven.
    driven.retain(|&(_, is_stream, idx)| {
        if is_stream {
            world.streams.iter().any(|s| s.index == idx)
        } else {
            world.sinks.iter().any(|s| s.index == idx)
        }
    });

    for (i, fader) in cfg.fader.iter().enumerate() {
        let Some(pct) = levels[i] else { continue };
        let resolved = resolve_targets(cfg, i, &world);
        for s in resolved.sinks {
            if driven.insert((i, false, s.index)) {
                pulse.set_sink_volume(s.index, s.channels, pct)?;
                println!(
                    "fadewired: \"{}\" -> sink {} ({}) = {pct}%",
                    fader.label, s.index, s.description
                );
            }
        }
        for s in resolved.streams {
            if driven.insert((i, true, s.index)) {
                pulse.set_stream_volume(s.index, s.channels, pct)?;
                println!(
                    "fadewired: \"{}\" -> stream {} ({}) = {pct}%",
                    fader.label, s.index, s.binary
                );
            }
        }
    }
    Ok(())
}

/// One-shot: apply a level to one fader's current targets, print, and exit.
/// The manual verification path until the D-Bus service lands.
pub fn set_once(cfg: &Config, label: &str, pct: u32) -> Result<()> {
    let idx = cfg
        .fader
        .iter()
        .position(|f| f.label.eq_ignore_ascii_case(label))
        .ok_or_else(|| anyhow::anyhow!("no fader labelled \"{label}\" in the config"))?;

    let mut pulse = Pulse::connect()?;
    let world = pulse.snapshot()?;
    let resolved = resolve_targets(cfg, idx, &world);
    if resolved.sinks.is_empty() && resolved.streams.is_empty() {
        println!("\"{label}\": no matching sinks or streams right now");
        return Ok(());
    }
    for s in resolved.sinks {
        pulse.set_sink_volume(s.index, s.channels, pct)?;
        println!("sink {} ({}) = {pct}%", s.index, s.description);
    }
    for s in resolved.streams {
        pulse.set_stream_volume(s.index, s.channels, pct)?;
        println!("stream {} ({}) = {pct}%", s.index, s.binary);
    }
    Ok(())
}

/// Print the audio world as FadeWire sees it (debug/verification).
pub fn list() -> Result<()> {
    let mut pulse = Pulse::connect()?;
    let world = pulse.snapshot()?;
    println!("sinks:");
    for s in &world.sinks {
        println!("  [{:>3}] {:>3}%  {}  ({})", s.index, s.volume_pct, s.description, s.name);
    }
    println!("streams:");
    for s in &world.streams {
        println!("  [{:>3}] {:>3}%  {}  ({})", s.index, s.volume_pct, s.binary, s.app_name);
    }
    Ok(())
}
