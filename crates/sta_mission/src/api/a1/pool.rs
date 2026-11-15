//! SIM-68: the Armada I damage pool in the shield-percent calls.
//!
//! Rule: for a unit with the `a1_compat` feature (the Armada I profile) the only pool is
//! `maxHealth`, held in the hull (pool = `maxHealth`: measured on the original). The sim
//! regenerates it (`damage::recharge`). The four calls act on it: `SetShieldPercent(h, f)`
//! sets the hull to its maximum times `f` (no upper clamp; a value of 0 or less leaves the least
//! positive hull, which a hit in the next step kills unless the regeneration that runs first in a
//! step lifts it), `SetMaxShields(h, v)` with `v` above 0 sets the hull maximum, then the hull, to `v`
//! (`v` of 0 or less leaves the hull alone: the single pool has no separate shield to remove; STA64
//! rule, inferred), and `GetShieldPercent` / `GetRealShieldPercent` answer the hull over its maximum (0
//! when the maximum is 0). Grade: INFERRED (the executable read says the calls act on the
//! `maxHealth` pool; the setters were not measured). A unit without the feature, including
//! one with authored shields, keeps the shield path of `objects.rs` / `seams.rs`.
//! Taint classification is unchanged (it is keyed by call name).

use mlua::{Function, Lua, Table, Value};
use sta_core::Fixed;
use sta_sim::command::Edit;
use sta_sim::view::UnitView;

use super::Env;

fn is_pool(e: &Env, u: &UnitView) -> bool {
    let c = e.ctx.borrow();
    c.content.as_ref().zip(c.def(u)).is_some_and(|(db, def)| db.unit_features(def).a1_compat)
}

fn hull_fraction(u: &UnitView) -> f64 {
    let (now, max) = (f64::from(u.hull[0].to_f32()), f64::from(u.hull[1].to_f32()));
    if max > 0.0 { now / max } else { 0.0 }
}

pub(super) fn install(lua: &Lua, env: &Env, table: &Table) -> mlua::Result<()> {
    // Each override parses like the call it replaces, answers for the pool unit and hands any
    // other unit (or any argument error) to the binding it replaced.
    let wrap = |name: &'static str, spec: &'static str, f: fn(&Env, &UnitView, &[super::A]) -> mlua::Result<Value>| -> mlua::Result<()> {
        let old: Function = table.get(name)?;
        let env = env.clone();
        table.set(
            name,
            lua.create_function(move |_, args: mlua::MultiValue| {
                if let Ok((_, a)) = env.parse(name, &[spec], &args) {
                    let unit = env.ctx.borrow().alive(a[0].id()).cloned();
                    if let Some(u) = unit.filter(|u| is_pool(&env, u)) {
                        return f(&env, &u, &a);
                    }
                }
                old.call::<Value>(args)
            })?,
        )
    };
    wrap("SetShieldPercent", "hf", |e, u, a| {
        let f = a[1].num();
        // In i128 and clamped: `Fixed`'s own multiply wraps, and the sim caps the value anyway.
        let wide = (i128::from(u.hull[1].to_bits()) * i128::from(Fixed::from_f64(f).to_bits())) >> 32;
        let to = Fixed::from_bits(wide.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64);
        e.edit("SetShieldPercent", Edit::HullValue(u.id, to))?;
        Ok(Value::Nil)
    })?;
    wrap("SetMaxShields", "hf", |e, u, a| {
        let v = Fixed::from_f64(a[1].num());
        if v <= Fixed::ZERO {
            return Ok(Value::Nil);
        }
        e.edit("SetMaxShields", Edit::MaxHull(u.id, v))?;
        e.edit("SetMaxShields", Edit::HullValue(u.id, v))?;
        Ok(Value::Nil)
    })?;
    wrap("GetShieldPercent", "h", |_, u, _| Ok(Value::Number(hull_fraction(u))))?;
    wrap("GetRealShieldPercent", "h", |_, u, _| Ok(Value::Number(hull_fraction(u))))?;
    Ok(())
}
