//! The guard order (LUA-39, `Order::Guard`, tag 19): a craft keeps station beside a
//! ward, fights what attacks the ward and stops when the ward is gone.
//!
//! Rule (Armada II GUARD, command 23; read from the executable, static; the
//! stock script call is `SetCommandWhatWhoParamVO(unit, 23, ward, param, voice)`):
//! the ward is the command's target object; the call needs both to be live
//! craft and tests no owner and no special-forces flag; the parameter is not
//! read by the guard behaviour. Each tick the original takes the ward's last
//! attacker (the ward hit within a second, the attacker a live enemy it can
//! hit) as its fire target, else the best enemy in range like any idle or
//! moving craft, and flies to a point beside the ward, on the enemy's side
//! when it has a target; a ward that is no longer a live craft stops the
//! guard. Not modelled: formation keeping behind a fleet leader, going through
//! a wormhole after the ward, special weapons, the voice flag.
//!
//! STA64's reading (the numbers are authored, **not measured**): the order is
//! [`Job::Guard`]. The craft keeps a station at `spacing` from the ward on the
//! side it is on, `spacing` being twice the two radii (at least
//! [`MIN_SPACING`]): it flies there when it is farther than one spacing from
//! it and stops within half a spacing. An enemy craft that is attacking the
//! ward and stands within the leash of the ward (the guard's attack range plus
//! the spacing) is attacked through the ordinary attack order, so the
//! approach closes in; the attack ends, and the craft goes back to its station,
//! when that enemy dies, is not seen or leaves the leash. Anything else in
//! range is left to the fire at targets of opportunity every moving or idle
//! craft has. The order ends (the craft stops) when the ward is gone, dying or
//! no craft. A player may guard an own or allied craft; scripts and the bench
//! any craft. Self-guarding is refused.

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, Vec3};

use super::{db_radius, flat};
use crate::combat::targeting;
use crate::movement::approach;
use crate::relations::Diplomacy;
use crate::world::{Entity, EntityKind};
use crate::Sim;

/// The smallest station distance from the ward (authored, not measured).
pub const MIN_SPACING: i32 = 100;

impl Sim {
    /// The ward as a live craft (hull left, a craft).
    fn live_ward(&self, ward: EntityId) -> Option<&Entity> {
        self.units.get(ward).filter(|w| w.kind == EntityKind::Craft && w.hull[0] > Fixed::ZERO)
    }

    /// The crafts of `ids` (already the ones `player` may give a scripted order)
    /// that may guard `ward`: not the ward itself, and for a player's seat not
    /// an enemy's ward. Empty when the ward is not a live craft.
    pub(super) fn guard_takers(&self, player: PlayerId, ids: Vec<EntityId>, ward: EntityId) -> Vec<EntityId> {
        let Some(w) = self.live_ward(ward) else { return Vec::new() };
        let dip = Diplomacy::new(&self.players, &self.relations);
        ids.into_iter()
            .filter(|&id| id != ward)
            .filter(|&id| self.units.get(id).is_some_and(|u| u.kind == EntityKind::Craft && (!player.is_player() || !dip.is_enemy(u.owner, w.owner))))
            .collect()
    }

    /// One tick of one guard (see the module docs). `seen`: what the guard's
    /// owner sees, as the other scripted orders take it.
    pub(super) fn step_guard(&mut self, id: EntityId, ward: EntityId, db: &ContentDb, dip: Diplomacy<'_>, seen: Option<&[EntityId]>) {
        let Some(u) = self.units.get(id).cloned() else { return };
        let Some((wpos, wradius)) = self.live_ward(ward).map(|w| (w.pos, db_radius(self, w))) else {
            self.weapons.stop_attack(&self.units, PlayerId::SCRIPT, &[id]);
            self.set_target(id, None);
            self.movement.clear_waypoints(id);
            self.scripted.remove(id);
            return;
        };
        let (pos, has_target) = (u.pos, u.target);
        let Some(def) = db.unit(u.def) else { return };
        let spacing = (Fixed::from_int(2) * (db_radius(self, &u) + wradius)).max(Fixed::from_int(MIN_SPACING));
        let leash = approach::attack_range(db, def).unwrap_or(Fixed::ZERO) + spacing;

        // The ward's attacker: the nearest enemy craft within the leash that
        // has the ward as its attack target (nobody is picked by a seatless unit).
        let ok = |t: &Entity| targeting::candidate(db, &u, t, dip, &self.perception, seen, None);
        let attacker = if dip.is_seat(u.owner) {
            self.units
                .iter()
                .filter(|t| t.kind == EntityKind::Craft && self.weapons.attack_of(t.id) == Some(ward) && ok(t) && flat(wpos, t.pos) <= leash)
                .min_by_key(|t| (flat(wpos, t.pos), t.id))
                .map(|t| t.id)
        } else {
            None
        };
        let current = self.weapons.attack_of(id);
        if let Some(a) = attacker {
            if current != Some(a) {
                self.weapons.set_attack(id, a);
            }
            return;
        }
        if let Some(c) = current {
            // Keep a fight in progress while its target is still a fair one inside the leash.
            if self.units.get(c).is_some_and(|t| ok(t) && flat(wpos, t.pos) <= leash) {
                return;
            }
            self.weapons.stop_attack(&self.units, PlayerId::SCRIPT, &[id]);
        }

        // Station keeping: one spacing from the ward on the side the guard is on.
        let d = flat(pos, wpos);
        let (sx, sz) = if d > Fixed::ONE { ((pos.x - wpos.x) / d, (pos.z - wpos.z) / d) } else { (Fixed::ONE, Fixed::ZERO) };
        let station = Vec3 { x: wpos.x + sx * spacing, y: pos.y, z: wpos.z + sz * spacing };
        let off = flat(pos, station);
        let half = spacing / Fixed::from_int(2);
        if off > spacing {
            if has_target.is_none_or(|t| flat(t, station) > half) {
                self.set_target(id, Some(station));
            }
        } else if has_target.is_some() && off <= half {
            self.set_target(id, None);
        }
    }
}
