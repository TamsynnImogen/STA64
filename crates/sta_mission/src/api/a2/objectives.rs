//! DLL-named objective display controls over host mission state.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{ObjectiveState, Shared};

fn objective_slot(value: &Value) -> mlua::Result<usize> {
    let slot = match value {
        Value::Integer(n) => i32::try_from(*n).ok().filter(|n| *n >= 0),
        Value::Number(n)
            if n.is_finite() && *n >= 0.0 && *n <= f64::from(i32::MAX) && n.fract() == 0.0 =>
        {
            Some(*n as i32)
        }
        _ => None,
    };
    slot.map(|n| n as usize).ok_or_else(|| {
        mlua::Error::runtime("A2.ObjectivesDisplaySetObjectiveComplete requires a nonnegative signed-32-bit integer slot")
    })
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let c = ctx.clone();
    table.set(
        "ObjectivesDisplay_Set_Visibility",
        lua.create_function(move |_, args: MultiValue| {
            if args.len() != 1 {
                return Err(mlua::Error::runtime(
                    "A2.ObjectivesDisplay_Set_Visibility requires exactly one boolean",
                ));
            }
            let Value::Boolean(visible) = &args[0] else {
                return Err(mlua::Error::runtime(
                    "A2.ObjectivesDisplay_Set_Visibility requires exactly one boolean",
                ));
            };
            c.borrow_mut().objectives_visible = *visible;
            Ok(())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "ObjectivesDisplaySetObjectiveComplete",
        lua.create_function(move |_, args: MultiValue| {
            if args.len() != 2 {
                return Err(mlua::Error::runtime(
                    "A2.ObjectivesDisplaySetObjectiveComplete requires exactly two arguments",
                ));
            }
            let slot = objective_slot(&args[0])?;
            let Value::Boolean(complete) = &args[1] else {
                return Err(mlua::Error::runtime(
                    "A2.ObjectivesDisplaySetObjectiveComplete requires a boolean completion value",
                ));
            };
            if let Some(objective) = c.borrow_mut().objectives.get_mut(slot) {
                objective.state = if *complete {
                    ObjectiveState::Complete
                } else {
                    ObjectiveState::Active
                };
            }
            Ok(())
        })?,
    )?;
    let c = ctx.clone();
    table.set(
        "ObjectivesDisplay_Set_Text_From_File",
        lua.create_function(move |_, args: MultiValue| {
            if args.len() != 2 {
                return Err(mlua::Error::runtime(
                    "A2.ObjectivesDisplay_Set_Text_From_File requires exactly two arguments",
                ));
            }
            let Value::String(path) = &args[0] else {
                return Err(mlua::Error::runtime(
                    "A2.ObjectivesDisplay_Set_Text_From_File requires a UTF-8 path string",
                ));
            };
            let Value::Boolean(reset_completed) = &args[1] else {
                return Err(mlua::Error::runtime(
                    "A2.ObjectivesDisplay_Set_Text_From_File requires a boolean resetCompleted value",
                ));
            };
            let path = path.to_str()?;
            crate::api::objectives::load_with_completion(&c, path.as_ref(), *reset_completed)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod completion_tests;

#[cfg(test)]
#[path = "objectives/file_tests.rs"]
mod file_tests;

#[cfg(test)]
mod tests {
    use crate::api::MissionConfig;
    use crate::runtime::MissionRuntime;

    fn runtime(script: &str) -> MissionRuntime {
        MissionRuntime::from_source_with(
            "objectives_visibility.lua",
            script,
            1,
            MissionConfig::default(),
        )
    }

    #[test]
    fn setter_and_objectives_property_share_visibility_without_changing_goals() {
        let mut rt = MissionRuntime::load("missions/T.lua", 1, |path| match path {
            "missions/T.lua" => Some(b"-- synthetic objective visibility fixture\n".to_vec()),
            "objectives/tiny.txt" => {
                Some(b"Tiny mission\nBriefing text.\n*Keep this goal\nNotes.\n".to_vec())
            }
            _ => None,
        });
        rt.eval::<()>("Objectives:load('tiny.txt'); Objectives.objectives[1].state = true")
            .unwrap();
        let objectives_before = rt.objectives();
        let briefing_before = rt.objective_briefing();

        rt.eval::<()>("A2.ObjectivesDisplay_Set_Visibility(false)")
            .unwrap();
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        assert!(!rt.eval::<bool>("return Objectives.visible").unwrap());
        assert_eq!(rt.objectives(), objectives_before);
        assert_eq!(rt.objective_briefing(), briefing_before);

        rt.eval::<()>("A2.ObjectivesDisplay_Set_Visibility(true)")
            .unwrap();
        assert!(rt.eval::<bool>("return Objectives.visible").unwrap());
        assert_eq!(rt.objectives(), objectives_before);
        assert_eq!(rt.objective_briefing(), briefing_before);
        assert!(rt.take_commands().is_empty());
        assert!(rt.unsupported().is_empty());
    }

    #[test]
    fn malformed_calls_preserve_visibility_and_objective_data() {
        let mut rt = runtime(
            r#"
            Objectives.objectives:add('Keep this goal')
            A2.ObjectivesDisplay_Set_Visibility(false)
            local malformed = {
                function() A2.ObjectivesDisplay_Set_Visibility() end,
                function() A2.ObjectivesDisplay_Set_Visibility(false, true) end,
                function() A2.ObjectivesDisplay_Set_Visibility(nil) end,
                function() A2.ObjectivesDisplay_Set_Visibility(0) end,
                function() A2.ObjectivesDisplay_Set_Visibility('false') end,
                function() A2.ObjectivesDisplay_Set_Visibility({}) end
            }
            for _, call in ipairs(malformed) do
                assert(not pcall(call))
                assert(Objectives.visible == false)
            end
            "#,
        );
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        assert!(!rt.eval::<bool>("return Objectives.visible").unwrap());
        assert_eq!(rt.objectives().len(), 1);
        assert_eq!(rt.objectives()[0].title, "Keep this goal");
        assert!(rt.take_commands().is_empty());
        assert!(rt.unsupported().is_empty());
    }

    #[test]
    fn visibility_is_scoped_to_each_runtime() {
        let first = runtime(
            "Objectives.objectives:add('First'); A2.ObjectivesDisplay_Set_Visibility(false)",
        );
        let second = runtime("Objectives.objectives:add('Second')");
        assert!(!first.eval::<bool>("return Objectives.visible").unwrap());
        assert!(second.eval::<bool>("return Objectives.visible").unwrap());
        assert_eq!(first.objectives()[0].title, "First");
        assert_eq!(second.objectives()[0].title, "Second");
        assert!(first.errors().is_empty(), "{:?}", first.errors());
        assert!(second.errors().is_empty(), "{:?}", second.errors());
        assert!(first.unsupported().is_empty());
        assert!(second.unsupported().is_empty());
    }
}
