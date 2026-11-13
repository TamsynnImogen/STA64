//! A match started from the multiplayer lobby, played in lockstep (INT-M5).
//!
//! ```text
//!   Lobby --Launch--> NetGameReady + NetLaunch --on_ready--> StartGame --> Loading
//!        Loading: the map view's `rebuild` builds the map and calls [`build_side`]
//!        (the sim, the same on every peer) and [`link`] (the lockstep peer under a SimLink)
//!   InGame: SimLink -> PeerRouter::take_ready -> Peer::step -> the sim, only ever
//! ```
//!
//! - **One sim per peer, stepped only through its [`Peer`].** The map view's `SimLink` is
//!   built over two adapters that share the peer: [`PeerRouter`] is its
//!   `CommandRouter` (`submit` goes to the lockstep router, `take_ready` runs one
//!   `Peer::step`) and [`PeerSource`] is its `WorldSource` (it hands back what that step
//!   produced). Input, the HUD and the renderers read the `SimLink` as in single player;
//!   the host's AI controllers think after every tick and submit through the same router
//!   as a player (`Control::Ai` seats run here, `Control::RemoteAi` seats do not).
//! - **Same map.** [`check_map`] compares the host's `Start.map.digest` with the map this
//!   peer is about to play on and refuses a start naming the map.
//! - **Duties** ([`net_duties`], every frame): router events (a protocol violation is a
//!   kick, [`protocol_reaction`]), departures (the host waits [`NetOptions::drop_grace`]
//!   for a rejoin, never drops a seat whose rejoin is in progress, then schedules the
//!   drop), resync events (a seat that keeps abusing the host's patience is kicked), the
//!   desync report (written once when the match halts), stalls and endings. What the
//!   player sees is in [`super::net_overlay`].
//! - **Not built**: a seat that is connected but frozen is waited for without end (NET.md
//!   section 8's vote: wait, eliminate, AI takeover), and a deliberate quit is held for the
//!   same grace as a lost connection (no `Leave` is sent or read).
//! - **What a drop does to the dropped player's units is an open design question**
//!   (docs/NET.md section 8, open question 9): this layer keeps NET-02's behaviour. The seat
//!   is closed in the router (`Action::Drop`, recorded as `DropChoice::Eliminate`, the only
//!   non-AI choice) and no command exists to eliminate it, so its units stay as they are and
//!   take no orders. Nothing is eliminated or handed to the AI here.
//!
//! Evidence grades: lockstep, drop and resync behaviour are NET-02/NET-04's (tested there);
//! this module's claims are measured by `net_match_tests` over a relay on 127.0.0.1.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use sta_content::{MapDef, Races};
use sta_core::{PlayerId, Tick};
use sta_net::lockstep::{Fault, Peer, RouterEvent, StepOutcome};
use sta_net::proto::{self, Action, DropChoice, Start};
use sta_net::resync::{ResyncEvent, ResyncLimits};
use sta_net::transport::{CloseReason, NetControl};
use sta_sim::command::{Command, CommandRouter};
use sta_sim::replay::StartRec;
use sta_sim::setup::{Controller, GameSetup};
use sta_sim::{Sim, TickOutput, WorldSource, WorldView};

use super::flow::LeaveMatch;
use super::{Seats, local_race};
use crate::app::{AppState, GameSet};
use crate::frontend::lobby::{Multiplayer, NetGameReady, NetLaunch, NetMatch};
use crate::frontend::screens::{Screen, StartGame};
use crate::sim_link::SimLink;
use crate::ui::notices::{Notice, Severity};

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Timings and places of a networked match. The defaults are the product's; tests shorten
/// them.
#[derive(Resource, Debug, Clone)]
pub struct NetOptions {
    /// Host: replaces the default `ResyncLimits` (tests).
    pub limits: Option<ResyncLimits>,
    /// Where desync reports and the replay of a match that ended go
    /// (default: `<config dir>/net-reports`, else the temp dir).
    pub report_dir: Option<PathBuf>,
    /// A stall shows its "waiting for" lines after this long (NET.md section 8: 3 s).
    pub stall_after: Duration,
    /// Host: how long a seat whose connection ended is kept for a rejoin before it is
    /// dropped. **A named default**: the design leaves the wait open.
    pub drop_grace: Duration,
}

