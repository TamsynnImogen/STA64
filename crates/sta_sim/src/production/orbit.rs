//! Orbital stations (SIM-62): a station of class `p_mining` (the orbital
//! mining and processing facilities) is built in a planet's orbit, not on open
//! ground. Stock Armada II's `PlanetMiningBaseClass`, which replaces the
//! general build-site rule for these stations:
//!
//! - The cursor picks a planet when its ground point is within the planet's
//!   bounding radius plus [`REACH`] of the planet's centre (X and Z).
//! - The planet must offer a slot (`canHaveOrbitalStation` of its resource
//!   node: planets 1, moons and rocks 0) and have no orbital station yet: one
//!   station per planet.
//! - The station stands on the planet's orbit, at the bearing of the cursor
//!   from the planet's centre: radius plus [`ALTITUDE`] from the centre, at the
//!   planet's height. Its yaw follows that bearing, so its heading is the
//!   outward radial.
//! - Nothing else is checked: not the map edge, not other objects, not
//!   ownership or exploration.
//!
//! Evidence: read from the executable, static (`CanPlaceHere`,
//! `GetRealBuildPosition`, `mClickedOnAPlanet`, `GetOrbitalPosition`); the
//! ODF keys are read from data in A2 Stock and STA2 Classic (Fleet Operations
//! and Roots have no orbital station ODFs). Not measured on the bench: which
//! side of the station faces the planet. The orbiting itself (`orbitTime`
//! seconds per orbit) is not implemented: a station keeps the bearing it was
//! built at.
//!
//! No state is kept: planets and occupied slots are read off the entities
//! (a station within [`REACH`] of a planet's radius belongs to that planet), so
//! snapshots and replays are unchanged. The site is computed by [`snap`], which
//! the sim applies to a rig's order and the game's placement ghost calls with
//! the same inputs ([`snap_view`]).

use sta_content::{ContentDb, UnitDef};
use sta_core::{ContentKey, EntityId, Fixed, Vec3};

use crate::world::Entities;

/// A cursor within this many units of a planet's surface picks it.
pub const REACH: i32 = 85;
/// The orbit's height above the planet's bounding sphere, units.
pub const ALTITUDE: i32 = 75;

/// A planet that can take an orbital station.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Planet {
    pub id: EntityId,
    pub pos: Vec3,
    /// Bounding radius, units.
    pub radius: Fixed,
}

/// Where an orbital station goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub planet: EntityId,
    pub at: Vec3,
    /// Unit vector in XZ, `(x, z)`: outward from the planet.
    pub heading: (Fixed, Fixed),
}

/// Why no slot is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrbitError {
    /// No planet with a slot under the cursor.
    NoPlanet,
    /// The planet already has its orbital station.
    Taken,
}

/// Whether `def` is a station that can only be built in a planet's orbit.
pub fn is_orbital(def: &UnitDef) -> bool {
    def.orbit_time.is_some()
}

fn hosts(def: &UnitDef) -> bool {
    def.can_have_orbital_station
}

/// The planet `at` picks: within radius + [`REACH`] of its centre, the one
/// whose surface is nearest (lowest id on a tie).
pub fn pick(planets: &[Planet], at: Vec3) -> Option<Planet> {
    let reach = Fixed::from_int(REACH);
    planets
        .iter()
        .filter_map(|p| {
            let (dx, dz) = (p.pos.x - at.x, p.pos.z - at.z);
            let d = (dx * dx + dz * dz).sqrt();
            (d < p.radius + reach).then_some((d - p.radius, p.id, *p))
        })
        .min_by_key(|(gap, id, _)| (*gap, *id))
        .map(|(_, _, p)| p)
}

/// The orbit slot for a cursor point `at`. `orbiters` are the positions of
/// the orbital stations that exist or are about to (deploying rigs); a planet
/// one of them belongs to is taken.
pub fn snap(planets: &[Planet], orbiters: &[Vec3], at: Vec3) -> Result<Slot, OrbitError> {
    let p = pick(planets, at).ok_or(OrbitError::NoPlanet)?;
    if orbiters.iter().any(|o| pick(planets, *o).is_some_and(|q| q.id == p.id)) {
        return Err(OrbitError::Taken);
    }
    Ok(slot(&p, at))
}

/// The slot on `p`'s orbit at the bearing of `at`. A cursor on the centre
/// takes bearing 0 (+z), as the original's `atan2(0, 0)` does.
pub fn slot(p: &Planet, at: Vec3) -> Slot {
    let (dx, dz) = (at.x - p.pos.x, at.z - p.pos.z);
    let d = (dx * dx + dz * dz).sqrt();
    let heading = if d == Fixed::ZERO { (Fixed::ZERO, Fixed::ONE) } else { (dx / d, dz / d) };
    let r = p.radius + Fixed::from_int(ALTITUDE);
    Slot { planet: p.id, at: Vec3 { x: p.pos.x + heading.0 * r, y: p.pos.y, z: p.pos.z + heading.1 * r }, heading }
}

/// Planets with a slot, and orbital stations, among the sim's entities.
pub(crate) fn from_entities(db: &ContentDb, units: &Entities) -> (Vec<Planet>, Vec<Vec3>) {
    let (mut planets, mut orbiters) = (Vec::new(), Vec::new());
    for e in units.iter() {
        let Some(d) = db.unit(e.def) else { continue };
        if hosts(d) {
            planets.push(Planet { id: e.id, pos: e.pos, radius: d.radius });
        }
        if is_orbital(d) {
            orbiters.push(e.pos);
        }
    }
    (planets, orbiters)
}

/// The same from a view, for callers outside the sim.
pub fn from_view(db: &ContentDb, view: &crate::WorldView) -> (Vec<Planet>, Vec<Vec3>) {
    let (mut planets, mut orbiters) = (Vec::new(), Vec::new());
    for u in &view.units {
        let Some(d) = db.unit(u.def) else { continue };
        if hosts(d) {
            planets.push(Planet { id: u.id, pos: u.pos, radius: d.radius });
        }
        if is_orbital(d) {
            orbiters.push(u.pos);
        }
    }
    (planets, orbiters)
}

/// The placement ghost's entry: `None` when `what` is not an orbital station
/// (the general [`site`](super::site) rule applies), else the slot the cursor
/// point `at` snaps to or why there is none. Rigs' pending deploys are
/// not in a view, so a slot another rig has claimed is only refused by the sim.
pub fn snap_view(
    db: &ContentDb,
    view: &crate::WorldView,
    what: ContentKey,
    at: Vec3,
) -> Option<Result<Slot, OrbitError>> {
    if !db.unit(what).is_some_and(is_orbital) {
        return None;
    }
    let (planets, orbiters) = from_view(db, view);
    Some(snap(&planets, &orbiters, at))
}
