//! The multiplayer lobby (UI-11): host a game, join one, seat everyone, launch.
//!
//! ```text
//!   Main --Multiplayer--> Multiplayer (name, relay) --Host game--> HostGame (password) --Create--+
//!                              |                                                                 v
//!                              +----------Join game--> JoinGame (code, password) --Join---> Lobby --Launch--> NetGameReady
//!                                                           ^                                  |
//!                                                           +----- refusal / kicked / lost ----+
//! ```
//!
//! Two halves:
//!
//! - [`Session`] (this file, no Bevy): the state machine of one player's room, over a
//!   [`NetTransport`] to a relay (NET-03). The host admits joiners with the `Hello` /
//!   `Welcome` / `Refuse` exchange (NET.md section 5, the keyed rules digest, a refused
//!   joiner is told the differing key), keeps the seat table, broadcasts `Lobby`, takes
//!   `SeatChoice` and `Chat`, and on launch sends one `Start`; every peer then builds the
//!   same [`NetMatch`] and plays through the lockstep router.
//! - The screens ([`view`], regions in [`layout`]) and the Bevy systems below: the
//!   connection runs on a thread (the transport's connect blocks), the session is polled
//!   once a frame, a refusal or loss returns to the connection screens with a message in
//!   plain words, and a launch puts the [`NetMatch`] in [`NetLaunch`] and writes
//!   [`NetGameReady`]. **Starting that match in the map view is INT-M5's** (it needs the
//!   map view to build its sim over the match's router); nothing here changes `AppState`.
//!
//! Decisions the design left open, each behind one clearly named function so the answer
//! can change in one place (the cheapest option is built, see the PR):
//!
//! - [`place_computers_around_humans`]: the relay seats a joiner in the lowest free seat
//!   (host 0), so computers cannot hold a fixed slot. The host plans computers per slot;
//!   a joiner who lands on a planned computer's slot displaces it to the highest free slot.
//! - [`Session::after_kick`]: a kicked player may rejoin an open room at once (nothing is
//!   reserved, nothing banned).
//! - [`Session::on_bad_frame`]: a frame that fails to decode has no sender (the relay's
//!   stamp is lost with it), so it is counted and ignored here; in the match the router's
//!   `RouterEvent::Protocol` is INT's to turn into a kick.
//! - [`rejoin_unavailable`]: rejoining a running game is NET-04; the entry point exists and
//!   says why it is off.
//!
//! **Originals.** Armada II's Game Setup in a multiplayer game has the player rows with a
//! ready light left of them (red, green once the player pressed Launch), the player, side
//! and team columns, a log under the table with a chat line, the map window and the
//! resource/officer settings (Refit `ui/MENUS.md`, measured); its "Multiplayer
//! Connection" is the player's name and the connection types. See [`layout`] for which
//! regions are measured and which arranged. Armada I and Fleet Operations are not measured.

// The screens are for INT-M4/M5 to start; the binary does not yet.
#![allow(dead_code, unused_imports)]

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::sync::Mutex;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use bevy::platform::cell::SyncCell;
use bevy::prelude::*;
use sta_content::{ContentDb, MapDef};
use sta_core::{PlayerId, TeamId};
use sta_net::lockstep::{Destination, Incoming, Peer, Transport, TransportError};
use sta_net::proto::{
    self, EngineId, Hello, LobbySeat, LobbyState, MAX_CHAT_BYTES, MAX_NAME_BYTES, MapRef, Message, Occupant, PackInfo, Refusal, RulesAssembler, RulesDigest, RulesIdentity,
    SeatChoice, Start, TurnConfig, Welcome,
};
use sta_net::transport::{ClientOptions, CloseReason, ConnectError, NetControl, NetTransport, RefuseReason, RoomCode};
use sta_sim::setup::{GameOptions, GameSetup};

use crate::frontend::screens::stock::StockBox;
use crate::ui::i18n::tr;
use crate::frontend::screens::{Browse, MAX_SEATS, MapEntry, MapReturn, SKIRMISH_AI, Screen, ScreenData, ScreenRoot, ScreensCamera, Settings, SetupDraft};
use crate::ui::theme::{ButtonClicked, DropdownChanged, Focusable, KitFocus, TextChanged, TextField, TextSubmitted, Theme};

pub mod layout;
#[cfg(test)]
pub(crate) mod tests;
mod view;

pub use view::{LobbyDrop, LobbyField, MpAct};

/// The relay's default port (`sta_net`'s `relay` binary listens on it).
pub const DEFAULT_RELAY_PORT: u16 = 7864;
/// How long a refused or kicked player has to leave on its own before the relay is asked
/// to close its connection.
pub const KICK_GRACE: Duration = Duration::from_millis(1500);
/// How long a joiner waits for the host's `Welcome` or `Refuse`.
pub const JOIN_ANSWER: Duration = Duration::from_secs(15);
/// Chat lines kept.
pub const CHAT_KEPT: usize = 100;
/// Match messages kept from the lobby for the router, at most.
const EARLY_KEPT: usize = 512;

// ---------------------------------------------------------------------------
// What the screens know about maps and sides
// ---------------------------------------------------------------------------

/// A map a room can use: its name, its digest (`proto::map_digest`; two peers must hold
/// the same file) and its start slots.
#[derive(Debug, Clone)]
pub struct MapInfo {
    pub name: String,
    pub digest: u64,
    pub slots: usize,
    pub def: Arc<MapDef>,
}

impl MapInfo {
    pub fn of(entry: &MapEntry) -> MapInfo {
        MapInfo { name: entry.def.name.clone(), digest: proto::map_digest(&entry.def).unwrap_or(0), slots: entry.slots(), def: entry.def.clone() }
    }
}

/// Everything a [`Session`] needs from the program around it.
#[derive(Clone)]
pub struct SessionCfg {
    pub engine: EngineId,
    pub content: Arc<ContentDb>,
    /// The player's name.
    pub name: String,
    /// The install's content lineage, informational (D10).
    pub lineage: String,
    /// The playable sides, in the install's skirmish order.
    pub races: Vec<String>,
    pub maps: Vec<MapInfo>,
    /// The match's seed; `None` draws one at launch.
    pub seed: Option<u64>,
}

impl SessionCfg {
    pub fn from_data(data: &ScreenData, name: &str) -> SessionCfg {
        SessionCfg {
            engine: engine_id(),
            content: data.content.clone(),
            name: clean_name(name, ""),
            // The content's own lineage, which a snapshot's provenance is checked against (NET-04).
            lineage: data.content.lineage().to_owned(),
            races: data.races(),
            maps: data.maps.iter().map(MapInfo::of).collect(),
            seed: None,
        }
    }

    fn map_named(&self, name: &str) -> Option<usize> {
        self.maps.iter().position(|m| m.name.eq_ignore_ascii_case(name))
    }
}

/// This build, as the handshake compares it: peers must run the same name, version and
/// build (NET.md 4.4). The commit comes from `STA64_GIT_COMMIT` at build time; without it
/// every build calls itself `dev`, so two such builds are taken to match.
pub fn engine_id() -> EngineId {
    EngineId { name: "sta64".into(), version: env!("CARGO_PKG_VERSION").into(), build: option_env!("STA64_GIT_COMMIT").unwrap_or("dev").into() }
}

/// A player name as the wire takes it: trimmed, no control characters, at most
/// `MAX_NAME_BYTES`; `fallback` (or "Player") when nothing is left.
pub fn clean_name(name: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > MAX_NAME_BYTES {
            break;
        }
        out.push(c);
    }
    let out = out.trim().to_owned();
    if out.is_empty() { if fallback.is_empty() { "Player".into() } else { fallback.into() } } else { out }
}

/// A relay address as typed: trimmed, with the default port when none is given. `None`
/// for nothing.
pub fn normalize_relay(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    // A bracketed IPv6 address with a port, or a host:port, is complete; a lone IPv6
    // address or a bare name gets the port.
    let has_port = if t.starts_with('[') { t.contains("]:") } else { t.matches(':').count() == 1 };
    Some(if has_port { t.to_owned() } else if t.contains(':') && !t.starts_with('[') { format!("[{t}]:{DEFAULT_RELAY_PORT}") } else { format!("{t}:{DEFAULT_RELAY_PORT}") })
}

fn random_u64() -> u64 {
    // `RandomState` is seeded from the operating system; no extra dependency.
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64));
    h.finish()
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

/// Captions and messages: the mounted install's `label.map` text where it has the key
/// (Armada II's own wording: `kicked`, `full`, `locked`, `host_quit`, `bad_password`,
/// `bad_crc`, `no_connection`, `kick`, `launch`, `join_game`, `create_game`...), else our
/// plain English.
#[derive(Clone, Copy, Default)]
pub struct Words<'a>(pub Option<&'a crate::ui::i18n::Strings>);

