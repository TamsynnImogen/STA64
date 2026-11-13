//! SIM-57: mission `SetPosition`, `PointAt` and `SetAngularVelocity` edits.
//! Synthetic content only (a mobile craft class on an A2-profile map).

use std::sync::Arc;

use sta_content::{ConfinementOverrides, ContentDb, MapDef, MoveStats, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::replay::{Recorder, Replay};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn at(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: n(y), z: n(z) }
}

fn setup() -> GameSetup {
    let mut def = UnitDef::new("subject", "craft");
    def.confinement = ConfinementOverrides::default();
    def.hull = n(100);
    def.movement = MoveStats::from_f32(120.0, 2.0, 2.0, 2.0, 1.0);
    let content = Arc::new(ContentDb::builder().profile(Profile::Sta2Classic).unit(def).build().unwrap());
    let mut map = MapDef::empty();
    map.bounds = (at(-500, -10, -500), at(500, 10, 500));
    GameSetup {
        seed: 57,
        map: Arc::new(map),
        content,
        players: vec![PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "synthetic".into(), control: Control::Idle }],
        options: GameOptions::default(),
    }
}

fn spawn(sim: &mut Sim, pos: Vec3, heading: (Fixed, Fixed)) -> EntityId {
    let key = sim.content().unwrap().key("subject").unwrap();
    sim.spawn_def(key, PlayerId(0), pos, heading).unwrap()
}

fn script(seq: u32, e: Edit) -> Command {
    Command { player: PlayerId::SCRIPT, seq, kind: CommandKind::Edit(e) }
}

fn rejected(events: &[SimEvent]) -> usize {
    events.iter().filter(|e| matches!(e, SimEvent::Rejected { .. })).count()
}

fn go(sim: &mut Sim, id: EntityId, to: Vec3) {
    let c = Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Move { units: vec![id], to, queue: false }) };
    assert_eq!(rejected(&sim.step(&[c]).events), 0);
}

fn near(a: Fixed, b: f64, tol: f64) -> bool {
    (a.to_f32() as f64 - b).abs() <= tol
}

#[test]
fn teleport_is_instant_zeroes_speed_and_keeps_heading_and_order() {
    let mut sim = Sim::from_setup(&setup());
    let id = spawn(&mut sim, at(0, 0, 0), (Fixed::ZERO, Fixed::ONE));
    go(&mut sim, id, at(0, 0, 400));
    for _ in 0..30 {
        sim.step(&[]);
    }
    let before = sim.unit(id).unwrap().clone();
    assert!(before.speed > Fixed::ZERO, "the unit is moving before the edit");
    assert!(before.target.is_some());
    let out = sim.step(&[script(1, Edit::Position(id, at(-300, 5, 100)))]);
    assert_eq!(rejected(&out.events), 0);
    let after = sim.unit(id).unwrap();
    // One tick of re-acceleration from rest at most; far less than the old speed.
    assert!(after.speed < before.speed / n(4), "speed was zeroed: {:?} vs {:?}", after.speed, before.speed);
    assert!(near(after.pos.x, -300.0, 1.0) && near(after.pos.z, 100.0, 1.0), "{:?}", after.pos);
    assert_eq!(after.target, before.target, "the order is kept");
    assert!(after.heading.1 > Fixed::ZERO, "the heading is kept (it only steers toward the target)");
}

#[test]
fn teleport_clamps_only_a_unit_whose_force_onto_map_flag_is_set() {
    let mut sim = Sim::from_setup(&setup());
    let id = spawn(&mut sim, at(0, 0, 0), (Fixed::ZERO, Fixed::ONE));
    assert_eq!(sim.force_onto_map(id), Some(true));
    let out = sim.step(&[script(1, Edit::Position(id, at(900, 100, -900)))]);
    assert_eq!(rejected(&out.events), 0);
    let p = sim.unit(id).unwrap().pos;
    // One edge buffer (100) and the epsilon (0.1) inside the bounds; y to the map's range.
    assert!(near(p.x, 399.9, 0.01) && near(p.z, -400.0, 0.01) && near(p.y, 10.0, 0.01), "{p:?}");

    let free = spawn(&mut sim, at(0, 0, 0), (Fixed::ZERO, Fixed::ONE));
    sim.step(&[script(2, Edit::ForceOntoMap { id: free, enabled: false })]);
    assert_eq!(sim.force_onto_map(free), Some(false));
    sim.step(&[script(3, Edit::Position(free, at(900, 100, -900)))]);
    let p = sim.unit(free).unwrap().pos;
    assert!(near(p.x, 900.0, 0.01) && near(p.y, 100.0, 0.01) && near(p.z, -900.0, 0.01), "unflagged unit is not clamped: {p:?}");
}

