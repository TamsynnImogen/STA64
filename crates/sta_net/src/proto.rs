//! The lockstep wire protocol (NET-01). The design and its reasons are in
//! docs/NET.md; this module is its executable half.
//!
//! - A frame is a fixed 10-byte header, then one [`Message`] in `postcard`:
//!   `MAGIC` (`S64N`), [`PROTOCOL_VERSION`] (`u16` LE), body length (`u32`
//!   LE). The header layout is frozen for every protocol version, so two
//!   builds always learn each other's version even when nothing else
//!   decodes, and a relay can forward frames of any version
//!   ([`peek_header`]).
//! - A frame is at most [`MAX_FRAME_BYTES`]. Every string and list has a
//!   bound ([`Message::validate`]); [`decode`] checks them after parsing and
//!   [`encode`] refuses to send what [`decode`] would refuse to read.
//! - [`decode`] never panics: wrong magic, another version, a truncated or
//!   oversized frame, an unknown tag, trailing bytes and out-of-bound fields
//!   are all [`ProtoError`]s.
//! - Who may send what is [`Message::role`]; a turn record's commands are
//!   checked by [`TurnCommands::authorize`].
//! - Evolution: `postcard` is positional, so any layout change, even an
//!   appended variant, is a [`PROTOCOL_VERSION`] bump. The messages embed
//!   sim types (`Command`, `GameOptions`); the golden test pins their bytes,
//!   so a change there fails it and forces the bump.
//! - The rules digest of docs/MODDING_DECISIONS.md D10 is [`RulesDigest`],
//!   supplied through [`RulesIdentity`], which the content database's keyed
//!   digest implements (`ContentDb::digest`, CON-35); [`rules_refusal`] is
//!   the host's comparison.

use serde::{Deserialize, Serialize};
use sta_content::MapDef;
use sta_core::{PlayerId, TeamId, Tick};
use sta_sim::command::{Command, CommandKind, MAX_MISSION_TEXT_BYTES};
use sta_sim::setup::{Control, GameOptions, PlayerSetup};

/// First four bytes of every frame.
pub const MAGIC: [u8; 4] = *b"S64N";
/// Bumped whenever any message's encoding changes.
pub const PROTOCOL_VERSION: u16 = 5;
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
/// Parts of one peer's rule list: at most 16,384 entries in all, so a host
/// never holds more than 64 frames for an unaccepted joiner.
pub const MAX_RULE_PARTS: u16 = 64;
/// Rule keys (`RTS_CFG.h:cfgPLANET_DEWARP_RADIUS_MULTIPLIER`). Longer keys
/// are refused by [`wire_entries`]; CON-35 keeps its keys within this.
pub const MAX_RULE_KEY_BYTES: usize = 96;
/// Rule values on the wire. A longer value travels as [`hashed_value`].
pub const MAX_RULE_VALUE_BYTES: usize = 96;
/// Commands in one turn record. [`TurnCommands::fill`] also bounds the
/// record by bytes; whatever does not fit waits for the next turn.
pub const MAX_COMMANDS_PER_TURN: usize = 256;
/// Scheduled changes in one turn record.
pub const MAX_SCHEDULE_PER_TURN: usize = 8;
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
    /// A message or command the sender may not send.
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
            ProtoError::Authority(what) => write!(f, "not allowed: {what}"),
        }
    }
}

impl std::error::Error for ProtoError {}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Everything peers say to each other. Tags are positional: any change,
/// appending included, bumps [`PROTOCOL_VERSION`].
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
    /// Joiner to host: one part of its rule entries.
    Rules(RulesPart),
    /// Host to all, in the lobby: map, options, seats.
    Lobby(LobbyState),
    /// Joiner to host, in the lobby.
    SeatChoice(SeatChoice),
    /// Host to all: the game starts. Also the replay's setup.
    Start(Start),
    /// Every peer, one per turn, in turn order without gaps.
    Turn(TurnCommands),
    /// Non-host to host: please schedule this (pause, speed, delay).
    Request(Action),
    /// Host to all: resume a paused game, effective on receipt.
    Resume {
        by: PlayerId,
    },
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
    /// A lost connection asking for its seat back.
    Rejoin(Rejoin),
    /// Host to a rejoining peer: accepted; replayed records follow.
    Rejoined(Rejoined),
    /// Host to a rejoining peer: a turn record it missed, any seat's.
    Replayed(TurnCommands),
    /// Host to a rejoining peer: the catch-up is complete; live play follows.
    CatchUpDone,
    /// Leaving on purpose.
    Leave(LeaveReason),
    /// Protocol 2: a peer's section digests for one turn, sent once after a
    /// digest mismatch there (NET_DECISIONS N6).
    Sections(SectionReport),
    /// Protocol 3 (NET-04): the host offers the state of the game at a turn
    /// boundary, to one peer that asked (a rejoin or a resync). Parts follow.
    SnapshotOffer(SnapshotOffer),
    /// Protocol 3: one numbered part of the offered snapshot.
    SnapshotPart(SnapshotPart),
    /// Protocol 3: the host will not send a snapshot, and why.
    SnapshotRefused(SnapshotRefusal),
    /// Protocol 3: a diverged peer asks the host for the authority's state
    /// (NET_DECISIONS N2, N3). Not a rejoin: the connection never left.
    ResyncRequest(ResyncRequest),
    /// Protocol 3: a peer that loaded a snapshot says which digest it
    /// stands at, and the host checks it against its own.
    ResyncAck(ResyncAck),
    /// Protocol 3: host to all after a resync: which seat, at which turn, how
    /// often so far. Every player is told (N3); the last one lets the
    /// peers that were stopped by the desync play on.
    Resynced(Resynced),
    /// Protocol 3: host to all after a rejoin: the seat is back at `turn`.
    Arrived(Arrived),
    /// Protocol 4 (NET-05, NET_DECISIONS N5): a referee asking the host to
    /// let it into the room, with the same identity a joiner shows. The
    /// host answers `Welcome` (the slot is the connection's, not a player's)
    /// or `Refuse`, exactly as for a `Hello`.
    RefereeHello(Hello),
    /// Protocol 4: host to all: the connection in `slot` is the room's
    /// referee, or `None`: there is none (any more). A referee is never a
    /// player's seat. Peers believe a `Reference` only from the slot the
    /// host named here.
    RefereeSeated { slot: Option<PlayerId> },
    /// Protocol 4: referee to all: its sim digest for one turn boundary, sent
    /// for every turn. Peers compare it as they compare a seat's.
    Reference(TickDigest),
}

/// Who may send a message, and to whom (docs/NET.md, section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Only the host sends it.
    Host,
    /// Only a non-host sends it, and only to the host.
    ToHost,
    /// Any seated peer, to the recipients it names.
    Any,
    /// Only the room's referee sends it (protocol 4).
    Referee,
}

/// Who sent a message, as the receiver knows it: the relay's host flag, and
/// the slot the host named as the referee (`RefereeSeated`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderKind {
    Host,
    Peer,
    Referee,
}

/// The build a peer runs. Peers must match exactly (docs/NET.md,
/// "Builds").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineId {
    /// `sta64`, or a headless tool's name.
    pub name: String,
    /// Crate version (`0.1.0`).
    pub version: String,
    /// Build identity (commit plus a dirty flag); empty when unknown.
    pub build: String,
}

impl EngineId {
    /// Whether two builds may play together: equal in every field, and
    /// both builds known. Two unknown builds never match, since nothing
    /// says their sims are alike.
    pub fn matches(&self, other: &EngineId) -> bool {
        !self.build.is_empty() && self == other
    }
}

/// An installed pack, shown in the lobby and recorded in replays; not
/// compared (D10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackInfo {
    pub name: String,
    pub version: String,
}

/// The digest of resolved sim rules that peers compare (D10): 32 bytes,
/// room for SHA-256. CON-35 defines its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesDigest(pub [u8; 32]);

/// First message from a joiner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub engine: EngineId,
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
    /// Presented by [`Rejoin`] to take the seat back. Sent to the joiner
    /// only.
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
    /// Another build, or a build of unknown identity.
    Engine { host: EngineId },
    /// The rules differ; `key` is the first differing key (D10).
    RulesDiffer(RuleDiff),
    /// The digests differ but no listed entry does: one side's
    /// [`RulesIdentity::entries`] leaves out something its digest covers.
    RulesUnlisted,
    /// Unknown seat or token, or the seat was dropped.
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