impl Words<'_> {
    /// The install's text for `key`, else `plain` (translated).
    pub fn get(&self, key: &str, plain: &str) -> String {
        self.0.and_then(|s| s.native_label(key)).map(str::trim).filter(|t| !t.is_empty()).map_or_else(|| tr(plain), str::to_owned)
    }

    /// The install's text for `key` followed by our detail, or the detail alone.
    fn with_detail(&self, key: &str, detail: String) -> String {
        match self.0.and_then(|s| s.native_label(key)).map(str::trim).filter(|t| !t.is_empty()) {
            Some(native) => format!("{native} {detail}"),
            None => detail,
        }
    }
}

/// Why the relay did not take a client in, in plain words.
pub fn refuse_reason_text(r: RefuseReason) -> String {
    refuse_reason_text_in(r, Words::default())
}

pub fn refuse_reason_text_in(r: RefuseReason, w: Words) -> String {
    match r {
        RefuseReason::Version => tr("The relay speaks another version of the network protocol. Use a relay from the same build."),
        RefuseReason::NoRoom => tr("There is no room with that code on this relay. Check the code and the relay address."),
        RefuseReason::BadPassword => w.get("bad_password", "The password is wrong."),
        RefuseReason::Full => w.get("room_full", "The room is full: all eight seats are taken."),
        RefuseReason::Closed => w.get("locked", "The room is closed to new players, or its host has left."),
        RefuseReason::BadReclaim => tr("That seat cannot be taken back: it is not vacated."),
        RefuseReason::RateLimited => tr("Too many tries from this address. Wait a minute and try again."),
        RefuseReason::Busy => tr("The relay is busy. Try again later."),
        RefuseReason::BadRequest => tr("The relay did not understand the request."),
    }
}

/// Why a connection could not be made, in plain words.
pub fn connect_error_text(e: &ConnectError, addr: &str) -> String {
    connect_error_text_in(e, addr, Words::default())
}

pub fn connect_error_text_in(e: &ConnectError, addr: &str, w: Words) -> String {
    match e {
        ConnectError::Io(e) => w.with_detail("no_connection", format!("Cannot reach the relay at {addr}: {e}.")),
        ConnectError::Refused(r) => refuse_reason_text_in(*r, w),
        ConnectError::Timeout => format!("The relay at {addr} did not answer."),
        ConnectError::Protocol(_) => format!("{addr} did not answer like an STA64 relay."),
    }
}

/// Why a host would not take a joiner, in plain words. A rules refusal names the key.
pub fn refusal_text(r: &Refusal) -> String {
    refusal_text_in(r, Words::default())
}

pub fn refusal_text_in(r: &Refusal, w: Words) -> String {
    match r {
        Refusal::Full => w.get("full", "The game is full."),
        Refusal::InProgress => w.get("launched", "The game has already started."),
        Refusal::Closed => w.get("locked", "The host has closed the game to new players."),
        Refusal::Engine { host } => w.with_detail(
            "bad_crc",
            format!("The host runs another build of the game ({} {}, build {}). Everyone must run the same build.", host.name, host.version, host.build),
        ),
        Refusal::RulesDiffer(d) => w.with_detail(
            "bad_crc",
            format!(
                "Your game rules differ from the host's. {} is {} on the host and {} on your side.",
                d.key,
                d.host.as_deref().unwrap_or("not defined"),
                d.joiner.as_deref().unwrap_or("not defined")
            ),
        ),
        Refusal::RulesUnlisted => w.with_detail("bad_crc", "Your game rules differ from the host's, but no single rule could be named.".into()),
        Refusal::BadRejoin => tr("The host would not give that seat back."),
        Refusal::Kicked => w.get("kicked", "The host removed you from the room."),
        Refusal::Protocol => tr("The host did not accept what was sent (protocol error)."),
    }
}

/// Why a room ended for this player.
#[derive(Debug, Clone, PartialEq)]
pub enum Ended {
    /// The host's `Refuse`.
    Refused(Refusal),
    HostLeft,
    /// The relay closed this connection without a word: removed from the room, or the
    /// relay stopped.
    RelayClosed,
    /// The connection failed.
    ConnectionLost(String),
    /// Nothing came back from the host.
    NoAnswer,
    /// The other side broke the protocol.
    Protocol(String),
    /// This player left.
    Left,
}

impl Ended {
    pub fn text(&self) -> String {
        self.text_in(Words::default())
    }

    pub fn text_in(&self, w: Words) -> String {
        match self {
            Ended::Refused(r) => refusal_text_in(r, w),
            Ended::HostLeft => w.get("host_quit", "The host left, so the room is closed."),
            Ended::RelayClosed => "The relay closed your connection. The host may have removed you, or the relay stopped.".into(),
            Ended::ConnectionLost(why) => w.with_detail("no_connection", format!("The connection to the relay was lost: {why}.")),
            Ended::NoAnswer => "The host did not answer.".into(),
            Ended::Protocol(why) => format!("Something went wrong in the conversation with the host: {why}."),
            Ended::Left => "You left the room.".into(),
        }
    }
}

fn close_text(r: &CloseReason) -> Ended {
    match r {
        CloseReason::Left => Ended::Left,
        CloseReason::RelayClosed => Ended::RelayClosed,
        CloseReason::TimedOut => Ended::ConnectionLost("the relay stopped answering".into()),
        CloseReason::Io(e) => Ended::ConnectionLost(e.clone()),
        CloseReason::Protocol(e) => Ended::Protocol(format!("the relay sent something invalid ({e})")),
        CloseReason::InboxFull => Ended::ConnectionLost("the game could not keep up with what arrived".into()),
        CloseReason::SendQueueFull => Ended::ConnectionLost("the relay could not take what was sent".into()),
    }
}

/// Rejoining a running game after a lost connection is NET-04's (`Rejoin`/`Replayed`
/// catch-up); the lobby offers the entry point and says why it is off.
pub fn rejoin_unavailable() -> &'static str {
    "Rejoining a running game is not built yet (NET-04). Ask the host to start a new game."
}

// ---------------------------------------------------------------------------
// The seat table
// ---------------------------------------------------------------------------

/// What holds a slot of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Nobody yet: a joiner may take it; at launch it becomes idle.
    Open,
    /// The `skirmish` computer player, run by the host.
    Computer,
    /// Nobody, ever: the seat exists and sends nothing.
    Idle,
    /// A person.
    Human,
}

/// The host's plan for one slot: what a joiner-less slot is, and the side and team it
/// would have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPlan {
    pub kind: Kind,
    pub race: String,
    pub team: u8,
}

/// The seat table as the relay's seating allows it. The relay puts a joiner in the lowest
/// free seat (host 0), so the host cannot keep a computer on a slot a joiner will land
/// on. `humans` has a bit per slot held by a person. A computer or idle seat planned on a
/// human's slot moves to the highest slot that is still open, keeping its side and team;
/// one that finds no room is dropped. **The cheapest answer to the open seating question**:
/// change this function to change the policy.
pub fn place_computers_around_humans(plan: &[SlotPlan], humans: u8) -> Vec<SlotPlan> {
    let human = |i: usize| humans & (1 << i) != 0;
    let mut out: Vec<SlotPlan> = plan.to_vec();
    let mut displaced = Vec::new();
    for (i, p) in plan.iter().enumerate() {
        if human(i) {
            if matches!(p.kind, Kind::Computer | Kind::Idle) {
                displaced.push(p.clone());
            }
            out[i].kind = Kind::Human;
        }
    }
    // Highest open slot first.
    let free: Vec<usize> = (0..out.len()).rev().filter(|&i| !human(i) && out[i].kind == Kind::Open).collect();
    for (d, i) in displaced.into_iter().zip(free) {
        out[i] = d;
    }
    out
}

/// One seat as the screens show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatView {
    pub slot: u8,
    pub kind: Kind,
    pub name: String,
    pub race: String,
    pub team: u8,
    pub ready: bool,
    /// This player's own seat.
    pub me: bool,
    pub host: bool,
}

/// The room as the screens show it; the host builds it from its own table, a joiner from
/// the host's last `Lobby`.
#[derive(Debug, Clone, PartialEq)]
pub struct LobbyView {
    pub map: String,
    /// Index in this machine's map list; `None` when this machine has no such map (a
    /// joiner then cannot ready up).
    pub map_index: Option<usize>,
    /// This machine's copy of the map is the host's (same digest).
    pub map_known: bool,
    pub options: GameOptions,
    pub closed: bool,
    /// One entry per slot of the map.
    pub seats: Vec<SeatView>,
}

