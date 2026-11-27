//! Save and load in the middle of a match (issue #1032, part of the first step
//! towards savegames): run to tick n, take the snapshot, load it into a fresh
//! sim, continue. The continued run must give the same digest on every tick,
//! the same events and view, and at the end the same snapshot bytes as the
//! uninterrupted run.
//!
//! Every scenario is run three ways and compared tick by tick:
//!
//! - (a) uninterrupted, the reference;
//! - (b) the snapshot taken at tick n, restored into a FRESH sim built from
//!   independently built content (nothing carried over in memory: path-planner
//!   caches and every other memo start cold, as in a loading process), then
//!   continued with the same scripted commands;
//! - (c) as (b), with the snapshot bytes written to a file under the build's
//!   own temp directory and read back first.
//!
//! The comparison is the sim digest (`TickOutput::view.digest`) on every tick,
//! the events of the tick, the whole view (units, players, visibility, ...),
//! and at the end the full snapshot bytes. Right after each restore the sim
//! must also save to the same bytes it was loaded from (idempotence).
//!
//! Covered, each as its own test so a failure names the area: nav and path
//! state round an obstacle, a formation group, order queues and scripted
//! orders (patrol, guard), combat in flight and a unit dying across the save
//! point, random-number state, production and construction, resources and
//! mining timers, the stuck-order watchdog, sensor visibility, and one long
//! mixed scenario saved at many ticks. Further tests pin: saving twice gives
//! the same bytes, restore then save gives the original bytes, a snapshot of a
//! sim with hot caches equals one of a sim restored cold, and a sim reloaded
//! from its own snapshot every k ticks stays on the uninterrupted run.
//!
//! Also covered, one small test each: alerts, autonomy, subsystem switches,
//! shield and hull edits, speed override, names and flags, mission switches,
//! relations and target permissions, a stock edit; a victory (elimination)
//! between save points; a station being decommissioned.
//!
//! Controls: three `#[should_panic]` tests drive the same `resume` helper with
//! a deliberate fault (a command withheld after the restore, a snapshot of
//! another tick, a snapshot of a run with another seed) and must fail it.
//!
//! Not covered (outside the sim snapshot): controller and AI state (`sta_ai`).
//! The mixed scenario's "AI seat" is only the sim's `ai_seats` flag; NO
//! controller runs and every command comes from the fixed script, so it says
//! nothing about saving AI players. Also mission Lua state and the client side
//! (explored-fog memory, camera, selection). Sensor visibility is derived from
//! sim state each tick and is compared as part of the view.
//!
//! Not exercised by any scenario here (the authored fakes cannot set it up, or
//! the order is rejected): boarding and transporters, colonies and population,
//! special weapons and cloaks, ship repair queues, station recrew (the order is
//! rejected for the fake station), decommission of a craft at a yard, approach
//! warp, the Director's Cut, start-helper expiry and replacements, subsystem
//! hit points and zero-shield holds, nebula effects (no nebula definition in
//! the fakes), and `Edit::Seed` (the sim rejects it). A loss in those cannot
//! show here; several have their own snapshot tests inside the crate.
//!
//! What the digest does not cover (so only the final snapshot bytes and the
//! save-restore-save equality catch a loss there): the stuck-order watchdog
//! rows, appearance factions, the map's start positions, regions and bounds,
//! derived ship-upgrade multipliers and the path-planner cache. The sim offers
//! no way to drop its caches in place, so "cold" means "restored from bytes".
//!
//! Authored data only: the fake content (`sta_content::fake`) plus a rig and
//! body sizes, and synthetic maps. No game files are read.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MoveStats, Region, RegionKind, RegionShape, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{AlertState, AutonomyKind, AutonomyLevel, Command, CommandKind, Edit, MissionSwitch, Order, Subsystem, Target, TeamOp};
use sta_sim::view::Relation;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::world::NEUTRAL;
use sta_sim::{Sim, SimEvent, TickOutput, WorldSource};

const P0: u8 = 0;
const P1: u8 = 1;

// ---------------------------------------------------------------- content

/// The fake content with body sizes (as the planner tests use them) and a
/// construction rig. Built anew on every call: a loading process builds its own.
fn content() -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = true;
    let mut rig = UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    let mut b = ContentDb::builder().profile(fake.profile()).features(features).unit(rig);
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        u.radius = Fixed::from_int(match u.odf.as_str() {
            "fx_cruiser" => 100,
            "fx_moon" => 400,
            _ => 0,
        });
        b = b.unit(u);
    }
    Arc::new(b.build().expect("authored content is consistent"))
}

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn seat(id: u8, control: Control) -> PlayerSetup {
    PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control }
}

fn world(seed: u64, regions: Vec<Region>, enemy: Control) -> Sim {
    let mut map = MapDef::empty();
    map.regions = regions;
    Sim::from_setup(&GameSetup {
        seed,
        map: Arc::new(map),
        content: content(),
        players: vec![seat(P0, Control::Human), seat(P1, enemy)],
        options: GameOptions::default(),
    })
}

fn field(x: i32, z: i32, r: i32) -> Region {
    Region {
        kind: RegionKind::AsteroidField,
        odf: "fx_asteroids".into(),
        shape: RegionShape::Sphere { centre: v(x, z), radius: Fixed::from_int(r) },
    }
}