/// Part `part` of `parts` of a peer's rule entries, as [`wire_entries`]
/// renders them, sorted by key across all parts.
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

/// One seat of a game, with what its peer reported (D10: lineage and packs
/// are shown and recorded, not compared).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub slot: PlayerId,
    pub team: TeamId,
    pub race: String,
    pub occupant: Occupant,
    /// Display name.
    pub name: String,
    /// The peer's lineage and packs; empty for computer and idle seats.
    pub lineage: String,
    pub packs: Vec<PackInfo>,
}

/// A seat in the lobby.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySeat {
    pub seat: Seat,
    pub ready: bool,
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
        Tick::from(turn) * Tick::from(self.ticks_per_turn.max(1))
    }

    /// The turn that contains `tick` (saturating far past any real game).
    pub fn turn_of(&self, tick: Tick) -> u32 {
        u32::try_from(tick / Tick::from(self.ticks_per_turn.max(1))).unwrap_or(u32::MAX)
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
    /// Ascending slot, with every peer's lineage and packs for the replay.
    pub seats: Vec<Seat>,
    /// The seat that runs computer players and the mission (WORKPLAN
    /// assumption D3).
    pub host: PlayerId,
    pub turn: TurnConfig,
    /// The rules digest every seated peer matched (D10).
    pub rules: RulesDigest,
}

impl Start {
    /// The players as the peer in seat `me` sets up its sim: its own seat
    /// is human; computer players run on the host only and arrive like
    /// remote input everywhere else, but every peer keeps them as AI seats
    /// (`Control::RemoteAi`): which seats are AI is simulation state.
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
                    Occupant::Ai(name) => Control::RemoteAi(name.clone()),
                    Occupant::Idle => Control::Idle,
                },
            })
            .collect()
    }

    /// Whether the host may send commands as `player` at the start of the
    /// game: computer seats and the mission script (assumption D3). After
    /// an AI takeover the router widens this to the taken-over seat.
    pub fn host_speaks_for(&self, player: PlayerId) -> bool {
        player == PlayerId::SCRIPT
            || self
                .seats
                .iter()
                .any(|s| s.slot == player && matches!(s.occupant, Occupant::Ai(_)))
    }
}

/// A sim digest at a tick: `Sim::digest` with the sim at `tick`, before
/// that tick's commands apply (as `sta_sim::replay` records it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickDigest {
    pub tick: Tick,
    pub digest: u64,
}

/// Most section digests one [`SectionReport`] carries: room for sections the
/// sim adds later, small enough that a hostile peer's report stays tiny.
pub const MAX_SECTIONS: usize = 16;

/// A peer's per-section digests (`sta_sim::digest::SectionDigests::values`,
/// in that order) for the state at `tick`, the first tick of a turn, as in
/// [`TickDigest`]. Sent to everyone once per mismatching turn, after
/// `Desync`; a peer keeps at most [`DIGESTS_KEPT`](crate::lockstep::DIGESTS_KEPT)
/// ticks of them per seat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionReport {
    pub tick: Tick,
    /// One digest per section, at least one, at most [`MAX_SECTIONS`].
    pub digests: Vec<u64>,
}

/// One peer's record for one turn. Every peer sends one per turn from
/// turn `delay_turns` on (earlier turns are empty by definition), in order
/// and without gaps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnCommands {
    pub turn: u32,
    /// The sending seat; receivers check it against the relay's stamp.
    pub slot: PlayerId,
    /// Sorted by `(player, seq)`.
    pub commands: Vec<Command>,
    /// Host only: changes that take effect at the start of this turn, in
    /// order. Riding in the host's own stream, they can never arrive after
    /// a peer has run the turn, and the replay keeps them.
    pub schedule: Vec<Scheduled>,
    /// The sender's digest at the first tick of the latest turn it has
    /// run, if it has run one since its last report.
    pub digest: Option<TickDigest>,
}

/// Body bytes [`TurnCommands::fill`] leaves free for everything but the
/// commands (tag, turn, slot, list lengths, digest) and the schedule.
const TURN_OVERHEAD_BYTES: usize = 64;

impl TurnCommands {
    /// Builds the record for `turn`, taking from the front of `pending`
    /// as many commands as fit by count and by bytes. Commands that do not
    /// fit stay in `pending` for the next turn. A single command too large
    /// for any frame is removed and returned, to be refused locally.
    pub fn fill(
        turn: u32,
        slot: PlayerId,
        schedule: Vec<Scheduled>,
        digest: Option<TickDigest>,
        pending: &mut Vec<Command>,
    ) -> (TurnCommands, Vec<Command>) {
        let schedule_bytes = postcard::to_allocvec(&schedule).map_or(MAX_BODY_BYTES, |b| b.len());
        let budget = MAX_BODY_BYTES.saturating_sub(TURN_OVERHEAD_BYTES + schedule_bytes);
        let (mut used, mut take) = (0, 0);
        let mut too_big = Vec::new();
        let mut i = 0;
        while i < pending.len() && take < MAX_COMMANDS_PER_TURN {
            let len = postcard::to_allocvec(&pending[i]).map_or(usize::MAX, |b| b.len());
            if len > budget {
                too_big.push(pending.remove(i));
                continue;
            }
            if used + len > budget {
                break;
            }
            used += len;
            take += 1;
            i += 1;
        }
        let commands = pending.drain(..take).collect();
        (
            TurnCommands {
                turn,
                slot,
                commands,
                schedule,
                digest,
            },
            too_big,
        )
    }

    /// Checks that `sender` may send this record: its own seat's commands,
    /// and, for the host, those `host_speaks_for` allows and the schedule.
    /// `Edit` comes only as [`PlayerId::SCRIPT`] from the host, never from
    /// a player or the bench.
    pub fn authorize(
        &self,
        sender: PlayerId,
        is_host: bool,
        host_speaks_for: impl Fn(PlayerId) -> bool,
    ) -> Result<(), ProtoError> {
        if self.slot != sender {
            return Err(ProtoError::Authority("turn sent for another seat"));
        }
        if !is_host && !self.schedule.is_empty() {
            return Err(ProtoError::Authority("schedule from a non-host"));
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

/// A change in the host's turn record, effective at that turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scheduled {
    /// Who asked for it (for "X paused the game"); display only.
    pub by: PlayerId,
    pub action: Action,
}

/// What a [`Scheduled`] change or a [`Message::Request`] does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Stop before running this turn; [`Message::Resume`] ends it.
    Pause,
    /// New [`TurnConfig::delay_turns`] ("changed the network latency").
    SetDelay(u8),
    /// New [`TurnConfig::speed_percent`] ("changed the game speed").
    SetSpeed(u16),
    /// Host only. The seat's records up to `last_turn` count, later ones are
    /// discarded; from `last_turn + 1` it is eliminated or played by the
    /// host's AI. The host decides it only after the relay has closed the
    /// seat's connection (docs/NET.md, section 8).
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

/// A lost connection coming back, with the same identity a joiner shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejoin {
    pub hello: Hello,
    pub slot: PlayerId,
    pub token: u64,
    /// The first turn it lacks, or `None` when it lost its sim and replays
    /// the game from tick 0 (the host then sends [`Message::Start`] first).
    pub next_turn: Option<u32>,
}

/// The host took a seat back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejoined {
    /// The last of the seat's own records the host has; the peer resends
    /// its kept records after it, or sends empty ones if it lost them.
    pub last_turn: Option<u32>,
    /// The token for the next rejoin; the old one is spent.
    pub rejoin_token: u64,
}

/// Most bytes of one snapshot a peer will accept (N4.2: bounded). A sim
/// state past it is refused by the host (`SnapshotRefusal::TooLarge`), not
/// split further.
pub const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;

/// Most bytes in one [`SnapshotPart`]: half a frame, so a part never
/// approaches [`MAX_FRAME_BYTES`].
pub const MAX_SNAPSHOT_PART_BYTES: usize = 32 * 1024;

/// Most parts of one snapshot: `MAX_SNAPSHOT_BYTES / MAX_SNAPSHOT_PART_BYTES`.
pub const MAX_SNAPSHOT_PARTS: u16 = 256;

