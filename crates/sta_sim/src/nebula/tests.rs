//! SIM-75 nebula effects, on authored data only.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, PathingRules, Profile, Region, RegionKind, RegionShape, Rules, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use super::*;
use crate::command::{Command, CommandKind, Order, Subsystem};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::specials::CloakState;
use crate::WorldSource;

pub(crate) fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn f(x: f64) -> Fixed {
    Fixed::from_f64(x)
}

/// A `nebula_obj` definition from ODF text, as the content loader makes one.
fn nebula(odf: &str, text: &str) -> UnitDef {
    let text = format!("classLabel = \"nebula_obj\"\nunitName = \"{odf}\"\n{text}");
    let stems = [odf.to_owned()];
    let (db, _) = sta_content::odf::load_with(Default::default(), &Default::default(), &stems, &mut |_, version| (version == 0).then(|| text.clone()))
        .unwrap();
    db.units().next().unwrap().clone()
}

/// Fake content, the cloakers, a cruiser with system hit points, and these
/// authored nebulae (the kinds the tests need, no game data):
/// - `fx_slow`: no keys (the speed coefficient's default)
/// - `fx_coef`: `craft_speed_coefficient = 0.65`
/// - `fx_mixed`: fraction 0.45, coefficient 0.55, `craft_speed_limit = 0`
/// - `fx_drag`: `craft_max_speed_fraction = 0.4`
/// - `fx_cap`: `craft_speed_coefficient = 1`, `craft_speed_limit = 30`
/// - `fx_burn`, `fx_burn2`: 60 damage per second, shields off
/// - `fx_calm`: no slowing, `regenValue = 30`, `repairValue = 3`
/// - `fx_still`, `fx_still2`: no slowing, shields off (nothing else)
/// - `fx_quiet`: no slowing, weapons, life support and cloak off
pub(crate) fn content(crew_loss: bool) -> Arc<ContentDb> {
    content_in(crew_loss, None)
}

/// As [`content`], under `profile` (none: the fake content's own).
pub(crate) fn content_in(crew_loss: bool, profile: Option<Profile>) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let (cloakers, devices) = crate::specials::fixture::defs();
    let mut features = fake.features().clone();
    features.crew_loss_on_hit = crew_loss;
    features.shield_zero_hold = profile == Some(Profile::Sta2Classic);
    let mut b = ContentDb::builder().profile(profile.unwrap_or(fake.profile())).features(features);
    for w in fake.weapons().cloned().chain(devices) {
        b = b.weapon(w);
    }
    for mut u in fake.units().cloned().chain(cloakers) {
        if u.odf == "fx_cruiser" {
            u.system_hit_points = [Fixed::from_int(100); 5];
        }
        b = b.unit(u);
    }
    let calm = "craft_speed_coefficient = 1\n";
    for (odf, text) in [
        ("fx_slow", String::new()),
        ("fx_coef", "craft_speed_coefficient = 0.65\n".to_owned()),
        ("fx_mixed", "craft_max_speed_fraction = 0.45\ncraft_speed_coefficient = 0.55\ncraft_speed_limit = 0\n".to_owned()),
        ("fx_drag", "craft_max_speed_fraction = 0.4\n".to_owned()),
        ("fx_cap", "craft_speed_coefficient = 1\ncraft_speed_limit = 30\n".to_owned()),
        ("fx_burn", format!("{calm}damageValue = 60\ndisableShields = 1\n")),
        ("fx_burn2", format!("{calm}damageValue = 60\ndisableShields = 1\n")),
        ("fx_calm", format!("{calm}regenValue = 30\nrepairValue = 3\n")),
        ("fx_still", format!("{calm}disableShields = 1\n")),
        ("fx_still2", format!("{calm}disableShields = 1\n")),
        ("fx_quiet", format!("{calm}disableWeapons = 1\ndisableCrew = 1\ndisableCloak = 1\n")),
    ] {
        b = b.unit(nebula(odf, &text));
    }
    // The planner would route round the nebulae; these tests fly through them.
    let off = PathingRules { enabled: false, ..Default::default() };
    Arc::new(b.build().unwrap().with_rules(Rules::default().with_pathing(off)))
}

pub(crate) fn neb(odf: &str, at: Vec3, radius: i32) -> Region {
    Region { kind: RegionKind::Nebula, odf: odf.into(), shape: RegionShape::Sphere { centre: at, radius: Fixed::from_int(radius) } }
}

