//! Save files (SIM-81, ADR 0010 V1 to V3): a container around the sim
//! snapshot.
//!
//! - [`save`] and [`load`] are pure functions over bytes. The file helpers in
//!   [`store`] write atomically into a directory the caller names; the sim
//!   holds no directory policy, no clock and no install.
//! - A save is its own file (V2): magic, the four numbers that decide
//!   loadability at fixed offsets (container version, snapshot format, replay
//!   format, seam revision: exact match, no migration), a [`Header`] a load
//!   list reads without the rest, then five sections: the sim snapshot
//!   (`Sim::snapshot` bytes, unchanged), the journal (the replay's commands
//!   and digests from tick 0, without the map), the controllers' state (one
//!   opaque record per computer seat, written and read by the controller,
//!   ADR 0010 V5), the commands in flight (what was submitted for the next
//!   tick and is in neither the snapshot nor the journal yet, with the
//!   link's next sequence number) and the client's state (one opaque
//!   record). The byte layout, and the rule for when [`CONTAINER_VERSION`]
//!   must rise, are in `container`.
//! - No map data and no assets are in a save (V3): the header names the map
//!   by path and hash; the caller reads the map file and hands its `MapDef`
//!   and hash to [`load`], which rebuilds the replay from the journal, the
//!   header's setup and that map. A match whose map has no file is refused
//!   at save ([`SaveError::NoMapFile`]).
//! - Nothing is migrated (V1): a file made by another container version, by
//!   a build with another snapshot, replay or seam number, on other content,
//!   on another map, with a controller this build cannot restore, or a
//!   damaged or truncated file is refused with [`SaveError`], whose message
//!   names the first mismatch and is the load list's reason.
//! - Saving the same state with the same [`SaveInfo`] gives the same bytes.
//!   The time and the description are the caller's input and are in the
//!   header; they are the only parts that vary between saves of one state.
//! - **A save from elsewhere is untrusted input, and the checksum is not
//!   protection against it**: it detects accidental damage only (anyone can
//!   recompute it). The container's own parsing is bounded (sizes, counts
//!   and lengths are checked before anything is allocated from them, see
//!   [`MAX_FILE`]) and its tests do not find a panic on the inputs they
//!   try; the snapshot decoder behind it is the resync decoder, whose
//!   behaviour on hostile content is its own (the tests here sample it with
//!   valid checksums).

mod container;
mod header;
pub mod store;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, MapDef, Profile};
use sta_core::{PlayerId, SEAM_REV, Tick};

pub use header::{CONTAINER_VERSION, GameKind, Header, MAGIC, MAX_HEADER, MapId, MissionIdentity, SaveInfo};
pub use store::{Listing, list, read_file, write_atomic, write_new};

use crate::replay::{self, ControlRec, Logged, Replay, SeatRec};
use crate::setup::{Controller, ControllerState, GameOptions};
use crate::snapshot::{self, SnapshotError};
use crate::Sim;
use crate::command::Command;

/// The largest file `load` and `read_file` accept (64 MiB), checked before
/// anything else is done with the bytes. A save is a snapshot (a few MiB for
/// a large match), a journal (a few bytes per command; a three-hour game is
/// well under a MiB) and small records, so this is far above any real one.
/// It is low because decoding a hostile file grows memory by a multiple of
/// its size (about 14 times for the smallest records), and a shared save is
/// untrusted.
pub const MAX_FILE: usize = 64 << 20;

/// Most seats a game has (`PlayerId::MAX_PLAYERS`); also the most controller
/// records a save may hold.
const MAX_SEATS: usize = PlayerId::MAX_PLAYERS as usize;
/// Bounds on the counts of the journal and of the commands in flight,
/// checked from the count's own bytes before the list is decoded: 2 million
/// journal commands, a million digests (a digest every 30 ticks is 33
/// million ticks), 100 000 commands submitted for one tick.
const MAX_COMMANDS: u64 = 2_000_000;
const MAX_DIGESTS: u64 = 1_000_000;
const MAX_IN_FLIGHT: u64 = 100_000;

