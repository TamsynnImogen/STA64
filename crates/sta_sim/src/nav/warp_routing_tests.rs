//! `warp_routing` (#1148): a craft that wants warp plans its way round the warp-suppression
//! fields of planets. On the approach scene's ships (`approach_tests::db`) and an authored
//! planet; the header's multiplier, buffer and cost are authored here. Nothing is game data.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, Profile, Rules, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order};
use crate::movement::approach::{WARP_OUT, warp_lead};
use crate::movement::approach_tests;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

const HZ: u32 = TICK_HZ;
const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);
/// The authored planet: its bounding radius, the header's multiplier and buffer.
const PLANET_RADIUS: i32 = 100;
const MULTIPLIER: i32 = 3;
const BUFFER: i32 = 60;
/// The planner's finest cell in the authored rules (the default resolution).
const CELL: i32 = 50;

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}
fn distance(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

/// The field's radius, and the radius the planner keeps a craft of bounding radius `hull` out of.
fn field() -> Fixed {
    n(PLANET_RADIUS * MULTIPLIER)
}
fn footprint(hull: Fixed) -> Fixed {
    field() + n(BUFFER) + hull
}

/// The approach scene's content with a planet class and a header that sets the field
/// multiplier, the buffer and the cost (`cost`: the header's text for it, `None`: no cost key).
fn db(routing: bool, cost: Option<&str>) -> Arc<ContentDb> {
    let base = approach_tests::db(Profile::Sta2Classic, Some(1100));
    let mut header = format!("int cfgPLANET_DEWARP_RADIUS_MULTIPLIER = {MULTIPLIER};\nfloat cfg_WARP_INHIBITION_BUFFER = {BUFFER}.0;\n");
    if let Some(cost) = cost {
        header.push_str(&format!("int DEFAULT_TERRAIN_VALUE = 100;\nint cfg_WARP_INHIBITION_COST = {cost};\n"));
    }
    let (rules, warnings) = Rules::from_header(&header);
    assert!(warnings.is_empty(), "{warnings:?}");
    let rules = rules.with_approach(base.rules().approach());
    let mut features = base.features().clone();
    assert!(features.warp_routing, "on for stock Armada II");
    features.warp_routing = routing;
    let mut b = ContentDb::builder().profile(base.profile()).features(features);
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    for u in base.units() {
        b = b.unit(u.clone());
    }
    let planet = UnitDef {
        raw: Arc::new(sta_odf::resolve("st_planet", &mut |_| Some("classLabel = \"planet\"\n".to_owned())).unwrap()),
        radius: n(PLANET_RADIUS),
        hull: n(1000),
        ..UnitDef::new("st_planet", "planet")
    };
    Arc::new(b.unit(planet).build().unwrap().with_rules(rules))
}

fn game(db: &Arc<ContentDb>, planets: &[Vec3]) -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "st".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 5, map: Arc::new(MapDef::empty()), content: Arc::clone(db), players: vec![seat(1), seat(2)], options: GameOptions::default() });
    for p in planets {
        sim.spawn_def(db.key("st_planet").unwrap(), PlayerId::BENCH, *p, AHEAD).unwrap();
    }
    sim
}

fn craft(sim: &mut Sim, at: Vec3) -> EntityId {
    let db = Arc::clone(sim.content.as_ref().unwrap());
    sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), at, AHEAD).unwrap()
}

fn warp_move(ids: &[EntityId], to: Vec3) -> Command {
    Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::MoveWarp { units: ids.to_vec(), to, queue: false }) }
}

/// What a craft did on its way.
#[derive(Debug, Default)]
struct Flight {
    /// Its nearest approach to any planet's centre.
    nearest: Option<Fixed>,
    /// Ticks at warp (or entering it), and ticks at warp inside a field as the gate counts it.
    at_warp: u32,
    /// How often it left warp, and how far from its goal each time.
    leaves: Vec<Fixed>,
    /// Its position when it was level with the first planet (`z` crossing the planet's).
    passing: Option<Vec3>,
    arrived: bool,
    end: Vec3,
    /// The whole-state digest after every tick.
    digests: Vec<u64>,
}

