//! `Entity` handles, `EntityFinder` and `giveOrder`.

use std::rc::Rc;

use mlua::{AnyUserData, Lua, MetaMethod, MultiValue, UserData, UserDataFields, UserDataMethods, UserDataRef, Value};
use sta_core::EntityId;
use sta_sim::command::Edit;
use sta_sim::view::{UnitFlags, UnitView};

use super::player::{player_of, Player};
use super::entity_action::{parse_order, EntityAction};
use super::entity_ref::EntityRef;
use super::spawn;
use super::{dist, point_of, pos_f64, stub, to_vec3, vector, Ctx, Shared};

/// A handle to a unit. Reads see the latest view; a dead unit's handle
/// stays valid as a value but reads `valid == false`.
pub(crate) struct Entity {
    pub key: EntityRef,
    pub ctx: Shared,
}

impl Entity {
    fn remove(&self) {
        spawn::queue_action(&mut self.ctx.borrow_mut(), self.key.clone(), EntityAction::Remove);
    }

    fn with<R>(&self, f: impl FnOnce(&Ctx, Option<&UnitView>) -> R) -> R {
        let c = self.ctx.borrow();
        let u = self.key.id().and_then(|id| c.alive(id)).or_else(|| self.key.hint());
        f(&c, u)
    }
}

/// Whether a def class satisfies `Entity.Type.<t>`.
pub(crate) fn is_type(class: &str, t: &str) -> bool {
    let (class, t) = (class.to_ascii_lowercase(), t.to_ascii_lowercase());
    if t == "entity" || t == "gameobject" || t == class {
        return true;
    }
    match t.as_str() {
        "craft" => matches!(
            class.as_str(),
            "freighter" | "cargoship" | "constructionrig" | "repairship" | "scavenger" | "evolver" | "miningship"
                // Parsed ODF craft labels include static sensors and yards.
                // Producer/Shipyard/Starbase also inherit Craft in the binding metadata.
                | "sensor" | "shipyard" | "starbase" | "miningstation"
        ),
        "producer" => matches!(class.as_str(), "shipyard" | "starbase" | "miningstation"),
        "shipyard" => class == "starbase",
        _ => false,
    }
}

fn entity_value(lua: &Lua, ctx: &Shared, id: EntityId) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(Entity { key: EntityRef::Existing(id), ctx: ctx.clone() })?))
}