/// Why a save was refused, at save or at load: one variant and one message
/// per reason (ADR 0010 V1). The message names the first mismatch. The four
/// number variants carry what the file has and what this build reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// Not a save file (wrong or missing magic).
    NotASave,
    /// Another container version: made by an older or a newer STA64.
    ContainerVersion { found: u16, expected: u16 },
    /// The snapshot format inside is not the one this build reads.
    SnapshotFormat { found: u16, expected: u16 },
    /// The replay format the journal was written for is not the one this build reads.
    ReplayFormat { found: u16, expected: u16 },
    /// Another seam revision: commands may not decode alike.
    SeamRev { found: u32, expected: u32 },
    /// Truncated, garbled or inconsistent.
    Damaged(String),
    /// The content is not the content the game was played on (other content
    /// or mods; lineage and packs say which).
    ContentMismatch { saved: u64, given: u64, lineage: String, packs: Vec<String> },
    /// The caller has no map file for the path the save names.
    MapMissing { path: String },
    /// The map file's hash is not the one at save time.
    MapChanged { path: String, saved: u64, given: u64 },
    /// The sim rejected the snapshot.
    Snapshot(SnapshotError),
    /// A computer seat's controller is unknown or its record is unreadable
    /// to this build (at load), or missing from the saved set.
    Controller { seat: PlayerId, reason: String },
    /// At save: this seat's controller has not opted in to saving.
    ControllerCannotSave { seat: PlayerId },
    /// A mission save: not supported yet (ADR 0010 V4).
    Mission,
    /// A networked match (a remote seat): not part of v1 (V6).
    Networked { seat: PlayerId },
    /// The match's map has no file (a generated or unsaved map): not
    /// supported; needs a decision.
    NoMapFile,
    /// A save of an observer (no local player): not supported; V7 keeps it
    /// to a player's own match.
    Observer,
    /// The inputs or the parts of a save do not fit together (journal of
    /// another run, a header whose setup is not the snapshot's, a header
    /// too long, a controller for a non-computer seat, ...).
    Invalid(String),
    /// Reading or writing the file failed.
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let when = |older: bool| if older { "an older" } else { "a newer" };
        match self {
            SaveError::NotASave => write!(f, "not a STA64 save file"),
            SaveError::ContainerVersion { found, expected } => write!(
                f,
                "save container version {found} is from {} STA64 (this build reads version {expected}); saves are not migrated",
                when(found < expected)
            ),
            SaveError::SnapshotFormat { found, expected } => write!(
                f,
                "the save holds snapshot format {found} from {} STA64 (this build reads format {expected}); saves are not migrated",
                when(found < expected)
            ),
            SaveError::ReplayFormat { found, expected } => write!(
                f,
                "the save holds replay format {found} from {} STA64 (this build reads format {expected}); saves are not migrated",
                when(found < expected)
            ),
            SaveError::SeamRev { found, expected } => write!(
                f,
                "the save was made against seam revision {found} from {} STA64 (this build is {expected}); saves are not migrated",
                when(found < expected)
            ),
            SaveError::Damaged(why) => write!(f, "the save file is damaged: {why}"),
            SaveError::ContentMismatch { saved, given, lineage, packs } => {
                write!(f, "the save was made on other content: hash {saved:#018x}, this install has {given:#018x} (it was {lineage}")?;
                if packs.is_empty() {
                    write!(f, ")")
                } else {
                    write!(f, " with {})", packs.join(", "))
                }
            }
            SaveError::MapMissing { path } => write!(f, "the map file \"{path}\" the save was made on is not available"),
            SaveError::MapChanged { path, saved, given } => {
                write!(f, "the map file \"{path}\" has changed since the save: hash {given:#018x}, saved {saved:#018x}")
            }
            SaveError::Snapshot(e) => write!(f, "the saved game state cannot be loaded: {e}"),
            SaveError::Controller { seat, reason } => write!(f, "the computer player of seat {} cannot be restored: {reason}", seat.0),
            SaveError::ControllerCannotSave { seat } => {
                write!(f, "the computer player of seat {} cannot be saved yet", seat.0)
            }
            SaveError::Mission => write!(f, "saves of missions are not supported yet"),
            SaveError::Networked { seat } => {
                write!(f, "a networked game cannot be saved or loaded (seat {} is remote)", seat.0)
            }
            SaveError::NoMapFile => write!(f, "this match's map has no map file (a generated or unsaved map); such matches cannot be saved yet"),
            SaveError::Observer => write!(f, "a game watched as an observer cannot be saved (there is no local player)"),
            SaveError::Invalid(why) => write!(f, "the parts of the save do not fit together: {why}"),
            SaveError::Io(why) => write!(f, "file error: {why}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// The client's state as a save keeps it: one versioned opaque record,
/// written and read by the client (UI-55).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientState {
    pub version: u32,
    pub data: Vec<u8>,
}

/// A computer seat's saved controller state, as a load hands it out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatController {
    pub seat: PlayerId,
    /// The registry name the seat's [`crate::setup::Control::Ai`] carries.
    pub name: String,
    pub version: u32,
    pub data: Vec<u8>,
}

