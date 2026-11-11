use std::collections::BTreeMap;

use crate::api::ObjectiveState;
use crate::runtime::MissionRuntime;

fn runtime(script: &str) -> MissionRuntime {
    let files: BTreeMap<String, Vec<u8>> = [
        ("missions/File.lua", script.as_bytes().to_vec()),
        (
            "objectives/first.txt",
            b"First heading\nFirst briefing.\n*Alpha\n*Beta\n*Gamma\nFirst notes.\n".to_vec(),
        ),
        (
            "objectives/second.txt",
            b"Second heading\nSecond briefing.\n*Gamma\n*New goal\n*Alpha\nSecond notes.\n".to_vec(),
        ),
        ("objectives/bad.txt", b"Broken heading\n*\n".to_vec()),
        ("objectives/encoded.txt", vec![0xff]),
    ]
    .into_iter()
    .map(|(name, bytes)| (name.to_owned(), bytes))
    .collect();
    MissionRuntime::load("missions/File.lua", 1, move |path| files.get(path).cloned())
}

#[test]
fn false_preserves_only_completed_ordinals_then_true_resets_with_fresh_ids() {
    let mut rt = runtime(
        r#"
        Objectives:load('first.txt')
        Objectives.objectives[1].state = true
        Objectives.objectives[2].state = 'failed'
        Objectives.objectives[1].description = 'Old detail'
        A2.ObjectivesDisplay_Set_Visibility(false)
        assert(select('#', A2.ObjectivesDisplay_Set_Text_From_File('second.txt', false)) == 0)
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let goals = rt.objectives();
    assert_eq!(goals.iter().map(|g| g.id).collect::<Vec<_>>(), [4, 5, 6]);
    assert_eq!(
        goals.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(),
        ["Gamma", "New goal", "Alpha"]
    );
    assert_eq!(
        goals.iter().map(|g| g.state).collect::<Vec<_>>(),
        [ObjectiveState::Complete, ObjectiveState::Active, ObjectiveState::Active]
    );
    assert!(goals.iter().all(|g| g.description.is_empty()));
    assert_eq!(rt.objective_briefing().unwrap().title, "Second heading");
    assert_eq!(rt.objective_briefing().unwrap().file, "objectives/second.txt");
    assert!(!rt.eval::<bool>("return Objectives.visible").unwrap());

    rt.eval::<()>("assert(select('#', A2.ObjectivesDisplay_Set_Text_From_File('first.txt', true)) == 0)")
        .unwrap();
    let reset = rt.objectives();
    assert_eq!(reset.iter().map(|g| g.id).collect::<Vec<_>>(), [7, 8, 9]);
    assert!(reset.iter().all(|g| g.state == ObjectiveState::Active));
    assert_eq!(rt.objective_briefing().unwrap().title, "First heading");
    assert!(!rt.eval::<bool>("return Objectives.visible").unwrap());
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn missing_malformed_path_and_argument_failures_preserve_host_state() {
    let mut rt = runtime(
        r#"
        Objectives:load('first.txt')
        Objectives.objectives[1].state = true
        Objectives.objectives[2].state = 'failed'
        A2.ObjectivesDisplay_Set_Visibility(false)
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let before = rt.objectives();
    let briefing = rt.objective_briefing();
    rt.eval::<()>(
        r#"
        local bad = {
            function() A2.ObjectivesDisplay_Set_Text_From_File() end,
            function() A2.ObjectivesDisplay_Set_Text_From_File('second.txt') end,
            function() A2.ObjectivesDisplay_Set_Text_From_File('second.txt', false, true) end,
            function() A2:ObjectivesDisplay_Set_Text_From_File('second.txt', false) end,
            function() A2.ObjectivesDisplay_Set_Text_From_File(12, false) end,
            function() A2.ObjectivesDisplay_Set_Text_From_File(nil, false) end,
            function() A2.ObjectivesDisplay_Set_Text_From_File(string.char(255), false) end,
            function() A2.ObjectivesDisplay_Set_Text_From_File('second.txt', 0) end,
            function() A2.ObjectivesDisplay_Set_Text_From_File('second.txt', 'false') end,
            function() A2.ObjectivesDisplay_Set_Text_From_File('../escape.txt', false) end,
            function() A2.ObjectivesDisplay_Set_Text_From_File('/absolute.txt', false) end,
            function() A2.ObjectivesDisplay_Set_Text_From_File('bad.txt', false) end,
            function() A2.ObjectivesDisplay_Set_Text_From_File('encoded.txt', false) end
        }
        for _, call in ipairs(bad) do
            assert(not pcall(call))
            assert(Objectives.objectives[1].state == 'complete')
            assert(Objectives.objectives[2].state == 'failed')
            assert(Objectives.visible == false)
        end
        assert(select('#', A2.ObjectivesDisplay_Set_Text_From_File('missing.txt', false)) == 0)
        "#,
    )
    .unwrap();
    assert_eq!(rt.objectives(), before);
    assert_eq!(rt.objective_briefing(), briefing);
    assert!(!rt.eval::<bool>("return Objectives.visible").unwrap());
    assert_eq!(
        rt.eval::<i64>("return Objectives.objectives:add('After failures').id")
            .unwrap(),
        4
    );
    assert!(rt.take_commands().is_empty());
    assert_eq!(rt.unsupported(), [("Objectives.load".to_owned(), 1)]);
}

#[test]
fn file_reader_and_completion_state_are_isolated_per_runtime() {
    let first =
        runtime("A2.ObjectivesDisplay_Set_Text_From_File('first.txt', false); Objectives.objectives[1].state = true");
    let second = runtime("A2.ObjectivesDisplay_Set_Text_From_File('second.txt', true)");
    assert!(first.errors().is_empty(), "{:?}", first.errors());
    assert!(second.errors().is_empty(), "{:?}", second.errors());
    assert_eq!(first.objectives()[0].title, "Alpha");
    assert_eq!(first.objectives()[0].state, ObjectiveState::Complete);
    assert_eq!(second.objectives()[0].title, "Gamma");
    assert_eq!(second.objectives()[0].state, ObjectiveState::Active);
    assert_eq!(first.objectives()[0].id, 1);
    assert_eq!(second.objectives()[0].id, 1);
    assert_eq!(first.objective_briefing().unwrap().title, "First heading");
    assert_eq!(second.objective_briefing().unwrap().title, "Second heading");
    assert!(first.unsupported().is_empty() && second.unsupported().is_empty());
}