impl Default for NetOptions {
    fn default() -> NetOptions {
        NetOptions { limits: None, report_dir: None, stall_after: Duration::from_secs(3), drop_grace: Duration::from_secs(30) }
    }
}

impl NetOptions {
    fn dir(&self) -> PathBuf {
        self.report_dir.clone().or_else(|| crate::platform::config_dir().map(|d| d.join("net-reports"))).unwrap_or_else(std::env::temp_dir)
    }
}

// ---------------------------------------------------------------------------
// The state of a running match
// ---------------------------------------------------------------------------

/// Why a networked match ended for this player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// The host's connection ended: computer players and missions go with it (NET.md section 8).
    HostLeft,
    /// The relay closed this connection without a word: the host removed this player, or
    /// the relay stopped.
    Removed,
    /// The connection failed.
    ConnectionLost(String),
    /// The host sent something that breaks the protocol.
    HostProtocol(String),
    /// This player left on purpose.
    Left,
}

impl Ending {
    /// In plain words.
    pub fn text(&self) -> String {
        match self {
            Ending::HostLeft => "The host left, so the game is over. The computer players and the mission ran on the host.".into(),
            Ending::Removed => "You were removed from the game by the host, or the relay stopped.".into(),
            Ending::ConnectionLost(why) => format!("The connection to the relay was lost: {why}."),
            Ending::HostProtocol(why) => format!("The host sent something the game cannot accept ({why}); the game is over."),
            Ending::Left => "You left the game.".into(),
        }
    }
}

/// What came of a desync, as far as this peer knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesyncStage {
    /// Stopped; this peer is asking the host for its state, or the host is repairing the others.
    Repairing,
    /// The host's state was loaded here (or the others were repaired): playing on.
    Repaired,
    /// Nothing more will happen: the match stays stopped.
    Stopped(String),
}

/// A desync this peer found or was told of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesyncInfo {
    pub tick: Tick,
    /// The report (`<stem>.star` and `<stem>.star.desync.txt`) if it could be written.
    pub report: Option<PathBuf>,
    pub error: Option<String>,
    pub stage: DesyncStage,
}

/// A dropped seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub last_turn: u32,
}

/// What the layers over the peer record about the match, for the overlay and the tests.
pub struct NetShared {
    peer: Peer,
    out: Option<TickOutput>,
    pub me: PlayerId,
    pub host: PlayerId,
    control: NetControl,
    opts: NetOptions,
    names: BTreeMap<PlayerId, String>,
    /// Host: where the peer's departure source also writes what it took.
    departures: Arc<Mutex<Vec<PlayerId>>>,
    /// Seats whose connection ended and that are not dropped yet, with the time the
    /// marker was read.
    pub departed: BTreeMap<PlayerId, Instant>,
    /// Seats to take out at once (kicked for a protocol violation).
    drop_now: BTreeSet<PlayerId>,
    pub dropped: BTreeMap<PlayerId, Dropped>,
    pub stall: Option<(Instant, Vec<PlayerId>)>,
    pub desync: Option<DesyncInfo>,
    pub ending: Option<Ending>,
    /// Every line said to the player, oldest first (the overlay shows the newest).
    pub said: VecDeque<String>,
    /// Lines not yet turned into notices.
    fresh: Vec<(Severity, String)>,
    /// Seats rejoining or back.
    pub rejoining: BTreeSet<PlayerId>,
    /// Steps that found the next turn incomplete, in all.
    pub stalls: u64,
    /// The first protocol violation per seat (the text the host kicked it for).
    pub violations: BTreeMap<PlayerId, String>,
    /// The replay was written to this path when the match ended without a desync.
    pub replay_path: Option<PathBuf>,
}

