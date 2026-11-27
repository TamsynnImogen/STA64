//! Save files (SIM-81, ADR 0010 V1 to V3): a container around the sim
//! snapshot.
//!
//! - [`save`] and [`load`] are pure functions over bytes. The file helpers in
//!   [`store`] write atomically into a directory the caller names; the sim
//!   holds no directory policy, no clock and no install.
//! - A save is its own file (V2): magic, a container version of its own (see
//!   [`CONTAINER_VERSION`]), a [`Header`] a load list reads without the rest,
//!   then four sections: the sim snapshot (`Sim::snapshot` bytes, unchanged),
//!   the journal (the replay's commands and digests from tick 0, without the
//!   map), the controllers' state (one opaque record per computer seat,
//!   written and read by the controller, ADR 0010 V5) and the client's state
//!   (one opaque record). The byte layout is in [`container`].
//! - No map data and no assets are in a save (V3): the header names the map
//!   by path and hash; the caller reads the map file and hands its `MapDef`
//!   and hash to [`load`], which rebuilds the replay from the journal, the
//!   header's setup and that map. A match whose map has no file is refused
//!   at save ([`SaveError::NoMapFile`]).
//! - Nothing is migrated (V1): a file made by another container version, by
//!   a build whose snapshot, replay or seam revision this build does not read,
//!   on other content, on another map, with a controller this build cannot
//!   restore, or a damaged or cut file is refused with [`SaveError`], whose
//!   message names the first mismatch and is the load list's reason.
//! - Saving the same state with the same [`SaveInfo`] gives the same bytes.
//!   The time and the description are the caller's input and are in the
//!   header; they are the only parts that vary between saves of one state.

mod container;
mod header;
pub mod store;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, MapDef};
use sta_core::{PlayerId, SEAM_REV, Tick};

pub use header::{CONTAINER_VERSION, GameKind, Header, MAGIC, MAX_HEADER, MapId, MissionIdentity, SaveInfo};
pub use store::{Listing, list, read_file, write_atomic, write_new};

use crate::replay::{ControlRec, Logged, Replay};
use crate::setup::{Controller, ControllerState};
use crate::snapshot::{self, SnapshotError};
use crate::{replay, Sim};

/// Why a save was refused, at save or at load: one variant and one message
/// per reason (ADR 0010 V1). The message names the first mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// Not a save file (wrong or missing magic).
    NotASave,
    /// Another container version: made by an older or a newer STA64.
    ContainerVersion { found: u16 },
    /// The snapshot format inside is one this build does not read.
    SnapshotFormat { found: u16 },
    /// The replay format the journal was written for is one this build does not read.
    ReplayFormat { found: u16 },
    /// Another seam revision: commands may not decode alike.
    SeamRev { found: u32 },
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
    /// The inputs of a save do not fit together (journal of another run, a
    /// header too long, ...).
    Invalid(String),
    /// Reading or writing the file failed.
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::NotASave => write!(f, "not a STA64 save file"),
            SaveError::ContainerVersion { found } if *found < CONTAINER_VERSION => {
                write!(f, "save container version {found} is from an older STA64 and cannot be loaded (this build reads version {CONTAINER_VERSION}); saves are not migrated")
            }
            SaveError::ContainerVersion { found } => {
                write!(f, "save container version {found} is from a newer STA64 (this build reads version {CONTAINER_VERSION})")
            }
            SaveError::SnapshotFormat { found } => {
                write!(f, "the save holds snapshot format {found}, which this build does not read (saves are not migrated)")
            }
            SaveError::ReplayFormat { found } => {
                write!(f, "the save holds replay format {found}, which this build does not read (saves are not migrated)")
            }
            SaveError::SeamRev { found } => {
                write!(f, "the save was made against seam revision {found}, this build is {SEAM_REV} (saves are not migrated)")
            }
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
            SaveError::Invalid(why) => write!(f, "cannot save: {why}"),
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

