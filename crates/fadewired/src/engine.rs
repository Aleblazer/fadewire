//! The daemon's drive loop.
//!
//! Two inputs, one loop:
//! - **HID events** (mpsc from the reader thread): each physical fader's raw
//!   axis runs through the ported signal path (`AxisFilter`: snap-band EMA,
//!   taper, mute detent, cap, hysteresis); when its applied % changes, the
//!   new level is pushed to all of its resolved targets immediately.
//! - **A 1 Hz world snapshot**: refreshes the sink/stream list and pushes
//!   levels only to *newly seen* nodes — so a manual tweak in pavucontrol
//!   isn't fought, while a freshly launched app still snaps to its fader's
//!   level (the "catch new apps" behaviour of the Windows app).

use std::collections::HashSet;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use fadewire_core::config::{Config, FaderKind};
use fadewire_core::filter::AxisFilter;
use fadewire_core::mixer::{resolve_targets, World};

use crate::hid::{self, HidEvent};
use crate::pulse::Pulse;

/// Nodes a fader has already driven: (fader index, is_stream, node index).
type Driven = HashSet<(usize, bool, u32)>;

/// Live state per configured fader.
struct FaderState {
    /// Current applied level. Virtual faders start at their persisted value;
    /// physical faders are None until their first HID report.
    level: Option<u32>,
    filter: AxisFilter,
    curve: Vec<(i32, i32)>,
}

pub fn run(cfg: Config) -> Result<()> {
    let mut pulse = Pulse::connect()?;
    println!("fadewired: connected to the audio server");

    let (tx, rx) = mpsc::channel();
    hid::spawn(tx);

    let mut faders: Vec<FaderState> = cfg
        .fader
        .iter()
        .map(|f| FaderState {
            level: match f.kind {
                FaderKind::Virtual => Some(f.value.clamp(0, 100) as u32),
                FaderKind::Physical => None,
            },
            filter: AxisFilter::new(),
            curve: f.calibration.build_curve(),
        })
        .collect();

    let mut driven: Driven = HashSet::new();
    let mut world = pulse.snapshot()?;
    let mut last_snapshot = Instant::now();

    // Drive the initial world (virtual faders' persisted levels).
    for i in 0..cfg.fader.len() {
        if let Some(pct) = faders[i].level {
            drive_fader(&cfg, i, pct, &world, &mut pulse, &mut driven, false)?;
        }
    }

    loop {
        match rx.recv_timeout(Duration::from_millis(1000)) {
            Ok(HidEvent::Axes { axes, count }) => {
                for (i, f) in cfg.fader.iter().enumerate() {
                    if f.kind != FaderKind::Physical {
                        continue;
                    }
                    let axis = f.axis;
                    if axis < 0 || axis as usize >= count {
                        continue;
                    }
                    let st = &mut faders[i];
                    let applied = st.filter.update(
                        axes[axis as usize],
                        &f.calibration,
                        &st.curve,
                        f.max_percent.clamp(1, 100),
                    ) as u32;
                    if st.level != Some(applied) {
                        st.level = Some(applied);
                        // Push to every current target now — fader moves must
                        // feel instant, not wait for the next snapshot tick.
                        drive_fader(&cfg, i, applied, &world, &mut pulse, &mut driven, true)?;
                    }
                }
            }
            Ok(HidEvent::Status { message, .. }) => println!("fadewired: {message}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => bail!("hid thread exited unexpectedly"),
        }

        if last_snapshot.elapsed() >= Duration::from_secs(1) {
            world = pulse.snapshot()?;
            last_snapshot = Instant::now();
            // Forget nodes that vanished so a restarted app gets re-driven.
            driven.retain(|&(_, is_stream, idx)| {
                if is_stream {
                    world.streams.iter().any(|s| s.index == idx)
                } else {
                    world.sinks.iter().any(|s| s.index == idx)
                }
            });
            for i in 0..cfg.fader.len() {
                if let Some(pct) = faders[i].level {
                    drive_fader(&cfg, i, pct, &world, &mut pulse, &mut driven, false)?;
                }
            }
        }
    }
}

/// Push one fader's level to its resolved targets. With `force`, every target
/// is written (a level change); without, only nodes not yet driven by this
/// fader (a new-node catch-up pass).
fn drive_fader(
    cfg: &Config,
    fader_index: usize,
    pct: u32,
    world: &World,
    pulse: &mut Pulse,
    driven: &mut Driven,
    force: bool,
) -> Result<()> {
    let resolved = resolve_targets(cfg, fader_index, world);
    for s in resolved.sinks {
        let new = driven.insert((fader_index, false, s.index));
        if new || force {
            pulse.set_sink_volume(s.index, s.channels, pct)?;
            if new && !force {
                println!(
                    "fadewired: \"{}\" -> sink {} ({}) = {pct}%",
                    cfg.fader[fader_index].label, s.index, s.description
                );
            }
        }
    }
    for s in resolved.streams {
        let new = driven.insert((fader_index, true, s.index));
        if new || force {
            pulse.set_stream_volume(s.index, s.channels, pct)?;
            if new && !force {
                println!(
                    "fadewired: \"{}\" -> stream {} ({}) = {pct}%",
                    cfg.fader[fader_index].label, s.index, s.binary
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
        println!(
            "  [{:>3}] {:>3}%  {}  ({})",
            s.index, s.volume_pct, s.description, s.name
        );
    }
    println!("streams:");
    for s in &world.streams {
        println!(
            "  [{:>3}] {:>3}%  {}  ({})",
            s.index, s.volume_pct, s.binary, s.app_name
        );
    }
    Ok(())
}

/// Dump raw axis values as reports arrive (hardware debugging aid, like the
/// Windows repo's fader_read.py).
pub fn watch() -> Result<()> {
    let (tx, rx) = mpsc::channel();
    hid::spawn(tx);
    println!("watching for fader reports (Ctrl+C to stop)…");
    loop {
        match rx.recv() {
            Ok(HidEvent::Axes { axes, count }) => {
                let vals: Vec<String> = axes[..count].iter().map(|v| format!("{v:>5}")).collect();
                println!("axes: {}", vals.join(" "));
            }
            Ok(HidEvent::Status { message, .. }) => println!("{message}"),
            Err(_) => bail!("hid thread exited unexpectedly"),
        }
    }
}