impl NetShared {
    fn name(&self, seat: PlayerId) -> String {
        self.display_name(seat)
    }

    /// A seat's name as the lobby showed it, else "Seat n".
    pub fn display_name(&self, seat: PlayerId) -> String {
        self.names.get(&seat).cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Seat {}", seat.0 + 1))
    }

    pub fn opts_stall_after(&self) -> Duration {
        self.opts.stall_after
    }

    pub fn opts_drop_grace(&self) -> Duration {
        self.opts.drop_grace
    }

    fn say(&mut self, sev: Severity, text: String) {
        if self.said.len() >= 50 {
            self.said.pop_front();
        }
        self.said.push_back(text.clone());
        self.fresh.push((sev, text));
    }

    pub fn peer(&self) -> &Peer {
        &self.peer
    }

    pub fn is_host(&self) -> bool {
        self.me == self.host
    }
}

/// The running match: the lockstep peer behind the map view's `SimLink`, and what the
/// overlay and the duties keep about it. Cloning shares it.
#[derive(Resource, Clone)]
pub struct NetGame(Arc<Mutex<NetShared>>);

impl NetGame {
    fn lock(&self) -> MutexGuard<'_, NetShared> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reads the state.
    pub fn read<R>(&self, f: impl FnOnce(&NetShared) -> R) -> R {
        f(&self.lock())
    }

    /// Reaches the peer itself (tests inject faults and read digests through it).
    pub fn with_peer<R>(&self, f: impl FnOnce(&mut Peer) -> R) -> R {
        f(&mut self.lock().peer)
    }

    /// The connection's handle (tests cut or kick through it).
    pub fn control(&self) -> NetControl {
        self.lock().control.clone()
    }

    pub fn inject(&self, fault: Fault) {
        self.with_peer(|p| p.inject(fault));
    }

    pub fn tick(&self) -> Tick {
        self.read(|s| s.peer.tick())
    }

    pub fn digest(&self) -> u64 {
        self.read(|s| s.peer.sim().digest())
    }

    pub fn ending(&self) -> Option<Ending> {
        self.read(|s| s.ending.clone())
    }
}

/// The `CommandRouter` the map view's `SimLink` has: commands go to the lockstep router
/// and a tick runs only when the router releases its turn.
pub struct PeerRouter(NetGame);

/// The `WorldSource` the map view's `SimLink` has: it holds the sim's last step for the
/// link and takes no commands of its own.
pub struct PeerSource(NetGame);

impl CommandRouter for PeerRouter {
    fn submit(&mut self, cmd: Command) {
        let mut s = self.0.lock();
        if s.ending.is_none() {
            s.peer.submit(cmd);
        }
    }

    fn take_ready(&mut self, _tick: Tick) -> Option<Vec<Command>> {
        let mut s = self.0.lock();
        if s.ending.is_some() {
            return None;
        }
        match s.peer.step() {
            StepOutcome::Ran { events, .. } => {
                s.stall = None;
                let view = s.peer.sim().view();
                s.out = Some(TickOutput { view, events });
                // The peer applied the turn's commands itself; the link steps "nothing".
                Some(Vec::new())
            }
            StepOutcome::Stalled { waiting } => {
                s.stalls += 1;
                let since = s.stall.as_ref().map_or_else(Instant::now, |(t, _)| *t);
                s.stall = Some((since, waiting));
                None
            }
            StepOutcome::Halted => {
                // The first halt writes the report, before any repair changes the sim.
                s.note_desync();
                None
            }
        }
    }
}

impl WorldSource for PeerSource {
    fn step(&mut self, _commands: &[Command]) -> TickOutput {
        let mut s = self.0.lock();
        match s.out.take() {
            Some(o) => o,
            None => TickOutput { view: s.peer.sim().view(), events: Vec::new() },
        }
    }

    fn view(&self) -> Arc<WorldView> {
        self.0.lock().peer.sim().view()
    }

    fn snapshot(&self) -> Option<Vec<u8>> {
        Some(self.0.lock().peer.sim().snapshot())
    }
}

