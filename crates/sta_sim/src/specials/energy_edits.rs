use super::*;
use crate::command::Edit;
use crate::replay::{Recorder, Replay};

fn energy(player: PlayerId, seq: u32, id: EntityId, to: Fixed) -> Command {
    Command {
        player,
        seq,
        kind: CommandKind::Edit(Edit::SetEnergy(id, to)),
    }
}

#[test]
fn set_energy_clamps_before_lazy_initialization_without_touching_damage() {
    let (mut sim, id, _, _) = scene("fx_cloaker", 2000);
    assert_eq!(sim.specials.energy_of(id), None);
    let hull = sim.unit(id).unwrap().hull;
    let shields = sim.unit(id).unwrap().shields;

    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::SetEnergy(id, Fixed::from_int(17))));
    assert_eq!(sim.specials.energy_of(id), Some(Fixed::from_int(17)));
    assert!(sim.apply_edit(
        PlayerId::BENCH,
        &Edit::SetEnergy(id, Fixed::from_bits(i64::MIN))
    ));
    assert_eq!(sim.specials.energy_of(id), Some(Fixed::ZERO));
    assert!(sim.apply_edit(
        PlayerId::SCRIPT,
        &Edit::SetEnergy(id, Fixed::from_bits(i64::MAX))
    ));
    assert_eq!(sim.specials.energy_of(id), Some(Fixed::from_int(600)));
    assert_eq!(sim.unit(id).unwrap().hull, hull);
    assert_eq!(sim.unit(id).unwrap().shields, shields);
}

#[test]
fn invalid_energy_edits_leave_no_rows_or_other_state_changes() {
    let (mut sim, id, no_pool, _) = scene("fx_cloaker", 2000);
    let before = sim.digest();
    assert!(!sim.apply_edit(PlayerId(1), &Edit::SetEnergy(id, Fixed::ONE)));
    assert!(!sim.apply_edit(
        PlayerId::SCRIPT,
        &Edit::SetEnergy(EntityId(u32::MAX), Fixed::ONE)
    ));
    assert!(!sim.apply_edit(PlayerId::BENCH, &Edit::SetEnergy(no_pool, Fixed::ONE)));
    assert!(sim.specials.is_empty());
    assert_eq!(sim.digest(), before);

    sim.units.get_mut(id).unwrap().hull[0] = Fixed::ZERO;
    let dead_digest = sim.digest();
    assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::SetEnergy(id, Fixed::ONE)));
    assert!(sim.specials.is_empty());
    assert_eq!(sim.digest(), dead_digest);
}

#[test]
fn tick_orders_energy_edits_before_regeneration_and_cloak_cost() {
    let (mut sim, id, _, _) = scene("fx_cloaker", 2000);
    let rate = energy_def(
        sim.content()
            .unwrap()
            .unit(sim.unit(id).unwrap().def)
            .unwrap(),
    )
    .unwrap()
    .rate;
    let mut order = cloak(254, id, true);
    order.seq = 3;
    let out = sim.step(&[
        energy(PlayerId::SCRIPT, 1, id, Fixed::from_int(10)),
        energy(PlayerId::SCRIPT, 2, id, Fixed::from_int(80)),
        order,
    ]);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    // The later edit supplies the cloak's 50-point activation cost. The
    // normal 0.5-point drain and regeneration still run on this tick.
    let drain = Fixed::from_f64(15.0) / Fixed::from_int(crate::TICK_HZ as i32);
    assert_eq!(
        sim.specials.energy_of(id),
        Some(Fixed::from_int(30) + rate - drain)
    );
    assert_eq!(sim.specials.cloak_state(id), Some(CloakState::Engaging(29)));
}

#[test]
fn low_energy_edit_cannot_pay_cloak_cost_but_still_regenerates() {
    let (mut sim, id, _, _) = scene("fx_cloaker", 2000);
    let rate = energy_def(
        sim.content()
            .unwrap()
            .unit(sim.unit(id).unwrap().def)
            .unwrap(),
    )
    .unwrap()
    .rate;
    let mut order = cloak(254, id, true);
    order.seq = 1;
    let out = sim.step(&[energy(PlayerId::SCRIPT, 0, id, Fixed::from_int(49)), order]);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.specials.cloak_state(id), None);
    assert_eq!(sim.specials.energy_of(id), Some(Fixed::from_int(49) + rate));
}

