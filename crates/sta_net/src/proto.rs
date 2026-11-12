//! The lockstep wire protocol (NET-01). The design and its reasons are in
//! docs/NET.md; this module is its executable half.
//!
//! - A frame is a fixed 10-byte header, then one [`Message`] in `postcard`:
//!   `MAGIC` (`S64N`), [`PROTOCOL_VERSION`] (`u16` LE), body length (`u32`
//!   LE). The header layout is frozen for every protocol version, so two
//!   builds always learn each other's version even when nothing else
//!   decodes.
//! - A frame is at most [`MAX_FRAME_BYTES`]. Every string and list has a
//!   bound ([`Message::validate`]); [`decode`] checks them after parsing and
//!   [`encode`] refuses to send what [`decode`] would refuse to read.
//! - [`decode`] never panics: wrong magic, another version, a truncated or
//!   oversized frame, an unknown tag, trailing bytes and out-of-bound fields
//!   are all [`ProtoError`]s.
//! - Evolution: `postcard` is positional, so a changed field or a removed
//!   variant is a [`PROTOCOL_VERSION`] bump. Within one version nothing
//!   changes; there is no "skip unknown" rule. Commands ride inside
//!   [`TurnCommands`] and are versioned by `SEAM_REV`, which [`Hello`]
//!   carries and the host compares.
//! - The rules digest of docs/MODDING_DECISIONS.md D10 is [`RulesDigest`],
//!   supplied through [`RulesIdentity`]; CON-35 implements it. Until then
//!   [`ContentHashRules`] stands in with `ContentDb::hash`.

use serde::{Deserialize, Serialize};
use sta_content::MapDef;
use sta_core::{PlayerId, TeamId, Tick};
use sta_sim::command::{Command, CommandKind, MAX_MISSION_TEXT_BYTES};
use sta_sim::setup::{Control, GameOptions, PlayerSetup};

/// First four bytes of every frame.
pub const MAGIC: [u8; 4] = *b"S64N";
/// Bumped whenever any message's encoding changes.
pub const PROTOCOL_VERSION: u16 = 1;
/// Magic, version and body length.
pub const HEADER_LEN: usize = 10;
/// Largest frame, header included.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Largest body.
pub const MAX_BODY_BYTES: usize = MAX_FRAME_BYTES - HEADER_LEN;

/// Player names, in bytes of UTF-8.
pub const MAX_NAME_BYTES: usize = 32;
/// Each field of [`EngineId`].
pub const MAX_ENGINE_FIELD_BYTES: usize = 64;
/// Lineage names (`STA2 Classic`, `FO 4.0`).
pub const MAX_LINEAGE_BYTES: usize = 64;
/// Packs listed per peer (informational, D10).
pub const MAX_PACKS: usize = 16;
/// Each field of [`PackInfo`].
pub const MAX_PACK_FIELD_BYTES: usize = 64;
/// Race names and computer player names.
pub const MAX_RACE_BYTES: usize = 32;
/// Map file stems.
pub const MAX_MAP_NAME_BYTES: usize = 128;
/// Rule entries in one [`RulesPart`].
pub const MAX_RULE_ENTRIES_PER_PART: usize = 256;
/// Rule keys (`RTS_CFG.h:cfgPLANET_DEWARP_RADIUS_MULTIPLIER`).
pub const MAX_RULE_KEY_BYTES: usize = 96;
/// Canonical rendering of a rule value.
pub const MAX_RULE_VALUE_BYTES: usize = 96;
/// Commands one peer sends for one turn. A sender with more holds the rest
/// for its next turn.
pub const MAX_COMMANDS_PER_TURN: usize = 128;
/// Turn records in one [`CatchUp`] frame.
pub const MAX_CATCHUP_TURNS: usize = 256;
/// Chat text.
pub const MAX_CHAT_BYTES: usize = 256;
/// Seats in a game: [`PlayerId::MAX_PLAYERS`]; a map may allow fewer.
pub const MAX_SEATS: usize = PlayerId::MAX_PLAYERS as usize;

/// Ticks per turn unless the host's settings say otherwise (100 ms at 30 Hz).
pub const DEFAULT_TICKS_PER_TURN: u8 = 3;
/// Turns between issuing a command and applying it: Armada's normal mode.
pub const DEFAULT_DELAY_TURNS: u8 = 2;
/// The same with Armada's "High Latency" box ticked.
pub const HIGH_LATENCY_DELAY_TURNS: u8 = 5;
/// Bounds of [`TurnConfig::ticks_per_turn`].
pub const TICKS_PER_TURN_RANGE: (u8, u8) = (1, 15);
/// Bounds of [`TurnConfig::delay_turns`].
pub const DELAY_TURNS_RANGE: (u8, u8) = (1, 30);
/// Bounds of [`TurnConfig::speed_percent`].
pub const SPEED_PERCENT_RANGE: (u16, u16) = (25, 400);

/// Why a frame was refused. Every malformed input ends here, never in a
/// panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    /// Fewer bytes than the header or its declared length.
    Truncated { need: usize, have: usize },
    /// Not an STA64 frame.
    BadMagic,
    /// The peer speaks another protocol version. Its body is not read.
    Version { found: u16 },
    /// Declared or actual size over [`MAX_FRAME_BYTES`].
    TooLarge { len: usize },
    /// More bytes than the header declared.
    TrailingBytes { extra: usize },
    /// A header-only frame of our own version: only a [`version_notice`]
    /// of another version may be empty.
    Empty,
    /// The body is not a message of this version (unknown tag, bad varint,
    /// bad UTF-8, short body).
    Corrupt(String),
    /// Parsed, but a field breaks a bound or an invariant.
    Invalid(&'static str),
    /// A turn's commands claim a player or a kind the sender may not send.
    Authority(&'static str),
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::Truncated { need, have } => {
                write!(f, "truncated frame: {have} of {need} bytes")
            }
            ProtoError::BadMagic => write!(f, "not an STA64 network frame"),
            ProtoError::Version { found } => {
                write!(
                    f,
                    "peer speaks protocol {found}, this build speaks {PROTOCOL_VERSION}"
                )
            }
            ProtoError::TooLarge { len } => {
                write!(f, "frame of {len} bytes, limit {MAX_FRAME_BYTES}")
            }
            ProtoError::TrailingBytes { extra } => write!(f, "{extra} bytes after the frame"),
            ProtoError::Empty => write!(f, "empty frame"),
            ProtoError::Corrupt(e) => write!(f, "corrupt frame: {e}"),
            ProtoError::Invalid(what) => write!(f, "invalid message: {what}"),
            ProtoError::Authority(what) => write!(f, "unauthorised command: {what}"),
        }
    }
}