// ---------------------------------------------------------------------------
// Building the match
// ---------------------------------------------------------------------------

/// Whether this peer's own copy of the map is the one the host's `Start` names. Refuses
/// a start naming the map, so two peers never play on maps that merely share a name.
pub fn check_map(start: &Start, own: &MapDef) -> Result<(), String> {
    let digest = proto::map_digest(own);
    if own.name.eq_ignore_ascii_case(&start.map.name) && digest == Some(start.map.digest) {
        return Ok(());
    }
    Err(if own.name.eq_ignore_ascii_case(&start.map.name) {
        format!("The map {} on this machine is not the host's copy of it (the files differ). Everyone must play the same map file.", start.map.name)
    } else {
        format!("The host chose the map {}, but this machine loaded {}.", start.map.name, own.name)
    })
}

/// What the sim's side of the match is made of.
pub struct NetSide {
    pub setup: GameSetup,
    pub sim: Sim,
    /// The AI seats this machine runs: the host's `Control::Ai` seats only.
    pub controllers: Vec<Box<dyn Controller>>,
    pub seats: Seats,
}

/// The sim every peer of this match builds: the setup from the host's `Start` over this
/// peer's own `map` (checked against `Start.map.digest`) and content, each seat with its
/// race's starting forces. Computer players: `Control::Ai` seats (this machine is the
/// host) get a controller, `Control::RemoteAi` seats stay AI seats in the sim without one.
pub fn build_side(nm: &NetMatch, own_map: MapDef, races: &Races) -> Result<NetSide, String> {
    check_map(&nm.start, &own_map)?;
    let mut setup = nm.setup.clone();
    setup.map = Arc::new(own_map);
    let (sim, _warnings) = Sim::from_setup_with_starts(&setup, races);
    let controllers = super::controllers(&setup);
    let me = setup.players.iter().find(|p| p.id == nm.me).ok_or("the game has no seat for you")?;
    let local = local_race(races, &me.race);
    let summary = format!("multiplayer: you are seat {} ({}), {} seats, {}", me.id.0, me.race, setup.players.len(), if nm.me == nm.start.host { "you are the host" } else { "the host runs the computer players" });
    let seats = Seats { players: setup.players.clone(), local, summary };
    Ok(NetSide { setup, sim, controllers, seats })
}

/// Puts the lockstep peer under the map view's link: the sim `side` built goes into a
/// `Peer` over the match's transport, and the link steps only through it. Host: serves
/// rejoins and reads departures. Returns the link and the handle the duties and the
/// overlay use.
pub fn link(mut nm: NetMatch, setup: &GameSetup, sim: Sim, races: &Races, controllers: Vec<Box<dyn Controller>>, opts: &NetOptions) -> Result<(SimLink, NetGame), String> {
    let transport = nm.take_transport().ok_or("the match's connection is already taken")?;
    let mut peer = Peer::with_sim(&nm.start, setup, sim, nm.me, transport).map_err(|e| format!("the game could not start: {e}"))?;
    peer.set_engines(nm.engines.clone());
    peer.set_starts(StartRec::Races { loadouts: sta_sim::start::loadouts_hash(setup, races) });
    let departures: Arc<Mutex<Vec<PlayerId>>> = Arc::default();
    let is_host = nm.me == nm.start.host;
    if is_host {
        peer.serve_rejoins(crate::frontend::lobby::engine_id(), nm.tokens.iter().copied());
        if let Some(l) = &opts.limits {
            peer.set_resync_limits(l.clone());
        }
        let (control, seen) = (nm.control.clone(), departures.clone());
        peer.set_departure_source(move || {
            let d = control.take_departures();
            if !d.is_empty() {
                seen.lock().unwrap_or_else(|e| e.into_inner()).extend(d.iter().copied());
            }
            d
        });
    }
    let names = nm.start.seats.iter().map(|s| (s.slot, s.name.clone())).collect();
    let shared = NetShared {
        peer,
        out: None,
        me: nm.me,
        host: nm.start.host,
        control: nm.control.clone(),
        opts: opts.clone(),
        names,
        departures,
        departed: BTreeMap::new(),
        drop_now: BTreeSet::new(),
        dropped: BTreeMap::new(),
        stall: None,
        desync: None,
        ending: None,
        said: VecDeque::new(),
        fresh: Vec::new(),
        rejoining: BTreeSet::new(),
        stalls: 0,
        violations: BTreeMap::new(),
        replay_path: None,
    };
    let game = NetGame(Arc::new(Mutex::new(shared)));
    let link = SimLink::new(Box::new(PeerSource(game.clone())), Box::new(PeerRouter(game.clone())), nm.me).with_controllers(controllers);
    Ok((link, game))
}

