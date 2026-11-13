//! `Player`, relations, resources, `Game` and objectives.

use mlua::{Lua, MetaMethod, MultiValue, UserData, UserDataFields, UserDataMethods, UserDataRef, Value};
use sta_core::{Fixed, PlayerId};
use sta_sim::command::Edit;
use sta_sim::view::Relation;
use sta_sim::TICK_HZ;

use super::{stub, Ctx, Objective, ObjectiveState, PlayerNumbering, Shared};

/// A seat. Reads see the latest view.
pub(crate) struct Player {
    pub id: PlayerId,
    pub ctx: Shared,
}

/// A player from a number or a `Player` handle.
pub(crate) fn player_of(ctx: &Ctx, v: &Value) -> Option<PlayerId> {
    match v {
        Value::Integer(n) => u8::try_from(*n).ok().and_then(|n| ctx.player_numbering.decode(n)),
        Value::Number(n) if *n >= 0.0 && *n < 256.0
            && (ctx.player_numbering == PlayerNumbering::Slots || n.fract() == 0.0) => ctx.player_numbering.decode(*n as u8),
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
                // SIM-54: the crew stock.
                None if k == "crew" && v.is_finite() => {
                    super::edit_feedback::queue(&mut c, "Player.resources.crew", Edit::TeamCrew(id, Fixed::from_f64(v)))?;
                }
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
        f.add_field_method_get("number", |_, p| Ok(p.ctx.borrow().player_numbering.encode(p.id)));
        f.add_field_method_get("name", |_, p| Ok(format!("Player {}", p.ctx.borrow().player_numbering.encode(p.id).unwrap_or(p.id.0))));
        f.add_field_method_get("race", |_, p| {
            // The race of the player's first unit that has one.
            let c = p.ctx.borrow();
            if let Ok(i) = c.view.attrs.player_races.binary_search_by_key(&p.id, |r| r.0) {
                return Ok(c.view.attrs.player_races[i].1.clone());
            }
            Ok(c.view.units.iter().filter(|u| u.owner == p.id).find_map(|u| {
                c.def(u).map(|d| d.race.clone()).filter(|r| !r.is_empty())
            }).unwrap_or_default())
        });
        // Cosmetic or rule-level player settings with no sim state: accepted and
        // recorded (a seat's race and display data come from the game setup).
        for (name, need) in [
            ("color", "a player-colour setting in the game setup"),
            ("name", "a player-name setting in the game setup"),
        ] {
            f.add_field_method_set(name, move |_, p, _: Value| {
                p.ctx.borrow_mut().unsupported(&format!("Player.{name} = x (needs {need})"));
                Ok(())
            });
        }
        // `Player.race = Race.find(...)` (SIM-54): the seat's race, kept and shown by the sim; the sim
        // refuses a name no unit carries. Rules that read a seat's race (build interface, forced
        // setup race of the local seat) are not modelled: not measured beyond the executable, static.
        f.add_field_method_set("race", |_, p, value: Value| {
            let mut c = p.ctx.borrow_mut();
            match super::race::race_name(&value) {
                Some(name) => super::edit_feedback::queue(&mut c, "Player.race", Edit::PlayerRace(p.id, name))?,
                None => c.unsupported("Player.race = non-race"),
            }
            Ok(())
        });
        f.add_field_method_get("isAI", |_, _| Ok(false));
        f.add_field_method_get("slotType", |_, p| {
            Ok(if p.ctx.borrow().seat == p.id { "Human" } else { "None" })
        });
        for name in ["maxOfficers", "maxCrew"] {
            f.add_field_method_get(name, |_, p| {
                Ok(p.ctx.borrow().view.player(p.id).map_or(0.0, |v| v.officers[1].to_f32() as f64))
            });
            // The limit is a typed rule (RTS_CFG.h SINGLE_PLAYER_OFFICER_LIMIT /
            // MP_OFFICER_LIMIT) held by the economy; the sim has no edit to
            // change it per player. Assigning the limit already in force changes
            // nothing (installed missions assign 199999, the unitcap_normal /
            // SINGLE_PLAYER_OFFICER_LIMIT value); any other value is recorded as
            // unsupported, never pretended. A headless setup without a chosen
            // limit reports 0, so there the call is recorded too.
            f.add_field_method_set(name, move |_, p, value: f64| {
                let mut c = p.ctx.borrow_mut();
                let current = c.view.player(p.id).map(|v| v.officers[1].to_f32() as f64);
                if !value.is_finite() {
                    return Err(mlua::Error::runtime(format!("Player.{name} requires a finite number")));
                }
                if name == "maxOfficers" {
                    // SIM-54: the officer limit edit; assigning the limit already in
                    // force changes nothing and is not sent. A seat with no officer
                    // limit is rejected by the sim and counted by the edit feedback.
                    if current != Some(value) {
                        super::edit_feedback::queue(&mut c, "Player.maxOfficers", Edit::MaxOfficers(p.id, Fixed::from_f64(value)))?;
                    }
                } else if current != Some(value) {
                    c.unsupported(&format!("Player.{name} = x (differs from the rule; needs a sim officer-limit edit)"));
                }
                Ok(())
            });
        }
        f.add_field_method_get("resources", resources);
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("getRelation", |_, p, other: Value| {
            let c = p.ctx.borrow();
            Ok(match player_of(&c, &other) {
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
            if let Some(o) = player_of(&c, &other) {
                c.relations.insert((p.id, o), rel);
            }
            Ok(())
        });
        m.add_method("loadAIP", |_, p, name: Option<String>| {
            let mut c = p.ctx.borrow_mut();
            c.loaded_aips.push((p.id, name.unwrap_or_default()));
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
        f.add_field_method_set("state", |_, o, value: Value| {
            let state = match value {
                Value::Boolean(true) => ObjectiveState::Complete,
                Value::Boolean(false) => ObjectiveState::Active,
                Value::String(s) => match s.to_str()?.as_ref() {
                    "active" => ObjectiveState::Active,
                    "complete" => ObjectiveState::Complete,
                    "failed" => ObjectiveState::Failed,
                    _ => return Err(mlua::Error::runtime("unknown objective state")),
                },
                _ => return Err(mlua::Error::runtime("objective state must be boolean or state name")),
            };
            let _ = o.with(|o| o.state = state);
            Ok(())
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

fn objective_at(lua: &Lua, ctx: &Shared, index: i64) -> mlua::Result<Value> {
    let id = index.checked_sub(1).and_then(|i| usize::try_from(i).ok())
        .and_then(|i| ctx.borrow().objectives.get(i).map(|o| o.id));
    match id {
        Some(id) => Ok(Value::UserData(lua.create_userdata(ObjectiveHandle { id, ctx: ctx.clone() })?)),
        None => Ok(Value::Nil),
    }
}

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
            let id = c.next_objective_id.checked_add(1).ok_or_else(|| mlua::Error::runtime("objective ids exhausted"))?;
            c.next_objective_id = id;
            c.objectives.push(Objective { id, title, description, state: ObjectiveState::Active });
            drop(c);
            lua.create_userdata(ObjectiveHandle { id, ctx: l.0.clone() })
        });
        m.add_method("get", |lua, l, i: i64| objective_at(lua, &l.0, i));
        m.add_meta_method(MetaMethod::Len, |_, l, ()| Ok(l.0.borrow().objectives.len()));
        m.add_meta_method(MetaMethod::Index, |lua, l, key: Value| match key {
            Value::Integer(i) => objective_at(lua, &l.0, i),
            Value::Number(n) if n.is_finite() && n.fract() == 0.0 => objective_at(lua, &l.0, n as i64),
            Value::Number(_) => Ok(Value::Nil),
            _ => stub(lua),
        });
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
        m.add_function("load", |lua, mut args: MultiValue| {
            let ctx = args.front().and_then(|v| v.as_userdata())
                .and_then(|u| u.borrow::<Objectives>().ok()).map(|o| o.0.clone());
            let ctx = match ctx {
                Some(c) => { args.pop_front(); c },
                None => lua.globals().raw_get::<mlua::AnyUserData>("Objectives")?.borrow::<Objectives>()?.0.clone(),
            };
            if args.len() != 1 { return Err(mlua::Error::runtime("Objectives.load expects one file name")); }
            match args.front() {
                Some(Value::String(name)) => super::objectives::load(&ctx, name.to_str()?.as_ref()),
                _ => Err(mlua::Error::runtime("Objectives.load expects a file name")),
            }
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
        let n = { let c = c.borrow(); args.iter().rev().find_map(|v| player_of(&c, v)) };
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
