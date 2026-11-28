//! The gates on entering and staying at warp (#1102): the engines system's share of hit
//! points, the tick's speed ceiling, and the rules file's distance for a warp move. On the
//! approach scene's ships (`approach_tests::db`: impulse 140, warp 400) with authored system
//! hit points, an authored slowing nebula and authored rule values.

use std::sync::Arc;

use sta_content::{ApproachRules, ContentDb, MapDef, PathingRules, Profile, Region, RegionKind, RegionShape, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};

use crate::command::{Command, CommandKind, Edit, Order, Subsystem};
use crate::movement::approach::{IN_WARP, WARP_ENGINES_SHARE, ceiling_allows_warp, engines_share_met};
use crate::movement::approach_tests;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

const HZ: u32 = TICK_HZ;
/// The engines system's hit points of the test ship.
const ENGINES_HP: i32 = 100;
const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}

/// A nebula definition from authored ODF text.
fn nebula(odf: &str, text: &str) -> UnitDef {
    let text = format!("classLabel = \"nebula_obj\"\nunitName = \"{odf}\"\n{text}");
    let stems = [odf.to_owned()];
    let (db, _) = sta_content::odf::load_with(Default::default(), &Default::default(), &stems, &mut |_, version| (version == 0).then(|| text.clone())).unwrap();
    let mut u = db.units().next().expect("a nebula_obj with unitName is a unit def").clone();
    u.odf = odf.to_owned();
    u
}

/// The approach scene's content under stock Armada II rules: its ships with engines hit
/// points, a nebula that halves a craft's speed (`gate_drag`), the path planner off (it
/// would route round the nebula), and `too_close` as the warp move's distance key when given.
fn db(too_close: Option<i32>) -> Arc<ContentDb> {
    db_with(too_close, true)
}

/// [`db`] with the feature switch `warp_gates` as given (the profile has it on).
fn db_with(too_close: Option<i32>, gates: bool) -> Arc<ContentDb> {
    let base = approach_tests::db(Profile::Sta2Classic, Some(1100));
    assert!(base.features().warp_gates, "on for stock Armada II");
    let mut features = base.features().clone();
    features.warp_gates = gates;
    let mut b = ContentDb::builder().profile(base.profile()).features(features);
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    for u in base.units() {
        let mut u = u.clone();
        u.system_hit_points[0] = n(ENGINES_HP);
        b = b.unit(u);
    }
    b = b.unit(nebula("gate_drag", "craft_max_speed_fraction = 0.5\n"));
    let mut approach = base.rules().approach();
    if let Some(d) = too_close {
        approach.too_close_to_warp = n(d);
    }
    let rules = base.rules().clone().with_approach(approach).with_pathing(PathingRules { enabled: false, ..Default::default() });
    Arc::new(b.build().unwrap().with_rules(rules))
}

fn game(db: &Arc<ContentDb>, regions: Vec<Region>) -> Sim {
    let mut map = MapDef::empty();
    map.regions = regions;
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "st".into(), control: Control::Idle };
    Sim::from_setup(&GameSetup { seed: 5, map: Arc::new(map), content: Arc::clone(db), players: vec![seat(1), seat(2)], options: GameOptions::default() })
}

fn ship(sim: &mut Sim, db: &ContentDb, at: Vec3) -> EntityId {
    sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), at, AHEAD).unwrap()
}

fn warp_move(id: EntityId, to: Vec3) -> Command {
    Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::MoveWarp { units: vec![id], to, queue: false }) }
}

fn script(e: Edit) -> Command {
    Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(e) }
}

fn at_warp(sim: &Sim, id: EntityId) -> bool {
    sim.approach_warp.phase(id) == Some(IN_WARP)
}

/// Steps `ticks` ticks, `first` going in on the first; whether the craft was ever at warp.
fn run(sim: &mut Sim, first: Vec<Command>, id: EntityId, ticks: u32) -> bool {
    let mut warped = false;
    for i in 0..ticks {
        sim.step(if i == 0 { &first } else { &[] });
        warped |= at_warp(sim, id);
    }
    warped
}

/// Damage that leaves the engines system exactly at the share, in hit points.
fn damage_to_the_share() -> Fixed {
    n(ENGINES_HP) - n(ENGINES_HP) * Fixed::from_int(WARP_ENGINES_SHARE.0 as i32) / Fixed::from_int(WARP_ENGINES_SHARE.1 as i32)
}

#[test]
fn the_share_is_met_at_the_threshold_and_not_just_below() {
    let hp = n(ENGINES_HP);
    let at = damage_to_the_share().to_bits();
    assert!(engines_share_met(hp, 0));
    assert!(engines_share_met(hp, at), "exactly at the share");
    assert!(!engines_share_met(hp, at + 1), "one bit below");
    assert!(!engines_share_met(hp, hp.to_bits()), "destroyed");
    assert!(!engines_share_met(hp, hp.to_bits() + 5), "damage beyond the maximum");
    assert!(engines_share_met(hp, -5), "negative damage is no damage");
    // STA64's own choice: an engines system without hit points cannot be damaged and passes.
    assert!(engines_share_met(Fixed::ZERO, 0) && engines_share_met(Fixed::ZERO, 1 << 40));
}

