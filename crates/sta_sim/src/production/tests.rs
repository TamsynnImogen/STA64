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
    let at = |builder: EntityId| got.iter().find(|g| g.1 == builder).unwrap().0;
    assert_eq!(at(a), t0 + 900);
    assert_eq!(at(b), t0 + 450);
    assert_eq!(got.iter().filter(|g| g.0 == t0 + 1800).count(), 1);
    assert_eq!(dilithium(&sim), Fixed::from_int(5000 - 600 - 250 - 1200));
    assert_eq!(sim.digest(), parallel().0.digest());
    assert_eq!(sim.digest(), PINNED_DIGEST, "digest {:#x}", sim.digest());
}

const PINNED_DIGEST: u64 = 0x1f6ba950228b3166;

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
