//! UI-19 tests on authored cfg and header text (no game files).

use std::sync::Arc;

use bevy::prelude::*;
use sta_content::{ButtonSlots, ContentDb, GuiCfg};
use sta_sim::command::{CommandKind, Order};
use sta_sim::fake::ScriptedSource;
use sta_sim::view::PlayerView;
use sta_sim::{EntityId, Fixed, PlayerId, TeamId, WorldSource};

use super::super::{StockBody, StockState, layout::StockLayout};
use super::control;
use crate::app::configure_sets;
use crate::sim_link::{CommandQueue, Selection};
use crate::ui::hud::{
    ExternalFeed, HudConfig, HudFeed, HudPlugin, HudState,
    layout::R,
    model::{Act, Look, Page, TargetMode, UnitOrder},
    panels::GridCell,
};
use crate::ui::{profile::UiProfile, theme::click};

/// An authored sheet in Armada I's key family, on the 640 x 480 canvas.
const CFG: &str = "\
resourcePanelArea = 301 3 177 19
minimapPanelArea = 11 301 129 171
infoPanelArea = 163 351 233 127
infoBlackArea = 7 9 211 99
infoSingleNameTextInformationArea = 91 27 101 11
infoSingleClassTextInformationArea = 91 13 103 8
infoSingleShieldBarArea = 33 29 51 6
infoSingleEnergyBarArea = 33 79 51 6
infoSingleCrewTextInformationArea = 71 61 37 11
controlPanelArea = 517 311 109 151
controlBlackArea = 6 6 93 121
controlButton1 = 8 7 29 29
controlButton2 = 43 7 29 29
controlButton3 = 78 7 29 29
controlButton4 = 8 42 29 29
controlButton5 = 43 42 29 29
controlButton6 = 78 42 29 29
controlButton7 = 8 77 29 29
controlButton8 = 43 77 29 29
controlButton9 = 78 77 29 29
controlButton10 = 8 112 29 29
controlButton11 = 43 112 29 29
controlButton12 = 78 112 29 29
cinematicPanelArea = 381 347 139 133
";

/// Authored slots: not the install's numbers.
const HEADER: &str = "\
int BUTTON_LOCATION_MOVE = 9;
int BUTTON_LOCATION_ATTACK = 10;
int BUTTON_LOCATION_STOP = 11;
int BUTTON_LOCATION_ORDERS = 12;
int BUTTON_LOCATION_ORDER_SCOUT = 2;
int BUTTON_LOCATION_ORDER_SEARCH_AND_DESTROY = 3;
int BUTTON_LOCATION_ORDER_PATROL = 4;
int BUTTON_LOCATION_ORDER_RED_ALERT = 6;
int BUTTON_LOCATION_BACK = 8;
int BUTTON_LOCATION_CANCEL = 7;
int BUTTON_LOCATION_BUILD = 1;
";

fn layout_cfg() -> sta_content::GuiLayout {
    let (cfg, warnings) = GuiCfg::parse("authored_a1.cfg", CFG);
    assert!(warnings.is_empty(), "{warnings:?}");
    cfg.layout_auto()
}

fn feed(db: &Arc<ContentDb>) -> HudFeed {
    let mut source = ScriptedSource::from_json5(
        "{ units: [{ id: 1, def: 'fx_cruiser', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] }] }",
        db,
    )
    .unwrap();
    let mut view = (*source.step(&[]).view).clone();
    view.players = vec![PlayerView {
        crew: None,
        id: PlayerId(1),
        team: TeamId(1),
        resources: [30, 20, 70, 5].map(Fixed::from_int).to_vec(),
        officers: [Fixed::ZERO; 2],
        eliminated: false,
        relations: Vec::new(),
    }];
    HudFeed { view: Arc::new(view), player: PlayerId(1), selected: vec![EntityId(1)], content: Some(db.clone()) }
}

fn app(aspect: f32) -> App {
    let db = Arc::new(sta_content::fake::db());
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    configure_sets(&mut app);
    app.insert_resource(UiProfile::Armada1)
        .insert_resource(HudConfig { gui: Some(layout_cfg()), aspect, fit_scale: false, ..default() })
        .insert_resource(feed(&db))
        .insert_resource(ExternalFeed)
        .insert_resource(Selection(vec![EntityId(1)]))
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(HudPlugin);
    app.world_mut().resource_mut::<StockState>().slots = Some(ButtonSlots::from_header(HEADER).0);
    app.update();
    app.update();
    app
}