/// Steps until the craft `id` has arrived (at most two minutes), after `first`.
fn fly(sim: &mut Sim, first: Vec<Command>, id: EntityId, goal: Vec3, planets: &[Vec3]) -> Flight {
    let mut f = Flight::default();
    let mut was_out = false;
    for i in 0..120 * HZ {
        let before = sim.unit(id).unwrap().pos;
        sim.step(if i == 0 { &first } else { &[] });
        f.digests.push(sim.digest());
        let u = sim.unit(id).unwrap();
        for p in planets {
            let d = distance(u.pos, *p);
            f.nearest = Some(f.nearest.map_or(d, |m| m.min(d)));
        }
        if let Some(p) = planets.first()
            && f.passing.is_none()
            && before.z < p.z
            && u.pos.z >= p.z
        {
            f.passing = Some(u.pos);
        }
        f.at_warp += u32::from(sim.approach_warp.contains(id));
        let out = sim.approach_warp.phase(id) == Some(WARP_OUT);
        if out && !was_out {
            f.leaves.push(distance(u.pos, goal));
        }
        was_out = out;
        if u.target.is_none() && u.speed == Fixed::ZERO && i > HZ {
            f.arrived = true;
            break;
        }
    }
    f.end = sim.unit(id).unwrap().pos;
    f
}

/// One craft on a warp move from `start` to `goal` past `planets`.
fn one(db: &Arc<ContentDb>, planets: &[Vec3], start: Vec3, goal: Vec3) -> Flight {
    let mut sim = game(db, planets);
    let s = craft(&mut sim, start);
    fly(&mut sim, vec![warp_move(&[s], goal)], s, goal, planets)
}

fn hull(db: &ContentDb) -> Fixed {
    db.unit(db.key("st_seer").unwrap()).unwrap().radius
}

/// The lead before its goal at which the scene's craft leaves warp.
fn lead(db: &ContentDb) -> Fixed {
    let m = db.unit(db.key("st_seer").unwrap()).unwrap().movement;
    warp_lead(m.warp_speed, m.max_speed, hull(db))
}

#[test]
fn only_stock_armada_ii_has_the_switch_on() {
    let on: Vec<Profile> = [Profile::Native, Profile::Fo4, Profile::Fo3, Profile::Sta2Classic, Profile::Sta1Classic].into_iter().filter(|p| p.features().warp_routing).collect();
    assert_eq!(on, [Profile::Sta2Classic]);
}

#[test]
fn a_warp_move_aimed_through_a_field_goes_round_it_at_warp_and_leaves_warp_once_near_its_goal() {
    let db = db(true, None);
    let (planet, goal) = (v(0, 0), v(0, 3000));
    let f = one(&db, &[planet], v(0, -3000), goal);
    assert!(f.arrived && distance(f.end, goal) < n(20), "it arrives: {:?}", f.end);
    // never nearer than the field with its buffer and the hull, less a planner cell
    let nearest = f.nearest.unwrap();
    assert!(nearest >= footprint(hull(&db)) - n(CELL), "nearest {nearest:?}, footprint {:?}", footprint(hull(&db)));
    // and not in a wide berth either: it goes round, it does not flee
    assert!(nearest < footprint(hull(&db)) + n(6 * CELL), "nearest {nearest:?}");
    // at warp for the way (far longer than the time at impulse would allow), and it left warp
    // once: at the lead before the real goal, not before an aim point of its detour
    assert_eq!(f.leaves.len(), 1, "left warp {:?}", f.leaves);
    assert!(f.leaves[0] < lead(&db) + n(30) && f.leaves[0] > lead(&db) - n(30), "left warp {:?} before the goal, the lead is {:?}", f.leaves[0], lead(&db));
    assert!(f.at_warp > 10 * HZ, "{} ticks at warp", f.at_warp);
}

#[test]
fn a_field_off_the_line_is_passed_on_the_nearer_side() {
    let db = db(true, None);
    for side in [1, -1] {
        // the planet lies to one side of the line; the way round is on the other
        let planet = v(150 * side, 0);
        let f = one(&db, &[planet], v(0, -3000), v(0, 3000));
        assert!(f.arrived && f.leaves.len() == 1, "side {side}: {f:?}");
        assert!(f.nearest.unwrap() >= footprint(hull(&db)) - n(CELL), "side {side}: nearest {:?}", f.nearest);
        let passing = f.passing.expect("it passes the planet");
        assert!(passing.x * n(side) < Fixed::ZERO, "side {side}: it passed at x {:?}", passing.x);
    }
}