/// Most turn records (any seat's) that follow a snapshot: every seat's
/// router queue is bounded by `MAX_QUEUED_TURNS` (128).
pub const MAX_CATCH_UP_RECORDS: usize = MAX_SEATS * 128;

/// Most `(sender, player)` pairs of a router's last-seen `seq` table: every
/// seat as a sender, for every seat and the script as the player.
pub const MAX_SEQ_SEEN: usize = MAX_SEATS * (MAX_SEATS + 1);

/// Why a snapshot is sent. The two paths share the transfer and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotKind {
    /// A seat whose connection was lost comes back (NET.md section 8).
    Rejoin,
    /// A peer that desynced is repaired (NET_DECISIONS N2).
    Resync,
}

/// One seat's place in the router the snapshot belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatSeed {
    pub slot: PlayerId,
    /// The turn this seat's next record must be for.
    pub next_turn: u32,
    /// After a drop: the last turn the seat has a record for.
    pub last_turn: Option<u32>,
    /// Records of this seat that follow the snapshot as `Replayed` frames,
    /// for turns `RouterSeed::turn` on.
    pub queued: u16,
}

/// The last `seq` the host's router accepted from `sender` for `player`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqSeen {
    pub sender: PlayerId,
    pub player: PlayerId,
    pub seq: u32,
}

/// What a router carries besides the sim: everything that earlier turns
/// changed in it (delay, takeovers, last `seq`s, pending resumes) so a peer
/// that starts from a snapshot releases the next turns exactly as the
/// others do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterSeed {
    /// The next turn the router releases: the snapshot's turn.
    pub turn: u32,
    pub cfg: TurnConfig,
    pub seats: Vec<SeatSeed>,
    /// The seats the host speaks for besides the script.
    pub host_speaks: Vec<PlayerId>,
    pub seq_seen: Vec<SeqSeen>,
    /// `Resume` messages received and not yet matched with a pause.
    pub resumes: u32,
}

/// The host announces a snapshot of the game at `tick`, a turn boundary,
/// before the sim runs it. Carries what the receiver verifies a loaded
/// snapshot against: the sim digest and the section digests (N6), so a bad
/// snapshot is refused with the differing section named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotOffer {
    pub kind: SnapshotKind,
    pub tick: Tick,
    /// `sta_sim::snapshot::FORMAT_VERSION` of the bytes.
    pub format: u16,
    /// `ContentDb::hash` the snapshot was taken with.
    pub content_hash: u64,
    /// `Sim::digest` at `tick`.
    pub digest: u64,
    /// `SectionDigests::values` at `tick`, at most [`MAX_SECTIONS`].
    pub sections: Vec<u64>,
    /// Total snapshot bytes, at most [`MAX_SNAPSHOT_BYTES`].
    pub len: u32,
    /// Bytes in every part but the last, `1..=MAX_SNAPSHOT_PART_BYTES`.
    pub chunk: u32,
    /// `len.div_ceil(chunk)`, at most [`MAX_SNAPSHOT_PARTS`].
    pub parts: u16,
    /// FNV-1a 64 of the bytes: catches a damaged transfer (it does not
    /// authenticate the host; NET.md section 8).
    pub hash: u64,
    pub seed: RouterSeed,
}

/// One part of a snapshot, in order from 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPart {
    pub index: u16,
    pub data: Vec<u8>,
}

/// Why the host does not send a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotRefusal {
    /// Another transfer is running; try again later.
    Busy,
    /// Asked again too soon after the last attempt.
    RateLimited,
    /// The seat has used its resyncs or rejoins; it is a stalled seat now
    /// (NET_DECISIONS N4.5).
    LimitReached,
    /// The request does not match a known divergence from the host.
    NotDiverged,
    /// The state is over [`MAX_SNAPSHOT_BYTES`].
    TooLarge,
    /// The host could not reach a usable turn boundary in time.
    Timeout,
    /// This host does not serve snapshots.
    Unsupported,
    /// Protocol 4: the referee will not resync the host: a host that differs
    /// from the referee ends the match with reports (NET-05; the policy is
    /// `referee::RefereePolicy::resync_host`).
    HostEndsMatch,
}

/// A diverged peer's request (see [`Message::ResyncRequest`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResyncRequest {
    /// The first tick at which this peer's digest differed.
    pub tick: Tick,
    /// This peer's digest at `tick`.
    pub digest: u64,
    /// The next turn this peer's router releases. The host waits until it
    /// holds every seat's records up to it, so nothing this peer already
    /// consumed is lost.
    pub position: u32,
}

/// A peer's check of a loaded snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResyncAck {
    pub tick: Tick,
    /// `Sim::digest` of the loaded sim.
    pub digest: u64,
}

/// The host resynced `slot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resynced {
    pub slot: PlayerId,
    /// The turn the seat resumed at.
    pub turn: u32,
    /// How many times this seat has been resynced in this match.
    pub count: u8,
    /// No other seat is still being resynced: peers stopped by the desync
    /// play on.
    pub last: bool,
}

/// The host took `slot` back after a rejoin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arrived {
    pub slot: PlayerId,
    /// The turn the seat resumed at.
    pub turn: u32,
}

/// Why a peer left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeaveReason {
    Quit,
    /// Back to the menus ("is in the shell").
    ToShell,
}

// ---------------------------------------------------------------------------
// Validation and roles
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

fn engine(e: &EngineId) -> Result<(), ProtoError> {
    for f in [&e.name, &e.version, &e.build] {
        short(f, MAX_ENGINE_FIELD_BYTES, "engine id")?;
    }
    Ok(())
}

fn packs(p: &[PackInfo]) -> Result<(), ProtoError> {
    few(p, MAX_PACKS, "too many packs")?;
    for p in p {
        short(&p.name, MAX_PACK_FIELD_BYTES, "pack name")?;
        short(&p.version, MAX_PACK_FIELD_BYTES, "pack version")?;
    }
    Ok(())
}

fn hello(h: &Hello) -> Result<(), ProtoError> {
    engine(&h.engine)?;
    short(&h.lineage, MAX_LINEAGE_BYTES, "lineage")?;
    short(&h.name, MAX_NAME_BYTES, "player name")?;
    packs(&h.packs)
}