/// The journal section: the replay without its map. The field order is the
/// encoding order and is decoded one field at a time, so that the counts of
/// the two lists are bounded before they are decoded.
#[derive(Serialize)]
struct JournalRec<'a> {
    end_tick: Tick,
    digests: &'a [(Tick, u64)],
    commands: &'a [Logged],
}

/// The commands in flight at a save, and the link's sequence counter.
///
/// A save is taken **between ticks, after the controllers have thought about
/// the last step and submitted**: what has been submitted for the next tick
/// (the controllers' commands of that think, and the local player's and the
/// sandbox's since) is in neither the snapshot nor the journal, which take a
/// command when it is stepped. These commands change the game, so they are
/// part of the save: `commands` in submission order, and `next_seq`, the
/// next sequence number the link hands out (stored, not derived: the link
/// shares one counter between the player's and the sandbox's commands).
///
/// After a load the caller submits `commands` to its router for the next
/// tick, restores `next_seq`, does **not** call the controllers' `think`
/// (they already thought about the saved tick; their records are their state
/// after that think) and steps on as usual.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InFlight {
    pub next_seq: u32,
    pub commands: Vec<Command>,
}

type Digests = Vec<(Tick, u64)>;

/// A restored controller and the seat it plays.
pub type SeatBox = (PlayerId, Box<dyn Controller>);

/// A successful load.
pub struct Loaded {
    /// The restored sim, at the saved tick.
    pub sim: Sim,
    /// The replay from tick 0 to the saved tick, rebuilt from the journal,
    /// the header's setup and the map the caller supplied: the caller goes on
    /// logging into it, so a capture of a loaded game still replays.
    pub replay: Replay,
    pub header: Header,
    /// What the caller gave at save: pass it (with a new time and
    /// description if wanted) to the next [`save`].
    pub info: SaveInfo,
    /// One record per computer seat, ascending seat. Restore them with
    /// [`Loaded::restore_controllers`] or by hand.
    pub controllers: Vec<SeatController>,
    /// The commands in flight and the link's next sequence number. **Submit
    /// them for the next tick and do not call the restored controllers'
    /// `think` before stepping**: see [`InFlight`].
    pub in_flight: InFlight,
    pub client: ClientState,
}

/// The map as the caller read it at load: its `MapDef` and its hash by the
/// same rule the caller used at save.
#[derive(Debug, Clone)]
pub struct MapSource {
    pub hash: u64,
    pub map: Arc<MapDef>,
}

impl Loaded {
    /// Builds each computer seat's controller with `restore(seat, registry
    /// name, record)`, which belongs to the controller's crate (it builds the
    /// controller and takes the record back) and refuses with a reason an
    /// unknown name or a record version it does not read: that refuses the
    /// load, naming the seat. The restored controllers are in their state
    /// after the think of the saved tick: submit [`Loaded::in_flight`] and
    /// step; do not think first.
    pub fn restore_controllers(
        &self,
        mut restore: impl FnMut(PlayerId, &str, &ControllerState) -> Result<Box<dyn Controller>, String>,
    ) -> Result<Vec<SeatBox>, SaveError> {
        self.controllers
            .iter()
            .map(|c| {
                let state = ControllerState { version: c.version, data: c.data.clone() };
                restore(c.seat, &c.name, &state)
                    .map(|ctl| (c.seat, ctl))
                    .map_err(|reason| SaveError::Controller { seat: c.seat, reason })
            })
            .collect()
    }
}

fn refuse_remote(seats: &[SeatRec]) -> Result<(), SaveError> {
    match seats.iter().find(|s| matches!(s.control, ControlRec::Remote | ControlRec::RemoteAi(_))) {
        Some(s) => Err(SaveError::Networked { seat: s.id }),
        None => Ok(()),
    }
}

