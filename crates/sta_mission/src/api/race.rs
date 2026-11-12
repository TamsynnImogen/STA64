//! `Race`: `Race.find(name)`, `Race.getAll()` and race values.
//!
//! The races a mission can name are, for now, the distinct non-empty `race` of
//! the loaded unit definitions (the same string `Entity.race` and `Player.race`
//! return). Nothing is hard-coded; with no content the set is empty and `find`
//! returns nil. The authoritative list is `races.odf` (`sta_content::Races`,
//! which also has display names and slots), but `ContentDb` does not keep it
//! and the mission config does not carry it: a race with no unit definition
//! (a script-only race) is therefore not found until that is exposed (a
//! content-side seam, listed in docs/LUA05_COMPLETION.md). `Race.name` is the
//! string; `displayName`, `number`, `boardingStrength` and `repairStrength`
//! are not bound (a race value answers them with the permissive stubs).

use std::collections::BTreeSet;

use mlua::{Lua, MultiValue, UserData, UserDataFields, Value};

use super::Shared;

/// A race value. Comparing two of the same name is true.
#[derive(Clone)]
pub(crate) struct Race(String);

impl UserData for Race {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("name", |_, r| Ok(r.0.clone()));
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(m: &mut M) {
        // `displayName`, `number` and the strengths are not bound: permissive stubs.
        m.add_meta_method(mlua::MetaMethod::Index, |lua, _, _: Value| super::stub(lua));
        m.add_meta_method(mlua::MetaMethod::Eq, |_, a, b: mlua::UserDataRef<Race>| Ok(a.0 == b.0));
        m.add_meta_method(mlua::MetaMethod::ToString, |_, r, ()| Ok(r.0.clone()));
    }
}

/// The distinct races of the content, sorted for a stable order.
fn races(ctx: &Shared) -> Vec<String> {
    let c = ctx.borrow();
    let Some(db) = c.content.as_ref() else {
        return Vec::new();
    };
    db.units().map(|d| d.race.clone()).filter(|r| !r.is_empty()).collect::<BTreeSet<_>>().into_iter().collect()
}

pub(crate) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    let class = lua.create_table()?;
    let c = ctx.clone();
    class.set(
        "find",
        lua.create_function(move |lua, args: MultiValue| {
            // Called as `Race.find(name)`; also tolerate `Race:find(name)`.
            let name = args.iter().rev().find_map(|v| match v {
                Value::String(s) => s.to_str().ok().map(|s| s.to_owned()),
                _ => None,
            });
            let Some(name) = name else {
                return Err(mlua::Error::runtime("Race.find expects a race name"));
            };
            match races(&c).into_iter().find(|r| r.eq_ignore_ascii_case(&name)) {
                Some(r) => Ok(Value::UserData(lua.create_userdata(Race(r))?)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    let c = ctx.clone();
    class.set(
        "getAll",
        lua.create_function(move |lua, _: MultiValue| {
            let all = lua.create_table()?;
            for (i, r) in races(&c).into_iter().enumerate() {
                all.raw_set(i + 1, lua.create_userdata(Race(r))?)?;
            }
            Ok(all)
        })?,
    )?;
    lua.globals().raw_set("Race", class)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::api::MissionConfig;
    use crate::runtime::MissionRuntime;

    fn runtime(src: &str) -> MissionRuntime {
        MissionRuntime::from_source_with(
            "race.lua",
            src,
            1,
            MissionConfig { content: Some(Arc::new(sta_content::fake::db())), ..MissionConfig::default() },
        )
    }

    #[test]
    fn find_and_get_all_come_from_the_content_races() {
        let rt = runtime("");
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        assert!(rt
            .eval::<bool>(
                "local a, b, all = Race.find('FX'), Race.find('nosuchrace'), Race.getAll() \
                 return a ~= nil and a.name == 'fx' and a == Race.find('fx') and b == nil and #all >= 1"
            )
            .unwrap());
        assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    }

    #[test]
    fn no_content_means_no_races() {
        let rt = MissionRuntime::from_source("race.lua", "", 1);
        assert!(rt.eval::<bool>("return Race.find('borg') == nil and #Race.getAll() == 0").unwrap());
    }
}