/// The journal section: the replay without its map.
#[derive(Serialize, Deserialize)]
struct JournalRec {
    commands: Vec<Logged>,
    digests: Vec<(Tick, u64)>,
    end_tick: Tick,
}

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
    /// load, naming the seat.
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

fn refuse_remote(seats: &[replay::SeatRec]) -> Result<(), SaveError> {
    match seats.iter().find(|s| matches!(s.control, ControlRec::Remote | ControlRec::RemoteAi(_))) {
        Some(s) => Err(SaveError::Networked { seat: s.id }),
        None => Ok(()),
    }
}

/// Writes a save of `sim` at its current tick.
///
/// `replay` is the game's replay so far (its journal is kept, its map is
/// not); it must end at the sim's tick and have been recorded on the same
/// content. `controllers` holds the controller of every computer seat; one
/// that has not opted in to saving ([`Controller::save_state`]) refuses the
/// save with its seat named. `info` carries what only the caller knows.
pub fn save(
    sim: &Sim,
    replay: &Replay,
    info: &SaveInfo,
    controllers: &[(PlayerId, &dyn Controller)],
    client: &ClientState,
) -> Result<Vec<u8>, SaveError> {
    if info.kind == GameKind::Mission {
        return Err(SaveError::Mission);
    }
    let Some(map) = info.map.clone() else {
        return Err(SaveError::NoMapFile);
    };
    refuse_remote(&replay.players)?;
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
        profile: sim.content().map_or_else(String::new, |c| format!("{:?}", c.profile())),
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
    let journal = JournalRec { commands: replay.commands.clone(), digests: replay.digests.clone(), end_tick: replay.end_tick };
    let invalid = |e: postcard::Error| SaveError::Invalid(e.to_string());
    container::seal(
        &header,
        &container::Parts {
            snapshot: snapshot_bytes,
            journal: postcard::to_allocvec(&journal).map_err(invalid)?,
            controllers: postcard::to_allocvec(&records).map_err(invalid)?,
            client_version: client.version,
            client: client.data.clone(),
        },
    )
}

/// Reads the header of a save from the start of its bytes (a prefix as long
/// as [`header_len_hint`] says is enough), checking magic, container version
/// and the header's checksum. Does not look at a section.
pub fn read_header(bytes: &[u8]) -> Result<Header, SaveError> {
    container::parse_header(bytes).map(|(h, _)| h)
}

/// How many bytes from the start of a file [`read_header`] can need at most.
pub const fn header_len_hint() -> usize {
    container::PREFIX + MAX_HEADER + 8
}

impl Header {
    /// Why a build holding `content` (or none, for a list that does not know
    /// yet) cannot load this save, from the header alone: the formats, the
    /// game kind, a networked seat and the content. `None` means the
    /// header raises no objection (the map and the sections are checked by
    /// [`load`]).
    pub fn refusal(&self, content: Option<&ContentDb>) -> Option<SaveError> {
        if let Some(e) = self.format_refusal() {
            return Some(e);
        }
        content.and_then(|db| self.content_refusal(db))
    }

    /// The snapshot's and the replay's own checks of their formats and the
    /// seam revision, run on a stub that carries only what they look at, so
    /// that what this build reads is decided in one place (their modules).
    fn format_refusal(&self) -> Option<SaveError> {
        let mut snap = Vec::new();
        snap.extend_from_slice(&snapshot::MAGIC);
        snap.extend_from_slice(&self.snapshot_format.to_le_bytes());
        snap.extend_from_slice(&self.seam_rev.to_le_bytes());
        snap.extend_from_slice(&self.content_hash.to_le_bytes());
        snap.extend_from_slice(&[0, 0]);
        match snapshot::snapshot_info(&snap) {
            Err(SnapshotError::FormatVersion { found }) => return Some(SaveError::SnapshotFormat { found }),
            Err(SnapshotError::SeamRev { found }) => return Some(SaveError::SeamRev { found }),
            _ => {}
        }
        let mut rep = Vec::new();
        rep.extend_from_slice(&replay::MAGIC);
        rep.extend_from_slice(&self.replay_format.to_le_bytes());
        rep.extend(postcard::to_allocvec(&self.seam_rev).unwrap_or_default());
        match Replay::decode(&rep) {
            Err(replay::ReplayError::FormatVersion { found }) => return Some(SaveError::ReplayFormat { found }),
            Err(replay::ReplayError::SeamRev { found }) => return Some(SaveError::SeamRev { found }),
            _ => {}
        }
        if self.kind == GameKind::Mission {
            return Some(SaveError::Mission);
        }
        refuse_remote(&self.seats).err()
    }

