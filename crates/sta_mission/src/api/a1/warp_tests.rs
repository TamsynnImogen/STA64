//! LUA-31: `A1.SetWarpSpeed`, `ResetWarpSpeed`, `GetWarpSpeed`, `GetImpulseSpeed`.

use super::*;
use sta_sim::present::Presentation as P;

fn warp_edits(commands: &[Command]) -> Vec<(EntityId, Option<Fixed>)> {
    commands
        .iter()
        .filter_map(|c| match &c.kind {
            CommandKind::Edit(Edit::WarpSpeed(id, to)) => {
                assert_eq!(c.player, PlayerId::SCRIPT);
                Some((*id, *to))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn set_and_reset_send_typed_script_edits_and_are_not_counted() {
    let s = scene();
    let (rt, commands) = run(&s, "A1.SetWarpSpeed(a, 30); A1.SetWarpSpeed(b, 40.5); A1.ResetWarpSpeed(a)");
    assert_eq!(
        warp_edits(&commands),
        vec![(s.ids[0], Some(Fixed::from_int(30))), (s.ids[1], Some(Fixed::from_f64(40.5))), (s.ids[0], None)]
    );
    assert_eq!(commands.len(), 3, "{commands:?}");
    assert!(rt.unsupported().iter().all(|(k, _)| !k.contains("WarpSpeed")), "{:?}", rt.unsupported());
}

#[test]
fn anything_but_a_live_craft_does_nothing_and_the_getter_answers_zero() {
    let s = scene();
    let (mut rt, commands) = run(
        &s,
        "A1.SetWarpSpeed(123456, 20); A1.ResetWarpSpeed(123456); A1.SetWarpSpeed(s, 20); A1.ResetWarpSpeed(s); \
         assert(A1.GetWarpSpeed(123456) == 0); assert(A1.GetWarpSpeed(s) == 0); assert(A1.GetImpulseSpeed(s) == 0)",
    );
    assert!(commands.is_empty(), "{commands:?}");
    assert!(rt.errors().is_empty());
    let _ = rt.take_commands();
}

#[test]
fn bad_arguments_raise_without_queueing() {
    let s = scene();
    for script in ["A1.SetWarpSpeed(a)", "A1.SetWarpSpeed(a, 0/0)", "A1.SetWarpSpeed(a, 'x')", "A1.ResetWarpSpeed()"] {
        fails(&s, script);
    }
}

#[test]
fn the_getter_follows_queued_edits_the_view_and_a_cineractive_end() {
    let mut s = scene();
    let a = s.ids[0];
    // The class value is the definition's (zero on the stand-in content); the view lists an override.
    let (mut rt, _) = run(&s, "assert(A1.GetWarpSpeed(a) == 0)");
    let _ = rt.take_commands();
    s.view.attrs.warp = vec![(a, Fixed::from_int(30))];
    let (_, commands) = run(&s, "assert(A1.GetWarpSpeed(a) == 30.0); A1.SetWarpSpeed(a, 45); assert(A1.GetWarpSpeed(a) == 45.0); A1.SetWarpSpeed(a, -3); assert(A1.GetWarpSpeed(a) == 0); A1.SetWarpSpeed(a, 2000); assert(A1.GetWarpSpeed(a) == 1000.0); A1.ResetWarpSpeed(a); assert(A1.GetWarpSpeed(a) == 0)");
    assert_eq!(warp_edits(&commands).len(), 4);
    // The end of a cineractive resets it, a start does not.
    let mut rt = runtime(&s);
    rt.eval::<()>("A1.CineractiveStart(); assert(A1.GetWarpSpeed(a) == 30.0); A1.CineractiveEnd(); assert(A1.GetWarpSpeed(a) == 0)").unwrap();
    assert!(rt.take_commands().iter().any(|c| matches!(&c.kind, CommandKind::Edit(Edit::Present(P::Cineractive { on: false, .. })))));
}

#[test]
fn get_impulse_speed_is_the_class_value_and_follows_set_impulse_speed() {
    let s = scene();
    let class = s.db.unit(s.view.units.iter().find(|u| u.id == s.ids[0]).unwrap().def).unwrap().movement.max_speed;
    let (_, commands) = run(&s, &format!("assert(A1.GetImpulseSpeed(a) == {}); A1.SetImpulseSpeed(a, 85); assert(A1.GetImpulseSpeed(a) == 85.0)", class.to_f32()));
    assert!(commands.iter().any(|c| matches!(&c.kind, CommandKind::Edit(Edit::MaxSpeed(id, Some(v))) if *id == s.ids[0] && *v == Fixed::from_int(85))));
}