/// A sim on a map whose minimum corner is the origin (the cells start there).
pub(crate) fn sim_with(regions: Vec<Region>, db: Arc<ContentDb>) -> Sim {
    let mut map = MapDef::empty();
    map.bounds = (Vec3 { x: Fixed::ZERO, y: Fixed::from_int(-500), z: Fixed::ZERO }, Vec3 { x: Fixed::from_int(8000), y: Fixed::from_int(500), z: Fixed::from_int(8000) });
    map.regions = regions;
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    Sim::from_setup(&GameSetup { seed: 3, map: Arc::new(map), content: db, players: vec![seat(1), seat(2)], options: GameOptions::default() })
}

fn sim(regions: Vec<Region>) -> Sim {
    sim_with(regions, content(false))
}

pub(crate) fn put(sim: &mut Sim, odf: &str, player: u8, at: Vec3) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    sim.spawn_def(key, PlayerId(player), at, (Fixed::ONE, Fixed::ZERO)).unwrap()
}

fn order(sim: &mut Sim, player: u8, kind: Order) -> Vec<SimEvent> {
    sim.step(&[Command { player: PlayerId(player), seq: 0, kind: CommandKind::Order(kind) }]).events
}

pub(crate) fn run(sim: &mut Sim, ticks: u32) -> Vec<SimEvent> {
    (0..ticks).flat_map(|_| sim.step(&[]).events).collect()
}

fn hull(sim: &Sim, id: EntityId) -> Fixed {
    sim.unit(id).unwrap().hull[0]
}

fn near(a: Fixed, b: Fixed) -> bool {
    (a - b).abs() < f(0.01)
}

const O: Vec3 = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };

#[test]
fn a_cell_counts_by_its_centre_and_the_craft_height_does_not() {
    let c = v(1000, 1000);
    let r = Fixed::from_int(100);
    // 1099 lies in the cell centred on 1075, 1101 in the one centred on 1125.
    assert!(covers(v(1099, 1000), O, None, c, r));
    assert!(!covers(v(1101, 1000), O, None, c, r), "within the radius, but its cell's centre is not");
    assert!(!covers(v(1100, 1000), O, None, c, r), "on the sphere, but its cell's centre is 125 out");
    let high = Vec3 { y: Fixed::from_int(4000), ..v(1000, 1000) };
    assert!(covers(high, O, None, c, r), "the craft's height is ignored");
    // The nebula's own height counts: (25, 60, 25) is inside 100, (25, 99, 25) is not.
    assert!(covers(v(1000, 1000), O, None, Vec3 { y: Fixed::from_int(60), ..c }, r));
    assert!(!covers(v(1000, 1000), O, None, Vec3 { y: Fixed::from_int(99), ..c }, r));
}

#[test]
fn cells_count_from_the_map_corner_and_nothing_off_the_map_is_inside() {
    let r = Fixed::from_int(40);
    let origin = v(10, 10);
    // From a corner at 10 the cell of 40 is centred on 35; from 0 it would be 25.
    assert!(covers(v(40, 40), origin, None, v(60, 60), r));
    assert!(!covers(v(40, 40), O, None, v(60, 60), r));
    let end = Some(v(100, 100));
    assert!(!covers(v(5, 40), origin, end, v(10, 40), r), "west of the corner");
    assert!(!covers(v(120, 40), origin, end, v(110, 40), r), "past the far edge");
}

#[test]
fn damage_goes_to_the_hull_once_per_kind() {
    let at = v(1000, 1000);
    let burn = |odfs: &[&str]| {
        let mut s = sim(odfs.iter().map(|o| neb(o, at, 300)).collect());
        let id = put(&mut s, "fx_cruiser", 1, at);
        run(&mut s, 30);
        Fixed::from_int(800) - hull(&s, id)
    };
    assert!(near(burn(&["fx_burn"]), Fixed::from_int(60)), "60 per second, shields held at 0: {:?}", burn(&["fx_burn"]));
    assert!(near(burn(&["fx_burn", "fx_burn"]), Fixed::from_int(60)), "two nebulae of one kind act once");
    assert!(near(burn(&["fx_burn", "fx_burn2"]), Fixed::from_int(120)), "two kinds both act");
}