/// A stable name for a content profile, written to the header. (Not
/// `Debug`: a rename of a variant must not change the file. A new variant is
/// a compile error here, so its name is chosen on purpose.)
fn profile_name(p: Profile) -> &'static str {
    match p {
        Profile::Native => "native",
        Profile::Fo4 => "fo4",
        Profile::Fo3 => "fo3",
        Profile::Sta2Classic => "sta2classic",
        Profile::Sta1Classic => "sta1classic",
    }
}

/// The seats of a header or a replay: a plausible set (at most
/// [`MAX_SEATS`], real player slots, ascending and unique ids).
fn check_seats(seats: &[SeatRec], local: Option<PlayerId>) -> Result<(), SaveError> {
    let bad = |why: &str| Err(SaveError::Invalid(why.to_string()));
    if seats.is_empty() || seats.len() > MAX_SEATS {
        return bad("the number of seats is not between 1 and 8");
    }
    if seats.iter().any(|s| !s.id.is_player()) || seats.windows(2).any(|w| w[0].id >= w[1].id) {
        return bad("the seats are not ascending, unique player slots");
    }
    match local {
        None => Err(SaveError::Observer),
        Some(l) if seats.iter().any(|s| s.id == l) => Ok(()),
        Some(_) => bad("the local seat is not one of the seats"),
    }
}

/// The setup a header or a replay records against the sim's own: the seats
/// and their teams, which seats a computer plays, the Director's Cut option.
/// A mismatch refuses, so a forged header cannot leave a computer seat
/// without its controller or give the rebuilt replay another setup than the
/// game's.
fn cross_check(sim: &Sim, seats: &[SeatRec], options: &GameOptions) -> Result<(), SaveError> {
    let bad = |why: &str| Err(SaveError::Invalid(format!("the recorded setup is not the game's: {why}")));
    let mut theirs: Vec<(PlayerId, sta_core::TeamId)> = seats.iter().map(|s| (s.id, s.team)).collect();
    theirs.sort();
    let mut mine = sim.players.clone();
    mine.sort();
    if theirs != mine {
        return bad("the seats or their teams differ");
    }
    let mut ai: Vec<PlayerId> =
        seats.iter().filter(|s| matches!(s.control, ControlRec::Ai(_) | ControlRec::RemoteAi(_))).map(|s| s.id).collect();
    ai.sort();
    let mut sim_ai = sim.ai_seats.clone();
    sim_ai.sort();
    if ai != sim_ai {
        return bad("the computer seats differ");
    }
    if options.directors_cut != sim.directors_cut {
        return bad("the Director's Cut option differs");
    }
    Ok(())
}

/// Every command in flight comes from a seat of the game or the sandbox
/// player, and there are not more of them than a tick can hold.
fn check_in_flight(seats: &[SeatRec], in_flight: &InFlight) -> Result<(), SaveError> {
    if in_flight.commands.len() as u64 > MAX_IN_FLIGHT {
        return Err(SaveError::Invalid(format!("too many commands in flight: {}, at most {MAX_IN_FLIGHT}", in_flight.commands.len())));
    }
    match in_flight.commands.iter().find(|c| c.player != PlayerId::BENCH && !seats.iter().any(|s| s.id == c.player)) {
        Some(c) => Err(SaveError::Invalid(format!("a command in flight is from player {}, who is not a seat of the game", c.player.0))),
        None => Ok(()),
    }
}

