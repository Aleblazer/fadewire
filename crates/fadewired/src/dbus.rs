//! The daemon's D-Bus service: `xyz.splitlogic.FadeWire` on the session bus,
//! object `/xyz/splitlogic/FadeWire`, interface `xyz.splitlogic.FadeWire1`.
//!
//! Methods do a request/reply round-trip over the engine's channel, so all
//! fader state stays owned by the engine thread (single writer). The
//! `fadewire` CLI is the reference client; compositor keybinds and bar
//! modules are the intended consumers.

use std::sync::mpsc::{channel, Sender};
use std::time::Duration;

use zbus::blocking::connection::{Builder, Connection};
use zbus::fdo;

use crate::engine::{CtlRequest, EngineMsg};

pub const BUS_NAME: &str = "xyz.splitlogic.FadeWire";
pub const OBJECT_PATH: &str = "/xyz/splitlogic/FadeWire";

const REPLY_TIMEOUT: Duration = Duration::from_secs(3);

pub struct FadeWireService {
    tx: Sender<EngineMsg>,
}

impl FadeWireService {
    fn ask<T>(&self, make: impl FnOnce(Sender<T>) -> CtlRequest) -> fdo::Result<T> {
        let (rtx, rrx) = channel();
        self.tx
            .send(EngineMsg::Ctl(make(rtx)))
            .map_err(|_| engine_gone())?;
        rrx.recv_timeout(REPLY_TIMEOUT).map_err(|_| engine_gone())
    }
}

fn engine_gone() -> fdo::Error {
    fdo::Error::Failed("the daemon engine is not responding".into())
}

fn failed(msg: String) -> fdo::Error {
    fdo::Error::Failed(msg)
}

#[zbus::interface(name = "xyz.splitlogic.FadeWire1")]
impl FadeWireService {
    /// Fader-device connection state: (connected, human-readable message).
    fn status(&self) -> fdo::Result<(bool, String)> {
        self.ask(|reply| CtlRequest::Status { reply })
    }

    /// All configured faders:
    /// (index, label, kind, level-percent or -1 if unknown, target).
    fn faders(&self) -> fdo::Result<Vec<(u32, String, String, i32, String)>> {
        self.ask(|reply| CtlRequest::Faders { reply })
    }

    /// Set a virtual fader's level (0..=100). Returns the applied level.
    fn set_level(&self, fader: &str, pct: u32) -> fdo::Result<u32> {
        self.ask(|reply| CtlRequest::SetLevel {
            label: fader.to_string(),
            pct,
            reply,
        })?
        .map_err(failed)
    }

    /// Step a virtual fader by a signed delta. Returns the new level.
    fn nudge(&self, fader: &str, delta: i32) -> fdo::Result<u32> {
        self.ask(|reply| CtlRequest::Nudge {
            label: fader.to_string(),
            delta,
            reply,
        })?
        .map_err(failed)
    }

    /// Toggle a virtual fader's mute. Returns the new muted state.
    fn toggle_mute(&self, fader: &str) -> fdo::Result<bool> {
        self.ask(|reply| CtlRequest::ToggleMute {
            label: fader.to_string(),
            reply,
        })?
        .map_err(failed)
    }
}

/// Claim the bus name and serve. The returned connection must be kept alive
/// for the daemon's lifetime.
pub fn serve(tx: Sender<EngineMsg>) -> zbus::Result<Connection> {
    Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, FadeWireService { tx })?
        .build()
}