fn cell(app: &mut App, f: impl Fn(&Act) -> bool) -> Entity {
    app.world_mut().query::<(Entity, &GridCell)>().iter(app.world()).find(|(_, c)| f(&c.0)).expect("no such cell").0
}

fn acts(app: &mut App) -> Vec<Act> {
    app.world_mut().query::<&GridCell>().iter(app.world()).map(|c| c.0.clone()).collect()
}

fn press(app: &mut App, e: Entity) {
    click(app.world_mut(), e);
    app.update();
    app.update();
}

#[test]
fn slot_rectangles_are_the_sheet_keys_on_one_640_by_480_canvas() {
    let l = StockLayout::new(&layout_cfg(), Vec2::new(640.0, 480.0));
    assert_eq!(l.canvas, Vec2::new(640.0, 480.0));
    let cells = control::cells(&l);
    assert_eq!(cells[0], Some(R::new(525.0, 318.0, 29.0, 29.0)));
    assert_eq!(cells[11], Some(R::new(595.0, 423.0, 29.0, 29.0)));
    assert!(cells.iter().all(Option::is_some));
    // The same sheet at 16:9 and 21:9 is one factor, centred: the grid keeps its shape (ADR 0008 H1, H2).
    for aspect in [16.0 / 9.0, 21.0 / 9.0] {
        let l = StockLayout::for_aspect(&layout_cfg(), aspect);
        let c = control::cells(&l);
        let (a, b) = (c[0].unwrap(), c[1].unwrap());
        assert_eq!((a.w, a.h), (c[11].unwrap().w, c[11].unwrap().h));
        assert!(((b.x - a.x) / l.scale - 35.0).abs() < 0.6, "pitch scales by the one factor");
        assert!(a.x >= l.hud.x && c[11].unwrap().right() <= l.hud.right());
    }
}

#[test]
fn panel_rectangles_stay_in_the_sheet_group() {
    let l = StockLayout::new(&layout_cfg(), Vec2::new(640.0, 480.0));
    let solid = super::solid(&l);
    assert!(solid.contains(&R::new(517.0, 311.0, 109.0, 151.0)));
    assert!(solid.contains(&R::new(163.0, 351.0, 233.0, 127.0)));
    assert!(solid.contains(&R::new(381.0, 347.0, 139.0, 133.0)), "the cinematic rectangle is reserved");
}

fn selection_feed() -> HudFeed {
    feed(&Arc::new(sta_content::fake::db()))
}

#[test]
fn top_page_puts_commands_in_the_header_slots() {
    let slots = ButtonSlots::from_header(HEADER).0;
    let b = control::buttons(&selection_feed(), Page::Top, &slots, None);
    let at = |slot: usize| b.iter().find(|b| b.cell == slot - 1).map(|b| b.act.clone());
    assert_eq!(at(9), Some(Act::Target(TargetMode::Move)));
    assert_eq!(at(11), Some(Act::Stop));
    assert_eq!(at(12), Some(Act::OpenOrders));
    assert_eq!(at(6), None, "orders-page commands are not on the top page");
    assert_eq!(at(8), None, "back is not on the top page");
    assert!(b.windows(2).all(|w| w[0].cell < w[1].cell));
}

#[test]
fn orders_page_has_its_slots_and_locks_what_the_sim_cannot_take() {
    let slots = ButtonSlots::from_header(HEADER).0;
    let b = control::buttons(&selection_feed(), Page::Orders, &slots, None);
    let at = |slot: usize| b.iter().find(|b| b.cell == slot - 1).cloned();
    assert_eq!(at(2).unwrap().act, Act::Order(UnitOrder::Scout));
    assert_eq!(at(2).unwrap().look, Look::Ready);
    assert_eq!(at(4).unwrap().look, Look::Locked, "patrol has no HUD pick yet");
    assert_eq!(at(6).unwrap().look, Look::Locked, "alert states have no player order yet");
    assert_eq!(at(8).unwrap().act, Act::Back);
    assert_eq!(at(7), None, "cancel belongs to the build page");
    let none = control::buttons(&HudFeed::default(), Page::Orders, &slots, None);
    assert!(none.is_empty(), "nothing selected: no grid");
}

#[test]
fn a_header_without_keys_gives_an_empty_grid() {
    assert!(control::buttons(&selection_feed(), Page::Top, &ButtonSlots::default(), None).is_empty());
}

