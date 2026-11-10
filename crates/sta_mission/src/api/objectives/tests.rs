use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::api::{Ctx, MissionConfig, ObjectiveState};
use crate::runtime::MissionRuntime;

fn runtime(script: &str) -> MissionRuntime {
    let files: BTreeMap<String, Vec<u8>> = [
        ("missions/T.lua", script.as_bytes().to_vec()),
        ("objectives/one.txt", b"Test heading\r\nBriefing paragraph.\r\n\r\n*Protect the beacon\r\n*Return safely\r\n\r\nHINT: stay together\r\n".to_vec()),
        ("objectives/two.txt", b"Next heading\n*Survey the field\n".to_vec()),
        ("objectives/empty.txt", b"No briefing\n".to_vec()),
        ("objectives/bad.txt", b"Broken heading\n*\n".to_vec()),
        ("objectives/encoded.txt", vec![0xff]),
    ].into_iter().map(|(k,v)| (k.to_owned(),v)).collect();
    MissionRuntime::load("missions/T.lua", 1, move |path| files.get(path).cloned())
}

#[test]
fn loader_populates_goals_metadata_and_numbered_state_updates() {
    let rt = runtime(
        r#"
        Objectives:load('one.txt')
        assert(Objectives.count == 2 and #Objectives.objectives == 2)
        assert(Objectives.objectives[1].title == 'Protect the beacon')
        assert(Objectives.objectives[2.0].title == 'Return safely')
        Objectives.objectives[1].state = true
        Objectives.objectives[2].state = 'failed'
        assert(Objectives.objectives[1].state == 'complete')
        Objectives.objectives[1].state = false
        assert(Objectives.objectives[1].state == 'active')
        assert(Objectives.objectives[0] == nil and Objectives.objectives[-1] == nil)
        assert(Objectives.objectives[1.5] == nil)
        assert(Objectives.objectives:get(math.mininteger) == nil)
        assert(Objectives.objectives:get(math.maxinteger) == nil)
    "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
    assert_eq!(rt.objectives()[1].state, ObjectiveState::Failed);
    let b = rt.objective_briefing().unwrap();
    assert_eq!(b.file, "objectives/one.txt");
    assert_eq!(b.title, "Test heading");
    assert_eq!(b.text, "Briefing paragraph.");
    assert_eq!(b.notes, "HINT: stay together");
}

#[test]
fn reloading_cannot_rebind_old_handles_and_dot_load_is_supported() {
    let rt = runtime(
        r#"
        Objectives:load('one.txt')
        OLD = Objectives.objectives[1]
        Objectives.load('two.txt')
        assert(Objectives.count == 1 and Objectives.objectives[1].id == 3)
        assert(OLD.title == nil)
        OLD:complete()
        OLD.state = true
        assert(Objectives.objectives[1].state == 'active')
        local added = Objectives.objectives:add('Another goal')
        assert(added.id == 4)
        Objectives:load('empty.txt')
        assert(Objectives.count == 0)
        assert(Objectives.objectives:add('Fresh goal').id == 5)
    "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(rt.objectives()[0].title, "Fresh goal");
    assert!(rt.unsupported().is_empty());
}

#[test]
fn malformed_missing_or_invalid_loads_preserve_existing_goals() {
    let rt = runtime(
        r#"
        Objectives:load('one.txt')
        local old = Objectives.objectives[1].id
        assert(not pcall(function() Objectives:load('bad.txt') end))
        assert(not pcall(function() Objectives:load('encoded.txt') end))
        for _, name in ipairs({'../escape.txt', '/absolute.txt', 'C:\\file.txt', '', 'a//b.txt'}) do
            assert(not pcall(function() Objectives:load(name) end))
        end
        assert(not pcall(function() Objectives:load('one.txt', 3) end))
        assert(not pcall(function() Objectives:load(42) end))
        assert(not pcall(function() Objectives.objectives[1].state = 'not-a-state' end))
        Objectives:load('missing.txt')
        assert(Objectives.count == 2 and Objectives.objectives[1].id == old)
        assert(Objectives.objectives[1].state == 'active')
    "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(rt.unsupported(), [("Objectives.load".to_owned(), 1)]);
    assert_eq!(rt.objective_briefing().unwrap().title, "Test heading");
}

#[test]
fn objective_id_exhaustion_does_not_partly_replace_a_loaded_list() {
    let ctx = Rc::new(RefCell::new(Ctx::new(MissionConfig::default(), 1)));
    {
        let mut c = ctx.borrow_mut();
        c.next_objective_id = u32::MAX;
        c.reader = Some(Arc::new(Mutex::new(|_: &str| Some(b"Test\n*Goal\n".to_vec()))));
    }
    assert!(super::load(&ctx, "test.txt").is_err());
    assert_eq!(ctx.borrow().next_objective_id, u32::MAX);
    assert!(ctx.borrow().objectives.is_empty());
    assert!(ctx.borrow().objective_briefing.is_none());
}
