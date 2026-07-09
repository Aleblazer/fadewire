//! Mixer backend over the PulseAudio client API, served by `pipewire-pulse`
//! on every modern distro. Sinks map to output devices, sink-inputs map to
//! per-app streams. A native `pipewire-rs` backend can replace this later
//! without touching the engine (the model lives in `fadewire_core::mixer`).
//!
//! Uses the *standard* (single-threaded, manually iterated) mainloop: all
//! calls happen on the daemon thread, no locking. Volumes are written on the
//! pulse scale (`PA_VOLUME_NORM * pct / 100`); the server applies the
//! perceptual curve.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{anyhow, bail, Result};
use libpulse_binding as pa;
use pa::callbacks::ListResult;
use pa::context::{Context as PaContext, FlagSet as CtxFlagSet, State as CtxState};
use pa::mainloop::standard::{IterateResult, Mainloop};
use pa::operation::{Operation, State as OpState};
use pa::volume::{ChannelVolumes, Volume};

use fadewire_core::mixer::{SinkNode, StreamNode, World};

pub struct Pulse {
    mainloop: Mainloop,
    context: PaContext,
}

impl Pulse {
    /// Connect to the local audio server (PipeWire's pulse socket).
    pub fn connect() -> Result<Self> {
        let mut mainloop = Mainloop::new().ok_or_else(|| anyhow!("creating pulse mainloop"))?;
        let mut context = PaContext::new(&mainloop, "FadeWire")
            .ok_or_else(|| anyhow!("creating pulse context"))?;
        context
            .connect(None, CtxFlagSet::NOFLAGS, None)
            .map_err(|e| anyhow!("connecting to the audio server: {e}"))?;
        loop {
            match mainloop.iterate(true) {
                IterateResult::Success(_) => {}
                IterateResult::Quit(_) => bail!("pulse mainloop quit during connect"),
                IterateResult::Err(e) => bail!("pulse mainloop error during connect: {e}"),
            }
            match context.get_state() {
                CtxState::Ready => break,
                CtxState::Failed | CtxState::Terminated => {
                    bail!("audio server refused the connection (is pipewire-pulse running?)")
                }
                _ => {}
            }
        }
        Ok(Self { mainloop, context })
    }

    /// Pump the mainloop until an operation finishes.
    fn wait<G: ?Sized>(&mut self, op: Operation<G>) -> Result<()> {
        loop {
            match self.mainloop.iterate(true) {
                IterateResult::Success(_) => {}
                IterateResult::Quit(_) => bail!("pulse mainloop quit"),
                IterateResult::Err(e) => bail!("pulse mainloop error: {e}"),
            }
            match op.get_state() {
                OpState::Done => return Ok(()),
                OpState::Cancelled => bail!("pulse operation cancelled"),
                OpState::Running => {}
            }
        }
    }

    /// Snapshot the current sinks and app streams.
    pub fn snapshot(&mut self) -> Result<World> {
        let sinks = Rc::new(RefCell::new(Vec::<SinkNode>::new()));
        {
            let acc = Rc::clone(&sinks);
            let op = self.context.introspect().get_sink_info_list(move |res| {
                if let ListResult::Item(s) = res {
                    acc.borrow_mut().push(SinkNode {
                        index: s.index,
                        name: s.name.as_deref().unwrap_or("").to_string(),
                        description: s.description.as_deref().unwrap_or("").to_string(),
                        channels: s.volume.len(),
                        volume_pct: to_pct(&s.volume),
                    });
                }
            });
            self.wait(op)?;
        }

        let streams = Rc::new(RefCell::new(Vec::<StreamNode>::new()));
        {
            let acc = Rc::clone(&streams);
            let op = self
                .context
                .introspect()
                .get_sink_input_info_list(move |res| {
                    if let ListResult::Item(s) = res {
                        let binary = s
                            .proplist
                            .get_str("application.process.binary")
                            .unwrap_or_default();
                        let app_name = s
                            .proplist
                            .get_str("application.name")
                            .or_else(|| s.name.as_deref().map(str::to_string))
                            .unwrap_or_default();
                        acc.borrow_mut().push(StreamNode {
                            index: s.index,
                            app_name,
                            binary,
                            channels: s.volume.len(),
                            volume_pct: to_pct(&s.volume),
                        });
                    }
                });
            self.wait(op)?;
        }

        Ok(World {
            sinks: sinks.take(),
            streams: streams.take(),
        })
    }

    /// Set an output device's volume (percent of normal).
    pub fn set_sink_volume(&mut self, index: u32, channels: u8, pct: u32) -> Result<()> {
        let cv = volume_for(channels, pct);
        let op = self
            .context
            .introspect()
            .set_sink_volume_by_index(index, &cv, None);
        self.wait(op)
    }

    /// Set one app stream's volume (percent of normal).
    pub fn set_stream_volume(&mut self, index: u32, channels: u8, pct: u32) -> Result<()> {
        let cv = volume_for(channels, pct);
        let op = self
            .context
            .introspect()
            .set_sink_input_volume(index, &cv, None);
        self.wait(op)
    }
}

fn volume_for(channels: u8, pct: u32) -> ChannelVolumes {
    let raw = (Volume::NORMAL.0 as u64 * pct.min(100) as u64 / 100) as u32;
    let mut cv = ChannelVolumes::default();
    cv.set(channels.max(1), Volume(raw));
    cv
}

fn to_pct(cv: &ChannelVolumes) -> u32 {
    (cv.avg().0 as u64 * 100 / Volume::NORMAL.0 as u64) as u32
}
