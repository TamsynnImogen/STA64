//! One peer of a lockstep game: a [`Sim`] stepped through a
//! [`LockstepRouter`], and the replay that is the desync report
//! (docs/NET.md, section 7).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_content::{ContentDb, MapDef};
use sta_core::{PlayerId, Tick};
use sta_sim::command::{Command, CommandRouter};
use sta_sim::replay::Replay;
use sta_sim::setup::GameSetup;
use sta_sim::{SimEvent, WorldSource};

use super::router::LockstepRouter;
use super::transport::Transport;
use crate::proto::{EngineId, Occupant, ProtoError, Start, PROTOCOL_VERSION};
use sta_sim::Sim;

/// What one [`Peer::step`] did.
#[derive(Debug)]
pub enum StepOutcome {
    /// The sim advanced one tick, to `tick`.
    Ran { tick: Tick, events: Vec<SimEvent> },
    /// The next turn is not complete (or a pause holds it); nothing changed.
    Stalled { waiting: Vec<PlayerId> },
    /// A desync stopped the match; call [`Peer::write_desync_report`].
    Halted,
}

/// A fault injected into one peer to test desync detection: the peer's sim
/// then differs from the others' although the same turns were released.
#[derive(Debug, Clone, PartialEq)]
pub enum Fault {
    /// Drops the command `(player, seq)` just before it reaches the sim; the
    /// replay logs what the sim was given.
    DropCommand { player: PlayerId, seq: u32 },
    /// Adds a command that only this peer applies, on its next step.
    Inject(Command),
}

/// Where a desync report was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesyncFiles {
    /// The replay (`sta_sim::replay` format), with a digest every turn.
    pub replay: PathBuf,
    /// `<replay>.desync.txt`.
    pub sidecar: PathBuf,
}

/// A sim, its router and its replay log.
pub struct Peer {
    sim: Sim,
    router: LockstepRouter,
    replay: Replay,
    start: Start,
    engines: Vec<(PlayerId, EngineId)>,
    tick: Tick,
    ticks_per_turn: Tick,
    digested: Option<Tick>,
    faults: Vec<Fault>,
}

impl Peer {
    /// A peer for seat `me` of the game `start` describes, built from the
    /// peer's own content and map (maps are not sent; docs/NET.md, section 6).
    /// The sim is `Sim::from_setup`; use [`Peer::with_sim`] to supply one
    /// with starting forces.
    pub fn start(
        start: &Start,
        me: PlayerId,
        content: Arc<ContentDb>,
        map: Arc<MapDef>,
        transport: Box<dyn Transport>,
    ) -> Result<Peer, ProtoError> {
        let setup = GameSetup {
            seed: start.seed,
            map,
            content,
            players: start.player_setups(me),
            options: start.options.clone(),
        };
        let sim = Sim::from_setup(&setup);
        Peer::with_sim(start, &setup, sim, me, transport)
    }

    /// As [`Peer::start`], for a sim the caller built from `setup` at tick 0.
    pub fn with_sim(
        start: &Start,
        setup: &GameSetup,
        sim: Sim,
        me: PlayerId,
        transport: Box<dyn Transport>,
    ) -> Result<Peer, ProtoError> {
        let router = LockstepRouter::new(start, me, transport)?;
        let mut replay = Replay::new(setup);
        replay.digests.push((0, sim.digest()));
        Ok(Peer {
            sim,
            router,
            replay,
            start: start.clone(),
            engines: Vec::new(),
            tick: 0,
            ticks_per_turn: Tick::from(start.turn.ticks_per_turn.max(1)),
            digested: None,
            faults: Vec::new(),
        })
    }

    /// Records each seat's build identity (from its `Hello`) for the desync
    /// sidecar.
    pub fn set_engines(&mut self, engines: Vec<(PlayerId, EngineId)>) {
        self.engines = engines;
    }

    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    pub fn router(&self) -> &LockstepRouter {
        &self.router
    }

    pub fn router_mut(&mut self) -> &mut LockstepRouter {
        &mut self.router
    }

    /// The sim's tick.
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// Injects a fault (see [`Fault`]); for tests of desync detection.
    pub fn inject(&mut self, fault: Fault) {
        self.faults.push(fault);
    }

    /// Submits a command; it applies `delay_turns` turns on.
    pub fn submit(&mut self, cmd: Command) {
        self.router.submit(cmd);
    }