/// Writes a save of `sim` at its current tick.
///
/// `replay` is the game's replay so far (its journal is kept, its map is
/// not); it must end at the sim's tick, have been recorded on the same
/// content and describe the same setup as the sim. `controllers` holds the
/// controller of every computer seat; one that has not opted in to saving
/// ([`Controller::save_state`]) refuses the save with its seat named.
/// Take the save **between ticks, after the controllers have thought about
/// the last step and submitted**: `in_flight` is what is waiting for the next
/// tick ([`InFlight`]), every command's player a seat of the game or the
/// sandbox player (`PlayerId::BENCH`), and `controllers` hold their state
/// after that think. `info` carries what only the caller knows;
/// `info.local_seat` must be a seat of the game (a save without one is an
/// observer's: refused).
pub fn save(
    sim: &Sim,
    replay: &Replay,
    info: &SaveInfo,
    controllers: &[(PlayerId, &dyn Controller)],
    in_flight: &InFlight,
    client: &ClientState,
) -> Result<Vec<u8>, SaveError> {
    if info.kind == GameKind::Mission {
        return Err(SaveError::Mission);
    }
    let Some(map) = info.map.clone() else {
        return Err(SaveError::NoMapFile);
    };
    refuse_remote(&replay.players)?;
    check_seats(&replay.players, info.local_seat)?;
    let snapshot_bytes = sim.snapshot();
    let snap = snapshot::snapshot_info(&snapshot_bytes).map_err(|e| SaveError::Invalid(e.to_string()))?;
    if snap.content_hash == 0 {
        return Err(SaveError::Invalid("a sim without content cannot be saved".into()));
    }
    if replay.content_hash != snap.content_hash {
        return Err(SaveError::Invalid("the journal was recorded on other content than the sim runs on".into()));
    }
    if replay.end_tick != sim.tick {
        return Err(SaveError::Invalid(format!("the journal ends at tick {}, the sim is at tick {}", replay.end_tick, sim.tick)));
    }
    cross_check(sim, &replay.players, &replay.options)?;
    check_in_flight(&replay.players, in_flight)?;
    let mut records = Vec::new();
    for seat in &replay.players {
        let ControlRec::Ai(name) = &seat.control else { continue };
        let Some((_, ctl)) = controllers.iter().find(|(id, _)| *id == seat.id) else {
            return Err(SaveError::Invalid(format!("no controller was given for the computer seat {}", seat.id.0)));
        };
        let Some(state) = ctl.save_state() else {
            return Err(SaveError::ControllerCannotSave { seat: seat.id });
        };
        records.push(SeatController { seat: seat.id, name: name.clone(), version: state.version, data: state.data });
    }
    if let Some((id, _)) = controllers
        .iter()
        .find(|(id, _)| !replay.players.iter().any(|s| s.id == *id && matches!(s.control, ControlRec::Ai(_))))
    {
        return Err(SaveError::Invalid(format!("a controller was given for seat {}, which is not a computer seat", id.0)));
    }
    records.sort_by_key(|r| r.seat);
    let header = Header {
        build_id: info.build_id.clone(),
        snapshot_format: snapshot::FORMAT_VERSION,
        replay_format: replay::FORMAT_VERSION,
        seam_rev: SEAM_REV,
        content_hash: snap.content_hash,
        lineage: snap.provenance.lineage,
        packs: snap.provenance.packs,
        profile: sim.content().map_or_else(String::new, |c| profile_name(c.profile()).to_string()),
        install_key: info.install_key.clone(),
        mod_key: info.mod_key.clone(),
        map,
        kind: info.kind,
        mission: info.mission.clone(),
        seats: replay.players.clone(),
        options: replay.options.clone(),
        starts: replay.starts,
        seed: replay.seed,
        local_seat: info.local_seat,
        tick: sim.tick,
        play_time_secs: info.play_time_secs,
        made_at: info.made_at,
        description: info.description.clone(),
    };
    let journal = JournalRec { end_tick: replay.end_tick, digests: &replay.digests, commands: &replay.commands };
    let invalid = |e: postcard::Error| SaveError::Invalid(e.to_string());
    let bytes = container::seal(
        &header,
        &container::Parts {
            snapshot: snapshot_bytes,
            journal: postcard::to_allocvec(&journal).map_err(invalid)?,
            controllers: postcard::to_allocvec(&records).map_err(invalid)?,
            in_flight: postcard::to_allocvec(in_flight).map_err(invalid)?,
            client_version: client.version,
            client: client.data.clone(),
        },
    )?;
    if bytes.len() > MAX_FILE {
        return Err(SaveError::Invalid(format!("the save is {} bytes, more than the {MAX_FILE} a save may have", bytes.len())));
    }
    Ok(bytes)
}

/// Reads the header of a save from the start of its bytes (a prefix as long
/// as [`header_len_hint`] is enough), checking magic, the four numbers by
/// exact match and the header's checksum. Does not look at a section.
pub fn read_header(bytes: &[u8]) -> Result<Header, SaveError> {
    container::parse_header(bytes).map(|(h, _)| h)
}

