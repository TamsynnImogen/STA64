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
//! Orbiting (SIM-65, `PlanetMiningBase::Simulate`): from the tick it is
//! finished, a station's bearing from its planet's centre advances by `2 pi /
//! orbitTime` radians per second, in the sense that turns the bearing's
//! `(sin a, cos a)` from +Z toward +X, on the circle of radius + [`ALTITUDE`]
//! at the planet's height; its heading is the outward radial. Read from the
//! executable, static; only a station on its planet's circle (within 5 units,
//! as [`owner`] reads it) orbits: one a map or a script put elsewhere, or moved
//! off the circle, stays where it is (the original pulls any station of a planet
//! onto the circle; STA64 keeps SIM-62's rule, a choice, not measured). The constant `2 pi` (one orbit per `orbitTime`, as the
//! ODF comment says) is inferred, not read. The planet's own turning is not
//! used by the original's step. The position is the saved entity position:
//! the next one is a function of it, the planet and the content, so the step
//! keeps no state of its own.
//!
//! Evidence: read from the executable, static (`CanPlaceHere`,
//! `GetRealBuildPosition`, `mClickedOnAPlanet`, `GetOrbitalPosition`); the
//! ODF keys are read from data in A2 Stock and STA2 Classic (Fleet Operations
//! and Roots have no orbital station ODFs). Not measured on the bench: which
//! side of the station faces the planet. The orbiting itself (`orbitTime`
//! seconds per orbit) is [`step`] (SIM-65), below.
//!
//! No state is kept: planets and occupied slots are read off the entities
//! (a station within [`REACH`] of a planet's radius belongs to that planet), so
//! snapshots and replays are unchanged. The site is computed by [`snap`], which
//! the sim applies to a rig's order and the game's placement ghost calls with
//! the same inputs ([`snap_view`]).

use sta_content::{ContentDb, UnitDef};
use sta_core::{ContentKey, EntityId, Fixed, Vec3, TICK_HZ};

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
            // Far planets are out before squaring (the fixed-point square wraps past ~46000).
            if dx.abs() >= p.radius + reach || dz.abs() >= p.radius + reach {
                return None;
            }
            let d = (dx * dx + dz * dz).sqrt();
            (d < p.radius + reach).then_some((d - p.radius, p.id, *p))
        })
        .min_by_key(|(gap, id, _)| (*gap, *id))
        .map(|(_, _, p)| p)
}

/// The planet whose orbit a station at `pos` lies on (within a few units of
/// its circle), and none when it is anywhere else.
pub fn on_ring(planets: &[Planet], pos: Vec3) -> Option<Planet> {
    let tol = Fixed::from_int(5);
    let alt = Fixed::from_int(ALTITUDE);
    planets
        .iter()
        .filter_map(|p| {
            let (dx, dz) = (p.pos.x - pos.x, p.pos.z - pos.z);
            if dx.abs() > p.radius + alt + tol || dz.abs() > p.radius + alt + tol {
                return None;
            }
            let off = ((dx * dx + dz * dz).sqrt() - (p.radius + alt)).abs();
            (off <= tol).then_some((off, p.id, *p))
        })
        .min_by_key(|(off, id, _)| (*off, *id))
        .map(|(_, _, p)| p)
}

/// The planet an orbital station at `pos` belongs to: the one whose orbit it
/// is on ([`on_ring`]), else the one [`pick`] gives. Close planets
/// do not steal each other's stations.
pub fn owner(planets: &[Planet], pos: Vec3) -> Option<Planet> {
    on_ring(planets, pos).or_else(|| pick(planets, pos))
}

/// The orbit slot for a cursor point `at`. `orbiters` are the positions of
/// the orbital stations that exist or are about to (deploying rigs); a planet
/// one of them belongs to is taken.
pub fn snap(planets: &[Planet], orbiters: &[Vec3], at: Vec3) -> Result<Slot, OrbitError> {
    let p = pick(planets, at).ok_or(OrbitError::NoPlanet)?;
    if orbiters.iter().any(|o| owner(planets, *o).is_some_and(|q| q.id == p.id)) {
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

/// `2 pi` as a [`Fixed`] (`round(2 pi * 2^32)`): one orbit is this many radians.
const TAU: Fixed = Fixed::from_bits(26_986_075_409);

/// The bearing's advance per tick for an orbit of `orbit_time` seconds,
/// less than one turn whatever the time (a tiny modded time spins the station,
/// it does not cost more to step); zero for a time that is not positive (no
/// orbiting) and for one so long the step is below the smallest [`Fixed`].
pub fn step_angle(orbit_time: Fixed) -> Fixed {
    let den = i128::from(orbit_time.to_bits()) * i128::from(TICK_HZ);
    if den <= 0 {
        return Fixed::ZERO;
    }
    let turn = i128::from(TAU.to_bits());
    let bits = ((turn << 32) / den) % turn;
    Fixed::from_bits(bits as i64)
}

/// One tick of every finished orbital station's revolution about its planet.
/// A station still being built stays at its site; one that is not on a
/// planet's orbit (placed or moved elsewhere by a script or a map) stays put, as does one whose
/// `orbitTime` is not positive.
pub(crate) fn step(db: &ContentDb, units: &mut Entities) {
    let (planets, orbiters) = from_entities(db, units);
    if planets.is_empty() || orbiters.is_empty() {
        return;
    }
    let moves: Vec<(EntityId, Vec3, (Fixed, Fixed))> = units
        .iter()
        .filter(|e| e.progress.is_none())
        .filter_map(|e| {
            let d = db.unit(e.def).filter(|d| is_orbital(d))?;
            let da = step_angle(d.orbit_time?);
            if da == Fixed::ZERO {
                return None;
            }
            let p = on_ring(&planets, e.pos)?;
            let (dx, dz) = (e.pos.x - p.pos.x, e.pos.z - p.pos.z);
            let len = (dx * dx + dz * dz).sqrt();
            let h = if len == Fixed::ZERO { (Fixed::ZERO, Fixed::ONE) } else { (dx / len, dz / len) };
            // `rotate_by` turns +X toward +Z; the original's bearing runs the other way.
            let h = crate::kinematics::rotate_by(h, -da);
            let r = p.radius + Fixed::from_int(ALTITUDE);
            Some((e.id, Vec3 { x: p.pos.x + h.0 * r, y: p.pos.y, z: p.pos.z + h.1 * r }, h))
        })
        .collect();
    for (id, pos, heading) in moves {
        if let Some(e) = units.get_mut(id) {
            e.pos = pos;
            e.heading = heading;
        }
    }
}