fn put(sim: &mut Sim, odf: &str, owner: PlayerId, x: i32, z: i32) -> EntityId {
    let key = sim.content().expect("content").key(odf).expect("authored unit");
    sim.spawn_def(key, owner, v(x, z), (Fixed::ZERO, Fixed::ONE)).expect("spawns")
}

fn mine(sim: &mut Sim, odf: &str, x: i32, z: i32) -> EntityId {
    put(sim, odf, PlayerId(P0), x, z)
}

fn theirs(sim: &mut Sim, odf: &str, x: i32, z: i32) -> EntityId {
    put(sim, odf, PlayerId(P1), x, z)
}

// ----------------------------------------------------------------- script

/// Commands by the tick they are issued at (the step from tick t to t+1).
#[derive(Default)]
struct Script {
    at: BTreeMap<u64, Vec<Command>>,
    seq: u32,
}

impl Script {
    fn push(&mut self, tick: u64, player: PlayerId, kind: CommandKind) {
        self.seq += 1;
        self.at.entry(tick).or_default().push(Command { player, seq: self.seq, kind });
    }
    fn order(&mut self, tick: u64, player: u8, o: Order) {
        self.push(tick, PlayerId(player), CommandKind::Order(o));
    }
    fn edit(&mut self, tick: u64, e: Edit) {
        self.push(tick, PlayerId::BENCH, CommandKind::Edit(e));
    }
    fn go(&mut self, tick: u64, player: u8, units: &[EntityId], to: Vec3, queue: bool) {
        self.order(tick, player, Order::Move { units: units.to_vec(), to, queue });
    }
    fn commands(&self, tick: u64) -> &[Command] {
        self.at.get(&tick).map_or(&[][..], Vec::as_slice)
    }
}

struct Scenario {
    name: &'static str,
    sim: Sim,
    script: Script,
    ticks: u64,
}

// ---------------------------------------------------------------- harness

/// The uninterrupted run: every tick's output, the snapshots asked for and the
/// bytes at the end.
struct Reference {
    name: &'static str,
    script: Script,
    ticks: u64,
    start_digest: u64,
    outs: Vec<TickOutput>,
    snaps: BTreeMap<u64, Vec<u8>>,
    last: Vec<u8>,
}

fn run_reference(sc: Scenario, saves: &[u64]) -> Reference {
    let Scenario { name, mut sim, script, ticks } = sc;
    let start_digest = sim.digest();
    let mut snaps = BTreeMap::new();
    let mut outs = Vec::new();
    if saves.contains(&0) {
        snaps.insert(0, sim.snapshot());
    }
    for t in 0..ticks {
        outs.push(sim.step(script.commands(t)));
        if saves.contains(&(t + 1)) {
            snaps.insert(t + 1, sim.snapshot());
        }
    }
    let last = sim.snapshot();
    Reference { name, script, ticks, start_digest, outs, snaps, last }
}

/// The bytes written to a file under the build's temp directory and read back.
/// The file name carries the scenario, the save tick and a process-wide counter,
/// so no two calls (in parallel tests) ever share a path.
fn through_file(name: &str, n: u64, bytes: &[u8]) -> Vec<u8> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("save-round-trip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}-{n}-{}.stas", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, bytes).expect("write the save file");
    let back = std::fs::read(&path).expect("read the save file");
    std::fs::remove_file(&path).expect("remove the save file");
    back
}

/// A deliberate fault for the controls: the harness must fail on each.
enum Tamper {
    Nothing,
    /// The commands of this tick are not delivered to the resumed run.
    Withhold(u64),
    /// This snapshot, of another run (another seed) at the same tick, is loaded.
    OtherRun(Vec<u8>),
    /// The snapshot of this other tick is loaded in place of the named one.
    OtherSnapshot(u64),
}

/// Restore the snapshot taken at tick `n` into a fresh sim and continue.
fn resume(r: &Reference, n: u64, file: bool) {
    resume_tampered(r, n, file, &Tamper::Nothing);
}

fn resume_tampered(r: &Reference, n: u64, file: bool, tamper: &Tamper) {
    let (name, via) = (r.name, if file { "file" } else { "memory" });
    let saved = match tamper {
        Tamper::OtherSnapshot(m) => &r.snaps[m],
        Tamper::OtherRun(bytes) => bytes,
        _ => &r.snaps[&n],
    };
    let bytes = if file { through_file(name, n, saved) } else { saved.clone() };
    assert!(&bytes == saved, "{name}: the file round trip changed the bytes (save at {n})");
    let mut sim = Sim::from_snapshot(&bytes, Some(content()))
        .unwrap_or_else(|e| panic!("{name}: save at {n} ({via}) does not load: {e:?}"));
    if !matches!(tamper, Tamper::OtherSnapshot(_) | Tamper::OtherRun(_)) {
        assert_eq!(sim.tick, n, "{name}: loaded tick");
    }
    let want = if n == 0 { r.start_digest } else { r.outs[n as usize - 1].view.digest };
    assert_eq!(sim.digest(), want, "{name}: digest right after loading the save at {n} ({via})");
    assert!(&sim.snapshot() == saved, "{name}: loading then saving the save at {n} ({via}) gives other bytes");
    for t in n..r.ticks {
        let mut cmds = r.script.commands(t).to_vec();
        match tamper {
            Tamper::Withhold(w) if *w == t => cmds.clear(),
            _ => {}
        }
        let out = sim.step(&cmds);
        let want = &r.outs[t as usize];
        assert_eq!(out.view.digest, want.view.digest, "{name}: save at {n} ({via}), digest of tick {t}");
        assert_eq!(out.events, want.events, "{name}: save at {n} ({via}), events of tick {t}");
        assert!(out == *want, "{name}: save at {n} ({via}), view of tick {t} differs although the digest is equal");
    }
    assert!(sim.snapshot() == r.last, "{name}: save at {n} ({via}), the bytes at the end differ");
}

