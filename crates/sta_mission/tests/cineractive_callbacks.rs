//! Observed mission-object cinematic callbacks through the public runtime.
//! Timing assertions describe the STA64 bridge policy, not native parity.

use sta_core::{Fixed, PlayerId};
use sta_mission::runtime::MissionRuntime;
use sta_sim::WorldView;
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::Controller;

fn runtime(script: &str) -> MissionRuntime {
    let rt = MissionRuntime::from_source("cinematic.lua", script, 7);
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
fn setup_update_cinematic_update_and_finish_keep_order_and_restart_next_tick() {
    let mut rt = runtime(
        r#"
        LOG={}; U=0; CU=0
        object={
            setup=function(self) LOG[#LOG+1]='setup'; Cineractive.active=true end,
            update=function(self)
                U=U+1; LOG[#LOG+1]='u'
                Player(0).resources.dilithium=U
                if U==2 then self.cineractiveUpdate=function() LOG[#LOG+1]='new-cu' end end
            end,
            cineractiveUpdate=function(self)
                CU=CU+1; LOG[#LOG+1]='old-cu'
                if CU==2 then Cineractive.active=false end
            end,
            cineractiveFinish=function(self, skipped)
                LOG[#LOG+1]='f:'..tostring(skipped)
                Cineractive.active=true
            end,
        }
        MMM.register(object)
    "#,
    );
    let first = tick(&mut rt, 0);
    assert_eq!(
        rt.eval::<String>("return table.concat(LOG, ',')").unwrap(),
        "setup,u,old-cu"
    );
    assert!(first.iter().any(|c| c.player == PlayerId::SCRIPT
        && c.kind == CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(1)))));
    tick(&mut rt, 1);
    assert_eq!(
        rt.eval::<String>("return table.concat(LOG, ',')").unwrap(),
        "setup,u,old-cu,u,old-cu,f:false"
    );
    assert!(rt.eval::<bool>("return Cineractive.active").unwrap());
    tick(&mut rt, 2);
    assert_eq!(
        rt.eval::<String>("return table.concat(LOG, ',')").unwrap(),
        "setup,u,old-cu,u,old-cu,f:false,u,new-cu"
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn external_stop_finishes_once_and_inactive_ticks_skip_cinematic_update() {
    let mut rt = runtime(
        r#"
        LOG={}
        MMM.register({
            setup=function() Cineractive.active=true end,
            update=function() LOG[#LOG+1]='u' end,
            cineractiveUpdate=function() LOG[#LOG+1]='cu' end,
            cineractiveFinish=function(_,skipped) LOG[#LOG+1]='f:'..tostring(skipped) end,
        })
    "#,
    );
    tick(&mut rt, 0);
    rt.eval::<()>("Cineractive.active=false").unwrap();
    tick(&mut rt, 1);
    tick(&mut rt, 2);
    assert_eq!(
        rt.eval::<String>("return table.concat(LOG, ',')").unwrap(),
        "u,cu,u,f:false,u"
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn cinematic_update_errors_share_five_error_budget_but_healthy_callbacks_continue() {
    let mut rt = runtime(
        r#"
        U=0; CU=0; HEALTHY=0
        MMM.register({
            setup=function() Cineractive.active=true end,
            update=function()
                U=U+1
                if U<=2 then error('normal update boom') end
            end,
            cineractiveUpdate=function() CU=CU+1; error('cinematic update boom') end,
        })
        MMM.register(function() HEALTHY=HEALTHY+1 end)
    "#,
    );
    for at in 0..7 {
        tick(&mut rt, at);
    }
    let counts: (u32, u32, u32) = rt.eval("return U, CU, HEALTHY").unwrap();
    assert_eq!(counts, (5, 3, 7));
    assert_eq!(rt.errors().len(), 5, "{:?}", rt.errors());
    assert_eq!(
        rt.errors().iter().filter(|e| e.contains("normal update boom")).count(),
        2
    );
    assert_eq!(
        rt.errors()
            .iter()
            .filter(|e| e.contains("cinematic update boom"))
            .count(),
        3
    );
}

#[test]
fn throwing_finish_is_consumed_once_and_does_not_stop_other_callbacks() {
    let mut rt = runtime(
        r#"
        U=0; HEALTHY=0
        MMM.register({
            setup=function() Cineractive.active=true end,
            update=function() U=U+1; Cineractive.active=false end,
            cineractiveFinish=function() error('finish boom') end,
        })
        MMM.register(function() HEALTHY=HEALTHY+1 end)
    "#,
    );
    for at in 0..4 {
        tick(&mut rt, at);
    }
    assert_eq!(rt.eval::<(u32, u32)>("return U, HEALTHY").unwrap(), (4, 4));
    assert_eq!(
        rt.errors().len(),
        1,
        "finish must not repeat after an error: {:?}",
        rt.errors()
    );
    assert!(rt.errors()[0].contains("finish boom"));
}

#[test]
fn invalid_optional_methods_do_not_consume_callback_ids() {
    let mut rt = runtime(
        r#"
        local valid=function() end
        BAD_CU=pcall(function() MMM.register({setup=valid,cineractiveUpdate=7}) end)
        BAD_CF=pcall(function() MMM.register({update=valid,cineractiveFinish='wrong'}) end)
        BAD_ONLY=pcall(function() MMM.register({cineractiveUpdate=valid}) end)
        GOOD_ID=MMM.register(function() end)
    "#,
    );
    let observed: (bool, bool, bool, u64) = rt.eval("return BAD_CU, BAD_CF, BAD_ONLY, GOOD_ID").unwrap();
    assert_eq!(observed, (false, false, false, 1));
    tick(&mut rt, 0);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn cinematic_update_error_defers_finish_until_a_successful_poll() {
    let mut rt = runtime(
        r#"
        U=0; FINISH=0; HEALTHY=0
        MMM.register({
            setup=function() Cineractive.active=true end,
            update=function() U=U+1 end,
            cineractiveUpdate=function()
                Cineractive.active=false
                error('cinematic end failed')
            end,
            cineractiveFinish=function(_,skipped)
                assert(skipped==false)
                FINISH=FINISH+1
            end,
        })
        MMM.register(function() HEALTHY=HEALTHY+1 end)
    "#,
    );
    tick(&mut rt, 0);
    assert_eq!(
        rt.eval::<(u32, u32, u32)>("return U,FINISH,HEALTHY").unwrap(),
        (1, 0, 1)
    );
    assert_eq!(rt.errors().len(), 1);
    assert!(rt.errors()[0].contains("cinematic end failed"));
    tick(&mut rt, 1);
    assert_eq!(
        rt.eval::<(u32, u32, u32)>("return U,FINISH,HEALTHY").unwrap(),
        (2, 1, 2)
    );
    tick(&mut rt, 2);
    assert_eq!(
        rt.eval::<(u32, u32, u32)>("return U,FINISH,HEALTHY").unwrap(),
        (3, 1, 3)
    );
    assert_eq!(rt.errors().len(), 1);
}

#[test]
fn repeated_normal_update_errors_exhaust_budget_before_pending_finish() {
    let mut rt = runtime(
        r#"
        U=0; FINISH=0; HEALTHY=0
        MMM.register({
            setup=function() Cineractive.active=true end,
            update=function()
                U=U+1
                if U>1 then error('normal update failed') end
            end,
            cineractiveFinish=function() FINISH=FINISH+1 end,
        })
        MMM.register(function() HEALTHY=HEALTHY+1 end)
    "#,
    );
    tick(&mut rt, 0);
    rt.eval::<()>("Cineractive.active=false").unwrap();
    for at in 1..8 {
        tick(&mut rt, at);
    }
    assert_eq!(
        rt.eval::<(u32, u32, u32)>("return U,FINISH,HEALTHY").unwrap(),
        (6, 0, 8)
    );
    assert_eq!(rt.errors().len(), 5);
    assert!(rt.errors().iter().all(|e| e.contains("normal update failed")));
}
