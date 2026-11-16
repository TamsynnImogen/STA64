//! Timer id reuse (LUA-42), against authored scripts: an id whose timer has expired or been
//! stopped is reusable; an id still running keeps the error; a script error raised in a hook that
//! began a non-skippable cineractive is reported, and a later end of the mission is still reachable
//! once the ids are free.

use sta_mission::runtime::MissionRuntime;
use sta_sim::setup::Controller;
use sta_sim::view::WorldView;

fn step(rt: &mut MissionRuntime, tick: u64) {
    let view = WorldView { tick, ..WorldView::default() };
    rt.think(&view, &[], &mut Vec::new());
}

fn run(rt: &mut MissionRuntime, from: u64, to: u64) {
    for t in from..=to {
        step(rt, t);
    }
}

#[test]
fn an_expired_id_is_reusable_and_fires_its_new_hook_only() {
    let mut rt = MissionRuntime::from_source(
        "authored-reuse-expiry.lua",
        r#"
        log = {}
        a = Timer('shared', 0.1, Timer.State.Started)
        a:hook('first', nil, function() log[#log + 1] = 'old' end, nil)
        "#,
        3,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    run(&mut rt, 0, 5);
    assert_eq!(rt.eval::<String>("return table.concat(log, ',')").unwrap(), "old");
    // The id has expired: a new timer of the same id is allowed and carries only its own hook.
    rt.eval::<()>("b = Timer('shared', 0.1, Timer.State.Started); b:hook('first', nil, function() log[#log + 1] = 'new' end, nil)")
        .unwrap();
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    run(&mut rt, 6, 12);
    assert_eq!(rt.eval::<String>("return table.concat(log, ',')").unwrap(), "old,new");
    assert_eq!(rt.eval::<i64>("return #Timer.getAll()").unwrap(), 1);
}

#[test]
fn a_stopped_id_is_reusable_and_the_old_deadline_is_gone() {
    let mut rt = MissionRuntime::from_source(
        "authored-reuse-stop.lua",
        r#"
        log = {}
        a = Timer('held', 0.2, Timer.State.Started)
        a:hook('h', nil, function() log[#log + 1] = 'old' end, nil)
        a:stop()
        b = Timer('held', 0.1, Timer.State.Started)
        b:hook('h', nil, function() log[#log + 1] = 'new' end, nil)
        "#,
        3,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    run(&mut rt, 0, 20);
    assert_eq!(rt.eval::<String>("return table.concat(log, ',')").unwrap(), "new");
    // `stopAll` frees every id too.
    rt.eval::<()>("c = Timer('long', 100, Timer.State.Started); Timer.stopAll(); d = Timer('long', 1, Timer.State.Stopped)")
        .unwrap();
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn a_running_id_still_raises_duplicate_id() {
    let mut rt = MissionRuntime::from_source(
        "authored-reuse-running.lua",
        r#"
        a = Timer('busy', 100, Timer.State.Started)
        ok, err = pcall(function() return Timer('busy', 1, Timer.State.Started) end)
        "#,
        3,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(!rt.eval::<bool>("return ok").unwrap());
    assert!(rt.eval::<String>("return tostring(err)").unwrap().contains("duplicate id"));
    // The running timer is untouched.
    assert_eq!(rt.eval::<String>("return a.state").unwrap(), "Started");
}

/// The shape of a stock win: an earlier hook uses ids, a later hook (a monitor's) begins a non-skippable
/// cineractive and then builds timers under the same ids, the last of which ends the mission. Before the
/// fix the later hook raised after the cineractive began and the mission could never end.
#[test]
fn a_late_end_sequence_reusing_ids_still_ends_the_mission() {
    let mut rt = MissionRuntime::from_source(
        "authored-late-end.lua",
        r#"
        early = Timer('fade', 0.1, Timer.State.Started)
        early:hook('h', nil, function() fired_early = true end, nil)
        function finish_sequence()
            Cineractive.active = true
            Cineractive.skippable = false
            local t = Timer('fade', 0.2, Timer.State.Started)
            t:hook('h', nil, function() Cineractive.active = false; Mission:finish(true, 0) end, nil)
        end
        "#,
        3,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    run(&mut rt, 0, 10);
    assert!(rt.eval::<bool>("return fired_early == true").unwrap());
    rt.eval::<()>("finish_sequence()").unwrap();
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    run(&mut rt, 11, 30);
    assert!(rt.finished(), "the late sequence did not reach Mission:finish");
}

/// A raised error in a hook that began a non-skippable cineractive is never silent: the runtime
/// lists it (the client shows a notice and logs it, `mission_host::run_mission`).
#[test]
fn a_hook_error_after_a_non_skippable_cineractive_is_reported() {
    let mut rt = MissionRuntime::from_source(
        "authored-hook-error-monitor.lua",
        r#"
        t = Timer('x', 100, Timer.State.Started)
        m = Monitor('m', false, nil, function() return true end, nil)
        m:hook('h', MonitorHook.Type.OnTrue, nil, function()
            Cineractive.active = true
            Cineractive.skippable = false
            Timer('x', 1, Timer.State.Started)
        end, nil)
        "#,
        3,
    );
    run(&mut rt, 0, 3);
    assert!(
        rt.errors().iter().any(|e| e.contains("duplicate id")),
        "the hook error was not listed: {:?}",
        rt.errors()
    );
}