/// Reference run plus every resume: in memory for each save, through a file for
/// every `file_every`-th.
fn check(sc: Scenario, saves: &[u64], file_every: usize) -> Reference {
    let r = run_reference(sc, saves);
    for (i, &n) in saves.iter().enumerate() {
        resume(&r, n, false);
        if i % file_every == 0 {
            resume(&r, n, true);
        }
    }
    r
}

/// The tick at which the first matching event is emitted (the output index).
fn first_event(outs: &[TickOutput], what: &str, f: impl Fn(&SimEvent) -> bool) -> u64 {
    outs.iter()
        .position(|o| o.events.iter().any(&f))
        .unwrap_or_else(|| panic!("the scenario never produces {what}")) as u64
}

fn count_events(outs: &[TickOutput], f: impl Fn(&SimEvent) -> bool) -> usize {
    outs.iter().map(|o| o.events.iter().filter(|e| f(e)).count()).sum()
}

fn pos_of(outs: &[TickOutput], id: EntityId, tick: usize) -> Option<Vec3> {
    outs[tick].view.unit(id).map(|u| u.pos)
}

/// Speed of `id` in the state at tick `n` (zero when it is gone).
fn speed_at(r: &Reference, n: u64, id: EntityId) -> Fixed {
    r.outs[n as usize - 1].view.unit(id).map_or(Fixed::ZERO, |u| u.speed)
}

/// The save ticks `ticks` must catch `ids` flying: live state, not an idle phase.
fn assert_flying(r: &Reference, ticks: &[u64], ids: &[EntityId]) {
    for &n in ticks {
        for &id in ids {
            assert!(speed_at(r, n, id) > Fixed::ZERO, "{}: {id:?} is not flying at the save at tick {n}", r.name);
        }
    }
}

fn probe(make: impl Fn() -> Scenario) -> Vec<TickOutput> {
    run_reference(make(), &[]).outs
}

fn around(t: u64, ticks: u64) -> Vec<u64> {
    let mut v: Vec<u64> = [t.saturating_sub(1), t, t + 1].into_iter().filter(|&n| n >= 1 && n <= ticks).collect();
    v.dedup();
    v
}

// -------------------------------------------------------------- scenarios

/// Ships flying round a moon and asteroid fields (the planner's grid, goal
/// fields, the avoidance steering), crossing each other's path.
fn route() -> (Scenario, [EntityId; 3]) {
    let mut sim = world(3, vec![field(700, 3000, 300), field(-700, 900, 300)], Control::Idle);
    put(&mut sim, "fx_moon", NEUTRAL, 0, 1500);
    let a = mine(&mut sim, "fx_cruiser", 0, 0);
    let b = mine(&mut sim, "fx_frigate", 300, 0);
    let c = theirs(&mut sim, "fx_cruiser", 0, 3600);
    let mut script = Script::default();
    script.go(0, P0, &[a], v(0, 3200), false);
    script.go(0, P0, &[b], v(-100, 3300), false);
    script.go(0, P1, &[c], v(0, -300), false);
    (Scenario { name: "route", sim, script, ticks: 1500 }, [a, b, c])
}

#[test]
fn nav_and_path_state_round_an_obstacle() {
    let (sc, [a, b, c]) = route();
    let r = check(sc, &[1, 37, 200, 450, 700, 1000], 2);
    // Live state: all three are under a move order and flying at these saves
    // (the route itself is not exposed; the bend is asserted below).
    assert_flying(&r, &[37, 200, 450], &[a, b, c]);
    // The route bends round the moon: it is not the straight line.
    let widest = (0..r.ticks as usize).filter_map(|t| pos_of(&r.outs, a, t)).map(|p| p.x.abs()).max().unwrap();
    assert!(widest > Fixed::from_int(300), "the ship flew straight through: {widest:?}");
}

/// A mixed group in formation ordered round the same obstacles, then given a
/// second formation and a new destination while still flying.
fn formation() -> (Scenario, Vec<EntityId>) {
    let mut sim = world(5, vec![field(700, 2600, 300)], Control::Idle);
    put(&mut sim, "fx_moon", NEUTRAL, 0, 1500);
    let mut group = Vec::new();
    for (i, odf) in ["fx_cruiser", "fx_cruiser", "fx_frigate", "fx_cruiser", "fx_frigate", "fx_frigate"].into_iter().enumerate() {
        group.push(mine(&mut sim, odf, -250 + 100 * i as i32, 0));
    }
    let mut script = Script::default();
    script.order(0, P0, Order::Formation { units: group.clone(), kind: 3 });
    script.go(0, P0, &group, v(200, 3200), false);
    script.go(1, P0, &group, v(-1500, 3500), true);
    script.order(500, P0, Order::Formation { units: group.clone(), kind: 2 });
    script.go(500, P0, &group, v(0, 600), false);
    (Scenario { name: "formation", sim, script, ticks: 1600 }, group)
}

