//! The HUD in an app: a `ScriptedSource` world on `fake::db()` (and a variant
//! with a tech tree and a construction craft), the plugin, and clicks through
//! the kit's `click`.

use std::sync::Arc;

use sta_content::{ContentDb, DefRef, TechTree};
use sta_sim::command::{CommandKind, LocalRouter, Order};
use sta_sim::fake::ScriptedSource;
use sta_sim::view::PlayerView;
use sta_sim::{EntityId, Fixed, PlayerId, TeamId, Vec3 as SimVec3, WorldSource};

use super::layout::{HudLayout, InfoHeight, SCREEN_H};
use super::model::Act;
use super::panels::*;
use super::*;
use crate::app::configure_sets;
use crate::input::select::AimState;
use crate::sim_link::SimLinkPlugin;
use crate::ui::theme::click;

const ME: PlayerId = PlayerId(1);

const TIMELINE: &str = "{ units: [
    { id: 1, def: 'fx_cruiser', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
    { id: 2, def: 'fx_yard', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 400, y: 0, z: 0 } }] },
    { id: 3, def: 'fx_frigate', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 100, y: 0, z: 100 } }] },
    { id: 4, def: 'fx_cruiser', owner: 2, hull: 100, rails: [{ at: 0, pos: { x: 900, y: 0, z: 0 } }] },
] }";

/// `fake::db()`, with a tech tree and the cruiser able to build a station
/// (a craft: it needs a site).
fn content(tree: Option<&str>) -> ContentDb {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        if u.odf == "fx_cruiser" {
            u.build.list = vec![DefRef::new("fx_station")];
        }
        b = b.unit(u);
    }
    if let Some(t) = tree {
        b = b.tech_tree(TechTree::parse("t.tt", t).0);
    }
    b.build().unwrap()
}

fn player(id: u8, team: u8, stock: [f64; 4]) -> PlayerView {
    PlayerView {
        id: PlayerId(id),
        team: TeamId(team),
        resources: stock.iter().map(|v| Fixed::from_f64(*v)).collect(),
        officers: [Fixed::ZERO; 2],
        eliminated: false,
        relations: Vec::new(),
    }
}

/// An app with the HUD on a scripted world, the first tick played, `selected` selected.
fn app(db: ContentDb, selected: &[u32]) -> App {
    let db = Arc::new(db);
    let mut src = ScriptedSource::from_json5(TIMELINE, &db).unwrap();
    let out = src.step(&[]);
    let mut view = (*out.view).clone();
    view.players = vec![player(1, 1, [3000.0, 20.0, 700.0, 5.0]), player(2, 2, [0.0; 4])];
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
        .insert_resource(Selection(selected.iter().map(|&i| EntityId(i)).collect()))
        .add_plugins(HudPlugin);
    // The link steps from the clock; these tests hold the world still.
    app.insert_resource(crate::sim_link::BenchDriven);
    app.update();
    app
}

fn count<C: Component>(app: &mut App) -> usize {
    app.world_mut().query_filtered::<(), With<C>>().iter(app.world()).count()
}

fn cell(app: &mut App, f: impl Fn(&Act) -> bool) -> Entity {
    app.world_mut().query::<(Entity, &GridCell)>().iter(app.world()).find(|(_, c)| f(&c.0)).unwrap_or_else(|| panic!("no such cell")).0
}

fn press(app: &mut App, e: Entity) {
    click(app.world_mut(), e);
    // The press acts in this frame; the panels follow in the next.
    app.update();
    app.update();
}

fn queued(app: &mut App) -> Vec<CommandKind> {
    std::mem::take(&mut app.world_mut().resource_mut::<CommandQueue>().0)
}

fn node(app: &mut App, e: Entity) -> Node {
    app.world().get::<Node>(e).unwrap().clone()
}

fn one<C: Component>(app: &mut App) -> Entity {
    app.world_mut().query_filtered::<Entity, With<C>>().single(app.world()).unwrap()
}

