//! UI-40: the TRANSPORT command: availability, the greyed button, target validity, the order, the press.

use std::sync::Arc;

use sta_content::{ContentDb, Profile};
use sta_sim::command::{CommandKind, LocalRouter, Order};
use sta_sim::fake::ScriptedSource;
use sta_sim::view::{PlayerView, UnitFlags, UnitView};
use sta_sim::{EntityId, Fixed, PlayerId, TeamId, Vec3, WorldSource, WorldView};

use super::model::{Act, Look, Page, TargetMode, grid};
use super::*;
use crate::app::configure_sets;
use crate::sim_link::SimLinkPlugin;
use crate::ui::theme::click;

const ME: PlayerId = PlayerId(1);

/// `fake::db()` under the given game family's switches (`crew_loss_on_hit` is what the sim gates on).
fn db(profile: Profile) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(profile).features(profile.features());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    Arc::new(b.build().unwrap())
}

fn crew(now: i32, max: i32) -> Option<[Fixed; 2]> {
    Some([Fixed::from_int(now), Fixed::from_int(max)])
}

// Ids: 1 own cruiser (40 of 50), 2 own frigate (6 of 50: too few to beam any away), 3 own cruiser (20 of 50),
// 4 enemy cruiser 5000 away, 5 enemy derelict (0 of 50), 6 a planet, 7 own yard (50 of 50), 8 an enemy hull with
// no crew room, 9 a dying enemy cruiser.
fn view(db: &ContentDb, mutate: impl FnOnce(&mut Vec<UnitView>)) -> WorldView {
    let make = |id: u32, odf: &str, owner: PlayerId, x: i32, c: Option<[Fixed; 2]>| UnitView {
        id: EntityId(id), def: db.key(odf).unwrap(), owner,
        pos: Vec3::from_f32([x as f32, 0.0, 0.0]), heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4], crew: c, energy: None, autonomy: None,
        flags: UnitFlags::SELECTABLE, progress: None,
    };
    let mut units = vec![
        make(1, "fx_cruiser", ME, 0, crew(40, 50)), make(2, "fx_frigate", ME, 10, crew(6, 50)), make(3, "fx_cruiser", ME, 20, crew(20, 50)),
        make(4, "fx_cruiser", PlayerId(2), 5000, crew(30, 50)), make(5, "fx_cruiser", PlayerId(2), 30, crew(0, 50)),
        make(6, "fx_moon", sta_sim::world::NEUTRAL, 40, None), make(7, "fx_yard", ME, 50, crew(50, 50)),
        make(8, "fx_cruiser", PlayerId(2), 60, None), make(9, "fx_cruiser", PlayerId(2), 70, crew(30, 50)),
    ];
    units[8].flags = UnitFlags::SELECTABLE.with(UnitFlags::DYING, true);
    mutate(&mut units);
    let player = |id: u8, team: u8| PlayerView {
        crew: None, id: PlayerId(id), team: TeamId(team), resources: Vec::new(), officers: [Fixed::ZERO; 2], eliminated: false, relations: Vec::new(),
    };
    WorldView { units, players: vec![player(1, 1), player(2, 2)], ..Default::default() }
}

fn feed_in(profile: Profile, selected: &[u32], mutate: impl FnOnce(&mut Vec<UnitView>)) -> HudFeed {
    let db = db(profile);
    HudFeed { view: Arc::new(view(&db, mutate)), player: ME, selected: selected.iter().copied().map(EntityId).collect(), content: Some(db) }
}

fn feed(selected: &[u32]) -> HudFeed {
    feed_in(Profile::Sta2Classic, selected, |_| {})
}

fn transport(f: &HudFeed, target: u32) -> Option<(Vec<Order>, Vec<EntityId>)> {
    target_order(f, TargetMode::Transport, Some(EntityId(target)), None)
}

fn button(f: &HudFeed) -> Option<super::model::Button> {
    grid(f, Page::Top).into_iter().find(|b| b.act == Act::Target(TargetMode::Transport))
}

#[test]
fn the_button_shows_for_crewed_ships_and_stations_with_the_stock_art_and_only_then() {
    let b = button(&feed(&[1])).expect("TRANSPORT button");
    assert_eq!(b.sprite_names, ["b_transport"]);
    assert_eq!(b.look, Look::Ready);
    assert!(button(&feed(&[7])).is_some(), "a station with a crew transports too");
    assert!(button(&feed(&[1, 3])).is_some());
    // No crew in the view (not a transporter), someone else's ship, a planet, nothing selected.
    assert!(button(&feed_in(Profile::Sta2Classic, &[1], |u| u[0].crew = None)).is_none());
    assert!(button(&feed(&[4])).is_none(), "an enemy ship is not ours to order");
    assert!(button(&feed(&[6])).is_none(), "a planet");
    assert!(button(&feed(&[])).is_none());
}

