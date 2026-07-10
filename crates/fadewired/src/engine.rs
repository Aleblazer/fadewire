//! The daemon's drive loop.
//!
//! One thread owns all fader state and the audio connection; three inputs
//! feed it over a single channel:
//! - **HID events**: each physical fader's raw axis runs through the ported
//!   signal path (`AxisFilter`: snap-band EMA, taper, mute detent, cap,
//!   hysteresis); a changed applied % is pushed to all resolved targets
//!   immediately.
//! - **Control requests** (from the D-Bus service): set/nudge/mute virtual
//!   faders, list state. Virtual levels persist to the state file.
//! - **A 1 Hz snapshot tick**: refreshes the sink/stream world and pushes
//!   levels only to *newly seen* nodes — so a manual tweak in pavucontrol
//!   isn't fought, while a freshly launched app still snaps to its fader's
//!   level.

use std::collections::HashSet;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use fadewire_core::config::{Config, FaderKind};
use fadewire_core::filter::AxisFilter;
use fadewire_core::mixer::{resolve_targets, World};
use fadewire_core::state::{State, VirtualLevel};

use crate::hid::{self, HidEvent};
use crate::paths;
use crate::pulse::Pulse;

/// Everything that can wake the engine.
pub enum EngineMsg {
    Hid(HidEvent),
    Ctl(CtlRequest),
}

/// Requests from the D-Bus service; each carries its reply channel.
pub enum CtlRequest {
    Status {
        reply: Sender<(bool, String)>,
    },
    Faders {
        reply: Sender<Vec<(u32, String, String, i32, String)>>,
    },
    SetLevel {
        label: String,
        pct: u32,
        reply: Sender<Result<u32, String>>,
    },
    Nudge {
        label: String,
        delta: i32,
        reply: Sender<Result<u32, String>>,
    },
    ToggleMute {
        label: String,
        reply: Sender<Result<bool, String>>,
    },
}

/// Nodes a fader has already driven: (fader index, is_stream, node index).
type Driven = HashSet<(usize, bool, u32)>;

/// Live state per configured fader.
struct FaderState {
    /// Current applied level. Virtual faders start at their persisted value;
    /// physical faders are None until their first HID report.
    level: Option<u32>,
    filter: AxisFilter,
    curve: Vec<(i32, i32)>,
    /// Virtual-fader mute (physical faders mute via their dead zone).
    muted: bool,
    pre_mute: u32,
}