#[derive(Debug, Clone)]
struct Guest {
    name: String,
    lineage: String,
    packs: Vec<PackInfo>,
    engine: EngineId,
    race: String,
    team: u8,
    ready: bool,
    token: u64,
}

struct Pending {
    hello: Hello,
    assembler: RulesAssembler,
    at: Instant,
}

/// What a session is doing.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Joined the relay, waiting for the host's `Welcome`.
    Joining,
    /// In the lobby.
    Open,
    /// The match is set: take it with [`Session::take_match`].
    Launched,
    Ended(Ended),
}

/// A line of the log under the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub from: String,
    pub text: String,
}

/// Everything a match needs, the same on every peer: the `Start` the host sent, the setup
/// as seen from this seat, and the transport the router runs over.
pub struct NetMatch {
    pub start: Start,
    pub me: PlayerId,
    pub control: NetControl,
    /// The seats' builds (from their `Hello`s) for the desync sidecar; the host's only.
    pub engines: Vec<(PlayerId, EngineId)>,
    /// The `rejoin_token` each guest got in its `Welcome` (the host's only), for
    /// `Peer::serve_rejoins`.
    pub tokens: Vec<(PlayerId, u64)>,
    /// This seat's own `rejoin_token` from the host's `Welcome` (a guest's only).
    pub my_token: Option<u64>,
    /// `GameSetup` from [`Start::player_setups`] for this seat.
    pub setup: GameSetup,
    transport: Option<SyncCell<Box<dyn Transport>>>,
}

impl NetMatch {
    /// The lockstep peer of this match (`Peer::start`): the sim at tick 0 and the router
    /// over the transport. Can be taken once.
    pub fn into_peer(mut self) -> Result<Peer, proto::ProtoError> {
        let transport = SyncCell::to_inner(self.transport.take().ok_or(proto::ProtoError::Invalid("transport already taken"))?);
        let mut peer = Peer::start(&self.start, self.me, self.setup.content.clone(), self.setup.map.clone(), transport)?;
        peer.set_engines(self.engines);
        Ok(peer)
    }

    /// The transport (with the messages that arrived before the match was built put
    /// back in front), for a host that builds its own router. Can be taken once.
    pub fn take_transport(&mut self) -> Option<Box<dyn Transport>> {
        self.transport.take().map(SyncCell::to_inner)
    }
}

impl std::fmt::Debug for NetMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetMatch").field("me", &self.me).field("start", &self.start).finish()
    }
}

/// Messages that arrived in the lobby but belong to the match (a peer that already
/// started sends its first turn at once), handed to the router first.
struct Early {
    first: VecDeque<Incoming>,
    inner: NetTransport,
}

impl Transport for Early {
    fn send(&mut self, to: Destination, msg: &Message) -> Result<(), TransportError> {
        self.inner.send(to, msg)
    }

    fn poll(&mut self) -> Result<Option<Incoming>, TransportError> {
        match self.first.pop_front() {
            Some(m) => Ok(Some(m)),
            None => self.inner.poll(),
        }
    }
}

/// Why the lobby cannot launch yet; `None` when it can.
pub type LaunchBlock = Option<String>;

/// One player's room. Create with [`Session::host`] or [`Session::guest`] on a connected
/// transport, call [`Session::update`] every frame, act through the methods.
pub struct Session {
    cfg: SessionCfg,
    host: bool,
    transport: Option<NetTransport>,
    control: NetControl,
    me: PlayerId,
    code: RoomCode,
    pub phase: Phase,
    changed: bool,
    chat: VecDeque<ChatLine>,
    bad_frames: u32,
    early: VecDeque<Incoming>,
    started: Option<NetMatch>,
    // Host.
    map: usize,
    options: GameOptions,
    plan: Vec<SlotPlan>,
    guests: [Option<Guest>; MAX_SEATS],
    pending: [Option<Pending>; MAX_SEATS],
    closed: bool,
    host_race: String,
    host_team: u8,
    kicks: Vec<(PlayerId, Instant)>,
    // Joiner.
    welcomed: bool,
    my_token: Option<u64>,
    lobby: Option<LobbyState>,
    asked: Instant,
    my_choice: Option<SeatChoice>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("host", &self.host).field("me", &self.me).field("code", &self.code.to_string()).field("phase", &self.phase).finish()
    }
}

impl Session {
    fn new(cfg: SessionCfg, transport: NetTransport, host: bool) -> Session {
        let control = transport.control();
        let (me, code) = (control.seat(), control.room_code());
        let mut s = Session {
            cfg,
            host,
            transport: Some(transport),
            control,
            me,
            code,
            phase: if host { Phase::Open } else { Phase::Joining },
            changed: true,
            chat: VecDeque::new(),
            bad_frames: 0,
            early: VecDeque::new(),
            started: None,
            map: 0,
            options: GameOptions::default(),
            plan: Vec::new(),
            guests: Default::default(),
            pending: Default::default(),
            closed: false,
            host_race: String::new(),
            host_team: 0,
            kicks: Vec::new(),
            welcomed: false,
            my_token: None,
            lobby: None,
            asked: Instant::now(),
            my_choice: None,
        };
        if host {
            s.host_race = s.race_for_slot(0);
            s.set_map_unchecked(0);
        }
        s
    }

    /// The host of a room the relay has just opened (seat 0), map 0, every other slot open.
    pub fn host(cfg: SessionCfg, transport: NetTransport) -> Session {
        Session::new(cfg, transport, true)
    }

    /// A joiner who has the relay's seat: sends its `Hello` and waits for the host.
    pub fn guest(cfg: SessionCfg, mut transport: NetTransport) -> Session {
        let hello = Hello {
            engine: cfg.engine.clone(),
            rules: cfg.content.digest().digest(),
            lineage: cfg.lineage.clone(),
            packs: Vec::new(),
            name: cfg.name.clone(),
        };
        let sent = transport.send(Destination::Seat(PlayerId(0)), &Message::Hello(hello));
        let mut s = Session::new(cfg, transport, false);
        if sent.is_err() {
            s.end(Ended::ConnectionLost("the connection closed before the host was greeted".into()));
        }
        s
    }

    // ---- what the screens read --------------------------------------------------

    pub fn is_host(&self) -> bool {
        self.host
    }

    pub fn me(&self) -> PlayerId {
        self.me
    }

    /// The code others join with.
    pub fn code(&self) -> RoomCode {
        self.code
    }

    pub fn control(&self) -> NetControl {
        self.control.clone()
    }

    pub fn chat(&self) -> &VecDeque<ChatLine> {
        &self.chat
    }

    /// Frames that failed to decode (they carry no sender: see [`Session::on_bad_frame`]).
    pub fn bad_frames(&self) -> u32 {
        self.bad_frames
    }

    /// Whether anything the screens show changed since the last call.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The match, once launched. Taken once.
    pub fn take_match(&mut self) -> Option<NetMatch> {
        self.started.take()
    }

    pub fn ended(&self) -> Option<&Ended> {
        match &self.phase {
            Phase::Ended(e) => Some(e),
            _ => None,
        }
    }

    pub fn races(&self) -> &[String] {
        &self.cfg.races
    }

    pub fn maps(&self) -> &[MapInfo] {
        &self.cfg.maps
    }

    /// The seats of this room.
    pub fn view(&self) -> LobbyView {
        if self.host {
            self.host_view()
        } else {
            self.guest_view()
        }
    }

    fn slots(&self) -> usize {
        self.cfg.maps.get(self.map).map_or(2, |m| m.slots).clamp(1, MAX_SEATS)
    }

