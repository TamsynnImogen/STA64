//! Lookup-only diagnostics for permissive mission API stubs. Argument values
//! are deliberately absent from the reported names.

use std::collections::BTreeMap;

use sta_mission::runtime::MissionRuntime;
use sta_sim::WorldView;
use sta_sim::setup::Controller;

fn tick(rt: &mut MissionRuntime, at: u64) {
    let view = WorldView {
        tick: at,
        ..WorldView::default()
    };
    let mut commands = Vec::new();
    rt.think(&view, &[], &mut commands);
    assert!(commands.is_empty());
}

#[test]
fn missing_lookup_paths_count_at_load_and_in_callbacks_without_arguments() {
    let mut rt = MissionRuntime::from_source(
        "stubs.lua",
        r#"
        local function probe()
            MissingAPI.api.run(17, 'sensitive-argument-marker')
        end
        probe()
        MMM.unknown('sensitive-argument-marker')
        local ignored = MissingFunction().prop
        local table_lookup = Mystery[setmetatable({}, {__tostring=function() error('must not stringify key') end})]
        MMM.register(function() probe() end)
    "#,
        3,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    for at in 0..3 {
        tick(&mut rt, at);
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let counts: BTreeMap<_, _> = rt.unsupported().into_iter().collect();
    assert_eq!(
        counts,
        BTreeMap::from([
            ("stub global MissingAPI".to_string(), 4),
            ("stub field MissingAPI.api".to_string(), 4),
            ("stub field MissingAPI.api.run".to_string(), 4),
            ("stub field MMM.unknown".to_string(), 1),
            ("stub global MissingFunction".to_string(), 1),
            ("stub field MissingFunction().prop".to_string(), 1),
            ("stub global Mystery".to_string(), 1),
            ("stub field Mystery.<table>".to_string(), 1),
        ])
    );
    assert!(counts.keys().all(|name| !name.contains("sensitive-argument-marker")));
    assert!(rt.take_commands().is_empty());

    let before = rt.unsupported();
    let known: (bool, u8) = rt.eval("return A2.IsAlive(0), Player(0).number").unwrap();
    assert_eq!(known, (false, 0));
    assert_eq!(
        rt.unsupported(),
        before,
        "known API reads should not masquerade as stubs"
    );
}

#[test]
fn permissive_stub_values_keep_existing_math_comparison_and_call_behavior() {
    let mut rt = MissionRuntime::from_source("stub-compat.lua", "", 3);
    let arithmetic: (i64, i64, i64, i64, i64, i64, i64, i64) = rt
        .eval(
            r#"
        local s=MissingMath
        return s+1, s-1, s*2, s/2, s%2, -s, s^2, #s
    "#,
        )
        .unwrap();
    assert_eq!(arithmetic, (0, 0, 0, 0, 0, 0, 0, 0));
    let rest: (i64, String, bool, bool, String, bool) = rt
        .eval(
            r#"
        local s=MissingOther
        return s//2, tostring(s), s<1, s<=1, s..'x', type(MissingCallable(1))=='table'
    "#,
        )
        .unwrap();
    assert_eq!(rest, (0, "<stub>".into(), false, false, "".into(), true));
    assert!(rt.errors().is_empty() && rt.take_commands().is_empty());
}
