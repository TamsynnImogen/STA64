//! The boundary toward the sim crate's save container (SIM-81, ADR 0010 V2).
//!
//! UI-55 develops in parallel with SIM-81 (docs/WORKPLAN.md, UI-55 *needs*): until SIM-81 is on `main`
//! this module stands in for `sta_sim::save`, with the same names, fields and signatures for what the
//! client uses, so that wiring the real container is replacing this file's body with
//! `pub use sta_sim::save::*;` (and its `store`) and adapting what the review of SIM-81 changed. The
//! stand-in keeps the journal, the snapshot, the client's record and a header in one buffer; it is
//! **not** a file format: nothing here is checksummed or versioned, and computer players are always
//! refused (no controller has opted in to saving before SIM-81's seam), so the first stateful
//! controller test belongs to the wiring commit. The client never depends on how the bytes look.

#![allow(dead_code)] // The names the container offers; the load list (UI-56) and the wiring use the rest.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_content::{ContentDb, MapDef};
use sta_core::PlayerId;
use sta_sim::Sim;
use sta_sim::replay::{ControlRec, Replay, SeatRec, StartRec};
use sta_sim::setup::{Controller, GameOptions};

/// What a save is of (V2). Only skirmishes are saved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GameKind {
    #[default]
    Skirmish,
    Mission,
}

/// The map by reference only (V3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapId {
    pub path: String,
    pub hash: u64,
}

/// What only the caller knows when it saves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveInfo {
    pub build_id: String,
    pub install_key: String,
    pub mod_key: String,
    pub map: Option<MapId>,
    pub kind: GameKind,
    pub local_seat: Option<PlayerId>,
    pub play_time_secs: u64,
    pub made_at: u64,
    pub description: String,
}

/// The client's record as the container keeps it: opaque, with its own version.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientState {
    pub version: u32,
    pub data: Vec<u8>,
}

/// A computer seat's saved controller record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatController {
    pub seat: PlayerId,
    pub name: String,
    pub version: u32,
    pub data: Vec<u8>,
}

/// Why a save or a load was refused. Mirrors the container's variants the client tells apart; the
/// message is the load list's reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    NotASave,
    Damaged(String),
    ContentMismatch { saved: u64, given: u64 },
    MapMissing { path: String },
    MapChanged { path: String, saved: u64, given: u64 },
    Controller { seat: PlayerId, reason: String },
    ControllerCannotSave { seat: PlayerId },
    Mission,
    Networked { seat: PlayerId },
    NoMapFile,
    Invalid(String),
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::NotASave => write!(f, "not a STA64 save file"),
            SaveError::Damaged(why) => write!(f, "the save file is damaged: {why}"),
            SaveError::ContentMismatch { saved, given } => write!(f, "the save was made on other content: hash {saved:#018x}, this install has {given:#018x}"),
            SaveError::MapMissing { path } => write!(f, "the map file \"{path}\" the save was made on is not available"),
            SaveError::MapChanged { path, .. } => write!(f, "the map file \"{path}\" has changed since the save"),
            SaveError::Controller { seat, reason } => write!(f, "the computer player of seat {} cannot be restored: {reason}", seat.0),
            SaveError::ControllerCannotSave { seat } => write!(f, "the computer player of seat {} cannot be saved yet", seat.0),
            SaveError::Mission => write!(f, "saves of missions are not supported yet"),
            SaveError::Networked { seat } => write!(f, "a networked game cannot be saved or loaded (seat {} is remote)", seat.0),
            SaveError::NoMapFile => write!(f, "this match's map has no map file (a generated or unsaved map); such matches cannot be saved yet"),
            SaveError::Invalid(why) => write!(f, "cannot save: {why}"),
            SaveError::Io(why) => write!(f, "file error: {why}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// What a load list shows and a loader needs before it reads a section.
#[derive(Debug, Clone)]
pub struct Header {
    pub content_hash: u64,
    pub map: MapId,
    pub kind: GameKind,
    pub seats: Vec<SeatRec>,
    pub options: GameOptions,
    pub starts: StartRec,
    pub seed: u64,
    pub local_seat: Option<PlayerId>,
    pub tick: u64,
    pub play_time_secs: u64,
    pub made_at: u64,
    pub description: String,
}

impl Header {
    /// Why this build cannot load the save from the header alone (a networked seat, a mission, other
    /// content when it is known).
    pub fn refusal(&self, content: Option<&ContentDb>) -> Option<SaveError> {
        if self.kind == GameKind::Mission {
            return Some(SaveError::Mission);
        }
        if let Some(s) = self.seats.iter().find(|s| matches!(s.control, ControlRec::Remote | ControlRec::RemoteAi(_))) {
            return Some(SaveError::Networked { seat: s.id });
        }
        content.filter(|db| db.hash() != self.content_hash).map(|db| SaveError::ContentMismatch { saved: self.content_hash, given: db.hash() })
    }
}

/// The map as the caller read it at load.
#[derive(Debug, Clone)]
pub struct MapSource {
    pub hash: u64,
    pub map: Arc<MapDef>,
}

/// A successful load.
pub struct Loaded {
    pub sim: Sim,
    pub replay: Replay,
    pub header: Header,
    pub info: SaveInfo,
    pub controllers: Vec<SeatController>,
    pub client: ClientState,
}

/// A restored controller and its seat.
pub type SeatBox = (PlayerId, Box<dyn Controller>);

impl Loaded {
    pub fn restore_controllers(
        &self,
        _restore: impl FnMut(PlayerId, &str, u32, &[u8]) -> Result<Box<dyn Controller>, String>,
    ) -> Result<Vec<SeatBox>, SaveError> {
        // The stand-in refuses computer seats at save, so none is ever recorded.
        Ok(Vec::new())
    }
}

/// Whether a controller has opted in to saving. The stand-in says no for all.
pub fn controller_can_save(_c: &dyn Controller) -> bool {
    false
}

fn put(out: &mut Vec<u8>, blob: &[u8]) {
    out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
    out.extend_from_slice(blob);
}

fn take<'a>(rest: &mut &'a [u8]) -> Result<&'a [u8], SaveError> {
    let bad = || SaveError::Damaged("the stand-in container is cut short".into());
    let n = rest.get(..4).ok_or_else(bad)?;
    let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
    let body = rest.get(4..4 + n).ok_or_else(bad)?;
    *rest = &rest[4 + n..];
    Ok(body)
}