    /// Runs one tick if the router releases it.
    pub fn step(&mut self) -> StepOutcome {
        if self.tick.is_multiple_of(self.ticks_per_turn) && self.digested != Some(self.tick) {
            self.digested = Some(self.tick);
            self.router.set_digest(self.tick, self.sim.digest());
        }
        let Some(mut cmds) = self.router.take_ready(self.tick) else {
            return if self.router.halted() {
                StepOutcome::Halted
            } else {
                StepOutcome::Stalled {
                    waiting: self.router.waiting_on(),
                }
            };
        };
        self.apply_faults(&mut cmds);
        let before = self.tick;
        let out = self.sim.step(&cmds);
        self.tick = out.view.tick;
        self.replay
            .log_step(before, &cmds, self.tick, out.view.digest, self.ticks_per_turn);
        StepOutcome::Ran {
            tick: self.tick,
            events: out.events,
        }
    }

    fn apply_faults(&mut self, cmds: &mut Vec<Command>) {
        if self.faults.is_empty() {
            return;
        }
        let faults = std::mem::take(&mut self.faults);
        let mut rest = Vec::new();
        for f in faults {
            match f {
                Fault::Inject(c) => cmds.push(c),
                Fault::DropCommand { player, seq } => {
                    let before = cmds.len();
                    cmds.retain(|c| (c.player, c.seq) != (player, seq));
                    if cmds.len() == before {
                        // Not released yet: keep waiting for it.
                        rest.push(Fault::DropCommand { player, seq });
                    }
                }
            }
        }
        self.faults = rest;
    }

    /// The replay so far: the setup, every command applied and the digest of
    /// every turn's first tick.
    pub fn replay(&self) -> &Replay {
        &self.replay
    }

    /// The text of the sidecar (docs/NET.md, section 7).
    pub fn desync_sidecar(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "STA64 desync report");
        let _ = writeln!(s, "protocol version: {PROTOCOL_VERSION}");
        let _ = writeln!(s, "this seat: {}", self.router.me().0);
        let _ = writeln!(s, "replay ends at tick: {}", self.replay.end_tick);
        let cfg = self.router.config();
        let _ = writeln!(
            s,
            "turn: {} ticks, delay {} turns",
            cfg.ticks_per_turn, cfg.delay_turns
        );
        let tick = self
            .router
            .desync()
            .map(|n| n.theirs.tick)
            .or_else(|| self.router.peer_desync().map(|(_, n)| n.ours.tick));
        if let Some(tick) = tick {
            let _ = writeln!(s, "first differing tick: {tick}");
            for (seat, d) in self.router.digests_at(tick) {
                let _ = writeln!(s, "digest at that tick, seat {}: {d:#018x}", seat.0);
            }
        }
        if let Some(n) = self.router.desync() {
            let _ = writeln!(
                s,
                "found against seat {}: ours {:#018x}, theirs {:#018x}",
                n.slot.0, n.ours.digest, n.theirs.digest
            );
        }
        if let Some((from, n)) = self.router.peer_desync() {
            let _ = writeln!(
                s,
                "reported by seat {}: tick {}, its {:#018x}, against seat {} {:#018x}",
                from.0, n.ours.tick, n.ours.digest, n.slot.0, n.theirs.digest
            );
        }
        for seat in &self.start.seats {
            let who = match &seat.occupant {
                Occupant::Peer => "peer",
                Occupant::Ai(_) => "ai",
                Occupant::Idle => "idle",
            };
            let _ = write!(
                s,
                "seat {}: {who}, lineage {:?}, packs [",
                seat.slot.0, seat.lineage
            );
            for (i, p) in seat.packs.iter().enumerate() {
                let _ = write!(s, "{}{} {}", if i > 0 { ", " } else { "" }, p.name, p.version);
            }
            let _ = writeln!(s, "]");
        }
        for (seat, e) in &self.engines {
            let _ = writeln!(s, "engine, seat {}: {} {} {}", seat.0, e.name, e.version, e.build);
        }
        s
    }

    /// Writes `<stem>.star` (the replay) and `<stem>.star.desync.txt` into
    /// `dir`.
    pub fn write_desync_report(&self, dir: &Path, stem: &str) -> std::io::Result<DesyncFiles> {
        let replay = dir.join(format!("{stem}.star"));
        let sidecar = dir.join(format!("{stem}.star.desync.txt"));
        std::fs::write(&replay, self.replay.encode())?;
        std::fs::write(&sidecar, self.desync_sidecar())?;
        Ok(DesyncFiles { replay, sidecar })
    }
}