#[test]
fn a_formation_group_with_its_leader_round_an_obstacle() {
    let (sc, group) = formation();
    let r = check(sc, &[1, 60, 250, 501, 800, 1200], 2);
    // Live state: the group is under way (not yet in place) at these saves,
    // before and after the second formation order at tick 500.
    assert_flying(&r, &[60, 250], &group);
    assert!(group.iter().any(|&id| speed_at(&r, 501, id) > Fixed::ZERO), "nobody flies at the save at 501");
    // They flew: every ship ended far from where it started.
    for id in group {
        let end = pos_of(&r.outs, id, r.ticks as usize - 1).expect("alive");
        assert!(end.z > Fixed::from_int(300) || end.x.abs() > Fixed::from_int(500), "{id:?} did not move: {end:?}");
    }
}

/// Order queues mid-execution and scripted orders: queued waypoints, a patrol,
/// a guard that is stopped later, a queued leg added after the savepoints.
fn orders() -> Scenario {
    let mut sim = world(9, vec![], Control::Idle);
    let q = mine(&mut sim, "fx_cruiser", 0, 0);
    let p = mine(&mut sim, "fx_frigate", -200, 0);
    let g = mine(&mut sim, "fx_cruiser", 200, 0);
    let mut script = Script::default();
    script.go(0, P0, &[q], v(1000, 0), false);
    script.go(0, P0, &[q], v(1000, 1000), true);
    script.go(0, P0, &[q], v(0, 1000), true);
    script.go(0, P0, &[q], v(0, 0), true);
    script.order(0, P0, Order::Patrol { units: vec![p], points: vec![v(-800, -800), v(800, -800), v(800, 800)] });
    script.order(0, P0, Order::Guard { units: vec![g], ward: q });
    script.order(700, P0, Order::Stop { units: vec![g] });
    script.go(900, P0, &[q], v(-600, 600), true);
    Scenario { name: "orders", sim, script, ticks: 1800 }
}

#[test]
fn queued_orders_and_scripted_orders_mid_execution() {
    let r = check(orders(), &[1, 45, 333, 701, 1000, 1500], 2);
    assert!(r.outs.iter().all(|o| o.view.units.len() == 3));
    // Live state at the saves: the queue ship and the patrol are flying at 45
    // and 333 (the queue itself is not exposed), the patrol later too.
    let ids: Vec<EntityId> = r.outs[0].view.units.iter().map(|u| u.id).collect();
    assert_flying(&r, &[45, 333], &[ids[0], ids[1]]);
    assert_flying(&r, &[1000, 1500], &[ids[1]]);
    // The patrol (second unit) really flies its legs, past the save points.
    let patrol = r.outs[0].view.units[1].id;
    let xs: Vec<Fixed> = (0..r.ticks as usize).filter_map(|t| pos_of(&r.outs, patrol, t)).map(|p| p.x).collect();
    assert!(*xs.iter().max().unwrap() - *xs.iter().min().unwrap() > Fixed::from_int(800), "the patrol did not fly");
}

// Controls for the harness itself: each goes through the same `resume` the
// scenarios use, with a deliberate fault, and must fail the comparison.

#[test]
#[should_panic(expected = "digest of tick 700")]
fn control_a_command_withheld_after_the_restore_is_noticed() {
    let r = run_reference(orders(), &[333]);
    resume_tampered(&r, 333, false, &Tamper::Withhold(700));
}

#[test]
#[should_panic(expected = "digest right after loading the save at 701")]
fn control_a_snapshot_of_another_tick_is_noticed() {
    let r = run_reference(orders(), &[333, 701]);
    resume_tampered(&r, 701, true, &Tamper::OtherSnapshot(333));
}

#[test]
#[should_panic(expected = "digest right after loading the save at 333")]
fn control_a_run_with_a_different_seed_is_noticed() {
    let r = run_reference(combat(7), &[333]);
    let other = run_reference(combat(8), &[333]);
    resume_tampered(&r, 333, false, &Tamper::OtherRun(other.snaps[&333].clone()));
}

/// Two cruisers against two frigates: beams, torpedoes in flight, shield
/// damage and kills.
fn combat(seed: u64) -> Scenario {
    let mut sim = world(seed, vec![], Control::Idle);
    let c1 = mine(&mut sim, "fx_cruiser", 2000, 2000);
    let c2 = mine(&mut sim, "fx_cruiser", 2100, 2000);
    let f1 = theirs(&mut sim, "fx_frigate", 2000, 2400);
    let f2 = theirs(&mut sim, "fx_frigate", 2150, 2450);
    let mut script = Script::default();
    script.order(0, P0, Order::Attack { units: vec![c1], target: f1 });
    script.order(0, P0, Order::Attack { units: vec![c2], target: f2 });
    Scenario { name: "combat", sim, script, ticks: 1500 }
}

