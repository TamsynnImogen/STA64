//! Snapshot save/load (SIM-10): the whole simulation as bytes, so a game can
//! be saved, a mission can `Start=snapshot:`, and a bench can rewind.
//!
//! Layout: `MAGIC`, [`FORMAT_VERSION`] (u16 LE), the seam revision (u32 LE),
//! the content hash (u64 LE), then the [`Body`] in `postcard`. The header is
//! read first so a wrong file, version or content set is refused before
//! anything else is parsed.
//!
//! The [`ContentDb`] is not stored (it is not serialisable, and the files
//! are the player's own): the snapshot keeps its hash and [`Sim::from_snapshot`]
//! takes the database from the caller and refuses one that hashes differently.
//!
//! Adding a subsystem's state (weapons from SIM-04, say) is one field on
//! [`Body`], filled in [`Sim::snapshot`] and read back in
//! [`Sim::from_snapshot`], plus a [`FORMAT_VERSION`] bump. Old layouts are
//! refused, not generally migrated. Formats 8/9 retain the old Body and optional
//! bounds; their narrow read path requires disabled confinement capability,
//! and format 8 accepts only its original mining phases. Format 10 adds explicit
//! bounds and a mandatory versioned confinement record after that old Body.
//! Format 11 appends a mandatory versioned perceived-team record.
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
/// Format 13 appends unit crew after the format-12 player-personnel suffix.
/// The Body and earlier suffix records remain unchanged.
pub const FORMAT_VERSION: u16 = 13;
const PERSONNEL_FORMAT: u16 = 12;
const PERCEPTION_FORMAT: u16 = 11;
const PREVIOUS_FORMAT: u16 = 10;
const LEGACY_CARGO_FORMAT: u16 = 9;
// Only compatible while Body is byte-for-byte the format-8 record schema.
const COMPATIBLE_MINING_FORMAT: u16 = 8;

