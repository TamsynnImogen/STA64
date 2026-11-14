//! Snapshot save/load (SIM-10): the whole simulation as bytes, so a game can
//! be saved, a mission can `Start=snapshot:`, and a bench can rewind.
//!
//! Layout: `MAGIC`, [`FORMAT_VERSION`] (u16 LE), the seam revision (u32 LE),
//! the content hash (u64 LE), then in `postcard` the [`Provenance`] (content
//! lineage and pack names, D10), the [`Body`], the map bounds and the
//! versioned confinement, perceived-team, personnel and unit-crew records.
//! The fixed header is read first so a wrong file, version or content set is
//! refused before anything else is parsed; [`snapshot_info`] reads the
//! header and provenance alone, for a load screen.
//!
//! The [`ContentDb`] is not stored (it is not serialisable, and the files
//! are the player's own): the snapshot keeps its hash and [`Sim::from_snapshot`]
//! takes the database from the caller and refuses one that hashes differently.
//! Lineage and packs are informational and never checked.
//!
//! Adding a subsystem's state (weapons from SIM-04, say) is one field on
//! [`Body`], filled in [`Sim::snapshot`] and read back in
//! [`Sim::from_snapshot`], plus a [`FORMAT_VERSION`] bump. Old layouts are
//! refused, not migrated. Format 14 (SIM-28) appended the pending start-helper
//! lifecycle rows; format 15 (CON-35) is the first with provenance and with
//! the keyed content digest's hash. Formats 8 to 14, which earlier builds
//! read with narrow compatibility paths, carry a content hash of the old
//! kind that matches no current database, so they are refused by version.
//!
//! The state types of other crates (`MoveStats`) have no serde, so the body
//! uses small mirror records here instead of touching those crates.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, MoveStats, Region, StartPos};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, SEAM_REV, TeamId, Vec3};

use crate::combat::weapons::{Weapons, WeaponsRec};
use crate::movement::{Course, Movement};
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, EntityKind, Shields, Table};
use crate::{Rng, Sim, SimEvent};

pub const MAGIC: [u8; 4] = *b"STAS";
/// Format 16 (SIM-38) appended craft autonomy after the start-helper rows; 15
/// (CON-35) added the [`Provenance`] record and the keyed content digest's
/// hash. Format 17 appends the mission-set subsystem switches and damage
/// (SIM-55). Format 18 (SIM-36) appends the attack approach record
/// (crafts in warp). Format 19 appends stuck-order watch rows; format 20
/// adds their anchor, start distance and birth tick. Format 21 appends scripted
/// orders (SIM-56) and per-craft switches (SIM-58). Format 22 appends mission
/// attributes (SIM-54). Format 23 appends cosmetic seat factions. Format 24
/// adds the derelict latch to the subsystem rows (SIM-39).
/// Only the current format is read.
pub const FORMAT_VERSION: u16 = 24;

const HEADER: usize = 4 + 2 + 4 + 8;

/// Where the snapshot's content came from (D10): shown, never compared.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// `ContentDb::lineage`; empty for a sim without content.
    pub lineage: String,
    /// `ContentDb::packs`, top first.
    pub packs: Vec<String>,
}

/// What a snapshot says about itself without loading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub content_hash: u64,
    pub provenance: Provenance,
}

/// Reads a snapshot's header and [`Provenance`] only, refusing a wrong file,
/// format or seam revision as [`Sim::from_snapshot`] does.
pub fn snapshot_info(bytes: &[u8]) -> Result<SnapshotInfo, SnapshotError> {
    let content_hash = check_header(bytes)?;
    let (provenance, _): (Provenance, _) =
        postcard::take_from_bytes(&bytes[HEADER..]).map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
    Ok(SnapshotInfo { content_hash, provenance })
}

/// Magic, format and seam revision; the saved content hash.
fn check_header(bytes: &[u8]) -> Result<u64, SnapshotError> {
    if bytes.len() < HEADER || bytes[..4] != MAGIC {
        return Err(SnapshotError::BadMagic);
    }
    let found = u16::from_le_bytes([bytes[4], bytes[5]]);
    if found != FORMAT_VERSION {
        return Err(SnapshotError::FormatVersion { found });
    }
    let seam = u32::from_le_bytes(bytes[6..10].try_into().unwrap());
    if seam != SEAM_REV {
        return Err(SnapshotError::SeamRev { found: seam });
    }
    Ok(u64::from_le_bytes(bytes[10..18].try_into().unwrap()))
}