impl std::error::Error for ProtoError {}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Everything peers say to each other. Tags are positional: append only
/// within a version, and bump [`PROTOCOL_VERSION`] for anything else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// Joiner to host, first message on a connection.
    Hello(Hello),
    /// Host to joiner: accepted into a seat.
    Welcome(Welcome),
    /// Host to joiner: not accepted, and why. The connection closes.
    Refuse(Refusal),
    /// Host to joiner after a rules digest mismatch: send your rule entries.
    RulesRequest,
    /// One part of a peer's rule entries.
    Rules(RulesPart),
    /// Host to all, in the lobby: map, options, seats.
    Lobby(LobbyState),
    /// Joiner to host, in the lobby.
    SeatChoice(SeatChoice),
    /// Host to all: the game starts. Also the replay's setup.
    Start(Start),
    /// Every peer, once per turn, in turn order without gaps.
    Turn(TurnCommands),
    /// Host to all: a change that takes effect on a given turn.
    Schedule(Scheduled),
    /// Any peer to host: a vote on a stalled player (Armada's drop dialog).
    DropVote(DropVote),
    /// Round-trip probe; answered with [`Message::Pong`] carrying the nonce.
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    /// Lobby or in-game chat.
    Chat(Chat),
    /// A peer's digest disagreed with ours.
    Desync(DesyncNotice),
    /// A dropped connection asking for its seat back.
    Rejoin(Rejoin),
    /// Host to a rejoining peer: turn records it missed.
    CatchUp(CatchUp),
    /// Leaving on purpose.
    Leave(LeaveReason),
}

/// The build a peer runs. Peers must match exactly (docs/NET.md,
/// "Handshake").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineId {
    /// `sta64`, or a headless tool's name.
    pub name: String,
    /// Crate version (`0.1.0`).
    pub version: String,
    /// Build identity (commit), empty when unknown.
    pub build: String,
}

/// An installed pack, shown in the lobby and recorded in replays; not
/// compared (D10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackInfo {
    pub name: String,
    pub version: String,
}

/// The digest of resolved sim rules that peers compare (D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RulesDigest {
    /// Transitional: `ContentDb::hash`, until CON-35 lands.
    ContentHash(u64),
    /// CON-35's digest of resolved rule values (32 bytes, e.g. SHA-256).
    Rules([u8; 32]),
}

/// First message from a joiner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub engine: EngineId,
    /// `sta_core::SEAM_REV`: commands decode alike only on equal revisions.
    pub seam_rev: u32,
    pub rules: RulesDigest,
    /// Content lineage (`STA2 Classic`); informational.
    pub lineage: String,
    /// Informational pack list.
    pub packs: Vec<PackInfo>,
    /// Player name.
    pub name: String,
}

/// A joiner accepted into a seat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub slot: PlayerId,
    /// The host's seat, which runs computer players and the mission.
    pub host: PlayerId,
    /// Presented by [`Rejoin`] to take the seat back.
    pub rejoin_token: u64,
}

/// Why a host would not take a joiner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// No free seat.
    Full,
    /// The game has started (and this is not a [`Rejoin`]).
    InProgress,
    /// The host closed the game (Armada's "Game closed").
    Closed,
    /// Another build.
    Engine { host: EngineId },
    /// Another command seam revision.
    SeamRev { host: u32 },
    /// The peers digest rules differently ([`RulesDigest`] variants differ).
    RulesScheme,
    /// The rules differ; `key` is the first differing key (D10).
    RulesDiffer(RuleDiff),
    /// Unknown seat or token.
    BadRejoin,
    /// The host removed the player.
    Kicked,
    /// The joiner sent something malformed or out of turn.
    Protocol,
}

/// One resolved rule, in its canonical rendering (CON-35 defines it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleEntry {
    pub key: String,
    pub value: String,
}

/// The first key on which two rule lists differ. A missing side is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleDiff {
    pub key: String,
    pub host: Option<String>,
    pub joiner: Option<String>,
}

/// Part `part` of `parts` of a peer's rule entries, sorted by key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesPart {
    pub part: u16,
    pub parts: u16,
    pub entries: Vec<RuleEntry>,
}

/// A map, by name and by digest of its parsed definition ([`map_digest`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapRef {
    pub name: String,
    pub digest: u64,
}

/// Who holds a seat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Occupant {
    /// A person on a peer (the seat's own connection).
    Peer,
    /// A computer player run by the host, by `sta_ai` registry name.
    Ai(String),
    /// Nobody; the seat never sends a command.
    Idle,
}

/// One seat of a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub slot: PlayerId,
    pub team: TeamId,
    pub race: String,
    pub occupant: Occupant,
    /// Display name.
    pub name: String,
}

/// A seat in the lobby, with what its peer reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySeat {
    pub seat: Seat,
    pub ready: bool,
    pub lineage: String,
    pub packs: Vec<PackInfo>,
}

/// The host's lobby, sent whenever it changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LobbyState {
    pub map: MapRef,
    pub options: GameOptions,
    pub seats: Vec<LobbySeat>,
    /// Armada's "Game closed": no new joiners.
    pub closed: bool,
}

/// A joiner's choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatChoice {
    pub team: TeamId,
    pub race: String,
    pub ready: bool,
}

/// Turn timing, fixed at start except where [`Action`] changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnConfig {
    /// Sim ticks per turn; constant for the game.
    pub ticks_per_turn: u8,
    /// Turns from issuing a command to applying it (a sender's lead).
    pub delay_turns: u8,
    /// Wall-clock pace, percent of `TICK_HZ`. The sim does not see it.
    pub speed_percent: u16,
}

impl Default for TurnConfig {
    fn default() -> TurnConfig {
        TurnConfig {
            ticks_per_turn: DEFAULT_TICKS_PER_TURN,
            delay_turns: DEFAULT_DELAY_TURNS,
            speed_percent: 100,
        }
    }
}

impl TurnConfig {
    /// The first tick of `turn`; its commands apply on this tick.
    pub fn tick_of(&self, turn: u32) -> Tick {
        Tick::from(turn) * Tick::from(self.ticks_per_turn)
    }

    /// The turn that contains `tick`.
    pub fn turn_of(&self, tick: Tick) -> u64 {
        tick / Tick::from(self.ticks_per_turn.max(1))
    }

    fn validate(&self) -> Result<(), ProtoError> {
        in_range(self.ticks_per_turn, TICKS_PER_TURN_RANGE, "ticks_per_turn")?;
        in_range(self.delay_turns, DELAY_TURNS_RANGE, "delay_turns")?;
        in_range(self.speed_percent, SPEED_PERCENT_RANGE, "speed_percent")
    }
}

/// The game starts: everything every peer needs to build the same
/// `GameSetup`, given its own content and map files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Start {
    pub seed: u64,
    pub map: MapRef,
    pub options: GameOptions,
    /// Ascending slot.
    pub seats: Vec<Seat>,
    /// The seat that runs computer players and the mission (D3).
    pub host: PlayerId,
    pub turn: TurnConfig,
    /// The host's rules digest, lineage and packs, for the replay (D10).
    pub rules: RulesDigest,
    pub lineage: String,
    pub packs: Vec<PackInfo>,
}