#[test]
fn the_panels_follow_the_selection_in_three_heights() {
    let mut app = app(content(None), &[]);
    let l = HudLayout::new(None, 16.0 / 9.0);
    // Nothing selected: no info panel, an empty grid of dim slots, the strip is up.
    let info = one::<InfoPanel>(&mut app);
    assert_eq!(app.world().get::<Visibility>(info), Some(&Visibility::Hidden));
    assert_eq!(count::<GridCell>(&mut app), 0);
    let strip = one::<ResourceStrip>(&mut app);
    assert_eq!((node(&mut app, strip).left, node(&mut app, strip).top), (Val::Px(l.resources.x), Val::Px(l.resources.y)));

    for (sel, height) in [(&[3][..], InfoHeight::Low), (&[2][..], InfoHeight::Middle), (&[1, 3][..], InfoHeight::High)] {
        app.world_mut().resource_mut::<Selection>().0 = sel.iter().map(|&i| EntityId(i)).collect();
        app.update();
        let want = l.bottom_row(Some(height)).info;
        let n = node(&mut app, info);
        assert_eq!((n.left, n.top, n.width, n.height), (Val::Px(want.x), Val::Px(want.y), Val::Px(want.w), Val::Px(want.h)), "{height:?}");
        assert_ne!(app.world().get::<Visibility>(info), Some(&Visibility::Hidden));
    }
    // The grid sits where the layout says, whatever is selected.
    let grid = one::<GridPanel>(&mut app);
    let g = l.bottom_row(Some(InfoHeight::High)).grid;
    assert_eq!(node(&mut app, grid).left, Val::Px(g.x));
    // A group shows a tile each; a click on one selects that unit, Shift removes it.
    assert_eq!(count::<GroupTile>(&mut app), 2);
    let tile = app.world_mut().query::<(Entity, &GroupTile)>().iter(app.world()).find(|(_, t)| t.0 == EntityId(3)).unwrap().0;
    press(&mut app, tile);
    assert_eq!(app.world().resource::<Selection>().0, [EntityId(3)]);
    app.world_mut().resource_mut::<Selection>().0 = vec![EntityId(1), EntityId(3)];
    app.update();
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::ShiftLeft);
    let tile = app.world_mut().query::<(Entity, &GroupTile)>().iter(app.world()).find(|(_, t)| t.0 == EntityId(3)).unwrap().0;
    press(&mut app, tile);
    assert_eq!(app.world().resource::<Selection>().0, [EntityId(1)]);
}

#[test]
fn the_resources_strip_shows_the_players_stock() {
    let mut app = app(content(None), &[]);
    let strip = one::<ResourceStrip>(&mut app);
    let words: Vec<String> = app.world_mut().query::<&Text>().iter(app.world()).map(|t| t.0.clone()).collect();
    for want in ["DILITHIUM", "3000", "METAL", "700", "LATINUM", "20", "BIOMATTER", "5"] {
        assert!(words.iter().any(|w| w == want), "{want} in {words:?}");
    }
    assert!(app.world().get::<Children>(strip).is_some());
    // Stock changes redraw it.
    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut v = (*feed.view).clone();
    v.players[0].resources[0] = Fixed::from_int(1234);
    feed.view = Arc::new(v);
    app.insert_resource(crate::ui::hud::ExternalFeed);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    let words: Vec<String> = app.world_mut().query::<&Text>().iter(app.world()).map(|t| t.0.clone()).collect();
    assert!(words.iter().any(|w| w == "1234") && !words.iter().any(|w| w == "3000"), "{words:?}");
}

#[test]
fn the_grid_issues_the_ui02_orders() {
    let mut app = app(content(None), &[1, 3]);
    assert!(queued(&mut app).is_empty());
    let stop = cell(&mut app, |a| *a == Act::Stop);
    press(&mut app, stop);
    assert_eq!(queued(&mut app), [CommandKind::Order(Order::Stop { units: vec![EntityId(1), EntityId(3)] })]);
    // Observe each acknowledgement while its message is live, as the voice
    // consumer does. Four later app updates may expire the first action.
    let stop_acks: Vec<_> = app.world_mut().resource_mut::<Messages<OrderIssued>>().drain().map(|m| m.kind).collect();
    assert_eq!(stop_acks, [OrderKind::Stop]);
    let wedge = cell(&mut app, |a| matches!(a, Act::Formation(_)));
    press(&mut app, wedge);
    let q = queued(&mut app);
    assert!(matches!(q[..], [CommandKind::Order(Order::Formation { ref units, .. })] if units.len() == 2), "{q:?}");
    // The voice ack hears about it, as for a key press.
    let formation_acks: Vec<_> = app.world_mut().resource_mut::<Messages<OrderIssued>>().drain().map(|m| m.kind).collect();
    assert_eq!(formation_acks, [OrderKind::Formation]);
    // An enemy gets no orders.
    app.world_mut().resource_mut::<Selection>().0 = vec![EntityId(4)];
    app.update();
    assert_eq!(count::<GridCell>(&mut app), 0);
}

