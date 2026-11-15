//! LUA-39: the GUARD command (23) of `SetCommandWhatWhoVO` and
//! `SetCommandWhatWhoParamVO`, on the authored two-player scene of the parent module.

use super::*;

#[test]
fn guard_with_and_without_a_parameter_is_the_guard_order_as_the_script() {
    let s = scene();
    let (a, b) = (s.ids[0], s.ids[1]);
    // The parameter is not read by the guard (read from the executable, static); the voice flag is not applied.
    let (rt, commands) = run(&s, "A2.SetCommandWhatWhoParamVO(a, 23, b, 1, false); A2.SetCommandWhatWhoVO(a, 23, b, true)");
    assert_eq!(
        orders(&commands),
        vec![
            (PlayerId::SCRIPT, Order::Guard { units: vec![a], ward: b }),
            (PlayerId::SCRIPT, Order::Guard { units: vec![a], ward: b }),
        ]
    );
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn guard_needs_two_live_craft_and_otherwise_does_nothing() {
    let s = scene();
    // An absent object, the unit itself and a station as the ward: the engine's guard makes the call a silent no-op.
    let (rt, commands) = run(
        &s,
        "A2.SetCommandWhatWhoParamVO(a, 23, 0, 1, false); A2.SetCommandWhatWhoParamVO(a, 23, a, 1, false); \
         A2.SetCommandWhatWhoParamVO(a, 23, s, 1, false); A2.SetCommandWhatWhoVO(0, 23, b, false)",
    );
    assert!(commands.is_empty(), "{commands:?}");
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}