#[test]
fn a_craft_just_outside_takes_nothing() {
    let mut s = sim(vec![neb("fx_burn", v(1000, 1000), 100)]);
    let inside = put(&mut s, "fx_cruiser", 1, v(1099, 1000));
    let outside = put(&mut s, "fx_cruiser", 1, v(1101, 1000));
    run(&mut s, 30);
    assert!(hull(&s, inside) < Fixed::from_int(800));
    assert_eq!(hull(&s, outside), Fixed::from_int(800));
    assert!(s.unit(outside).unwrap().shields.now[0] > Fixed::ZERO, "its shields are its own");
}

#[test]
fn damage_can_destroy() {
    let at = v(1000, 1000);
    let mut s = sim(vec![neb("fx_burn", at, 300)]);
    let id = put(&mut s, "fx_cruiser", 1, at);
    let events = run(&mut s, 30 * 14);
    assert!(s.unit(id).is_none());
    assert!(events.iter().any(|e| matches!(e, SimEvent::Destroyed { id: d, .. } if *d == id)));
}

/// The fastest the cruiser goes on a long move through the nebula `odf`
/// (none: open space) under `profile`, and its own top speed.
fn top_speed_in(profile: Profile, odf: Option<&str>) -> (Fixed, Fixed) {
    let mut s = sim_with(odf.map(|o| vec![neb(o, v(2500, 1000), 2400)]).unwrap_or_default(), content_in(false, Some(profile)));
    let id = put(&mut s, "fx_cruiser", 1, v(500, 1000));
    order(&mut s, 1, Order::Move { units: vec![id], to: v(4500, 1000), queue: false });
    let mut top = Fixed::ZERO;
    for _ in 0..600 {
        s.step(&[]);
        top = top.max(s.unit(id).unwrap().speed);
    }
    let u = s.unit(id).unwrap();
    assert_eq!(u.stats, s.content().unwrap().unit(u.def).unwrap().movement, "the caps last one step only");
    (top, u.stats.max_speed.max(u.stats.warp_speed))
}

/// The Armada II family profiles (Native, Fleet Operations 3 and 4, stock
/// Armada II) all take the measured rule.
const A2_FAMILY: [Profile; 4] = [Profile::Native, Profile::Fo3, Profile::Fo4, Profile::Sta2Classic];

#[test]
fn a2_without_a_fraction_the_speed_is_unchanged() {
    for profile in A2_FAMILY {
        let (open, own) = top_speed_in(profile, None);
        assert!(open > own / Fixed::from_int(2) + f(1.0), "{profile:?}: the open-space run is fast enough to tell: {open:?}");
        // Absent coefficient, a coefficient of 0.65, a limit with coefficient 1:
        // none of them slows the craft.
        for odf in ["fx_slow", "fx_coef", "fx_cap"] {
            let (top, _) = top_speed_in(profile, Some(odf));
            assert_eq!(top, open, "{profile:?} {odf}");
        }
    }
}

#[test]
fn a2_a_max_speed_fraction_caps_at_that_share_of_impulse() {
    for profile in A2_FAMILY {
        let (top, _) = top_speed_in(profile, Some("fx_drag"));
        let impulse = Fixed::from_int(140);
        assert!(top <= impulse * f(0.4f32 as f64) + f(0.01), "{profile:?}: {top:?}");
        assert!(top > impulse * f(0.3), "{profile:?}: {top:?}");
    }
}

/// A kind with a fraction, a coefficient and a limit of 0: the fraction decides
/// (the limit does not hold the craft, the coefficient does not slow it more).
#[test]
fn the_fraction_decides_over_the_coefficient_and_the_limit() {
    let impulse = Fixed::from_int(140);
    for profile in A2_FAMILY.into_iter().chain([Profile::Sta1Classic]) {
        let (top, _) = top_speed_in(profile, Some("fx_mixed"));
        assert!(top <= impulse * f(0.45f32 as f64) + f(0.01), "{profile:?}: {top:?}");
        assert!(top > impulse * f(0.4), "{profile:?}: {top:?}");
    }
}

/// Interim, not measured: Armada I keeps the coefficient rule.
#[test]
fn a1_keeps_the_coefficient_rule() {
    let p = Profile::Sta1Classic;
    let (open, own) = top_speed_in(p, None);
    assert!(open > own / Fixed::from_int(2) + f(1.0), "the open-space run is fast enough to tell: {open:?}");
    let (slow, own) = top_speed_in(p, Some("fx_slow"));
    assert!(slow <= own / Fixed::from_int(2) + f(0.01), "an absent coefficient halves: {slow:?} of {own:?}");
    let (half, own) = top_speed_in(p, Some("fx_coef"));
    assert!(half <= own * f(0.65) + f(0.01) && half > own * f(0.6), "{half:?} of {own:?}");
    let (top, _) = top_speed_in(p, Some("fx_cap"));
    assert!(top <= Fixed::from_int(30) + f(0.01) && top > Fixed::from_int(25), "{top:?}");
    let (drag, _) = top_speed_in(p, Some("fx_drag"));
    assert!(drag <= Fixed::from_int(140) * f(0.4f32 as f64) + f(0.01), "{drag:?}");
}