/// Why a snapshot could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// Not a snapshot (wrong or missing magic).
    BadMagic,
    /// Written by a different layout of this file.
    FormatVersion { found: u16 },
    /// Written against a different seam revision.
    SeamRev { found: u32 },
    /// The content database is not the one the snapshot was taken with
    /// (hash 0 means none).
    ContentMismatch { saved: u64, given: u64 },
    /// Truncated, garbled, or inconsistent (ids out of order).
    Corrupt(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::BadMagic => write!(f, "not a snapshot"),
            SnapshotError::FormatVersion { found } if *found < FORMAT_VERSION => {
                write!(f, "snapshot format {found} is from an older STA64 and cannot be read (this build reads format {FORMAT_VERSION} only)")
            }
            SnapshotError::FormatVersion { found } => {
                write!(f, "snapshot format {found}, expected {FORMAT_VERSION}")
            }
            SnapshotError::SeamRev { found } => {
                write!(f, "snapshot seam revision {found}, expected {SEAM_REV}")
            }
            SnapshotError::ContentMismatch { saved, given } => {
                write!(
                    f,
                    "snapshot content hash {saved:#018x}, given database {given:#018x}"
                )
            }
            SnapshotError::Corrupt(why) => write!(f, "corrupt snapshot: {why}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// SIM-36's attack approach state: the crafts in warp (phase 1) or dropping
/// out of it (2), ascending id.
#[derive(Serialize, Deserialize)]
struct AttackRec {
    version: u8,
    warp: Vec<(EntityId, u8)>,
    /// Recent hits `(tick, attacker)` and shots, for assists.
    engagement: crate::combat::targeting::EngagementRec,
}

#[derive(Serialize, Deserialize)]
struct StatsRec {
    max_speed: Fixed,
    forward_accel: Fixed,
    turn_omega: Fixed,
    turn_alpha: Fixed,
    turn_at_rest: Fixed,
    /// The smooth-physics fields (SIM-15), as `MoveStats::smooth_fields`.
    smooth: [Fixed; 7],
}

#[derive(Serialize, Deserialize)]
struct EntityRec {
    id: EntityId,
    kind: u8,
    def: ContentKey,
    owner: PlayerId,
    pos: Vec3,
    heading: (Fixed, Fixed),
    speed: Fixed,
    omega: Fixed,
    target: Option<Vec3>,
    stats: StatsRec,
    hull: [Fixed; 2],
    shields_now: [Fixed; 4],
    shields_max: Fixed,
    shields_directional: bool,
    flags: UnitFlags,
    progress: Option<Fixed>,
}

/// Format 23: cosmetic owning-seat faction labels. No gameplay rules.
#[derive(Serialize, Deserialize)]
struct AppearanceRec {
    version: u8,
    factions: Vec<(PlayerId, String)>,
}

/// Format 19, changed in 20: the stuck-order watch rows (`watchdog`).
#[derive(Serialize, Deserialize)]
struct WatchRec {
    version: u8,
    rows: Vec<(EntityId, crate::watchdog::Row)>,
}

#[derive(Serialize, Deserialize)]
struct CourseRec {
    waypoints: Vec<Vec3>,
    formation: u8,
}

/// Everything in [`Sim`] but the content database. One field per subsystem.
#[derive(Serialize, Deserialize)]
struct Body {
    tick: u64,
    rng: u64,
    players: Vec<(PlayerId, TeamId)>,
    /// SIM-09 AI-controlled seats.
    ai_seats: Vec<PlayerId>,
    starts: Vec<StartPos>,
    regions: Vec<Region>,
    next_entity: u32,
    entities: Vec<EntityRec>,
    labels: Vec<(EntityId, String)>,
    events: Vec<SimEvent>,
    /// SIM-02 courses and formations.
    courses: Vec<(EntityId, CourseRec)>,
    /// SIM-04 mounts, attack orders and projectiles in flight.
    weapons: WeaponsRec,
    /// SIM-07 stock, nodes and miners (build costs and refunds use the stock).
    economy: crate::economy::EconomyRec,
    /// SIM-08 yard queues, rig deploys and welds.
    production: crate::production::ProductionRec,
    /// SIM-13 eliminated seats.
    victory: crate::victory::VictoryRec,
    /// SIM-14 special energy and cloaks.
    specials: crate::specials::SpecialsRec,
}

/// Mandatory v10+ suffix; the old body stays byte-for-byte unchanged.
#[derive(Serialize, Deserialize)]
struct BoundaryRec {
    version: u8,
    rows: Vec<(EntityId, bool)>,
}

/// Mandatory v11 suffix after the confinement record.
#[derive(Serialize, Deserialize)]
struct PerceptionRec {
    version: u8,
    rows: Vec<(EntityId, PlayerId)>,
}

/// Mandatory v12 suffix. Earlier Body and suffix schemas are unchanged.
#[derive(Serialize, Deserialize)]
struct PersonnelRec {
    version: u8,
    rows: Vec<(PlayerId, Option<i64>, Option<i64>)>,
}

/// Mandatory v13 suffix; absence from rows means unknown, not full capacity.
#[derive(Serialize, Deserialize)]
struct UnitCrewRec {
    version: u8,
    rows: Vec<(EntityId, [i64; 2])>,
}

/// Mandatory suffix since v14: pending start-helper lifecycle rows (SIM-28).
#[derive(Serialize, Deserialize)]
struct StartEffectsRec {
    version: u8,
    rows: crate::start_effects::Rows,
}

/// Mandatory suffix since v17: switched-off and damaged subsystems (SIM-55).
#[derive(Serialize, Deserialize)]
struct SubsystemsRec {
    version: u8,
    rows: Vec<(EntityId, crate::combat::damage::SubsystemRow)>,
}

/// Format 21: scripted orders (SIM-56).
#[derive(Serialize, Deserialize)]
struct ScriptedRec {
    version: u8,
    rows: Vec<(EntityId, crate::orders::Job)>,
}

/// Format 21: per-craft switches (SIM-58).
#[derive(Serialize, Deserialize)]
struct SwitchesRec {
    version: u8,
    rows: Vec<(EntityId, crate::switches::Switches)>,
}

fn kind_from(b: u8) -> Option<EntityKind> {
    Some(match b {
        0 => EntityKind::Craft,
        1 => EntityKind::Station,
        2 => EntityKind::Resource,
        3 => EntityKind::Projectile,
        4 => EntityKind::Prop,
        _ => return None,
    })
}

impl EntityRec {
    fn of(e: &Entity) -> EntityRec {
        EntityRec {
            id: e.id,
            kind: e.kind as u8,
            def: e.def,
            owner: e.owner,
            pos: e.pos,
            heading: e.heading,
            speed: e.speed,
            omega: e.omega,
            target: e.target,
            stats: StatsRec {
                max_speed: e.stats.max_speed,
                forward_accel: e.stats.forward_accel,
                turn_omega: e.stats.turn_omega,
                turn_alpha: e.stats.turn_alpha,
                turn_at_rest: e.stats.turn_at_rest,
                smooth: e.stats.smooth_fields(),
            },
            hull: e.hull,
            shields_now: e.shields.now,
            shields_max: e.shields.max,
            shields_directional: e.shields.directional,
            flags: e.flags,
            progress: e.progress,
        }
    }

    fn into_entity(self) -> Result<Entity, SnapshotError> {
        let kind = kind_from(self.kind)
            .ok_or_else(|| SnapshotError::Corrupt(format!("entity kind {}", self.kind)))?;
        let s = self.stats;
        Ok(Entity {
            id: self.id,
            kind,
            def: self.def,
            owner: self.owner,
            pos: self.pos,
            heading: self.heading,
            speed: self.speed,
            omega: self.omega,
            target: self.target,
            stats: MoveStats {
                max_speed: s.max_speed,
                forward_accel: s.forward_accel,
                turn_omega: s.turn_omega,
                turn_alpha: s.turn_alpha,
                turn_at_rest: s.turn_at_rest,
                combat_speed: s.smooth[0],
                turn_alpha_at_rest: s.smooth[1],
                too_close_to_turn: s.smooth[2],
                path_lead: s.smooth[3],
                turn_control_angle: s.smooth[4],
                turn_control_squared: s.smooth[5] != Fixed::ZERO,
                forward_control_distance: s.smooth[6],
                // Not in the snapshot: `from_snapshot` re-reads it from the
                // definition once the entities exist.
                warp_speed: Fixed::ZERO,
                too_close_for_warp: Fixed::ZERO,
                heading_close_for_warp: Fixed::ZERO,
            },
            hull: self.hull,
            crew: None,
            shields: Shields {
                now: self.shields_now,
                max: self.shields_max,
                directional: self.shields_directional,
            },
            flags: self.flags,
            progress: self.progress,
        })
    }
}

fn corrupt(what: &str) -> SnapshotError {
    SnapshotError::Corrupt(format!("{what} out of order"))
}

impl Sim {
    /// The whole simulation as bytes. Loading them with the same content
    /// database gives a sim that steps exactly as this one would.
    pub fn snapshot(&self) -> Vec<u8> {
        let body = Body {
            tick: self.tick,
            rng: self.rng.0,
            players: self.players.clone(),
            ai_seats: self.ai_seats.clone(),
            starts: self.starts.clone(),
            regions: self.regions.clone(),
            next_entity: self.units.next_id().0,
            entities: self.units.iter().map(EntityRec::of).collect(),
            labels: self.labels.iter().map(|(id, l)| (id, l.clone())).collect(),
            events: self.events.clone(),
            courses: self
                .movement
                .rows()
                .into_iter()
                .map(|(id, c)| {
                    (
                        id,
                        CourseRec {
                            waypoints: c.waypoints,
                            formation: c.formation,
                        },
                    )
                })
                .collect(),
            weapons: self.weapons.to_rec(),
            economy: self.economy.to_rec(),
            production: self.production.to_rec(),
            victory: self.victory.to_rec(),
            specials: self.specials.to_rec(),
        };
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&SEAM_REV.to_le_bytes());
        out.extend_from_slice(&self.content.as_ref().map_or(0, |c| c.hash()).to_le_bytes());
        let provenance = self.content.as_ref().map_or_else(Provenance::default, |c| Provenance {
            lineage: c.lineage().to_owned(),
            packs: c.packs().to_vec(),
        });
        out.extend(postcard::to_allocvec(&provenance).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&body).expect("snapshot serialises"));
        // The new suffix is mandatory, even without content, bounds or actors.
        out.extend(postcard::to_allocvec(&self.bounds).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&BoundaryRec { version: 1, rows: self.confinement.rows() }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&PerceptionRec {
            version: 1,
            rows: self.perception.iter().map(|(id, player)| (id, *player)).collect(),
        }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&PersonnelRec { version: 1, rows: self.economy.personnel_rec() }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&UnitCrewRec {
            version: 1,
            rows: self.units.iter().filter_map(|unit| unit.crew.map(|pool| (unit.id, pool.map(Fixed::to_bits)))).collect(),
        }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&StartEffectsRec { version: 1, rows: self.start_effects.to_rec() }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&crate::autonomy::AutonomyWire { version: 1, rows: self.autonomy.to_rec() }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&SubsystemsRec { version: 1, rows: self.subsystems.to_rows() }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&AttackRec { version: 1, warp: self.approach_warp.to_rec(), engagement: self.engagement.to_rec() }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&WatchRec { version: 1, rows: self.watchdog.to_rec() }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&ScriptedRec { version: 1, rows: self.scripted.to_rows() }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&SwitchesRec { version: 1, rows: self.switches.to_rows() }).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&self.attrs.to_rec()).expect("snapshot serialises"));
        out.extend(postcard::to_allocvec(&AppearanceRec { version: 1, factions: self.appearance_factions.clone() }).expect("snapshot serialises"));
        out
    }

    /// Rebuild a sim from [`Sim::snapshot`]. `content` must be the database
    /// the snapshot was taken with (`None` for a sim that had none); its hash
    /// is checked.
    pub fn from_snapshot(
        bytes: &[u8],
        content: Option<Arc<ContentDb>>,
    ) -> Result<Sim, SnapshotError> {
        let saved = check_header(bytes)?;
        let given = content.as_ref().map_or(0, |c| c.hash());
        if saved != given {
            return Err(SnapshotError::ContentMismatch { saved, given });
        }
        let corrupt_postcard = |e: postcard::Error| SnapshotError::Corrupt(e.to_string());
        // Informational; read past (and checked to parse).
        let (_, rest): (Provenance, &[u8]) = postcard::take_from_bytes(&bytes[HEADER..]).map_err(corrupt_postcard)?;
        let (b, rest): (Body, &[u8]) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        let (bounds, rest) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        let (record, rest): (BoundaryRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown confinement record version".into())); }
        let boundary_rows = record.rows;
        let (record, rest): (PerceptionRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown perceived-team record version".into())); }
        let perception_rows = record.rows;
        let (record, rest): (PersonnelRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown personnel record version".into())); }
        let personnel_rows = record.rows;
        let (record, rest): (UnitCrewRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown unit crew record version".into())); }
        let crew_rows = record.rows;
        let (record, rest): (StartEffectsRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown start effects record version".into())); }
        let effect_rows = record.rows;
        let (record, rest): (crate::autonomy::AutonomyWire, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown autonomy record version".into())); }
        let autonomy_rows = record.rows;
        let (record, rest): (SubsystemsRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown subsystems record version".into())); }
        let subsystem_rows = record.rows;
        let (record, rest): (AttackRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown attack approach record version".into())); }
        let warp_rows = record.warp;
        let engagement_rows = record.engagement;
        let (record, rest): (WatchRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown order watch record version".into())); }
        let watch_rows = record.rows;
        let (record, rest): (ScriptedRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown scripted orders record version".into())); }
        let scripted_rows = record.rows;
        let (record, rest): (SwitchesRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown switches record version".into())); }
        let switch_rows = record.rows;
        let (record, rest): (crate::attrs::AttrsRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        let attrs = crate::attrs::Attrs::from_rec(record).map_err(|why| SnapshotError::Corrupt(why.into()))?;
        let (record, rest): (AppearanceRec, _) = postcard::take_from_bytes(rest).map_err(corrupt_postcard)?;
        if record.version != 1 { return Err(SnapshotError::Corrupt("unknown appearance record version".into())); }
        let appearance_factions = record.factions;
        if !appearance_factions.windows(2).all(|w| w[0].0 < w[1].0)
            || appearance_factions.iter().any(|(id, name)| name.is_empty() || name.trim() != name || name.to_ascii_lowercase() != *name || !b.players.iter().any(|p| p.0 == *id)) {
            return Err(SnapshotError::Corrupt("invalid appearance faction rows".into()));
        }
        if !rest.is_empty() { return Err(SnapshotError::Corrupt("trailing bytes".into())); }
        if let Some(row) = b.economy.miners.iter().find(|row| row.5 > 4) {
            return Err(SnapshotError::Corrupt(format!("mining phase {} is not supported", row.5)));
        }

        let list = b
            .entities
            .into_iter()
            .map(EntityRec::into_entity)
            .collect::<Result<Vec<_>, _>>()?;
        let mut units =
            Entities::from_parts(list, b.next_entity).ok_or_else(|| corrupt("entity ids"))?;
        // `warpSpeed` is a per-definition stat that a spawn copies and the
        // snapshot does not carry: re-read it (typed `UnitDef` field) so a
        // restored sim equals a live one.
        if let Some(db) = content.as_deref() {
            let ids: Vec<EntityId> = units.iter().map(|u| u.id).collect();
            for id in ids {
                let Some(unit) = units.get_mut(id) else { continue };
                if let Some(def) = db.unit(unit.def) {
                    unit.stats.warp_speed = def.movement.warp_speed;
                    unit.stats.too_close_for_warp = def.movement.too_close_for_warp;
                    unit.stats.heading_close_for_warp = def.movement.heading_close_for_warp;
                }
            }
        }
        if crew_rows.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(corrupt("unit crew rows"));
        }
        for (id, [now, max]) in crew_rows {
            // Native creation permits starting crew above an authored capacity;
            // preserve both independent nonnegative values without refilling.
            if now < 0 || max < 0 {
                return Err(SnapshotError::Corrupt("invalid unit crew pool".into()));
            }
            let unit = units.get_mut(id).ok_or_else(|| SnapshotError::Corrupt("unknown unit crew entity".into()))?;
            if !matches!(unit.kind, EntityKind::Craft | EntityKind::Station) {
                return Err(SnapshotError::Corrupt("unit crew entity is not a craft or station".into()));
            }
            unit.crew = Some([Fixed::from_bits(now), Fixed::from_bits(max)]);
        }
        let (mut confinement, _) = crate::boundary::state::Confinement::prepare(content.as_deref(), bounds);
        confinement.restore(&units, boundary_rows).map_err(|why| SnapshotError::Corrupt(why.into()))?;
        let perception = crate::perception::restore(&units, perception_rows)
            .map_err(|why| SnapshotError::Corrupt(why.into()))?;
        let labels = Table::from_rows(b.labels).ok_or_else(|| corrupt("labels"))?;
        let courses = b
            .courses
            .into_iter()
            .map(|(id, c)| {
                (
                    id,
                    Course {
                        waypoints: c.waypoints,
                        formation: c.formation,
                    },
                )
            })
            .collect();
        let movement = Movement::from_rows(courses).ok_or_else(|| corrupt("courses"))?;
        {
            let v = attrs.view();
            if v.races.iter().any(|(id, _)| !units.get(*id).is_some_and(|u| matches!(u.kind, EntityKind::Craft | EntityKind::Station))) {
                return Err(SnapshotError::Corrupt("attribute entity is not a craft or station".into()));
            }
            if v.player_races.iter().any(|(p, _)| !b.players.iter().any(|(id, _)| id == p)) {
                return Err(SnapshotError::Corrupt("attribute seat does not exist".into()));
            }
        }
        let weapons = Weapons::from_rec(b.weapons).ok_or_else(|| corrupt("weapons"))?;
        let autonomy = crate::autonomy::Autonomy::from_rec(autonomy_rows).ok_or_else(|| corrupt("autonomy"))?;
        if autonomy.to_rec().iter().any(|(id, _)| units.get(*id).is_none_or(|u| u.kind != EntityKind::Craft)) {
            return Err(corrupt("autonomy entity"));
        }
        if watch_rows.iter().any(|(_, r)| r.since > b.tick || r.anchored > b.tick || r.born > b.tick) {
            return Err(SnapshotError::Corrupt("order watch row from the future".into()));
        }
        let watchdog = crate::watchdog::Watchdog::from_rec(watch_rows).ok_or_else(|| corrupt("order watch rows"))?;
        if watchdog.to_rec().iter().any(|(id, _)| units.get(*id).is_none_or(|u| u.kind != EntityKind::Craft)) {
            return Err(SnapshotError::Corrupt("order watch entity is not a craft".into()));
        }
        let scripted = crate::orders::Scripted::from_rows(scripted_rows).ok_or_else(|| corrupt("scripted order rows"))?;
        if scripted.to_rows().iter().any(|(id, j)| units.get(*id).is_none_or(|u| u.kind != EntityKind::Craft) || matches!(j, crate::orders::Job::Patrol { points, leg } if points.len() < 2 || *leg >= points.len())) {
            return Err(SnapshotError::Corrupt("scripted order entity is not a craft or a patrol has no loop".into()));
        }
        let switches = crate::switches::SwitchTable::from_rows(switch_rows).ok_or_else(|| corrupt("switch rows"))?;
        if switches.to_rows().iter().any(|(id, _)| units.get(*id).is_none()) {
            return Err(SnapshotError::Corrupt("switches of an unknown entity".into()));
        }
        let approach_warp = crate::movement::approach::Warp::from_rec(warp_rows).ok_or_else(|| SnapshotError::Corrupt("attack warp rows out of order or of an unknown phase".into()))?;
        if approach_warp.to_rec().iter().any(|(id, _)| units.get(*id).is_none_or(|u| u.kind != EntityKind::Craft)) {
            return Err(SnapshotError::Corrupt("attack warp entity is not a craft".into()));
        }
        let engagement = crate::combat::targeting::Engagement::from_rec(engagement_rows, b.tick, |id| units.contains(id))
            .ok_or_else(|| SnapshotError::Corrupt("engagement rows out of order, from the future or of an unknown entity".into()))?;
        let specials = crate::specials::Specials::from_rec(b.specials).ok_or_else(|| corrupt("specials"))?;
        let start_effects = crate::start_effects::StartEffects::from_rec(effect_rows, |id| units.contains(id))
            .ok_or_else(|| corrupt("start effect rows"))?;
        let subsystems = crate::combat::damage::Subsystems::from_rows(subsystem_rows).ok_or_else(|| SnapshotError::Corrupt("invalid subsystem rows".into()))?;
        if subsystems.to_rows().iter().any(|(id, _)| !units.get(*id).is_some_and(|u| matches!(u.kind, EntityKind::Craft | EntityKind::Station))) {
            return Err(SnapshotError::Corrupt("subsystem entity is not a craft or station".into()));
        }
        if let Some(db) = content.as_deref() {
            for (id, row) in subsystems.to_rows() {
                let hp = units.get(id).and_then(|u| db.unit(u.def)).map(|d| d.system_hit_points);
                if hp.is_some_and(|hp| row.damage.iter().zip(hp).any(|(d, hp)| *d > hp.to_bits())) {
                    return Err(SnapshotError::Corrupt("subsystem damage above hit points".into()));
                }
            }
        }
        let mut economy = crate::economy::Economy::from_rec(&b.economy);
        economy.restore_personnel(personnel_rows, &b.players).map_err(|why| SnapshotError::Corrupt(why.into()))?;
        Ok(Sim {
            tick: b.tick,
            rng: Rng(b.rng),
            units,
            labels,
            players: b.players,
            ai_seats: b.ai_seats,
            content,
            starts: b.starts,
            regions: b.regions,
            bounds,
            confinement,
            perception,
            events: b.events,
            movement,
            weapons,
            economy,
            production: crate::production::Production::from_rec(&b.production).ok_or_else(|| corrupt("rallies"))?,
            victory: crate::victory::Victory::from_rec(&b.victory),
            specials,
            autonomy,
            approach_warp,
            watchdog,
            engagement,
            start_effects,
            nav: Default::default(),
            subsystems,
            scripted,
            switches,
            attrs,
            team_changes: Vec::new(),
            appearance_factions,
        })
    }
}

#[cfg(test)]
#[path = "snapshot/subsystem_tests.rs"]
mod subsystem_tests;

#[cfg(test)]
#[path = "snapshot/attrs_tests.rs"]
mod attrs_tests;

#[cfg(test)]
#[path = "snapshot/appearance_tests.rs"]
mod appearance_tests;

#[cfg(test)]
#[path = "snapshot/attack_tests.rs"]
mod attack_tests;
#[cfg(test)]
#[path = "snapshot/watch_tests.rs"]
mod watch_tests;
#[cfg(test)]
#[path = "snapshot/scripted_tests.rs"]
mod scripted_tests;

#[cfg(test)]
#[path = "snapshot/chain.rs"]
mod chain;

#[cfg(test)]
#[path = "snapshot/confinement_tests.rs"]
mod confinement_tests;

#[cfg(test)]
#[path = "snapshot/perception_tests.rs"]
mod perception_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldSource;
    use crate::command::{Command, CommandKind, Order};
    use sta_core::TeamId;

    fn v(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3 {
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
            z: Fixed::from_int(z),
        }
    }

    fn cmd(seq: u32, o: Order) -> Command {
        Command {
            player: PlayerId(0),
            seq,
            kind: CommandKind::Order(o),
        }
    }

    /// A few units with queued waypoints and a formation, mid-flight.
    fn busy_sim() -> Sim {
        let mut sim = Sim::new(7);
        let st = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
        let ids: Vec<_> = (0..4)
            .map(|i| sim.spawn(0, v(i * 50, 0, 0), (Fixed::ZERO, Fixed::ONE), st))
            .collect();
        sim.labels.insert(ids[1], "flagship".into());
        sim.step(&[
            cmd(
                0,
                Order::Formation {
                    units: ids.clone(),
                    kind: 1,
                },
            ),
            cmd(
                1,
                Order::Move {
                    units: ids.clone(),
                    to: v(400, 0, 300),
                    queue: false,
                },
            ),
            cmd(
                2,
                Order::Move {
                    units: ids.clone(),
                    to: v(-300, 20, 900),
                    queue: true,
                },
            ),
        ]);
        for _ in 0..40 {
            sim.rng.next_u64();
            sim.step(&[]);
        }
        sim
    }

    #[test]
    fn reloaded_digest_equals_continued_run_300_ticks_on() {
        let mut live = busy_sim();
        let mut loaded = Sim::from_snapshot(&live.snapshot(), None).unwrap();
        assert_eq!(loaded.digest(), live.digest());
        // Orders issued after the load, too.
        for t in 0..300 {
            let cmds = if t == 100 {
                vec![cmd(
                    0,
                    Order::Move {
                        units: vec![EntityId(2)],
                        to: v(0, 0, -500),
                        queue: false,
                    },
                )]
            } else {
                vec![]
            };
            let a = live.step(&cmds);
            let b = loaded.step(&cmds);
            assert_eq!(a.view.digest, b.view.digest, "tick {t}");
        }
        assert_eq!(loaded.digest(), live.digest());
        assert_eq!(loaded.tick, live.tick);
    }

    /// A cruiser attacking a frigate: a torpedo is in flight at the snapshot.
    #[test]
    fn mid_combat_snapshot_continues_identically() {
        use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
        let seat = |id| PlayerSetup {
            id: PlayerId(id),
            team: TeamId(id),
            race: "fx".into(),
            control: Control::Idle,
        };
        let db = sta_content::fake::db();
        let (cruiser, frigate) = (db.key("fx_cruiser").unwrap(), db.key("fx_frigate").unwrap());
        let db = Arc::new(db);
        let setup = GameSetup {
            seed: 7,
            map: Arc::new(sta_content::MapDef::empty()),
            content: Arc::clone(&db),
            players: vec![seat(1), seat(2)],
            options: GameOptions::default(),
        };
        let mut live = Sim::from_setup(&setup);
        let ahead = (Fixed::ZERO, Fixed::ONE);
        let s = live
            .spawn_def(cruiser, PlayerId(1), v(2000, 0, 2000), ahead)
            .unwrap();
        let t = live
            .spawn_def(frigate, PlayerId(2), v(2000, 0, 2350), ahead)
            .unwrap();
        let attack = Command {
            player: PlayerId(1),
            seq: 0,
            kind: CommandKind::Order(Order::Attack {
                units: vec![s],
                target: t,
            }),
        };
        live.step(&[attack]);
        for _ in 0..5 {
            live.step(&[]);
        }
        assert!(
            live.weapons.shots_in_flight() > 0,
            "snapshot should catch a shot in flight"
        );
        assert_eq!(live.weapons.attack_of(s), Some(t));
        let mut loaded = Sim::from_snapshot(&live.snapshot(), Some(Arc::clone(&db))).unwrap();
        assert_eq!(loaded.digest(), live.digest());
        let mut events = 0;
        for tick in 0..300 {
            let a = live.step(&[]);
            let b = loaded.step(&[]);
            assert_eq!(a.view.digest, b.view.digest, "tick {tick}");
            assert_eq!(a.events, b.events, "tick {tick}");
            events += a.events.len();
        }
        assert!(events > 0, "combat should produce events");
        assert_eq!(loaded.snapshot(), live.snapshot());
    }

    /// A yard half through two ships and a rig welding: loaded matches live
    /// across a `Built`.
    #[test]
    fn mid_build_snapshot_continues_identically() {
        use crate::production::tests::mid_build;
        let (mut live, db) = mid_build();
        assert!(!live.production.is_empty());
        let mut loaded = Sim::from_snapshot(&live.snapshot(), Some(Arc::clone(&db))).unwrap();
        assert_eq!(loaded.digest(), live.digest());
        let mut built = 0;
        for tick in 0..400 {
            let a = live.step(&[]);
            let b = loaded.step(&[]);
            assert_eq!(a.view.digest, b.view.digest, "tick {tick}");
            assert_eq!(a.events, b.events, "tick {tick}");
            built += a.events.iter().filter(|e| matches!(e, SimEvent::Built { .. })).count();
        }
        assert!(built >= 1, "a build should finish after the snapshot");
        assert_eq!(loaded.snapshot(), live.snapshot());
    }

    /// Step `live` and a copy loaded from its snapshot `n` ticks; they agree
    /// on every digest and event list and end with the same bytes.
    fn assert_continues(live: &mut Sim, db: Arc<sta_content::ContentDb>, n: u64) -> usize {
        let mut loaded = Sim::from_snapshot(&live.snapshot(), Some(db)).unwrap();
        assert_eq!(loaded.digest(), live.digest());
        let mut events = 0;
        for tick in 0..n {
            let a = live.step(&[]);
            let b = loaded.step(&[]);
            assert_eq!(a.view.digest, b.view.digest, "tick {tick}");
            assert_eq!(a.events, b.events, "tick {tick}");
            events += a.events.iter().filter(|e| matches!(e, SimEvent::Delivered { .. } | SimEvent::Fired { .. })).count();
        }
        assert_eq!(loaded.snapshot(), live.snapshot());
        events
    }

    #[test]
    fn mid_mining_snapshot_continues_identically() {
        let (mut live, db) = crate::economy::tests::mid_mining();
        assert!(!live.economy.is_empty());
        let delivered = assert_continues(&mut live, db, 300);
        assert!(delivered > 0, "a delivery falls inside the window");
    }

    const MINING_V8: &[u8] = include_bytes!("../tests/fixtures/mining-v8.stas");

    /// The game `tests/fixtures/mining-v8.stas` was saved from (its JSON
    /// record): seed 3, fake content, empty map, seats 0 and 1 idle, moon,
    /// station and miner at the origin, seat 0's mine order, 99 more ticks.
    /// Format 8 is refused since CON-35, so the state is rebuilt here.
    fn mining_v8_state() -> (Sim, Arc<sta_content::ContentDb>) {
        use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
        let db = Arc::new(sta_content::fake::db());
        let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
        let mut sim = Sim::from_setup(&GameSetup {
            seed: 3,
            map: Arc::new(sta_content::MapDef::empty()),
            content: db.clone(),
            players: vec![seat(0), seat(1)],
            options: GameOptions::default(),
        });
        let ahead = (Fixed::ZERO, Fixed::ONE);
        for (odf, owner) in [("fx_moon", crate::world::NEUTRAL), ("fx_station", PlayerId(0)), ("fx_miner", PlayerId(0))] {
            sim.spawn_def(db.key(odf).unwrap(), owner, Vec3::ZERO, ahead).unwrap();
        }
        sim.step(&[cmd(0, Order::Mine { units: vec![EntityId(3)], node: EntityId(1) })]);
        for _ in 0..99 {
            sim.step(&[]);
        }
        (sim, db)
    }

    #[test]
    fn original_v8_mining_game_is_refused_as_a_file_and_rebuilt_keeps_its_continuation() {
        // The genuine file is refused by version now ...
        assert_eq!(u16::from_le_bytes(MINING_V8[4..6].try_into().unwrap()), 8);
        let db = Arc::new(sta_content::fake::db());
        assert_eq!(Sim::from_snapshot(MINING_V8, Some(db)).err(), Some(SnapshotError::FormatVersion { found: 8 }));
        // ... and the same game rebuilt keeps its recorded digests (crew
        // and the post-CON-35 content hash left out, as they were then).
        let (mut live, _) = mining_v8_state();
        assert_eq!(live.tick, 100);
        assert_eq!(crate::unit_status_tests::digest_before_crew(&live), 0x14c7_0e04_4af1_fb70);
        assert_eq!(live.economy.to_rec().miners[0].3, Fixed::from_int(50).to_bits());
        for _ in 0..800 { live.step(&[]); }
        assert_eq!(live.tick, 900);
        assert_eq!(crate::unit_status_tests::digest_before_crew(&live), 0x1fad_adc8_8e06_0e5a);
        assert_eq!(live.view().player(PlayerId(0)).unwrap().resources[0], Fixed::from_int(150));
        assert_eq!(u16::from_le_bytes(live.snapshot()[4..6].try_into().unwrap()), FORMAT_VERSION);
    }

    #[test]
    fn a_snapshot_names_its_lineage_and_packs() {
        let packs = vec!["Fleet Ops 4.0".to_owned(), "Data".to_owned()];
        let db = Arc::new(sta_content::fake::db().with_packs(packs.clone()));
        let mut sim = Sim::new(5);
        sim.content = Some(db.clone());
        let bytes = sim.snapshot();
        let info = snapshot_info(&bytes).unwrap();
        assert_eq!(info.content_hash, db.hash());
        assert_eq!(info.provenance, Provenance { lineage: "native".into(), packs });
        // Informational: content with the same digest loads, whatever its packs.
        let loaded = Sim::from_snapshot(&bytes, Some(Arc::new(sta_content::fake::db()))).unwrap();
        assert_eq!(loaded.digest(), sim.digest());
        assert_eq!(snapshot_info(&Sim::new(5).snapshot()).unwrap().provenance, Provenance::default());
    }

    fn stopped_loaded_miner() -> (Sim, Arc<sta_content::ContentDb>) {
        let (mut sim, db) = mining_v8_state();
        let stopped = sim.step(&[Command {
            player: PlayerId(0), seq: 0,
            kind: CommandKind::Order(Order::Stop { units: vec![EntityId(3)] }),
        }]);
        assert!(!stopped.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
        let rec = sim.economy.to_rec();
        assert_eq!(rec.miners.len(), 1);
        assert_eq!((rec.miners[0].3, rec.miners[0].5), (Fixed::from_int(50).to_bits(), 4));
        (sim, db)
    }

    #[test]
    fn suspended_cargo_snapshot_stays_idle_and_resumes_identically() {
        let (mut live, db) = stopped_loaded_miner();
        assert_eq!(assert_continues(&mut live, db.clone(), 120), 0);
        assert_eq!(live.economy.to_rec().miners[0].3, Fixed::from_int(50).to_bits());
        assert_eq!(live.view().player(PlayerId(0)).unwrap().resources.first().copied().unwrap_or(Fixed::ZERO), Fixed::ZERO);
        let mut loaded = Sim::from_snapshot(&live.snapshot(), Some(db)).unwrap();
        let order = Command {
            player: PlayerId(0), seq: 1,
            kind: CommandKind::Order(Order::Mine { units: vec![EntityId(3)], node: EntityId(1) }),
        };
        let a = live.step(std::slice::from_ref(&order));
        let b = loaded.step(&[order]);
        assert_eq!(a, b);
        for tick in 0..700 {
            let a = live.step(&[]);
            let b = loaded.step(&[]);
            assert_eq!(a, b, "tick {tick}");
        }
        assert_eq!(loaded.snapshot(), live.snapshot());
        assert!(live.view().player(PlayerId(0)).unwrap().resources[0] >= Fixed::from_int(50));
    }

    #[test]
    fn old_formats_and_unknown_mining_phases_are_refused() {
        let (sim, db) = stopped_loaded_miner();
        let mut bytes = include_bytes!("../tests/fixtures/mining-idle-v9.stas").to_vec();
        bytes[4..6].copy_from_slice(&8u16.to_le_bytes());
        assert_eq!(Sim::from_snapshot(&bytes, Some(db.clone())).err(), Some(SnapshotError::FormatVersion { found: 8 }));

        let bytes = sim.snapshot();
        let (provenance, rest): (Provenance, _) = postcard::take_from_bytes(&bytes[HEADER..]).unwrap();
        let (mut body, suffix): (Body, _) = postcard::take_from_bytes(rest).unwrap();
        body.economy.miners[0].5 = 5;
        let mut unknown = bytes[..HEADER].to_vec();
        unknown.extend(postcard::to_allocvec(&provenance).unwrap());
        unknown.extend(postcard::to_allocvec(&body).unwrap());
        unknown.extend_from_slice(suffix);
        assert!(matches!(Sim::from_snapshot(&unknown, Some(db)), Err(SnapshotError::Corrupt(why)) if why.contains("mining phase 5")));
    }

    #[test]
    fn mid_pd_saturation_snapshot_continues_identically() {
        let (mut live, db) = crate::combat::pd_tests::mid_saturation();
        assert!(live.weapons.shots_in_flight() > 0);
        let fired = assert_continues(&mut live, db, 300);
        assert!(fired > 0);
    }

    #[test]
    fn mid_pd_energy_snapshot_keeps_the_timer_and_continues_identically() {
        let (mut live, db) = crate::combat::engine_timing_tests::mid_saturation_energy();
        assert!(live.weapons.shots_in_flight() > 0);
        let fired = assert_continues(&mut live, db, 300);
        assert!(fired > 0);
    }

    /// A snapshot after P2 is eliminated (P3 still alive) continues the same,
    /// and the eliminated seat's orders are still rejected.
    #[test]
    fn snapshot_keeps_eliminated_seats() {
        use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
        let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
        let db = sta_content::fake::db();
        let (cruiser, frigate) = (db.key("fx_cruiser").unwrap(), db.key("fx_frigate").unwrap());
        let db = Arc::new(db);
        let setup = GameSetup {
            seed: 7,
            map: Arc::new(sta_content::MapDef::empty()),
            content: Arc::clone(&db),
            players: vec![seat(1), seat(2), seat(3)],
            options: GameOptions::default(),
        };
        let mut live = Sim::from_setup(&setup);
        let ahead = (Fixed::ZERO, Fixed::ONE);
        let s = live.spawn_def(cruiser, PlayerId(1), v(2000, 0, 2000), ahead).unwrap();
        let t = live.spawn_def(frigate, PlayerId(2), v(2000, 0, 2350), ahead).unwrap();
        live.spawn_def(frigate, PlayerId(3), v(6000, 0, 6000), ahead).unwrap();
        let attack = Command {
            player: PlayerId(1),
            seq: 0,
            kind: CommandKind::Order(Order::Attack { units: vec![s], target: t }),
        };
        live.step(&[attack]);
        for _ in 0..1500 {
            live.step(&[]);
        }
        assert!(live.is_eliminated(PlayerId(2)) && !live.is_eliminated(PlayerId(3)));
        let mut loaded = Sim::from_snapshot(&live.snapshot(), Some(Arc::clone(&db))).unwrap();
        assert!(loaded.is_eliminated(PlayerId(2)));
        assert_eq!(loaded.digest(), live.digest());
        let stop = Command { player: PlayerId(2), seq: 9, kind: CommandKind::Order(Order::Stop { units: vec![] }) };
        for tick in 0..300 {
            let cmds = if tick == 0 { vec![stop.clone()] } else { vec![] };
            let a = live.step(&cmds);
            let b = loaded.step(&cmds);
            assert_eq!(a.view.digest, b.view.digest, "tick {tick}");
            assert_eq!(a.events, b.events, "tick {tick}");
            if tick == 0 {
                assert!(b.events.contains(&SimEvent::Rejected { player: PlayerId(2), seq: 9 }));
            }
        }
        assert_eq!(loaded.snapshot(), live.snapshot());
    }

    #[test]
    fn snapshot_is_stable_and_round_trips_bytes() {
        let sim = busy_sim();
        let bytes = sim.snapshot();
        assert_eq!(bytes, sim.snapshot());
        assert_eq!(Sim::from_snapshot(&bytes, None).unwrap().snapshot(), bytes);
    }

    #[test]
    fn keeps_pending_events_and_removed_ids() {
        let mut sim = busy_sim();
        sim.despawn(EntityId(2), None);
        let next = sim.units.next_id();
        let mut loaded = Sim::from_snapshot(&sim.snapshot(), None).unwrap();
        assert_eq!(loaded.units.next_id(), next);
        assert!(loaded.unit(EntityId(2)).is_none());
        assert_eq!(loaded.step(&[]).events, sim.step(&[]).events);
    }

    #[test]
    fn content_is_checked_by_hash() {
        let db = Arc::new(sta_content::fake::db());
        let mut sim = busy_sim();
        sim.content = Some(Arc::clone(&db));
        let bytes = sim.snapshot();
        let loaded = Sim::from_snapshot(&bytes, Some(Arc::clone(&db))).unwrap();
        assert_eq!(loaded.digest(), sim.digest());
        assert!(matches!(
            Sim::from_snapshot(&bytes, None),
            Err(SnapshotError::ContentMismatch { .. })
        ));
        let plain = busy_sim().snapshot();
        assert!(matches!(
            Sim::from_snapshot(&plain, Some(db)),
            Err(SnapshotError::ContentMismatch { .. })
        ));
    }

    #[test]
    fn refuses_bad_input() {
        let bytes = busy_sim().snapshot();
        assert_eq!(
            Sim::from_snapshot(b"nope", None).unwrap_err(),
            SnapshotError::BadMagic
        );
        let mut b = bytes.clone();
        b[4] = 99;
        assert_eq!(
            Sim::from_snapshot(&b, None).unwrap_err(),
            SnapshotError::FormatVersion { found: 99 }
        );
        let mut b = bytes.clone();
        b[6] ^= 0xff;
        assert!(matches!(
            Sim::from_snapshot(&b, None),
            Err(SnapshotError::SeamRev { .. })
        ));
        assert!(matches!(
            Sim::from_snapshot(&bytes[..bytes.len() - 5], None),
            Err(SnapshotError::Corrupt(_))
        ));
        let mut b = bytes;
        b.push(0);
        assert!(matches!(
            Sim::from_snapshot(&b, None),
            Err(SnapshotError::Corrupt(_))
        ));
    }
}

#[cfg(test)]
#[path = "snapshot/personnel_tests.rs"]
mod personnel_tests;

#[cfg(test)]
#[path = "snapshot/unit_crew_tests.rs"]
mod unit_crew_tests;
