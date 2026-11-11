use std::collections::BTreeMap;

use crate::api::ObjectiveState;
use crate::runtime::MissionRuntime;

fn runtime(script: &str) -> MissionRuntime {
    let files: BTreeMap<String, Vec<u8>> = [
        ("missions/Synthetic.lua", script.as_bytes()),
        (
            "objectives/first.txt",
            b"First heading\nBriefing one.\n*First goal\n*Second goal\nNOTES: first.\n",
        ),
        (
            "objectives/second.txt",
            b"Second heading\nBriefing two.\n*Fresh goal\n*Last goal\nNOTES: second.\n",
        ),
        ("objectives/empty.txt", b"Empty heading\nNo goals.\n"),
    ]
    .into_iter()
    .map(|(name, bytes)| (name.to_owned(), bytes.to_vec()))
    .collect();
    MissionRuntime::load("missions/Synthetic.lua", 1, move |path| {
        files.get(path).cloned()
    })
}

#[test]
fn zero_slot_and_last_slot_change_only_state_and_keep_host_metadata() {
    let mut rt = runtime(
        r#"
        Objectives:load('first.txt')
        Objectives.objectives[1].description = 'First detail'
        Objectives.objectives[2].description = 'Second detail'
        Objectives.objectives[2].state = 'failed'
        A2.ObjectivesDisplay_Set_Visibility(false)
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let before = rt.objectives();
    let briefing = rt.objective_briefing();
    assert_eq!(before.iter().map(|o| o.id).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(before[1].state, ObjectiveState::Failed);

    rt.eval::<()>("A2.ObjectivesDisplaySetObjectiveComplete(0, true)")
        .unwrap();
    assert_eq!(rt.objectives()[0].state, ObjectiveState::Complete);
    assert_eq!(rt.objectives()[1].state, ObjectiveState::Failed);
    rt.eval::<()>("A2.ObjectivesDisplaySetObjectiveComplete(1.0, false)")
        .unwrap();
    assert_eq!(rt.objectives()[1].state, ObjectiveState::Active);
    rt.eval::<()>("A2.ObjectivesDisplaySetObjectiveComplete(1, true); A2.ObjectivesDisplaySetObjectiveComplete(1, false)").unwrap();

    let after = rt.objectives();
    assert_eq!(after[0].state, ObjectiveState::Complete);
    assert_eq!(after[1].state, ObjectiveState::Active);
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(
            (old.id, &old.title, &old.description),
            (new.id, &new.title, &new.description)
        );
    }
    assert_eq!(rt.objective_briefing(), briefing);
    assert!(!rt.eval::<bool>("return Objectives.visible").unwrap());
    assert_eq!(
        rt.eval::<i64>("return Objectives.objectives:add('Third goal').id")
            .unwrap(),
        3
    );
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn slot_is_current_list_ordinal_not_one_based_index_or_old_id() {
    let mut rt = runtime(
        r#"
        Objectives:load('first.txt')
        local old = Objectives.objectives[1]
        Objectives:load('second.txt')
        assert(old.title == nil)
        assert(Objectives.objectives[1].id == 3 and Objectives.objectives[2].id == 4)
        A2.ObjectivesDisplaySetObjectiveComplete(0, true)
        assert(Objectives.objectives[1].state == 'complete')
        assert(Objectives.objectives[2].state == 'active')
        A2.ObjectivesDisplaySetObjectiveComplete(1, true)
        assert(Objectives.objectives[2].state == 'complete')
        A2.ObjectivesDisplaySetObjectiveComplete(3, false)
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let goals = rt.objectives();
    assert_eq!(goals.iter().map(|o| o.id).collect::<Vec<_>>(), [3, 4]);
    assert!(goals.iter().all(|o| o.state == ObjectiveState::Complete));
    assert_eq!(rt.objective_briefing().unwrap().title, "Second heading");
    assert_eq!(
        rt.eval::<i64>("return Objectives.objectives:add('After reload').id")
            .unwrap(),
        5
    );
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn empty_and_out_of_list_nonnegative_slots_are_successful_no_ops() {
    let mut rt = runtime(
        r#"
        A2.ObjectivesDisplaySetObjectiveComplete(0, true)
        A2.ObjectivesDisplaySetObjectiveComplete(2147483647, false)
        Objectives:load('empty.txt')
        A2.ObjectivesDisplaySetObjectiveComplete(0.0, true)
        Objectives:load('first.txt')
        A2.ObjectivesDisplaySetObjectiveComplete(2, true)
        A2.ObjectivesDisplaySetObjectiveComplete(2147483647, true)
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(
        rt.objectives()
            .iter()
            .all(|o| o.state == ObjectiveState::Active)
    );
    assert_eq!(
        rt.eval::<i64>("return Objectives.objectives:add('After no-ops').id")
            .unwrap(),
        3
    );
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn malformed_calls_fail_before_any_mutation() {
    let mut rt = runtime(
        r#"
        Objectives:load('first.txt')
        Objectives.objectives[1].state = 'failed'
        A2.ObjectivesDisplay_Set_Visibility(false)
        local bad = {
            function() A2.ObjectivesDisplaySetObjectiveComplete() end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(0) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(0, true, false) end,
            function() A2:ObjectivesDisplaySetObjectiveComplete(0, true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(-1, true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(2147483648, true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(-1.0, true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(1.5, true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(math.huge, true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(-math.huge, true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(0/0, true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete('0', true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(nil, true) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(0, 1) end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(0, 'true') end,
            function() A2.ObjectivesDisplaySetObjectiveComplete(0, nil) end
        }
        for _, call in ipairs(bad) do
            assert(not pcall(call))
            assert(Objectives.objectives[1].state == 'failed')
            assert(Objectives.objectives[2].state == 'active')
            assert(Objectives.visible == false)
        end
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let goals = rt.objectives();
    assert_eq!(
        (goals[0].state, goals[1].state),
        (ObjectiveState::Failed, ObjectiveState::Active)
    );
    assert_eq!(goals.iter().map(|o| o.id).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(rt.objective_briefing().unwrap().title, "First heading");
    assert_eq!(
        rt.eval::<i64>("return Objectives.objectives:add('After errors').id")
            .unwrap(),
        3
    );
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn objective_completion_is_isolated_per_runtime() {
    let mut first =
        runtime("Objectives:load('first.txt'); A2.ObjectivesDisplaySetObjectiveComplete(0, true)");
    let mut second =
        runtime("Objectives:load('first.txt'); A2.ObjectivesDisplaySetObjectiveComplete(1, true)");
    assert!(first.errors().is_empty(), "{:?}", first.errors());
    assert!(second.errors().is_empty(), "{:?}", second.errors());
    assert_eq!(
        first
            .objectives()
            .iter()
            .map(|o| o.state)
            .collect::<Vec<_>>(),
        [ObjectiveState::Complete, ObjectiveState::Active]
    );
    assert_eq!(
        second
            .objectives()
            .iter()
            .map(|o| o.state)
            .collect::<Vec<_>>(),
        [ObjectiveState::Active, ObjectiveState::Complete]
    );
    assert!(first.take_commands().is_empty() && second.take_commands().is_empty());
    assert!(first.unsupported().is_empty() && second.unsupported().is_empty());
}
