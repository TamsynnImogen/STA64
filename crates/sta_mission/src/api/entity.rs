//! `Entity` handles, `EntityFinder` and `giveOrder`.

use mlua::{AnyUserData, Lua, MetaMethod, MultiValue, UserData, UserDataFields, UserDataMethods, UserDataRef, Value};
use sta_core::{EntityId, Fixed};
use sta_sim::command::{Edit, Order, Target};
use sta_sim::view::{UnitFlags, UnitView};

use super::player::{player_of, Player};
use super::{dist, point_of, pos_f64, stub, to_vec3, vector, Ctx, Shared};

/// A handle to a unit. Reads see the latest view; a dead unit's handle
/// stays valid as a value but reads `valid == false`.
pub(crate) struct Entity {
    pub id: EntityId,
    pub ctx: Shared,
}

impl Entity {
    fn with<R>(&self, f: impl FnOnce(&Ctx, Option<&UnitView>) -> R) -> R {
        let c = self.ctx.borrow();
        let u = c.alive(self.id);
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
        ),
        "producer" => matches!(class.as_str(), "shipyard" | "starbase" | "miningstation"),
        _ => false,
    }
}

fn entity_value(lua: &Lua, ctx: &Shared, id: EntityId) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(Entity { id, ctx: ctx.clone() })?))
}

impl UserData for Entity {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("id", |_, e| Ok(e.id.0));
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
        f.add_field_method_get("race", |_, e| {
            Ok(e.with(|c, u| u.and_then(|u| c.def(u)).map(|d| d.race.clone()).unwrap_or_default()))
        });
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
        f.add_field_method_get("label", |_, e| Ok(e.ctx.borrow().labels.get(&e.id).cloned()));
        f.add_field_method_get("cloaked", |_, e| {
            Ok(e.with(|_, u| u.is_some_and(|u| u.flags.contains(UnitFlags::CLOAKED))))
        });
        f.add_field_method_get("invincible", |_, e| {
            Ok(e.with(|_, u| u.is_some_and(|u| u.flags.contains(UnitFlags::IMMORTAL))))
        });
        f.add_field_method_set("invincible", |_, e, on: bool| {
            e.ctx.borrow_mut().edit(Edit::Immortal(e.id, on));
            Ok(())
        });
        f.add_field_method_set("hull", |_, e, to: f64| {
            let mut c = e.ctx.borrow_mut();
            if let Some(now) = c.alive(e.id).map(|u| u.hull[0].to_f32() as f64) {
                if to < now {
                    c.edit(Edit::Damage(e.id, Fixed::from_f64(now - to)));
                } else if to > now {
                    c.edit(Edit::Heal(e.id));
                }
            }
            Ok(())
        });
        f.add_field_method_set("player", |_, e, p: Value| {
            let mut c = e.ctx.borrow_mut();
            match player_of(&p) {
                Some(p) => c.edit(Edit::Team(e.id, p)),
                None => c.unsupported("entity.player = non-player"),
            }
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
            let Some(p) = player_of(&p) else { return Ok(false) };
            Ok(c.view.visible_to(p).iter().any(|u| u.id == e.id))
        });
        m.add_method("remove", |_, e, _: MultiValue| {
            e.ctx.borrow_mut().edit(Edit::Kill(e.id));
            Ok(())
        });
        m.add_method("cloak", |_, e, _: MultiValue| {
            e.ctx.borrow_mut().order(e.id, |units| Order::Cloak { units, on: true });
            Ok(())
        });
        m.add_method("decloak", |_, e, _: MultiValue| {
            e.ctx.borrow_mut().order(e.id, |units| Order::Cloak { units, on: false });
            Ok(())
        });
        m.add_method("giveOrder", |_, e, args: MultiValue| {
            give_order(&e.ctx, e.id, &args);
            Ok(())
        });
        m.add_meta_method(MetaMethod::Eq, |_, e, o: UserDataRef<Entity>| Ok(e.id == o.id));
        m.add_meta_method(MetaMethod::ToString, |_, e, ()| Ok(format!("Entity#{}", e.id.0)));
        m.add_meta_method(MetaMethod::Index, |lua, _, _: Value| stub(lua));
    }
}

/// `unit:giveOrder(Order.X, target?, queue?)`. Orders the sim has no
/// command for are counted in `Ctx::unsupported`.
fn give_order(ctx: &Shared, id: EntityId, args: &MultiValue) {
    let mut c = ctx.borrow_mut();
    let Some(Value::String(name)) = args.front() else {
        c.unsupported("giveOrder: no order");
        return;
    };
    let name = name.to_string_lossy();
    let target = args.get(1).cloned().unwrap_or(Value::Nil);
    let queue = matches!(args.get(2), Some(Value::Boolean(true)));
    let target_entity = |v: &Value| match v {
        Value::UserData(u) => u.borrow::<Entity>().ok().map(|e| e.id),
        _ => None,
    };
    match name.as_str() {
        "Go" | "GoSingle" | "MoveHigh" | "MoveMedium" | "MoveLow" | "GoWarp" | "GoWarpSingle" | "Scout" => {
            match point_of(&c, &target) {
                Some((x, y, z)) => {
                    c.order(id, |units| Order::Move { units, to: to_vec3(x, y, z), queue });
                }
                None => c.unsupported("giveOrder Go: no point"),
            }
        }
        "Stop" | "None" | "Default" => {
            c.order(id, |units| Order::Stop { units });
        }
        "Attack" | "SearchAndDestroy" | "AnchoredAttack" => match target_entity(&target) {
            Some(t) => {
                c.order(id, |units| Order::Attack { units, target: t });
            }
            None => c.unsupported("giveOrder Attack: no target entity"),
        },
        "Mine" => match target_entity(&target) {
            Some(t) => {
                c.order(id, |units| Order::Mine { units, node: t });
            }
            None => c.unsupported("giveOrder Mine: no target entity"),
        },
        "SpecialWeaponHigh" | "SpecialWeaponMedium" | "SpecialAttack" => {
            let slot = if name == "SpecialWeaponMedium" { 1 } else { 0 };
            let tgt = match (target_entity(&target), point_of(&c, &target)) {
                (Some(t), _) => Target::Entity(t),
                (None, Some((x, y, z))) => Target::Point(to_vec3(x, y, z)),
                _ => Target::None,
            };
            c.order(id, |units| Order::Special { units, slot, target: tgt });
        }
        other => c.unsupported(&format!("giveOrder {other}")),
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

fn find(lua: &Lua, ctx: &Shared, filter: &Value, predicate: Option<&mlua::Function>) -> mlua::Result<Vec<EntityId>> {
    let c = ctx.borrow();
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
            owner = player_of(&t.get::<Value>("player")?);
            let centre = point_of(&c, &t.get::<Value>("center")?);
            let radius: Option<f64> = t.get("radius")?;
            if let (Some(p), Some(r)) = (centre, radius) {
                circle = Some((p, r));
            }
        }
        Value::Integer(_) | Value::Number(_) | Value::UserData(_) => owner = player_of(filter),
        _ => {}
    }
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
    lua.create_userdata(Entity { id, ctx: ctx.clone() })
}
