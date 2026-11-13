use super::*;
use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, MoveStats, UnitDef};
use sta_core::TeamId;
use std::sync::Arc;

const P0: PlayerId = PlayerId(0);

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

/// The fake content plus a construction rig, with `pay_on_queue` as asked.
fn content(pay_on_queue: bool) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = pay_on_queue;
    let mut rig = UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    let mut b = ContentDb::builder()
        .profile(fake.profile())
        .features(features)
        .unit(rig);
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    Arc::new(b.build().unwrap())
}

fn game(pay_on_queue: bool) -> Sim {
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "fx".into(),
        control: Control::Human,
    };
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: content(pay_on_queue),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    })
}

fn cmd(player: PlayerId, seq: u32, kind: CommandKind) -> Command {
    Command { player, seq, kind }
}

fn key(sim: &Sim, odf: &str) -> ContentKey {
    sim.content().unwrap().key(odf).unwrap()
}

fn spawn(sim: &mut Sim, odf: &str, at: Vec3) -> EntityId {
    let e = Edit::Spawn {
        odf: key(sim, odf),
        player: P0,
        pos: at,
        heading: (Fixed::ZERO, Fixed::ONE),
    };
    let id = sim.units.next_id();
    sim.step(&[cmd(PlayerId::BENCH, 0, CommandKind::Edit(e))]);
    assert!(sim.unit(id).is_some());
    id
}

fn fund(sim: &mut Sim, dilithium: i32) {
    let e = Edit::Cargo(P0, 0, Fixed::from_int(dilithium));
    sim.step(&[cmd(PlayerId::BENCH, 1, CommandKind::Edit(e))]);
}

fn dilithium(sim: &Sim) -> Fixed {
    sim.economy
        .stock_of(P0)
        .first()
        .copied()
        .unwrap_or(Fixed::ZERO)
}

fn build(sim: &mut Sim, builder: EntityId, odf: &str, at: Option<Vec3>) -> bool {
    let what = key(sim, odf);
    let o = Order::Build { builder, what, at };
    let out = sim.step(&[cmd(P0, 7, CommandKind::Order(o))]);
    !out.events
        .iter()
        .any(|e| matches!(e, SimEvent::Rejected { .. }))
}

/// Step `n` ticks, noting `(tick of the step, event)` for the build events.
fn run(sim: &mut Sim, n: u64, log: &mut Vec<(u64, SimEvent)>) {
    for _ in 0..n {
        let t = sim.tick;
        for e in sim.step(&[]).events {
            if matches!(e, SimEvent::Built { .. } | SimEvent::BuildStarted { .. }) {
                log.push((t, e));
            }
        }
    }
}

fn built(log: &[(u64, SimEvent)]) -> Vec<(u64, EntityId, EntityId)> {
    log.iter()
        .filter_map(|(t, e)| match e {
            SimEvent::Built { builder, id } => Some((*t, *builder, *id)),
            _ => None,
        })
        .collect()
}

#[test]
fn yard_queues_two_ships_paid_up_front_built_in_order() {
    let mut sim = game(true);
    let yard = spawn(&mut sim, "fx_yard", v(0, 0));
    fund(&mut sim, 1000);

    let t0 = sim.tick;
    assert!(build(&mut sim, yard, "fx_cruiser", None)); // 600
    assert_eq!(dilithium(&sim), Fixed::from_int(400));
    assert!(build(&mut sim, yard, "fx_frigate", None)); // 250
    assert_eq!(dilithium(&sim), Fixed::from_int(150));
    // Not enough for another cruiser: rejected, nothing taken, queue unchanged.
    assert!(!build(&mut sim, yard, "fx_cruiser", None));
    assert_eq!(dilithium(&sim), Fixed::from_int(150));
    assert_eq!(sim.production.queue(yard).len(), 2);
    // A def the yard's list does not name is rejected too.
    assert!(!build(&mut sim, yard, "fx_station", None));

    let mut log = Vec::new();
    run(&mut sim, 1400, &mut log);
    let b = built(&log);
    assert_eq!(b.len(), 2, "{log:?}");
    // Queued in the step of tick t0 (the first build call): cruiser 900
    // ticks, then frigate 450 more.
    assert_eq!(b[0].0, t0 + 900);
    assert_eq!(b[1].0, t0 + 900 + 450);
    assert_eq!(sim.unit(b[0].2).unwrap().def, key(&sim, "fx_cruiser"));
    assert_eq!(sim.unit(b[1].2).unwrap().def, key(&sim, "fx_frigate"));
    assert!(b[0].2 < b[1].2);
    assert_eq!(sim.unit(b[0].2).unwrap().owner, P0);
    assert!(sim.production.is_empty());
}

