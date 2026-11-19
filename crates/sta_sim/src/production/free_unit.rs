//! The free unit of a mining station: when a construction rig finishes a
//! station whose def names `freighterName`, that unit appears for the
//! station's owner at the station's dock node.
//!
//! Rule (read from the executables, static; on screen not measured):
//! - Only the completion of a station welded by a rig. A station placed by a
//!   map, spawned or replaced by a mission script, or taken over gets none:
//!   those paths never reach [`spawn`] (it is called from the weld
//!   completion only).
//! - The unit is created for the station's owner at the pose of the dock
//!   node (`dockingHardpoint`, default `dock`); it costs no resources, no
//!   build time and no officers, and its starting crew (`crewCost`) is taken
//!   from the station's crew. Interim: the original does not clamp the
//!   station's crew at zero when it is lower than that cost; here it stops
//!   at zero.
//! - First act: the original queues it at the station's dock. STA64 has no
//!   dock queue state, so it stands at the dock node; whether it then mines
//!   by itself is not measured and not done.
//! - A name that is not a unit, or an absent key, spawns nothing.
//! - A def without a readable dock node (STA64's own fallback, interim):
//!   [`sta_content::launch::DOCK_FALLBACK_AHEAD`] units in front of the station.

use sta_content::{ContentDb, Fixed};
use sta_core::EntityId;

use crate::event::SimEvent;
use crate::world::{Entities, Entity};

/// Spawn the free unit of the just-finished station `station`, if its def has one.
pub(super) fn spawn(
    units: &mut Entities,
    db: &ContentDb,
    station: EntityId,
    confinement: &mut crate::boundary::state::Confinement,
    events: &mut Vec<SimEvent>,
) {
    let Some(s) = units.get(station) else { return };
    let (owner, pos, heading, def) = (s.owner, s.pos, s.heading, s.def);
    let Some(sdef) = db.unit(def) else { return };
    let Some(name) = sdef.free_unit.as_deref() else { return };
    let Some(free) = db.key(name).and_then(|k| db.unit(k)) else { return };
    let (at, face) = match sdef.dock.as_ref().filter(|d| d.node.is_some()) {
        Some(d) => {
            let (ox, oz) = super::launch::turn(heading, d.offset);
            let dir = super::launch::turn(heading, d.dir);
            (sta_content::Vec3 { x: pos.x + ox, y: pos.y, z: pos.z + oz }, dir)
        }
        None => {
            let a = Fixed::from_int(sta_content::launch::DOCK_FALLBACK_AHEAD);
            (sta_content::Vec3 { x: pos.x + heading.0 * a, y: pos.y, z: pos.z + heading.1 * a }, heading)
        }
    };
    let unit = Entity::from_def(free, db.features(), owner, at, face);
    let cost = unit.crew.map_or(Fixed::ZERO, |c| c[0]);
    let id = units.spawn(unit);
    confinement.initialize(units.get_mut(id).expect("spawned"));
    if let Some(c) = units.get_mut(station).and_then(|s| s.crew.as_mut()) {
        c[0] = (c[0] - cost).max(Fixed::ZERO);
    }
    events.push(SimEvent::Spawned { id });
}