#[test]
fn combat_in_flight_and_a_unit_dying_across_the_save() {
    let outs = probe(|| combat(7));
    let fired = first_event(&outs, "a shot", |e| matches!(e, SimEvent::Fired { .. }));
    let hit = first_event(&outs, "a hit", |e| matches!(e, SimEvent::Hit { .. }));
    let dead = first_event(&outs, "a kill", |e| matches!(e, SimEvent::Destroyed { .. }));
    let mut saves = vec![1, 5, 40, 120];
    saves.extend(around(fired + 3, 1500));
    saves.extend(around(hit, 1500));
    saves.extend(around(dead, 1500));
    saves.push(dead + 60);
    saves.sort_unstable();
    saves.dedup();
    let r = check(combat(7), &saves, 3);
    assert!(count_events(&r.outs, |e| matches!(e, SimEvent::Destroyed { .. })) >= 1);
    // Live state: damage is pending at the save after the first hit and a unit
    // is alive at the save just before the kill and gone at the one after it.
    let damaged = |n: u64| {
        r.outs[n as usize - 1].view.units.iter().any(|u| u.hull[0] < u.hull[1] || u.shields.iter().any(|s| s[0] < s[1]))
    };
    assert!(damaged(hit + 1), "nothing is damaged at the save after the first hit");
    let alive = |n: u64| r.outs[n as usize - 1].view.units.len();
    assert!(alive(dead) > alive(dead + 1), "the kill does not fall between the saves at {dead} and {}", dead + 1);
    assert!(
        r.outs[(fired as usize)..=(fired as usize + 3)].iter().any(|o| o.events.iter().any(|e| matches!(e, SimEvent::Fired { .. }))),
        "no shot is fired just before the save at {}",
        fired + 3
    );
}

/// The sim's own random draws (hits, casualties) decide the outcome: another
/// seed gives another game, and a reseed after the save point is replayed.
#[test]
fn random_number_state_decides_the_outcome_across_a_reload() {
    let end = |seed| run_reference(combat(seed), &[]).outs.last().unwrap().view.digest;
    let digests: Vec<u64> = [1, 2, 3].into_iter().map(end).collect();
    assert!(
        digests[0] != digests[1] || digests[1] != digests[2],
        "no seed changes the combat: the scenario does not draw from the sim's random state"
    );
    // (`Edit::Seed` is not applied by the sim today: it is always rejected, so
    // it is not used here.)
    for seed in [2, 3] {
        check(combat(seed), &[1, 100, 299, 300, 301, 600, 1000], 3);
    }
}

/// A yard with two ships queued, a rally point, and a rig welding a station.
fn production() -> Scenario {
    let mut sim = world(3, vec![], Control::Idle);
    let yard = mine(&mut sim, "fx_yard", 0, 0);
    let rig = mine(&mut sim, "fx_rig", -300, 0);
    let what = |sim: &Sim, odf: &str| sim.content().unwrap().key(odf).unwrap();
    let (cruiser, frigate, station) = (what(&sim, "fx_cruiser"), what(&sim, "fx_frigate"), what(&sim, "fx_station"));
    let mut script = Script::default();
    script.edit(0, Edit::Cargo(PlayerId(P0), 0, Fixed::from_int(5000)));
    script.order(2, P0, Order::Build { builder: yard, what: cruiser, at: None });
    script.order(2, P0, Order::Build { builder: yard, what: frigate, at: None });
    script.order(2, P0, Order::Build { builder: rig, what: station, at: Some(v(-300, 20)) });
    script.order(50, P0, Order::Rally { builders: vec![yard], at: Target::Point(v(500, 500)) });
    Scenario { name: "production", sim, script, ticks: 2000 }
}

#[test]
fn production_and_construction_in_progress() {
    let outs = probe(production);
    let first = first_event(&outs, "a finished build", |e| matches!(e, SimEvent::Built { .. }));
    let started = first_event(&outs, "a build start", |e| matches!(e, SimEvent::BuildStarted { .. }));
    assert!(count_events(&outs, |e| matches!(e, SimEvent::Built { .. })) >= 2, "two builds finish");
    let mut saves = vec![1, 3, 51, 100, 450, 600];
    saves.extend(around(first, 2000));
    saves.extend(around(started + 450, 2000));
    saves.extend([1300, 1790, 1850]);
    saves.sort_unstable();
    saves.dedup();
    let r = check(production(), &saves, 3);
    // Live state: something is under construction (progress strictly between
    // 0 and 1 in the view) at these saves.
    for n in [100u64, 450, 600] {
        let building = r.outs[n as usize - 1]
            .view
            .units
            .iter()
            .any(|u| u.progress.is_some_and(|p| p > Fixed::ZERO && p < Fixed::ONE));
        assert!(building, "nothing is under construction at the save at tick {n}");
    }
}

/// Two freighters mining two moons for a station: cargo, trips, docking
/// queue, delivery timers and the stock; a stock edit after the save points.
fn mining() -> Scenario {
    let mut sim = world(3, vec![], Control::Idle);
    let moon_a = put(&mut sim, "fx_moon", NEUTRAL, 0, 0);
    let moon_b = put(&mut sim, "fx_moon", NEUTRAL, 0, -600);
    mine(&mut sim, "fx_station", 800, -300);
    let m1 = mine(&mut sim, "fx_miner", 0, 0);
    let m2 = mine(&mut sim, "fx_miner", 0, -600);
    let mut script = Script::default();
    script.order(0, P0, Order::Mine { units: vec![m1], node: moon_a });
    script.order(0, P0, Order::Mine { units: vec![m2], node: moon_b });
    script.edit(800, Edit::Cargo(PlayerId(P0), 0, Fixed::from_int(10)));
    Scenario { name: "mining", sim, script, ticks: 2600 }
}

