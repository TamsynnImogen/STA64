//! SIM-29: `activateOnTeamChange` re-activation after a completed team swap.

use crate::command::{Command, CommandKind, Edit};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::specials::fixture;
use crate::view::UnitFlags;
use crate::{EntityId, Fixed, PlayerId, Sim, TeamId, Vec3, WorldSource};
use sta_content::ContentDb;
use std::sync::Arc;

fn db(activate: bool, feature: bool, cloak_cost: &str) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let (units, mut weapons) = fixture::defs();
    for w in &mut weapons {
        if w.odf == "fx_cloak" {
            w.activate_on_team_change = activate;
            let text = format!("effectTimer = 0.1\nenergyDrainRate = 1.0\nspecialEnergyCost = {cloak_cost}\n");
            w.raw = Arc::new(sta_odf::resolve("fx_cloak", &mut |_| Some(text.clone())).unwrap());
        }
    }
    let mut features = fake.features().clone();
    features.team_change_weapons = feature;
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for w in fake.weapons().cloned().chain(weapons) {
        b = b.weapon(w);
    }
    for u in fake.units().cloned().chain(units) {
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

/// Seats 1 and 3 are allied (team 1), seat 2 is team 2. A cloaker of seat 1.
fn world(db: &Arc<ContentDb>) -> (Sim, EntityId) {
    let seat = |id: u8, team: u8| PlayerSetup { id: PlayerId(id), team: TeamId(team), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::clone(db),
        players: vec![seat(1, 1), seat(2, 2), seat(3, 1)],
        options: GameOptions::default(),
    });
    let at = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };
    let id = sim.spawn_def(db.key("fx_cloaker").unwrap(), PlayerId(1), at, (Fixed::ONE, Fixed::ZERO)).unwrap();
    (sim, id)
}

fn give(sim: &mut Sim, id: EntityId, to: u8) {
    let c = Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::Team(id, PlayerId(to))) };
    let out = sim.step(&[c]);
    assert!(!out.events.iter().any(|e| matches!(e, crate::SimEvent::Rejected { .. })), "{:?}", out.events);
}

fn cloaking(sim: &Sim, id: EntityId) -> bool {
    sim.specials.cloak_state(id).is_some()
}

#[test]
fn a_cloak_engages_for_the_new_owner_after_a_team_change() {
    let d = db(true, true, "0");
    let (mut sim, id) = world(&d);
    assert!(!cloaking(&sim, id));
    give(&mut sim, id, 2);
    assert!(cloaking(&sim, id), "engaged on the tick of the swap");
    for _ in 0..10 {
        sim.step(&[]);
    }
    assert!(sim.units.get(id).unwrap().flags.contains(UnitFlags::CLOAKED));
}

#[test]
fn same_team_neutral_missing_key_and_missing_feature_activate_nothing() {
    let d = db(true, true, "0");
    let (mut sim, id) = world(&d);
    give(&mut sim, id, 3); // allied seat, same team
    assert!(!cloaking(&sim, id));
    give(&mut sim, id, crate::world::NEUTRAL.0);
    assert!(!cloaking(&sim, id));
    for (activate, feature) in [(false, true), (true, false)] {
        let d = db(activate, feature, "0");
        let (mut sim, id) = world(&d);
        give(&mut sim, id, 2);
        assert!(!cloaking(&sim, id), "key {activate} feature {feature}");
    }
}

#[test]
fn the_weapons_own_restrictions_stay_and_a_refused_activation_is_dropped() {
    let d = db(true, true, "100000"); // more energy than the unit has
    let (mut sim, id) = world(&d);
    give(&mut sim, id, 2);
    assert!(!cloaking(&sim, id));
    for _ in 0..5 {
        sim.step(&[]);
    }
    assert!(!cloaking(&sim, id), "no retry on later ticks");
}

#[test]
fn the_key_reaches_the_content_hash_and_the_state_snapshots_after_a_swap() {
    assert_ne!(db(true, true, "0").hash(), db(false, true, "0").hash());
    let d = db(true, true, "0");
    let (mut sim, id) = world(&d);
    give(&mut sim, id, 2);
    let mut copy = Sim::from_snapshot(&sim.snapshot(), Some(Arc::clone(&d))).unwrap();
    assert_eq!(copy.digest(), sim.digest());
    for _ in 0..40 {
        sim.step(&[]);
        copy.step(&[]);
        assert_eq!(copy.digest(), sim.digest());
    }
}