#[test]
fn clicking_slots_issues_orders_through_the_shared_path_and_pages() {
    let mut app = app(4.0 / 3.0);
    let stop = cell(&mut app, |a| *a == Act::Stop);
    press(&mut app, stop);
    let sent = std::mem::take(&mut app.world_mut().resource_mut::<CommandQueue>().0);
    assert!(matches!(sent.as_slice(), [CommandKind::Order(Order::Stop { units })] if units == &[EntityId(1)]), "{sent:?}");

    let orders = cell(&mut app, |a| *a == Act::OpenOrders);
    press(&mut app, orders);
    assert_eq!(app.world().resource::<HudState>().page, Page::Orders);
    assert!(acts(&mut app).contains(&Act::Order(UnitOrder::Scout)));
    assert!(!acts(&mut app).contains(&Act::Stop), "the Orders page replaces the top page");
    let scout = cell(&mut app, |a| *a == Act::Order(UnitOrder::Scout));
    press(&mut app, scout);
    let sent = std::mem::take(&mut app.world_mut().resource_mut::<CommandQueue>().0);
    assert!(matches!(sent.as_slice(), [CommandKind::Order(Order::Scout { .. })]), "{sent:?}");

    let back = cell(&mut app, |a| *a == Act::Back);
    press(&mut app, back);
    assert_eq!(app.world().resource::<HudState>().page, Page::Top);
    assert!(acts(&mut app).contains(&Act::Stop));
}

#[test]
fn a_locked_slot_issues_nothing() {
    let mut app = app(4.0 / 3.0);
    let orders = cell(&mut app, |a| *a == Act::OpenOrders);
    press(&mut app, orders);
    let patrol = cell(&mut app, |a| *a == Act::Order(UnitOrder::Unavailable));
    press(&mut app, patrol);
    assert!(app.world().resource::<CommandQueue>().0.is_empty());
    assert_eq!(app.world().resource::<HudState>().page, Page::Orders);
}

#[test]
fn the_orders_page_closes_when_the_selection_goes() {
    let mut app = app(4.0 / 3.0);
    app.world_mut().resource_mut::<HudState>().page = Page::Orders;
    app.world_mut().resource_mut::<HudFeed>().selected.clear();
    app.update();
    app.update();
    assert_eq!(app.world().resource::<HudState>().page, Page::Top);
    assert!(acts(&mut app).is_empty());
}

#[test]
fn the_body_is_built_for_armada_one_only() {
    let mut app = app(16.0 / 9.0);
    assert_eq!(app.world_mut().query_filtered::<(), With<StockBody>>().iter(app.world()).count(), 1);
    assert!(!acts(&mut app).is_empty());
}

#[test]
fn emblem_names_follow_the_race_name_or_stem() {
    assert_eq!(super::info::emblem_names("federation"), ["federation_icon"]);
    assert_eq!(super::info::emblem_names("kli")[1], "klingon_icon");
    assert!(super::info::emblem_names("").is_empty());
}

#[test]
fn changing_energy_and_crew_patches_in_place_and_keeps_the_buttons() {
    let mut app = app(4.0 / 3.0);
    let set = |app: &mut App, energy: i32, crew: i32| {
        let mut feed = app.world_mut().resource_mut::<HudFeed>();
        let unit = &mut Arc::make_mut(&mut feed.view).units[0];
        unit.energy = Some([Fixed::from_int(energy), Fixed::from_int(100)]);
        unit.crew = Some([Fixed::from_int(crew), Fixed::from_int(100)]);
    };
    set(&mut app, 40, 90);
    app.update();
    app.update();
    let buttons = |app: &mut App| {
        let mut v: Vec<Entity> = app.world_mut().query_filtered::<Entity, With<GridCell>>().iter(app.world()).collect();
        v.sort();
        v
    };
    let crew_text = |app: &mut App| {
        app.world_mut().query_filtered::<&crate::ui::sprite_font::SpriteText, With<super::info::CrewText>>().iter(app.world()).map(|t| t.text.clone()).collect::<Vec<_>>()
    };
    let before = buttons(&mut app);
    assert!(!before.is_empty());
    assert_eq!(crew_text(&mut app), ["90"]);
    set(&mut app, 41, 89);
    app.update();
    app.update();
    assert_eq!(buttons(&mut app), before, "the buttons are not respawned when energy or crew change");
    assert_eq!(crew_text(&mut app), ["89"], "the crew readout is patched in place");
}
