//! `Player`, relations, resources, `Game` and objectives.

use mlua::{Lua, MetaMethod, MultiValue, UserData, UserDataFields, UserDataMethods, UserDataRef, Value};
use sta_core::{Fixed, PlayerId};
use sta_sim::command::Edit;
use sta_sim::view::Relation;
use sta_sim::TICK_HZ;

use super::{stub, Objective, ObjectiveState, Shared};

/// A seat. Reads see the latest view.
pub(crate) struct Player {
    pub id: PlayerId,
    pub ctx: Shared,
}

/// A player from a number or a `Player` handle.
pub(crate) fn player_of(v: &Value) -> Option<PlayerId> {
    match v {
        Value::Integer(n) => u8::try_from(*n).ok().map(PlayerId),
        Value::Number(n) if *n >= 0.0 && *n < 256.0 => Some(PlayerId(*n as u8)),
        Value::UserData(u) => u.borrow::<Player>().ok().map(|p| p.id),
        _ => None,
    }
}

fn relation_name(r: Relation) -> &'static str {
    match r {
        Relation::Ally => "Ally",
        Relation::Enemy => "Enemy",
        _ => "Neutral",
    }
}

const RESOURCES: [(&str, u8); 4] = [("dilithium", 0), ("latinum", 1), ("metal", 2), ("biomatter", 3)];

fn resources(lua: &Lua, p: &Player) -> mlua::Result<mlua::Table> {
    let t = lua.create_table()?;
    let mt = lua.create_table()?;
    let (c, id) = (p.ctx.clone(), p.id);
    mt.set(
        "__index",
        lua.create_function(move |lua, (_t, k): (Value, String)| {
            let c = c.borrow();
            let view = c.view.player(id);
            if k == "crew" {
                return Ok(Value::Number(view.map_or(0.0, |p| p.officers[0].to_f32() as f64)));
            }
            match RESOURCES.iter().find(|(n, _)| *n == k) {
                Some((_, i)) => {
                    Ok(Value::Number(view.and_then(|p| p.resources.get(*i as usize)).map_or(0.0, |v| v.to_f32() as f64)))
                }
                None => stub(lua),
            }
        })?,
    )?;
    let (c, id) = (p.ctx.clone(), p.id);
    mt.set(
        "__newindex",
        lua.create_function(move |_, (_t, k, v): (Value, String, f64)| {
            let mut c = c.borrow_mut();
            match RESOURCES.iter().find(|(n, _)| *n == k) {
                Some((_, i)) => c.edit(Edit::Cargo(id, *i, Fixed::from_f64(v))),
                None => c.unsupported(&format!("resources.{k} = x")),
            }
            Ok(())
        })?,
    )?;
    t.set_metatable(Some(mt))?;
    Ok(t)
}

impl UserData for Player {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("number", |_, p| Ok(p.id.0));
        f.add_field_method_get("name", |_, p| Ok(format!("Player {}", p.id.0)));
        f.add_field_method_get("race", |_, p| {
            // The race of the player's first unit that has one.
            let c = p.ctx.borrow();
            Ok(c.view.units.iter().filter(|u| u.owner == p.id).find_map(|u| {
                c.def(u).map(|d| d.race.clone()).filter(|r| !r.is_empty())
            }).unwrap_or_default())
        });
        f.add_field_method_get("isAI", |_, _| Ok(false));
        f.add_field_method_get("slotType", |_, p| {
            Ok(if p.ctx.borrow().seat == p.id { "Human" } else { "None" })
        });
        for name in ["maxOfficers", "maxCrew"] {
            f.add_field_method_get(name, |_, p| {
                Ok(p.ctx.borrow().view.player(p.id).map_or(0.0, |v| v.officers[1].to_f32() as f64))
            });
        }
        f.add_field_method_get("resources", resources);
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("getRelation", |_, p, other: Value| {
            let c = p.ctx.borrow();
            Ok(match player_of(&other) {
                Some(o) => relation_name(c.relation(p.id, o)),
                None => "Neutral",
            })
        });
        m.add_method("setRelation", |_, p, (other, rel): (Value, String)| {
            let mut c = p.ctx.borrow_mut();
            let rel = match rel.as_str() {
                "Ally" => Relation::Ally,
                "Enemy" => Relation::Enemy,
                _ => Relation::Neutral,
            };
            if let Some(o) = player_of(&other) {
                c.relations.insert((p.id, o), rel);
            }
            Ok(())
        });
        m.add_method("loadAIP", |_, p, name: Option<String>| {
            p.ctx.borrow_mut().loaded_aips.push((p.id, name.unwrap_or_default()));
            Ok(())
        });
        m.add_meta_method(MetaMethod::Eq, |_, p, o: UserDataRef<Player>| Ok(p.id == o.id));
        m.add_meta_method(MetaMethod::ToString, |_, p, ()| Ok(format!("Player#{}", p.id.0)));
        m.add_meta_method(MetaMethod::Index, |lua, _, _: Value| stub(lua));
    }
}

/// One objective, by id.
struct ObjectiveHandle {
    id: u32,
    ctx: Shared,
}

impl ObjectiveHandle {
    fn with<R>(&self, f: impl FnOnce(&mut Objective) -> R) -> Option<R> {
        self.ctx.borrow_mut().objectives.iter_mut().find(|o| o.id == self.id).map(f)
    }
}

