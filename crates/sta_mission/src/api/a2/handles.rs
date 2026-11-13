//! `GetHandle(label)`: how a mission names a pre-placed map object.
//!
//! # The handle model
//!
//! A converted mission's `ScriptInit` resolves each object it refers to by the
//! label the map gives it (`GetHandle("Enterprise")`) and keeps the integer. The
//! label is the object's `label` in the map file; here it is the host's label
//! map (`MissionConfig::labels`, filled from the map's object labels when the
//! sim is built). Rules, read from the executable (static): the lookup walks
//! the live object list and returns the handle of the first object whose label
//! is byte-for-byte equal to the argument (a case-sensitive string compare, no
//! trimming); when none matches (or the object is no longer in the list) it
//! returns the invalid handle, which is 0. An invalid handle in a later call is
//! "no such object": setters do nothing and return 0, queries answer false or
//! 0 (the object guard in every handle-taking call). Which object wins when
//! two share a label follows the engine's list order, not measured; here the
//! lowest entity id. Evidence: first-match, case-sensitive and 0 for none are
//! read from the executable, static; label data is the map's (read from data).

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::Shared;

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let c = ctx.clone();
    table.set(
        "GetHandle",
        lua.create_function(move |_, args: MultiValue| {
            let [Value::String(label)] = &args.iter().cloned().collect::<Vec<_>>()[..] else {
                return Err(mlua::Error::runtime("A2.GetHandle requires exactly one label string"));
            };
            let label = label.to_str()?;
            let c = c.borrow();
            let id = c
                .labels
                .iter()
                .find(|(id, l)| l.as_str() == &*label && c.unit(**id).is_some())
                .map_or(0, |(id, _)| i64::from(id.0));
            Ok(id)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sta_core::EntityId;

    use crate::api::MissionConfig;
    use crate::runtime::MissionRuntime;

    fn runtime(labels: &[(u32, &str)]) -> MissionRuntime {
        let labels: BTreeMap<EntityId, String> = labels.iter().map(|(i, l)| (EntityId(*i), (*l).to_owned())).collect();
        let rt = MissionRuntime::from_source_with("h.lua", "", 1, MissionConfig { labels, ..MissionConfig::default() });
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        rt
    }

    #[test]
    fn a_label_is_found_exactly_and_first_when_the_object_is_in_the_view() {
        use sta_core::{Fixed, Vec3};
        use sta_sim::{MoveStats, Sim, WorldSource};
        let mut sim = Sim::new(1);
        let mut spawn = |x: f32| sim.spawn(0, Vec3::from_f32([x, 0.0, 0.0]), (Fixed::ZERO, Fixed::ONE), MoveStats::from_f32(10., 1., 1., 1., 0.2));
        let (a, b) = (spawn(0.0), spawn(10.0));
        let labels: BTreeMap<EntityId, String> =
            [(a, "Hero"), (b, "Hero"), (EntityId(999), "Ghost")].into_iter().map(|(i, l)| (i, l.to_owned())).collect();
        let cfg = MissionConfig { labels, view: Some((*sim.view()).clone()), ..MissionConfig::default() };
        let rt = MissionRuntime::from_source_with("h.lua", "", 1, cfg);
        assert_eq!(rt.eval::<i64>("return A2.GetHandle('Hero')").unwrap(), i64::from(a.0), "first match wins");
        assert_eq!(rt.eval::<i64>("return A2.GetHandle('hero')").unwrap(), 0, "case-sensitive");
        assert_eq!(rt.eval::<i64>("return A2.GetHandle('Ghost')").unwrap(), 0, "not in the object list");
    }

    #[test]
    fn unknown_labels_and_objects_gone_from_the_view_are_the_invalid_handle() {
        // The default view holds no units: every label names an object that is not in the list.
        let rt = runtime(&[(7, "Hero")]);
        assert_eq!(rt.eval::<i64>("return A2.GetHandle('Hero')").unwrap(), 0);
        assert_eq!(rt.eval::<i64>("return A2.GetHandle('nobody')").unwrap(), 0);
        for bad in ["", "1", "'a', 'b'", "nil"] {
            assert!(rt.eval::<()>(&format!("A2.GetHandle({bad})")).is_err(), "{bad}");
        }
    }
}
