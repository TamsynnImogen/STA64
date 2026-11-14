//! Armada I orders and AI calls. Orders are sent as the unit's owner, like a
//! player's input; calls with no order or AI seam in the simulation are
//! retained as `A1.<Name>` calls and counted (see `seams.rs`).
//!
//! The trailing `int` of `Goto` and `Stop` is validated and ignored: its
//! meaning is not measured.

use mlua::{Lua, Table, Value};
use sta_core::EntityId;
use sta_sim::command::Order;

use super::{A, Env, args_of, def};

fn send(e: &Env, name: &str, id: EntityId, make: impl FnOnce(Vec<EntityId>) -> Order) -> mlua::Result<()> {
    e.unit(name, id)?;
    if e.ctx.borrow_mut().order(id, make) {
        Ok(())
    } else {
        Err(mlua::Error::runtime(format!("A1.{name}: object {} has no player owner or the order could not be queued", id.0)))
    }
}

/// How long after its last shot a unit counts as in attack mode (not measured).
const ATTACK_MODE_SECONDS: u32 = 3;

pub(super) fn install(lua: &Lua, env: &Env, table: &Table) -> mlua::Result<()> {
    // `Goto(unit, path, int)` moves along the path's points in order;
    // `Goto(unit, target, int)` moves to the target's position.
    table.set("Goto", def(lua, env, "Goto", &["hsi", "hhi"], |lua, e, ov, a| {
        let id = a[0].id();
        // Check everything before queueing the first leg, so a failure leaves nothing queued.
        if !e.unit("Goto", id)?.owner.is_player() {
            return Err(mlua::Error::runtime("A1.Goto: object has no player owner"));
        }
        if ov == 0 {
            // An absent path name: the engine's lookup is null and the call does nothing.
            let Some(points) = e.find_path(lua, "Goto", a[1].text())? else { return Ok(Value::Nil) };
            for (n, to) in points.into_iter().enumerate() {
                send(e, "Goto", id, |units| Order::Move { units, to, queue: n > 0 })?;
            }
        } else {
            let to = e.unit("Goto", a[1].id())?.pos;
            send(e, "Goto", id, |units| Order::Move { units, to, queue: false })?;
        }
        Ok(Value::Nil)
    })?)?;
    table.set("Stop", def(lua, env, "Stop", &["hi"], |_, e, _, a| {
        send(e, "Stop", a[0].id(), |units| Order::Stop { units })?;
        Ok(Value::Nil)
    })?)?;
    // `Attack(unit, target, int)`; the position form is an attack-move, which
    // has no order in the simulation.
    table.set("Attack", def(lua, env, "Attack", &["hhi", "hv"], |_, e, ov, a| {
        if ov == 0 {
            let target = e.unit("Attack", a[1].id())?.id;
            send(e, "Attack", a[0].id(), |units| Order::Attack { units, target })?;
        } else {
            e.unit("Attack", a[0].id())?;
            e.gap("Attack", "no attack-move order in the sim seam", args_of(a))?;
        }
        Ok(Value::Nil)
    })?)?;
    table.set("Mine", def(lua, env, "Mine", &["hh"], |_, e, _, a| {
        let node = e.unit("Mine", a[1].id())?.id;
        send(e, "Mine", a[0].id(), |units| Order::Mine { units, node })?;
        Ok(Value::Nil)
    })?)?;
    table.set("Build", def(lua, env, "Build", &["hs"], |_, e, _, a| {
        let odf = a[1].text();
        let key = e
            .ctx
            .borrow()
            .content
            .as_ref()
            .and_then(|db| db.key(super::objects::stem(odf)))
            .ok_or_else(|| mlua::Error::runtime(format!("A1.Build: unknown ODF '{odf}'")))?;
        send(e, "Build", a[0].id(), |builder| Order::Build { builder: builder[0], what: key, at: None })?;
        Ok(Value::Nil)
    })?)?;
    // `StopAllShips(team, odf-or-nil, bool, bool)`: stop every live unit of the
    // team (matching the ODF when one is given). The two bools are not measured.
    table.set("StopAllShips", def(lua, env, "StopAllShips", &["iSbb"], |_, e, _, a| {
        let team = e.team("StopAllShips", a[0].int())?;
        let ids: Vec<EntityId> = {
            let c = e.ctx.borrow();
            c.view
                .units
                .iter()
                .filter(|u| u.owner == team && c.alive(u.id).is_some())
                .filter(|u| match &a[1] {
                    A::Str(s) if !s.is_empty() => {
                        c.odf_name(u).trim_end_matches(".odf").eq_ignore_ascii_case(s.trim_end_matches(".odf"))
                    }
                    _ => true,
                })
                .map(|u| u.id)
                .collect()
        };
        for id in ids {
            e.ctx.borrow_mut().order(id, |units| Order::Stop { units });
        }
        Ok(Value::Nil)
    })?)?;

    // Queries over order state the view does not carry.
    table.set("InStopMode", no_order_state(lua, env, "InStopMode")?)?;
    table.set("OutOfSpecialAttackMode", no_order_state(lua, env, "OutOfSpecialAttackMode")?)?;
    // The view carries no order queue, so these two answer from what it does carry
    // (LUA-17; approximations, not measured against the original):
    // `InQueue(unit)`: the craft is still under way (speed above zero), which is how
    // stock missions use it ("arrived": near the point and no longer in its queue).
    table.set("InQueue", def(lua, env, "InQueue", &["h"], |_, e, _, a| {
        Ok(Value::Boolean(e.unit("InQueue", a[0].id())?.speed > sta_core::Fixed::ZERO))
    })?)?;
    // `InAttackMode(unit)`: the unit fired a weapon within the last
    // `ATTACK_MODE_SECONDS` (stock missions ask it before ordering an idle patrol to engage).
    table.set("InAttackMode", def(lua, env, "InAttackMode", &["h"], |_, e, _, a| {
        let id = e.unit("InAttackMode", a[0].id())?.id;
        let c = e.ctx.borrow();
        let within = u64::from(ATTACK_MODE_SECONDS * sta_core::TICK_HZ);
        Ok(Value::Boolean(c.last_fired.get(&id).is_some_and(|t| c.view.tick.saturating_sub(*t) <= within)))
    })?)?;
    table.set("GetAttackLocation", def(lua, env, "GetAttackLocation", &["hh"], |_, e, _, _| {
        Err(e.no_data("GetAttackLocation", "the world view carries no order target"))
    })?)?;
    table.set("GetAllyShipAttacks", def(lua, env, "GetAllyShipAttacks", &["ii"], |_, e, _, _| {
        Err(e.no_data("GetAllyShipAttacks", "AI attack statistics are not in the world view"))
    })?)?;
    table.set("GetAllyStationAttacks", def(lua, env, "GetAllyStationAttacks", &["ii"], |_, e, _, _| {
        Err(e.no_data("GetAllyStationAttacks", "AI attack statistics are not in the world view"))
    })?)?;
    Ok(())
}

fn no_order_state(lua: &Lua, env: &Env, name: &'static str) -> mlua::Result<mlua::Function> {
    def(lua, env, name, &["h"], move |_, e, _, a| {
        e.unit(name, a[0].id())?;
        Err(e.no_data(name, "the world view carries no order state"))
    })
}