const MAGIC: &[u8; 4] = b"STUB";

/// Writes a save of `sim` at its tick (see `sta_sim::save::save`).
pub fn save(sim: &Sim, replay: &Replay, info: &SaveInfo, controllers: &[(PlayerId, &dyn Controller)], client: &ClientState) -> Result<Vec<u8>, SaveError> {
    if info.kind == GameKind::Mission {
        return Err(SaveError::Mission);
    }
    let Some(map) = info.map.clone() else { return Err(SaveError::NoMapFile) };
    if let Some(s) = replay.players.iter().find(|s| matches!(s.control, ControlRec::Remote | ControlRec::RemoteAi(_))) {
        return Err(SaveError::Networked { seat: s.id });
    }
    if replay.end_tick != sim.tick {
        return Err(SaveError::Invalid(format!("the journal ends at tick {}, the sim is at tick {}", replay.end_tick, sim.tick)));
    }
    for seat in &replay.players {
        if matches!(seat.control, ControlRec::Ai(_)) {
            let can = controllers.iter().any(|(id, c)| *id == seat.id && controller_can_save(*c));
            if !can {
                return Err(SaveError::ControllerCannotSave { seat: seat.id });
            }
        }
    }
    let header = serde_json::json!({
        "map_path": map.path, "map_hash": map.hash, "local": info.local_seat.map(|p| p.0),
        "play": info.play_time_secs, "made": info.made_at, "desc": info.description,
    });
    let mut out = MAGIC.to_vec();
    put(&mut out, header.to_string().as_bytes());
    put(&mut out, &sim.snapshot());
    put(&mut out, &replay.encode());
    let mut c = client.version.to_le_bytes().to_vec();
    c.extend_from_slice(&client.data);
    put(&mut out, &c);
    Ok(out)
}

struct Parts<'a> {
    header: serde_json::Value,
    snapshot: &'a [u8],
    replay: Replay,
    client: ClientState,
}

fn parts(bytes: &[u8]) -> Result<Parts<'_>, SaveError> {
    let mut rest = bytes.strip_prefix(MAGIC.as_slice()).ok_or(SaveError::NotASave)?;
    let header = serde_json::from_slice(take(&mut rest)?).map_err(|e| SaveError::Damaged(e.to_string()))?;
    let snapshot = take(&mut rest)?;
    let replay = Replay::decode(take(&mut rest)?).map_err(|e| SaveError::Damaged(e.to_string()))?;
    let c = take(&mut rest)?;
    let version = c.get(..4).map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]])).ok_or_else(|| SaveError::Damaged("no client record".into()))?;
    Ok(Parts { header, snapshot, replay, client: ClientState { version, data: c[4..].to_vec() } })
}