    fn host_view(&self) -> LobbyView {
        let eff = self.effective();
        let info = self.cfg.maps.get(self.map);
        LobbyView {
            map: info.map_or_else(String::new, |m| m.name.clone()),
            map_index: info.map(|_| self.map),
            map_known: info.is_some(),
            options: self.options.clone(),
            closed: self.closed,
            seats: eff
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let g = self.guests[i].as_ref();
                    SeatView {
                        slot: i as u8,
                        kind: p.kind,
                        name: match p.kind {
                            Kind::Human if i == 0 => self.cfg.name.clone(),
                            Kind::Human => g.map_or_else(String::new, |g| g.name.clone()),
                            Kind::Computer => "Computer".into(),
                            _ => String::new(),
                        },
                        race: if p.kind == Kind::Human { g.map_or_else(|| self.host_race.clone(), |g| g.race.clone()) } else { p.race.clone() },
                        team: if p.kind == Kind::Human { g.map_or(self.host_team, |g| g.team) } else { p.team },
                        ready: match p.kind {
                            Kind::Human => i == 0 || g.is_some_and(|g| g.ready),
                            Kind::Computer => true,
                            _ => false,
                        },
                        me: i == 0,
                        host: i == 0,
                    }
                })
                .collect(),
        }
    }

    fn guest_view(&self) -> LobbyView {
        let Some(l) = &self.lobby else {
            return LobbyView { map: String::new(), map_index: None, map_known: false, options: GameOptions::default(), closed: false, seats: Vec::new() };
        };
        let index = self.cfg.map_named(&l.map.name);
        let known = index.is_some_and(|i| self.cfg.maps[i].digest == l.map.digest);
        let slots = index.map_or(0, |i| self.cfg.maps[i].slots).max(l.seats.iter().map(|s| usize::from(s.seat.slot.0) + 1).max().unwrap_or(0)).clamp(2, MAX_SEATS);
        let seats = (0..slots)
            .map(|i| {
                let got = l.seats.iter().find(|s| usize::from(s.seat.slot.0) == i);
                match got {
                    None => SeatView { slot: i as u8, kind: Kind::Open, name: String::new(), race: String::new(), team: i as u8, ready: false, me: false, host: i == 0 },
                    Some(s) => SeatView {
                        slot: i as u8,
                        kind: match s.seat.occupant {
                            Occupant::Peer => Kind::Human,
                            Occupant::Ai(_) => Kind::Computer,
                            Occupant::Idle => Kind::Idle,
                        },
                        name: s.seat.name.clone(),
                        race: s.seat.race.clone(),
                        team: s.seat.team.0,
                        ready: s.ready,
                        me: s.seat.slot == self.me,
                        host: s.seat.slot == PlayerId(0),
                    },
                }
            })
            .collect();
        LobbyView { map: l.map.name.clone(), map_index: index, map_known: known, options: l.options.clone(), closed: l.closed, seats }
    }

    /// Why Launch (for the host) cannot be pressed, in plain words.
    pub fn launch_block(&self) -> LaunchBlock {
        if !self.host {
            return Some("Only the host launches the game.".into());
        }
        if !matches!(self.phase, Phase::Open) {
            return Some("The room is not open.".into());
        }
        if self.cfg.races.is_empty() {
            return Some("The install has no sides to play.".into());
        }
        if self.cfg.maps.get(self.map).is_none() {
            return Some("No map is chosen.".into());
        }
        if self.pending.iter().any(Option::is_some) {
            return Some("A player is still joining.".into());
        }
        let eff = self.effective();
        for (i, p) in eff.iter().enumerate() {
            if p.kind == Kind::Human && i != 0 && !self.guests[i].as_ref().is_some_and(|g| g.ready) {
                let name = self.guests[i].as_ref().map_or("A player", |g| g.name.as_str());
                return Some(format!("Waiting for {name} to be ready."));
            }
        }
        if eff.iter().filter(|p| matches!(p.kind, Kind::Human | Kind::Computer)).count() < 2 {
            return Some("At least two seats must be played, by people or computers.".into());
        }
        None
    }

    /// The reason a joiner cannot press Ready, if there is one.
    pub fn ready_block(&self) -> Option<String> {
        let v = self.guest_view();
        if self.host || !matches!(self.phase, Phase::Open) {
            return Some("Not in a room.".into());
        }
        if v.map_index.is_none() {
            return Some(format!("You do not have the map {}.", v.map));
        }
        if !v.map_known {
            return Some(format!("Your copy of {} is not the host's.", v.map));
        }
        None
    }

    // ---- host: the table ----------------------------------------------------------

    fn humans_mask(&self) -> u8 {
        let mut m = 1u8;
        for (i, g) in self.guests.iter().enumerate() {
            if g.is_some() {
                m |= 1 << i;
            }
        }
        m
    }

    fn effective(&self) -> Vec<SlotPlan> {
        place_computers_around_humans(&self.plan, self.humans_mask())
    }

    fn race_for_slot(&self, slot: usize) -> String {
        self.cfg.races.get(slot % self.cfg.races.len().max(1)).cloned().unwrap_or_default()
    }

    fn set_map_unchecked(&mut self, index: usize) {
        self.map = index.min(self.cfg.maps.len().saturating_sub(1));
        let slots = self.slots();
        self.plan.truncate(slots);
        while self.plan.len() < slots {
            let i = self.plan.len();
            self.plan.push(SlotPlan { kind: if i == 0 { Kind::Human } else { Kind::Open }, race: self.race_for_slot(i), team: i as u8 });
        }
        self.plan[0].kind = Kind::Human;
        for p in &mut self.plan {
            p.team = p.team.min(slots as u8 - 1);
        }
        self.host_team = self.host_team.min(slots as u8 - 1);
        for g in self.guests.iter_mut().flatten() {
            g.team = g.team.min(slots as u8 - 1);
        }
    }

    /// Host: choose the map. Refused when it has fewer slots than the highest seat held by
    /// a person.
    pub fn set_map(&mut self, index: usize) -> Result<(), String> {
        if !self.host || index >= self.cfg.maps.len() {
            return Err("No such map.".into());
        }
        let need = self.guests.iter().rposition(Option::is_some).map_or(1, |i| i + 1);
        if self.cfg.maps[index].slots < need {
            return Err(format!("{} has only {} seats, and players are seated up to seat {}.", self.cfg.maps[index].name, self.cfg.maps[index].slots, need));
        }
        if index != self.map {
            self.set_map_unchecked(index);
            self.reset_ready();
            self.broadcast_lobby();
        }
        Ok(())
    }

    /// Host: the starting resources and the officer limit.
    pub fn set_options(&mut self, options: GameOptions) {
        if self.host && options != self.options {
            self.options = options;
            self.reset_ready();
            self.broadcast_lobby();
        }
    }

    /// Host: what a slot without a person is: open, a computer, or idle.
    pub fn set_slot_kind(&mut self, slot: usize, kind: Kind) {
        if !self.host || slot == 0 || slot >= self.plan.len() || !matches!(kind, Kind::Open | Kind::Computer | Kind::Idle) {
            return;
        }
        if self.plan[slot].kind != kind {
            self.plan[slot].kind = kind;
            self.reset_ready();
            self.broadcast_lobby();
        }
    }

    /// Host: the side of a seat it may set (its own, a computer's, an open slot's).
    pub fn set_slot_race(&mut self, slot: usize, race: &str) {
        let Some(race) = self.cfg.races.iter().find(|r| r.eq_ignore_ascii_case(race)).cloned() else { return };
        if !self.host || slot >= self.plan.len() || self.guests.get(slot).is_some_and(Option::is_some) {
            return;
        }
        if slot == 0 {
            self.host_race = race;
        } else {
            self.plan[slot].race = race;
        }
        self.reset_ready();
        self.broadcast_lobby();
    }

    /// Host: the team of a seat it may set.
    pub fn set_slot_team(&mut self, slot: usize, team: u8) {
        if !self.host || slot >= self.plan.len() || self.guests.get(slot).is_some_and(Option::is_some) {
            return;
        }
        let team = team.min(self.plan.len() as u8 - 1);
        if slot == 0 {
            self.host_team = team;
        } else {
            self.plan[slot].team = team;
        }
        self.reset_ready();
        self.broadcast_lobby();
    }

    /// Host: open or close the room to new players (Armada's "Game closed").
    pub fn set_room_open(&mut self, open: bool) {
        if !self.host || self.closed == !open {
            return;
        }
        self.closed = !open;
        let _ = self.control.set_open(open);
        self.broadcast_lobby();
    }

    /// Host: remove a player. They are told why and asked to leave; if they have not
    /// within [`KICK_GRACE`] the relay closes their connection. See [`Session::after_kick`].
    pub fn kick(&mut self, slot: usize) {
        if !self.host || slot == 0 || slot >= MAX_SEATS || self.guests[slot].is_none() {
            return;
        }
        let seat = PlayerId(slot as u8);
        self.guests[slot] = None;
        self.send(Destination::Seat(seat), &Message::Refuse(Refusal::Kicked));
        self.kicks.push((seat, Instant::now() + KICK_GRACE));
        self.after_kick(seat);
        self.broadcast_lobby();
    }

    /// What follows a kick. **The cheapest answer to the open question**: nothing. The seat
    /// is free at once and the kicked player may join again while the room is open; no
    /// seat is held back, no one is banned (the relay knows no identity, only the host's
    /// `Hello` names one). Change it here (a ban list by name and address, a reserved seat).
    pub fn after_kick(&mut self, _seat: PlayerId) {}

    fn reset_ready(&mut self) {
        for g in self.guests.iter_mut().flatten() {
            g.ready = false;
        }
    }

    // ---- both: chat, leaving ------------------------------------------------------

    /// Say something to the room.
    pub fn say(&mut self, text: &str) {
        let mut text: String = text.chars().filter(|c| !c.is_control()).collect();
        text = text.trim().to_owned();
        while text.len() > MAX_CHAT_BYTES {
            text.pop();
        }
        if text.is_empty() || !matches!(self.phase, Phase::Open) || (!self.host && !self.welcomed) {
            return;
        }
        let msg = Message::Chat(proto::Chat { from: self.me, scope: proto::ChatScope::All, recipients: 1u8.checked_shl(u32::from(self.me.0)).map_or(0xFF, |b| !b), text: text.clone() });
        self.send(Destination::All, &msg);
        let from = self.name_of(self.me);
        self.push_chat(ChatLine { from, text });
    }

    fn name_of(&self, seat: PlayerId) -> String {
        self.view().seats.iter().find(|s| s.slot == seat.0).map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Seat {}", seat.0 + 1))
    }

    fn push_chat(&mut self, line: ChatLine) {
        if self.chat.len() >= CHAT_KEPT {
            self.chat.pop_front();
        }
        self.chat.push_back(line);
        self.changed = true;
    }

    /// Leave the room (the host leaving ends it for newcomers; members are told).
    pub fn leave(&mut self) {
        if matches!(self.phase, Phase::Open | Phase::Joining) {
            self.control.leave();
            self.end(Ended::Left);
        }
    }

    // ---- joiner: choices ----------------------------------------------------------

    /// Joiner: choose side and team. The host decides what it accepts.
    pub fn choose(&mut self, race: &str, team: u8) {
        if self.host || !self.welcomed || !matches!(self.phase, Phase::Open) {
            return;
        }
        // A change of side or team takes the ready mark back: what was agreed to has changed.
        let c = SeatChoice { team: TeamId(team), race: race.to_owned(), ready: false };
        self.send(Destination::Seat(PlayerId(0)), &Message::SeatChoice(c.clone()));
        self.my_choice = Some(c);
    }

    /// Joiner: ready or not (the original's Launch light). Refused while [`Session::ready_block`] says so.
    pub fn set_ready(&mut self, ready: bool) {
        if self.host || !self.welcomed || !matches!(self.phase, Phase::Open) || (ready && self.ready_block().is_some()) {
            return;
        }
        let mine = self.guest_view().seats.into_iter().find(|s| s.me);
        let (race, team) = mine.map_or_else(|| (self.cfg.races.first().cloned().unwrap_or_default(), 0), |s| (s.race, s.team));
        self.send(Destination::Seat(PlayerId(0)), &Message::SeatChoice(SeatChoice { team: TeamId(team), race, ready }));
    }

    // ---- the conversation ---------------------------------------------------------

    fn send(&mut self, to: Destination, msg: &Message) {
        let Some(t) = self.transport.as_mut() else { return };
        if t.send(to, msg).is_err() {
            self.on_closed();
        }
    }

    fn end(&mut self, why: Ended) {
        if matches!(self.phase, Phase::Ended(_) | Phase::Launched) {
            return;
        }
        self.phase = Phase::Ended(why);
        self.transport = None;
        self.changed = true;
    }

    fn on_closed(&mut self) {
        let why = self.control.close_reason().map_or(Ended::RelayClosed, |r| close_text(&r));
        self.end(why);
    }

    /// A frame failed to decode. The relay's stamp does not survive a bad frame, so it
    /// cannot be traced to a seat here: counted and ignored. **The open question**: in the
    /// match the router's `RouterEvent::Protocol` names a seat, and INT maps that to a kick.
    pub fn on_bad_frame(&mut self, _why: &proto::ProtoError) {
        self.bad_frames = self.bad_frames.saturating_add(1);
    }

    /// Work through what has arrived. Call once a frame.
    pub fn update(&mut self, now: Instant) {
        if !matches!(self.phase, Phase::Open | Phase::Joining) {
            return;
        }
        while let Some(t) = self.transport.as_mut() {
            match t.poll() {
                Ok(Some(inc)) => {
                    // A departure marker the poll has just passed comes before this frame: a seat
                    // freed and taken again must be emptied before its new occupant is admitted.
                    self.departures();
                    if !matches!(self.phase, Phase::Open | Phase::Joining) {
                        return;
                    }
                    self.handle(inc, now);
                }
                Ok(None) => break,
                Err(TransportError::Frame(e)) => self.on_bad_frame(&e),
                Err(TransportError::Closed) => {
                    self.on_closed();
                    break;
                }
            }
            if !matches!(self.phase, Phase::Open | Phase::Joining) {
                return;
            }
        }
        if !matches!(self.phase, Phase::Open | Phase::Joining) {
            return;
        }
        self.departures();
        if !matches!(self.phase, Phase::Open | Phase::Joining) {
            return;
        }
        // A joiner that was asked for its rules and never answered is refused.
        for i in 1..MAX_SEATS {
            if self.pending[i].as_ref().is_some_and(|p| now.duration_since(p.at) > JOIN_ANSWER) {
                self.refuse(PlayerId(i as u8), Refusal::Protocol, now);
            }
        }
        if let Some(r) = self.control.close_reason() {
            self.end(close_text(&r));
            return;
        }
        let due: Vec<PlayerId> = self.kicks.iter().filter(|(_, at)| now >= *at).map(|(s, _)| *s).collect();
        if !due.is_empty() {
            self.kicks.retain(|(_, at)| now < *at);
            for s in due {
                let _ = self.control.kick(s);
            }
        }
        if !self.host && !self.welcomed && now.duration_since(self.asked) > JOIN_ANSWER {
            self.control.leave();
            self.end(Ended::NoAnswer);
        }
    }

    fn departures(&mut self) {
        for seat in self.control.take_departures() {
            self.on_departure(seat);
            if !matches!(self.phase, Phase::Open | Phase::Joining) {
                return;
            }
        }
    }

    fn on_departure(&mut self, seat: PlayerId) {
        self.kicks.retain(|(s, _)| *s != seat);
        if self.host {
            let i = usize::from(seat.0);
            if i < MAX_SEATS {
                let had = self.guests[i].take().is_some() | self.pending[i].take().is_some();
                if had {
                    self.broadcast_lobby();
                }
            }
        } else if seat == PlayerId(0) {
            self.control.leave();
            self.end(Ended::HostLeft);
        } else {
            // A fellow joiner left; the host's next `Lobby` says so.
        }
    }

    fn handle(&mut self, inc: Incoming, now: Instant) {
        let Incoming { from, msg } = inc;
        if msg.check_sender(from == PlayerId(0)).is_err() || from == self.me {
            self.bad_frames = self.bad_frames.saturating_add(1);
            return;
        }
        if self.host {
            self.host_handle(from, msg, now);
        } else {
            self.guest_handle(from, msg);
        }
    }

    // ---- host: admitting ----------------------------------------------------------

    fn host_handle(&mut self, from: PlayerId, msg: Message, now: Instant) {
        let i = usize::from(from.0);
        if i >= MAX_SEATS {
            return;
        }
        match msg {
            Message::Hello(h) => self.on_hello(from, h, now),
            Message::Rules(part) => self.on_rules(from, part, now),
            Message::SeatChoice(c) => self.on_choice(from, c),
            Message::Chat(c) => self.on_chat(from, c),
            _ => {}
        }
    }

    fn refuse(&mut self, seat: PlayerId, why: Refusal, now: Instant) {
        let i = usize::from(seat.0);
        self.pending[i] = None;
        self.send(Destination::Seat(seat), &Message::Refuse(why));
        self.kicks.push((seat, now + KICK_GRACE));
    }

    fn on_hello(&mut self, from: PlayerId, h: Hello, now: Instant) {
        let i = usize::from(from.0);
        if self.guests[i].is_some() || self.pending[i].is_some() {
            return self.refuse(from, Refusal::Protocol, now);
        }
        if self.kicks.iter().any(|(s, _)| *s == from) {
            return self.refuse(from, Refusal::Kicked, now);
        }
        if self.closed {
            return self.refuse(from, Refusal::Closed, now);
        }
        if i >= self.slots() {
            return self.refuse(from, Refusal::Full, now);
        }
        if !self.cfg.engine.matches(&h.engine) {
            return self.refuse(from, Refusal::Engine { host: self.cfg.engine.clone() }, now);
        }
        if h.rules == self.cfg.content.digest().digest() {
            return self.admit(from, h);
        }
        // The rules differ: ask for the joiner's entries to name the first key.
        self.pending[i] = Some(Pending { hello: h, assembler: RulesAssembler::default(), at: now });
        self.send(Destination::Seat(from), &Message::RulesRequest);
        self.changed = true;
    }

    fn on_rules(&mut self, from: PlayerId, part: proto::RulesPart, now: Instant) {
        let i = usize::from(from.0);
        let Some(p) = self.pending[i].as_mut() else { return };
        match p.assembler.push(part) {
            Err(_) => self.refuse(from, Refusal::Protocol, now),
            Ok(None) => {}
            Ok(Some(entries)) => {
                let theirs = p.hello.rules;
                let answer = match proto::rules_refusal(self.cfg.content.digest(), theirs, &entries) {
                    Ok(Some(r)) => r,
                    // Equal digests cannot get here; unreadable entries are a protocol error.
                    Ok(None) | Err(_) => Refusal::Protocol,
                };
                self.refuse(from, answer, now);
                self.changed = true;
            }
        }
    }

    fn admit(&mut self, from: PlayerId, h: Hello) {
        let i = usize::from(from.0);
        let mut name = clean_name(&h.name, &format!("Player {}", i + 1));
        let taken = |n: &str, s: &Session| s.cfg.name == n || s.guests.iter().flatten().any(|g| g.name == n);
        if taken(&name, self) {
            name = clean_name(&format!("{name} {}", i + 1), &name);
        }
        let token = random_u64();
        // The relay chose this seat. A computer or idle seat planned here moves to the highest open slot
        // (dropped if none), so the table the host edits is the table everyone sees.
        if matches!(self.plan[i].kind, Kind::Computer | Kind::Idle) {
            let moved = self.plan[i].clone();
            if let Some(k) = (0..self.plan.len()).rev().find(|&k| k != i && self.plan[k].kind == Kind::Open && self.guests[k].is_none()) {
                self.plan[k] = moved;
            }
            self.plan[i].kind = Kind::Open;
        }
        let plan = &self.plan[i];
        self.guests[i] = Some(Guest { name, lineage: h.lineage, packs: h.packs, engine: h.engine, race: plan.race.clone(), team: plan.team, ready: false, token });
        self.send(Destination::Seat(from), &Message::Welcome(Welcome { slot: from, host: self.me, rejoin_token: token }));
        self.broadcast_lobby();
    }

    fn on_choice(&mut self, from: PlayerId, c: SeatChoice) {
        let i = usize::from(from.0);
        let slots = self.slots();
        let Some(race) = self.cfg.races.iter().find(|r| r.eq_ignore_ascii_case(&c.race)).cloned() else { return };
        let Some(g) = self.guests[i].as_mut() else { return };
        let team = c.team.0.min(slots as u8 - 1);
        let before = (g.race.clone(), g.team, g.ready);
        g.race = race;
        g.team = team;
        g.ready = c.ready;
        if before != (g.race.clone(), g.team, g.ready) {
            self.broadcast_lobby();
        }
    }

    fn on_chat(&mut self, from: PlayerId, c: proto::Chat) {
        if c.from != from || self.guests[usize::from(from.0)].is_none() {
            return;
        }
        let text = c.text.trim().to_owned();
        if !text.is_empty() {
            let name = self.name_of(from);
            self.push_chat(ChatLine { from: name, text });
        }
    }

    fn lobby_state(&self) -> LobbyState {
        let eff = self.effective();
        let view = self.host_view();
        LobbyState {
            map: self.map_ref(),
            options: self.options.clone(),
            closed: self.closed,
            seats: eff
                .iter()
                .enumerate()
                .filter(|(_, p)| p.kind != Kind::Open)
                .map(|(i, p)| {
                    let v = &view.seats[i];
                    let g = self.guests[i].as_ref();
                    LobbySeat {
                        seat: proto::Seat {
                            slot: PlayerId(i as u8),
                            team: TeamId(v.team),
                            race: v.race.clone(),
                            occupant: match p.kind {
                                Kind::Human => Occupant::Peer,
                                Kind::Computer => Occupant::Ai(SKIRMISH_AI.into()),
                                _ => Occupant::Idle,
                            },
                            name: v.name.clone(),
                            lineage: if i == 0 { self.cfg.lineage.clone() } else { g.map_or_else(String::new, |g| g.lineage.clone()) },
                            packs: if i == 0 { self.cfg.content.packs().iter().map(|n| PackInfo { name: n.clone(), version: String::new() }).collect() } else { g.map_or_else(Vec::new, |g| g.packs.clone()) },
                        },
                        ready: v.ready,
                    }
                })
                .collect(),
        }
    }

    fn map_ref(&self) -> MapRef {
        self.cfg.maps.get(self.map).map_or(MapRef { name: String::new(), digest: 0 }, |m| MapRef { name: m.name.clone(), digest: m.digest })
    }

    fn broadcast_lobby(&mut self) {
        self.changed = true;
        if !self.host || !matches!(self.phase, Phase::Open) {
            return;
        }
        let msg = Message::Lobby(self.lobby_state());
        for i in 1..MAX_SEATS {
            if self.guests[i].is_some() {
                self.send(Destination::Seat(PlayerId(i as u8)), &msg);
            }
        }
    }

    /// Host: start the game. Closes the room to newcomers, sends one `Start` to every seated
    /// player and builds the host's own [`NetMatch`] (take it with [`Session::take_match`]).
    /// Open slots become idle seats. `Err` says why not, in plain words.
    pub fn launch(&mut self) -> Result<(), String> {
        self.update(Instant::now());
        if let Some(why) = self.launch_block() {
            return Err(why);
        }
        let eff = self.effective();
        let view = self.host_view();
        let slots = self.slots();
        let seats: Vec<proto::Seat> = eff
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let v = &view.seats[i];
                let g = self.guests[i].as_ref();
                proto::Seat {
                    slot: PlayerId(i as u8),
                    team: TeamId(v.team),
                    race: if v.race.is_empty() { self.race_for_slot(i) } else { v.race.clone() },
                    occupant: match p.kind {
                        Kind::Human => Occupant::Peer,
                        Kind::Computer => Occupant::Ai(SKIRMISH_AI.into()),
                        Kind::Open | Kind::Idle => Occupant::Idle,
                    },
                    name: if p.kind == Kind::Open { String::new() } else { v.name.clone() },
                    lineage: if i == 0 { self.cfg.lineage.clone() } else { g.map_or_else(String::new, |g| g.lineage.clone()) },
                    packs: if i == 0 { self.cfg.content.packs().iter().map(|n| PackInfo { name: n.clone(), version: String::new() }).collect() } else { g.map_or_else(Vec::new, |g| g.packs.clone()) },
                }
            })
            .collect();
        debug_assert_eq!(seats.len(), slots);
        let start = Start {
            seed: self.cfg.seed.unwrap_or_else(random_u64),
            map: self.map_ref(),
            options: self.options.clone(),
            seats,
            host: self.me,
            turn: TurnConfig::default(),
            rules: self.cfg.content.digest().digest(),
        };
        if Message::Start(start.clone()).validate().is_err() {
            return Err("The game setup is not valid.".into());
        }
        let _ = self.control.set_open(false);
        for (s, _) in std::mem::take(&mut self.kicks) {
            let _ = self.control.kick(s);
        }
        let msg = Message::Start(start.clone());
        for i in 1..MAX_SEATS {
            if self.guests[i].is_some() {
                self.send(Destination::Seat(PlayerId(i as u8)), &msg);
            }
        }
        if !matches!(self.phase, Phase::Open) {
            return Err(self.ended().map_or_else(|| "The room closed.".into(), Ended::text));
        }
        let engines = (1..MAX_SEATS).filter_map(|i| self.guests[i].as_ref().map(|g| (PlayerId(i as u8), g.engine.clone()))).collect();
        self.finish_launch(start, engines)
    }

    fn finish_launch(&mut self, start: Start, engines: Vec<(PlayerId, EngineId)>) -> Result<(), String> {
        let info = self.cfg.maps.get(self.map).or_else(|| self.cfg.map_named(&start.map.name).and_then(|i| self.cfg.maps.get(i))).ok_or("The map is gone.")?;
        let setup = GameSetup {
            seed: start.seed,
            map: info.def.clone(),
            content: self.cfg.content.clone(),
            players: start.player_setups(self.me),
            options: start.options.clone(),
        };
        let inner = self.transport.take().ok_or("The connection is closed.")?;
        let transport: Box<dyn Transport> = Box::new(Early { first: std::mem::take(&mut self.early), inner });
        let tokens = (1..MAX_SEATS).filter_map(|i| self.guests[i].as_ref().map(|g| (PlayerId(i as u8), g.token))).collect();
        self.started = Some(NetMatch { start, me: self.me, control: self.control.clone(), engines, tokens, my_token: self.my_token, setup, transport: Some(SyncCell::new(transport)) });
        self.phase = Phase::Launched;
        self.changed = true;
        Ok(())
    }

    // ---- joiner: the host's side of the conversation ---------------------------------

    fn guest_handle(&mut self, from: PlayerId, msg: Message) {
        match msg {
            Message::Welcome(w) if from == PlayerId(0) && w.slot == self.me => {
                self.welcomed = true;
                self.my_token = Some(w.rejoin_token);
                self.phase = Phase::Open;
                self.changed = true;
            }
            Message::Refuse(r) if from == PlayerId(0) => {
                self.control.leave();
                self.end(Ended::Refused(r));
            }
            Message::RulesRequest if from == PlayerId(0) => self.send_rules(),
            Message::Lobby(l) if from == PlayerId(0) && self.welcomed => {
                self.lobby = Some(l);
                self.changed = true;
            }
            Message::Start(s) if from == PlayerId(0) && self.welcomed => self.on_start(s),
            Message::Chat(c) if self.welcomed => {
                if c.from == from && (from == PlayerId(0) || self.guest_view().seats.iter().any(|x| x.slot == from.0 && x.kind == Kind::Human)) {
                    let name = self.name_of(from);
                    let text = c.text.trim().to_owned();
                    if !text.is_empty() {
                        self.push_chat(ChatLine { from: name, text });
                    }
                }
            }
            // Everything else is the match's (a peer that has started sends turns at once):
            // kept for the router.
            Message::Turn(_) | Message::Replayed(_) | Message::Ping { .. } | Message::Pong { .. } | Message::Desync(_) | Message::Sections(_) | Message::Request(_) | Message::Resume { .. } | Message::DropVote(_) | Message::Leave(_) if self.early.len() < EARLY_KEPT => {
                self.early.push_back(Incoming { from, msg });
            }
            _ => {}
        }
    }

    fn send_rules(&mut self) {
        match proto::rules_parts(&RulesIdentity::entries(self.cfg.content.digest())) {
            Ok(parts) => {
                for p in parts {
                    self.send(Destination::Seat(PlayerId(0)), &Message::Rules(p));
                }
            }
            Err(e) => {
                self.control.leave();
                self.end(Ended::Protocol(format!("your rules cannot be listed ({e})")));
            }
        }
    }

    fn on_start(&mut self, s: Start) {
        let fail = |me: &mut Session, why: &str| {
            me.control.leave();
            me.end(Ended::Protocol(why.into()));
        };
        if s.rules != self.cfg.content.digest().digest() {
            return fail(self, "the game's rules are not the ones you joined with");
        }
        let Some(index) = self.cfg.map_named(&s.map.name).filter(|&i| self.cfg.maps[i].digest == s.map.digest) else {
            return fail(self, "you do not have the host's map");
        };
        if !s.seats.iter().any(|x| x.slot == self.me && x.occupant == Occupant::Peer) {
            return fail(self, "the game has no seat for you");
        }
        self.map = index;
        let engines = Vec::new();
        if let Err(e) = self.finish_launch(s, engines) {
            fail(self, &e);
        }
    }
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

