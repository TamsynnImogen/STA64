//! Observed condition-monitor call shapes through the public mission runtime.
//! These tests use only synthetic tick views and queued script commands.

use sta_core::{Fixed, PlayerId};
use sta_mission::runtime::MissionRuntime;
use sta_sim::WorldView;
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::Controller;

fn runtime(script: &str) -> MissionRuntime {
    let rt = MissionRuntime::from_source("conditions.lua", script, 5);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn tick(rt: &mut MissionRuntime, at: u64) -> Vec<Command> {
    let view = WorldView {
        tick: at,
        ..WorldView::default()
    };
    let mut commands = Vec::new();
    rt.think(&view, &[], &mut commands);
    commands
}

#[test]
fn initial_true_suppresses_first_true_then_edge_commits_before_callback() {
    let mut rt = runtime(
        r#"
        P = true; FIRED = 0; SEEN = {}
        m = Monitor('edge', true, nil, function(instance, arg)
            assert(instance == m and arg == nil)
            return P
        end, nil)
        m:hook('hit', MonitorHook.Type.OnTrue, nil, function(instance, arg)
            FIRED = FIRED + 1
            SEEN = {instance == m, arg.argument == nil, instance.id, instance.state, instance.hooks}
            Player(0).resources.dilithium = 7
        end, nil)
    "#,
    );
    assert_eq!(tick(&mut rt, 0).len(), 0);
    assert_eq!(rt.eval::<u32>("return FIRED").unwrap(), 0);
    rt.eval::<()>("P=false").unwrap();
    assert!(tick(&mut rt, 1).is_empty());
    assert!(!rt.eval::<bool>("return m.state").unwrap());
    rt.eval::<()>("P=true").unwrap();
    let commands = tick(&mut rt, 2);
    assert_eq!(rt.eval::<u32>("return FIRED").unwrap(), 1);
    let seen: (bool, bool, String, bool, u32) = rt.eval("return SEEN[1],SEEN[2],SEEN[3],SEEN[4],SEEN[5]").unwrap();
    assert_eq!(seen, (true, true, "edge".into(), true, 1));
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].player, PlayerId::SCRIPT);
    assert_eq!(
        commands[0].kind,
        CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(7)))
    );
    assert!(tick(&mut rt, 3).is_empty(), "level true must not refire");
    assert_eq!(rt.eval::<u32>("return FIRED").unwrap(), 1);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn cancellation_and_new_registrations_are_safe_within_a_poll_batch() {
    let mut rt = runtime(
        r#"
        P = false; LOG = {}
        local function put(x) LOG[#LOG+1] = x end
        first = Monitor('first', false, nil, function() return P end, nil)
        other = Monitor('other', false, nil, function() return P end, nil)
        first:hook('one', MonitorHook.Type.OnTrue, nil, function(instance, arg)
            assert(instance == first and arg.argument == 'caller')
            put('first')
            first:unhook('later')
            other:stop()
            late = Monitor('late', false, nil, function() return P end, nil)
            late:hook('late-hook', MonitorHook.Type.OnTrue, nil, function() put('late'); Monitor.stopAll() end, nil)
        end, 'caller')
        first:hook('later', MonitorHook.Type.OnTrue, nil, function() put('cancelled-hook') end, nil)
        other:hook('other-hook', MonitorHook.Type.OnTrue, nil, function() put('cancelled-monitor') end, nil)
    "#,
    );
    rt.eval::<()>("P=true").unwrap();
    assert!(tick(&mut rt, 0).is_empty());
    assert_eq!(rt.eval::<Vec<String>>("return LOG").unwrap(), ["first".to_string()]);
    assert_eq!(rt.eval::<u32>("return first.hooks").unwrap(), 1);
    assert!(tick(&mut rt, 1).is_empty());
    assert_eq!(
        rt.eval::<Vec<String>>("return LOG").unwrap(),
        ["first".to_string(), "late".to_string()]
    );
    assert!(tick(&mut rt, 2).is_empty());
    assert_eq!(
        rt.eval::<Vec<String>>("return LOG").unwrap(),
        ["first".to_string(), "late".to_string()]
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn predicate_and_hook_errors_accumulate_without_stopping_healthy_monitors() {
    let mut rt = runtime(
        r#"
        P=false; BAD_CALLS=0; GOOD_CALLS=0
        bad = Monitor('bad-predicate', false, nil, function() error('predicate boom') end, nil)
        bad:hook('unused', MonitorHook.Type.OnTrue, nil, function() error('must not fire') end, nil)
        flaky = Monitor('flaky', false, nil, function() return P end, nil)
        flaky:hook('sometimes', MonitorHook.Type.OnTrue, nil, function()
            BAD_CALLS=BAD_CALLS+1
            if BAD_CALLS % 2 == 1 then error('hook boom') end
        end, nil)
        healthy = Monitor('healthy', false, nil, function() return P end, nil)
        healthy:hook('ok', MonitorHook.Type.OnTrue, nil, function() GOOD_CALLS=GOOD_CALLS+1 end, nil)
    "#,
    );
    for at in 0..20 {
        rt.eval::<()>(&format!("P={}", at % 2 == 0)).unwrap();
        assert!(tick(&mut rt, at).is_empty());
    }
    let calls: (u32, u32) = rt.eval("return BAD_CALLS, GOOD_CALLS").unwrap();
    assert_eq!(calls, (9, 10), "fifth total hook error disables only that hook");
    assert_eq!(
        rt.errors().len(),
        10,
        "five predicate and five hook errors: {:?}",
        rt.errors()
    );
    assert!(rt.errors().iter().any(|e| e.contains("predicate boom")));
    assert!(rt.errors().iter().any(|e| e.contains("hook boom")));
    assert!(rt.unsupported().is_empty());
}

#[test]
fn malformed_constructor_and_hook_calls_leave_registration_unchanged() {
    let mut rt = runtime(
        "m=Monitor('good',false,nil,function() return false end,nil); m:hook('ok',MonitorHook.Type.OnTrue,nil,function()end,nil)",
    );
    assert_eq!(rt.eval::<u32>("return m.hooks").unwrap(), 1);
    for script in [
        "Monitor('missing',false,nil)",
        "Monitor(1,false,nil,function() return false end,nil)",
        "Monitor('good',false,nil,function() return false end,nil)",
        "Monitor('badbool',0,nil,function() return false end,nil)",
        "Monitor('badslot',false,1,function() return false end,nil)",
        "m:hook('ok',MonitorHook.Type.OnTrue,nil,function()end,nil)",
        "m:hook(1,MonitorHook.Type.OnTrue,nil,function()end,nil)",
        "m:hook('badtype','Unsupported',nil,function()end,nil)",
        "m:hook('badslot',MonitorHook.Type.OnTrue,1,function()end,nil)",
        "m:hook('missing',MonitorHook.Type.OnTrue,nil,function()end)",
    ] {
        assert!(rt.eval::<()>(script).is_err(), "{script}");
        assert_eq!(rt.eval::<u32>("return m.hooks").unwrap(), 1, "{script}");
        assert!(rt.take_commands().is_empty(), "{script} queued a command");
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn predicate_mutations_keep_old_hook_serial_out_of_the_current_edge() {
    let mut rt = runtime(
        r#"
        P=true; CHANGED=false; LOG={}
        m=Monitor('source',false,nil,function(instance)
            if not CHANGED then
                CHANGED=true
                Player(0).resources.dilithium=42
                m:unhook('swap')
                m:hook('swap',MonitorHook.Type.OnTrue,nil,function() LOG[#LOG+1]='replacement' end,nil)
                n=Monitor('new',false,nil,function() return P end,nil)
                n:hook('new-hook',MonitorHook.Type.OnTrue,nil,function() LOG[#LOG+1]='new' end,nil)
            end
            return P
        end,nil)
        m:hook('swap',MonitorHook.Type.OnTrue,nil,function() LOG[#LOG+1]='old' end,nil)
    "#,
    );
    let commands = tick(&mut rt, 0);
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].kind,
        CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(42)))
    );
    assert!(rt.eval::<Vec<String>>("return LOG").unwrap().is_empty());
    assert_eq!(rt.eval::<u32>("return m.hooks").unwrap(), 1);
    assert!(tick(&mut rt, 1).is_empty());
    assert_eq!(rt.eval::<Vec<String>>("return LOG").unwrap(), ["new".to_string()]);
    rt.eval::<()>("P=false").unwrap();
    assert!(tick(&mut rt, 2).is_empty());
    rt.eval::<()>("P=true").unwrap();
    assert!(tick(&mut rt, 3).is_empty());
    assert_eq!(
        rt.eval::<Vec<String>>("return LOG").unwrap(),
        ["new".to_string(), "replacement".to_string(), "new".to_string()]
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn stopped_predicate_error_is_logged_once_and_nonboolean_results_disable_after_five() {
    let mut rt = runtime(
        r#"
        stopped=Monitor('stopped',false,nil,function(instance)
            instance:stop()
            error('stop then fail')
        end,nil)
        nil_result=Monitor('nil-result',false,nil,function() return nil end,nil)
        string_result=Monitor('string-result',false,nil,function() return 'true' end,nil)
    "#,
    );
    for at in 0..5 {
        assert!(tick(&mut rt, at).is_empty());
    }
    assert_eq!(
        rt.errors().len(),
        11,
        "one stopped error plus five of each invalid result: {:?}",
        rt.errors()
    );
    assert!(rt.errors().iter().any(|e| e.contains("stop then fail")));
    assert_eq!(
        rt.errors()
            .iter()
            .filter(|e| e.contains("predicate must return a boolean"))
            .count(),
        10
    );
    assert!(tick(&mut rt, 5).is_empty());
    assert_eq!(rt.errors().len(), 11, "disabled predicates must not restart");
    assert!(rt.unsupported().is_empty());
}

#[test]
fn throwing_hook_does_not_charge_its_replacement_with_the_same_name() {
    let mut rt = runtime(
        r#"
        P=false; OLD=0; NEW=0
        m=Monitor('replace-on-error',false,nil,function() return P end,nil)
        m:hook('swap',MonitorHook.Type.OnTrue,nil,function(instance)
            OLD=OLD+1
            if OLD==5 then
                instance:unhook('swap')
                instance:hook('swap',MonitorHook.Type.OnTrue,nil,function()
                    NEW=NEW+1
                    error('new hook boom')
                end,nil)
            end
            error('old hook boom')
        end,nil)
    "#,
    );
    for at in 0..20 {
        rt.eval::<()>(&format!("P={}", at % 2 == 0)).unwrap();
        assert!(tick(&mut rt, at).is_empty());
    }
    let calls: (u32, u32) = rt.eval("return OLD, NEW").unwrap();
    assert_eq!(calls, (5, 5), "replacement must get its own five-error budget");
    assert_eq!(rt.eval::<u32>("return m.hooks").unwrap(), 1);
    assert_eq!(
        rt.errors().len(),
        10,
        "old and new hooks each fail five times: {:?}",
        rt.errors()
    );
    assert_eq!(rt.errors().iter().filter(|e| e.contains("old hook boom")).count(), 5);
    assert_eq!(rt.errors().iter().filter(|e| e.contains("new hook boom")).count(), 5);
    rt.eval::<()>("P=true").unwrap();
    assert!(tick(&mut rt, 20).is_empty());
    assert_eq!(rt.eval::<(u32, u32)>("return OLD, NEW").unwrap(), (5, 5));
    assert_eq!(rt.errors().len(), 10, "disabled replacement must not restart");
}