const HEADER: usize = 4 + 2 + 4 + 8;

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
            SnapshotError::FormatVersion { found } => {
                write!(f, "snapshot format {found}, expected {FORMAT_VERSION} or compatible {PERSONNEL_FORMAT}/{PERCEPTION_FORMAT}/{PREVIOUS_FORMAT}/{LEGACY_CARGO_FORMAT}/{COMPATIBLE_MINING_FORMAT}")
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
        out
    }

    /// Rebuild a sim from [`Sim::snapshot`]. `content` must be the database
    /// the snapshot was taken with (`None` for a sim that had none); its hash
    /// is checked.
    pub fn from_snapshot(
        bytes: &[u8],
        content: Option<Arc<ContentDb>>,
    ) -> Result<Sim, SnapshotError> {
        if bytes.len() < HEADER || bytes[..4] != MAGIC {
            return Err(SnapshotError::BadMagic);
        }
        let found = u16::from_le_bytes([bytes[4], bytes[5]]);
        if ![FORMAT_VERSION, PERSONNEL_FORMAT, PERCEPTION_FORMAT, PREVIOUS_FORMAT, LEGACY_CARGO_FORMAT, COMPATIBLE_MINING_FORMAT].contains(&found) {
            return Err(SnapshotError::FormatVersion { found });
        }
        let seam = u32::from_le_bytes(bytes[6..10].try_into().unwrap());
        let expected_seam = match found { FORMAT_VERSION => SEAM_REV, PERSONNEL_FORMAT => 3, PERCEPTION_FORMAT => 2, _ => 1 };
        if seam != expected_seam {
            return Err(SnapshotError::SeamRev { found: seam });
        }
        let saved = u64::from_le_bytes(bytes[10..18].try_into().unwrap());
        let given = content.as_ref().map_or(0, |c| c.hash());
        let historical_personnel_identity = found < PERSONNEL_FORMAT && content.as_ref()
            .is_some_and(|db| db.hash_without_starting_rules() == saved);
        if saved != given && !historical_personnel_identity {
            return Err(SnapshotError::ContentMismatch { saved, given });
        }
        let (b, rest): (Body, &[u8]) = postcard::take_from_bytes(&bytes[HEADER..])
            .map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
        if found < PREVIOUS_FORMAT && content.as_ref().is_some_and(|db| db.has_map_confinement()) {
            return Err(SnapshotError::Corrupt("old snapshot has no explicit confinement state".into()));
        }
        let (bounds, boundary_rows, perception_rows, personnel_rows, crew_rows) = if found >= PREVIOUS_FORMAT {
            let (bounds, rest) = postcard::take_from_bytes(rest).map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
            let (record, rest): (BoundaryRec, _) = postcard::take_from_bytes(rest).map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
            if record.version != 1 { return Err(SnapshotError::Corrupt("unknown confinement record version".into())); }
            let (perception_rows, rest) = if found >= PERCEPTION_FORMAT {
                let (record, rest): (PerceptionRec, _) = postcard::take_from_bytes(rest).map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
                if record.version != 1 { return Err(SnapshotError::Corrupt("unknown perceived-team record version".into())); }
                (record.rows, rest)
            } else {
                (Vec::new(), rest)
            };
            let (personnel_rows, rest) = if found >= PERSONNEL_FORMAT {
                let (record, rest): (PersonnelRec, _) = postcard::take_from_bytes(rest).map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
                if record.version != 1 { return Err(SnapshotError::Corrupt("unknown personnel record version".into())); }
                (record.rows, rest)
            } else { (Vec::new(), rest) };
            let (crew_rows, rest) = if found == FORMAT_VERSION {
                let (record, rest): (UnitCrewRec, _) = postcard::take_from_bytes(rest).map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
                if record.version != 1 { return Err(SnapshotError::Corrupt("unknown unit crew record version".into())); }
                (record.rows, rest)
            } else { (Vec::new(), rest) };
            if !rest.is_empty() { return Err(SnapshotError::Corrupt("trailing bytes".into())); }
            (bounds, record.rows, perception_rows, personnel_rows, crew_rows)
        } else if rest.is_empty() {
            (None, Vec::new(), Vec::new(), Vec::new(), Vec::new())
        } else {
            let (bounds, rest) = postcard::take_from_bytes(rest).map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
            if !rest.is_empty() {
                return Err(SnapshotError::Corrupt("trailing bytes".into()));
            }
            (bounds, Vec::new(), Vec::new(), Vec::new(), Vec::new())
        };
        let max_phase = if found == COMPATIBLE_MINING_FORMAT { 3 } else { 4 };
        if let Some(row) = b.economy.miners.iter().find(|row| row.5 > max_phase) {
            return Err(SnapshotError::Corrupt(format!(
                "mining phase {} is not supported by snapshot format {found}", row.5
            )));
        }

        let list = b
            .entities
            .into_iter()
            .map(EntityRec::into_entity)
            .collect::<Result<Vec<_>, _>>()?;
        let mut units =
            Entities::from_parts(list, b.next_entity).ok_or_else(|| corrupt("entity ids"))?;
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
        let perception = if found >= PERCEPTION_FORMAT {
            crate::perception::restore(&units, perception_rows)
                .map_err(|why| SnapshotError::Corrupt(why.into()))?
        } else {
            Table::default()
        };
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
        let weapons = Weapons::from_rec(b.weapons).ok_or_else(|| corrupt("weapons"))?;
        let specials = crate::specials::Specials::from_rec(b.specials).ok_or_else(|| corrupt("specials"))?;
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
        })
    }
}

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

    #[test]
    fn original_v8_mining_fixture_keeps_its_baseline_continuation() {
        let db = Arc::new(sta_content::fake::db());
        assert_eq!(u16::from_le_bytes(MINING_V8[4..6].try_into().unwrap()), 8);
        let mut live = Sim::from_snapshot(MINING_V8, Some(db)).unwrap();
        assert_eq!(live.tick, 100);
        assert_eq!(live.digest(), 0x14c7_0e04_4af1_fb70);
        assert_eq!(live.economy.to_rec().miners[0].3, Fixed::from_int(50).to_bits());
        for _ in 0..800 { live.step(&[]); }
        assert_eq!(live.tick, 900);
        assert_eq!(live.digest(), 0x1fad_adc8_8e06_0e5a);
        assert_eq!(live.view().player(PlayerId(0)).unwrap().resources[0], Fixed::from_int(150));
        assert_eq!(u16::from_le_bytes(live.snapshot()[4..6].try_into().unwrap()), FORMAT_VERSION);
    }

    fn stopped_loaded_miner() -> (Sim, Arc<sta_content::ContentDb>) {
        let db = Arc::new(sta_content::fake::db());
        let mut sim = Sim::from_snapshot(MINING_V8, Some(db.clone())).unwrap();
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
    fn new_or_unknown_mining_phases_cannot_masquerade_as_v8() {
        let (sim, db) = stopped_loaded_miner();
        let mut bytes = include_bytes!("../tests/fixtures/mining-idle-v9.stas").to_vec();
        bytes[4..6].copy_from_slice(&COMPATIBLE_MINING_FORMAT.to_le_bytes());
        assert!(matches!(Sim::from_snapshot(&bytes, Some(db.clone())), Err(SnapshotError::Corrupt(why)) if why.contains("mining phase 4")));

        let bytes = sim.snapshot();
        let (mut body, suffix): (Body, _) = postcard::take_from_bytes(&bytes[HEADER..]).unwrap();
        body.economy.miners[0].5 = 5;
        let mut unknown = bytes[..HEADER].to_vec();
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
