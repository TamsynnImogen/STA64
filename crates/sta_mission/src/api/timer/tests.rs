use crate::runtime::MissionRuntime;
use sta_sim::setup::Controller;
use sta_sim::view::WorldView;

fn step(rt: &mut MissionRuntime, tick: u64) {
    let view = WorldView {
        tick,
        ..WorldView::default()
    };
    rt.think(&view, &[], &mut Vec::new());
}

fn eval<T: mlua::FromLuaMulti>(rt: &MissionRuntime, code: &str) -> T {
    rt.eval(code).unwrap()
}

#[test]
fn exact_ticks_lookup_and_one_shot() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        hits = {}; t = Timer('alpha', 0.05, Timer.State.Started)
        t:hook('h', nil, function(timer, value) hits[#hits+1] = {timer.id, value} end, nil)
    "#,
        1,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(eval::<bool>(&rt, "return Timer.find('alpha').id == 'alpha' and Timer.find('missing') == nil and #Timer.getAll() == 1"));
    step(&mut rt, 0);
    step(&mut rt, 1);
    assert_eq!(eval::<i64>(&rt, "return #hits"), 0);
    step(&mut rt, 2); // 0.05 * 30 = 1.5 -> two ticks
    step(&mut rt, 3);
    assert!(eval::<bool>(&rt, "return #hits == 1 and hits[1][1] == 'alpha' and hits[1][2] == 'h' and t.state == 'Stopped'"));
}

#[test]
fn repeats_restart_stop_all_and_interval_reschedule() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        count = 0; t = Timer('r', 2/30, Timer.State.Started); t.repeats = true
        t:hook('h', nil, function() count = count + 1 end, nil)
    "#,
        1,
    );
    for tick in 0..=4 {
        step(&mut rt, tick);
    }
    assert_eq!(eval::<i64>(&rt, "return count"), 2);
    eval::<()>(&rt, "t:stop(); t:start()");
    step(&mut rt, 5);
    assert_eq!(eval::<i64>(&rt, "return count"), 2);
    eval::<()>(&rt, "t.interval = 1/30"); // active interval reset at tick 5
    step(&mut rt, 6);
    assert_eq!(eval::<i64>(&rt, "return count"), 3);
    eval::<()>(&rt, "Timer.stopAll()");
    for tick in 7..=10 {
        step(&mut rt, tick);
    }
    assert_eq!(eval::<i64>(&rt, "return count"), 3);
    assert!(eval::<bool>(&rt, "return t.state == Timer.State.Stopped"));
}

