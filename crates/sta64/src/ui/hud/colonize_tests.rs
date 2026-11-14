//! UI-38: the COLONIZE command: availability, target validity, the order.

use std::sync::Arc;

use sta_content::{ColonizePodDef, ContentDb, DefRef, Hardpoint, PlanetRole, PopulationRules, Rules, UnitDef, WeaponDef, WeaponKind};
use sta_sim::colony::ColonyView;
use sta_sim::command::Order;
use sta_sim::view::{PlayerView, Relation, UnitFlags, UnitView};
use sta_sim::world::NEUTRAL;
use sta_sim::{EntityId, Fixed, PlayerId, TeamId, Vec3, WorldView};

use super::model::{Act, Page, TargetMode, grid};
use super::*;

const ME: PlayerId = PlayerId(1);
// Ids: ships 1 pod ship, 2 Borg beam ship, 3 warship, 4 pod ship without the stock flag.
// Planets 10 unowned, 11 own, 12 teammate's, 13 ally's, 14 enemy's, 15 not colonizable, 16 with a colonizer,
// 17 dying. 20 a ship of the enemy.
fn db() -> Arc<ContentDb> {
    let n = |v: i32| Fixed::from_int(v);
    let mut pod = WeaponDef::new("ui_pod", "colonizerpod", WeaponKind::Projectile);
    pod.refire = 150;
    pod.colonize = Some(ColonizePodDef { party_size: n(100), orbit_range: n(150) });
    let beam = WeaponDef::new("ui_colbeam", "colonybm", WeaponKind::Beam);
    let mut gun = WeaponDef::new("ui_gun", "phaser", WeaponKind::Beam);
    gun.damage = Fixed::ONE;
    let flag = Arc::new(sta_odf::resolve("s", &mut |_| Some("colony_ship = 1".into())).unwrap());
    let ship = |odf: &str, class: &str, weapon: &str, flagged: bool| {
        let mut s = UnitDef::new(odf, class);
        s.movement.max_speed = Fixed::ONE;
        s.hardpoints = vec![Hardpoint { weapon: DefRef::new(weapon), node: "hp01".into(), arc: None }];
        if flagged {
            s.raw = flag.clone();
        }
        s
    };
    let planet = |odf: &str, colonizable: bool| {
        let mut p = UnitDef::new(odf, "planet");
        p.planet_role = Some(PlanetRole::Producer);
        p.planet_colonizable = colonizable;
        p.planet_max_population = 1;
        p.radius = n(100);
        p
    };
    let rules = Rules::default().with_population(PopulationRules { levels: [n(1000), n(2000), n(3000), n(5000)], civilian_rate_modifier: Fixed::from_f64(0.2) });
    let db = ContentDb::builder()
        .weapon(pod)
        .weapon(beam)
        .weapon(gun)
        .unit(ship("ui_colony", "colony", "ui_pod", true))
        .unit(ship("ui_borg_colony", "colony", "ui_colbeam", true))
        .unit(ship("ui_warship", "cruiser", "ui_gun", false))
        .unit(ship("ui_unflagged", "colony", "ui_pod", false))
        .unit(planet("ui_planet", true))
        .unit(planet("ui_dead_planet", false))
        .build()
        .unwrap();
    Arc::new(db.with_rules(rules))
}

fn feed(selected: &[u32], mutate: impl FnOnce(&mut Vec<UnitView>, &mut WorldView)) -> HudFeed {
    let db = db();
    let make = |id: u32, odf: &str, owner: PlayerId| UnitView {
        id: EntityId(id), def: db.key(odf).unwrap(), owner,
        pos: Vec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4], crew: None, energy: None, autonomy: None,
        flags: UnitFlags::SELECTABLE, progress: None,
    };
    let mut units = vec![
        make(1, "ui_colony", ME), make(2, "ui_borg_colony", ME), make(3, "ui_warship", ME), make(4, "ui_unflagged", ME),
        make(10, "ui_planet", NEUTRAL), make(11, "ui_planet", ME), make(12, "ui_planet", PlayerId(2)),
        make(13, "ui_planet", PlayerId(3)), make(14, "ui_planet", PlayerId(4)), make(15, "ui_dead_planet", NEUTRAL),
        make(16, "ui_planet", NEUTRAL), make(17, "ui_planet", NEUTRAL), make(20, "ui_warship", PlayerId(4)),
    ];
    units.iter_mut().find(|u| u.id == EntityId(17)).unwrap().flags = UnitFlags::SELECTABLE.with(UnitFlags::DYING, true);
    let player = |id: u8, team: u8, relations: Vec<(PlayerId, Relation)>| PlayerView {
        crew: None, id: PlayerId(id), team: TeamId(team), resources: Vec::new(), officers: [Fixed::ZERO; 2], eliminated: false, relations,
    };
    let mut view = WorldView {
        players: vec![
            player(1, 1, vec![(PlayerId(3), Relation::Ally), (PlayerId(4), Relation::Enemy)]),
            player(2, 1, Vec::new()), player(3, 3, Vec::new()), player(4, 4, Vec::new()),
        ],
        colonies: vec![ColonyView { id: EntityId(16), population: Fixed::ZERO, max_population: Fixed::from_int(1000), colonizer: Some(EntityId(9)) }],
        ..Default::default()
    };
    mutate(&mut units, &mut view);
    view.units = units;
    HudFeed { view: Arc::new(view), player: ME, selected: selected.iter().copied().map(EntityId).collect(), content: Some(db) }
}