#[test]
fn build_buttons_come_from_the_builders_list_and_queue_the_order() {
    let mut app = app(content(None), &[2]);
    // A yard: the opener is there, the page is the top one.
    let open = cell(&mut app, |a| *a == Act::OpenBuild);
    press(&mut app, open);
    assert_eq!(app.world().resource::<HudState>().page, Page::Build);
    // Its list: the cruiser and the frigate, then cancel and back.
    let acts: Vec<Act> = app.world_mut().query::<&GridCell>().iter(app.world()).map(|c| c.0.clone()).collect();
    assert_eq!(acts.len(), 4, "{acts:?}");
    let cruiser = cell(&mut app, |a| matches!(a, Act::Build { odf, .. } if odf == "fx_cruiser"));
    press(&mut app, cruiser);
    let db = app.world().resource::<ContentRes>().0.clone();
    assert_eq!(
        queued(&mut app),
        [CommandKind::Order(Order::Build { builder: EntityId(2), what: db.key("fx_cruiser").unwrap(), at: None })]
    );
    // Cancel drops the queue; back leaves the page.
    let cancel = cell(&mut app, |a| matches!(a, Act::Cancel(_)));
    press(&mut app, cancel);
    assert_eq!(queued(&mut app), [CommandKind::Order(Order::Stop { units: vec![EntityId(2)] })]);
    let back = cell(&mut app, |a| *a == Act::Back);
    press(&mut app, back);
    assert_eq!(app.world().resource::<HudState>().page, Page::Top);
    // Deselecting the builder while its page is open closes the page.
    let again = open_again(&mut app);
    press(&mut app, again);
    app.world_mut().resource_mut::<Selection>().0 = vec![EntityId(3)];
    app.update();
    assert_eq!(app.world().resource::<HudState>().page, Page::Top);
}

fn open_again(app: &mut App) -> Entity {
    cell(app, |a| *a == Act::OpenBuild)
}

#[test]
fn a_tech_tree_disables_what_is_not_yet_available() {
    // The frigate wants a station; the player has none.
    let mut app = app(content(Some("fx_cruiser 0\nfx_frigate 1 fx_station\nfx_yard 0\nfx_station 0\n")), &[2]);
    let open = cell(&mut app, |a| *a == Act::OpenBuild);
    press(&mut app, open);
    let frigate = cell(&mut app, |a| matches!(a, Act::Build { odf, .. } if odf == "fx_frigate"));
    assert_eq!(app.world().get::<crate::ui::theme::WidgetState>(frigate), Some(&crate::ui::theme::WidgetState::Disabled));
    // A press on a disabled button does nothing.
    press(&mut app, frigate);
    assert!(queued(&mut app).is_empty());
    let cruiser = cell(&mut app, |a| matches!(a, Act::Build { odf, .. } if odf == "fx_cruiser"));
    press(&mut app, cruiser);
    assert_eq!(queued(&mut app).len(), 1);
}

#[test]
fn a_craft_builder_waits_for_a_site() {
    // The cruiser is a craft here: it builds a station where the click lands.
    let mut app = app(content(None), &[1]);
    app.init_resource::<Aim>();
    let open = cell(&mut app, |a| *a == Act::OpenBuild);
    press(&mut app, open);
    let station = cell(&mut app, |a| matches!(a, Act::Build { site: true, .. }));
    press(&mut app, station);
    assert!(queued(&mut app).is_empty(), "nothing is sent until the click");
    assert_eq!(app.world().resource::<HudState>().placing.as_ref().map(|a| a.odf.as_str()), Some("fx_station"));

    // The click, with the pointer over the map.
    let ray = Ray3d::new(Vec3::new(-1600.0, 1000.0, 300.0), Dir3::NEG_Y);
    app.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
    app.update();
    let db = app.world().resource::<ContentRes>().0.clone();
    let at = SimVec3::from_f32([-1600.0, 0.0, -300.0]);
    assert_eq!(
        queued(&mut app),
        [CommandKind::Order(Order::Build { builder: EntityId(1), what: db.key("fx_station").unwrap(), at: Some(at) })]
    );
    assert!(app.world().resource::<HudState>().placing.is_none());
}

#[test]
fn escape_cancels_a_placement() {
    let mut app = app(content(None), &[1]);
    app.init_resource::<Aim>();
    let open = cell(&mut app, |a| *a == Act::OpenBuild);
    press(&mut app, open);
    let station = cell(&mut app, |a| matches!(a, Act::Build { site: true, .. }));
    press(&mut app, station);
    assert!(app.world().resource::<HudState>().placing.is_some());
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
    app.update();
    assert!(app.world().resource::<HudState>().placing.is_none());
    assert!(queued(&mut app).is_empty());
}