#[test]
fn resources_and_timers_mid_mining() {
    let outs = probe(mining);
    let delivered = first_event(&outs, "a delivery", |e| matches!(e, SimEvent::Delivered { .. }));
    let deliveries = count_events(&outs, |e| matches!(e, SimEvent::Delivered { .. }));
    assert!(deliveries >= 2, "{deliveries} deliveries");
    let mut saves = vec![1, 100, 500, 799, 800, 801, 1500, 2300];
    saves.extend(around(delivered, 2600));
    saves.sort_unstable();
    saves.dedup();
    let r = check(mining(), &saves, 3);
    // Live state: across the saves a freighter is under way at some and
    // working (mining or unloading, at rest) at others; ids 4 and 5 are the
    // two freighters (spawn order). Phase and cargo are not in the view.
    let (m1, m2) = (EntityId(4), EntityId(5));
    let flying = saves.iter().filter(|&&n| speed_at(&r, n, m1) > Fixed::ZERO || speed_at(&r, n, m2) > Fixed::ZERO).count();
    let at_rest = saves.iter().filter(|&&n| speed_at(&r, n, m1) == Fixed::ZERO || speed_at(&r, n, m2) == Fixed::ZERO).count();
    assert!(flying > 0 && at_rest > 0, "saves catch no trip ({flying}) or no work phase ({at_rest})");
    let stock = |t: usize| r.outs[t].view.players[0].resources.clone();
    assert_ne!(stock(0), stock(2599), "the stock changes over the run");
}

/// A craft walled in by four fields with an order it cannot fly: the stuck
/// detector's counters run for two minutes and then end the order.
fn walled() -> Scenario {
    let mut sim = world(3, vec![field(380, 0, 300), field(-380, 0, 300), field(0, 380, 300), field(0, -380, 300)], Control::Idle);
    let s = mine(&mut sim, "fx_cruiser", 0, 0);
    let mut script = Script::default();
    script.go(0, P0, &[s], v(0, 3000), false);
    Scenario { name: "walled", sim, script, ticks: 4200 }
}

#[test]
fn the_stuck_order_watchdog_counters() {
    let outs = probe(walled);
    let ended = first_event(&outs, "an abandoned order", |e| matches!(e, SimEvent::OrderAbandoned { .. }));
    let mut saves = vec![1, 100, 1800, 3000];
    saves.extend(around(ended, 4200));
    saves.push((ended + 50).min(4200));
    saves.sort_unstable();
    saves.dedup();
    let r = check(walled(), &saves, 2);
    // The counters (`watchdog::Row`) are not readable through the public API
    // or the view, so liveness is shown by position: the saves cover the whole
    // two-minute window (a craft still holding a move order, the order alive)
    // and the ones after the abandon; the craft is alive all along.
    let early = saves.iter().filter(|&&n| n < ended).count();
    assert!(early >= 3, "only {early} saves fall inside the stuck window");
    assert!(r.outs.iter().all(|o| o.view.units.len() == 1), "the craft is lost");
}

/// Two ships of two owners passing each other at the edge of sensor range.
fn passing() -> (Scenario, EntityId) {
    let mut sim = world(3, vec![], Control::Idle);
    let a = mine(&mut sim, "fx_cruiser", -2000, 0);
    let b = theirs(&mut sim, "fx_cruiser", 2000, 800);
    let mut script = Script::default();
    script.go(0, P0, &[a], v(2000, 0), false);
    script.go(0, P1, &[b], v(-2000, 800), false);
    (Scenario { name: "passing", sim, script, ticks: 900 }, b)
}

#[test]
fn sensor_visibility_across_a_reload() {
    let (sc, b) = passing();
    let r = check(sc, &[1, 60, 150, 300, 450, 600], 2);
    let sees = |t: usize| {
        r.outs[t].view.visibility.iter().find(|(p, _)| *p == PlayerId(P0)).is_some_and(|(_, ids)| ids.contains(&b))
    };
    let seen = (0..r.ticks as usize).filter(|&t| sees(t)).count();
    assert!(seen > 0 && seen < r.ticks as usize, "visibility changes during the run ({seen} ticks of {})", r.ticks);
}

