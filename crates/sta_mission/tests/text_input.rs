//! Headless TextInput delivery through public controller ticks and sim events.
//! These tests exercise the STA64 bridge policy, not OS input or native MMM.

use sta_core::{Fixed, PlayerId};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::Controller;
use sta_sim::{SimEvent, WorldView};

fn runtime(source: &str, seat: u8) -> MissionRuntime {
    let cfg = MissionConfig {
        seat: Some(PlayerId(seat)),
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("text-input.lua", source, 13, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn event(player: u8, seq: u32, text: &str) -> SimEvent {
    SimEvent::MissionText {
        player: PlayerId(player),
        seq,
        text: text.into(),
    }
}

fn tick(rt: &mut MissionRuntime, at: u64, events: &[SimEvent]) -> Vec<Command> {
    let view = WorldView {
        tick: at,
        ..WorldView::default()
    };
    let mut commands = Vec::new();
    rt.think(&view, events, &mut commands);
    commands
}

#[test]
fn seat_pattern_nil_argument_and_captured_find_queue_one_command() {
    let mut rt = runtime(
        r#"
        R={name='receiver'}; LOG={}
        TextInput:hook('answer', R, function(self, text, argument)
            assert(self==R and argument==nil)
            LOG[#LOG+1]=text
            Player(0).resources.dilithium=17
        end, nil, '^answer%s+%d+$')
        string.find=function() error('replaced global must not run') end
    "#,
        1,
    );
    let commands = tick(
        &mut rt,
        0,
        &[
            event(0, 1, "answer 42"),
            event(1, 2, "not an answer"),
            event(1, 3, "answer 42"),
            event(PlayerId::SCRIPT.0, 4, "answer 42"),
            event(1, 5, &"x".repeat(sta_sim::command::MAX_MISSION_TEXT_BYTES + 1)),
        ],
    );
    assert_eq!(rt.eval::<Vec<String>>("return LOG").unwrap(), ["answer 42"]);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].player, PlayerId::SCRIPT);
    assert_eq!(
        commands[0].kind,
        CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(17)))
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn event_identity_is_consumed_once_per_tick_and_hooks_run_in_registration_order() {
    let mut rt = runtime(
        r#"
        LOG={}; R={}
        TextInput:hook('a',R,function(_,text,arg) LOG[#LOG+1]='a:'..text..':'..arg end,'A','.')
        TextInput:hook('b',R,function(_,text,arg) LOG[#LOG+1]='b:'..text..':'..arg end,'B','.')
    "#,
        0,
    );
    let events = [event(0, 7, "one"), event(0, 7, "duplicate"), event(0, 8, "two")];
    assert!(tick(&mut rt, 10, &events).is_empty());
    assert!(tick(&mut rt, 10, &events).is_empty());
    assert_eq!(
        rt.eval::<String>("return table.concat(LOG, ',')").unwrap(),
        "a:one:A,b:one:B,a:two:A,b:two:B"
    );
    assert!(tick(&mut rt, 11, &[event(0, 7, "next tick")]).is_empty());
    assert_eq!(rt.eval::<u32>("return #LOG").unwrap(), 6);
}

#[test]
fn batch_snapshot_skips_removed_serial_and_delays_replacement() {
    let mut rt = runtime(
        r#"
        R={}; LOG={}
        TextInput:hook('first',R,function(_,text)
            LOG[#LOG+1]='first:'..text
            if text=='one' then
                TextInput:unhook('later')
                TextInput:hook('later',R,function(_,t) LOG[#LOG+1]='new:'..t end,nil,'.')
            end
        end,nil,'.')
        TextInput:hook('later',R,function(_,text) LOG[#LOG+1]='old:'..text end,nil,'.')
    "#,
        0,
    );
    assert!(tick(&mut rt, 0, &[event(0, 1, "one"), event(0, 2, "two")]).is_empty());
    assert_eq!(
        rt.eval::<String>("return table.concat(LOG, ',')").unwrap(),
        "first:one,first:two"
    );
    assert!(tick(&mut rt, 1, &[event(0, 3, "three")]).is_empty());
    assert_eq!(
        rt.eval::<String>("return table.concat(LOG, ',')").unwrap(),
        "first:one,first:two,first:three,new:three"
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn malformed_registration_is_atomic_and_preflight_catches_immediate_pattern_errors() {
    let mut rt = runtime(
        r#"
        R={}; COUNT=0; function cb() COUNT=COUNT+1 end
        assert(not pcall(function() TextInput:hook('few',R,cb,nil) end))
        assert(not pcall(function() TextInput:hook(3,R,cb,nil,'.') end))
        assert(not pcall(function() TextInput:hook('table',false,cb,nil,'.') end))
        assert(not pcall(function() TextInput:hook('fn',R,false,nil,'.') end))
        assert(not pcall(function() TextInput:hook('type',R,cb,nil,3) end))
        assert(not pcall(function() TextInput:hook('large',R,cb,nil,string.rep('x',4097)) end))
        assert(not pcall(function() TextInput:hook('bad',R,cb,nil,'[') end))
        TextInput:hook('ok',R,cb,nil,'.')
        assert(not pcall(function() TextInput:hook('ok',R,cb,nil,'.') end))
        assert(not pcall(function() TextInput:unhook() end))
        assert(not pcall(function() TextInput:unhook('ok','extra') end))
    "#,
        0,
    );
    tick(&mut rt, 0, &[event(0, 1, "one")]);
    assert_eq!(rt.eval::<u32>("return COUNT").unwrap(), 1);
    rt.eval::<()>("TextInput:unhook('ok')").unwrap();
    tick(&mut rt, 1, &[event(0, 2, "two")]);
    assert_eq!(rt.eval::<u32>("return COUNT").unwrap(), 1);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn deferred_pattern_error_counts_once_per_event_and_disables_only_that_hook() {
    let mut rt = runtime(
        r#"
        R={}; GOOD=0; BAD=0
        TextInput:hook('latebad',R,function() BAD=BAD+1 end,nil,'x[')
        TextInput:hook('healthy',R,function() GOOD=GOOD+1 end,nil,'x')
    "#,
        0,
    );
    for tick_no in 0..6 {
        let input = event(0, tick_no + 1, "x");
        tick(&mut rt, tick_no as u64, std::slice::from_ref(&input));
        if tick_no == 0 {
            assert_eq!(rt.errors().len(), 1);
            tick(&mut rt, tick_no as u64, &[input]);
            assert_eq!(
                rt.errors().len(),
                1,
                "consumed event must not retry after a match error"
            );
        }
    }
    assert_eq!(rt.eval::<(u32, u32)>("return GOOD,BAD").unwrap(), (6, 0));
    assert_eq!(rt.errors().len(), 5, "{:?}", rt.errors());
    assert!(rt.errors().last().unwrap().contains("disabled"));
}

#[test]
fn throwing_replacement_gets_its_own_five_error_budget() {
    let mut rt = runtime(
        r#"
        R={}; OLD=0; NEW=0
        TextInput:hook('same',R,function()
            OLD=OLD+1
            if OLD==5 then
                TextInput:unhook('same')
                TextInput:hook('same',R,function() NEW=NEW+1; error('new failure') end,nil,'.')
            end
            error('old failure')
        end,nil,'.')
    "#,
        0,
    );
    for n in 0..11 {
        tick(&mut rt, n, &[event(0, n as u32 + 1, "text")]);
    }
    assert_eq!(rt.eval::<(u32, u32)>("return OLD,NEW").unwrap(), (5, 5));
    assert_eq!(rt.errors().len(), 10, "{:?}", rt.errors());
    assert!(rt.errors().iter().any(|e| e.contains("old failure")));
    assert!(
        rt.errors()
            .iter()
            .any(|e| e.contains("new failure") && e.contains("disabled"))
    );
}
