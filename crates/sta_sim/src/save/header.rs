//! The save file's header: everything a load list shows and a loader checks
//! before it touches a section (ADR 0010 V2).

use serde::{Deserialize, Serialize};
use sta_core::{PlayerId, Tick};

use crate::replay::{SeatRec, StartRec};
use crate::setup::GameOptions;

/// First four bytes of every save file.
pub const MAGIC: [u8; 4] = *b"STAG";

/// The container's own layout version (V2), independent of the snapshot,
/// replay and protocol numbers. It starts at 1; the steward's ledger, if one
/// is kept for it, hands out later numbers. The rule for raising it is in
/// `container`'s docs: any change to the serialised form of the header, of
/// the types it embeds from other modules, or of the section records.
pub const CONTAINER_VERSION: u16 = 1;

/// Largest header a file may declare. The encoded header is checked against it
/// at save and at load, before any allocation sized by the file.
pub const MAX_HEADER: usize = 64 * 1024;

/// What a save holds a game of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameKind {
    /// A skirmish: restored from the snapshot (ADR 0010 scope).
    #[default]
    Skirmish,
    /// A mission: reserved in the layout, refused until mission saves exist
    /// (V4, not in v1).
    Mission,
}

/// V2's identity fields of a mission. Empty for a skirmish.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionIdentity {
    pub campaign_digest: u64,
    pub mission: String,
    pub script_path: String,
    /// One hash over the script and every file it included.
    pub script_hash: u64,
    pub converter_version: u32,
}

/// The map a game was played on, by reference only: map data never enters a
/// save (V3). The path is as the caller names it; the hash is the caller's
/// hash of the map file. **Only that hash ties the map to the save**: the
/// sim never sees the file, cannot tell whether the `MapDef` handed to
/// [`super::load`] is the one that was played (it only feeds the rebuilt
/// replay, the game state comes from the snapshot) and does not compare the
/// path, which the caller resolves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapId {
    pub path: String,
    pub hash: u64,
}

/// What the header says. Readable with [`super::read_header`] without
/// parsing a section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// Informational: the build that wrote the file.
    pub build_id: String,
    /// The snapshot format, replay format and seam revision of the writing
    /// build. They are not in the postcard header: they sit at fixed offsets
    /// in the file's prefix (see `container`) and are filled in from there
    /// when a header is read.
    #[serde(skip)]
    pub snapshot_format: u16,
    #[serde(skip)]
    pub replay_format: u16,
    #[serde(skip)]
    pub seam_rev: u32,
    /// `ContentDb::hash`, and where the content came from (shown, not compared).
    pub content_hash: u64,
    pub lineage: String,
    pub packs: Vec<String>,
    /// The content profile (as `Profile`'s name), and the caller's install and
    /// mod keys. Informational.
    pub profile: String,
    pub install_key: String,
    pub mod_key: String,
    pub map: MapId,
    pub kind: GameKind,
    pub mission: MissionIdentity,
    pub seats: Vec<SeatRec>,
    pub options: GameOptions,
    pub starts: StartRec,
    pub seed: u64,
    pub local_seat: Option<PlayerId>,
    pub tick: Tick,
    /// Whole seconds played, supplied by the caller (no clock in the sim).
    pub play_time_secs: u64,
    /// Unix seconds the save was made, supplied by the caller.
    pub made_at: u64,
    /// The player's description.
    pub description: String,
}

/// What only the caller knows when it saves (the sim holds no clock, no
/// install and no paths). A load gives it back in [`super::Loaded::info`], so
/// load then save with it gives the same bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveInfo {
    pub build_id: String,
    pub install_key: String,
    pub mod_key: String,
    /// The map file's path and hash, or `None` for a match whose map has no
    /// file (a generated map, an editor's test-play map): refused at save
    /// ([`super::SaveError::NoMapFile`]). Supporting those needs a decision.
    pub map: Option<MapId>,
    pub kind: GameKind,
    pub mission: MissionIdentity,
    pub local_seat: Option<PlayerId>,
    pub play_time_secs: u64,
    pub made_at: u64,
    pub description: String,
}