#[test]
fn without_pay_on_queue_the_head_waits_for_funds_and_pays_on_start() {
    let mut sim = game(false);
    let yard = spawn(&mut sim, "fx_yard", v(0, 0));
    assert!(build(&mut sim, yard, "fx_frigate", None));
    let mut log = Vec::new();
    run(&mut sim, 100, &mut log);
    assert!(log.is_empty());
    assert_eq!(
        sim.production.queue(yard),
        vec![(key(&sim, "fx_frigate"), 0)]
    );
    // The funding step's own production pass starts the head.
    let t = sim.tick;
    fund(&mut sim, 300);
    run(&mut sim, 500, &mut log);
    assert_eq!(dilithium(&sim), Fixed::from_int(50));
    assert_eq!(built(&log)[0].0, t + 450);
}

#[test]
fn stop_on_a_yard_refunds_the_queue() {
    let mut sim = game(true);
    let yard = spawn(&mut sim, "fx_yard", v(0, 0));
    fund(&mut sim, 1000);
    build(&mut sim, yard, "fx_cruiser", None);
    build(&mut sim, yard, "fx_frigate", None);
    let mut log = Vec::new();
    run(&mut sim, 50, &mut log);
    sim.step(&[cmd(
        P0,
        9,
        CommandKind::Order(Order::Stop { units: vec![yard] }),
    )]);
    assert_eq!(dilithium(&sim), Fixed::from_int(1000));
    assert!(sim.production.queue(yard).is_empty());
}

#[test]
fn another_players_yard_cannot_be_ordered() {
    let mut sim = game(true);
    let yard = spawn(&mut sim, "fx_yard", v(0, 0));
    fund(&mut sim, 1000);
    let what = key(&sim, "fx_frigate");
    let o = Order::Build {
        builder: yard,
        what,
        at: None,
    };
    let out = sim.step(&[cmd(PlayerId(1), 0, CommandKind::Order(o))]);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    assert_eq!(dilithium(&sim), Fixed::from_int(1000));
}

#[test]
fn rig_deploys_at_the_site_welds_then_reveals_the_station() {
    let mut sim = game(true);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    fund(&mut sim, 1500);
    // A rig needs a site.
    assert!(!build(&mut sim, rig, "fx_station", None));
    assert_eq!(dilithium(&sim), Fixed::from_int(1500));

    let mut log = Vec::new();
    assert!(build(&mut sim, rig, "fx_station", Some(v(400, 0))));
    assert_eq!(dilithium(&sim), Fixed::from_int(300));
    assert!(sim.production.is_deploying(rig));
    let n = sim.units.len();
    run(&mut sim, 2400, &mut log);

    let d = log
        .iter()
        .find(|(_, e)| matches!(e, SimEvent::BuildStarted { .. }))
        .unwrap()
        .0;
    assert!(d > 100, "the rig had to travel: {d}");
    let b = built(&log);
    assert_eq!(b.len(), 1);
    // Built exactly `ticks` after the deploy tick.
    assert_eq!(b[0].0, d + 1800);
    let s = sim.unit(b[0].2).unwrap();
    assert_eq!(s.kind, EntityKind::Station);
    assert_eq!(s.progress, None);
    assert_eq!(s.pos, v(400, 0));
    assert_eq!(sim.units.len(), n + 1);
    assert!(!sim.production.is_deploying(rig));
    assert!(sim.production.is_empty());
}