fn seat(s: &Seat) -> Result<(), ProtoError> {
    seat_id(s.slot)?;
    short(&s.race, MAX_RACE_BYTES, "race")?;
    short(&s.name, MAX_NAME_BYTES, "player name")?;
    short(&s.lineage, MAX_LINEAGE_BYTES, "lineage")?;
    packs(&s.packs)?;
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

fn action(a: &Action) -> Result<(), ProtoError> {
    match a {
        Action::Pause => Ok(()),
        Action::SetDelay(d) => in_range(*d, DELAY_TURNS_RANGE, "delay_turns"),
        Action::SetSpeed(p) => in_range(*p, SPEED_PERCENT_RANGE, "speed_percent"),
        Action::Drop { slot, then, .. } => {
            seat_id(*slot)?;
            drop_choice(then)
        }
    }
}

fn turn(t: &TurnCommands) -> Result<(), ProtoError> {
    seat_id(t.slot)?;
    few(
        &t.commands,
        MAX_COMMANDS_PER_TURN,
        "too many commands in a turn",
    )?;
    few(
        &t.schedule,
        MAX_SCHEDULE_PER_TURN,
        "too many scheduled changes",
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
        if let CommandKind::Edit(e) = &c.kind {
            use sta_sim::command::Edit;
            match e {
                Edit::Name(_, s) | Edit::UnitRace(_, s) | Edit::PlayerRace(_, s) => short(s, sta_sim::attrs::MAX_NAME_BYTES, "attribute text")?,
                _ => {}
            }
        }
    }
    for s in &t.schedule {
        seat_id(s.by)?;
        action(&s.action)?;
    }
    Ok(())
}

fn snapshot_offer(o: &SnapshotOffer) -> Result<(), ProtoError> {
    few(&o.sections, MAX_SECTIONS, "too many sections")?;
    let (len, chunk) = (o.len as usize, o.chunk as usize);
    if len == 0 || len > MAX_SNAPSHOT_BYTES {
        return Err(ProtoError::Invalid("snapshot size"));
    }
    if chunk == 0 || chunk > MAX_SNAPSHOT_PART_BYTES {
        return Err(ProtoError::Invalid("snapshot chunk size"));
    }
    if o.parts == 0 || o.parts > MAX_SNAPSHOT_PARTS || usize::from(o.parts) != len.div_ceil(chunk) {
        return Err(ProtoError::Invalid("snapshot part count"));
    }
    let seed = &o.seed;
    seed.cfg.validate()?;
    few(&seed.seats, MAX_SEATS, "too many seats")?;
    few(&seed.host_speaks, MAX_SEATS, "too many host seats")?;
    few(&seed.seq_seen, MAX_SEQ_SEEN, "too many seq entries")?;
    seats_ascending(seed.seats.iter().map(|s| s.slot))?;
    let mut records = 0usize;
    for s in &seed.seats {
        seat_id(s.slot)?;
        records += usize::from(s.queued);
        if usize::from(s.queued) > 128 {
            return Err(ProtoError::Invalid("too many queued records"));
        }
    }
    if records > MAX_CATCH_UP_RECORDS {
        return Err(ProtoError::Invalid("too many catch-up records"));
    }
    for p in &seed.host_speaks {
        seat_id(*p)?;
    }
    for e in &seed.seq_seen {
        seat_id(e.sender)?;
        if e.player != PlayerId::SCRIPT {
            seat_id(e.player)?;
        }
    }
    Ok(())
}

fn rules_part(p: &RulesPart) -> Result<(), ProtoError> {
    if p.parts == 0 || p.parts > MAX_RULE_PARTS || p.part >= p.parts {
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

impl Message {
    /// Checks every bound and structural invariant that does not need game
    /// context. [`decode`] runs it on every frame; [`encode`] before sending.
    pub fn validate(&self) -> Result<(), ProtoError> {
        match self {
            Message::Hello(h) => hello(h),
            Message::Welcome(w) => {
                seat_id(w.slot)?;
                seat_id(w.host)
            }
            Message::Refuse(r) => match r {
                Refusal::Engine { host } => engine(host),
                Refusal::RulesDiffer(d) => {
                    short(&d.key, MAX_RULE_KEY_BYTES, "rule key")?;
                    for v in [&d.host, &d.joiner].into_iter().flatten() {
                        short(v, MAX_RULE_VALUE_BYTES, "rule value")?;
                    }
                    Ok(())
                }
                _ => Ok(()),
            },
            Message::RulesRequest | Message::CatchUpDone | Message::Leave(_) => Ok(()),
            Message::Rules(p) => rules_part(p),
            Message::Lobby(l) => {
                short(&l.map.name, MAX_MAP_NAME_BYTES, "map name")?;
                few(&l.seats, MAX_SEATS, "too many seats")?;
                for s in &l.seats {
                    seat(&s.seat)?;
                }
                seats_ascending(l.seats.iter().map(|s| s.seat.slot))
            }
            Message::SeatChoice(c) => short(&c.race, MAX_RACE_BYTES, "race"),
            Message::Start(s) => {
                short(&s.map.name, MAX_MAP_NAME_BYTES, "map name")?;
                few(&s.seats, MAX_SEATS, "too many seats")?;
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
                s.turn.validate()
            }
            Message::Turn(t) | Message::Replayed(t) => turn(t),
            Message::Request(a) => match a {
                Action::Drop { .. } => Err(ProtoError::Invalid("drops are voted, not requested")),
                a => action(a),
            },
            Message::Resume { by } => seat_id(*by),
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
            Message::Sections(r) => {
                if r.digests.is_empty() {
                    return Err(ProtoError::Invalid("section report without digests"));
                }
                few(&r.digests, MAX_SECTIONS, "too many sections")
            }
            Message::Rejoin(r) => {
                hello(&r.hello)?;
                seat_id(r.slot)
            }
            Message::Rejoined(_) => Ok(()),
            Message::SnapshotOffer(o) => snapshot_offer(o),
            Message::SnapshotPart(p) => {
                if p.data.is_empty() || p.data.len() > MAX_SNAPSHOT_PART_BYTES {
                    return Err(ProtoError::Invalid("snapshot part size"));
                }
                if p.index >= MAX_SNAPSHOT_PARTS {
                    return Err(ProtoError::Invalid("snapshot part index"));
                }
                Ok(())
            }
            Message::SnapshotRefused(_) | Message::ResyncRequest(_) | Message::ResyncAck(_) => {
                Ok(())
            }
            Message::Resynced(r) => seat_id(r.slot),
            Message::Arrived(a) => seat_id(a.slot),
            Message::RefereeHello(h) => hello(h),
            Message::RefereeSeated { slot } => slot.map_or(Ok(()), seat_id),
            Message::Reference(_) => Ok(()),
        }
    }

    /// Who may send this message. Receivers check it against the relay's
    /// stamp before acting on a message (docs/NET.md, section 9).
    pub fn role(&self) -> Role {
        match self {
            Message::Welcome(_)
            | Message::Refuse(_)
            | Message::RulesRequest
            | Message::Lobby(_)
            | Message::Start(_)
            | Message::Resume { .. }
            | Message::Rejoined(_)
            | Message::Replayed(_)
            | Message::CatchUpDone
            | Message::SnapshotOffer(_)
            | Message::SnapshotPart(_)
            | Message::SnapshotRefused(_)
            | Message::Resynced(_)
            | Message::Arrived(_)
            | Message::RefereeSeated { .. } => Role::Host,
            Message::Reference(_) => Role::Referee,
            Message::Hello(_)
            | Message::RefereeHello(_)
            | Message::Rules(_)
            | Message::SeatChoice(_)
            | Message::Request(_)
            | Message::DropVote(_)
            | Message::Rejoin(_)
            | Message::ResyncRequest(_)
            | Message::ResyncAck(_) => Role::ToHost,
            Message::Turn(_)
            | Message::Ping { .. }
            | Message::Pong { .. }
            | Message::Chat(_)
            | Message::Desync(_)
            | Message::Sections(_)
            | Message::Leave(_) => Role::Any,
        }
    }

    /// Checks [`Message::role`] for a message the relay stamped as from
    /// `sender`, received by a peer that knows the host's seat. A joiner
    /// before `Welcome` passes `sender_is_host` from the room's host flag.
    pub fn check_sender(&self, sender_is_host: bool) -> Result<(), ProtoError> {
        self.check_sender_kind(if sender_is_host {
            SenderKind::Host
        } else {
            SenderKind::Peer
        })
    }

    /// [`Message::check_sender`] with the referee told apart (protocol 4).
    /// A referee sends its own digests and section digests, and, when it
    /// serves a resync (N5), the host's snapshot messages and `Resynced`; it
    /// sends no turn record, no lobby message and no host message besides
    /// those.
    pub fn check_sender_kind(&self, kind: SenderKind) -> Result<(), ProtoError> {
        match (self.role(), kind) {
            (Role::Host, SenderKind::Peer) => Err(ProtoError::Authority("host message from a non-host")),
            (Role::ToHost, SenderKind::Host) => Err(ProtoError::Authority("joiner message from the host")),
            (Role::Referee, SenderKind::Host | SenderKind::Peer) => {
                Err(ProtoError::Authority("referee message from a seat"))
            }
            (_, SenderKind::Referee) if !self.referee_may_send() => {
                Err(ProtoError::Authority("message a referee does not send"))
            }
            _ => Ok(()),
        }
    }

    /// What a referee connection may send (see [`Message::check_sender_kind`]).
    fn referee_may_send(&self) -> bool {
        matches!(
            self,
            Message::Reference(_)
                | Message::Sections(_)
                | Message::RefereeHello(_)
                | Message::Ping { .. }
                | Message::Pong { .. }
                | Message::Leave(_)
                | Message::SnapshotOffer(_)
                | Message::SnapshotPart(_)
                | Message::SnapshotRefused(_)
                | Message::Replayed(_)
                | Message::CatchUpDone
                | Message::Resynced(_)
        )
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

/// Reads a header of any version: `(version, body length)`. Checks magic
/// and the length bound only, so a relay can forward every version's
/// frames, the [`version_notice`] included. Peers use [`frame_len`].
pub fn peek_header(bytes: &[u8]) -> Result<(u16, usize), ProtoError> {
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
    let (version, len) = peek_header(frame)?;
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
    match peek_header(prefix) {
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

/// What a peer compares at the handshake (D10), implemented by the content
/// database's keyed digest ([`sta_content::ContentDigest`], CON-35): pass
/// `content_db.digest()`.
pub trait RulesIdentity {
    /// The digest of resolved sim-visible rule values.
    fn digest(&self) -> RulesDigest;
    /// Every resolved rule the digest covers, sorted bytewise by key, keys
    /// unique and at most [`MAX_RULE_KEY_BYTES`]: sent when digests differ,
    /// so both sides can name the first differing key.
    fn entries(&self) -> Vec<RuleEntry>;
}

/// The resolved rules of a `ContentDb` (`ContentDb::digest`): SHA-256 over
/// its keyed entries, and the entries in their canonical rendering
/// (`RTS_CFG.h:<KEY>`, `unit:<odf>`, `geometry:<odf>`, ...; see
/// `sta_content::digest`).
impl RulesIdentity for sta_content::ContentDigest {
    fn digest(&self) -> RulesDigest {
        RulesDigest(self.sha256())
    }

    fn entries(&self) -> Vec<RuleEntry> {
        self.entries()
            .iter()
            .map(|e| RuleEntry {
                key: e.key.clone(),
                value: e.value.clone(),
            })
            .collect()
    }
}

/// The host's verdict on a joiner's rules (D10), given the joiner's digest
/// from its `Hello` and, when that differs, its entry list as the
/// [`RulesAssembler`] returned it. `None`: the rules agree. Otherwise
/// [`Refusal::RulesDiffer`] with the first differing key and both values,
/// or [`Refusal::RulesUnlisted`] when the digests differ but no entry does.
/// `joiner_entries` is only read when the digests differ.
pub fn rules_refusal(
    host: &dyn RulesIdentity,
    joiner_digest: RulesDigest,
    joiner_entries: &[RuleEntry],
) -> Result<Option<Refusal>, ProtoError> {
    if host.digest() == joiner_digest {
        return Ok(None);
    }
    let ours = wire_entries(&host.entries())?;
    let theirs = wire_entries(joiner_entries)?;
    Ok(Some(match first_difference(&ours, &theirs) {
        Some(diff) => Refusal::RulesDiffer(diff),
        None => Refusal::RulesUnlisted,
    }))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// How a value longer than [`MAX_RULE_VALUE_BYTES`] travels: `#` and its
/// length and FNV-1a-64, so long values still compare exactly enough to
/// name the key (both sides render their own the same way).
pub fn hashed_value(value: &str) -> String {
    format!("#{}:{:016x}", value.len(), fnv1a64(value.as_bytes()))
}

/// A rule list as it travels and as the host compares it: checked sorted,
/// unique and within the key bound; long values replaced by
/// [`hashed_value`]. Nothing is truncated.
pub fn wire_entries(entries: &[RuleEntry]) -> Result<Vec<RuleEntry>, ProtoError> {
    if entries
        .windows(2)
        .any(|w| w[0].key.as_bytes() >= w[1].key.as_bytes())
    {
        return Err(ProtoError::Invalid("rule keys not ascending"));
    }
    entries
        .iter()
        .map(|e| {
            short(&e.key, MAX_RULE_KEY_BYTES, "rule key")?;
            let value = if e.value.len() > MAX_RULE_VALUE_BYTES {
                hashed_value(&e.value)
            } else {
                e.value.clone()
            };
            Ok(RuleEntry {
                key: e.key.clone(),
                value,
            })
        })
        .collect()
}

/// Splits a rule list into [`RulesPart`]s that each fit a frame.
pub fn rules_parts(entries: &[RuleEntry]) -> Result<Vec<RulesPart>, ProtoError> {
    let wire = wire_entries(entries)?;
    let chunks: Vec<&[RuleEntry]> = if wire.is_empty() {
        vec![&[]]
    } else {
        wire.chunks(MAX_RULE_ENTRIES_PER_PART).collect()
    };
    if chunks.len() > usize::from(MAX_RULE_PARTS) {
        return Err(ProtoError::Invalid("too many rule entries"));
    }
    let parts = chunks.len() as u16;
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(i, c)| RulesPart {
            part: i as u16,
            parts,
            entries: c.to_vec(),
        })
        .collect())
}

/// Collects a joiner's parts, which must arrive as `0..parts` in order with
/// keys ascending across them. Feed each part as it arrives; `Ok(Some)`
/// once the list is complete.
#[derive(Debug, Default)]
pub struct RulesAssembler {
    entries: Vec<RuleEntry>,
    next: u16,
    parts: Option<u16>,
}

impl RulesAssembler {
    pub fn push(&mut self, p: RulesPart) -> Result<Option<Vec<RuleEntry>>, ProtoError> {
        rules_part(&p)?;
        if p.part != self.next || self.parts.is_some_and(|n| n != p.parts) {
            return Err(ProtoError::Invalid("rules parts out of order"));
        }
        if let (Some(last), Some(first)) = (self.entries.last(), p.entries.first())
            && last.key.as_bytes() >= first.key.as_bytes()
        {
            return Err(ProtoError::Invalid("rule keys not ascending"));
        }
        self.parts = Some(p.parts);
        self.next += 1;
        self.entries.extend(p.entries);
        Ok((self.next == p.parts).then(|| std::mem::take(&mut self.entries)))
    }
}

/// The first key where two sorted lists differ, by key or value. Pass both
/// through [`wire_entries`] first, so long values compare as their hashes.
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

/// FNV-1a 64 over the map's `postcard` encoding: equal on every peer that
/// parsed the same map with the same build. `None` if it cannot be encoded.
pub fn map_digest(map: &MapDef) -> Option<u64> {
    postcard::to_allocvec(map).ok().map(|b| fnv1a64(&b))
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

    fn hello_fixture() -> Hello {
        Hello {
            engine: engine(),
            rules: RulesDigest({
                let mut d = [0; 32];
                d[0] = 9;
                d
            }),
            lineage: "FO 4.0".into(),
            packs: packs_fixture(),
            name: "Cedric".into(),
        }
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

    fn edit(seq: u32) -> Command {
        Command {
            player: PlayerId::SCRIPT,
            seq,
            kind: CommandKind::Edit(Edit::Kill(EntityId(3))),
        }
    }

    fn text(player: u8, seq: u32, t: &str) -> Command {
        Command {
            player: PlayerId(player),
            seq,
            kind: CommandKind::MissionText { text: t.into() },
        }
    }

    fn seat(slot: u8, occupant: Occupant) -> Seat {
        Seat {
            slot: PlayerId(slot),
            team: TeamId(slot),
            race: "Federation".into(),
            occupant,
            name: format!("p{slot}"),
            lineage: "STA2 Classic".into(),
            packs: packs_fixture(),
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
                seat(3, Occupant::Idle),
            ],
            host: PlayerId(0),
            turn: TurnConfig::default(),
            rules: RulesDigest([7; 32]),
        }
    }

    fn turn_msg() -> TurnCommands {
        TurnCommands {
            turn: 17,
            slot: PlayerId(1),
            commands: vec![cmd(1, 3), cmd(1, 4), text(1, 5, "taunt")],
            schedule: vec![],
            digest: Some(TickDigest {
                tick: 45,
                digest: 0xdead_beef,
            }),
        }
    }

    /// The host's record: its own seat, a computer seat, the script, and a
    /// schedule.
    fn host_turn() -> TurnCommands {
        TurnCommands {
            turn: 20,
            slot: PlayerId(0),
            commands: vec![
                cmd(0, 0),
                cmd(2, 0),
                edit(0),
                text(PlayerId::SCRIPT.0, 1, ""),
            ],
            schedule: vec![
                Scheduled {
                    by: PlayerId(1),
                    action: Action::Pause,
                },
                Scheduled {
                    by: PlayerId(0),
                    action: Action::SetDelay(HIGH_LATENCY_DELAY_TURNS),
                },
                Scheduled {
                    by: PlayerId(0),
                    action: Action::SetSpeed(150),
                },
                Scheduled {
                    by: PlayerId(0),
                    action: Action::Drop {
                        slot: PlayerId(1),
                        last_turn: 19,
                        then: DropChoice::Eliminate,
                    },
                },
                Scheduled {
                    by: PlayerId(0),
                    action: Action::Drop {
                        slot: PlayerId(3),
                        last_turn: 19,
                        then: DropChoice::AiTakeover("hard".into()),
                    },
                },
            ],
            digest: None,
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
            Message::Hello(hello_fixture()),
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
            Message::Turn(host_turn()),
            Message::Turn(TurnCommands {
                turn: 2,
                slot: PlayerId(0),
                commands: vec![],
                schedule: vec![],
                digest: None,
            }),
            Message::Request(Action::Pause),
            Message::Request(Action::SetSpeed(50)),
            Message::Resume { by: PlayerId(1) },
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
                hello: hello_fixture(),
                slot: PlayerId(1),
                token: 5,
                next_turn: Some(100),
            }),
            Message::Rejoin(Rejoin {
                hello: hello_fixture(),
                slot: PlayerId(1),
                token: 5,
                next_turn: None,
            }),
            Message::Rejoined(Rejoined {
                last_turn: Some(99),
                rejoin_token: 6,
            }),
            Message::Rejoined(Rejoined {
                last_turn: None,
                rejoin_token: 6,
            }),
            Message::Replayed(host_turn()),
            Message::CatchUpDone,
            Message::Sections(SectionReport {
                tick: 90,
                digests: vec![1, 2, 3, 4, 5],
            }),
            Message::Leave(LeaveReason::Quit),
            Message::Leave(LeaveReason::ToShell),
            Message::SnapshotOffer(SnapshotOffer {
                kind: SnapshotKind::Rejoin,
                tick: 300,
                format: 16,
                content_hash: 7,
                digest: 8,
                sections: vec![1, 2, 3, 4, 5],
                len: 70_000,
                chunk: 32_768,
                parts: 3,
                hash: 9,
                seed: RouterSeed {
                    turn: 100,
                    cfg: TurnConfig {
                        ticks_per_turn: 3,
                        delay_turns: 2,
                        speed_percent: 100,
                    },
                    seats: vec![
                        SeatSeed {
                            slot: PlayerId(0),
                            next_turn: 103,
                            last_turn: None,
                            queued: 3,
                        },
                        SeatSeed {
                            slot: PlayerId(1),
                            next_turn: 101,
                            last_turn: Some(100),
                            queued: 1,
                        },
                    ],
                    host_speaks: vec![PlayerId(1)],
                    seq_seen: vec![SeqSeen {
                        sender: PlayerId(0),
                        player: PlayerId::SCRIPT,
                        seq: 12,
                    }],
                    resumes: 1,
                },
            }),
            Message::SnapshotOffer(SnapshotOffer {
                kind: SnapshotKind::Resync,
                tick: 3,
                format: 16,
                content_hash: 1,
                digest: 2,
                sections: vec![],
                len: 1,
                chunk: 1,
                parts: 1,
                hash: 3,
                seed: RouterSeed {
                    turn: 1,
                    cfg: TurnConfig {
                        ticks_per_turn: 3,
                        delay_turns: 2,
                        speed_percent: 100,
                    },
                    seats: vec![],
                    host_speaks: vec![],
                    seq_seen: vec![],
                    resumes: 0,
                },
            }),
            Message::SnapshotPart(SnapshotPart {
                index: 2,
                data: vec![1, 2, 3],
            }),
            Message::SnapshotRefused(SnapshotRefusal::Busy),
            Message::SnapshotRefused(SnapshotRefusal::RateLimited),
            Message::SnapshotRefused(SnapshotRefusal::LimitReached),
            Message::SnapshotRefused(SnapshotRefusal::NotDiverged),
            Message::SnapshotRefused(SnapshotRefusal::TooLarge),
            Message::SnapshotRefused(SnapshotRefusal::Timeout),
            Message::SnapshotRefused(SnapshotRefusal::Unsupported),
            Message::SnapshotRefused(SnapshotRefusal::HostEndsMatch),
            Message::ResyncRequest(ResyncRequest {
                tick: 90,
                digest: 5,
                position: 31,
            }),
            Message::ResyncAck(ResyncAck { tick: 93, digest: 6 }),
            Message::Resynced(Resynced {
                slot: PlayerId(1),
                turn: 31,
                count: 2,
                last: true,
            }),
            Message::Arrived(Arrived {
                slot: PlayerId(1),
                turn: 31,
            }),
            Message::RefereeHello(hello_fixture()),
            Message::RefereeSeated { slot: Some(PlayerId(4)) },
            Message::RefereeSeated { slot: None },
            Message::Reference(TickDigest { tick: 93, digest: 7 }),
        ];
        for r in [
            Refusal::Full,
            Refusal::InProgress,
            Refusal::Closed,
            Refusal::Engine { host: engine() },
            Refusal::RulesDiffer(diff),
            Refusal::RulesUnlisted,
            Refusal::BadRejoin,
            Refusal::Kicked,
            Refusal::Protocol,
        ] {
            v.push(Message::Refuse(r));
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

    /// Pins the encoding of every sample. A change to any message, or to a
    /// sim type a message embeds (`Command`, `GameOptions`), fails here:
    /// bump `PROTOCOL_VERSION` and update the constant.
    #[test]
    fn golden_encoding_is_pinned() {
        let all: Vec<u8> = samples().iter().flat_map(|m| encode(m).unwrap()).collect();
        assert_eq!(PROTOCOL_VERSION, 5);
        assert_eq!(
            fnv1a64(&all),
            GOLDEN,
            "wire encoding changed: bump PROTOCOL_VERSION"
        );
    }

    const GOLDEN: u64 = 8549157114671075894;

    /// Tags are positional and never move: NET-04 appended 22 to 28 after
    /// `Sections` (21), the last tag of protocol 2.
    #[test]
    fn existing_tags_are_unmoved_and_new_ones_appended() {
        let tag = |m: &Message| encode(m).unwrap()[HEADER_LEN];
        let all = samples();
        let find = |f: fn(&Message) -> bool| all.iter().find(|m| f(m)).unwrap();
        assert_eq!(tag(find(|m| matches!(m, Message::Hello(_)))), 0);
        assert_eq!(tag(find(|m| matches!(m, Message::Turn(_)))), 8);
        assert_eq!(tag(find(|m| matches!(m, Message::Rejoin(_)))), 16);
        assert_eq!(tag(find(|m| matches!(m, Message::Sections(_)))), 21);
        assert_eq!(tag(find(|m| matches!(m, Message::SnapshotOffer(_)))), 22);
        assert_eq!(tag(find(|m| matches!(m, Message::SnapshotPart(_)))), 23);
        assert_eq!(tag(find(|m| matches!(m, Message::SnapshotRefused(_)))), 24);
        assert_eq!(tag(find(|m| matches!(m, Message::ResyncRequest(_)))), 25);
        assert_eq!(tag(find(|m| matches!(m, Message::ResyncAck(_)))), 26);
        assert_eq!(tag(find(|m| matches!(m, Message::Resynced(_)))), 27);
        assert_eq!(tag(find(|m| matches!(m, Message::Arrived(_)))), 28);
        assert_eq!(tag(find(|m| matches!(m, Message::RefereeHello(_)))), 29);
        assert_eq!(tag(find(|m| matches!(m, Message::RefereeSeated { .. }))), 30);
        assert_eq!(tag(find(|m| matches!(m, Message::Reference(_)))), 31);
    }

    #[test]
    fn snapshot_messages_refuse_out_of_bound_fields() {
        let offer = |f: &dyn Fn(&mut SnapshotOffer)| {
            let Message::SnapshotOffer(mut o) = samples()
                .into_iter()
                .find(|m| matches!(m, Message::SnapshotOffer(_)))
                .unwrap()
            else {
                unreachable!()
            };
            f(&mut o);
            Message::SnapshotOffer(o)
        };
        assert!(offer(&|_| {}).validate().is_ok());
        for bad in [
            offer(&|o| o.len = MAX_SNAPSHOT_BYTES as u32 + 1),
            offer(&|o| o.len = 0),
            offer(&|o| o.chunk = 0),
            offer(&|o| o.chunk = MAX_SNAPSHOT_PART_BYTES as u32 + 1),
            offer(&|o| o.parts = 2),
            offer(&|o| o.parts = 0),
            offer(&|o| o.sections = vec![0; MAX_SECTIONS + 1]),
            offer(&|o| o.seed.seats[0].queued = 129),
            offer(&|o| o.seed.seq_seen = vec![o.seed.seq_seen[0]; MAX_SEQ_SEEN + 1]),
            offer(&|o| o.seed.cfg.ticks_per_turn = 0),
            Message::SnapshotPart(SnapshotPart { index: 0, data: vec![] }),
            Message::SnapshotPart(SnapshotPart {
                index: 0,
                data: vec![0; MAX_SNAPSHOT_PART_BYTES + 1],
            }),
            Message::SnapshotPart(SnapshotPart { index: MAX_SNAPSHOT_PARTS, data: vec![1] }),
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
            assert!(encode(&bad).is_err());
        }
        // Roles: snapshots come from the host only, requests go to it.
        assert_eq!(
            Message::SnapshotPart(SnapshotPart { index: 0, data: vec![1] }).role(),
            Role::Host
        );
        assert_eq!(
            Message::ResyncRequest(ResyncRequest { tick: 0, digest: 0, position: 0 }).role(),
            Role::ToHost
        );
        assert_eq!(Message::ResyncAck(ResyncAck { tick: 0, digest: 0 }).role(), Role::ToHost);
    }

    #[test]
    fn header_is_little_endian_and_frozen() {
        let bytes = encode(&Message::Ping { nonce: 0 }).unwrap();
        // Ping is tag 12, nonce 0: two body bytes.
        assert_eq!(bytes, [b'S', b'6', b'4', b'N', 5, 0, 2, 0, 0, 0, 12, 0]);
        assert_eq!(version_notice(), [b'S', b'6', b'4', b'N', 5, 0, 0, 0, 0, 0]);
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

    /// A stream reader: frames back to back, delivered a byte at a time.
    #[test]
    fn stream_of_frames_reassembles_byte_by_byte() {
        let msgs = samples();
        let stream: Vec<u8> = msgs.iter().flat_map(|m| encode(m).unwrap()).collect();
        let (mut buf, mut got) = (Vec::new(), Vec::new());
        for &b in &stream {
            buf.push(b);
            if let Some(n) = frame_len(&buf).unwrap()
                && buf.len() == n
            {
                got.push(decode(&buf).unwrap());
                buf.clear();
            }
        }
        assert!(buf.is_empty());
        assert_eq!(got, msgs);
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

    fn stop(units: std::ops::Range<u32>) -> Command {
        Command {
            player: PlayerId(1),
            seq: units.start,
            kind: CommandKind::Order(Order::Stop {
                units: units.map(EntityId).collect(),
            }),
        }
    }

    #[test]
    fn oversized_frames_are_refused_before_reading() {
        let mut h = version_notice();
        h[6..10].copy_from_slice(&(MAX_BODY_BYTES as u32 + 1).to_le_bytes());
        assert!(matches!(frame_len(&h), Err(ProtoError::TooLarge { .. })));
        assert!(matches!(decode(&h), Err(ProtoError::TooLarge { .. })));
        assert!(matches!(peek_header(&h), Err(ProtoError::TooLarge { .. })));
        h[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(decode(&h), Err(ProtoError::TooLarge { .. })));
        // A message too big to send is refused at the sender too.
        let m = Message::Turn(TurnCommands {
            turn: 0,
            slot: PlayerId(1),
            commands: vec![stop(20_000..60_000)],
            schedule: vec![],
            digest: None,
        });
        assert!(matches!(encode(&m), Err(ProtoError::TooLarge { .. })));
    }

    #[test]
    fn fill_bounds_a_record_by_bytes_and_count() {
        // 20 commands of ~3 KB each: about 20 fit by bytes, the rest wait.
        let mut pending: Vec<Command> = (0..40u32)
            .map(|i| stop(100_000 + i * 1000..100_000 + i * 1000 + 1000))
            .collect();
        let (t, too_big) = TurnCommands::fill(5, PlayerId(1), vec![], None, &mut pending);
        assert!(too_big.is_empty());
        assert!(!t.commands.is_empty() && !pending.is_empty());
        assert_eq!(t.commands.len() + pending.len(), 40);
        assert!(encode(&Message::Turn(t)).is_ok());

        // By count.
        let mut pending: Vec<Command> = (0..300).map(|s| cmd(1, s)).collect();
        let (t, _) = TurnCommands::fill(5, PlayerId(1), vec![], None, &mut pending);
        assert_eq!(t.commands.len(), MAX_COMMANDS_PER_TURN);
        assert_eq!(pending.len(), 300 - MAX_COMMANDS_PER_TURN);
        assert!(encode(&Message::Turn(t)).is_ok());

        // A command that fits no frame is handed back, the rest go on.
        let mut pending = vec![stop(20_000..60_000), cmd(1, 70_000)];
        let (t, too_big) = TurnCommands::fill(5, PlayerId(1), vec![], None, &mut pending);
        assert_eq!(too_big.len(), 1);
        assert_eq!(t.commands, [cmd(1, 70_000)]);
        assert!(pending.is_empty());
    }

    #[test]
    fn other_versions_and_magic_are_named() {
        let mut bytes = encode(&Message::Ping { nonce: 3 }).unwrap();
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(decode(&bytes), Err(ProtoError::Version { found: 1 }));
        assert_eq!(frame_len(&bytes), Err(ProtoError::Version { found: 1 }));
        // A relay still reads it.
        assert_eq!(peek_header(&bytes), Ok((1, bytes.len() - HEADER_LEN)));
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
        // Chat (tag 14) with invalid UTF-8 text.
        assert!(matches!(
            decode(&frame(&[14, 0, 0, 1, 2, 0xff, 0xfe])),
            Err(ProtoError::Corrupt(_))
        ));
        // Sections (tag 21) claiming u32::MAX digests, with none behind them.
        assert!(matches!(
            decode(&frame(&[21, 90, 0xff, 0xff, 0xff, 0xff, 0x0f])),
            Err(ProtoError::Corrupt(_))
        ));
        // A Turn whose command list claims u32::MAX entries.
        assert!(matches!(
            decode(&frame(&[8, 0, 1, 0xff, 0xff, 0xff, 0xff, 0x0f])),
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

        let sched = |action| TurnCommands {
            schedule: vec![Scheduled {
                by: PlayerId(0),
                action,
            }],
            ..host_turn()
        };
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
            Message::Turn(sched(Action::SetDelay(0))),
            Message::Turn(sched(Action::SetSpeed(1000))),
            Message::Turn(TurnCommands {
                schedule: vec![Scheduled {
                    by: PlayerId(9),
                    action: Action::Pause,
                }],
                ..host_turn()
            }),
            Message::Turn(TurnCommands {
                schedule: vec![
                    Scheduled {
                        by: PlayerId(0),
                        action: Action::Pause
                    };
                    MAX_SCHEDULE_PER_TURN + 1
                ],
                ..host_turn()
            }),
            Message::Request(Action::Drop {
                slot: PlayerId(1),
                last_turn: 0,
                then: DropChoice::Eliminate,
            }),
            Message::Resume { by: PlayerId(200) },
            Message::Rules(RulesPart {
                part: 1,
                parts: 1,
                entries: vec![],
            }),
            Message::Rules(RulesPart {
                part: 0,
                parts: MAX_RULE_PARTS + 1,
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
            Message::Sections(SectionReport {
                tick: 3,
                digests: vec![0; MAX_SECTIONS + 1],
            }),
            Message::Sections(SectionReport {
                tick: 3,
                digests: Vec::new(),
            }),
            Message::Turn(TurnCommands {
                commands: vec![cmd(1, 4), cmd(1, 3)],
                ..turn_msg()
            }),
            Message::Turn(TurnCommands {
                commands: vec![cmd(255, 0)],
                ..turn_msg()
            }),
            Message::Turn(TurnCommands {
                commands: vec![text(1, 0, &"x".repeat(MAX_MISSION_TEXT_BYTES + 1))],
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
            Message::Rejoin(Rejoin {
                hello: Hello {
                    name: "n".repeat(MAX_NAME_BYTES + 1),
                    ..hello_fixture()
                },
                slot: PlayerId(1),
                token: 0,
                next_turn: None,
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
    fn roles_follow_the_host() {
        for m in samples() {
            let host = m.check_sender(true);
            let other = m.check_sender(false);
            match m.role() {
                Role::Host => assert!(host.is_ok() && other.is_err(), "{m:?}"),
                Role::ToHost => assert!(host.is_err() && other.is_ok(), "{m:?}"),
                Role::Any => assert!(host.is_ok() && other.is_ok(), "{m:?}"),
                Role::Referee => assert!(host.is_err() && other.is_err(), "{m:?}"),
            }
        }
        assert_eq!(Message::Start(start()).role(), Role::Host);
        assert_eq!(Message::Replayed(turn_msg()).role(), Role::Host);
        assert_eq!(Message::Hello(hello_fixture()).role(), Role::ToHost);
        assert_eq!(Message::Turn(turn_msg()).role(), Role::Any);
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

        // The host sends for its own seat, computer seats and the script,
        // and schedules changes.
        assert_eq!(host_turn().authorize(PlayerId(0), true, speaks), Ok(()));
        let as_host = |commands: Vec<Command>| TurnCommands {
            commands,
            schedule: vec![],
            ..host_turn()
        };
        let as_peer = |commands: Vec<Command>| TurnCommands {
            slot: PlayerId(1),
            commands,
            ..as_host(vec![])
        };
        // A non-host may not speak for the AI or the script, nor schedule...
        assert!(
            as_peer(vec![cmd(2, 0)])
                .authorize(PlayerId(1), false, speaks)
                .is_err()
        );
        assert!(
            as_peer(vec![edit(0)])
                .authorize(PlayerId(1), false, speaks)
                .is_err()
        );
        assert!(
            as_peer(vec![text(PlayerId::SCRIPT.0, 0, "")])
                .authorize(PlayerId(1), false, speaks)
                .is_err()
        );
        let scheduled = TurnCommands {
            schedule: host_turn().schedule,
            ..as_peer(vec![])
        };
        assert_eq!(
            scheduled.authorize(PlayerId(1), false, speaks),
            Err(ProtoError::Authority("schedule from a non-host"))
        );
        // ...nor the host for another person's seat, an idle seat or the
        // bench, and players never edit.
        for c in [cmd(1, 0), cmd(3, 0), cmd(255, 0)] {
            assert!(
                as_host(vec![c.clone()])
                    .authorize(PlayerId(0), true, speaks)
                    .is_err(),
                "{c:?}"
            );
        }
        let own_edit = Command {
            player: PlayerId(0),
            seq: 1,
            kind: CommandKind::Edit(Edit::Kill(EntityId(3))),
        };
        assert_eq!(
            as_host(vec![own_edit]).authorize(PlayerId(0), true, speaks),
            Err(ProtoError::Authority("edit from a player"))
        );
        // Mission text from one's own seat is fine.
        assert_eq!(
            as_peer(vec![text(1, 0, "go")]).authorize(PlayerId(1), false, speaks),
            Ok(())
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
                Control::Ai("normal".into()),
                Control::Idle
            ]
        );
        let joiner: Vec<_> = s
            .player_setups(PlayerId(1))
            .into_iter()
            .map(|p| p.control)
            .collect();
        assert_eq!(
            joiner,
            [
                Control::Remote,
                Control::Human,
                Control::RemoteAi("normal".into()),
                Control::Idle
            ]
        );
        assert_eq!(s.turn.tick_of(10), 30);
        assert_eq!(s.turn.turn_of(32), 10);
        assert_eq!(s.turn.turn_of(Tick::MAX), u32::MAX);
    }

    #[test]
    fn engines_match_only_when_known_and_equal() {
        assert!(engine().matches(&engine()));
        let unknown = EngineId {
            build: String::new(),
            ..engine()
        };
        assert!(!unknown.matches(&unknown));
        assert!(!engine().matches(&EngineId {
            build: "abc124".into(),
            ..engine()
        }));
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
    }

    /// A content digest of `(key, value)` entries.
    fn digest_of(entries: &[(&str, &str)]) -> sta_content::ContentDigest {
        sta_content::ContentDigest::from_entries(
            entries
                .iter()
                .map(|(k, v)| sta_content::DigestEntry::new(*k, *v))
                .collect(),
        )
    }

    #[test]
    fn the_content_digest_is_the_rules_identity() {
        let classic = digest_of(&[("RTS_CFG.h:A", "1"), ("unit:x", "hull=1")]);
        let enhanced = digest_of(&[("RTS_CFG.h:A", "3"), ("unit:x", "hull=1")]);
        assert_eq!(RulesIdentity::digest(&classic).0, classic.sha256());
        assert_ne!(RulesIdentity::digest(&classic), RulesIdentity::digest(&enhanced));
        assert_eq!(rules_refusal(&classic, RulesIdentity::digest(&classic), &[]).unwrap(), None);
        assert_eq!(
            rules_refusal(&classic, RulesIdentity::digest(&enhanced), &RulesIdentity::entries(&enhanced)).unwrap(),
            Some(Refusal::RulesDiffer(RuleDiff {
                key: "RTS_CFG.h:A".into(),
                host: Some("1".into()),
                joiner: Some("3".into())
            }))
        );
        // A joiner whose list leaves out what its digest covers.
        assert_eq!(
            rules_refusal(&classic, RulesIdentity::digest(&enhanced), &RulesIdentity::entries(&classic)).unwrap(),
            Some(Refusal::RulesUnlisted)
        );
    }

    /// The whole D10 flow: the joiner's list split into parts, sent,
    /// reassembled at the host and compared with the host's own.
    fn send_and_compare(host: &[RuleEntry], joiner: &[RuleEntry]) -> Option<RuleDiff> {
        let mut asm = RulesAssembler::default();
        let mut got = None;
        for p in rules_parts(joiner).unwrap() {
            let Message::Rules(p) = decode(&encode(&Message::Rules(p)).unwrap()).unwrap() else {
                unreachable!()
            };
            got = asm.push(p).unwrap();
        }
        first_difference(
            &wire_entries(host).unwrap(),
            &got.expect("all parts arrived"),
        )
    }

    #[test]
    fn rules_cross_the_wire_and_name_the_key() {
        let long = "9".repeat(500);
        let list: Vec<_> = (0..1000)
            .map(|i| {
                e(
                    &format!("RTS_CFG.h:key{i:05}{}", "k".repeat(70)),
                    if i % 7 == 0 { &long } else { "1" },
                )
            })
            .collect();
        assert_eq!(rules_parts(&list).unwrap().len(), 4);
        assert_eq!(send_and_compare(&list, &list), None);

        // A difference past byte 96 of a long value is still found.
        let mut other = list.clone();
        other[700].value = format!("{}8", "9".repeat(499));
        let d = send_and_compare(&list, &other).unwrap();
        assert_eq!(d.key, list[700].key);
        assert!(d.host.unwrap().starts_with("#500:"));

        // Keys that share a long prefix stay distinct; over-long keys are
        // refused, not cut.
        let prefix = "p".repeat(MAX_RULE_KEY_BYTES - 1);
        let close = [e(&format!("{prefix}a"), "1"), e(&format!("{prefix}b"), "1")];
        assert_eq!(send_and_compare(&close, &close), None);
        assert!(rules_parts(&[e(&"k".repeat(MAX_RULE_KEY_BYTES + 1), "1")]).is_err());
        assert!(rules_parts(&[e("b", "1"), e("a", "1")]).is_err());
        let too_many: Vec<_> = (0..MAX_RULE_ENTRIES_PER_PART * usize::from(MAX_RULE_PARTS) + 1)
            .map(|i| e(&format!("{i:08}"), ""))
            .collect();
        assert!(rules_parts(&too_many).is_err());
        assert_eq!(rules_parts(&[]).unwrap().len(), 1);
    }

    #[test]
    fn rules_assembler_refuses_disorder() {
        let parts = rules_parts(
            &(0..600)
                .map(|i| e(&format!("{i:04}"), ""))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let mut asm = RulesAssembler::default();
        assert!(asm.push(parts[1].clone()).is_err(), "out of order");
        let mut asm = RulesAssembler::default();
        asm.push(parts[0].clone()).unwrap();
        assert!(asm.push(parts[0].clone()).is_err(), "duplicate");
        let mut asm = RulesAssembler::default();
        asm.push(parts[0].clone()).unwrap();
        let mut back = parts[1].clone();
        back.entries[0].key = "0000".into();
        assert!(asm.push(back).is_err(), "keys going backwards across parts");
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