impl Start {
    /// The players as the peer in seat `me` sets up its sim: its own seat
    /// is human; computer players run on the host only and arrive like
    /// remote input everywhere else.
    pub fn player_setups(&self, me: PlayerId) -> Vec<PlayerSetup> {
        self.seats
            .iter()
            .map(|s| PlayerSetup {
                id: s.slot,
                team: s.team,
                race: s.race.clone(),
                control: match &s.occupant {
                    Occupant::Peer if s.slot == me => Control::Human,
                    Occupant::Peer => Control::Remote,
                    Occupant::Ai(name) if me == self.host => Control::Ai(name.clone()),
                    Occupant::Ai(_) => Control::Remote,
                    Occupant::Idle => Control::Idle,
                },
            })
            .collect()
    }

    /// Whether the host may send commands as `player`: computer seats and
    /// the mission script (D3).
    pub fn host_speaks_for(&self, player: PlayerId) -> bool {
        player == PlayerId::SCRIPT
            || self
                .seats
                .iter()
                .any(|s| s.slot == player && matches!(s.occupant, Occupant::Ai(_)))
    }
}

/// A sim digest at a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickDigest {
    pub tick: Tick,
    pub digest: u64,
}

/// One peer's commands for one turn. Every peer sends one per turn, empty
/// or not, in order and without gaps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnCommands {
    pub turn: u32,
    /// The sending seat; the transport checks it is the connection's.
    pub slot: PlayerId,
    /// Sorted by `(player, seq)`.
    pub commands: Vec<Command>,
    /// The sender's digest at the first tick of the latest turn it has
    /// run, if it has run one since its last report.
    pub digest: Option<TickDigest>,
}

impl TurnCommands {
    /// Checks that `sender` may send these commands: its own seat's, and,
    /// for the host, those `host_speaks_for` allows. `Edit` comes only from
    /// [`PlayerId::SCRIPT`] via the host, never from a player or the bench.
    pub fn authorize(
        &self,
        sender: PlayerId,
        is_host: bool,
        host_speaks_for: impl Fn(PlayerId) -> bool,
    ) -> Result<(), ProtoError> {
        if self.slot != sender {
            return Err(ProtoError::Authority("turn sent for another seat"));
        }
        for c in &self.commands {
            let own = c.player == sender && sender.is_player();
            if !own && !(is_host && host_speaks_for(c.player)) {
                return Err(ProtoError::Authority(
                    "command for a seat the sender does not hold",
                ));
            }
            if matches!(c.kind, CommandKind::Edit(_)) && c.player != PlayerId::SCRIPT {
                return Err(ProtoError::Authority("edit from a player"));
            }
        }
        Ok(())
    }
}

/// A change every peer applies at the start of `at_turn`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scheduled {
    pub at_turn: u32,
    /// Who asked for it (for "X paused the game").
    pub by: PlayerId,
    pub action: Action,
}

/// What a [`Scheduled`] change does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Pause,
    Resume,
    /// New [`TurnConfig::delay_turns`] ("changed the network latency").
    SetDelay(u8),
    /// New [`TurnConfig::speed_percent`] ("changed the game speed").
    SetSpeed(u16),
    /// The seat sends no more turns after `last_turn`; from `last_turn + 1`
    /// it is eliminated or played by the host's AI.
    Drop {
        slot: PlayerId,
        last_turn: u32,
        then: DropChoice,
    },
}

/// Armada II's two answers to a stalled player.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DropChoice {
    Eliminate,
    /// A computer player, by `sta_ai` registry name.
    AiTakeover(String),
}

/// A vote on a stalled seat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropVote {
    pub slot: PlayerId,
    /// `None`: keep waiting.
    pub choice: Option<DropChoice>,
}

/// Armada's chat scopes, kept for display; delivery follows `recipients`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatScope {
    All,
    Allies,
    Enemies,
    Private,
}

/// A chat line. The sender resolves its scope to seats from its own view;
/// the relay delivers only to `recipients`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chat {
    pub from: PlayerId,
    pub scope: ChatScope,
    /// Bit `n` set: seat `n` receives it.
    pub recipients: u8,
    pub text: String,
}

/// A peer found a digest that is not its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesyncNotice {
    /// The peer whose report disagreed.
    pub slot: PlayerId,
    pub ours: TickDigest,
    pub theirs: TickDigest,
}

/// A lost connection coming back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejoin {
    pub slot: PlayerId,
    pub token: u64,
    /// The next turn it needs, or `None` when it lost its sim and replays
    /// the game from tick 0.
    pub next_turn: Option<u32>,
}

/// Turn records for a rejoining peer, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatchUp {
    pub turns: Vec<TurnCommands>,
    /// Scheduled changes in the same span.
    pub schedule: Vec<Scheduled>,
    /// Last frame of the catch-up; live turns follow.
    pub done: bool,
}

/// Why a peer left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeaveReason {
    Quit,
    /// Back to the menus ("is in the shell").
    ToShell,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn in_range<T: PartialOrd>(v: T, (lo, hi): (T, T), what: &'static str) -> Result<(), ProtoError> {
    if v < lo || v > hi {
        Err(ProtoError::Invalid(what))
    } else {
        Ok(())
    }
}

fn short(s: &str, max: usize, what: &'static str) -> Result<(), ProtoError> {
    if s.len() > max {
        Err(ProtoError::Invalid(what))
    } else {
        Ok(())
    }
}

fn few<T>(v: &[T], max: usize, what: &'static str) -> Result<(), ProtoError> {
    if v.len() > max {
        Err(ProtoError::Invalid(what))
    } else {
        Ok(())
    }
}

fn seat_id(p: PlayerId) -> Result<(), ProtoError> {
    if p.0 < PlayerId::MAX_PLAYERS {
        Ok(())
    } else {
        Err(ProtoError::Invalid("seat out of range"))
    }
}

fn packs(p: &[PackInfo]) -> Result<(), ProtoError> {
    few(p, MAX_PACKS, "too many packs")?;
    for p in p {
        short(&p.name, MAX_PACK_FIELD_BYTES, "pack name")?;
        short(&p.version, MAX_PACK_FIELD_BYTES, "pack version")?;
    }
    Ok(())
}

fn seat(s: &Seat) -> Result<(), ProtoError> {
    seat_id(s.slot)?;
    short(&s.race, MAX_RACE_BYTES, "race")?;
    short(&s.name, MAX_NAME_BYTES, "player name")?;
    if let Occupant::Ai(n) = &s.occupant {
        short(n, MAX_RACE_BYTES, "AI name")?;
    }
    Ok(())
}

