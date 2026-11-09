//! Damage (SIM-05): applies the `Hit`s the weapons emitted this tick.
//!
//! - Shields take a hit first: the facing it struck with directional shields,
//!   the single pool otherwise. What the shield cannot absorb goes to the hull.
//! - [`apply`] rewrites each `Hit` in place so `shield` and `hull` are the
//!   damage each really took (weapons emit the raw damage as `hull`), and
//!   follows it with `ShieldsDown { facing }` when that hit emptied a pool.
//! - A hull at zero destroys the entity at the end of the tick's hits. The
//!   killer is the `by` of the first lethal `Hit`; kills are returned in that
//!   order, for `Sim::despawn` (`Destroyed`, removal). Later hits that tick on
//!   a unit already dead do no damage.
//! - [`recharge`] gives every shield pool its definition's `recharge` per tick
//!   (capped at `max`), before the tick's hits. No delay after a hit.
//!
//! Holds no state of its own: shields and hull live on the entity, so
//! snapshots and the digest already cover it.
//!
//! Not yet: systems damage (weapons or engines offline below a hull
//! threshold): `UnitDef` has no fields for it.

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed};

use crate::event::SimEvent;
use crate::view::UnitFlags;
use crate::world::Entities;

/// Recharge every unit's shields by its definition's rate.
pub(crate) fn recharge(units: &mut Entities, db: &ContentDb) {
    for u in units.iter_mut() {
        if u.hull[0] <= Fixed::ZERO {
            continue;
        }
        let Some(def) = db.unit(u.def) else { continue };
        let (rate, max) = (def.shields.recharge, u.shields.max);
        if rate <= Fixed::ZERO {
            continue;
        }
        let pools = if u.shields.directional { 4 } else { 1 };
        for now in &mut u.shields.now[..pools] {
            if *now < max {
                *now = (*now + rate).min(max);
            }
        }
    }
}

/// Apply the `Hit`s in `events[from..]`; returns `(killed, killer)` in the
/// order the lethal hits came.
pub(crate) fn apply(
    units: &mut Entities,
    events: &mut Vec<SimEvent>,
    from: usize,
) -> Vec<(EntityId, Option<EntityId>)> {
    let mut killed = Vec::new();
    let tail = events.split_off(from);
    for e in tail {
        let SimEvent::Hit {
            target,
            by,
            weapon,
            facing,
            hull: damage,
            at,
            ..
        } = e
        else {
            events.push(e);
            continue;
        };
        let (mut shield, mut hull, mut down) = (Fixed::ZERO, Fixed::ZERO, false);
        if let Some(t) = units.get_mut(target).filter(|t| t.hull[0] > Fixed::ZERO) {
            let pool = if t.shields.directional {
                usize::from(facing).min(3)
            } else {
                0
            };
            let held = t.shields.now[pool];
            shield = damage.min(held);
            t.shields.now[pool] = held - shield;
            down = shield > Fixed::ZERO && t.shields.now[pool] == Fixed::ZERO;
            let floor = if t.flags.contains(UnitFlags::IMMORTAL) { Fixed::ONE } else { Fixed::ZERO };
            hull = (damage - shield).min((t.hull[0] - floor).max(Fixed::ZERO));
            t.hull[0] = t.hull[0] - hull;
            if t.hull[0] <= Fixed::ZERO {
                killed.push((target, Some(by)));
            }
        }
        events.push(SimEvent::Hit {
            target,
            by,
            weapon,
            facing,
            shield,
            hull,
            at,
        });
        if down {
            events.push(SimEvent::ShieldsDown { id: target, facing });
        }
    }
    killed
}