/// Open a room on the relay at `addr` and host it. Blocks up to the client's timeouts.
pub fn connect_host(addr: &str, password: &str, cfg: SessionCfg, opts: &ClientOptions) -> Result<Session, ConnectError> {
    let t = NetTransport::create(addr, password, opts)?;
    Ok(Session::host(cfg, t))
}

/// Join the room `code` on the relay at `addr`. Blocks up to the client's timeouts.
pub fn connect_guest(addr: &str, code: RoomCode, password: &str, cfg: SessionCfg, opts: &ClientOptions) -> Result<Session, ConnectError> {
    let t = NetTransport::join(addr, code, password, opts)?;
    Ok(Session::guest(cfg, t))
}

// ---------------------------------------------------------------------------
// In the app
// ---------------------------------------------------------------------------

/// What the connection screens' fields hold. Kept apart from [`Multiplayer`] so typing
/// never redraws the screen.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct MpFields {
    pub name: String,
    pub relay: String,
    pub code: String,
    pub password: String,
    pub chat: String,
    /// The field that had the keyboard when the screen was redrawn under it.
    pub focus: Option<LobbyField>,
}

/// A connection in progress (the transport's connect blocks, so it runs on a thread).
struct Connecting {
    rx: Mutex<Receiver<Result<Session, (ConnectError, String)>>>,
    host: bool,
}

