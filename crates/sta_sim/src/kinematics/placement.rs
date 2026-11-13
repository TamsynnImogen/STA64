//! Mission placement and facing (SIM-57): `SetPosition`, `PointAt`,
//! `SetAngularVelocity` and the Roots `position`, `forward`, `omega` setters.
//!
//! Rules (stock Armada II 1.1; evidence grades in brackets, measurement detail
//! kept outside the repository):
//!
//! - Position: the point is taken as given, y included, with no ground snap
//!   [measured, read from the executable, static]. The unit's speed is zeroed
//!   [measured]; its heading, turn rate, target and queued waypoints are kept
//!   [measured: a moving ship teleported rejoined its route and reached its old
//!   destination]. When the unit's ForceOntoMap flag is set the map-confinement
//!   update runs on the new point as on every position change [measured: a
//!   ship sent past the edge landed one edge buffer inside]; a unit without
//!   the flag is not clamped [measured]. Any live unit may be moved; a unit
//!   under construction is rejected (STA64 policy, not measured).
//! - Heading: instant [measured]. Only the yaw is modelled: the original also
//!   pitches toward a higher target for a quarter of a second [measured]. The
//!   unit's orders are kept and it steers back onto its course [measured]; its
//!   speed is kept (the original re-reads its velocity along the new facing:
//!   not modelled).
//! - Turn rate: a craft with its engines on has the value overwritten by its
//!   steering within a tick, so it has no lasting effect [measured, read from
//!   the executable, static]; a craft with its engines off keeps the rate
//!   without decay [measured: 21 s constant]. Pitch and roll components are not
//!   modelled. Not a craft: accepted, nothing stored (the original stores it and
//!   integrates nothing) [read from the executable, static, not exhaustive].

use sta_core::{EntityId, Fixed, Vec3};

use super::{dt, rotate_by};
use crate::Sim;
use crate::command::MAX_SCRIPT_OMEGA;
use crate::view::UnitFlags;
use crate::world::EntityKind;

impl Sim {
    pub(crate) fn edit_position(&mut self, id: EntityId, to: Vec3) -> bool {
        let Some(u) = self.units.get_mut(id).filter(|u| !u.flags.contains(UnitFlags::DYING) && u.progress.is_none()) else {
            return false;
        };
        u.pos = to;
        u.speed = Fixed::ZERO;
        self.confinement.position_updated(u);
        true
    }

    pub(crate) fn edit_heading(&mut self, id: EntityId, dir: (Fixed, Fixed)) -> bool {
        // Scale by the larger component first so the squares cannot overflow.
        let m = dir.0.abs().max(dir.1.abs());
        if m == Fixed::ZERO {
            return false;
        }
        let dir = (dir.0 / m, dir.1 / m);
        let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt();
        let Some(u) = self.units.get_mut(id).filter(|u| !u.flags.contains(UnitFlags::DYING)) else {
            return false;
        };
        u.heading = (dir.0 / len, dir.1 / len);
        true
    }

    pub(crate) fn edit_angular_velocity(&mut self, id: EntityId, omega: Fixed) -> bool {
        let Some(u) = self.units.get_mut(id).filter(|u| !u.flags.contains(UnitFlags::DYING)) else {
            return false;
        };
        if u.kind == EntityKind::Craft && u.flags.contains(UnitFlags::ENGINES_OFF) {
            let cap = Fixed::from_int(MAX_SCRIPT_OMEGA);
            u.omega = omega.max(-cap).min(cap);
        }
        true
    }

    /// Engines-off craft keep turning at their rate, without decay.
    pub(crate) fn step_drifting_spin(&mut self) {
        let dt = dt();
        for u in self.units.iter_mut() {
            if u.kind == EntityKind::Craft && u.flags.contains(UnitFlags::ENGINES_OFF) && u.omega != Fixed::ZERO {
                u.heading = rotate_by(u.heading, u.omega * dt);
            }
        }
    }
}