/// How many bytes from the start of a file [`read_header`] can need at most.
pub const fn header_len_hint() -> usize {
    container::PREFIX + MAX_HEADER + 8
}

impl Header {
    /// Why a build holding `content` (or none, for a list that does not know
    /// yet) cannot load this save, from the header alone: a mission, a
    /// networked or observer game, an implausible set of seats, other
    /// content. (The numbers that decide loadability were checked when the
    /// header was read.) `None` means the header raises no objection (the
    /// map and the sections are checked by [`load`]).
    pub fn refusal(&self, content: Option<&ContentDb>) -> Option<SaveError> {
        if let Some(e) = self.setup_refusal() {
            return Some(e);
        }
        content.and_then(|db| self.content_refusal(db))
    }

    fn setup_refusal(&self) -> Option<SaveError> {
        if self.kind == GameKind::Mission {
            return Some(SaveError::Mission);
        }
        if let Err(e) = refuse_remote(&self.seats) {
            return Some(e);
        }
        check_seats(&self.seats, self.local_seat).err()
    }

    fn content_refusal(&self, db: &ContentDb) -> Option<SaveError> {
        (db.hash() != self.content_hash).then(|| SaveError::ContentMismatch {
            saved: self.content_hash,
            given: db.hash(),
            lineage: self.lineage.clone(),
            packs: self.packs.clone(),
        })
    }
}

/// Loads a save: checks it in the order of ADR 0010 V1 (size, container, the
/// numbers by exact match, header, damage, content, map, then the sections)
/// and refuses with the first mismatch. `content` is the database the game is
/// to run on; `map` is the map file the header names, as the caller read it
/// (`None` if it could not). The container's parsing is bounded in memory by
/// the file's size and the caps in this module, whatever `bytes` hold.
pub fn load(bytes: &[u8], content: Arc<ContentDb>, map: Option<MapSource>) -> Result<Loaded, SaveError> {
    if bytes.len() > MAX_FILE {
        return Err(SaveError::Damaged(format!("the file is larger than any save ({} bytes, at most {MAX_FILE})", bytes.len())));
    }
    let (header, body_at) = container::parse_header(bytes)?;
    if let Some(e) = header.setup_refusal() {
        return Err(e);
    }
    container::verify_whole(bytes, body_at)?;
    if let Some(e) = header.content_refusal(&content) {
        return Err(e);
    }
    let Some(map) = map else {
        return Err(SaveError::MapMissing { path: header.map.path.clone() });
    };
    if map.hash != header.map.hash {
        return Err(SaveError::MapChanged { path: header.map.path.clone(), saved: header.map.hash, given: map.hash });
    }
    let sections = container::parse_sections(&bytes[body_at..bytes.len() - 8])?;
    let sim = Sim::from_snapshot(sections.snapshot, Some(content)).map_err(SaveError::Snapshot)?;
    if sim.tick != header.tick {
        return Err(SaveError::Damaged("the header's tick is not the snapshot's".into()));
    }
    cross_check(&sim, &header.seats, &header.options)?;
    let (commands, digests) = decode_journal(sections.journal, header.tick)?;
    let in_flight = decode_in_flight(sections.in_flight, &header.seats)?;
    let (controllers, rest) = take_vec::<SeatController>(sections.controllers, MAX_SEATS as u64, "controller records")?;
    ended(rest, "controller records")?;
    check_controllers(&header, &controllers)?;
    let replay = Replay {
        seam_rev: SEAM_REV,
        content_hash: header.content_hash,
        lineage: header.lineage.clone(),
        packs: header.packs.clone(),
        peers: Vec::new(),
        seed: header.seed,
        map: (*map.map).clone(),
        players: header.seats.clone(),
        options: header.options.clone(),
        starts: header.starts,
        commands,
        digests,
        end_tick: header.tick,
    };
    let info = SaveInfo {
        build_id: header.build_id.clone(),
        install_key: header.install_key.clone(),
        mod_key: header.mod_key.clone(),
        map: Some(header.map.clone()),
        kind: header.kind,
        mission: header.mission.clone(),
        local_seat: header.local_seat,
        play_time_secs: header.play_time_secs,
        made_at: header.made_at,
        description: header.description.clone(),
    };
    let client = ClientState { version: sections.client_version, data: sections.client.to_vec() };
    Ok(Loaded { sim, replay, header, info, controllers, in_flight, client })
}