/// Where a lobby session is: nothing, connecting, or in a room.
#[derive(Resource, Default)]
pub struct Multiplayer {
    connecting: Option<Connecting>,
    pub session: Option<Session>,
    /// A line in plain words for the connection screens: why the last try failed, why the
    /// room ended.
    pub notice: Option<String>,
    /// Set when the player asks to launch and it cannot; shown in the lobby.
    pub lobby_note: Option<String>,
}

impl Multiplayer {
    /// Connecting, or in a room.
    pub fn busy(&self) -> bool {
        self.connecting.is_some() || self.session.is_some()
    }

    pub fn connecting(&self) -> bool {
        self.connecting.is_some()
    }

    /// Start hosting: the connection runs on a thread; [`drive`] picks the result up.
    pub fn start_host(&mut self, addr: String, password: String, cfg: SessionCfg) {
        self.spawn(true, move || connect_host(&addr, &password, cfg, &ClientOptions::default()).map_err(|e| (e, addr)));
    }

    /// Start joining.
    pub fn start_join(&mut self, addr: String, code: RoomCode, password: String, cfg: SessionCfg) {
        self.spawn(false, move || connect_guest(&addr, code, &password, cfg, &ClientOptions::default()).map_err(|e| (e, addr)));
    }

    fn spawn(&mut self, host: bool, work: impl FnOnce() -> Result<Session, (ConnectError, String)> + Send + 'static) {
        let (tx, rx) = channel();
        self.notice = None;
        self.lobby_note = None;
        self.connecting = Some(Connecting { rx: Mutex::new(rx), host });
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
    }

