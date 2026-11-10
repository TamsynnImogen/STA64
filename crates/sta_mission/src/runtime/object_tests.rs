use super::*;

#[test]
fn mission_objects_setup_once_on_the_first_controller_tick_and_update_in_order() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        SETUPS, UPDATES, LOAD_TIME = 0, 0, Game.time
        class 'MissionObject'
        function MissionObject:__init() self.value = 40 end
        function MissionObject:setup()
            SETUPS = SETUPS + 1
            SETUP_TIME = Game.time
            self.value = self.value + 1
        end
        function MissionObject:update(time)
            UPDATES = UPDATES + 1
            VALUE, ARG_TIME, API_TIME = self.value, time, Game.time
        end
        HANDLE = MMM.register(MissionObject())
    "#,
        7,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(rt.eval::<i64>("return SETUPS").unwrap(), 0);
    rt.step(15);
    rt.step(16);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(
        rt.eval::<(i64, i64, i64)>("return SETUPS, UPDATES, VALUE").unwrap(),
        (1, 2, 41)
    );
    assert_eq!(rt.eval::<f64>("return SETUP_TIME").unwrap(), 0.5);
    assert_eq!(
        rt.eval::<(f64, f64)>("return ARG_TIME, API_TIME").unwrap(),
        (16.0 / 30.0, 16.0 / 30.0)
    );
    rt.eval::<()>("MMM.unregister(HANDLE)").unwrap();
    rt.step(17);
    assert_eq!(rt.eval::<i64>("return UPDATES").unwrap(), 2);
}

#[test]
fn setup_registration_is_deferred_and_existing_function_callbacks_keep_their_order() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        LOG = ''
        MMM.register({setup = function(self)
            LOG = LOG .. 's'
            MMM.register(function() LOG = LOG .. 'n' end)
        end, update = function(self) LOG = LOG .. 'o' end})
        MMM.register(function() LOG = LOG .. 'f' end)
    "#,
        1,
    );
    rt.step(0);
    assert_eq!(rt.eval::<String>("return LOG").unwrap(), "sof");
    rt.step(1);
    assert_eq!(rt.eval::<String>("return LOG").unwrap(), "sofofn");
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn setup_errors_share_the_callback_budget_and_do_not_stop_other_callbacks() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        GOOD, ATTEMPTS, UPDATES = 0, 0, 0
        MMM.register({setup = function() ATTEMPTS=ATTEMPTS+1; error('bad setup') end,
                      update = function() UPDATES=UPDATES+1 end})
        MMM.register(function() GOOD=GOOD+1 end)
    "#,
        1,
    );
    for tick in 0..20 {
        rt.step(tick);
    }
    assert_eq!(
        rt.eval::<(u32, u32, u32)>("return GOOD,ATTEMPTS,UPDATES").unwrap(),
        (20, MAX_CALLBACK_ERRORS, 0)
    );
    assert_eq!(rt.errors().len(), MAX_CALLBACK_ERRORS as usize);
    assert!(rt.errors()[0].contains("bad setup"));
    assert_eq!(rt.active_callbacks(), 1);
}

#[test]
fn invalid_mission_objects_do_not_consume_callback_ids() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        assert(not pcall(function() MMM.register({}) end))
        assert(not pcall(function() MMM.register({update = 3}) end))
        assert(not pcall(function() MMM.register('bad') end))
        ID = MMM.register({update = function() end})
    "#,
        1,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(rt.eval::<u64>("return ID").unwrap(), 1);
    rt.step(0);
    assert!(rt.errors().is_empty());
}

#[test]
fn a_setup_only_object_and_update_only_object_are_valid() {
    let mut rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        SETUPS,UPDATES=0,0
        MMM.register({setup=function() SETUPS=SETUPS+1 end})
        MMM.register({update=function() UPDATES=UPDATES+1 end})
    "#,
        1,
    );
    for tick in 0..3 {
        rt.step(tick);
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(rt.eval::<(i64, i64)>("return SETUPS,UPDATES").unwrap(), (1, 3));
}