#[test]
fn the_radar_and_the_notices_are_hosted_at_their_places() {
    let mut app = app(content(None), &[]);
    let l = HudLayout::new(None, 16.0 / 9.0);
    let radar = one::<RadarRoot>(&mut app);
    let n = node(&mut app, radar);
    assert_eq!((n.left, n.bottom), (Val::Px(l.minimap.x), Val::Px(SCREEN_H - l.minimap.bottom())));
    let notices = one::<NoticeRoot>(&mut app);
    assert_eq!(node(&mut app, notices).top, Val::Px(l.resources.bottom() + layout::GAP));
    assert_eq!(count::<HudRoot>(&mut app), 1);
}

#[test]
fn a_cfg_resizes_the_panels() {
    let (cfg, _) = sta_content::GuiCfg::parse(
        "g.cfg",
        "screenWidth = 2134\nscreenHeight = 1200\ninfoPanelArea = 0 900 600 300\ninfoPanelArea_1 = 0 1000 600 200\ninfoPanelArea_2 = 0 1100 600 100\n",
    );
    let gui = cfg.layout_auto();
    let mut app = app(content(None), &[3]);
    app.world_mut().resource_mut::<HudConfig>().gui = Some(gui.clone());
    app.update();
    let info = one::<InfoPanel>(&mut app);
    let want = HudLayout::new(Some(&gui), 16.0 / 9.0).bottom_row(Some(InfoHeight::Low)).info;
    let n = node(&mut app, info);
    assert_eq!((n.width, n.height), (Val::Px(want.w), Val::Px(want.h)));
    assert_eq!(want.h, 90.0);
}

#[test]
fn clicks_over_the_hud_are_kept_off_the_map() {
    let app = app(content(None), &[]);
    // No window, no pointer: the map has the input; a waiting placement takes it.
    let mut app = app;
    assert!(app.world_mut().run_system_once(map_input_free).unwrap());
    app.world_mut().resource_mut::<HudPointer>().0 = true;
    assert!(!app.world_mut().run_system_once(map_input_free).unwrap());
}

use bevy::ecs::system::RunSystemOnce;

/// Arm the cruiser's station build and put the pointer over the sim point `(x, z)`.
fn armed_at(app: &mut App, x: f32, z: f32) {
    app.init_resource::<Aim>();
    let open = cell(app, |a| *a == Act::OpenBuild);
    press(app, open);
    let station = cell(app, |a| matches!(a, Act::Build { site: true, .. }));
    press(app, station);
    let ray = Ray3d::new(Vec3::new(x, 1000.0, -z), Dir3::NEG_Y);
    app.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
    app.update();
}

fn ghost_status(app: &mut App) -> Option<Result<(), sta_sim::production::site::SiteError>> {
    let mut q = app.world_mut().query::<&ghost::Ghost>();
    q.iter(app.world()).next().expect("a ghost").status
}

#[test]
fn the_ghost_is_clear_on_open_space_and_blocked_over_a_structure_or_off_the_map() {
    use sta_sim::production::site::SiteError;
    let mut app = app(content(None), &[1]);
    armed_at(&mut app, -1600.0, 300.0);
    assert_eq!(app.world_mut().query::<&ghost::Ghost>().iter(app.world()).count(), 1);
    assert_eq!(ghost_status(&mut app), Some(Ok(())));
    // The yard (id 2) stands at (400, 0).
    let ray = Ray3d::new(Vec3::new(410.0, 1000.0, 0.0), Dir3::NEG_Y);
    app.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
    app.update();
    assert_eq!(ghost_status(&mut app), Some(Err(SiteError::Blocked)));
    // Off the map: the same function the sim uses, with the map's bounds.
    let b = |v: f32| SimVec3::from_f32([v, 0.0, v]);
    app.insert_resource(ghost::PlacementBounds(Some((b(-2000.0), b(2000.0)))));
    let ray = Ray3d::new(Vec3::new(1990.0, 1000.0, 0.0), Dir3::NEG_Y);
    app.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
    app.update();
    assert_eq!(ghost_status(&mut app), Some(Err(SiteError::OffMap)));
}

#[test]
fn a_click_on_a_blocked_site_is_refused_and_the_placement_stays_armed() {
    let mut app = app(content(None), &[1]);
    armed_at(&mut app, 400.0, 0.0);
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
    app.update();
    assert!(queued(&mut app).is_empty());
    assert!(app.world().resource::<HudState>().placing.is_some());
    // Moving on to open space and clicking places it.
    let ray = Ray3d::new(Vec3::new(-1600.0, 1000.0, 300.0), Dir3::NEG_Y);
    app.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().release(MouseButton::Left);
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().clear();
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
    app.update();
    assert_eq!(queued(&mut app).len(), 1);
    assert!(app.world().resource::<HudState>().placing.is_none());
    app.update();
    assert_eq!(app.world_mut().query::<&ghost::Ghost>().iter(app.world()).count(), 0, "the ghost goes with the placement");
}