    /// Leave the room (if in one) and forget the connection.
    pub fn leave(&mut self) {
        if let Some(mut s) = self.session.take() {
            s.leave();
        }
        self.connecting = None;
        self.lobby_note = None;
    }
}

/// The match a launched lobby made, for the host program to start (INT-M5): take it with
/// `Option::take`.
#[derive(Resource, Default)]
pub struct NetLaunch(pub Option<NetMatch>);

/// A lobby launched: [`NetLaunch`] holds the match.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetGameReady;

/// The screens this module draws.
pub fn covers(screen: Screen) -> bool {
    matches!(screen, Screen::Multiplayer | Screen::HostGame | Screen::JoinGame | Screen::Lobby)
}

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Multiplayer>()
            .init_resource::<MpFields>()
            .init_resource::<NetLaunch>()
            .add_message::<NetGameReady>()
            .add_systems(OnEnter(Screen::Multiplayer), init_fields);
    }
}

fn init_fields(mut fields: ResMut<MpFields>, settings: Res<Settings>) {
    if fields.name.is_empty() {
        fields.name = settings.player_name.clone();
    }
    if fields.relay.is_empty() {
        fields.relay = settings.relay.clone();
    }
}

/// Once a frame: pick up a finished connection, poll the room, move between screens when
/// it ends.
pub fn drive(
    mut mp: ResMut<Multiplayer>,
    mut next: ResMut<NextState<Screen>>,
    screen: Res<State<Screen>>,
    draft: Option<Res<SetupDraft>>,
    mut launch: ResMut<NetLaunch>,
    mut ready: MessageWriter<NetGameReady>,
    strings: Option<Res<crate::ui::i18n::Strings>>,
) {
    let words = Words(strings.as_deref());
    // Look without marking the resource changed: only a real change redraws the screen.
    let m = mp.bypass_change_detection();
    let mut changed = false;
    if let Some(c) = &m.connecting {
        let got = c.rx.lock().map_or(Err(TryRecvError::Disconnected), |rx| rx.try_recv());
        match got {
            Ok(Ok(mut session)) => {
                m.connecting = None;
                if let Some(d) = draft.as_ref().filter(|_| session.is_host()) {
                    let _ = session.set_map(d.map);
                }
                m.session = Some(session);
                m.notice = None;
                next.set(Screen::Lobby);
                changed = true;
            }
            Ok(Err((e, addr))) => {
                m.connecting = None;
                m.notice = Some(connect_error_text_in(&e, &addr, words));
                changed = true;
            }
            Err(TryRecvError::Disconnected) => {
                m.connecting = None;
                m.notice = Some("The connection attempt stopped unexpectedly.".into());
                changed = true;
            }
            Err(TryRecvError::Empty) => {}
        }
    }
    let mut back_to = None;
    if let Some(s) = m.session.as_mut() {
        s.update(Instant::now());
        if s.take_changed() {
            changed = true;
        }
        match &s.phase {
            Phase::Launched => {
                if let Some(nm) = s.take_match() {
                    launch.0 = Some(nm);
                    ready.write(NetGameReady);
                }
            }
            Phase::Ended(why) => {
                m.notice = Some(why.text_in(words));
                back_to = Some(if s.is_host() { Screen::HostGame } else { Screen::JoinGame });
            }
            _ => {}
        }
    }
    if let Some(to) = back_to {
        m.session = None;
        m.lobby_note = None;
        if matches!(*screen.get(), Screen::Lobby | Screen::MapSelect) {
            next.set(to);
        }
        changed = true;
    }
    if changed {
        mp.set_changed();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn on_buttons(
    mut clicks: MessageReader<ButtonClicked>,
    acts: Query<&MpAct>,
    mut mp: ResMut<Multiplayer>,
    fields: Res<MpFields>,
    data: Res<ScreenData>,
    mut settings: ResMut<Settings>,
    mut next: ResMut<NextState<Screen>>,
    mut browse: ResMut<Browse>,
    mut ret: ResMut<MapReturn>,
) {
    for ButtonClicked(e) in clicks.read() {
        let Ok(&act) = acts.get(*e) else { continue };
        match act {
            MpAct::Goto(s) => {
                mp.notice = None;
                next.set(s);
            }
            MpAct::StartHost | MpAct::StartJoin => {
                let join = act == MpAct::StartJoin;
                let Some(addr) = normalize_relay(&fields.relay) else {
                    mp.notice = Some("Enter the relay's address first, like 127.0.0.1:7864 (the Multiplayer screen has the field).".into());
                    continue;
                };
                let name = clean_name(&fields.name, "Player");
                if settings.player_name != name {
                    settings.player_name = name.clone();
                }
                if settings.relay != fields.relay.trim() {
                    settings.relay = fields.relay.trim().to_owned();
                }
                let cfg = SessionCfg::from_data(&data, &name);
                if join {
                    match RoomCode::parse(&fields.code) {
                        Some(code) => mp.start_join(addr, code, fields.password.clone(), cfg),
                        None => mp.notice = Some("That is not a room code. A code is six letters and digits, like K7M-Q2X.".into()),
                    }
                } else {
                    mp.start_host(addr, fields.password.clone(), cfg);
                }
            }
            MpAct::Rejoin => mp.notice = Some(rejoin_unavailable().into()),
            MpAct::Leave => {
                mp.leave();
                next.set(Screen::Multiplayer);
            }
            MpAct::Kick(i) => {
                if let Some(s) = mp.session.as_mut() {
                    s.kick(i);
                }
            }
            MpAct::ToggleOpen => {
                if let Some(s) = mp.session.as_mut() {
                    let closed = s.view().closed;
                    s.set_room_open(closed);
                }
            }
            MpAct::Launch => {
                let note = mp.session.as_mut().map(|s| s.launch().err());
                if let Some(note) = note {
                    mp.lobby_note = note;
                }
            }
            MpAct::ToggleReady => {
                if let Some(s) = mp.session.as_mut() {
                    let ready = s.view().seats.iter().find(|x| x.me).is_some_and(|x| x.ready);
                    s.set_ready(!ready);
                }
            }
            MpAct::PickMap => {
                if let Some(i) = mp.session.as_ref().and_then(|s| s.view().map_index) {
                    browse.map = i;
                }
                ret.0 = Screen::Lobby;
                next.set(Screen::MapSelect);
            }
            MpAct::CycleResources => {
                if let Some(s) = mp.session.as_mut() {
                    let mut o = s.view().options;
                    o.starting_resources = match o.starting_resources {
                        sta_content::StartingResources::Normal => sta_content::StartingResources::Lots,
                        sta_content::StartingResources::Lots => sta_content::StartingResources::Normal,
                    };
                    s.set_options(o);
                }
            }
            MpAct::CycleOfficers => {
                let count = data.content.rules().officer_limits.len();
                if let (Some(s), true) = (mp.session.as_mut(), count > 0) {
                    let mut o = s.view().options;
                    o.officer_limit = ((o.officer_limit as usize + 1) % count) as u32;
                    s.set_options(o);
                }
            }
        }
    }
}

/// A seat's dropdown changed: the session follows.
pub fn on_dropdowns(mut changes: MessageReader<DropdownChanged>, drops: Query<&view::LobbyDrop>, mut mp: ResMut<Multiplayer>) {
    for c in changes.read() {
        let Ok(&view::LobbyDrop { slot, col }) = drops.get(c.entity) else { continue };
        let Some(s) = mp.session.as_mut() else { continue };
        let seat = s.view().seats.get(slot).cloned();
        let Some(seat) = seat else { continue };
        let races = s.races().to_vec();
        match (col, s.is_host()) {
            (view::DropCol::Kind, true) => s.set_slot_kind(slot, [Kind::Open, Kind::Computer, Kind::Idle][c.index.min(2)]),
            (view::DropCol::Side, true) => {
                if let Some(r) = races.get(c.index) {
                    s.set_slot_race(slot, r);
                }
            }
            (view::DropCol::Team, true) => s.set_slot_team(slot, c.index as u8),
            (view::DropCol::Side, false) => {
                if let Some(r) = races.get(c.index) {
                    s.choose(r, seat.team);
                }
            }
            (view::DropCol::Team, false) => s.choose(&seat.race, c.index as u8),
            (view::DropCol::Kind, false) => {}
        }
    }
}

/// Typing: the fields' values are kept; Enter in the chat line says it.
pub fn on_text(mut changes: MessageReader<TextChanged>, mut submits: MessageReader<TextSubmitted>, texts: Query<(&view::LobbyField, &TextField)>, mut fields: ResMut<MpFields>, mut mp: ResMut<Multiplayer>) {
    use view::LobbyField as F;
    for c in changes.read() {
        let Ok((kind, tf)) = texts.get(c.entity) else { continue };
        let slot = match kind {
            F::Name => &mut fields.name,
            F::Relay => &mut fields.relay,
            F::Code => &mut fields.code,
            F::Password => &mut fields.password,
            F::Chat => &mut fields.chat,
        };
        if *slot != tf.text {
            slot.clone_from(&tf.text);
        }
        fields.focus = Some(*kind);
    }
    for c in submits.read() {
        let Ok((F::Chat, _)) = texts.get(c.entity) else { continue };
        let text = std::mem::take(&mut fields.chat);
        if let Some(s) = mp.session.as_mut() {
            s.say(&text);
        }
    }
}

/// The host's room follows the map picked on the map select screen.
pub fn sync_map(draft: Option<Res<SetupDraft>>, mut mp: ResMut<Multiplayer>) {
    let Some(d) = draft else { return };
    if !d.is_changed() {
        return;
    }
    let m = mp.bypass_change_detection();
    let Some(s) = m.session.as_mut().filter(|s| s.is_host()) else { return };
    if s.view().map_index != Some(d.map) {
        match s.set_map(d.map) {
            Ok(()) => m.lobby_note = None,
            Err(why) => m.lobby_note = Some(why),
        }
        mp.set_changed();
    }
}

/// Draw the multiplayer screen when it is shown or what it shows changed.
#[allow(clippy::too_many_arguments)]
pub fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<ScreenRoot>>,
    screen: Res<State<Screen>>,
    theme: Res<Theme>,
    mp: Res<Multiplayer>,
    fields: Res<MpFields>,
    data: Res<ScreenData>,
    strings: Option<Res<crate::ui::i18n::Strings>>,
    stock_box: Res<StockBox>,
    camera: Option<Res<ScreensCamera>>,
) {
    let screen = *screen.get();
    if !covers(screen) {
        return;
    }
    let changed = mp.is_changed() || theme.is_changed() || stock_box.is_changed() || data.is_changed() || strings.as_ref().is_some_and(|s| s.is_changed());
    if !roots.is_empty() && !changed {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    view::spawn(&mut commands, screen, &view::Ctx { theme: &theme, data: &data, strings: strings.as_deref(), mp: &mp, fields: &fields, b: stock_box.0 }, camera.map(|c| c.0));
}

/// Give a redrawn field the keyboard its predecessor had (a chat line keeps typing while
/// the room changes under it).
pub fn restore_focus(fields: Res<MpFields>, texts: Query<(Entity, &view::LobbyField)>, alive: Query<(), With<Focusable>>, mut focus: ResMut<KitFocus>, mut last: Local<Option<Entity>>) {
    let Some(kind) = fields.focus else {
        *last = None;
        return;
    };
    let Some((e, _)) = texts.iter().find(|(_, k)| **k == kind) else { return };
    if *last != Some(e) {
        *last = Some(e);
        if focus.0.is_none_or(|f| !alive.contains(f)) {
            focus.0 = Some(e);
        }
    }
}