#[test]
fn a_goal_inside_a_field_is_flown_to_and_warp_ends_at_the_fields_edge() {
    let db = db(true, None);
    let (planet, goal) = (v(0, 0), v(0, -200));
    let f = one(&db, &[planet], v(0, -3000), goal);
    assert!(f.arrived && distance(f.end, goal) < n(20), "it arrives: {:?}", f.end);
    // straight in: no way round a field the goal lies in
    assert!(f.end.x.abs() < n(5), "{:?}", f.end);
    assert_eq!(f.leaves.len(), 1);
    // it left warp where the gate stops it or the lead before its goal does, whichever came first
    let edge = field() + hull(&db) - (field() - n(200));
    assert!(f.leaves[0] <= lead(&db).max(edge) + n(30), "left warp {:?} before the goal", f.leaves[0]);
}

#[test]
fn two_fields_with_no_room_between_are_passed_round_both_and_a_wide_gap_is_flown_through() {
    let db = db(true, None);
    let reach = footprint(hull(&db));
    // the footprints overlap: the way is round the pair
    let near = [v(-340, 0), v(340, 0)];
    let f = one(&db, &near, v(0, -3000), v(0, 3000));
    assert!(f.arrived && f.leaves.len() == 1, "{f:?}");
    assert!(f.nearest.unwrap() >= reach - n(CELL));
    let passing = f.passing.unwrap();
    assert!(passing.x.abs() > n(340) + reach - n(CELL), "it passed between them at x {:?}", passing.x);
    // room for the craft between the footprints: straight through
    let far = [v(-900, 0), v(900, 0)];
    let f = one(&db, &far, v(0, -3000), v(0, 3000));
    assert!(f.arrived && f.leaves.len() == 1, "{f:?}");
    assert!(f.passing.unwrap().x.abs() < n(5), "it did not fly straight through: {:?}", f.passing);
}

/// The fields cost, they do not block: with a cost barely above open space the way through is
/// the cheaper one, and the craft crosses, leaving warp inside the field as the gate has it.
#[test]
fn a_cost_low_enough_makes_the_way_through_the_cheaper_one() {
    let (planet, start, goal) = (v(150, 0), v(0, -3000), v(0, 3000));
    let dear = one(&db(true, Some("DEFAULT_TERRAIN_VALUE * 50")), &[planet], start, goal);
    assert!(dear.nearest.unwrap() >= footprint(n(12)) - n(CELL) && dear.leaves.len() == 1, "{dear:?}");
    let cheap = one(&db(true, Some("DEFAULT_TERRAIN_VALUE + 1")), &[planet], start, goal);
    assert!(cheap.arrived, "{cheap:?}");
    assert!(cheap.nearest.unwrap() < field(), "it crossed the field: nearest {:?}", cheap.nearest);
    assert!(cheap.leaves.len() >= 2, "it left warp in the field and again before its goal: {:?}", cheap.leaves);
    // a cost at or below open space is no reason for a way round at all
    let free = one(&db(true, Some("DEFAULT_TERRAIN_VALUE")), &[planet], start, goal);
    assert!(free.nearest.unwrap() < field());
}

#[test]
fn every_craft_of_a_group_warp_move_keeps_out_of_the_field() {
    let db = db(true, None);
    let planet = v(0, 0);
    let mut sim = game(&db, &[planet]);
    let ids: Vec<EntityId> = [-80, 0, 80].into_iter().map(|x| craft(&mut sim, v(x, -3000))).collect();
    let goal = v(0, 3000);
    let first = warp_move(&ids, goal);
    let mut nearest = n(100_000);
    let mut warped = vec![false; ids.len()];
    for i in 0..120 * HZ {
        sim.step(if i == 0 { std::slice::from_ref(&first) } else { &[] });
        for (k, id) in ids.iter().enumerate() {
            nearest = nearest.min(distance(sim.unit(*id).unwrap().pos, planet));
            warped[k] |= sim.approach_warp.contains(*id);
        }
        if ids.iter().all(|id| sim.unit(*id).is_some_and(|u| u.target.is_none() && u.speed == Fixed::ZERO)) && i > HZ {
            break;
        }
    }
    assert!(warped.iter().all(|w| *w), "every member went to warp: {warped:?}");
    assert!(nearest >= footprint(hull(&db)) - n(CELL), "a member came as near as {nearest:?}");
    assert!(ids.iter().all(|id| sim.unit(*id).unwrap().pos.z > n(2500)), "they all got past");
}