// ---------------------------------------------------------------------------
// Starting from the lobby
// ---------------------------------------------------------------------------

/// A launched lobby's match, waiting for the map view to build it (`map::rebuild` takes it).
#[derive(Resource, Default)]
pub struct NetStart(pub Option<NetMatch>);

/// The map view's access to a match waiting to be built (one system parameter).
#[derive(bevy::ecs::system::SystemParam)]
pub struct NetBuild<'w> {
    start: Option<ResMut<'w, NetStart>>,
    failed: Option<ResMut<'w, NetStartFailed>>,
    opts: Option<Res<'w, NetOptions>>,
}

impl NetBuild<'_> {
    /// The waiting match, once.
    pub fn take(&mut self) -> Option<NetMatch> {
        self.start.as_mut().and_then(|s| s.0.take())
    }

    pub fn options(&self) -> NetOptions {
        self.opts.as_deref().cloned().unwrap_or_default()
    }

    /// The match is refused: the room is left and the reason goes to the multiplayer screen.
    pub fn fail(&mut self, nm: NetMatch, why: String) {
        nm.control.leave();
        self.failed_with(why);
    }

    pub fn failed_with(&mut self, why: String) {
        if let Some(f) = self.failed.as_mut() {
            f.0 = Some(why);
        }
    }
}

/// Why a started match could not be built: shown on the multiplayer screen.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct NetStartFailed(pub Option<String>);

/// `NetGameReady`: the lobby launched. The match goes to the front end flow as an ordinary
/// start (loading screen, then the map view builds it), and the multiplayer screen is where
/// the player returns to.
pub fn on_ready(
    mut ready: MessageReader<NetGameReady>,
    mut launch: ResMut<NetLaunch>,
    mut start: ResMut<NetStart>,
    mut starts: MessageWriter<StartGame>,
    mut mp: ResMut<Multiplayer>,
    mut next: ResMut<NextState<Screen>>,
) {
    if ready.read().count() == 0 {
        return;
    }
    let Some(nm) = launch.0.take() else { return };
    starts.write(StartGame(nm.setup.clone()));
    start.0 = Some(nm);
    // The room is the match now; whatever the lobby session still holds is spent.
    mp.leave();
    mp.notice = None;
    next.set(Screen::Multiplayer);
}

/// The match did not start (the map view could not build it, or refused it): back to the
/// multiplayer screen with the reason, and the room is left so the others are not kept waiting.
pub fn on_start_failed(mut failed: ResMut<NetStartFailed>, mut start: ResMut<NetStart>, mut mp: ResMut<Multiplayer>) {
    if let Some(why) = failed.0.take() {
        if let Some(nm) = start.0.take() {
            nm.control.leave();
        }
        mp.notice = Some(why);
    }
}

// ---------------------------------------------------------------------------
// Duties
// ---------------------------------------------------------------------------

/// What a protocol violation by `from` leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    /// Host: the relay closes the seat's connection; it is then dropped like any departed seat.
    Kick(PlayerId),
    /// A peer's violation that this peer cannot act on: said, nothing more (the host sees
    /// the same stream and decides).
    Note,
    /// The host broke the protocol: the match is over for this peer.
    EndMatch,
}