#[test]
fn station_is_visible_and_unfinished_while_welding() {
    let mut sim = game(true);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    fund(&mut sim, 1500);
    let t0 = sim.tick;
    assert!(build(&mut sim, rig, "fx_station", Some(v(10, 0)))); // already in reach
    let id = EntityId(rig.0 + 1);
    assert_eq!(sim.unit(id).unwrap().progress, Some(Fixed::ZERO));
    let mut log = Vec::new();
    run(&mut sim, 900, &mut log);
    let p = sim.unit(id).unwrap().progress.unwrap();
    assert!(
        p > Fixed::from_f64(0.45) && p < Fixed::from_f64(0.55),
        "{p:?}"
    );
    run(&mut sim, 1000, &mut log);
    assert_eq!(built(&log)[0].0, t0 + 1800);
    assert_eq!(sim.unit(id).unwrap().progress, None);
}

fn welding_fixture() -> (Sim, EntityId, EntityId) {
    welding_fixture_profile(sta_content::Profile::Native)
}

fn welding_fixture_profile(profile: sta_content::Profile) -> (Sim, EntityId, EntityId) {
    let mut sim = game(true);
    let source = sim.content().unwrap().clone();
    let mut content = ContentDb::builder().profile(profile).features(source.features().clone());
    for unit in source.units() { content = content.unit(unit.clone()); }
    for weapon in source.weapons() { content = content.weapon(weapon.clone()); }
    sim.content = Some(Arc::new(content.build().unwrap()));
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    fund(&mut sim, 1500);
    assert!(build(&mut sim, rig, "fx_station", Some(v(10, 0))));
    assert!(sim.production.is_welding(rig));
    (sim, rig, EntityId(rig.0 + 1))
}