#[test]
fn an_attack_approach_from_beyond_the_warp_distance_is_routed_round_the_field_too() {
    let db = db(true, None);
    let planet = v(0, 0);
    let mut sim = game(&db, &[planet]);
    let s = craft(&mut sim, v(0, -3000));
    let t = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, 3000), AHEAD).unwrap();
    let attack = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![s], target: t }) };
    let (mut nearest, mut at_warp) = (n(100_000), 0);
    for i in 0..60 * HZ {
        sim.step(if i == 0 { std::slice::from_ref(&attack) } else { &[] });
        nearest = nearest.min(distance(sim.unit(s).unwrap().pos, planet));
        at_warp += u32::from(sim.approach_warp.contains(s));
    }
    assert!(at_warp > 5 * HZ, "{at_warp} ticks at warp");
    assert!(nearest >= footprint(hull(&db)) - n(CELL), "nearest {nearest:?}");
    assert!(distance(sim.unit(s).unwrap().pos, sim.unit(t).unwrap().pos) < n(700), "it closed on its target");
}

/// Only a craft that wants warp plans round the fields: a plain move past the planet is the
/// same flight, tick for tick, with the switch on and off.
#[test]
fn a_plain_move_past_the_planet_is_unchanged() {
    let run = |routing: bool| {
        let db = db(routing, None);
        let mut sim = game(&db, &[v(150, 0)]);
        let s = craft(&mut sim, v(0, -1500));
        let goal = v(0, 1500);
        let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Move { units: vec![s], to: goal, queue: false, mode: None }) };
        let mut track = Vec::new();
        for i in 0..40 * HZ {
            sim.step(if i == 0 { std::slice::from_ref(&order) } else { &[] });
            let u = sim.unit(s).unwrap();
            track.push((u.pos, u.heading, u.speed));
        }
        assert!(distance(sim.unit(s).unwrap().pos, goal) < n(20));
        track
    };
    assert_eq!(run(true), run(false));
}

/// With the switch off a warp leg is planned like any move: it flies into the field and leaves
/// warp there. The header's buffer and cost then change nothing but the content's hash.
#[test]
fn with_the_switch_off_the_warp_leg_flies_into_the_field_and_the_keys_change_nothing() {
    let (planet, start, goal) = (v(150, 0), v(0, -3000), v(0, 3000));
    let off = one(&db(false, None), &[planet], start, goal);
    assert!(off.arrived);
    assert!(off.nearest.unwrap() < field(), "it entered the field: nearest {:?}", off.nearest);
    assert!(off.leaves.len() >= 2, "it left warp at the field and again before its goal: {:?}", off.leaves);
    let with_cost = one(&db(false, Some("DEFAULT_TERRAIN_VALUE * 50")), &[planet], start, goal);
    assert_eq!((off.end, off.at_warp, &off.leaves), (with_cost.end, with_cost.at_warp, &with_cost.leaves), "the same flight");
    assert_ne!(db(false, None).hash(), db(false, Some("DEFAULT_TERRAIN_VALUE * 50")).hash(), "the keys are part of the content");
    assert_ne!(db(false, None).hash(), db(true, None).hash(), "and so is the switch");
}

/// The route is derived each tick from positions: nothing is stored, so a snapshot taken on
/// the way round reloads and runs on identically.
#[test]
fn a_snapshot_on_the_way_round_reloads_and_runs_on_identically() {
    let db = db(true, None);
    let mut sim = game(&db, &[v(0, 0)]);
    let s = craft(&mut sim, v(0, -3000));
    sim.step(&[warp_move(&[s], v(0, 3000))]);
    while sim.unit(s).unwrap().pos.z < n(-700) {
        sim.step(&[]);
    }
    assert!(sim.approach_warp.contains(s), "on its way round, at warp");
    let mut back = Sim::from_snapshot(&sim.snapshot(), Some(Arc::clone(&db))).unwrap();
    assert_eq!(back.digest(), sim.digest());
    for tick in 0..10 * HZ {
        sim.step(&[]);
        back.step(&[]);
        assert_eq!(back.digest(), sim.digest(), "tick {tick}");
    }
}