fn header_of(p: &Parts<'_>) -> Header {
    let h = &p.header;
    let n = |k: &str| h[k].as_u64().unwrap_or(0);
    Header {
        content_hash: p.replay.content_hash,
        map: MapId { path: h["map_path"].as_str().unwrap_or("").to_owned(), hash: n("map_hash") },
        kind: GameKind::Skirmish,
        seats: p.replay.players.clone(),
        options: p.replay.options.clone(),
        starts: p.replay.starts,
        seed: p.replay.seed,
        local_seat: h["local"].as_u64().map(|v| PlayerId(v as u8)),
        tick: p.replay.end_tick,
        play_time_secs: n("play"),
        made_at: n("made"),
        description: h["desc"].as_str().unwrap_or("").to_owned(),
    }
}

/// The header alone.
pub fn read_header(bytes: &[u8]) -> Result<Header, SaveError> {
    parts(bytes).map(|p| header_of(&p))
}

/// Loads a save (see `sta_sim::save::load`).
pub fn load(bytes: &[u8], content: Arc<ContentDb>, map: Option<MapSource>) -> Result<Loaded, SaveError> {
    let p = parts(bytes)?;
    let header = header_of(&p);
    if let Some(e) = header.refusal(Some(&content)) {
        return Err(e);
    }
    let Some(map) = map else { return Err(SaveError::MapMissing { path: header.map.path }) };
    if map.hash != header.map.hash {
        return Err(SaveError::MapChanged { path: header.map.path, saved: header.map.hash, given: map.hash });
    }
    let sim = Sim::from_snapshot(p.snapshot, Some(content)).map_err(|e| SaveError::Damaged(e.to_string()))?;
    let mut replay = p.replay;
    replay.map = (*map.map).clone();
    let info = SaveInfo {
        map: Some(header.map.clone()),
        local_seat: header.local_seat,
        play_time_secs: header.play_time_secs,
        made_at: header.made_at,
        description: header.description.clone(),
        ..SaveInfo::default()
    };
    Ok(Loaded { sim, replay, header, info, controllers: Vec::new(), client: p.client })
}

/// File helpers: a folder of saves.
pub mod store {
    use super::*;

    pub const EXTENSION: &str = "stasave";

    fn io(what: &str, e: &std::io::Error) -> SaveError {
        SaveError::Io(format!("{what}: {e}"))
    }

    /// Writes atomically (a temporary file in the same folder, then a rename).
    pub fn write_atomic(dir: &Path, file_name: &str, bytes: &[u8]) -> Result<PathBuf, SaveError> {
        std::fs::create_dir_all(dir).map_err(|e| io("cannot create the saves folder", &e))?;
        let target = dir.join(file_name);
        let temp = dir.join(format!("{file_name}.tmp"));
        std::fs::write(&temp, bytes).map_err(|e| io("cannot write the save", &e))?;
        std::fs::rename(&temp, &target).map_err(|e| io("cannot write the save", &e))?;
        Ok(target)
    }

    /// A new file under a generated name, never over an existing one.
    pub fn write_new(dir: &Path, bytes: &[u8]) -> Result<PathBuf, SaveError> {
        let h = read_header(bytes)?;
        let stem = format!("save-{:012}-t{}", h.made_at, h.tick);
        let mut name = format!("{stem}.{EXTENSION}");
        let mut n = 1;
        while dir.join(&name).exists() {
            n += 1;
            name = format!("{stem}-{n}.{EXTENSION}");
        }
        write_atomic(dir, &name, bytes)
    }

    pub fn read_file(path: &Path) -> Result<Vec<u8>, SaveError> {
        std::fs::read(path).map_err(|e| io("cannot read the save", &e))
    }

    /// One file of a listing.
    #[derive(Debug)]
    pub struct Listing {
        pub file_name: String,
        pub path: PathBuf,
        pub outcome: Result<Header, SaveError>,
    }

    pub fn list(dir: &Path, content: Option<&ContentDb>) -> Result<Vec<Listing>, SaveError> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Ok(Vec::new()) };
        let mut out: Vec<Listing> = entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some(EXTENSION))
            .map(|e| {
                let path = e.path();
                let outcome = read_file(&path).and_then(|b| read_header(&b)).and_then(|h| h.refusal(content).map_or(Ok(h), Err));
                Listing { file_name: e.file_name().to_string_lossy().into_owned(), path, outcome }
            })
            .collect();
        out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        Ok(out)
    }
}