#[test]
fn the_button_is_greyed_when_the_sim_would_refuse_every_carrier() {
    let ready = |f: &HudFeed| button(f).unwrap().look == Look::Ready;
    assert!(ready(&feed(&[1])));
    // Crew at or below the transporter floor (9), or at or below the party (5): shown, disabled.
    assert!(!ready(&feed(&[2])));
    assert!(!ready(&feed_in(Profile::Sta2Classic, &[1], |u| u[0].crew = crew(9, 50))));
    assert!(ready(&feed_in(Profile::Sta2Classic, &[1], |u| u[0].crew = crew(10, 50))));
    // A mixed selection is ready when any ship can; the other gets nothing.
    assert!(ready(&feed(&[2, 1])));
    assert!(commands::shared(&feed(&[2])).transport_family, "low crew in a family that allows it");
    // Native: the sim refuses, so the button is greyed. This follows the sim's `transport_allowed`, not a
    // hard-coded profile check at the button. Armada I (SIM-71) is allowed: every crewed craft transports.
    let a1 = feed_in(Profile::Sta1Classic, &[1], |_| {});
    assert_eq!(button(&a1).map(|b| b.look), Some(Look::Ready), "Armada I");
    assert!(commands::shared(&a1).transport_family);
    for p in [Profile::Native] {
        let f = feed_in(p, &[1], |_| {});
        assert_eq!(button(&f).map(|b| b.look), Some(Look::Locked), "{p:?}");
        assert!(!target_mode_available(&f, TargetMode::Transport), "{p:?}");
        assert!(!commands::shared(&f).transport_family, "{p:?}: the press says the game is refused, not the crew");
        assert!(transport(&f, 3).is_none(), "{p:?}");
    }
}

#[test]
fn a_valid_target_submits_exactly_one_transport_order_for_the_ready_carriers() {
    for (target, what) in [(3, "an own ship"), (4, "an enemy ship, far out of range: the sim decides"), (5, "a derelict"), (7, "an own station")] {
        let (orders, ids) = transport(&feed(&[1]), target).unwrap_or_else(|| panic!("{what}"));
        assert!(matches!(orders.as_slice(), [Order::Transport { units, target: t, attack: false }]
            if units == &[EntityId(1)] && *t == EntityId(target)), "{what}: {orders:?}");
        assert_eq!(ids, [EntityId(1)]);
    }
    // Several ships: one order, only the ones the sim takes (ship 2 has too little crew), never the target itself.
    let (orders, ids) = transport(&feed(&[1, 2, 3]), 4).unwrap();
    assert!(matches!(orders.as_slice(), [Order::Transport { units, target: EntityId(4), .. }] if units == &[EntityId(1), EntityId(3)]), "{orders:?}");
    assert_eq!(ids, [EntityId(1), EntityId(3)]);
    let (orders, _) = transport(&feed(&[1, 3]), 3).unwrap();
    assert!(matches!(orders.as_slice(), [Order::Transport { units, .. }] if units == &[EntityId(1)]));
    assert!(transport(&feed(&[3]), 3).is_none(), "a ship cannot beam to itself");
}

#[test]
fn a_target_the_sim_would_drop_is_refused_and_the_pick_stays_armed() {
    let f = feed(&[1]);
    assert!(transport(&f, 6).is_none(), "a planet");
    assert!(transport(&f, 8).is_none(), "no crew room: the sim drops it");
    assert!(transport(&f, 9).is_none(), "a dying ship");
    assert!(transport(&f, 99).is_none(), "not in the view");
    assert!(target_order(&f, TargetMode::Transport, None, Some(Vec3::ZERO)).is_none(), "open ground");
    // Nobody ready: nothing to send even at a good target.
    assert!(transport(&feed(&[2]), 3).is_none());
}

const TIMELINE: &str = "{ units: [
    { id: 1, def: 'fx_cruiser', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
] }";

/// An app with the HUD, ship 1 selected and given `crew_now` crew.
fn app(profile: Profile, crew_now: i32) -> App {
    let db = db(profile);
    let mut src = ScriptedSource::from_json5(TIMELINE, &db).unwrap();
    let out = src.step(&[]);
    let mut view = (*out.view).clone();
    view.units[0].crew = crew(crew_now, 50);
    view.players = vec![PlayerView { crew: None, id: ME, team: TeamId(1), resources: vec![Fixed::ZERO; 4], officers: [Fixed::ZERO; 2], eliminated: false, relations: Vec::new() }];
    let mut link = SimLink::new(Box::new(src), Box::new(LocalRouter::new()), ME);
    link.feed(Arc::new(view));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    configure_sets(&mut app);
    app.init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(SimLinkPlugin)
        .insert_resource(link)
        .insert_resource(ContentRes(db))
        .insert_resource(Selection(vec![EntityId(1)]))
        .add_plugins(HudPlugin);
    app.insert_resource(crate::sim_link::BenchDriven);
    app.update();
    app
}

fn press_transport(app: &mut App) {
    let e = app.world_mut().query::<(Entity, &GridCell)>().iter(app.world())
        .find(|(_, c)| c.0 == Act::Target(TargetMode::Transport)).expect("a TRANSPORT cell").0;
    click(app.world_mut(), e);
    app.update();
    app.update();
}

#[test]
fn pressing_the_button_arms_the_transport_pick_and_queues_nothing_until_the_click() {
    let mut app = app(Profile::Sta2Classic, 40);
    press_transport(&mut app);
    assert_eq!(app.world().resource::<HudState>().targeting, Some(TargetMode::Transport));
    assert!(app.world().resource::<CommandQueue>().0.iter().all(|c| !matches!(c, CommandKind::Order(Order::Transport { .. }))));
}

#[test]
fn pressing_the_greyed_button_arms_nothing() {
    for (profile, crew_now) in [(Profile::Sta2Classic, 7), (Profile::Sta1Classic, 7), (Profile::Native, 40)] {
        let mut app = app(profile, crew_now);
        press_transport(&mut app);
        assert_eq!(app.world().resource::<HudState>().targeting, None, "{profile:?} with {crew_now}");
    }
}