pub fn run(cfg: Config) -> Result<()> {
    let mut pulse = Pulse::connect()?;
    println!("fadewired: connected to the audio server");

    let (tx, rx) = mpsc::channel::<EngineMsg>();

    // HID reader, forwarded onto the engine channel.
    {
        let (hid_tx, hid_rx) = mpsc::channel();
        hid::spawn(hid_tx);
        let tx = tx.clone();
        std::thread::spawn(move || {
            for ev in hid_rx {
                if tx.send(EngineMsg::Hid(ev)).is_err() {
                    break;
                }
            }
        });
    }

    // D-Bus service. Its absence (bare TTY session, no session bus) degrades
    // to hardware-only operation rather than failing the daemon.
    let _dbus = match crate::dbus::serve(tx.clone()) {
        Ok(conn) => {
            println!("fadewired: D-Bus service ready ({})", crate::dbus::BUS_NAME);
            Some(conn)
        }
        Err(e) => {
            eprintln!("fadewired: D-Bus unavailable ({e}) — CLI control disabled");
            None
        }
    };

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
            muted: false,
            pre_mute: f.value.clamp(0, 100) as u32,
        })
        .collect();

    // Restore persisted virtual levels (they override the config's value).
    if let Ok(saved) = State::load(&paths::state()) {
        for (i, f) in cfg.fader.iter().enumerate() {
            if f.kind != FaderKind::Virtual {
                continue;
            }
            if let Some(vs) = saved.fader.iter().find(|s| s.label.eq_ignore_ascii_case(&f.label)) {
                faders[i].level = Some(vs.level.clamp(0, 100) as u32);
                faders[i].muted = vs.muted;
                faders[i].pre_mute = vs.pre_mute.clamp(0, 100) as u32;
            }
        }
    }

    let mut driven: Driven = HashSet::new();
    let mut world = pulse.snapshot()?;
    let mut last_snapshot = Instant::now();
    let mut hid_status = (false, String::from("starting…"));
    let mut dirty = false;

    // Drive the initial world (virtual faders' restored levels).
    for i in 0..cfg.fader.len() {
        if let Some(pct) = faders[i].level {
            drive_fader(&cfg, i, pct, &world, &mut pulse, &mut driven, false)?;
        }
    }

    loop {
        match rx.recv_timeout(Duration::from_millis(1000)) {
            Ok(EngineMsg::Hid(HidEvent::Axes { axes, count })) => {
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
            Ok(EngineMsg::Hid(HidEvent::Status { connected, message })) => {
                println!("fadewired: {message}");
                hid_status = (connected, message);
            }
            Ok(EngineMsg::Ctl(req)) => handle_ctl(
                req,
                &cfg,
                &mut faders,
                &mut pulse,
                &world,
                &mut driven,
                &hid_status,
                &mut dirty,
            )?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => bail!("engine channel closed unexpectedly"),
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
            // Persist virtual levels at most once per tick.
            if dirty {
                if let Err(e) = build_state(&cfg, &faders).save(&paths::state()) {
                    eprintln!("fadewired: couldn't save state: {e}");
                }
                dirty = false;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_ctl(
    req: CtlRequest,
    cfg: &Config,
    faders: &mut [FaderState],
    pulse: &mut Pulse,
    world: &World,
    driven: &mut Driven,
    hid_status: &(bool, String),
    dirty: &mut bool,
) -> Result<()> {
    match req {
        CtlRequest::Status { reply } => {
            let _ = reply.send(hid_status.clone());
        }
        CtlRequest::Faders { reply } => {
            let rows = cfg
                .fader
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    (
                        i as u32,
                        f.label.clone(),
                        match f.kind {
                            FaderKind::Physical => "physical".to_string(),
                            FaderKind::Virtual => "virtual".to_string(),
                        },
                        faders[i].level.map(|v| v as i32).unwrap_or(-1),
                        describe_target(cfg, i),
                    )
                })
                .collect();
            let _ = reply.send(rows);
        }
        CtlRequest::SetLevel { label, pct, reply } => {
            let result = with_virtual(cfg, faders, &label, |st| {
                st.muted = false;
                st.level = Some(pct.min(100));
                pct.min(100)
            });
            finish_change(&result, cfg, faders, pulse, world, driven, dirty)?;
            let _ = reply.send(result.map(|(_, v)| v));
        }
        CtlRequest::Nudge { label, delta, reply } => {
            let result = with_virtual(cfg, faders, &label, |st| {
                st.muted = false;
                let cur = st.level.unwrap_or(50) as i32;
                let new = (cur + delta).clamp(0, 100) as u32;
                st.level = Some(new);
                new
            });
            finish_change(&result, cfg, faders, pulse, world, driven, dirty)?;
            let _ = reply.send(result.map(|(_, v)| v));
        }
        CtlRequest::ToggleMute { label, reply } => {
            let result = with_virtual(cfg, faders, &label, |st| {
                if st.muted {
                    st.muted = false;
                    st.level = Some(st.pre_mute);
                } else {
                    st.pre_mute = st.level.unwrap_or(50);
                    st.muted = true;
                    st.level = Some(0);
                }
                u32::from(st.muted)
            });
            finish_change(&result, cfg, faders, pulse, world, driven, dirty)?;
            let _ = reply.send(result.map(|(_, v)| v != 0));
        }
    }
    Ok(())
}

/// Find a *virtual* fader by label and apply a mutation, returning its index
/// and the mutation's value.
fn with_virtual<T>(
    cfg: &Config,
    faders: &mut [FaderState],
    label: &str,
    mutate: impl FnOnce(&mut FaderState) -> T,
) -> Result<(usize, T), String> {
    let Some(i) = cfg
        .fader
        .iter()
        .position(|f| f.label.eq_ignore_ascii_case(label))
    else {
        return Err(format!("no fader labelled \"{label}\""));
    };
    if cfg.fader[i].kind != FaderKind::Virtual {
        return Err(format!(
            "\"{label}\" is a physical fader — its hardware position sets the level"
        ));
    }
    Ok((i, mutate(&mut faders[i])))
}

/// After a successful virtual-fader change: push the new level and mark the
/// state dirty for the next persistence tick.
fn finish_change<T>(
    result: &Result<(usize, T), String>,
    cfg: &Config,
    faders: &[FaderState],
    pulse: &mut Pulse,
    world: &World,
    driven: &mut Driven,
    dirty: &mut bool,
) -> Result<()> {
    if let Ok((i, _)) = result {
        if let Some(pct) = faders[*i].level {
            drive_fader(cfg, *i, pct, world, pulse, driven, true)?;
        }
        *dirty = true;
    }
    Ok(())
}

fn describe_target(cfg: &Config, i: usize) -> String {
    use fadewire_core::config::Target;
    match &cfg.fader[i].target {
        Target::Sink { name_match } => format!("sink:{name_match}"),
        Target::App { binary } => format!("app:{binary}"),
        Target::Category { name } => format!("category:{name}"),
        Target::Unassigned => "everything-else".to_string(),
    }
}

fn build_state(cfg: &Config, faders: &[FaderState]) -> State {
    State {
        fader: cfg
            .fader
            .iter()
            .zip(faders)
            .filter(|(f, _)| f.kind == FaderKind::Virtual)
            .map(|(f, st)| VirtualLevel {
                label: f.label.clone(),
                level: st.level.unwrap_or(50) as i32,
                muted: st.muted,
                pre_mute: st.pre_mute as i32,
            })
            .collect(),
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
/// Works without a running daemon (talks to the audio server directly).
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