impl UserData for Entity {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("id", |_, e| Ok(e.key.id().map(|id| id.0)));
        f.add_field_method_get("pending", |_, e| Ok(e.key.pending()));
        f.add_field_method_get("valid", |_, e| Ok(e.with(|_, u| u.is_some())));
        f.add_field_method_get("destroyed", |_, e| Ok(e.with(|_, u| u.is_none())));
        f.add_field_method_get("position", |lua, e| {
            vector(lua, e.with(|_, u| u.map(|u| pos_f64(u.pos)).unwrap_or_default()))
        });
        f.add_field_method_get("forward", |lua, e| {
            let h = e.with(|_, u| u.map(|u| (u.heading.0.to_f32() as f64, u.heading.1.to_f32() as f64)));
            let (x, z) = h.unwrap_or((0.0, 1.0));
            vector(lua, (x, 0.0, z))
        });
        f.add_field_method_get("velocity", |lua, e| {
            let v = e.with(|_, u| {
                u.map(|u| {
                    let s = u.speed.to_f32() as f64;
                    (u.heading.0.to_f32() as f64 * s, 0.0, u.heading.1.to_f32() as f64 * s)
                })
            });
            vector(lua, v.unwrap_or_default())
        });
        f.add_field_method_get("hull", |_, e| Ok(e.with(|_, u| u.map_or(0.0, |u| u.hull[0].to_f32() as f64))));
        f.add_field_method_get("maxHull", |_, e| Ok(e.with(|_, u| u.map_or(0.0, |u| u.hull[1].to_f32() as f64))));
        f.add_field_method_get("shields", |_, e| {
            Ok(e.with(|_, u| u.map_or(0.0, |u| u.shields.iter().map(|s| s[0].to_f32() as f64).sum::<f64>() / 4.0)))
        });
        f.add_field_method_get("maxShields", |_, e| {
            Ok(e.with(|_, u| u.map_or(0.0, |u| u.shields.iter().map(|s| s[1].to_f32() as f64).sum::<f64>() / 4.0)))
        });
        for name in ["crew", "maxCrew"] {
            f.add_field_method_get(name, |_, e| {
                Ok(e.with(|c, u| u.and_then(|u| c.def(u)).map_or(0.0, |d| d.crew.to_f32() as f64)))
            });
        }
        for name in ["player", "owner"] {
            f.add_field_method_get(name, |lua, e| {
                let owner = e.with(|_, u| u.map(|u| u.owner));
                match owner {
                    Some(o) => Ok(Value::UserData(lua.create_userdata(Player { id: o, ctx: e.ctx.clone() })?)),
                    None => Ok(Value::Nil),
                }
            });
        }
        f.add_field_method_get("perceivedTeam", |lua, e| {
            let owner = {
                let c = e.ctx.borrow();
                e.key.id().filter(|&id| c.alive(id).is_some()).and_then(|id| c.view.perceived_owner(id))
            };
            match owner {
                Some(id) => Ok(Value::UserData(lua.create_userdata(Player { id, ctx: e.ctx.clone() })?)),
                None => Ok(Value::Nil),
            }
        });
        f.add_field_method_get("race", |_, e| {
            Ok(e.with(|c, u| u.and_then(|u| c.def(u)).map(|d| d.race.clone()).unwrap_or_default()))
        });
        // Mutable race is not sim state (SetRace needs a sim edit); the assignment is accepted and recorded as unsupported so
        // the rest of the callback still runs.
        f.add_field_method_set("race", |_, e, _: Value| {
            e.ctx.borrow_mut().unsupported("Entity.race = x (needs a sim SetRace edit)");
            Ok(())
        });
        // SIM-54: current crew, clamped by the sim to 0..the maximum crew.
        f.add_field_method_set("crew", |_, e, value: Value| {
            let mut c = e.ctx.borrow_mut();
            let to = match value {
                Value::Number(n) => n,
                Value::Integer(n) => n as f64,
                _ => {
                    c.unsupported("Entity.crew = non-number");
                    return Ok(());
                }
            };
            if !to.is_finite() {
                c.unsupported("Entity.crew = non-finite number");
                return Ok(());
            }
            match e.key.id() {
                Some(id) => super::edit_feedback::queue(&mut c, "Entity.crew", Edit::UnitCrew(id, sta_core::Fixed::from_f64(to)))?,
                None => c.unsupported("entity.crew: pending or failed spawn"),
            }
            Ok(())
        });
        // `Entity.shields = 1.0`: a fraction of the maximum (SIM-55).
        f.add_field_method_set("shields", |_, e, to: f64| {
            if !to.is_finite() { return Err(mlua::Error::runtime("Entity.shields requires a finite number")); }
            let mut c = e.ctx.borrow_mut();
            match e.key.id() {
                Some(id) => super::edit_feedback::queue(&mut c, "Entity.shields", Edit::ShieldPercent(id, sta_core::Fixed::from_f64(to)))?,
                None => c.unsupported("entity.shields: pending or failed spawn"),
            }
            Ok(())
        });
        // SIM-57: `forward = Vector(x, y, z)` faces the unit along (x, z) at once;
        // `omega = Vector(x, y, z)` sets its turn rate, y being the yaw (positive
        // turns +z toward +x, as in Armada II, whose rule the Roots engine is
        // taken to share: not measured on Roots). Bad input is recorded, not raised.
        for (name, label) in [("forward", "Entity.forward"), ("omega", "Entity.omega")] {
            f.add_field_method_set(name, move |_, e, value: Value| {
                let mut c = e.ctx.borrow_mut();
                let num = |t: &mlua::Table, k: &str| match t.raw_get::<Value>(k) {
                    Ok(Value::Number(n)) if n.is_finite() => Some(n),
                    Ok(Value::Integer(n)) => Some(n as f64),
                    _ => None,
                };
                let Value::Table(t) = value else {
                    c.unsupported(&format!("{label} = non-Vector"));
                    return Ok(());
                };
                let (Some(x), Some(y), Some(z)) = (num(&t, "x"), num(&t, "y"), num(&t, "z")) else {
                    c.unsupported(&format!("{label} = a Vector without finite x, y and z"));
                    return Ok(());
                };
                let Some(id) = e.key.id() else {
                    c.unsupported(&format!("{label}: pending or failed spawn"));
                    return Ok(());
                };
                let edit = if name == "forward" {
                    // Scaled to unit size here only to keep the Fixed conversion in range.
                    let m = x.abs().max(z.abs());
                    if m < 1e-9 {
                        c.unsupported("Entity.forward = a Vector with no horizontal direction");
                        return Ok(());
                    }
                    Edit::Heading(id, (sta_core::Fixed::from_f64(x / m), sta_core::Fixed::from_f64(z / m)))
                } else {
                    Edit::AngularVelocity(id, sta_core::Fixed::from_f64(-y.clamp(-(sta_sim::command::MAX_SCRIPT_OMEGA as f64), sta_sim::command::MAX_SCRIPT_OMEGA as f64)))
                };
                super::edit_feedback::queue(&mut c, label, edit)?;
                Ok(())
            });
        }
        // Other installed-mission assignments with no sim edit behind them yet
        // (autonomy, names): accepted and
        // recorded so the callback keeps running; the report names each.
        for (name, need) in [
            ("movementAutonomy", "a sim autonomy edit"),
            ("specialWeaponAutonomy", "a sim autonomy edit"),
            ("name", "a sim SetName edit"),
        ] {
            f.add_field_method_set(name, move |_, e, _: Value| {
                e.ctx.borrow_mut().unsupported(&format!("Entity.{name} = x (needs {need})"));
                Ok(())
            });
        }
        // `odf` is the ODF name as a string (the real binding may return a
        // class object with `odf`/`name`/`label`; see the module docs).
        // Converted MMM scripts compare this property with file names such
        // as "fente.odf", while content keys remain canonical stems.
        f.add_field_method_get("odf", |_, e| Ok(e.with(|c, u| {
            let name = u.map(|u| c.odf_name(u)).unwrap_or_default();
            if name.is_empty() || odf_stem(&name) != name { name } else { format!("{name}.odf") }
        })));
        f.add_field_method_get("class", |_, e| {
            Ok(e.with(|c, u| u.and_then(|u| c.def(u)).map(|d| d.class.0.clone()).unwrap_or_default()))
        });
        f.add_field_method_get("label", |_, e| Ok(e.key.id().and_then(|id| e.ctx.borrow().labels.get(&id).cloned())));
        f.add_field_method_get("cloaked", |_, e| {
            Ok(e.with(|_, u| u.is_some_and(|u| u.flags.contains(UnitFlags::CLOAKED))))
        });
        f.add_field_method_get("invincible", |_, e| {
            Ok(e.with(|_, u| u.is_some_and(|u| u.flags.contains(UnitFlags::IMMORTAL))))
        });
        f.add_field_method_set("invincible", |_, e, on: bool| {
            spawn::queue_action(&mut e.ctx.borrow_mut(), e.key.clone(), EntityAction::Immortal(on));
            Ok(())
        });
        f.add_field_method_set("hull", |_, e, to: f64| {
            if !to.is_finite() { return Err(mlua::Error::runtime("entity.hull requires a finite value")); }
            spawn::queue_action(&mut e.ctx.borrow_mut(), e.key.clone(), EntityAction::Hull(to));
            Ok(())
        });
        f.add_field_method_set("player", |_, e, p: Value| {
            let mut c = e.ctx.borrow_mut();
            match (e.key.id(), player_of(&c, &p)) {
                (Some(id), Some(p)) => super::edit_feedback::queue(&mut c, "Entity.player", Edit::Team(id, p))?,
                (None, _) => c.unsupported("entity.player: pending or failed spawn"),
                (_, None) => c.unsupported("entity.player = non-player"),
            }
            Ok(())
        });
        f.add_field_method_set("forceOntoMap", |_, e, value: Value| {
            let Value::Boolean(enabled) = value else {
                return Err(mlua::Error::runtime("Entity.forceOntoMap requires a boolean"));
            };
            if e.key.id().is_some_and(|id| e.ctx.borrow().alive(id).is_none()) {
                return Err(mlua::Error::runtime("Entity.forceOntoMap requires a live object"));
            }
            spawn::queue_action(&mut e.ctx.borrow_mut(), e.key.clone(), EntityAction::ForceOntoMap(enabled));
            Ok(())
        });
        f.add_field_method_set("perceivedTeam", |_, e, value: Value| {
            if matches!(&value, Value::Number(n) if !n.is_finite() || n.fract() != 0.0) {
                return Err(mlua::Error::runtime("Entity.perceivedTeam requires a Player or integral player number"));
            }
            let destination = player_of(&e.ctx.borrow(), &value)
                .filter(|p| p.is_player() || *p == sta_sim::world::NEUTRAL)
                .ok_or_else(|| mlua::Error::runtime("Entity.perceivedTeam requires a real Player or neutral"))?;
            if e.key.id().is_some_and(|id| e.ctx.borrow().alive(id).is_none()) {
                return Err(mlua::Error::runtime("Entity.perceivedTeam requires a live object"));
            }
            spawn::queue_action(&mut e.ctx.borrow_mut(), e.key.clone(), EntityAction::PerceivedTeam(destination));
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("isType", |_, e, t: Value| {
            let Value::String(t) = t else { return Ok(false) };
            let t = t.to_string_lossy();
            Ok(e.with(|c, u| match u.and_then(|u| c.def(u)) {
                Some(d) => is_type(&d.class.0, &t),
                None => u.is_some() && is_type("", &t),
            }))
        });
        m.add_method("canPlayerSee", |_, e, p: Value| {
            let c = e.ctx.borrow();
            let Some(p) = player_of(&c, &p) else { return Ok(false) };
            let Some(id) = e.key.id() else { return Ok(false) };
            Ok(c.view.visible_to(p).iter().any(|u| u.id == id))
        });
        m.add_method("remove", |_, e, _: MultiValue| {
            e.remove();
            Ok(())
        });
        m.add_method("cloak", |_, e, _: MultiValue| {
            spawn::queue_action(&mut e.ctx.borrow_mut(), e.key.clone(), EntityAction::Cloak(true));
            Ok(())
        });
        m.add_method("decloak", |_, e, _: MultiValue| {
            spawn::queue_action(&mut e.ctx.borrow_mut(), e.key.clone(), EntityAction::Cloak(false));
            Ok(())
        });
        m.add_method("giveOrder", |_, e, args: MultiValue| {
            give_order(&e.ctx, e.key.clone(), &args);
            Ok(())
        });
        m.add_meta_method(MetaMethod::Eq, |_, e, o: UserDataRef<Entity>| Ok(Rc::ptr_eq(&e.ctx, &o.ctx) && e.key.same_entity(&o.key)));
        m.add_meta_method(MetaMethod::ToString, |_, e, ()| Ok(match e.key.id() {
            Some(id) => format!("Entity#{}", id.0),
            None => format!("Entity<{}>", if e.key.pending() { "pending" } else { "invalid" }),
        }));
        m.add_meta_method(MetaMethod::Index, |lua, _, _: Value| stub(lua));
    }
}

/// `unit:giveOrder(Order.X, target?, queue?)`. Orders the sim has no
/// command for are counted in `Ctx::unsupported`.
fn give_order(ctx: &Shared, key: EntityRef, args: &MultiValue) {
    let parsed = parse_order(ctx, &key, args);
    match parsed {
        Ok(intent) => { spawn::queue_action(&mut ctx.borrow_mut(), key, EntityAction::Order(intent)); }
        Err(reason) => ctx.borrow_mut().unsupported(&reason),
    }
}

fn str_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// Entities matching a filter, ascending id.
fn odf_stem(name: &str) -> &str {
    if name.get(name.len().saturating_sub(4)..).is_some_and(|s| s.eq_ignore_ascii_case(".odf")) {
        &name[..name.len() - 4]
    } else {
        name
    }
}

fn validate_owner_filter(ctx: &Shared, value: &Value, owner: Option<sta_core::PlayerId>) -> mlua::Result<()> {
    if !value.is_nil() && owner.is_none() && ctx.borrow().player_numbering == super::PlayerNumbering::ArmadaTeams {
        return Err(mlua::Error::runtime("EntityFinder requires a valid Armada team number or Player"));
    }
    Ok(())
}

fn find(lua: &Lua, ctx: &Shared, filter: &Value, predicate: Option<&mlua::Function>) -> mlua::Result<Vec<EntityId>> {
    let ci = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let (mut label, mut odf, mut class, mut ty, mut owner) = (None, None, None, None, None);
    let mut any_name: Option<String> = None;
    let mut circle = None;
    match filter {
        Value::String(s) => any_name = Some(s.to_string_lossy()),
        Value::Table(t) => {
            label = t.get::<Value>("label")?.as_string().map(|s| s.to_string_lossy());
            odf = t.get::<Value>("odf")?.as_string().map(|s| s.to_string_lossy());
            class = t.get::<Value>("class")?.as_string().map(|s| s.to_string_lossy());
            ty = str_of(&t.get::<Value>("type")?);
            let player = t.get::<Value>("player")?;
            owner = player_of(&ctx.borrow(), &player);
            validate_owner_filter(ctx, &player, owner)?;
            let centre = point_of(ctx, &t.get::<Value>("center")?);
            let radius: Option<f64> = t.get("radius")?;
            if let (Some(p), Some(r)) = (centre, radius) {
                circle = Some((p, r));
            }
        }
        Value::Integer(_) | Value::Number(_) | Value::UserData(_) => {
            owner = player_of(&ctx.borrow(), filter);
            validate_owner_filter(ctx, filter, owner)?;
        }
        _ => {}
    }
    // Filter fields may execute Lua and queue edits. Borrow the view only
    // after all such lookups finish, as with the predicates below.
    let c = ctx.borrow();
    let mut out = Vec::new();
    for u in &c.view.units {
        if u.flags.contains(UnitFlags::DYING) {
            continue;
        }
        let lab = c.labels.get(&u.id).map(String::as_str);
        let def = c.def(u);
        let name = c.odf_name(u);
        let cls = def.map(|d| d.class.0.as_str()).unwrap_or("");
        if any_name.as_ref().is_some_and(|a| !(lab.is_some_and(|l| ci(l, a)) || ci(odf_stem(&name), odf_stem(a)) || ci(cls, a))) {
            continue;
        }
        if label.as_deref().is_some_and(|l| !lab.is_some_and(|x| ci(x, l))) {
            continue;
        }
        if odf.as_deref().is_some_and(|o| !ci(odf_stem(&name), odf_stem(o))) {
            continue;
        }
        if class.as_deref().is_some_and(|k| !ci(cls, k)) {
            continue;
        }
        if ty.as_deref().is_some_and(|t| !is_type(cls, t)) {
            continue;
        }
        if owner.is_some_and(|o| u.owner != o) {
            continue;
        }
        if circle.is_some_and(|(p, r)| dist(pos_f64(u.pos), p) > r) {
            continue;
        }
        out.push(u.id);
    }
    drop(c);
    out.sort();
    if predicate.is_none() && filter.as_function().is_none() { return Ok(out); }
    // Lua predicates may query entities or queue commands. Never retain a
    // context borrow while calling them, and snapshot candidate IDs first.
    let mut matched = Vec::new();
    for id in out {
        let entity = entity_value(lua, ctx, id)?;
        let mut keep = true;
        for predicate in [predicate, filter.as_function()].into_iter().flatten() {
            if !predicate.call::<bool>(entity.clone())? {
                keep = false;
                break;
            }
        }
        if keep { matched.push(id); }
    }
    Ok(matched)
}

/// The filter argument of `finder:find(f)`, with or without `self`.
fn filter_arg(args: &MultiValue) -> Value {
    match args.len() {
        0 => Value::Nil,
        1 => args[0].clone(),
        _ => args[1].clone(),
    }
}

fn finder_methods(lua: &Lua, ctx: &Shared, predicate: Option<mlua::Function>) -> mlua::Result<mlua::Table> {
    let finder = lua.create_table()?;
    let c = ctx.clone();
    let pred = predicate.clone();
    finder.set(
        "find",
        lua.create_function(move |lua, args: MultiValue| {
            let ids = find(lua, &c, &filter_arg(&args), pred.as_ref())?;
            let t = lua.create_table()?;
            for (i, id) in ids.into_iter().enumerate() {
                t.set(i + 1, entity_value(lua, &c, id)?)?;
            }
            Ok(t)
        })?,
    )?;
    let c = ctx.clone();
    finder.set(
        "findOne",
        lua.create_function(move |lua, args: MultiValue| match find(lua, &c, &filter_arg(&args), predicate.as_ref())?.first() {
            Some(&id) => entity_value(lua, &c, id),
            None => Ok(Value::Nil),
        })?,
    )?;
    Ok(finder)
}

pub(crate) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    let entities: mlua::Table = lua.globals().raw_get("Entity")?;
    entities.raw_set("remove", lua.create_function(|_, args: MultiValue| {
        if args.len() != 1 {
            return Err(mlua::Error::runtime("Entity.remove expects one Entity handle"));
        }
        let Value::UserData(value) = &args[0] else {
            return Err(mlua::Error::runtime("Entity.remove requires an Entity handle"));
        };
        let entity = value.borrow::<Entity>()
            .map_err(|_| mlua::Error::runtime("Entity.remove requires an Entity handle"))?;
        entity.remove();
        Ok(())
    })?)?;
    let c = ctx.clone();
    entities.raw_set("add", lua.create_function(move |lua, args: MultiValue| {
        if args.len() != 3 {
            return Err(mlua::Error::runtime("Entity.add expects ODF, player and Vector"));
        }
        let Value::String(name) = &args[0] else {
            return Err(mlua::Error::runtime("Entity.add requires an ODF name"));
        };
        let name = name.to_str()?.to_owned();
        if let Value::Number(n) = &args[1]
            && (!n.is_finite() || n.fract() != 0.0) {
            return Err(mlua::Error::runtime("Entity.add requires an integer player slot"));
        }
        let owner = player_of(&c.borrow(), &args[1])
            .filter(|p| p.is_player() || *p == sta_sim::world::NEUTRAL)
            .ok_or_else(|| mlua::Error::runtime("Entity.add requires a valid player or neutral owner in the configured numbering mode"))?;
        let Value::Table(point) = &args[2] else {
            return Err(mlua::Error::runtime("Entity.add requires a Vector"));
        };
        // Read Lua fields before borrowing the context: user-defined table
        // lookups may reenter the API. Coordinates must fit the sim's fixed point.
        let (x, y, z) = (point.get::<f64>("x")?, point.get::<Option<f64>>("y")?.unwrap_or(0.0), point.get::<f64>("z")?);
        if [x, y, z].iter().any(|v| !v.is_finite() || !(-2147483648.0..2147483648.0).contains(v)) {
            return Err(mlua::Error::runtime("Entity.add position must be finite and fit 32.32 fixed point"));
        }
        let key = {
            let c = c.borrow();
            c.content.as_ref().and_then(|db| db.key(odf_stem(&name)).filter(|key| db.unit(*key).is_some()))
                .ok_or_else(|| mlua::Error::runtime("Entity.add requires a known unit ODF"))?
        };
        let key = spawn::queue_spawn(&mut c.borrow_mut(), key, owner, to_vec3(x, y, z))
            .map_err(mlua::Error::runtime)?;
        lua.create_userdata(Entity { key, ctx: c.clone() })
    })?)?;
    // `EntityFinder()` makes an object with the same methods; the methods
    // ignore `self`, so `EntityFinder.find(f)` works too.
    let class = lua.create_table()?;
    let methods = finder_methods(lua, ctx, None)?;
    let class_mt = lua.create_table()?;
    let c = ctx.clone();
    class_mt.set(
        "__call",
        lua.create_function(move |lua, args: MultiValue| {
            if args.len() > 2 {
                return Err(mlua::Error::runtime("EntityFinder expects at most one predicate"));
            }
            let predicate = match args.get(1) {
                None | Some(Value::Nil) => None,
                Some(Value::Function(f)) => Some(f.clone()),
                _ => return Err(mlua::Error::runtime("EntityFinder predicate must be a function")),
            };
            let o = lua.create_table()?;
            let omt = lua.create_table()?;
            omt.set("__index", finder_methods(lua, &c, predicate)?)?;
            o.set_metatable(Some(omt))?;
            Ok(o)
        })?,
    )?;
    class_mt.set("__index", methods)?;
    class.set_metatable(Some(class_mt))?;
    lua.globals().raw_set("EntityFinder", class)?;
    Ok(())
}

/// An entity handle for `id`, for monitors.
pub(crate) fn handle(lua: &Lua, ctx: &Shared, id: EntityId) -> mlua::Result<AnyUserData> {
    lua.create_userdata(Entity { key: EntityRef::Existing(id), ctx: ctx.clone() })
}