/// Everything at once: two bases and an AI seat, a formation group sent round
/// a moon, mining, a yard and a rig, a patrol, an attack on the enemy station.
fn mixed(ticks: u64) -> Scenario {
    let mut sim = world(11, vec![field(800, 800, 300)], Control::Ai("skirmish".into()));
    put(&mut sim, "fx_moon", NEUTRAL, 0, 1500);
    let node = put(&mut sim, "fx_moon", NEUTRAL, -2000, -1300);
    mine(&mut sim, "fx_station", -2500, -1000);
    let yard = mine(&mut sim, "fx_yard", -3000, -1000);
    let rig = mine(&mut sim, "fx_rig", -2800, -700);
    let m1 = mine(&mut sim, "fx_miner", -2000, -1300);
    let m2 = mine(&mut sim, "fx_miner", -2400, -1000);
    let fleet: Vec<_> = [("fx_cruiser", -1600), ("fx_cruiser", -1500), ("fx_cruiser", -1400), ("fx_frigate", -1550), ("fx_frigate", -1450)]
        .into_iter()
        .map(|(odf, x)| mine(&mut sim, odf, x, -200))
        .collect();
    let scout = mine(&mut sim, "fx_frigate", -1000, -1500);
    let base = theirs(&mut sim, "fx_station", 3000, 2500);
    for (odf, x, z) in [("fx_cruiser", 2500, 2300), ("fx_cruiser", 2600, 2300), ("fx_frigate", 2500, 2500), ("fx_frigate", 2600, 2500)] {
        theirs(&mut sim, odf, x, z);
    }
    let key = |odf: &str| sim.content().unwrap().key(odf).unwrap();
    let (cruiser, frigate, station) = (key("fx_cruiser"), key("fx_frigate"), key("fx_station"));
    let mut script = Script::default();
    script.edit(0, Edit::Cargo(PlayerId(P0), 0, Fixed::from_int(3000)));
    script.edit(0, Edit::Cargo(PlayerId(P1), 0, Fixed::from_int(1000)));
    script.order(0, P0, Order::Mine { units: vec![m1], node });
    script.order(0, P0, Order::Mine { units: vec![m2], node });
    script.order(0, P0, Order::Formation { units: fleet.clone(), kind: 3 });
    script.go(0, P0, &fleet, v(1500, 2200), false);
    script.order(0, P0, Order::Patrol { units: vec![scout], points: vec![v(-1000, -1500), v(-200, -2500), v(600, -1200)] });
    script.order(2, P0, Order::Build { builder: yard, what: cruiser, at: None });
    script.order(2, P0, Order::Build { builder: yard, what: frigate, at: None });
    script.order(2, P0, Order::Build { builder: rig, what: station, at: Some(v(-2800, -650)) });
    script.order(300, P0, Order::Rally { builders: vec![yard], at: Target::Point(v(-1500, 0)) });
    script.order(900, P0, Order::Attack { units: fleet[..3].to_vec(), target: base });
    script.order(1200, P0, Order::Stop { units: vec![fleet[3]] });
    script.order(1500, P0, Order::Formation { units: fleet.clone(), kind: 2 });
    script.go(1500, P0, &fleet, v(-1500, 0), false);
    Scenario { name: "mixed", sim, script, ticks }
}

#[test]
fn one_long_mixed_scenario_saved_at_many_ticks() {
    let saves: Vec<u64> = (0..13).map(|i| 1 + i * 157).collect();
    let r = check(mixed(2000), &saves, 4);
    assert!(count_events(&r.outs, |e| matches!(e, SimEvent::Fired { .. })) > 0, "there is combat");
    assert!(count_events(&r.outs, |e| matches!(e, SimEvent::Delivered { .. })) > 0, "there is mining");
    assert!(count_events(&r.outs, |e| matches!(e, SimEvent::Built { .. })) > 0, "there is production");
    // Live state: at most saves some craft is moving, at some a build is in
    // progress, at some a shot was fired in the last 30 ticks.
    let moving = |n: u64| r.outs[n as usize - 1].view.units.iter().any(|u| u.speed > Fixed::ZERO);
    let building = |n: u64| r.outs[n as usize - 1].view.units.iter().any(|u| u.progress.is_some_and(|p| p > Fixed::ZERO && p < Fixed::ONE));
    let shooting = |n: u64| r.outs[(n as usize).saturating_sub(30)..n as usize].iter().any(|o| o.events.iter().any(|e| matches!(e, SimEvent::Fired { .. })));
    let (m, b, s) = (
        saves.iter().filter(|&&n| moving(n)).count(),
        saves.iter().filter(|&&n| building(n)).count(),
        saves.iter().filter(|&&n| shooting(n)).count(),
    );
    assert!(m * 4 >= saves.len() * 3, "only {m} of {} saves catch movement", saves.len());
    assert!(b >= 2 && s >= 1, "saves catch {b} builds and {s} fights");
}

// ------------------------------------------ further state, one test each

fn rejected(outs: &[TickOutput]) -> usize {
    count_events(outs, |e| matches!(e, SimEvent::Rejected { .. }))
}

/// The combat world with the mission and bench edits and orders that set
/// small pieces of state: alert levels, autonomy, subsystem switches, shield
/// percent, a speed override, names and flags, mission switches, relations,
/// target permissions, a stock edit and a hull edit; all after or around the
/// save points.
fn edits() -> Scenario {
    let mut sc = combat(7);
    let ids: Vec<EntityId> = sc.sim.units.iter().map(|u| u.id).collect();
    let (c1, c2, f1, f2) = (ids[0], ids[1], ids[2], ids[3]);
    let s = &mut sc.script;
    s.order(5, P0, Order::Alert { units: vec![c1, c2], level: AlertState::Red });
    s.order(10, P0, Order::Autonomy { units: vec![c1], kind: AutonomyKind::Movement, level: AutonomyLevel::High });
    s.edit(50, Edit::Subsystem(c2, Subsystem::Weapons, false));
    s.edit(200, Edit::Subsystem(c2, Subsystem::Weapons, true));
    s.edit(60, Edit::ShieldPercent(f1, Fixed::from_f64(0.5)));
    s.edit(80, Edit::MaxSpeed(c1, Some(Fixed::from_int(50))));
    s.edit(90, Edit::Name(f2, "Delta".into()));
    s.edit(90, Edit::SpecialForces(c1, true));
    s.edit(100, Edit::MissionSwitch(MissionSwitch::PathPlanning, false));
    s.edit(100, Edit::MissionSwitch(MissionSwitch::AllowWarp, false));
    s.edit(120, Edit::TeamPool(PlayerId(P0), TeamOp::ResourceAdd(0), Fixed::from_int(10)));
    s.edit(130, Edit::AllowAsTarget { id: f1, player: PlayerId(P0), allowed: false });
    s.edit(140, Edit::Hull(c1, Fixed::from_int(500)));
    s.edit(150, Edit::SetRelation { from: PlayerId(P1), to: PlayerId(P0), relation: Relation::Neutral });
    s.edit(300, Edit::SetRelation { from: PlayerId(P1), to: PlayerId(P0), relation: Relation::Enemy });
    s.edit(320, Edit::MaxSpeed(c1, None));
    sc.name = "edits";
    sc
}

