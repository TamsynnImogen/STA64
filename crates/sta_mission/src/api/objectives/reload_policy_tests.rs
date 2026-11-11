use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::api::{Ctx, MissionConfig, ObjectiveState, Shared};
use crate::runtime::MissionRuntime;

fn files() -> BTreeMap<String, Vec<u8>> {
    [
        (
            "objectives/first.txt",
            b"First heading\nFirst briefing.\n*Alpha\n*Beta\n*Gamma\nFirst notes.\n".to_vec(),
        ),
        (
            "objectives/reordered.txt",
            b"Second heading\nSecond briefing.\n*Gamma\n*New goal\n*Alpha\nSecond notes.\n"
                .to_vec(),
        ),
        (
            "objectives/long.txt",
            b"Long heading\n*One\n*Two\n*Three\n*Four\n".to_vec(),
        ),
        ("objectives/short.txt", b"Short heading\n*Only\n".to_vec()),
        (
            "objectives/empty.txt",
            b"Empty heading\nNo objectives.\n".to_vec(),
        ),
        ("objectives/malformed.txt", b"Broken heading\n*\n".to_vec()),
        ("objectives/encoded.txt", vec![0xff]),
    ]
    .into_iter()
    .map(|(name, bytes)| (name.to_owned(), bytes))
    .collect()
}

fn context() -> Shared {
    let ctx = Rc::new(RefCell::new(Ctx::new(MissionConfig::default(), 3)));
    let contents = files();
    ctx.borrow_mut().reader = Some(Arc::new(Mutex::new(move |path: &str| {
        contents.get(path).cloned()
    })));
    ctx
}

fn state(ctx: &Shared) -> Vec<ObjectiveState> {
    ctx.borrow()
        .objectives
        .iter()
        .map(|goal| goal.state)
        .collect()
}

#[test]
fn false_preserves_completion_by_position_and_true_resets_after_fresh_ids() {
    let ctx = context();
    super::load_with_completion(&ctx, "first.txt", true).unwrap();
    {
        let mut c = ctx.borrow_mut();
        c.objectives[0].state = ObjectiveState::Complete;
        c.objectives[1].state = ObjectiveState::Failed;
        c.objectives[2].state = ObjectiveState::Active;
        c.objectives[0].description = "old description".into();
        c.objectives_visible = false;
    }
    let old_ids: Vec<_> = ctx.borrow().objectives.iter().map(|goal| goal.id).collect();
    assert_eq!(old_ids, [1, 2, 3]);

    super::load_with_completion(&ctx, "reordered.txt", false).unwrap();
    let c = ctx.borrow();
    assert_eq!(
        c.objectives.iter().map(|goal| goal.id).collect::<Vec<_>>(),
        [4, 5, 6]
    );
    assert_eq!(
        c.objectives
            .iter()
            .map(|goal| goal.title.as_str())
            .collect::<Vec<_>>(),
        ["Gamma", "New goal", "Alpha"]
    );
    assert_eq!(
        c.objectives
            .iter()
            .map(|goal| goal.state)
            .collect::<Vec<_>>(),
        [
            ObjectiveState::Complete,
            ObjectiveState::Active,
            ObjectiveState::Active
        ]
    );
    assert!(c.objectives.iter().all(|goal| goal.description.is_empty()));
    assert_eq!(c.next_objective_id, 6);
    assert!(!c.objectives_visible);
    let briefing = c.objective_briefing.as_ref().unwrap();
    assert_eq!(
        (
            briefing.file.as_str(),
            briefing.title.as_str(),
            briefing.text.as_str(),
            briefing.notes.as_str()
        ),
        (
            "objectives/reordered.txt",
            "Second heading",
            "Second briefing.",
            "Second notes."
        )
    );
    assert!(c.commands.is_empty() && c.unsupported.is_empty());
    drop(c);

    super::load_with_completion(&ctx, "first.txt", true).unwrap();
    assert_eq!(state(&ctx), [ObjectiveState::Active; 3]);
    let c = ctx.borrow();
    assert_eq!(
        c.objectives.iter().map(|goal| goal.id).collect::<Vec<_>>(),
        [7, 8, 9]
    );
    assert_eq!(c.next_objective_id, 9);
    assert!(!c.objectives_visible);
    assert!(c.commands.is_empty() && c.unsupported.is_empty());
}

