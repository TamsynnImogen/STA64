//! The flight of a torpedo under `ordnance_flight` (issue #1068, stock Armada II).
//!
//! A shot of a torpedo class ([`sta_content::weapon::TORPEDO_CLASSES`]) whose ordnance names a
//! `lifeSpan` is flown as the original flies it, instead of homing on its target's current
//! position every tick:
//!
//! - It has a life, `lifeSpan`, counted from the tick it was fired. On the tick the life has
//!   run out it vanishes with no effect and no hit event (measured, repeated; the exact tick
//!   of the boundary is INTERIM).
//! - While its `seekTime` runs it flies straight ahead along its heading at `shotSpeed` and
//!   turns toward its target at most `omegaTurn` radians per second (read from the
//!   executable, static; the capped turn is measured once per geometry, as a flight time).
//!   Afterwards it points at its target every tick, with no turn limit (read from the
//!   executable, static).
//! - It steers only while its target is within [`SEEK_RANGE_FACTOR`] times the weapon's range;
//!   beyond that it flies straight on (read from the executable, static).
//! - It hits when the remaining distance is under one step (read from the executable,
//!   static). A target that cloaks is kept and hit (measured, once, with a control); one that
//!   starts to die is still followed and hit (read from the executable, static). A target that
//!   is gone, or phased, is lost: the shot flies straight on until its life ends and hits
//!   nothing (read from the executable, static).
//!
//! INTERIM, not established: the launch heading (the shooter's heading at the moment it fires;
//! the measured flight times leave a gap to this reading), the aim point (the target's centre,
//! not a hardpoint), flight in the ground plane only, and the shield's shape (a hit lands at
//! the centre). Pulse weapons (their lead aim) and the plasma cannon and chain pulsar (a flight
//! of their own) are not covered; a torpedo whose ordnance has no `lifeSpan` flies as without
//! the switch.
//!
//! The heading and the end tick are the shot's own state ([`Flight`]); the seek phase's end and
//! the turn rate come from the weapon's data keys and are not stored.

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, Features, WeaponDef, WeaponKind};
use sta_core::{EntityId, Fixed, TICK_HZ, Vec3};

use super::weapons::{BEAM_BREAK_RANGE_FACTOR, Shot, flat};
use crate::world::{Entities, Entity};

/// A torpedo steers only while its target is within this many times the weapon's range: the
/// constant that cuts a beam (read from the executable, static; the same constant).
pub const SEEK_RANGE_FACTOR: i32 = BEAM_BREAK_RANGE_FACTOR;

/// `seekTime` of an ordnance that does not set it, seconds: no launch phase (read from the
/// executable, static).
pub const DEFAULT_SEEK_TIME: Fixed = Fixed::ZERO;

/// `omegaTurn` of an ordnance that does not set it, radians per second (read from the
/// executable, static).
pub const DEFAULT_OMEGA_TURN: Fixed = Fixed::ONE;

/// What a flown shot carries beyond a straight one: its heading and the tick its life ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Flight {
    /// A unit vector in the ground plane.
    pub(super) heading: (Fixed, Fixed),
    /// The tick the shot vanishes: the fire tick plus its life in ticks.
    pub(super) ends: u64,
}

/// What a shot's tick came to.
pub(super) enum Fate {
    Flying,
    Hit,
    Gone,
}

/// The shot's life in ticks under the switch: `None` when `w` does not fly this way (the switch
/// is off, `w` is not a torpedo, or its ordnance has no `lifeSpan`).
pub(super) fn life_ticks(features: &Features, w: &WeaponDef) -> Option<u64> {
    if !features.ordnance_flight || w.kind != WeaponKind::Projectile || !sta_content::weapon::TORPEDO_CLASSES.contains(&w.class.0.as_str()) {
        return None;
    }
    let life = w.base.life_span.filter(|l| *l > Fixed::ZERO)?;
    Some(ticks(life).max(1))
}

/// Seconds as ticks, rounded to the nearest, as the data's other times are.
fn ticks(seconds: Fixed) -> u64 {
    let t = seconds * Fixed::from_int(TICK_HZ as i32) + Fixed::from_bits(1 << 31);
    (t.to_bits() >> 32).max(0) as u64
}

/// The flight of a shot of `w` fired at `tick` by a unit heading `heading`, when it flies this way.
pub(super) fn launch(features: &Features, w: &WeaponDef, heading: (Fixed, Fixed), tick: u64) -> Option<Flight> {
    life_ticks(features, w).map(|life| Flight { heading, ends: tick.saturating_add(life) })
}

/// The flight a shot loaded from a snapshot that had none gets: its heading toward its target
/// (the shooter's heading at the shot's launch is not known), and a life counted from `tick`.
pub(super) fn adopt(features: &Features, w: &WeaponDef, from: Vec3, target: Option<&Entity>, tick: u64) -> Option<Flight> {
    let toward = target.map(|t| flat(from, t.pos).0).filter(|d| *d != (Fixed::ZERO, Fixed::ZERO));
    launch(features, w, toward.unwrap_or((Fixed::ZERO, Fixed::ONE)), tick)
}

/// One tick of a flown shot at tick `tick`: it turns, moves, and hits or is gone. `target` is the
/// shot's target when it is a valid one (alive, not phased).
pub(super) fn fly(db: &ContentDb, tick: u64, s: &mut Shot, target: Option<&Entity>) -> Fate {
    let Some(mut f) = s.flight else { return Fate::Gone };
    let Some(w) = db.weapon(s.weapon) else { return Fate::Gone };
    let Some(life) = life_ticks(db.weapon_features(w), w) else { return Fate::Gone };
    if tick >= f.ends {
        return Fate::Gone;
    }
    let (dir, d) = target.map_or(((Fixed::ZERO, Fixed::ZERO), Fixed::ZERO), |t| flat(s.pos, t.pos));
    if target.is_some() && d <= s.step {
        return Fate::Hit;
    }
    let age = tick.saturating_sub(f.ends - life);
    let seek_ticks = ticks(w.base.seek_time.unwrap_or(DEFAULT_SEEK_TIME));
    let in_reach = d <= w.range * Fixed::from_int(SEEK_RANGE_FACTOR);
    if target.is_some() && in_reach {
        f.heading = if age <= seek_ticks {
            let omega = w.base.omega_turn.unwrap_or(DEFAULT_OMEGA_TURN);
            turn(f.heading, dir, omega / Fixed::from_int(TICK_HZ as i32))
        } else {
            dir
        };
    }
    s.pos.x = s.pos.x + f.heading.0 * s.step;
    s.pos.z = s.pos.z + f.heading.1 * s.step;
    s.flight = Some(f);
    Fate::Flying
}

/// `heading` turned toward `dir` by at most `max` radians.
fn turn(heading: (Fixed, Fixed), dir: (Fixed, Fixed), max: Fixed) -> (Fixed, Fixed) {
    let cross = heading.0 * dir.1 - heading.1 * dir.0;
    let dot = heading.0 * dir.0 + heading.1 * dir.1;
    let angle = crate::kinematics::atan2(cross, dot);
    if angle.abs() <= max {
        dir
    } else {
        crate::kinematics::rotate_by(heading, if angle < Fixed::ZERO { -max } else { max })
    }
}

/// The target of a shot as the flight sees it: alive (a unit in its dying phase still counts)
/// and not phased.
pub(super) fn valid<'a>(units: &'a Entities, specials: &crate::specials::Specials, db: &ContentDb, id: EntityId) -> Option<&'a Entity> {
    units.get(id).filter(|u| u.hull[1] > Fixed::ZERO && !specials.phased(db, u))
}
