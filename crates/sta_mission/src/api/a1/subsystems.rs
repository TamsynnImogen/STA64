//! `DestroyCritical` and the five `Destroy<System>` calls (LUA-30).
//!
//! Rule (read from the Armada I executable, static; the effect of the hit on a
//! unit in play is not measured): each call, for a live Craft, applies one hit of
//! 100000 damage typed by the system (engines, life support, weapons, shield
//! generator, sensors; `DestroyCritical` the hull type). A handle that names
//! nothing live, a dying object or a non-Craft does nothing (no error; whether
//! the original's Craft flag also covers stations is not read). It is the same
//! hit as Armada II's `DestroyCritical`: a lethal SCRIPT hull `Damage`.
//! For a system, the sim's `SubsystemDamage` only fills the damage points
//! (clamped to the system's hit points) and does not switch the system off, so
//! the call also queues the switch-off a destroyed system gets in combat
//! (`Edit::Subsystem(.., false)`). A system the unit has no hit points for is
//! rejected by the sim and counted by the edit feedback.
//! Not measured: whether the original's damage routine also passes damage beyond
//! a system's hit points on to the hull (which would make the five calls lethal).
//! Repeated `DestroyCritical` calls on one unit in one tick queue one hit: the
//! view is a tick old, so a second lethal hit would be rejected and counted.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use mlua::{Lua, Table, Value};
use sta_core::{EntityId, Fixed};
use sta_sim::command::{Edit, Subsystem};
use sta_sim::world::EntityKind;

use super::{Env, def};
use crate::api::edit_feedback;

/// The hit's amount.
const HIT: f64 = 100_000.0;

/// The ids that already have a lethal hit queued, and the tick they were queued in.
type Lethal = Rc<RefCell<(u64, BTreeSet<EntityId>)>>;

pub(super) fn install(lua: &Lua, env: &Env, table: &Table) -> mlua::Result<()> {
    let calls: [(&'static str, &'static str, Option<Subsystem>); 6] = [
        ("DestroyCritical", "A1.DestroyCritical", None),
        ("DestroyEngines", "A1.DestroyEngines", Some(Subsystem::Engines)),
        ("DestroyLifeSupport", "A1.DestroyLifeSupport", Some(Subsystem::LifeSupport)),
        ("DestroyWeapons", "A1.DestroyWeapons", Some(Subsystem::Weapons)),
        ("DestroyShieldGenerator", "A1.DestroyShieldGenerator", Some(Subsystem::Shields)),
        ("DestroySensors", "A1.DestroySensors", Some(Subsystem::Sensors)),
    ];
    let lethal: Lethal = Rc::default();
    for (name, label, system) in calls {
        let lethal = lethal.clone();
        // "H": a pending or failed spawn is "no object", so the call does nothing.
        table.set(
            name,
            def(lua, env, name, &["H"], move |_, e, _, a| {
                let id = a[0].id();
                let mut c = e.ctx.borrow_mut();
                let is_craft = c
                    .alive(id)
                    .cloned()
                    .and_then(|u| c.def(&u).map(EntityKind::of_def))
                    .is_some_and(|k| k == EntityKind::Craft);
                if !is_craft {
                    return Ok(Value::Nil);
                }
                let amount = Fixed::from_f64(HIT);
                match system {
                    None => {
                        let tick = c.view.tick;
                        let mut seen = lethal.borrow_mut();
                        if seen.0 != tick {
                            *seen = (tick, BTreeSet::new());
                        }
                        if seen.1.insert(id) {
                            edit_feedback::queue(&mut c, label, Edit::Damage(id, amount))?;
                        }
                    }
                    Some(sub) => {
                        edit_feedback::queue(&mut c, label, Edit::SubsystemDamage(id, sub, amount))?;
                        edit_feedback::queue(&mut c, label, Edit::Subsystem(id, sub, false))?;
                    }
                }
                Ok(Value::Nil)
            })?,
        )?;
    }
    Ok(())
}
