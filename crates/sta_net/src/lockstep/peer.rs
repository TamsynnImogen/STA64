//! One peer of a lockstep game: a [`Sim`] stepped through a
//! [`LockstepRouter`], and the replay that is the desync report
//! (docs/NET.md, section 7).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use sta_content::{ContentDb, MapDef};
use sta_core::{PlayerId, Tick};
use sta_sim::command::{Command, CommandRouter};
use sta_sim::replay::{Replay, StartRec};
use sta_sim::digest::SectionDigests;
use sta_sim::setup::GameSetup;
use sta_sim::{SimEvent, WorldSource};

use super::router::LockstepRouter;
use crate::resync::NetState;
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
    pub(crate) sim: Sim,
    pub(crate) router: LockstepRouter,
    pub(crate) replay: Replay,
    pub(crate) start: Start,
    engines: Vec<(PlayerId, EngineId)>,
    pub(crate) tick: Tick,
    pub(crate) ticks_per_turn: Tick,
    pub(crate) digested: Option<Tick>,
    faults: Vec<Fault>,
    /// This peer's own content (a snapshot is checked against it).
    pub(crate) content: Arc<ContentDb>,
    /// NET-04: rejoin and resync state.
    pub(crate) net: NetState,
    /// The tick the replay starts at: 0, or the snapshot's after a rejoin
    /// (a replay with no commands before it cannot be rerun from tick 0).
    pub(crate) replay_from: Tick,
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
        Peer::with_router(start, setup, sim, router)
    }

    /// A referee's peer (NET-05, NET_DECISIONS N5) on the connection in slot
    /// `me`, which is no seat of `start`: it runs the whole game from `Start`
    /// and the others' turn records on its own content, sends no record, and
    /// never stops for a desync (see `LockstepRouter::spectator`). Wrap it in
    /// a [`Referee`](crate::referee::Referee) for the verdicts and reports.
    pub fn spectator(
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
        let router = LockstepRouter::spectator(start, me, transport)?;
        Peer::with_router(start, &setup, sim, router)
    }

    fn with_router(
        start: &Start,
        setup: &GameSetup,
        sim: Sim,
        router: LockstepRouter,
    ) -> Result<Peer, ProtoError> {
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
            content: Arc::clone(&setup.content),
            net: NetState::default(),
            replay_from: 0,
        })
    }

    /// Records how the sim's seats got their starting forces in the replay
    /// (`StartRec::Races` after `Sim::from_setup_with_starts`), so a desync
    /// report is rebuilt the way the game was (INT-M5). The default is
    /// `StartRec::Bare`, which is what [`Peer::start`] builds.
    pub fn set_starts(&mut self, starts: StartRec) {
        self.replay.starts = starts;
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
        self.step_at(Instant::now())
    }

    /// [`Peer::step`] at a given time: the clock of rejoin and resync
    /// deadlines and rate limits (tests).
    pub fn step_at(&mut self, now: Instant) -> StepOutcome {
        self.router.pump();
        if let Some(out) = self.net_pre(now) {
            return out;
        }
        if self.tick.is_multiple_of(self.ticks_per_turn) && self.digested != Some(self.tick) {
            self.digested = Some(self.tick);
            let sections = self.sim.section_digests();
            self.router.set_sections(self.tick, sections.values().to_vec());
            self.router.set_digest(self.tick, self.sim.digest());
        }
        if self.tick.is_multiple_of(self.ticks_per_turn) {
            self.net_boundary(now);
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
        for line in self.net.sidecar_lines() {
            let _ = writeln!(s, "{line}");
        }
        if self.replay_from > 0 {
            let _ = writeln!(
                s,
                "replay starts at tick {} (joined by snapshot): it holds no commands before it",
                self.replay_from
            );
        }
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
            .or_else(|| self.router.peer_desync().map(|(_, n)| n.ours.tick))
            .or_else(|| self.router.divergences().iter().map(|d| d.theirs.tick).min());
        if let Some(tick) = tick {
            let _ = writeln!(s, "first differing tick: {tick}");
            for (seat, d) in self.router.digests_at(tick) {
                let _ = writeln!(s, "digest at that tick, seat {}: {d:#018x}", seat.0);
            }
        }
        if let Some(tick) = tick {
            self.sections_report(&mut s, tick);
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
        for d in self.router.divergences() {
            let _ = writeln!(
                s,
                "referee: seat {} reported {:#018x} at tick {}, the referee's is {:#018x}",
                d.seat.0, d.theirs.digest, d.theirs.tick, d.reference
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

    /// The section digests known for `tick` and the sections that differ
    /// (NET_DECISIONS N6). Lines, in seat order:
    /// `section digests at that tick, seat N: roster=0x.., movement=0x.., ...`
    /// and `first differing sections, seat A against seat B: movement, combat`
    /// (A is this seat, B another that sent its digests).
    fn sections_report(&self, s: &mut String, tick: Tick) {
        let name = |i: usize| SectionDigests::NAMES.get(i).map_or_else(|| format!("section#{i}"), |n| (*n).to_owned());
        let known = self.router.sections_at(tick);
        for (seat, d) in &known {
            let list: Vec<String> = d.iter().enumerate().map(|(i, v)| format!("{}={v:#018x}", name(i))).collect();
            let _ = writeln!(s, "section digests at that tick, seat {}: {}", seat.0, list.join(", "));
        }
        let me = self.router.me();
        let Some((_, ours)) = known.iter().find(|(p, _)| *p == me) else {
            return;
        };
        for (seat, theirs) in known.iter().filter(|(p, _)| *p != me) {
            let n = ours.len().max(theirs.len());
            let differing: Vec<String> = (0..n).filter(|&i| ours.get(i) != theirs.get(i)).map(name).collect();
            let text = if differing.is_empty() { "none".to_owned() } else { differing.join(", ") };
            let _ = writeln!(s, "first differing sections, seat {} against seat {}: {text}", me.0, seat.0);
        }
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