fn colonize(f: &HudFeed, planet: u32) -> Option<(Vec<Order>, Vec<EntityId>)> {
    target_order(f, TargetMode::Colonize, Some(EntityId(planet)), None)
}

#[test]
fn offered_for_pod_colony_ships_of_the_stock_command_source_only() {
    let offered = |sel: &[u32]| target_mode_available(&feed(sel, |_, _| {}), TargetMode::Colonize);
    assert!(offered(&[1]));
    assert!(!offered(&[2]), "a Borg beam ship: the sim refuses its order");
    assert!(!offered(&[3]), "a warship");
    assert!(!offered(&[4]), "no `colony_ship` flag, the command's source");
    assert!(!offered(&[]));
    // As Mine: shown when any selected unit can, the order goes to those.
    assert!(offered(&[3, 1]));
    // A welding / building ship takes nothing.
    assert!(!target_mode_available(&feed(&[1], |u, _| u[0].flags = UnitFlags::SELECTABLE.with(UnitFlags::BUILDING, true)), TargetMode::Colonize));
    // Someone else's ship is not ours to order.
    assert!(!target_mode_available(&feed(&[1], |u, _| u[0].owner = PlayerId(4)), TargetMode::Colonize));
}

#[test]
fn grid_shows_the_button_with_the_stock_sprite_only_for_colony_ships() {
    let on = grid(&feed(&[1], |_, _| {}), Page::Top);
    let b = on.iter().find(|b| b.act == Act::Target(TargetMode::Colonize)).expect("COLONIZE button");
    assert_eq!(b.sprite_names, ["b_colonize"]);
    assert_eq!(b.cell, 8);
    assert!(on.iter().any(|b| b.act == Act::Target(TargetMode::Move)), "the colony ship still moves");
    for sel in [&[3][..], &[2], &[4]] {
        assert!(!grid(&feed(sel, |_, _| {}), Page::Top).iter().any(|b| b.act == Act::Target(TargetMode::Colonize)), "{sel:?}");
    }
}

#[test]
fn target_validity_follows_the_sims_gate() {
    let f = feed(&[1], |_, _| {});
    assert!(colonize(&f, 10).is_some(), "unowned");
    assert!(colonize(&f, 11).is_some(), "own");
    assert!(colonize(&f, 12).is_none(), "a teammate's");
    assert!(colonize(&f, 13).is_none(), "an ally's");
    assert!(colonize(&f, 14).is_none(), "an enemy's");
    assert!(colonize(&f, 20).is_none(), "a ship");
    assert!(colonize(&f, 1).is_none(), "the colony ship itself");
    assert!(colonize(&f, 15).is_none(), "a planet that is not colonizable");
    assert!(colonize(&f, 16).is_none(), "another colonizer is at work");
    assert!(colonize(&f, 17).is_none(), "a dying planet");
    assert!(colonize(&f, 99).is_none(), "not in the view");
    assert!(target_order(&f, TargetMode::Colonize, None, Some(Vec3::ZERO)).is_none(), "ground is not a planet");
}

#[test]
fn issues_order_colonize_for_the_colony_ships_of_the_selection() {
    let f = feed(&[1], |_, _| {});
    let (orders, ids) = colonize(&f, 10).unwrap();
    assert!(matches!(orders.as_slice(), [Order::Colonize { units, planet: EntityId(10) }] if units == &[EntityId(1)]));
    assert_eq!(ids, [EntityId(1)]);
    // A mixed selection: only the colony ship takes part.
    let (orders, ids) = colonize(&feed(&[3, 1], |_, _| {}), 11).unwrap();
    assert!(matches!(orders.as_slice(), [Order::Colonize { units, planet: EntityId(11) }] if units == &[EntityId(1)]));
    assert_eq!(ids, [EntityId(1)]);
    // No colony ship among them: nothing.
    assert!(colonize(&feed(&[3], |_, _| {}), 10).is_none());
}

#[test]
fn without_population_rules_no_planet_is_a_target() {
    let mut f = feed(&[1], |_, _| {});
    // The pod ship and planet definitions exist but the content has no population rules.
    let mut b = ContentDb::builder();
    for w in f.content.as_deref().unwrap().weapons() { b = b.weapon(w.clone()); }
    for u in f.content.as_deref().unwrap().units() { b = b.unit(u.clone()); }
    f.content = Some(Arc::new(b.build().unwrap()));
    assert!(colonize(&f, 10).is_none());
}