fn seats_ascending(slots: impl Iterator<Item = PlayerId>) -> Result<(), ProtoError> {
    let mut last: Option<PlayerId> = None;
    for s in slots {
        if last.is_some_and(|l| l >= s) {
            return Err(ProtoError::Invalid("seats not ascending"));
        }
        last = Some(s);
    }
    Ok(())
}

fn drop_choice(c: &DropChoice) -> Result<(), ProtoError> {
    match c {
        DropChoice::Eliminate => Ok(()),
        DropChoice::AiTakeover(n) => short(n, MAX_RACE_BYTES, "AI name"),
    }
}

fn turn(t: &TurnCommands) -> Result<(), ProtoError> {
    seat_id(t.slot)?;
    few(
        &t.commands,
        MAX_COMMANDS_PER_TURN,
        "too many commands in a turn",
    )?;
    if t.commands
        .windows(2)
        .any(|w| w[0].sort_key() >= w[1].sort_key())
    {
        return Err(ProtoError::Invalid("commands not in (player, seq) order"));
    }
    for c in &t.commands {
        if c.player == PlayerId::BENCH {
            return Err(ProtoError::Invalid("bench command on the wire"));
        }
        if let CommandKind::MissionText { text } = &c.kind {
            short(text, MAX_MISSION_TEXT_BYTES, "mission text")?;
        }
    }
    Ok(())
}

fn scheduled(s: &Scheduled) -> Result<(), ProtoError> {
    match &s.action {
        Action::Pause | Action::Resume => Ok(()),
        Action::SetDelay(d) => in_range(*d, DELAY_TURNS_RANGE, "delay_turns"),
        Action::SetSpeed(p) => in_range(*p, SPEED_PERCENT_RANGE, "speed_percent"),
        Action::Drop { slot, then, .. } => {
            seat_id(*slot)?;
            drop_choice(then)
        }
    }
}