#[test]
fn callback_can_mutate_timer_store_and_preserve_data() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        output = {}; data = {tag='kept'}
        first = Timer('first', 1/30, Timer.State.Started)
        second = Timer('second', 1/30, Timer.State.Started)
        first:hook('f', nil, function(timer, value)
            output[#output+1] = value.tag
            second:stop()
            later = Timer('later', 1/30, Timer.State.Started)
            later:hook('l', nil, function() output[#output+1] = 'later' end, nil)
        end, data)
        second:hook('s', nil, function() output[#output+1] = 'second' end, nil)
    "#,
        1,
    );
    step(&mut rt, 0);
    step(&mut rt, 1);
    step(&mut rt, 2);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(eval::<bool>(&rt, "return #output == 2 and output[1] == 'kept' and output[2] == 'later' and #Timer.getAll() == 3"));
    eval::<()>(&rt, "later:unhook('l'); later:start()");
    step(&mut rt, 3);
    assert_eq!(eval::<i64>(&rt, "return #output"), 2);
}

#[test]
fn errors_are_isolated_and_invalid_calls_do_not_mutate() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        good = 0; t = Timer('bad', 1/30, Timer.State.Started); t.repeats = true
        t:hook('bad', nil, function() error('timer boom') end, nil)
        t:hook('good', nil, function() good = good + 1 end, nil)
    "#,
        1,
    );
    assert!(eval::<bool>(
        &rt,
        "return not pcall(function() Timer('bad', 1) end) and #Timer.getAll() == 1"
    ));
    assert!(eval::<bool>(
        &rt,
        "return not pcall(function() t:hook('x', true, function() end, nil) end) and t.hooks == 2"
    ));
    assert!(eval::<bool>(
        &rt,
        "return not pcall(function() t.interval = 0/0 end) and t.interval == 1/30"
    ));
    assert!(eval::<bool>(
        &rt,
        "return not pcall(function() t.repeats = 'yes' end) and t.repeats"
    ));
    for tick in 0..=7 {
        step(&mut rt, tick);
    }
    assert_eq!(eval::<i64>(&rt, "return good"), 7);
    assert_eq!(rt.errors().len(), 5);
    assert!(rt.errors()[0].contains("timer boom"));
    assert!(rt.errors()[4].contains("disabled"));
}

#[test]
fn runtime_stores_are_separate() {
    let a = MissionRuntime::from_source("a.lua", "t = Timer('same', 1)", 1);
    let b = MissionRuntime::from_source("b.lua", "t = Timer('same', 1)", 2);
    assert!(a.errors().is_empty() && b.errors().is_empty());
    assert_eq!(eval::<i64>(&a, "return #Timer.getAll()"), 1);
    assert_eq!(eval::<i64>(&b, "return #Timer.getAll()"), 1);
}

#[test]
fn callable_table_constructor_uses_user_arguments() {
    let rt =
        MissionRuntime::from_source("T.lua", "t = Timer('named', 1/30, Timer.State.Started)", 1);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(eval::<bool>(
        &rt,
        "return t.id == 'named' and t.state == 'Started'"
    ));
}

#[test]
fn throwing_callback_that_unhooks_itself_does_not_penalize_sibling() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
            good = 0; t = Timer('r', 1/30, Timer.State.Started); t.repeats = true
            t:hook('bad', nil, function() t:unhook('bad'); error('removed boom') end, nil)
            t:hook('good', nil, function() good = good + 1 end, nil)
        "#,
        1,
    );
    step(&mut rt, 0);
    step(&mut rt, 1);
    step(&mut rt, 2);
    assert_eq!(rt.errors().len(), 1);
    assert!(rt.errors()[0].contains("removed boom"));
    assert_eq!(eval::<i64>(&rt, "return good"), 1);
    assert_eq!(eval::<i64>(&rt, "return t.hooks"), 1);
}

#[test]
fn self_restart_then_throw_still_exhausts_error_budget() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
            calls = 0; t = Timer('r', 1/30, Timer.State.Started)
            t:hook('h', nil, function()
                calls = calls + 1
                t:start()
                error('restart boom')
            end, nil)
        "#,
        1,
    );
    for tick in 0..=8 {
        step(&mut rt, tick);
    }
    assert_eq!(eval::<i64>(&rt, "return calls"), 5);
    assert_eq!(rt.errors().len(), 5);
    assert!(rt.errors()[4].contains("disabled"));
}

#[test]
fn replacement_hook_does_not_inherit_removed_hook_error() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
            calls = 0; t = Timer('r', 1/30, Timer.State.Started); t.repeats = true
            t:hook('h', nil, function()
                t:unhook('h')
                t:hook('h', nil, function() calls = calls + 1 end, nil)
                error('old hook boom')
            end, nil)
        "#,
        1,
    );
    for tick in 0..=7 {
        step(&mut rt, tick);
    }
    assert_eq!(rt.errors().len(), 1);
    assert_eq!(eval::<i64>(&rt, "return calls"), 6);
}

#[test]
fn tick_overflow_and_huge_intervals_reject_without_mutating() {
    let mut rt = MissionRuntime::from_source("T.lua", "t = Timer('r', 2/30)", 1);
    assert!(eval::<bool>(
        &rt,
        "return not pcall(function() Timer('huge', 1e100) end) and #Timer.getAll() == 1"
    ));
    step(&mut rt, u64::MAX - 1);
    assert!(eval::<bool>(
        &rt,
        "return not pcall(function() t:start() end) and t.state == 'Stopped'"
    ));
    assert!(eval::<bool>(
        &rt,
        "return not pcall(function() t.interval = 1e100 end) and t.interval == 2/30"
    ));
}