/// **The named policy for `RouterEvent::Protocol`** (the lobby's open question 4): the host
/// kicks the seat that sent it, a peer ends the match when the host did and otherwise only
/// says so. Change the answer here.
pub fn protocol_reaction(me: PlayerId, host: PlayerId, from: PlayerId) -> Reaction {
    if from == me {
        Reaction::Note
    } else if me == host {
        Reaction::Kick(from)
    } else if from == host {
        Reaction::EndMatch
    } else {
        Reaction::Note
    }
}

/// What the desync report is called: the time it was written, the seat and the tick.
fn report_stem(s: &NetShared, tick: Tick) -> String {
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("desync-{when}-seat{}-tick{tick}", s.me.0)
}

fn lost_text(r: &CloseReason) -> Ending {
    match r {
        CloseReason::Left => Ending::Left,
        CloseReason::RelayClosed => Ending::Removed,
        CloseReason::TimedOut => Ending::ConnectionLost("the relay stopped answering".into()),
        CloseReason::Io(e) => Ending::ConnectionLost(e.clone()),
        CloseReason::Protocol(e) => Ending::ConnectionLost(format!("the relay sent something invalid ({e})")),
        CloseReason::InboxFull => Ending::ConnectionLost("the game could not keep up with what arrived".into()),
        CloseReason::SendQueueFull => Ending::ConnectionLost("the relay could not take what was sent".into()),
    }
}