#[test]
fn placement_edits_are_for_scripts_and_the_bench_and_need_a_live_unit() {
    let mut sim = Sim::from_setup(&setup());
    let id = spawn(&mut sim, at(0, 0, 0), (Fixed::ZERO, Fixed::ONE));
    let edits = [
        Edit::Position(id, at(10, 0, 10)),
        Edit::Heading(id, (Fixed::ONE, Fixed::ZERO)),
        Edit::AngularVelocity(id, Fixed::ONE),
    ];
    for (i, e) in edits.iter().enumerate() {
        let seq = i as u32 * 10;
        // The unmodified edit from the script is accepted ...
        assert_eq!(rejected(&sim.step(&[script(seq, e.clone())]).events), 0, "{e:?}");
        // ... from the bench too, and the same edit from a player is not.
        let bench = Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(e.clone()) };
        assert_eq!(rejected(&sim.step(&[bench]).events), 0, "{e:?}");
        let player = Command { player: PlayerId(0), seq: seq + 1, kind: CommandKind::Edit(e.clone()) };
        assert_eq!(rejected(&sim.step(&[player]).events), 1, "{e:?}");
    }
    let ghost = EntityId(999);
    for e in [
        Edit::Position(ghost, at(1, 0, 1)),
        Edit::Heading(ghost, (Fixed::ONE, Fixed::ZERO)),
        Edit::AngularVelocity(ghost, Fixed::ONE),
    ] {
        assert_eq!(rejected(&sim.step(&[script(100, e.clone())]).events), 1, "{e:?}");
    }
}

#[test]
fn heading_is_instant_normalised_and_a_zero_direction_is_rejected() {
    let mut sim = Sim::from_setup(&setup());
    let id = spawn(&mut sim, at(0, 0, 0), (Fixed::ZERO, Fixed::ONE));
    let out = sim.step(&[script(1, Edit::Heading(id, (n(3), n(4))))]);
    assert_eq!(rejected(&out.events), 0);
    let h = sim.unit(id).unwrap().heading;
    assert!(near(h.0, 0.6, 0.001) && near(h.1, 0.8, 0.001), "{h:?}");
    let out = sim.step(&[script(2, Edit::Heading(id, (Fixed::ZERO, Fixed::ZERO)))]);
    assert_eq!(rejected(&out.events), 1);
    assert_eq!(sim.unit(id).unwrap().heading, h, "a rejected edit changes nothing");
}

#[test]
fn heading_keeps_the_order_and_the_unit_steers_back_onto_its_course() {
    // Control: without the edit the unit stops where this model stops it.
    let mut control = Sim::from_setup(&setup());
    let c = spawn(&mut control, at(0, 0, 0), (Fixed::ZERO, Fixed::ONE));
    go(&mut control, c, at(0, 0, 350));
    for _ in 0..920 {
        control.step(&[]);
    }
    assert!(control.unit(c).unwrap().target.is_none());
    let stop_z = control.unit(c).unwrap().pos.z.to_f32() as f64;
    let mut sim = Sim::from_setup(&setup());
    let id = spawn(&mut sim, at(0, 0, 0), (Fixed::ZERO, Fixed::ONE));
    go(&mut sim, id, at(0, 0, 350));
    for _ in 0..20 {
        sim.step(&[]);
    }
    let target = sim.unit(id).unwrap().target;
    assert!(target.is_some());
    sim.step(&[script(1, Edit::Heading(id, (Fixed::ZERO, -Fixed::ONE)))]);
    assert!(sim.unit(id).unwrap().heading.1 < Fixed::ZERO);
    assert_eq!(sim.unit(id).unwrap().target, target);
    for _ in 0..900 {
        sim.step(&[]);
        if sim.unit(id).unwrap().target.is_none() {
            break;
        }
    }
    assert!(sim.unit(id).unwrap().target.is_none(), "the original destination is reached");
    assert!(near(sim.unit(id).unwrap().pos.z, stop_z, 40.0), "{:?}", sim.unit(id).unwrap().pos);
}

