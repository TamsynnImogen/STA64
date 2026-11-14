//! `DestroyCritical` and the five `Destroy<System>` calls (LUA-30).
//!
//! Rule (read from data: a stock script destroys shipyards with `DestroyCritical` and
//! waits for them to die; the executable's object flag test is read, which objects
//! carry it is inferred from that use. The effect of the hit in play is not
//! measured): each call, for a live Craft or Station, applies one hit of
//! 100000 damage typed by the system (engines, life support, weapons, shield
//! generator, sensors; `DestroyCritical` the hull type). A handle that names
//! nothing live, a dying object or anything but a Craft or Station does nothing
//! (no error). It is the same
//! hit as Armada II's `DestroyCritical`: a lethal SCRIPT hull `Damage`.
//! For a system, the sim's `SubsystemDamage` only fills the damage points
//! (clamped to the system's hit points) and does not switch the system off, so
//! the call also queues the switch (`Edit::Subsystem(.., false)`) that combat sets
//! on a destroyed system. Both edits are queued only for a unit whose definition
//! has hit points above 0 for that system (such a system cannot be destroyed in
//! combat either); otherwise the call does nothing. Differences from combat: the
//! script switch for engines also zeroes speed, turn rate and target (the craft
//! stops dead; combat only sets the flag).
//! Not measured / not applied: whether the original's damage routine also passes
//! damage beyond a system's hit points on to the hull (which would make the five
//! calls lethal), and the crew loss of a destroyed system (not read).
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
        // "H": a failed spawn is "no object", so the call does nothing. A pending token never
        // reaches here: the hold wrapper (hold.rs) holds the call and replays it after the receipt.
        table.set(
            name,
            def(lua, env, name, &["H"], move |_, e, _, a| {
                let id = a[0].id();
                let mut c = e.ctx.borrow_mut();
                let Some(unit) = c.alive(id).cloned() else { return Ok(Value::Nil) };
                let Some(def) = c.def(&unit) else { return Ok(Value::Nil) };
                if !matches!(EntityKind::of_def(def), EntityKind::Craft | EntityKind::Station) {
                    return Ok(Value::Nil);
                }
                let has_hit_points = system.is_none_or(|sub| {
                    let n = match sub {
                        Subsystem::Engines => 0,
                        Subsystem::Weapons => 1,
                        Subsystem::Shields => 2,
                        Subsystem::LifeSupport => 3,
                        Subsystem::Sensors => 4,
                        _ => return false,
                    };
                    def.system_hit_points[n] > Fixed::ZERO
                });
                if !has_hit_points {
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