impl NetShared {
    fn end(&mut self, why: Ending) {
        if self.ending.is_some() {
            return;
        }
        // Every peer writes its replay (NET.md section 8); a desync already wrote its own.
        if self.desync.is_none() && !matches!(why, Ending::Left) {
            let dir = self.opts.dir();
            let path = dir.join(format!("match-{}-seat{}-tick{}.star", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()), self.me.0, self.peer.tick()));
            if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&path, self.peer.replay().encode()).is_ok() {
                self.replay_path = Some(path);
            }
        }
        let sev = if matches!(why, Ending::Left) { Severity::Info } else { Severity::Bad };
        self.say(sev, why.text());
        self.ending = Some(why);
    }

    fn kick(&mut self, seat: PlayerId, why: &str) {
        if self.control.kick(seat).is_ok() {
            self.drop_now.insert(seat);
            let name = self.name(seat);
            self.say(Severity::Bad, format!("{name} was removed from the game: {why}."));
        }
    }

    fn on_router_event(&mut self, e: RouterEvent) {
        match e {
            RouterEvent::Protocol { from, error } => {
                let text = error.to_string();
                self.violations.entry(from).or_insert_with(|| text.clone());
                match protocol_reaction(self.me, self.host, from) {
                    Reaction::Kick(seat) => self.kick(seat, &format!("it broke the network protocol ({text})")),
                    Reaction::EndMatch => self.end(Ending::HostProtocol(text)),
                    Reaction::Note => {}
                }
            }
            RouterEvent::SeatDropped { slot, last_turn, .. } => {
                self.dropped.insert(slot, Dropped { last_turn });
                self.departed.remove(&slot);
                self.drop_now.remove(&slot);
                self.rejoining.remove(&slot);
                let name = self.name(slot);
                self.say(
                    Severity::Bad,
                    format!("{name} dropped out of the game. Their ships stay where they are and take no orders; what should happen to a dropped player's forces is not decided yet, so nothing is eliminated or handed to the computer."),
                );
            }
            RouterEvent::Transport(_) => {}
            _ => {}
        }
    }

    fn on_resync_event(&mut self, e: ResyncEvent) {
        match e {
            ResyncEvent::SeatAbuse { slot } if self.is_host() => self.kick(slot, "it asked to rejoin or resync too often"),
            ResyncEvent::RejoinAccepted { slot } => {
                self.rejoining.insert(slot);
                let name = self.name(slot);
                self.say(Severity::Info, format!("{name} is rejoining the game."));
            }
            ResyncEvent::Arrived { slot, .. } => {
                self.rejoining.remove(&slot);
                self.departed.remove(&slot);
                self.drop_now.remove(&slot);
                let name = self.name(slot);
                self.say(Severity::Good, format!("{name} is back in the game."));
            }
            ResyncEvent::Repaired { .. } => {
                if let Some(d) = self.desync.as_mut() {
                    d.stage = DesyncStage::Repaired;
                }
                self.say(Severity::Good, "This game was repaired from the host's state and plays on.".into());
            }
            ResyncEvent::Resynced { last: true, .. } => {
                if let Some(d) = self.desync.as_mut() {
                    d.stage = DesyncStage::Repaired;
                }
            }
            ResyncEvent::Failed(why) => {
                if let Some(d) = self.desync.as_mut() {
                    d.stage = DesyncStage::Stopped(format!("the host's state could not be loaded ({why})"));
                }
                self.say(Severity::Bad, format!("The game could not be repaired from the host's state ({why}). The match stays stopped."));
            }
            ResyncEvent::LimitReached { slot, .. } => {
                if let Some(d) = self.desync.as_mut() {
                    d.stage = DesyncStage::Stopped("the host will not repair this seat again".into());
                }
                let name = self.name(slot);
                self.say(Severity::Bad, format!("{name} has been repaired as often as the game allows. The match stays stopped."));
            }
            ResyncEvent::RepairOverdue => {
                if let Some(d) = self.desync.as_mut() {
                    d.stage = DesyncStage::Stopped("the host did not repair the game in time".into());
                }
            }
            _ => {}
        }
    }

    /// Once, when the match halts on a digest mismatch: the report that
    /// `sta-bench desync` and `sta_net::diagnose` read.
    fn note_desync(&mut self) {
        let r = self.peer.router();
        if self.desync.is_some() || !r.halted() {
            return;
        }
        let Some(tick) = r.desync().map(|n| n.theirs.tick).or_else(|| r.peer_desync().map(|(_, n)| n.ours.tick)) else { return };
        let dir = self.opts.dir();
        let stem = report_stem(self, tick);
        let written = std::fs::create_dir_all(&dir).and_then(|()| self.peer.write_desync_report(&dir, &stem));
        let (report, error) = match written {
            Ok(f) => (Some(f.replay), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let line = match (&report, &error) {
            (Some(p), _) => format!("The game went out of step at tick {tick}. The match is stopped while it is repaired from the host's state. The report is {}; compare yours with another player's using `sta-bench desync`.", p.display()),
            (_, Some(e)) => format!("The game went out of step at tick {tick}. The report could not be written ({e})."),
            _ => String::new(),
        };
        self.say(Severity::Bad, line);
        self.desync = Some(DesyncInfo { tick, report, error, stage: DesyncStage::Repairing });
    }

    /// Host: a seat whose connection ended is dropped once its rejoin grace has passed (never
    /// while its rejoin is in progress), or at once when it was kicked for a violation.
    fn drop_departed(&mut self, now: Instant) {
        if !self.is_host() || self.peer.router().halted() {
            return;
        }
        let due: Vec<PlayerId> = self
            .departed
            .iter()
            .filter(|(seat, since)| {
                !self.dropped.contains_key(seat) && (self.drop_now.contains(seat) || (!self.peer.rejoin_in_progress(**seat) && now.saturating_duration_since(**since) >= self.opts.drop_grace))
            })
            .map(|(s, _)| *s)
            .collect();
        for seat in due {
            // The marker was read after every record the seat sent: the last one is final.
            let last_turn = self.peer.router().last_record(seat).unwrap_or(0);
            // NET-02's behaviour, said plainly in the overlay: see the module notes.
            self.peer.router_mut().schedule(Action::Drop { slot: seat, last_turn, then: DropChoice::Eliminate });
            // One extension per drop: `extend_lead` is documented as called once per drop.
            self.peer.router_mut().extend_lead();
            self.departed.remove(&seat);
            self.drop_now.remove(&seat);
            self.dropped.insert(seat, Dropped { last_turn });
        }
    }

    fn duties(&mut self, now: Instant) {
        // The connection first: a closed one ends the match whatever else is pending.
        if self.ending.is_none() {
            // Only one reader takes departure markers: on the host that is the peer's departure
            // source (it hands them to the rejoin logic and to `departures`), else a marker
            // read here would never reach `note_departed` and the seat would never be dropped.
            let gone = if self.is_host() { Vec::new() } else { self.control.take_departures() };
            if self.is_host() {
                let seen: Vec<PlayerId> = std::mem::take(&mut *self.departures.lock().unwrap_or_else(|e| e.into_inner()));
                for seat in seen {
                    if seat != self.me {
                        self.departed.entry(seat).or_insert(now);
                        let name = self.name(seat);
                        if !self.dropped.contains_key(&seat) && !self.drop_now.contains(&seat) {
                            self.say(Severity::Warn, format!("{name} lost the connection. Their seat is held for {} seconds for a rejoin.", self.opts.drop_grace.as_secs()));
                        }
                    }
                }
            }
            let host_gone = gone.contains(&self.host);
            if host_gone {
                self.end(Ending::HostLeft);
            }
            if let Some(r) = self.control.close_reason() {
                // A host whose room closes, or a peer whose connection ended.
                let why = match (&r, self.is_host()) {
                    // The host's own relay closed: nobody removed the host.
                    (CloseReason::RelayClosed, true) => Ending::ConnectionLost("the relay closed the room".into()),
                    _ => lost_text(&r),
                };
                self.end(why);
            }
        }
        self.note_desync();
        for e in self.peer.router_mut().take_events() {
            self.on_router_event(e);
        }
        for e in self.peer.take_resync_events() {
            self.on_resync_event(e);
        }
        self.note_desync();
        if self.peer.resync_failed()
            && let Some(d) = self.desync.as_mut()
            && !matches!(d.stage, DesyncStage::Stopped(_))
        {
            d.stage = DesyncStage::Stopped("the host's state could not be loaded".into());
        }
        if let Some(d) = self.desync.as_mut()
            && d.stage == DesyncStage::Repairing
            && !self.peer.router().halted()
            && !self.peer.resyncing()
        {
            d.stage = DesyncStage::Repaired;
        }
        self.drop_departed(now);
    }
}

/// Every frame: the connection, the events, the host's drop policy, the desync report.
/// Says what happened as notices, and leaves the match when it is over.
pub fn net_duties(game: Option<Res<NetGame>>, mut notices: MessageWriter<Notice>, mut leave: MessageWriter<LeaveMatch>, mut mp: ResMut<Multiplayer>) {
    let Some(game) = game else { return };
    let mut s = game.lock();
    s.duties(Instant::now());
    for (sev, text) in s.fresh.drain(..) {
        notices.write(Notice::new(sev, text));
    }
    // A match that is over for this peer returns to the multiplayer screen with the reason.
    if let Some(end) = s.ending.clone() {
        mp.notice = Some(end.text());
        leave.write(LeaveMatch);
    }
}

/// The match's resources go with it: the peer's connection closes when the last handle drops.
pub fn clear_net(mut commands: Commands, game: Option<Res<NetGame>>, start: Option<ResMut<NetStart>>) {
    if let Some(g) = game {
        let s = g.lock();
        // Leaving a running match on purpose: tell the relay now rather than at drop time.
        s.control.leave();
    }
    commands.remove_resource::<NetGame>();
    if let Some(mut s) = start
        && let Some(nm) = s.0.take()
    {
        nm.control.leave();
    }
}

pub struct NetMatchPlugin;

impl Plugin for NetMatchPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<NetOptions>() {
            app.init_resource::<NetOptions>();
        }
        app.init_resource::<NetStart>()
            .init_resource::<NetStartFailed>()
            .add_systems(OnEnter(AppState::FrontEnd), clear_net)
            .add_systems(
                Update,
                (
                    on_ready.run_if(in_state(AppState::FrontEnd)),
                    on_start_failed,
                    net_duties.run_if(resource_exists::<NetGame>).run_if(super::flow::match_running),
                )
                    .chain()
                    .in_set(GameSet::Ui),
            );
    }
}