#[test]
fn false_handles_growth_shrink_and_empty_replacements() {
    let ctx = context();
    super::load_with_completion(&ctx, "short.txt", true).unwrap();
    ctx.borrow_mut().objectives[0].state = ObjectiveState::Complete;
    super::load_with_completion(&ctx, "long.txt", false).unwrap();
    assert_eq!(
        state(&ctx),
        [
            ObjectiveState::Complete,
            ObjectiveState::Active,
            ObjectiveState::Active,
            ObjectiveState::Active
        ]
    );
    assert_eq!(
        ctx.borrow()
            .objectives
            .iter()
            .map(|goal| goal.id)
            .collect::<Vec<_>>(),
        [2, 3, 4, 5]
    );

    ctx.borrow_mut().objectives[3].state = ObjectiveState::Complete;
    super::load_with_completion(&ctx, "short.txt", false).unwrap();
    assert_eq!(state(&ctx), [ObjectiveState::Complete]);
    assert_eq!(ctx.borrow().objectives[0].id, 6);

    super::load_with_completion(&ctx, "empty.txt", false).unwrap();
    let c = ctx.borrow();
    assert!(c.objectives.is_empty());
    assert_eq!(c.next_objective_id, 6);
    assert_eq!(
        c.objective_briefing.as_ref().unwrap().title,
        "Empty heading"
    );
    assert!(c.commands.is_empty() && c.unsupported.is_empty());
}

#[test]
fn failures_preserve_failed_state_metadata_visibility_and_allocator() {
    let ctx = context();
    super::load_with_completion(&ctx, "first.txt", true).unwrap();
    {
        let mut c = ctx.borrow_mut();
        c.objectives[0].state = ObjectiveState::Complete;
        c.objectives[1].state = ObjectiveState::Failed;
        c.objectives[1].description = "keep this".into();
        c.objectives_visible = false;
    }
    let before = ctx.borrow().objectives.clone();
    let briefing = ctx.borrow().objective_briefing.clone();
    let next = ctx.borrow().next_objective_id;

    for path in [
        "malformed.txt",
        "encoded.txt",
        "../escape.txt",
        "/absolute.txt",
        "a//b.txt",
        "",
    ] {
        assert!(
            super::load_with_completion(&ctx, path, false).is_err(),
            "{path:?}"
        );
        let c = ctx.borrow();
        assert_eq!(c.objectives, before, "{path:?}");
        assert_eq!(c.objective_briefing, briefing, "{path:?}");
        assert_eq!(c.next_objective_id, next, "{path:?}");
        assert!(!c.objectives_visible);
        assert!(c.commands.is_empty() && c.unsupported.is_empty());
    }

    super::load_with_completion(&ctx, "missing.txt", false).unwrap();
    {
        let c = ctx.borrow();
        assert_eq!(c.objectives, before);
        assert_eq!(c.objective_briefing, briefing);
        assert_eq!(c.next_objective_id, next);
        assert_eq!(c.unsupported.get("Objectives.load"), Some(&1));
        assert!(c.commands.is_empty());
    }

    ctx.borrow_mut().next_objective_id = u32::MAX;
    assert!(super::load_with_completion(&ctx, "short.txt", false).is_err());
    let c = ctx.borrow();
    assert_eq!(c.objectives, before);
    assert_eq!(c.objective_briefing, briefing);
    assert_eq!(c.next_objective_id, u32::MAX);
    assert!(!c.objectives_visible);
    assert_eq!(c.unsupported.get("Objectives.load"), Some(&1));
    assert!(c.commands.is_empty());
}

#[test]
fn generic_lua_load_keeps_default_reset_exact_arity_void_and_stale_handles() {
    let mut contents = files();
    contents.insert("missions/Reload.lua".into(), b"-- synthetic\n".to_vec());
    let mut rt = MissionRuntime::load("missions/Reload.lua", 5, move |path| {
        contents.get(path).cloned()
    });
    rt.eval::<()>(
        r#"
        assert(select('#', Objectives:load('first.txt')) == 0)
        local old = Objectives.objectives[1]
        old.state = true
        Objectives.objectives[2].state = 'failed'
        assert(not pcall(function() Objectives.load('short.txt', false) end))
        assert(not pcall(function() Objectives:load('short.txt', false) end))
        assert(not pcall(function() Objectives:load() end))
        assert(old.state == 'complete' and Objectives.objectives[2].state == 'failed')
        assert(select('#', Objectives.load('short.txt')) == 0)
        assert(old.title == nil)
        old:complete()
        assert(Objectives.objectives[1].state == 'active')
        assert(Objectives.objectives[1].id == 4)
        assert(Objectives.objectives:add('After reload').id == 5)
    "#,
    )
    .unwrap();
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(
        rt.objectives()
            .iter()
            .map(|goal| goal.state)
            .collect::<Vec<_>>(),
        [ObjectiveState::Active, ObjectiveState::Active]
    );
    assert_eq!(rt.objective_briefing().unwrap().title, "Short heading");
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}
