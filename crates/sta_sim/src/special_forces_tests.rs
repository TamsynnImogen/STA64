//! LUA-28: a flagged craft in the whole-state digest and the snapshot, and what the flag
//! does not change. The flag is `Switches.special_forces` (`Edit::SpecialForces`, tag 37),
//! saved and hashed since SIM-58; this pins that layout with a sample that holds one.

use crate::command::{Command, CommandKind, Edit, Order};
use crate::{Fixed, PlayerId, Sim, SimEvent, Vec3, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// Two authored cruisers of two seats (fake content), one moving, with the first flagged.
fn flagged_sample() -> (Sim, crate::EntityId) {
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    let db = std::sync::Arc::new(sta_content::fake::db());
    let seat = |id| PlayerSetup { id: PlayerId(id), team: sta_core::TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 1,
        map: std::sync::Arc::new(sta_content::MapDef::empty()),
        content: std::sync::Arc::clone(&db),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let key = db.key("fx_cruiser").unwrap();
    let a = sim.spawn_def(key, PlayerId(0), v(0, 0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    sim.spawn_def(key, PlayerId(1), v(1000, 0), (-Fixed::ONE, Fixed::ZERO)).unwrap();
    sim.step(&[Command { player: PlayerId(0), seq: 0, kind: CommandKind::Order(Order::Move { units: vec![a], to: v(500, -300), queue: false, mode: None }) }]);
    for _ in 0..40 {
        sim.step(&[]);
    }
    let out = sim.step(&[Command { player: PlayerId::SCRIPT, seq: 1, kind: CommandKind::Edit(Edit::SpecialForces(a, true)) }]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })), "{:?}", out.events);
    (sim, a)
}

/// Pinned: the full digest of [`flagged_sample`]. A change is a change of the digest layout
/// of the switch rows and needs a note in the PR.
const FLAGGED_DIGEST: u64 = 0xe503_1001_6340_27fc;

#[test]
fn a_flagged_craft_is_in_the_whole_state_digest_and_pinned() {
    let (sim, a) = flagged_sample();
    assert!(sim.switches(a).unwrap().special_forces);
    assert_eq!(sim.digest(), FLAGGED_DIGEST, "{:#x}", sim.digest());
    // The same run without the flag differs.
    let mut plain = flagged_sample().0;
    plain.step(&[Command { player: PlayerId::SCRIPT, seq: 2, kind: CommandKind::Edit(Edit::SpecialForces(a, false)) }]);
    assert!(!plain.switches(a).unwrap().special_forces);
    assert_ne!(plain.digest(), sim.digest());
}

#[test]
fn the_flag_survives_a_snapshot_with_the_same_digest_and_the_view_lists_it() {
    let (sim, a) = flagged_sample();
    let loaded = Sim::from_snapshot(&sim.snapshot(), sim.content.clone()).unwrap();
    assert!(loaded.switches(a).unwrap().special_forces);
    assert_eq!(loaded.digest(), sim.digest());
    assert_eq!(loaded.digest(), FLAGGED_DIGEST);
    assert_eq!(loaded.view().attrs.special_forces, vec![a]);
    assert!(loaded.view().is_special_forces(a));
}

#[test]
fn the_view_list_follows_set_and_clear() {
    let (mut sim, a) = flagged_sample();
    assert_eq!(sim.view().attrs.special_forces, vec![a]);
    sim.step(&[Command { player: PlayerId::SCRIPT, seq: 2, kind: CommandKind::Edit(Edit::SpecialForces(a, false)) }]);
    assert!(sim.view().attrs.special_forces.is_empty());
}

#[test]
fn the_owner_can_still_order_a_flagged_craft_and_a_team_change_clears_the_flag() {
    let (mut sim, a) = flagged_sample();
    let before = sim.view().unit(a).unwrap().pos;
    let out = sim.step(&[Command { player: PlayerId(0), seq: 5, kind: CommandKind::Order(Order::Move { units: vec![a], to: v(-400, 300), queue: false, mode: None }) }]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })), "{:?}", out.events);
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert_ne!(sim.view().unit(a).unwrap().pos, before, "the flagged craft followed its owner's order");
    sim.step(&[Command { player: PlayerId::SCRIPT, seq: 6, kind: CommandKind::Edit(Edit::Team(a, PlayerId(1))) }]);
    assert!(!sim.view().is_special_forces(a));
}