#[test]
fn regeneration_adds_shields_and_repair_mends_live_systems() {
    let at = v(1000, 1000);
    let drained = |regions: Vec<Region>| {
        let mut s = sim(regions);
        let id = put(&mut s, "fx_cruiser", 1, at);
        s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
        assert!(s.edit_subsystem_damage(id, Subsystem::Engines, Fixed::from_int(50)));
        assert!(s.edit_subsystem_damage(id, Subsystem::Sensors, Fixed::from_int(100)));
        run(&mut s, 30);
        let shields = s.unit(id).unwrap().shields.now[0];
        (shields, s.subsystem_state(id, Subsystem::Engines).unwrap().1, s.subsystem_state(id, Subsystem::Sensors).unwrap().1)
    };
    let (open, engines_open, _) = drained(vec![]);
    let (calm, engines, sensors) = drained(vec![neb("fx_calm", at, 300)]);
    assert!(near(calm - open, Fixed::from_int(30)), "30 a second over the recharge: {:?}", calm - open);
    assert_eq!(engines_open, Fixed::from_int(50));
    assert!(near(engines, Fixed::from_int(47)), "3 a second: {engines:?}");
    assert_eq!(sensors, Fixed::from_int(100), "a destroyed system is not repaired (interim)");
}

#[test]
fn nebula_regeneration_waits_for_an_empty_shield_pool_hold() {
    let at = v(1000, 1000);
    let mut s = sim_with(vec![neb("fx_calm", at, 300)], content_in(false, Some(Profile::Sta2Classic)));
    let id = put(&mut s, "fx_cruiser", 1, at);
    s.units.get_mut(id).unwrap().shields.now[0] = Fixed::ZERO;
    s.zero_shield_holds.hold(id, 0, 2);
    run(&mut s, 2);
    assert_eq!(s.unit(id).unwrap().shields.now[0], Fixed::ZERO);
    run(&mut s, 1);
    assert!(s.unit(id).unwrap().shields.now[0] > Fixed::ZERO);
}

#[test]
fn weapons_hold_fire_inside() {
    let fired = |regions: Vec<Region>| {
        let mut s = sim(regions);
        let a = put(&mut s, "fx_cruiser", 1, v(1000, 1000));
        let _b = put(&mut s, "fx_frigate", 2, v(1200, 1000));
        run(&mut s, 150).iter().filter(|e| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == a)).count()
    };
    assert!(fired(vec![]) > 0, "the cruiser fires in open space");
    assert_eq!(fired(vec![neb("fx_quiet", v(1100, 1000), 400)]), 0);
}

#[test]
fn life_support_off_drains_crew_under_the_casualty_model() {
    let crew = |regions: Vec<Region>| {
        let mut s = sim_with(regions, content(true));
        let id = put(&mut s, "fx_cruiser", 1, v(1000, 1000));
        run(&mut s, 30);
        s.unit(id).unwrap().crew.unwrap()[0]
    };
    assert_eq!(crew(vec![]), Fixed::from_int(300));
    assert!(crew(vec![neb("fx_quiet", v(1000, 1000), 300)]) < Fixed::from_int(300));
}

#[test]
fn no_cloak_inside() {
    let cloaked = |regions: Vec<Region>| {
        let mut s = sim(regions);
        let id = put(&mut s, "fx_cloaker", 1, v(1000, 1000));
        order(&mut s, 1, Order::Cloak { units: vec![id], on: true });
        run(&mut s, 60);
        s.specials.cloak_state(id)
    };
    assert_eq!(cloaked(vec![]), Some(CloakState::Cloaked));
    assert_ne!(cloaked(vec![neb("fx_quiet", v(1000, 1000), 300)]), Some(CloakState::Cloaked));
}

#[test]
fn a_map_without_effect_nebulae_has_no_rows() {
    let s = sim(vec![]);
    assert!(s.nebula_contents().is_empty());
}