#[test]
fn angular_velocity_is_lost_with_engines_on_and_kept_without_decay_with_engines_off() {
    let mut sim = Sim::from_setup(&setup());
    let on = spawn(&mut sim, at(0, 0, 0), (Fixed::ONE, Fixed::ZERO));
    let off = spawn(&mut sim, at(100, 0, 0), (Fixed::ONE, Fixed::ZERO));
    sim.step(&[script(1, Edit::Engines(off, false))]);
    // +0.5 rad/s turns +x toward +z.
    let half = Fixed::from_f64(0.5);
    sim.step(&[script(2, Edit::AngularVelocity(on, half)), script(3, Edit::AngularVelocity(off, half))]);
    assert_eq!(sim.unit(on).unwrap().omega, Fixed::ZERO, "the steering owns the turn rate of a craft with engines on");
    assert_eq!(sim.unit(off).unwrap().omega, half);
    for _ in 0..59 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(on).unwrap().heading, (Fixed::ONE, Fixed::ZERO));
    assert_eq!(sim.unit(off).unwrap().omega, half, "no decay");
    // 60 ticks at 30 Hz = 2 s at 0.5 rad/s: one radian (cos 1, sin 1).
    let h = sim.unit(off).unwrap().heading;
    assert!(near(h.0, 0.5403, 0.01) && near(h.1, 0.8415, 0.01), "{h:?}");
    // The rate is capped for a script.
    sim.step(&[script(4, Edit::AngularVelocity(off, n(1000)))]);
    assert_eq!(sim.unit(off).unwrap().omega, n(sta_sim::command::MAX_SCRIPT_OMEGA));
    sim.step(&[script(5, Edit::AngularVelocity(off, -n(1000)))]);
    assert_eq!(sim.unit(off).unwrap().omega, -n(sta_sim::command::MAX_SCRIPT_OMEGA));
}

#[test]
fn placement_edits_replay_and_snapshot_to_the_same_digest() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let mut rec = Recorder::with_interval(&setup, 1);
    let key = setup.content.key("subject").unwrap();
    let spawn_edit = |seq, x, heading| script(seq, Edit::Spawn { odf: key, player: PlayerId(0), pos: at(x, 0, 0), heading });
    sim.step(&[]);
    rec.step(&[]);
    let (a, b) = (EntityId(1), EntityId(2));
    let batches = [
        vec![spawn_edit(10, 0, (Fixed::ZERO, Fixed::ONE)), spawn_edit(11, 50, (Fixed::ONE, Fixed::ZERO))],
        vec![script(1, Edit::Engines(b, false))],
        vec![script(2, Edit::Position(a, at(120, 3, -80))), script(3, Edit::Heading(a, (n(2), n(-1))))],
        vec![script(4, Edit::AngularVelocity(b, Fixed::from_f64(0.3)))],
        vec![],
        vec![],
    ];
    for batch in &batches {
        let expected = sim.step(batch);
        let recorded = rec.step(batch);
        assert_eq!(expected.events, recorded.events);
    }
    assert!(sim.unit(b).unwrap().omega != Fixed::ZERO);
    let replay = Replay::decode(&rec.finish().encode()).unwrap();
    assert_eq!(replay.play(setup.content.clone()).unwrap().digest(), sim.digest());
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), Some(setup.content.clone())).unwrap();
    assert_eq!(loaded.digest(), sim.digest());
    for _ in 0..90 {
        sim.step(&[]);
        loaded.step(&[]);
    }
    assert_eq!(loaded.digest(), sim.digest(), "the saved spin and heading continue identically");
}