#[test]
fn extreme_refill_caps_without_wrap_and_negative_rate_floors_at_zero() {
    let fake = sta_content::fake::db();
    let (mut units, weapons) = fixture::defs();
    // Past the 32.32 range (saturates), and a negative rate the ODF loader
    // would clamp but content built in code may carry.
    units[0].special_energy_max = Fixed::from_f64(2147483648.0);
    units[0].special_energy_rate = Fixed::from_f64(2147483648.0);
    units[1].special_energy_max = Fixed::from_int(100);
    units[1].special_energy_rate = Fixed::from_int(-3000);
    let mut builder = sta_content::ContentDb::builder()
        .profile(fake.profile())
        .features(fake.features().clone());
    for weapon in fake.weapons().cloned().chain(weapons) {
        builder = builder.weapon(weapon);
    }
    for unit in fake.units().cloned().chain(units) {
        builder = builder.unit(unit);
    }
    let db = std::sync::Arc::new(builder.build().unwrap());
    let setup = GameSetup {
        seed: 3,
        map: std::sync::Arc::new(sta_content::MapDef::empty()),
        content: std::sync::Arc::clone(&db),
        players: vec![],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let high = sim
        .spawn_def(
            db.key("fx_cloaker").unwrap(),
            PlayerId::SCRIPT,
            v(0, 0),
            AHEAD,
        )
        .unwrap();
    let low = sim
        .spawn_def(
            db.key("fx_cloaker_frail").unwrap(),
            PlayerId::SCRIPT,
            v(100, 0),
            AHEAD,
        )
        .unwrap();
    assert!(sim.apply_edit(
        PlayerId::SCRIPT,
        &Edit::SetEnergy(high, Fixed::from_bits(i64::MAX - 1))
    ));
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::SetEnergy(low, Fixed::from_int(1))));
    sim.step(&[]);
    assert_eq!(
        sim.specials.energy_of(high),
        Some(Fixed::from_bits(i64::MAX))
    );
    assert_eq!(sim.specials.energy_of(low), Some(Fixed::ZERO));
}

#[test]
fn energy_edit_snapshot_and_recorded_replay_continue_identically() {
    let db = std::sync::Arc::new(fixture::db());
    let mut map = sta_content::MapDef::empty();
    map.objects.push(sta_content::MapObject {
        odf: "fx_cloaker".into(),
        owner: Some(1),
        pos: v(0, 0),
        heading: AHEAD,
        label: None,
    });
    let setup = GameSetup {
        seed: 3,
        map: std::sync::Arc::new(map),
        content: std::sync::Arc::clone(&db),
        players: vec![PlayerSetup {
            id: PlayerId(1),
            team: TeamId(1),
            race: "fx".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    };
    let mut recorder = Recorder::with_interval(&setup, 1);
    let id = recorder.sim().view().units[0].id;
    recorder.step(&[energy(PlayerId::SCRIPT, 0, id, Fixed::from_int(70))]);
    let mut live =
        Sim::from_snapshot(&recorder.sim().snapshot(), Some(std::sync::Arc::clone(&db))).unwrap();
    assert_eq!(live.digest(), recorder.sim().digest());
    for tick in 0..40 {
        let commands = if tick == 0 {
            vec![cloak(1, id, true)]
        } else {
            vec![]
        };
        let recorded = recorder.step(&commands);
        let loaded = live.step(&commands);
        assert_eq!(loaded, recorded, "continuation tick {tick}");
    }
    let digest = live.digest();
    assert_eq!(recorder.sim().snapshot(), live.snapshot());
    let replay = Replay::decode(&recorder.finish().encode()).unwrap();
    assert_eq!(replay.play(db).unwrap().digest(), digest);
}