#[test]
fn welding_rig_refuses_direct_queued_and_replacement_orders() {
    for profile in [sta_content::Profile::Native, sta_content::Profile::Sta1Classic,
        sta_content::Profile::Sta2Classic, sta_content::Profile::Fo4] {
    let (mut sim, rig, station) = welding_fixture_profile(profile);
    let at = sim.unit(rig).unwrap().pos;
    let stock = dilithium(&sim);
    for (seq, queue) in [false, true].into_iter().enumerate() {
        let out = sim.step(&[cmd(P0, seq as u32, CommandKind::Order(Order::Move {
            units: vec![rig], to: v(1000, 900), queue,
        }))]);
        assert!(out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
        assert_eq!(sim.unit(rig).unwrap().pos, at);
        assert!(sim.movement.course(rig).is_none());
        assert!(sim.unit(station).unwrap().progress.is_some());
        assert_eq!(dilithium(&sim), stock);
    }
    assert!(!build(&mut sim, rig, "fx_station", Some(v(500, 400))));
    assert_eq!(dilithium(&sim), stock);
    assert!(sim.view().unit(rig).unwrap().flags.contains(crate::view::UnitFlags::BUILDING));
    }
}

#[test]
fn welding_holds_velocity_courses_and_collision_position_but_not_other_units() {
    let (mut sim, rig, _station) = welding_fixture();
    let at = sim.unit(rig).unwrap().pos;
    // A restored stale velocity/target or another actor cannot move the rig.
    let unit = sim.units.get_mut(rig).unwrap();
    unit.speed = Fixed::from_int(100);
    unit.omega = Fixed::ONE;
    unit.target = Some(v(1000, 1000));
    let other = spawn(&mut sim, "fx_cruiser", at);
    let out = sim.step(&[cmd(P0, 20, CommandKind::Order(Order::Move {
        units: vec![rig, other], to: v(800, 800), queue: true,
    }))]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    for _ in 0..90 {
        sim.step(&[]);
        let unit = sim.unit(rig).unwrap();
        assert_eq!(unit.pos, at);
        assert_eq!(unit.speed, Fixed::ZERO);
        assert_eq!(unit.omega, Fixed::ZERO);
        assert_eq!(unit.target, None);
    }
    assert_ne!(sim.unit(other).unwrap().pos, at);
}

#[test]
fn stop_cancels_weld_without_refunding_and_releases_move_in_same_batch() {
    let (mut sim, rig, station) = welding_fixture();
    let stock = dilithium(&sim);
    let out = sim.step(&[cmd(PlayerId(1), 0, CommandKind::Order(Order::Stop { units: vec![rig] }))]);
    assert!(out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert!(sim.production.is_welding(rig));
    let out = sim.step(&[
        cmd(P0, 30, CommandKind::Order(Order::Stop { units: vec![rig] })),
        cmd(P0, 31, CommandKind::Order(Order::Move { units: vec![rig], to: v(700, 0), queue: false })),
    ]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert!(out.events.iter().any(|e| matches!(e, SimEvent::Destroyed { id, .. } if *id == station)));
    assert!(sim.unit(station).is_none());
    assert!(!sim.production.is_welding(rig));
    assert_eq!(dilithium(&sim), stock);
    assert!(!sim.view().unit(rig).unwrap().flags.contains(crate::view::UnitFlags::BUILDING));
    assert_eq!(sim.unit(rig).unwrap().target, Some(v(700, 0)));
}

#[test]
fn weld_lock_survives_snapshot_and_completion_releases_it_at_existing_tick() {
    let (mut sim, rig, station) = welding_fixture();
    let db = sim.content().unwrap().clone();
    let mut restored = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
    assert_eq!(sim.digest(), restored.digest());
    let at = sim.unit(rig).unwrap().pos;
    for _ in 0..1800 {
        let a = sim.step(&[]);
        let b = restored.step(&[]);
        assert_eq!(a, b);
        assert_eq!(sim.unit(rig).unwrap().pos, at);
    }
    assert_eq!(sim.unit(station).unwrap().progress, None);
    assert!(!sim.production.is_welding(rig));
    assert!(!sim.view().unit(rig).unwrap().flags.contains(crate::view::UnitFlags::BUILDING));
    let out = sim.step(&[cmd(P0, 50, CommandKind::Order(Order::Move {
        units: vec![rig], to: v(700, 0), queue: false,
    }))]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert_ne!(sim.unit(rig).unwrap().pos, at);
}

#[test]
fn destroying_unfinished_station_releases_constructor_lock() {
    let (mut sim, rig, station) = welding_fixture();
    assert!(sim.despawn(station, None));
    assert!(!sim.production.is_welding(rig));
    let out = sim.step(&[cmd(P0, 60, CommandKind::Order(Order::Move {
        units: vec![rig], to: v(700, 0), queue: false,
    }))]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
}

#[test]
fn welding_rig_stays_fixed_when_a_field_overlaps_its_parked_position() {
    let (mut sim, rig, _station) = welding_fixture();
    let at = sim.unit(rig).unwrap().pos;
    sim.regions.push(sta_content::Region {
        kind: sta_content::RegionKind::AsteroidField,
        odf: "test_field".into(),
        shape: sta_content::RegionShape::Sphere { centre: at, radius: Fixed::from_int(100) },
    });
    for _ in 0..10 {
        sim.step(&[]);
        assert_eq!(sim.unit(rig).unwrap().pos, at);
    }
}

#[test]
fn stop_on_the_unfinished_station_cancels_only_its_weld() {
    let (mut sim, rig, station) = welding_fixture();
    let stock = dilithium(&sim);
    let out = sim.step(&[cmd(P0, 70, CommandKind::Order(Order::Stop { units: vec![station] }))]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert!(sim.unit(station).is_none());
    assert!(!sim.production.is_welding(rig));
    assert!(sim.unit(rig).is_some());
    assert_eq!(dilithium(&sim), stock);
}

#[test]
fn moving_a_rig_cancels_its_deploy_with_a_refund() {
    let mut sim = game(true);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    fund(&mut sim, 1500);
    build(&mut sim, rig, "fx_station", Some(v(400, 0)));
    let o = Order::Move {
        units: vec![rig],
        to: v(0, 300),
        queue: false,
    };
    sim.step(&[cmd(P0, 9, CommandKind::Order(o))]);
    assert_eq!(dilithium(&sim), Fixed::from_int(1500));
    assert!(!sim.production.is_deploying(rig));
    let mut log = Vec::new();
    run(&mut sim, 600, &mut log);
    assert!(log.is_empty());
}

/// Two yards, a ship each, plus a rig welding: all at once.
fn parallel() -> (Sim, Vec<(u64, SimEvent)>, [EntityId; 2], u64) {
    let mut sim = game(true);
    let a = spawn(&mut sim, "fx_yard", v(0, 0));
    let b = spawn(&mut sim, "fx_yard", v(500, 0));
    let rig = spawn(&mut sim, "fx_rig", v(-300, 0));
    fund(&mut sim, 5000);
    let t0 = sim.tick;
    let (c, f, s) = (
        key(&sim, "fx_cruiser"),
        key(&sim, "fx_frigate"),
        key(&sim, "fx_station"),
    );
    let orders = [
        Order::Build {
            builder: a,
            what: c,
            at: None,
        },
        Order::Build {
            builder: b,
            what: f,
            at: None,
        },
        Order::Build {
            builder: rig,
            what: s,
            at: Some(v(-300, 20)),
        },
    ];
    let cmds: Vec<_> = orders
        .into_iter()
        .enumerate()
        .map(|(i, o)| cmd(P0, i as u32, CommandKind::Order(o)))
        .collect();
    let out = sim.step(&cmds);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    let mut log = Vec::new();
    run(&mut sim, 2000, &mut log);
    (sim, log, [a, b], t0)
}

#[test]
fn two_yards_build_in_parallel_and_the_digest_is_pinned() {
    let (sim, log, [a, b], t0) = parallel();
    let got = built(&log);
    assert_eq!(got.len(), 3, "{log:?}");
    // Completed production uses the same initialized canonical crew pool.
    for &(_, _, id) in &got {
        let unit = sim.unit(id).unwrap();
        let definition = sim.content.as_ref().unwrap().unit(unit.def).unwrap();
        assert_eq!(unit.crew, Some([definition.crew.max(Fixed::ZERO); 2]));
    }
    let at = |builder: EntityId| got.iter().find(|g| g.1 == builder).unwrap().0;
    assert_eq!(at(a), t0 + 900);
    assert_eq!(at(b), t0 + 450);
    assert_eq!(got.iter().filter(|g| g.0 == t0 + 1800).count(), 1);
    assert_eq!(dilithium(&sim), Fixed::from_int(5000 - 600 - 250 - 1200));
    assert_eq!(sim.digest(), parallel().0.digest());
    assert_eq!(crate::unit_status_tests::digest_before_crew(&sim), 0x5326_e562_7d02_6437);
    assert_eq!(sim.digest(), PINNED_DIGEST, "digest {:#x}", sim.digest());
}

/// Re-pinned by SIM-21: the rig now parks beside its site (the footprint's
/// edge plus a gap) instead of deploying where it happens to be, so its
/// final position, and the digest, moved. It was `0x1f6ba950228b3166`.
// Initialized unit crew is now part of the canonical state.
const PINNED_DIGEST: u64 = 0x5d7e_9249_c496_f448;

/// A yard 600 ticks into a cruiser (frigate queued behind) and a rig 600
/// ticks into welding a station; for the snapshot test.
pub(crate) fn mid_build() -> (Sim, Arc<ContentDb>) {
    let mut sim = game(true);
    let yard = spawn(&mut sim, "fx_yard", v(0, 0));
    let rig = spawn(&mut sim, "fx_rig", v(-300, 0));
    fund(&mut sim, 5000);
    build(&mut sim, yard, "fx_cruiser", None);
    build(&mut sim, yard, "fx_frigate", None);
    build(&mut sim, rig, "fx_station", Some(v(-300, 20)));
    let mut log = Vec::new();
    run(&mut sim, 600, &mut log);
    let db = Arc::clone(sim.content().unwrap());
    (sim, db)
}

#[test]
fn records_round_trip() {
    let mut sim = game(true);
    let yard = spawn(&mut sim, "fx_yard", v(0, 0));
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    fund(&mut sim, 3000);
    build(&mut sim, yard, "fx_cruiser", None);
    build(&mut sim, yard, "fx_frigate", None);
    build(&mut sim, rig, "fx_station", Some(v(900, 0)));
    let mut log = Vec::new();
    run(&mut sim, 20, &mut log);
    let rec = sim.production.to_rec();
    let bytes = postcard::to_allocvec(&rec).unwrap();
    let back: ProductionRec = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(Production::from_rec(&back).unwrap(), sim.production);
    assert!(!sim.production.is_empty());
}

#[test]
fn deploy_reach_follows_the_rigs_turning_circle() {
    let reach = |speed, omega| deploy_reach(&MoveStats::from_f32(speed, 1.0, omega, 1.0, 0.2));
    // Orbit = 0.1 * 200 / (2 * (0.2 + 0.8 * 0.1)) = 35.7; plus a quarter and the arrival distance.
    let r = reach(200.0, 2.0).to_f32();
    assert!((r - (35.7 * 1.25 + 4.0)).abs() < 0.5, "{r}");
    // A nimble rig has to nearly arrive; a sluggish one deploys from further out.
    assert!(reach(100.0, 20.0) < reach(100.0, 2.0));
    // No turn rate: only the arrival distance, never a panic.
    assert_eq!(reach(100.0, 0.0), crate::arrive_distance());
}

/// A rig whose turning circle is wider than the old fixed reach of 30 cannot
/// close on a site abeam of it, and used to circle it for ever.
#[test]
fn wide_turning_rig_still_deploys() {
    let mut sim = game(true);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    sim.units.get_mut(rig).unwrap().stats = MoveStats::from_f32(200.0, 1.0, 0.5, 1.0, 0.2);
    fund(&mut sim, 1500);
    assert!(build(&mut sim, rig, "fx_station", Some(v(300, 0))));
    let mut log = Vec::new();
    run(&mut sim, 2400, &mut log);
    assert!(
        log.iter()
            .any(|(_, e)| matches!(e, SimEvent::BuildStarted { .. }))
    );
    assert_eq!(built(&log).len(), 1);
}

#[test]
fn a_site_off_the_map_or_under_another_structure_is_refused() {
    let mut sim = game(true);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    spawn(&mut sim, "fx_yard", v(1000, 0));
    fund(&mut sim, 5000);
    // Under the yard, and a little beside it (inside both footprints).
    assert!(!build(&mut sim, rig, "fx_station", Some(v(1000, 0))));
    assert!(!build(&mut sim, rig, "fx_station", Some(v(1020, 10))));
    // Past the empty map's edge (8192).
    assert!(!build(&mut sim, rig, "fx_station", Some(v(8190, 0))));
    assert!(!build(&mut sim, rig, "fx_station", Some(v(-9000, 0))));
    // Refusals cost nothing and leave no deploy.
    assert_eq!(dilithium(&sim), Fixed::from_int(5000));
    assert!(!sim.production.is_deploying(rig));
    // Open space, and the builder itself never blocks.
    assert!(build(&mut sim, rig, "fx_station", Some(v(3000, 3000))));
    assert!(sim.production.is_deploying(rig));
}

#[test]
fn the_ghost_rule_is_the_sims_rule() {
    use super::site::{SiteError, check_site, footprint, view_obstacles};
    let mut sim = game(true);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    spawn(&mut sim, "fx_yard", v(1000, 0));
    fund(&mut sim, 5000);
    let db = sim.content().unwrap().clone();
    let what = key(&sim, "fx_station");
    let view = sim.view();
    let bounds = (v(-8192, -8192), v(8192, 8192));
    let status = |at| check_site(&db, Some(&bounds), what, at, view_obstacles(&db, &view, Some(rig)));
    assert_eq!(status(v(3000, 3000)), Ok(()));
    assert_eq!(status(v(1000, 20)), Err(SiteError::Blocked));
    assert_eq!(status(v(8200, 0)), Err(SiteError::OffMap));
    // Just clear of the yard's footprint plus the station's.
    let gap = footprint(&db, what) + footprint(&db, key(&sim, "fx_yard"));
    assert_eq!(status(Vec3 { x: v(1000, 0).x + gap + Fixed::ONE, ..v(0, 0) }), Ok(()));
    // The sim agrees on each.
    for (at, ok) in [(v(3000, 3000), true), (v(1000, 20), false), (v(8200, 0), false)] {
        assert_eq!(build(&mut sim, rig, "fx_station", Some(at)), ok, "{at:?}");
    }
}

/// SIM-21: the structure goes up at the chosen point, the rig parks beside it
/// (outside the footprint, neither pushed), and leaves when done.
#[test]
fn rig_parks_beside_its_site_and_neither_is_pushed() {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = true;
    let mut rig = UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    rig.radius = Fixed::from_int(60); // body 45
    let mut b = ContentDb::builder().profile(fake.profile()).features(features).unit(rig);
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        if u.odf == "fx_station" {
            u.radius = Fixed::from_int(200); // body 150
        }
        b = b.unit(u);
    }
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(b.build().unwrap()),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    fund(&mut sim, 1500);
    let site = v(600, 0);
    assert!(build(&mut sim, rig, "fx_station", Some(site)));
    let gap = |sim: &Sim, id| {
        let p = sim.unit(id).unwrap().pos;
        let (dx, dz) = (p.x - site.x, p.z - site.z);
        (dx * dx + dz * dz).sqrt().to_f32() as f64
    };
    let mut log = Vec::new();
    run(&mut sim, 300, &mut log);
    let station = EntityId(rig.0 + 1);
    assert_eq!(sim.unit(station).unwrap().pos, site, "the structure stands on the chosen point");
    // Beside the footprint: outside station body plus rig body, a gap and the
    // deploy reach from it.
    let ring = 150.0 + 45.0 + 10.0;
    let d = gap(&sim, rig);
    // Welding now holds at the deployment boundary instead of coasting closer
    // after BuildStarted. Its outer limit is the rig's authored deploy reach.
    let reach = deploy_reach(&sim.unit(rig).unwrap().stats).to_f32() as f64;
    assert!(d >= 150.0 + 45.0 && d <= ring + reach + 1.0, "rig {d} from the site");
    let rig_at = sim.unit(rig).unwrap().pos;
    run(&mut sim, 2400, &mut log);
    assert_eq!(sim.unit(rig).unwrap().pos, rig_at, "the rig is not pushed while it welds");
    assert_eq!(sim.unit(station).unwrap().pos, site);
    assert_eq!(sim.unit(station).unwrap().progress, None, "built");
    // Afterwards the rig leaves without touching the structure.
    let to = Order::Move { units: vec![rig], to: v(0, 600), queue: false };
    sim.step(&[cmd(P0, 9, CommandKind::Order(to))]);
    for i in 0..600 {
        sim.step(&[]);
        let g = gap(&sim, rig);
        // The rig faces the structure and drifts ahead a little as it turns
        // away; it never gets more than 20 units into the structure's body (own
        // structures are not solid to their owner's craft).
        assert!(g >= 130.0, "rig inside the structure: {g} at {i}, rig {:?} was {rig_at:?}", sim.unit(rig).unwrap().pos);
    }
}