#[test]
fn a_craft_enters_warp_with_its_engines_at_the_share_and_not_just_below() {
    let db = db(None);
    let damaged = |extra: Fixed| {
        let mut sim = game(&db, Vec::new());
        let s = ship(&mut sim, &db, v(0, 0));
        sim.step(&[script(Edit::SubsystemDamage(s, Subsystem::Engines, damage_to_the_share() + extra))]);
        let warped = run(&mut sim, vec![warp_move(s, v(0, 6000))], s, 8 * HZ);
        let u = sim.unit(s).unwrap();
        (warped, u.pos.z, u.speed <= u.stats.max_speed)
    };
    let (warped, far, _) = damaged(Fixed::ZERO);
    assert!(warped, "at the share the craft warps");
    let (warped, near, at_impulse) = damaged(Fixed::from_bits(1));
    assert!(!warped, "just below the share it does not");
    assert!(at_impulse && near > n(500) && near < far, "it flies the move at impulse: {near:?} against {far:?}");
}

#[test]
fn a_craft_at_warp_leaves_it_when_its_engines_fall_below_the_share_and_returns_when_they_are_back() {
    let db = db(None);
    let mut sim = game(&db, Vec::new());
    let s = ship(&mut sim, &db, v(0, 0));
    run(&mut sim, vec![warp_move(s, v(0, 8000))], s, 4 * HZ);
    assert!(at_warp(&sim, s));
    let impulse = sim.unit(s).unwrap().stats.max_speed;
    assert!(sim.unit(s).unwrap().speed > impulse);

    let hurt = damage_to_the_share() + Fixed::ONE;
    sim.step(&[script(Edit::SubsystemDamage(s, Subsystem::Engines, hurt))]);
    assert!(!at_warp(&sim, s), "out of the warp phase on the tick the gate fails");
    assert!(!run(&mut sim, Vec::new(), s, 3 * HZ), "and it stays out");
    let u = sim.unit(s).unwrap();
    assert!(u.speed <= impulse && u.speed > Fixed::ZERO, "it goes on at impulse: {:?}", u.speed);
    assert!(u.target.is_some(), "the order stands");

    // Repaired to the share: warp resumes by itself.
    sim.step(&[script(Edit::SubsystemDamage(s, Subsystem::Engines, -Fixed::ONE))]);
    assert!(run(&mut sim, Vec::new(), s, 4 * HZ), "back at warp without a new order");
}

#[test]
fn the_ceiling_must_lie_strictly_above_impulse() {
    let impulse = n(140);
    assert!(ceiling_allows_warp(impulse, None), "no cap, no ceiling");
    assert!(ceiling_allows_warp(impulse, Some((n(100), n(141)))));
    assert!(!ceiling_allows_warp(impulse, Some((n(100), n(140)))), "a ceiling at impulse blocks");
    assert!(!ceiling_allows_warp(impulse, Some((n(70), n(70)))), "and one below it");
}

fn drag(z: i32, radius: i32) -> Region {
    Region { kind: RegionKind::Nebula, odf: "gate_drag".into(), shape: RegionShape::Sphere { centre: v(0, z), radius: n(radius) } }
}

/// What a craft did on its way through the slowing nebula across its path.
struct Crossing {
    /// At warp on a tick before it reached the nebula.
    before: bool,
    /// Ticks that began well inside the nebula, and how many of them ended in the warp phase.
    inside: usize,
    inside_at_warp: usize,
    /// It came out on the far side.
    crossed: bool,
    sim: Sim,
    ship: EntityId,
}

/// The nebula of [`cross`]: its centre and radius along the craft's path.
const NEBULA: (i32, i32) = (2500, 600);

/// A craft at the origin given `order` (built for it and, when asked, a target 6000 ahead),
/// flown until it is past the nebula. Everything lies inside the empty map's bounds: a goal
/// beyond them would be clamped.
fn cross(db: &Arc<ContentDb>, attack: bool) -> Crossing {
    let (centre, radius) = NEBULA;
    let mut sim = game(db, vec![drag(centre, radius)]);
    let s = ship(&mut sim, db, v(0, 0));
    let order = if attack {
        let t = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, 6000), AHEAD).unwrap();
        Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![s], target: t }) }
    } else {
        warp_move(s, v(0, 7800))
    };
    sim.step(&[order]);
    let mut c = Crossing { before: false, inside: 0, inside_at_warp: 0, crossed: false, sim, ship: s };
    for _ in 0..120 * HZ {
        // The cap of a tick comes from where the craft is when the tick starts. "Well inside"
        // keeps a margin from the edge, so the test does not depend on how the edge is counted.
        let z = c.sim.unit(s).unwrap().pos.z;
        let margin = 100;
        let was_inside = z > n(centre - radius + margin) && z < n(centre + radius - margin);
        c.sim.step(&[]);
        if was_inside {
            c.inside += 1;
            c.inside_at_warp += usize::from(at_warp(&c.sim, s));
        } else if z < n(centre - radius) {
            c.before |= at_warp(&c.sim, s);
        }
        if z > n(centre + radius) {
            c.crossed = true;
            break;
        }
    }
    c
}