    fn content_refusal(&self, db: &ContentDb) -> Option<SaveError> {
        let same = db.hash() == self.content_hash
            || (!self.options.directors_cut && db.matches_pre_directors_cut_hash(self.content_hash));
        (!same).then(|| SaveError::ContentMismatch {
            saved: self.content_hash,
            given: db.hash(),
            lineage: self.lineage.clone(),
            packs: self.packs.clone(),
        })
    }
}

/// Loads a save: checks it in the order of ADR 0010 V1 (container, formats,
/// damage, content, map, then the sections) and refuses with the first
/// mismatch. `content` is the database the game is to run on; `map` is the
/// map file the header names, as the caller read it (`None` if it could not).
/// Never panics, whatever `bytes` hold.
pub fn load(bytes: &[u8], content: Arc<ContentDb>, map: Option<MapSource>) -> Result<Loaded, SaveError> {
    let (header, body_at) = container::parse_header(bytes)?;
    if let Some(e) = header.format_refusal() {
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
    let journal: JournalRec = decode(sections.journal, "journal")?;
    if journal.end_tick != header.tick {
        return Err(SaveError::Damaged("the journal does not end at the saved tick".into()));
    }
    if journal.commands.windows(2).any(|w| w[0].tick > w[1].tick) || journal.commands.iter().any(|c| c.tick >= header.tick) {
        return Err(SaveError::Damaged("the journal's commands are out of order".into()));
    }
    if journal.digests.windows(2).any(|w| w[0].0 >= w[1].0) || journal.digests.iter().any(|d| d.0 > header.tick) {
        return Err(SaveError::Damaged("the journal's digests are out of order".into()));
    }
    let controllers: Vec<SeatController> = decode(sections.controllers, "controller records")?;
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
        commands: journal.commands,
        digests: journal.digests,
        end_tick: journal.end_tick,
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
    Ok(Loaded { sim, replay, header, info, controllers, client })
}

fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8], what: &str) -> Result<T, SaveError> {
    let (value, rest) = postcard::take_from_bytes::<T>(bytes).map_err(|e| SaveError::Damaged(format!("the {what} do not parse: {e}")))?;
    if !rest.is_empty() {
        return Err(SaveError::Damaged(format!("the {what} have trailing bytes")));
    }
    Ok(value)
}

/// The records must be exactly the header's computer seats, by name.
fn check_controllers(header: &Header, records: &[SeatController]) -> Result<(), SaveError> {
    let wanted: Vec<(PlayerId, &str)> = header
        .seats
        .iter()
        .filter_map(|s| match &s.control {
            ControlRec::Ai(name) => Some((s.id, name.as_str())),
            _ => None,
        })
        .collect();
    for r in records {
        if !wanted.iter().any(|(id, _)| *id == r.seat) {
            return Err(SaveError::Damaged(format!("a controller record for seat {}, which is not a computer seat", r.seat.0)));
        }
    }
    for (id, name) in wanted {
        match records.iter().filter(|r| r.seat == id).count() {
            1 => {}
            0 => return Err(SaveError::Controller { seat: id, reason: "the save holds no record for it".into() }),
            _ => return Err(SaveError::Damaged(format!("two controller records for seat {}", id.0))),
        }
        if records.iter().any(|r| r.seat == id && r.name != name) {
            return Err(SaveError::Damaged(format!("the controller record of seat {} names another controller than the setup", id.0)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