#[test]
fn alerts_autonomy_switches_relations_and_other_scripted_edits() {
    let r = check(edits(), &[1, 6, 55, 85, 101, 135, 151, 250, 301, 400, 900], 3);
    assert_eq!(rejected(&r.outs), 0, "every scripted edit is accepted, so the state it sets is live");
}

/// A cruiser kills the enemy's only craft: the seat is eliminated (victory
/// state and event) between the save points.
fn duel() -> Scenario {
    let mut sim = world(3, vec![], Control::Idle);
    let c = mine(&mut sim, "fx_cruiser", 2000, 2000);
    let f = theirs(&mut sim, "fx_frigate", 2000, 2400);
    let mut script = Script::default();
    script.order(0, P0, Order::Attack { units: vec![c], target: f });
    Scenario { name: "victory", sim, script, ticks: 1200 }
}

#[test]
fn a_victory_condition_firing_across_a_reload() {
    let outs = probe(duel);
    let out = first_event(&outs, "an elimination", |e| matches!(e, SimEvent::PlayerEliminated { .. }));
    let mut saves = vec![1, 100];
    saves.extend(around(out, 1200));
    saves.push(out + 40);
    saves.sort_unstable();
    saves.dedup();
    check(duel(), &saves, 2);
}

/// A station decommissioned in place (a recycle job with a timer and a refund).
#[test]
fn decommission_in_progress() {
    let mut sim = world(3, vec![], Control::Idle);
    let station = mine(&mut sim, "fx_station", 1000, 0);
    let mut script = Script::default();
    script.order(10, P0, Order::Decommission { units: vec![station], yard: None });
    let sc = Scenario { name: "decommission", sim, script, ticks: 1500 };
    let r = check(sc, &[1, 11, 50, 200, 600, 1000], 2);
    assert_eq!(rejected(&r.outs), 0, "the order is accepted");
    assert!(r.outs[1499].view.units.is_empty() || r.outs[1499].view.units.len() < r.outs[0].view.units.len(), "the station was recycled");
}

// ------------------------------------------------- bytes and cold caches

fn mixed_at(ticks: u64) -> Sim {
    let Scenario { mut sim, script, .. } = mixed(ticks);
    for t in 0..ticks {
        sim.step(script.commands(t));
    }
    sim
}

#[test]
fn saving_twice_in_a_row_gives_identical_bytes() {
    for n in [0, 1, 250, 900, 1500] {
        let sim = mixed_at(n);
        let (first, second) = (sim.snapshot(), sim.snapshot());
        assert!(first == second, "two saves of the same sim at tick {n} differ");
    }
}

#[test]
fn restore_then_save_gives_the_bytes_it_was_loaded_from() {
    for n in [0, 1, 250, 900, 1500] {
        let saved = mixed_at(n).snapshot();
        let once = Sim::from_snapshot(&saved, Some(content())).expect("loads");
        let again = Sim::from_snapshot(&once.snapshot(), Some(content())).expect("loads again");
        assert!(once.snapshot() == saved, "restore then save at tick {n} changes the bytes");
        assert!(again.snapshot() == saved, "restoring twice at tick {n} changes the bytes");
        assert_eq!(once.digest(), again.digest());
    }
}

/// A sim that ran with hot caches and one restored cold (its path-planner
/// grids, goal fields and every other memo empty) save to the same bytes.
#[test]
fn a_hot_sim_and_a_cold_restored_one_save_the_same_bytes() {
    let hot = mixed_at(700);
    let cold = Sim::from_snapshot(&hot.snapshot(), Some(content())).expect("loads");
    assert_eq!(hot.digest(), cold.digest());
    assert!(hot.snapshot() == cold.snapshot());
}

/// Reloading from its own snapshot every k ticks never moves the game off the
/// uninterrupted run: k = 1 (a cold start every tick) and a prime k.
#[test]
fn a_sim_reloaded_every_k_ticks_stays_on_the_uninterrupted_run() {
    for (k, ticks) in [(1u64, 150u64), (7, 1400)] {
        let reference = run_reference(mixed(ticks), &[]);
        let Scenario { mut sim, script, .. } = mixed(ticks);
        // One content database for all the loads (a loading process builds
        // it once), built apart from the one the original sim holds.
        let db = content();
        for t in 0..ticks {
            let out = sim.step(script.commands(t));
            let want = &reference.outs[t as usize];
            assert_eq!(out.view.digest, want.view.digest, "k = {k}, tick {t}");
            assert_eq!(out.events, want.events, "k = {k}, tick {t}");
            if (t + 1) % k == 0 {
                sim = Sim::from_snapshot(&sim.snapshot(), Some(Arc::clone(&db))).expect("reloads");
            }
        }
        assert!(sim.snapshot() == reference.last, "k = {k}: bytes at the end");
    }
}