/// A craft at warp that comes under a nebula's speed cap is out of the warp phase on every
/// tick the cap applies, and warps again by itself past it. (Entering warp under such a cap
/// was already impossible for these craft: the cap keeps them below the speed a warp start
/// asks for. What the ceiling adds is the craft that is already at warp.)
#[test]
fn a_speed_cap_at_or_below_impulse_drops_a_warping_craft() {
    let mut c = cross(&db(None), false);
    assert!(c.before, "at warp on the way to the nebula");
    assert!(c.crossed && c.inside > HZ as usize, "the craft crossed the nebula ({} ticks inside)", c.inside);
    assert_eq!(c.inside_at_warp, 0, "never in the warp phase on a tick the cap applies");
    assert!(run(&mut c.sim, Vec::new(), c.ship, 6 * HZ), "past the nebula it warps again");
}

/// The same on the attack approach.
#[test]
fn an_attack_approach_drops_out_of_warp_under_a_speed_cap() {
    let c = cross(&db(None), true);
    assert!(c.before, "the approach warps on the way to the nebula");
    assert!(c.crossed && c.inside > HZ as usize, "the attacker crossed the nebula ({} ticks inside)", c.inside);
    assert_eq!(c.inside_at_warp, 0, "never in the warp phase on a tick the cap applies");
}

#[test]
fn the_warp_move_reads_its_distance_from_the_rules() {
    // A goal closer than the authored key and farther than the default one.
    let default = ApproachRules::DEFAULT_TOO_CLOSE_TO_WARP;
    let (key, goal) = (4000, 3000);
    assert!(n(goal) > default + n(1000) && goal < key);
    let warps_to = |db: &Arc<ContentDb>, z: i32| {
        let mut sim = game(db, Vec::new());
        let s = ship(&mut sim, db, v(0, 0));
        run(&mut sim, vec![warp_move(s, v(0, z))], s, 10 * HZ)
    };
    let (plain, keyed) = (db(None), db(Some(key)));
    assert!(warps_to(&plain, goal), "without the key the default distance applies");
    assert!(!warps_to(&keyed, goal), "with the key the goal is too close for warp");
    assert!(warps_to(&keyed, 3 * key), "and a goal beyond it still warps");
    assert_ne!(plain.hash(), keyed.hash(), "the key is part of the content digest");
}

/// The same gate on the attack approach: an attacker far from its target warps towards it with
/// its engines at the share, and flies the approach at impulse just below it.
#[test]
fn an_attack_approach_warps_only_with_the_engines_at_the_share() {
    let db = db(None);
    let approach = |extra: Fixed| {
        let mut sim = game(&db, Vec::new());
        let s = ship(&mut sim, &db, v(0, 0));
        let t = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, 6000), AHEAD).unwrap();
        sim.step(&[script(Edit::SubsystemDamage(s, Subsystem::Engines, damage_to_the_share() + extra))]);
        let attack = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![s], target: t }) };
        run(&mut sim, vec![attack], s, 8 * HZ)
    };
    assert!(approach(Fixed::ZERO), "at the share the approach warps");
    assert!(!approach(Fixed::from_bits(1)), "just below it does not");
}

/// With the switch off neither gate applies: damaged engines and a nebula's cap leave the warp
/// decision as it was before the rule (a craft keeps its warp phase under the cap, on both
/// paths).
#[test]
fn with_the_switch_off_neither_gate_applies() {
    let off = db_with(None, false);
    let mut sim = game(&off, Vec::new());
    let s = ship(&mut sim, &off, v(0, 0));
    sim.step(&[script(Edit::SubsystemDamage(s, Subsystem::Engines, damage_to_the_share() + Fixed::ONE))]);
    assert!(run(&mut sim, vec![warp_move(s, v(0, 6000))], s, 8 * HZ), "engines below the share still warp");

    for attack in [false, true] {
        let c = cross(&off, attack);
        assert!(c.before && c.inside > 0, "attack {attack}: {} ticks inside", c.inside);
        assert_eq!(c.inside_at_warp, c.inside, "attack {attack}: the warp phase is kept under the cap, as before");
    }

    // The switch is part of the content digest, and the profiles that have it on are the two
    // whose executable the reading covers.
    assert_ne!(off.hash(), db(None).hash());
    let on: Vec<Profile> = [Profile::Native, Profile::Fo4, Profile::Fo3, Profile::Sta2Classic, Profile::Sta1Classic].into_iter().filter(|p| p.features().warp_gates).collect();
    assert_eq!(on, [Profile::Fo4, Profile::Sta2Classic]);
}
