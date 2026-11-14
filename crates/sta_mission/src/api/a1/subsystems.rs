//! `DestroyCritical` and the five `Destroy<System>` calls (LUA-30).
//!
//! Rule (read from the Armada I executable, static; the effect of the hit on a
//! unit is not measured): each call, for a live object, applies one hit of
//! 100000 damage typed by the system (engines, life support, weapons, shield
//! generator, sensors; `DestroyCritical` the hull type). A handle that names
//! nothing live, or a dying object, does nothing (no error). It is the same hit
//! as Armada II's `DestroyCritical`, so it is queued the same way: a lethal
//! SCRIPT hull `Damage` for the critical hit, and `SubsystemDamage` (clamped
//! by the sim to the system's hit points, so the system is destroyed) for
//! the others. The sim rejects a system the unit has no hit points for; the edit
//! feedback reports and counts that.

use mlua::{Lua, Table, Value};
use sta_core::Fixed;
use sta_sim::command::{Edit, Subsystem};

use super::{Env, def};
use crate::api::edit_feedback;

/// The hit's amount (the constant the executable builds).
const HIT: f64 = 100_000.0;

pub(super) fn install(lua: &Lua, env: &Env, table: &Table) -> mlua::Result<()> {
    let calls: [(&'static str, &'static str, Option<Subsystem>); 6] = [
        ("DestroyCritical", "A1.DestroyCritical", None),
        ("DestroyEngines", "A1.DestroyEngines", Some(Subsystem::Engines)),
        ("DestroyLifeSupport", "A1.DestroyLifeSupport", Some(Subsystem::LifeSupport)),
        ("DestroyWeapons", "A1.DestroyWeapons", Some(Subsystem::Weapons)),
        ("DestroyShieldGenerator", "A1.DestroyShieldGenerator", Some(Subsystem::Shields)),
        ("DestroySensors", "A1.DestroySensors", Some(Subsystem::Sensors)),
    ];
    for (name, label, system) in calls {
        table.set(
            name,
            def(lua, env, name, &["h"], move |_, e, _, a| {
                let id = a[0].id();
                if e.ctx.borrow().alive(id).is_none() {
                    return Ok(Value::Nil);
                }
                let amount = Fixed::from_f64(HIT);
                let edit = match system {
                    None => Edit::Damage(id, amount),
                    Some(sub) => Edit::SubsystemDamage(id, sub, amount),
                };
                edit_feedback::queue(&mut e.ctx.borrow_mut(), label, edit)?;
                Ok(Value::Nil)
            })?,
        )?;
    }
    Ok(())
}