impl Message {
    /// Checks every bound and structural invariant that does not need game
    /// context. [`decode`] runs it on every frame; [`encode`] before sending.
    pub fn validate(&self) -> Result<(), ProtoError> {
        match self {
            Message::Hello(h) => {
                for f in [&h.engine.name, &h.engine.version, &h.engine.build] {
                    short(f, MAX_ENGINE_FIELD_BYTES, "engine id")?;
                }
                short(&h.lineage, MAX_LINEAGE_BYTES, "lineage")?;
                short(&h.name, MAX_NAME_BYTES, "player name")?;
                packs(&h.packs)
            }
            Message::Welcome(w) => {
                seat_id(w.slot)?;
                seat_id(w.host)
            }
            Message::Refuse(r) => match r {
                Refusal::Engine { host } => {
                    for f in [&host.name, &host.version, &host.build] {
                        short(f, MAX_ENGINE_FIELD_BYTES, "engine id")?;
                    }
                    Ok(())
                }
                Refusal::RulesDiffer(d) => {
                    short(&d.key, MAX_RULE_KEY_BYTES, "rule key")?;
                    for v in [&d.host, &d.joiner].into_iter().flatten() {
                        short(v, MAX_RULE_VALUE_BYTES, "rule value")?;
                    }
                    Ok(())
                }
                _ => Ok(()),
            },
            Message::RulesRequest => Ok(()),
            Message::Rules(p) => {
                if p.parts == 0 || p.part >= p.parts {
                    return Err(ProtoError::Invalid("rules part out of range"));
                }
                few(
                    &p.entries,
                    MAX_RULE_ENTRIES_PER_PART,
                    "too many rule entries",
                )?;
                for e in &p.entries {
                    short(&e.key, MAX_RULE_KEY_BYTES, "rule key")?;
                    short(&e.value, MAX_RULE_VALUE_BYTES, "rule value")?;
                }
                if p.entries
                    .windows(2)
                    .any(|w| w[0].key.as_bytes() >= w[1].key.as_bytes())
                {
                    return Err(ProtoError::Invalid("rule keys not ascending"));
                }
                Ok(())
            }
            Message::Lobby(l) => {
                short(&l.map.name, MAX_MAP_NAME_BYTES, "map name")?;
                few(&l.seats, MAX_SEATS, "too many seats")?;
                for s in &l.seats {
                    seat(&s.seat)?;
                    short(&s.lineage, MAX_LINEAGE_BYTES, "lineage")?;
                    packs(&s.packs)?;
                }
                seats_ascending(l.seats.iter().map(|s| s.seat.slot))
            }
            Message::SeatChoice(c) => short(&c.race, MAX_RACE_BYTES, "race"),
            Message::Start(s) => {
                short(&s.map.name, MAX_MAP_NAME_BYTES, "map name")?;
                few(&s.seats, MAX_SEATS, "too many seats")?;
                if s.seats.is_empty() {
                    return Err(ProtoError::Invalid("no seats"));
                }
                for x in &s.seats {
                    seat(x)?;
                }
                seats_ascending(s.seats.iter().map(|x| x.slot))?;
                if !s
                    .seats
                    .iter()
                    .any(|x| x.slot == s.host && x.occupant == Occupant::Peer)
                {
                    return Err(ProtoError::Invalid("host is not a seated peer"));
                }
                s.turn.validate()?;
                short(&s.lineage, MAX_LINEAGE_BYTES, "lineage")?;
                packs(&s.packs)
            }
            Message::Turn(t) => turn(t),
            Message::Schedule(s) => scheduled(s),
            Message::DropVote(v) => {
                seat_id(v.slot)?;
                v.choice.as_ref().map_or(Ok(()), drop_choice)
            }
            Message::Ping { .. } | Message::Pong { .. } => Ok(()),
            Message::Chat(c) => {
                seat_id(c.from)?;
                if c.recipients == 0 {
                    return Err(ProtoError::Invalid("chat without recipients"));
                }
                short(&c.text, MAX_CHAT_BYTES, "chat text")
            }
            Message::Desync(d) => seat_id(d.slot),
            Message::Rejoin(r) => seat_id(r.slot),
            Message::CatchUp(c) => {
                few(&c.turns, MAX_CATCHUP_TURNS, "too many catch-up turns")?;
                few(&c.schedule, MAX_CATCHUP_TURNS, "too many catch-up changes")?;
                c.turns.iter().try_for_each(turn)?;
                c.schedule.iter().try_for_each(scheduled)?;
                let key = |t: &TurnCommands| (t.turn, t.slot);
                if c.turns.windows(2).any(|w| key(&w[0]) >= key(&w[1])) {
                    return Err(ProtoError::Invalid("catch-up turns not ascending"));
                }
                Ok(())
            }
            Message::Leave(_) => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

fn header(version: u16, body_len: usize) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..4].copy_from_slice(&MAGIC);
    h[4..6].copy_from_slice(&version.to_le_bytes());
    // Callers bound body_len by MAX_BODY_BYTES, far below u32::MAX.
    h[6..10].copy_from_slice(&(body_len as u32).to_le_bytes());
    h
}

/// Reads a header: `(version, body length)`. Checks magic and the length
/// bound, not the version.
fn read_header(bytes: &[u8]) -> Result<(u16, usize), ProtoError> {
    if bytes.len() < HEADER_LEN {
        if bytes.len() >= 4 && bytes[..4] != MAGIC {
            return Err(ProtoError::BadMagic);
        }
        return Err(ProtoError::Truncated {
            need: HEADER_LEN,
            have: bytes.len(),
        });
    }
    if bytes[..4] != MAGIC {
        return Err(ProtoError::BadMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    let len = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    if len > MAX_BODY_BYTES {
        return Err(ProtoError::TooLarge {
            len: len.saturating_add(HEADER_LEN),
        });
    }
    Ok((version, len))
}

/// One message as a frame. Refuses messages [`decode`] would refuse.
pub fn encode(msg: &Message) -> Result<Vec<u8>, ProtoError> {
    msg.validate()?;
    let body = postcard::to_allocvec(msg).map_err(|e| ProtoError::Corrupt(e.to_string()))?;
    if body.len() > MAX_BODY_BYTES {
        return Err(ProtoError::TooLarge {
            len: body.len() + HEADER_LEN,
        });
    }
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(&header(PROTOCOL_VERSION, body.len()));
    out.extend_from_slice(&body);
    Ok(out)
}

/// Reads exactly one frame.
pub fn decode(frame: &[u8]) -> Result<Message, ProtoError> {
    let (version, len) = read_header(frame)?;
    if version != PROTOCOL_VERSION {
        return Err(ProtoError::Version { found: version });
    }
    let have = frame.len() - HEADER_LEN;
    if have < len {
        return Err(ProtoError::Truncated {
            need: HEADER_LEN + len,
            have: frame.len(),
        });
    }
    if have > len {
        return Err(ProtoError::TrailingBytes { extra: have - len });
    }
    if len == 0 {
        return Err(ProtoError::Empty);
    }
    let (msg, rest) = postcard::take_from_bytes::<Message>(&frame[HEADER_LEN..])
        .map_err(|e| ProtoError::Corrupt(e.to_string()))?;
    if !rest.is_empty() {
        return Err(ProtoError::Corrupt(format!(
            "{} bytes left in the body",
            rest.len()
        )));
    }
    msg.validate()?;
    Ok(msg)
}

/// For stream transports: the length of the frame that starts `prefix`,
/// once its header is in (`None` before). Errors on bad magic, another
/// version (after which the stream is not read further) or an oversized
/// declared length, so a reader never buffers more than [`MAX_FRAME_BYTES`].
pub fn frame_len(prefix: &[u8]) -> Result<Option<usize>, ProtoError> {
    match read_header(prefix) {
        Err(ProtoError::Truncated { .. }) => Ok(None),
        Err(e) => Err(e),
        Ok((version, _)) if version != PROTOCOL_VERSION => {
            Err(ProtoError::Version { found: version })
        }
        Ok((_, len)) => Ok(Some(HEADER_LEN + len)),
    }
}

/// A header-only frame naming this build's version: what a peer sends
/// before closing a connection that spoke another version, so both sides
/// can tell their players which versions met.
pub fn version_notice() -> Vec<u8> {
    header(PROTOCOL_VERSION, 0).to_vec()
}

// ---------------------------------------------------------------------------
// Rules identity (D10) and map identity
// ---------------------------------------------------------------------------

/// What a peer compares at the handshake (D10). CON-35 implements it for
/// its typed `Rules`; until then [`ContentHashRules`] does.
pub trait RulesIdentity {
    /// The digest of resolved sim-visible rule values.
    fn digest(&self) -> RulesDigest;
    /// The resolved rules, sorted bytewise by key, keys unique: sent when
    /// digests differ, so both sides can name the first differing key.
    fn entries(&self) -> Vec<RuleEntry>;
}

/// The transitional rules identity: `ContentDb::hash` and nothing finer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentHashRules(pub u64);

impl RulesIdentity for ContentHashRules {
    fn digest(&self) -> RulesDigest {
        RulesDigest::ContentHash(self.0)
    }

    fn entries(&self) -> Vec<RuleEntry> {
        vec![RuleEntry {
            key: "content.hash".into(),
            value: format!("{:016x}", self.0),
        }]
    }
}

/// The first key where two sorted entry lists differ, by key or value.
pub fn first_difference(host: &[RuleEntry], joiner: &[RuleEntry]) -> Option<RuleDiff> {
    let (mut h, mut j) = (host.iter().peekable(), joiner.iter().peekable());
    loop {
        match (h.peek(), j.peek()) {
            (None, None) => return None,
            (Some(a), Some(b)) if a.key == b.key => {
                if a.value != b.value {
                    return Some(RuleDiff {
                        key: a.key.clone(),
                        host: Some(a.value.clone()),
                        joiner: Some(b.value.clone()),
                    });
                }
                h.next();
                j.next();
            }
            (Some(a), None) => {
                return Some(RuleDiff {
                    key: a.key.clone(),
                    host: Some(a.value.clone()),
                    joiner: None,
                });
            }
            (Some(a), Some(b)) if a.key.as_bytes() < b.key.as_bytes() => {
                return Some(RuleDiff {
                    key: a.key.clone(),
                    host: Some(a.value.clone()),
                    joiner: None,
                });
            }
            (_, Some(b)) => {
                return Some(RuleDiff {
                    key: b.key.clone(),
                    host: None,
                    joiner: Some(b.value.clone()),
                });
            }
        }
    }
}

/// Splits sorted entries into [`RulesPart`]s that each fit a frame.
/// Entries over the key or value bound are truncated to it on a character
/// boundary (the digest, not this list, is what peers agree on).
pub fn rules_parts(entries: &[RuleEntry]) -> Vec<RulesPart> {
    fn clip(s: &str, max: usize) -> String {
        let mut end = s.len().min(max);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    }
    let chunks: Vec<&[RuleEntry]> = if entries.is_empty() {
        vec![&[]]
    } else {
        entries.chunks(MAX_RULE_ENTRIES_PER_PART).collect()
    };
    let parts = chunks.len().min(usize::from(u16::MAX)) as u16;
    chunks
        .into_iter()
        .take(usize::from(parts))
        .enumerate()
        .map(|(i, c)| RulesPart {
            part: i as u16,
            parts,
            entries: c
                .iter()
                .map(|e| RuleEntry {
                    key: clip(&e.key, MAX_RULE_KEY_BYTES),
                    value: clip(&e.value, MAX_RULE_VALUE_BYTES),
                })
                .collect(),
        })
        .collect()
}

/// FNV-1a 64 over the map's `postcard` encoding: equal on every peer that
/// parsed the same map with the same seam revision.
pub fn map_digest(map: &MapDef) -> u64 {
    let bytes = postcard::to_allocvec(map).unwrap_or_default();
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sta_core::{EntityId, Vec3};
    use sta_sim::command::{Edit, Order};

    fn engine() -> EngineId {
        EngineId {
            name: "sta64".into(),
            version: "0.1.0".into(),
            build: "abc123".into(),
        }
    }

    fn packs_fixture() -> Vec<PackInfo> {
        vec![
            PackInfo {
                name: "STA2 Classic".into(),
                version: "1.1".into(),
            },
            PackInfo {
                name: "HD textures".into(),
                version: "".into(),
            },
        ]
    }

    fn cmd(player: u8, seq: u32) -> Command {
        Command {
            player: PlayerId(player),
            seq,
            kind: CommandKind::Order(Order::Move {
                units: vec![EntityId(4), EntityId(5)],
                to: Vec3::from_f32([1.5, 0.0, -2.0]),
                queue: true,
            }),
        }
    }

    fn seat(slot: u8, occupant: Occupant) -> Seat {
        Seat {
            slot: PlayerId(slot),
            team: TeamId(slot),
            race: "Federation".into(),
            occupant,
            name: format!("p{slot}"),
        }
    }

    fn start() -> Start {
        Start {
            seed: 0x5eed,
            map: MapRef {
                name: "a2_borg01".into(),
                digest: 42,
            },
            options: GameOptions::default(),
            seats: vec![
                seat(0, Occupant::Peer),
                seat(1, Occupant::Peer),
                seat(2, Occupant::Ai("normal".into())),
            ],
            host: PlayerId(0),
            turn: TurnConfig::default(),
            rules: RulesDigest::Rules([7; 32]),
            lineage: "STA2 Classic".into(),
            packs: packs_fixture(),
        }
    }

    fn turn_msg() -> TurnCommands {
        TurnCommands {
            turn: 17,
            slot: PlayerId(1),
            commands: vec![cmd(1, 3), cmd(1, 4)],
            digest: Some(TickDigest {
                tick: 45,
                digest: 0xdead_beef,
            }),
        }
    }

    /// One of every message, including every enum variant inside.
    fn samples() -> Vec<Message> {
        let diff = RuleDiff {
            key: "RTS_CFG.h:cfgFREIGHTERS_PER_PLANET".into(),
            host: Some("5".into()),
            joiner: None,
        };
        let mut v = vec![
            Message::Hello(Hello {
                engine: engine(),
                seam_rev: sta_core::SEAM_REV,
                rules: RulesDigest::ContentHash(9),
                lineage: "FO 4.0".into(),
                packs: packs_fixture(),
                name: "Cedric".into(),
            }),
            Message::Welcome(Welcome {
                slot: PlayerId(1),
                host: PlayerId(0),
                rejoin_token: u64::MAX,
            }),
            Message::RulesRequest,
            Message::Rules(RulesPart {
                part: 0,
                parts: 1,
                entries: vec![
                    RuleEntry {
                        key: "a".into(),
                        value: "1".into(),
                    },
                    RuleEntry {
                        key: "b".into(),
                        value: "2.5".into(),
                    },
                ],
            }),
            Message::Lobby(LobbyState {
                map: MapRef {
                    name: "m".into(),
                    digest: 1,
                },
                options: GameOptions::default(),
                seats: vec![LobbySeat {
                    seat: seat(0, Occupant::Idle),
                    ready: true,
                    lineage: "STA2".into(),
                    packs: packs_fixture(),
                }],
                closed: false,
            }),
            Message::SeatChoice(SeatChoice {
                team: TeamId(2),
                race: "Borg".into(),
                ready: false,
            }),
            Message::Start(start()),
            Message::Turn(turn_msg()),
            Message::Turn(TurnCommands {
                turn: 0,
                slot: PlayerId(0),
                commands: vec![],
                digest: None,
            }),
            Message::DropVote(DropVote {
                slot: PlayerId(3),
                choice: None,
            }),
            Message::DropVote(DropVote {
                slot: PlayerId(3),
                choice: Some(DropChoice::Eliminate),
            }),
            Message::Ping { nonce: 1 },
            Message::Pong { nonce: 1 },
            Message::Desync(DesyncNotice {
                slot: PlayerId(2),
                ours: TickDigest {
                    tick: 90,
                    digest: 1,
                },
                theirs: TickDigest {
                    tick: 90,
                    digest: 2,
                },
            }),
            Message::Rejoin(Rejoin {
                slot: PlayerId(1),
                token: 5,
                next_turn: Some(100),
            }),
            Message::Rejoin(Rejoin {
                slot: PlayerId(1),
                token: 5,
                next_turn: None,
            }),
            Message::CatchUp(CatchUp {
                turns: vec![
                    turn_msg(),
                    TurnCommands {
                        turn: 18,
                        ..turn_msg()
                    },
                ],
                schedule: vec![Scheduled {
                    at_turn: 18,
                    by: PlayerId(0),
                    action: Action::Pause,
                }],
                done: true,
            }),
            Message::Leave(LeaveReason::Quit),
            Message::Leave(LeaveReason::ToShell),
        ];
        for r in [
            Refusal::Full,
            Refusal::InProgress,
            Refusal::Closed,
            Refusal::Engine { host: engine() },
            Refusal::SeamRev { host: 3 },
            Refusal::RulesScheme,
            Refusal::RulesDiffer(diff),
            Refusal::BadRejoin,
            Refusal::Kicked,
            Refusal::Protocol,
        ] {
            v.push(Message::Refuse(r));
        }
        for action in [
            Action::Pause,
            Action::Resume,
            Action::SetDelay(HIGH_LATENCY_DELAY_TURNS),
            Action::SetSpeed(150),
            Action::Drop {
                slot: PlayerId(1),
                last_turn: 40,
                then: DropChoice::Eliminate,
            },
            Action::Drop {
                slot: PlayerId(1),
                last_turn: 40,
                then: DropChoice::AiTakeover("hard".into()),
            },
        ] {
            v.push(Message::Schedule(Scheduled {
                at_turn: 50,
                by: PlayerId(1),
                action,
            }));
        }
        for scope in [
            ChatScope::All,
            ChatScope::Allies,
            ChatScope::Enemies,
            ChatScope::Private,
        ] {
            v.push(Message::Chat(Chat {
                from: PlayerId(0),
                scope,
                recipients: 0b10,
                text: "gl hf".into(),
            }));
        }
        v
    }

    #[test]
    fn every_message_round_trips() {
        for m in samples() {
            let bytes = encode(&m).unwrap_or_else(|e| panic!("{m:?}: {e}"));
            assert_eq!(&bytes[..4], b"S64N");
            assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), PROTOCOL_VERSION);
            let len = u32::from_le_bytes(bytes[6..10].try_into().unwrap()) as usize;
            assert_eq!(len, bytes.len() - HEADER_LEN);
            assert_eq!(frame_len(&bytes), Ok(Some(bytes.len())));
            assert_eq!(decode(&bytes), Ok(m));
        }
    }

    #[test]
    fn header_is_little_endian_and_frozen() {
        let bytes = encode(&Message::Ping { nonce: 0 }).unwrap();
        // Ping is tag 11, nonce 0: two body bytes.
        assert_eq!(bytes, [b'S', b'6', b'4', b'N', 1, 0, 2, 0, 0, 0, 11, 0]);
        assert_eq!(version_notice(), [b'S', b'6', b'4', b'N', 1, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn every_truncation_is_an_error() {
        for m in samples() {
            let bytes = encode(&m).unwrap();
            for n in 0..bytes.len() {
                assert!(
                    decode(&bytes[..n]).is_err(),
                    "{m:?} cut to {n} bytes decoded"
                );
            }
            for n in 0..HEADER_LEN {
                assert_eq!(frame_len(&bytes[..n]), Ok(None));
            }
        }
    }

    #[test]
    fn trailing_bytes_and_inflated_lengths_are_errors() {
        let mut bytes = encode(&Message::Turn(turn_msg())).unwrap();
        bytes.push(0);
        assert_eq!(decode(&bytes), Err(ProtoError::TrailingBytes { extra: 1 }));
        // Header length grown by one to cover the extra byte: body leftover.
        let len = (bytes.len() - HEADER_LEN) as u32;
        bytes[6..10].copy_from_slice(&len.to_le_bytes());
        assert!(matches!(decode(&bytes), Err(ProtoError::Corrupt(_))));
    }

    #[test]
    fn oversized_frames_are_refused_before_reading() {
        let mut h = version_notice();
        h[6..10].copy_from_slice(&(MAX_BODY_BYTES as u32 + 1).to_le_bytes());
        assert!(matches!(frame_len(&h), Err(ProtoError::TooLarge { .. })));
        assert!(matches!(decode(&h), Err(ProtoError::TooLarge { .. })));
        h[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(decode(&h), Err(ProtoError::TooLarge { .. })));
        // A message too big to send is refused at the sender too.
        let huge = Command {
            player: PlayerId(1),
            seq: 0,
            kind: CommandKind::Order(Order::Stop {
                units: (20_000..60_000).map(EntityId).collect(),
            }),
        };
        let m = Message::Turn(TurnCommands {
            turn: 0,
            slot: PlayerId(1),
            commands: vec![huge],
            digest: None,
        });
        assert!(matches!(encode(&m), Err(ProtoError::TooLarge { .. })));
    }

    #[test]
    fn other_versions_and_magic_are_named() {
        let mut bytes = encode(&Message::Ping { nonce: 3 }).unwrap();
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(decode(&bytes), Err(ProtoError::Version { found: 2 }));
        assert_eq!(frame_len(&bytes), Err(ProtoError::Version { found: 2 }));
        // A version notice from another build reads as its version.
        let mut notice = version_notice();
        notice[4..6].copy_from_slice(&7u16.to_le_bytes());
        assert_eq!(decode(&notice), Err(ProtoError::Version { found: 7 }));
        // Our own version with no body is not a message.
        assert_eq!(decode(&version_notice()), Err(ProtoError::Empty));
        bytes[0] = b'X';
        assert_eq!(decode(&bytes), Err(ProtoError::BadMagic));
        assert_eq!(decode(b"XXXX"), Err(ProtoError::BadMagic));
        assert_eq!(
            decode(b""),
            Err(ProtoError::Truncated {
                need: HEADER_LEN,
                have: 0
            })
        );
    }

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut v = header(PROTOCOL_VERSION, body.len()).to_vec();
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn unknown_tags_and_bad_bodies_are_corrupt() {
        assert!(matches!(
            decode(&frame(&[200])),
            Err(ProtoError::Corrupt(_))
        ));
        // Hello whose first string claims 2^28 bytes.
        assert!(matches!(
            decode(&frame(&[0, 0x80, 0x80, 0x80, 0x80, 0x01])),
            Err(ProtoError::Corrupt(_))
        ));
        // Chat with invalid UTF-8 text.
        assert!(matches!(
            decode(&frame(&[13, 0, 0, 1, 2, 0xff, 0xfe])),
            Err(ProtoError::Corrupt(_))
        ));
    }

    #[test]
    fn bounds_are_enforced_on_both_sides() {
        let long = "x".repeat(MAX_CHAT_BYTES + 1);
        let chat = Message::Chat(Chat {
            from: PlayerId(0),
            scope: ChatScope::All,
            recipients: 1,
            text: long,
        });
        assert_eq!(encode(&chat), Err(ProtoError::Invalid("chat text")));
        // Bypass encode: the decoder enforces the same bound.
        let body = postcard::to_allocvec(&chat).unwrap();
        assert_eq!(decode(&frame(&body)), Err(ProtoError::Invalid("chat text")));

        let bad = [
            Message::Chat(Chat {
                from: PlayerId(0),
                scope: ChatScope::All,
                recipients: 0,
                text: String::new(),
            }),
            Message::Welcome(Welcome {
                slot: PlayerId(8),
                host: PlayerId(0),
                rejoin_token: 0,
            }),
            Message::Schedule(Scheduled {
                at_turn: 0,
                by: PlayerId(0),
                action: Action::SetDelay(0),
            }),
            Message::Schedule(Scheduled {
                at_turn: 0,
                by: PlayerId(0),
                action: Action::SetSpeed(1000),
            }),
            Message::Rules(RulesPart {
                part: 1,
                parts: 1,
                entries: vec![],
            }),
            Message::Rules(RulesPart {
                part: 0,
                parts: 1,
                entries: vec![
                    RuleEntry {
                        key: "b".into(),
                        value: String::new(),
                    },
                    RuleEntry {
                        key: "a".into(),
                        value: String::new(),
                    },
                ],
            }),
            Message::Turn(TurnCommands {
                commands: vec![cmd(1, 4), cmd(1, 3)],
                ..turn_msg()
            }),
            Message::Turn(TurnCommands {
                commands: vec![cmd(255, 0)],
                slot: PlayerId(1),
                ..turn_msg()
            }),
            Message::Turn(TurnCommands {
                commands: (0..=MAX_COMMANDS_PER_TURN as u32)
                    .map(|s| cmd(1, s))
                    .collect(),
                ..turn_msg()
            }),
            Message::Start(Start {
                host: PlayerId(2),
                ..start()
            }),
            Message::Start(Start {
                seats: vec![],
                ..start()
            }),
            Message::Start(Start {
                turn: TurnConfig {
                    ticks_per_turn: 0,
                    ..TurnConfig::default()
                },
                ..start()
            }),
            Message::Start(Start {
                seats: vec![seat(1, Occupant::Peer), seat(0, Occupant::Peer)],
                ..start()
            }),
            Message::CatchUp(CatchUp {
                turns: vec![turn_msg(), turn_msg()],
                schedule: vec![],
                done: false,
            }),
        ];
        for m in bad {
            assert!(
                matches!(encode(&m), Err(ProtoError::Invalid(_))),
                "{m:?} encoded"
            );
            let body = postcard::to_allocvec(&m).unwrap();
            assert!(
                matches!(decode(&frame(&body)), Err(ProtoError::Invalid(_))),
                "{m:?} decoded"
            );
        }
    }

    #[test]
    fn authority_follows_seats_and_d3() {
        let s = start();
        let speaks = |p| s.host_speaks_for(p);
        let mine = turn_msg();
        assert_eq!(mine.authorize(PlayerId(1), false, speaks), Ok(()));
        assert!(
            mine.authorize(PlayerId(0), true, speaks).is_err(),
            "turn for another seat"
        );

        // The host sends for its own seat, computer seats and the script.
        let edit = Command {
            player: PlayerId::SCRIPT,
            seq: 0,
            kind: CommandKind::Edit(Edit::Kill(EntityId(3))),
        };
        let host_turn = TurnCommands {
            turn: 1,
            slot: PlayerId(0),
            commands: vec![cmd(0, 0), cmd(2, 0), edit.clone()],
            digest: None,
        };
        assert_eq!(host_turn.authorize(PlayerId(0), true, speaks), Ok(()));
        // A non-host may not speak for the AI or the script...
        let t = TurnCommands {
            slot: PlayerId(1),
            commands: vec![cmd(2, 0)],
            ..host_turn.clone()
        };
        assert!(t.authorize(PlayerId(1), false, speaks).is_err());
        let t = TurnCommands {
            slot: PlayerId(1),
            commands: vec![edit],
            ..host_turn.clone()
        };
        assert!(t.authorize(PlayerId(1), false, speaks).is_err());
        // ...nor the host for another person's seat, and players never edit.
        let t = TurnCommands {
            commands: vec![cmd(1, 0)],
            ..host_turn.clone()
        };
        assert!(t.authorize(PlayerId(0), true, speaks).is_err());
        let own_edit = Command {
            player: PlayerId(0),
            seq: 1,
            kind: CommandKind::Edit(Edit::Kill(EntityId(3))),
        };
        let t = TurnCommands {
            commands: vec![own_edit],
            ..host_turn
        };
        assert_eq!(
            t.authorize(PlayerId(0), true, speaks),
            Err(ProtoError::Authority("edit from a player"))
        );
    }

    #[test]
    fn start_maps_seats_to_local_control() {
        let s = start();
        let host: Vec<_> = s
            .player_setups(PlayerId(0))
            .into_iter()
            .map(|p| p.control)
            .collect();
        assert_eq!(
            host,
            [
                Control::Human,
                Control::Remote,
                Control::Ai("normal".into())
            ]
        );
        let joiner: Vec<_> = s
            .player_setups(PlayerId(1))
            .into_iter()
            .map(|p| p.control)
            .collect();
        assert_eq!(joiner, [Control::Remote, Control::Human, Control::Remote]);
        assert_eq!(s.turn.tick_of(10), 30);
        assert_eq!(s.turn.turn_of(32), 10);
    }

    fn e(k: &str, v: &str) -> RuleEntry {
        RuleEntry {
            key: k.into(),
            value: v.into(),
        }
    }

    #[test]
    fn first_difference_names_the_key() {
        let classic = [e("cfgA", "1"), e("cfgB", "2"), e("cfgC", "3")];
        assert_eq!(first_difference(&classic, &classic), None);
        let enhanced = [e("cfgA", "1"), e("cfgB", "4"), e("cfgC", "3")];
        assert_eq!(
            first_difference(&classic, &enhanced),
            Some(RuleDiff {
                key: "cfgB".into(),
                host: Some("2".into()),
                joiner: Some("4".into())
            })
        );
        let missing = [e("cfgA", "1"), e("cfgC", "3")];
        assert_eq!(first_difference(&classic, &missing).unwrap().joiner, None);
        assert_eq!(first_difference(&missing, &classic).unwrap().host, None);
        assert_eq!(
            first_difference(&classic, &classic[..2]).unwrap().key,
            "cfgC"
        );
        assert_eq!(first_difference(&[], &classic).unwrap().key, "cfgA");

        let a = ContentHashRules(1);
        let b = ContentHashRules(2);
        assert_ne!(a.digest(), b.digest());
        assert_eq!(
            first_difference(&a.entries(), &b.entries()).unwrap().key,
            "content.hash"
        );
    }

    #[test]
    fn rules_parts_fit_frames_and_reassemble() {
        let entries: Vec<_> = (0..1000)
            .map(|i| {
                e(
                    &format!("RTS_CFG.h:key{i:05}{}", "k".repeat(80)),
                    &"9".repeat(200),
                )
            })
            .collect();
        let parts = rules_parts(&entries);
        assert_eq!(parts.len(), 4);
        let mut back = Vec::new();
        for p in parts {
            let m = Message::Rules(p);
            assert_eq!(decode(&encode(&m).unwrap()), Ok(m.clone()));
            let Message::Rules(p) = m else { unreachable!() };
            back.extend(p.entries);
        }
        assert_eq!(back.len(), 1000);
        assert!(back.iter().all(|r| r.key.len() <= MAX_RULE_KEY_BYTES && r.value.len() == MAX_RULE_VALUE_BYTES));
        assert_eq!(rules_parts(&[]).len(), 1);
    }

    #[test]
    fn map_digest_tracks_content() {
        let a = sta_content::fake::two_bases();
        let mut b = a.clone();
        assert_eq!(map_digest(&a), map_digest(&b));
        b.name.push('x');
        assert_ne!(map_digest(&a), map_digest(&b));
    }

    /// xorshift64*: deterministic, no dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
    }

    /// Random bodies and mutated valid frames: decode returns, never
    /// panics, and whatever it accepts survives another round trip.
    #[test]
    fn fuzz_decode_never_panics() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let valid: Vec<Vec<u8>> = samples().iter().map(|m| encode(m).unwrap()).collect();
        let mut accepted = 0;
        for i in 0..20_000 {
            let mut bytes = if i % 2 == 0 {
                let n = (rng.next() % 96) as usize;
                let body: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                frame(&body)
            } else {
                valid[(rng.next() as usize) % valid.len()].clone()
            };
            for _ in 0..=(rng.next() % 4) {
                let at = HEADER_LEN + (rng.next() as usize) % (bytes.len() - HEADER_LEN).max(1);
                if at < bytes.len() {
                    bytes[at] = rng.next() as u8;
                }
            }
            if let Ok(m) = decode(&bytes) {
                accepted += 1;
                assert_eq!(decode(&encode(&m).unwrap()), Ok(m));
            }
        }
        assert!(accepted > 0, "the fuzzer never produced a valid frame");
    }
}