/// The commands in flight: next sequence number, then the commands (count
/// bounded before decoding), each from a seat or the sandbox player.
fn decode_in_flight(bytes: &[u8], seats: &[SeatRec]) -> Result<InFlight, SaveError> {
    let (next_seq, rest) =
        postcard::take_from_bytes::<u32>(bytes).map_err(|e| SaveError::Damaged(format!("the commands in flight do not parse: {e}")))?;
    let (commands, rest) = take_vec::<Command>(rest, MAX_IN_FLIGHT, "commands in flight")?;
    ended(rest, "commands in flight")?;
    let in_flight = InFlight { next_seq, commands };
    check_in_flight(seats, &in_flight)?;
    Ok(in_flight)
}

fn ended(rest: &[u8], what: &str) -> Result<(), SaveError> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(SaveError::Damaged(format!("the {what} have trailing bytes")))
    }
}

/// Decodes a postcard list whose count is refused above `max` before the list
/// is decoded: the count is the list's first bytes (a varint), so a hostile
/// count costs nothing.
fn take_vec<'a, T: Deserialize<'a>>(bytes: &'a [u8], max: u64, what: &str) -> Result<(Vec<T>, &'a [u8]), SaveError> {
    let (n, _) = postcard::take_from_bytes::<u64>(bytes).map_err(|e| SaveError::Damaged(format!("the {what} do not parse: {e}")))?;
    if n > max {
        return Err(SaveError::Damaged(format!("too many {what}: {n}, at most {max}")));
    }
    postcard::take_from_bytes::<Vec<T>>(bytes).map_err(|e| SaveError::Damaged(format!("the {what} do not parse: {e}")))
}

/// The journal: end tick, digests, commands; checked for order and against
/// the saved tick.
fn decode_journal(bytes: &[u8], tick: Tick) -> Result<(Vec<Logged>, Digests), SaveError> {
    let parse = |e: postcard::Error| SaveError::Damaged(format!("the journal does not parse: {e}"));
    let (end_tick, rest) = postcard::take_from_bytes::<Tick>(bytes).map_err(parse)?;
    if end_tick != tick {
        return Err(SaveError::Damaged("the journal does not end at the saved tick".into()));
    }
    let (digests, rest) = take_vec::<(Tick, u64)>(rest, MAX_DIGESTS, "journal digests")?;
    let (commands, rest) = take_vec::<Logged>(rest, MAX_COMMANDS, "journal commands")?;
    ended(rest, "journal")?;
    if commands.windows(2).any(|w| w[0].tick > w[1].tick) || commands.iter().any(|c| c.tick >= tick) {
        return Err(SaveError::Damaged("the journal's commands are out of order".into()));
    }
    if digests.windows(2).any(|w| w[0].0 >= w[1].0) || digests.iter().any(|d| d.0 > tick) {
        return Err(SaveError::Damaged("the journal's digests are out of order".into()));
    }
    Ok((commands, digests))
}

/// The records must be exactly the header's computer seats, by name: both
/// lists ascending by seat (seats are validated ascending; the writer sorts
/// the records), compared in one pass.
fn check_controllers(header: &Header, records: &[SeatController]) -> Result<(), SaveError> {
    if records.windows(2).any(|w| w[0].seat >= w[1].seat) {
        return Err(SaveError::Damaged("the controller records are not ascending by seat".into()));
    }
    let wanted = header.seats.iter().filter_map(|s| match &s.control {
        ControlRec::Ai(name) => Some((s.id, name.as_str())),
        _ => None,
    });
    let mut have = records.iter().peekable();
    for (id, name) in wanted {
        if let Some(extra) = have.next_if(|r| r.seat < id) {
            return Err(SaveError::Damaged(format!("a controller record for seat {}, which is not a computer seat", extra.seat.0)));
        }
        match have.next_if(|r| r.seat == id) {
            None => return Err(SaveError::Controller { seat: id, reason: "the save holds no record for it".into() }),
            Some(r) if r.name != name => {
                return Err(SaveError::Damaged(format!("the controller record of seat {} names another controller than the setup", id.0)));
            }
            Some(_) => {}
        }
    }
    match have.next() {
        Some(extra) => Err(SaveError::Damaged(format!("a controller record for seat {}, which is not a computer seat", extra.seat.0))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests;