impl UserData for ObjectiveHandle {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("id", |_, o| Ok(o.id));
        f.add_field_method_get("title", |_, o| Ok(o.with(|o| o.title.clone())));
        f.add_field_method_set("title", |_, o, t: String| { let _ = o.with(|o| o.title = t); Ok(()) });
        f.add_field_method_get("description", |_, o| Ok(o.with(|o| o.description.clone())));
        f.add_field_method_set("description", |_, o, t: String| { let _ = o.with(|o| o.description = t); Ok(()) });
        f.add_field_method_get("state", |_, o| {
            Ok(o.with(|o| match o.state {
                ObjectiveState::Active => "active",
                ObjectiveState::Complete => "complete",
                ObjectiveState::Failed => "failed",
            }))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("complete", |_, o, _: MultiValue| { let _ = o.with(|o| o.state = ObjectiveState::Complete); Ok(()) });
        m.add_method("fail", |_, o, _: MultiValue| { let _ = o.with(|o| o.state = ObjectiveState::Failed); Ok(()) });
        m.add_meta_method(MetaMethod::Index, |lua, _, _: Value| stub(lua));
    }
}

/// `Objectives.objectives`: the list.
struct ObjectiveList(Shared);

impl UserData for ObjectiveList {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("count", |_, l| Ok(l.0.borrow().objectives.len()));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("add", |lua, l, args: MultiValue| {
            // `add(title)` or `add{ title = , description = }`, with or
            // without a leading `self`.
            let arg = args.iter().rev().find(|v| matches!(v, Value::String(_) | Value::Table(_))).cloned();
            let (title, description) = match arg {
                Some(Value::String(s)) => (s.to_string_lossy(), String::new()),
                Some(Value::Table(t)) => (
                    t.get::<Option<String>>("title")?.unwrap_or_default(),
                    t.get::<Option<String>>("description")?.unwrap_or_default(),
                ),
                _ => (String::new(), String::new()),
            };
            let mut c = l.0.borrow_mut();
            let id = c.objectives.last().map_or(1, |o| o.id + 1);
            c.objectives.push(Objective { id, title, description, state: ObjectiveState::Active });
            drop(c);
            lua.create_userdata(ObjectiveHandle { id, ctx: l.0.clone() })
        });
        m.add_method("get", |lua, l, i: i64| {
            let id = l.0.borrow().objectives.get((i - 1).max(0) as usize).map(|o| o.id);
            match id {
                Some(id) if i >= 1 => Ok(Value::UserData(lua.create_userdata(ObjectiveHandle { id, ctx: l.0.clone() })?)),
                _ => Ok(Value::Nil),
            }
        });
        m.add_meta_method(MetaMethod::Len, |_, l, ()| Ok(l.0.borrow().objectives.len()));
        m.add_meta_method(MetaMethod::Index, |lua, _, _: Value| stub(lua));
    }
}

/// `Objectives`.
struct Objectives(Shared);

impl UserData for Objectives {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("objectives", |lua, o| lua.create_userdata(ObjectiveList(o.0.clone())));
        f.add_field_method_get("count", |_, o| Ok(o.0.borrow().objectives.len()));
        f.add_field_method_get("visible", |_, o| Ok(o.0.borrow().objectives_visible));
        f.add_field_method_set("visible", |_, o, v: bool| { o.0.borrow_mut().objectives_visible = v; Ok(()) });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // Loading an objectives file needs its format; recorded, no effect.
        m.add_function("load", |_, args: MultiValue| {
            for v in args.iter() {
                if let Some(o) = v.as_userdata().and_then(|u| u.borrow::<Objectives>().ok()) {
                    o.0.borrow_mut().unsupported("Objectives.load");
                }
            }
            Ok(())
        });
        m.add_meta_method(MetaMethod::Index, |lua, _, _: Value| stub(lua));
    }
}

pub(crate) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    let g = lua.globals();

    // `Player(n)` and `Player.get(n)`.
    let class = lua.create_table()?;
    class.set("Relation", g.raw_get::<Value>("Relation")?)?;
    let c = ctx.clone();
    let get = lua.create_function(move |lua, args: MultiValue| {
        let n = args.iter().rev().find_map(player_of);
        match n {
            Some(id) => Ok(Value::UserData(lua.create_userdata(Player { id, ctx: c.clone() })?)),
            None => Ok(Value::Nil),
        }
    })?;
    class.set("get", get.clone())?;
    let mt = lua.create_table()?;
    mt.set("__call", get)?;
    class.set_metatable(Some(mt))?;
    g.raw_set("Player", class)?;

    g.raw_set("Objectives", lua.create_userdata(Objectives(ctx.clone()))?)?;

    // `Game`: read-only fields from the context.
    let game = lua.create_table()?;
    let gmt = lua.create_table()?;
    let c = ctx.clone();
    gmt.set(
        "__index",
        lua.create_function(move |lua, (_t, k): (Value, String)| {
            let b = c.borrow();
            Ok(match k.as_str() {
                "time" => Value::Number(b.view.tick as f64 / TICK_HZ as f64),
                "localSlot" => Value::Integer(b.seat.0 as i64),
                "localPlayer" => {
                    let id = b.seat;
                    drop(b);
                    Value::UserData(lua.create_userdata(Player { id, ctx: c.clone() })?)
                }
                "seed" => Value::Integer(b.seed as i64),
                "isHost" | "allowSave" => Value::Boolean(true),
                "gameSpeed" => Value::Number(1.0),
                _ => {
                    drop(b);
                    stub(lua)?
                }
            })
        })?,
    )?;
    game.set_metatable(Some(gmt))?;
    g.raw_set("Game", game)?;
    Ok(())
}
