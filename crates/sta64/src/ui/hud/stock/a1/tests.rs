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
use crate::ui::{gui_sprites::GuiImage, profile::UiProfile, theme::click};

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
infoSingleCrewTextLabel = \"MY_CREW_LABEL\"
infoSingleCrewTextLabelArea = 40 61 30 11
infoSingleCrewTextColor = 0 1 0
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
cinematicDisplayArea = 11 33 119 88
cinematicCommButton = \"cine_comm\"
cinematicMenuButton = \"cine_menu\"
cinematicCommButtonArea = 12 12 50 16
cinematicMenuButtonArea = 72 12 50 16
speedPanelArea = 100 200 300 50
speedSingleButtonArea = 10 6 24 20
speedSeparatorArea = 10 6 6 30
speedSingleButtonGap = 5
speedSeparator = \"sep_sprite\"
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
    HudFeed { view: Arc::new(view), player: PlayerId(1), selected: vec![EntityId(1)], content: Some(db.clone()), ..Default::default() }
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
fn a1_compat_stock_slots_show_selected_unit_commands() {
    let slots = ButtonSlots::a1_compat_from_header("").0;
    let b = control::buttons(&selection_feed(), Page::Top, &slots, None);
    let at = |slot: usize| b.iter().find(|b| b.cell == slot - 1).map(|b| b.act.clone());
    assert_eq!(at(1), Some(Act::Target(TargetMode::Move)));
    assert_eq!(at(2), Some(Act::Target(TargetMode::Attack)));
    assert_eq!(at(3), Some(Act::Stop));
    assert_eq!(at(5), Some(Act::OpenOrders));
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

/// The authored crew label colour and the compatibility renderer's green amount.
#[test]
fn the_crew_label_uses_its_cfg_colour_and_the_amount_is_green() {
    let mut app = app(4.0 / 3.0);
    let (cfg, _) = GuiCfg::parse("crew.cfg", &CFG.replace("infoSingleCrewTextColor = 0 1 0", "infoSingleCrewTextColor = 1 1 1"));
    app.world_mut().resource_mut::<HudConfig>().gui = Some(cfg.layout_auto());
    {
        let mut feed = app.world_mut().resource_mut::<HudFeed>();
        let unit = &mut Arc::make_mut(&mut feed.view).units[0];
        unit.crew = Some([Fixed::from_int(90), Fixed::from_int(100)]);
    }
    app.update();
    app.update();
    let texts: Vec<(String, Color)> = app.world_mut().query::<&crate::ui::sprite_font::SpriteText>().iter(app.world()).map(|t| (t.text.clone(), t.color)).collect();
    let green = Color::srgba(0.0, 1.0, 0.0, 1.0);
    let white = Color::srgb(1.0, 1.0, 1.0);
    assert!(texts.iter().any(|(t, c)| t == "CREW" && *c == white), "{texts:?}");
    assert!(texts.iter().any(|(t, c)| t == "90" && *c == green), "{texts:?}");
}

#[test]
fn absent_identity_colours_use_black_and_the_class_has_its_panel_piece() {
    let mut app = app(4.0 / 3.0);
    let cfg = CFG.replace("infoSingleCrewTextColor = 0 1 0", "");
    let sheet = format!("{cfg}\ninfoSingleBackground=\"single_background\"\ninfoSingleBackgroundSize=1\ninfoSingleBackground_0=90 12 112 11\n");
    let (cfg, warnings) = GuiCfg::parse("a1_text.cfg", &sheet);
    assert!(warnings.is_empty(), "{warnings:?}");
    app.world_mut().resource_mut::<HudConfig>().gui = Some(cfg.layout_auto());
    Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).units[0].crew =
        Some([Fixed::from_int(90), Fixed::from_int(100)]);
    app.update();
    app.update();

    let world = app.world_mut();
    let mut texts = world.query::<&crate::ui::sprite_font::SpriteText>();
    assert!(texts.iter(world).any(|t| t.text == "CREW" && t.color == Color::BLACK));
    assert!(world.query::<&GuiImage>().iter(world).any(|i| i.names == ["single_background"]));
}

#[test]
fn status_uses_authored_tracks_and_live_shield_and_team_tints() {
    use super::info::LiveTint;
    use crate::viewer::TeamColours;

    let mut app = app(4.0 / 3.0);
    let extras = "infoSingleRaceIconArea=34 13 60 13\ninfoSingleRaceIconDisplayArea=55 13 19 13\ninfoSingleShieldBarDisableColor=0.2 0.3 0.4\n";
    let (cfg, warnings) = GuiCfg::parse("tints.cfg", &format!("{CFG}{extras}"));
    assert!(warnings.is_empty(), "{warnings:?}");
    app.world_mut().resource_mut::<HudConfig>().gui = Some(cfg.layout_auto());
    let owner = PlayerId(1);
    let team = Color::srgb(0.25, 0.5, 1.0);
    app.insert_resource(TeamColours::default().with_local_colour(owner, team, &[owner]));
    let set_shields = |app: &mut App, shields: i32| {
        let mut feed = app.world_mut().resource_mut::<HudFeed>();
        Arc::make_mut(&mut feed.view).units[0].shields[0] = [Fixed::from_int(shields), Fixed::from_int(100)];
    };
    set_shields(&mut app, 25);
    app.update();
    app.update();

    let mut query = app.world_mut().query::<(Entity, &GuiImage, Option<&LiveTint>)>();
    let mut shield = None;
    let mut race_layers = 0;
    let mut track = false;
    for (entity, image, tint) in query.iter(app.world()) {
        match tint {
            Some(LiveTint::Shield(_)) => {
                assert_eq!(image.color, Color::srgb(0.75, 0.25, 0.0));
                shield = Some(entity);
            }
            Some(LiveTint::Race(_)) => {
                assert_eq!(image.color, team);
                race_layers += 1;
            }
            None if image.names.iter().any(|name| name == "large_shield_bar") => {
                assert_eq!(image.color, Color::srgb(0.2, 0.3, 0.4));
                track = true;
            }
            _ => {}
        }
    }
    assert!(track);
    assert_eq!(race_layers, 2);
    let shield = shield.expect("shield fill");
    set_shields(&mut app, 75);
    app.update();
    assert_eq!(app.world().get::<GuiImage>(shield).unwrap().color, Color::srgb(0.25, 0.75, 0.0));
}

fn find<C: Component>(app: &mut App, f: impl Fn(&C) -> bool) -> Vec<Entity> {
    app.world_mut().query::<(Entity, &C)>().iter(app.world()).filter(|(_, c)| f(c)).map(|(e, _)| e).collect()
}

fn with_view(mut app: App) -> App {
    app.world_mut().insert_resource(crate::render::action_camera::ActionView { image: Handle::default() });
    app.world_mut().insert_resource(crate::render::action_camera::ActionSubject(Some(EntityId(1))));
    let unit = app.world_mut().spawn_empty().id();
    app.world_mut().insert_resource(crate::sim_link::SimEntities([(EntityId(1), unit)].into_iter().collect()));
    // Rebuild the body with the picture's resource in place.
    app.world_mut().resource_mut::<HudConfig>().aspect += 0.001;
    app.update();
    app.update();
    app
}

#[test]
fn the_window_has_comm_and_menu_at_their_cfg_places_inside_the_panel_on_every_aspect() {
    use crate::ui::hud::stock::buttons::PanelButton;
    use crate::ui::theme::WidgetState;
    for aspect in [4.0 / 3.0, 16.0 / 9.0, 21.0 / 9.0] {
        let mut app = app(aspect);
        let layout = app.world().resource::<StockState>().layout.clone().expect("layout");
        let panel = layout.panel("cinematic").unwrap();
        let mut q = app.world_mut().query::<(&PanelButton, &Node, &WidgetState)>();
        let buttons: Vec<_> = q.iter(app.world()).filter(|(a, ..)| matches!(a, PanelButton::Menu | PanelButton::Unavailable)).collect();
        // Only the window's two buttons exist on the Armada I sheet (the Armada II panel buttons are not drawn).
        assert_eq!(buttons.len(), 2, "{aspect}");
        for (_, node, _) in &buttons {
            let (Val::Px(x), Val::Px(y), Val::Px(w), Val::Px(h)) = (node.left, node.top, node.width, node.height) else { panic!("px") };
            let r = R::new(x, y, w, h);
            assert!(r.x >= panel.x && r.y >= panel.y && r.right() <= panel.right() && r.bottom() <= panel.bottom(), "{aspect}: inside the panel");
            assert!(r.x >= layout.hud.x && r.right() <= layout.hud.right(), "{aspect}: inside the sheet group");
        }
        let at = |key: &str| layout.at(Vec2::new(panel.x, panel.y), key).unwrap();
        let rects: Vec<R> = buttons.iter().map(|(_, n, _)| { let (Val::Px(x), Val::Px(y), Val::Px(w), Val::Px(h)) = (n.left, n.top, n.width, n.height) else { unreachable!() }; R::new(x, y, w, h) }).collect();
        assert!(rects.contains(&at("cinematicMenuButtonArea")) && rects.contains(&at("cinematicCommButtonArea")));
        let state_of = |act: PanelButton| buttons.iter().find(|(a, ..)| **a == act).map(|(_, _, s)| **s);
        assert_eq!(state_of(PanelButton::Menu), Some(WidgetState::Normal));
        assert_eq!(state_of(PanelButton::Unavailable), Some(WidgetState::Disabled), "no comm menu in a match");
    }
}

#[test]
fn menu_opens_the_in_game_menu_and_comm_does_nothing() {
    use crate::ui::hud::stock::buttons::PanelButton;
    use crate::ui::ingame_menu::OpenInGameMenu;
    let mut app = app(16.0 / 9.0);
    let menu = find::<PanelButton>(&mut app, |b| *b == PanelButton::Menu);
    let comm = find::<PanelButton>(&mut app, |b| *b == PanelButton::Unavailable);
    assert_eq!((menu.len(), comm.len()), (1, 1));
    press(&mut app, comm[0]);
    assert_eq!(app.world().resource::<Messages<OpenInGameMenu>>().len(), 0, "a disabled button sends nothing");
    press(&mut app, menu[0]);
    assert_eq!(app.world().resource::<Messages<OpenInGameMenu>>().len(), 1);
}

#[test]
fn the_picture_is_drawn_in_the_display_area_and_a_click_goes_to_the_subject() {
    use crate::input::select::CameraFocus;
    use crate::render::action_camera::ActionPicture;
    let mut app = with_view(app(16.0 / 9.0));
    let layout = app.world().resource::<StockState>().layout.clone().unwrap();
    let panel = layout.panel("cinematic").unwrap();
    let picture = app.world_mut().query_filtered::<(Entity, &Node), With<ActionPicture>>().iter(app.world()).map(|(e, n)| (e, n.clone())).collect::<Vec<_>>();
    assert_eq!(picture.len(), 1);
    // #1073: the shared window code left the Armada I buttons in place.
    let buttons = app.world_mut().query::<&crate::ui::hud::stock::buttons::PanelButton>().iter(app.world()).filter(|b| matches!(b, crate::ui::hud::stock::buttons::PanelButton::Menu | crate::ui::hud::stock::buttons::PanelButton::Unavailable)).count();
    assert_eq!(buttons, 2, "the Armada I window keeps its COMM and MENU buttons");
    let want = layout.at(Vec2::new(panel.x, panel.y), "cinematicDisplayArea").unwrap();
    assert_eq!((picture[0].1.left, picture[0].1.width), (Val::Px(want.x), Val::Px(want.w)));
    press(&mut app, picture[0].0);
    let focus: Vec<_> = app.world_mut().resource_mut::<Messages<CameraFocus>>().drain().collect();
    assert_eq!(focus.len(), 1, "one click, one move");
    // The subject leaves sight (the renderer no longer has it): the click moves nothing.
    app.world_mut().resource_mut::<crate::sim_link::SimEntities>().0.clear();
    press(&mut app, picture[0].0);
    assert_eq!(app.world_mut().resource_mut::<Messages<CameraFocus>>().drain().count(), 0, "no jump to an unseen unit");
}

#[test]
fn without_the_action_view_resource_there_is_no_picture_node() {
    use crate::render::action_camera::ActionPicture;
    let mut app = app(4.0 / 3.0);
    assert_eq!(app.world_mut().query_filtered::<(), With<ActionPicture>>().iter(app.world()).count(), 0);
}

// ---- the speed rail's TRANSPORT button (the sheet's rail numbers above are made up for the test, not any install's)

use sta_content::Profile;
use sta_sim::view::{UnitFlags, UnitView};
use super::speed;

/// `fake::db()`'s units under Armada I's game family (every crewed craft may transport).
fn a1_db() -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(Profile::Sta1Classic).features(Profile::Sta1Classic.features());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    Arc::new(b.build().unwrap())
}

/// Own cruiser 1 with `crew` of 50 (None: no crew in the view), enemy cruiser 2 with 30 of 50.
fn rail_feed(crew: Option<i32>) -> HudFeed {
    let db = a1_db();
    let unit = |id: u32, owner: u8, x: f32, c: Option<[Fixed; 2]>| UnitView {
        id: EntityId(id), def: db.key("fx_cruiser").unwrap(), owner: PlayerId(owner), pos: sta_sim::Vec3::from_f32([x, 0.0, 0.0]),
        heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO, hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4], crew: c,
        energy: None, autonomy: None, flags: UnitFlags::SELECTABLE, progress: None,
    };
    let view = sta_sim::WorldView {
        units: vec![
            unit(1, 1, 0.0, crew.map(|c| [Fixed::from_int(c), Fixed::from_int(50)])),
            unit(2, 2, 100.0, Some([Fixed::from_int(30), Fixed::from_int(50)])),
        ],
        players: vec![
            PlayerView { crew: None, id: PlayerId(1), team: TeamId(1), resources: Vec::new(), officers: [Fixed::ZERO; 2], eliminated: false, relations: Vec::new() },
            PlayerView { crew: None, id: PlayerId(2), team: TeamId(2), resources: Vec::new(), officers: [Fixed::ZERO; 2], eliminated: false, relations: Vec::new() },
        ],
        ..Default::default()
    };
    HudFeed { view: Arc::new(view), player: PlayerId(1), selected: vec![EntityId(1)], content: Some(db), ..Default::default() }
}

fn rail_app(f: HudFeed) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    configure_sets(&mut app);
    app.insert_resource(UiProfile::Armada1)
        .insert_resource(HudConfig { gui: Some(layout_cfg()), aspect: 4.0 / 3.0, fit_scale: false, ..default() })
        .insert_resource(f)
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

#[test]
fn the_rail_positions_come_from_the_sheet_keys() {
    let l = StockLayout::new(&layout_cfg(), Vec2::new(640.0, 480.0));
    // Origin (100, 200); a button advances by its width and the gap (24 + 5), the separator by its own
    // width and the gap (6 + 5); position 5 is the separator, position 6 transport.
    assert_eq!(speed::rect(&l, 0), Some(R::new(110.0, 206.0, 24.0, 20.0)));
    assert_eq!(speed::rect(&l, 4), Some(R::new(226.0, 206.0, 24.0, 20.0)));
    assert_eq!(speed::rect(&l, 5), Some(R::new(255.0, 206.0, 6.0, 30.0)));
    assert_eq!(speed::rect(&l, 6), Some(R::new(266.0, 206.0, 24.0, 20.0)));
    assert_eq!(speed::rect(&l, 7), None);
    assert_eq!(speed::separator(&l).map(|(_, name)| name), Some("sep_sprite".to_owned()));
    // Inside the panel, and on 16:9 the same shape under the one factor.
    assert!(speed::rect(&l, 6).unwrap().right() <= l.panel("speed").unwrap().right());
    let w = StockLayout::for_aspect(&layout_cfg(), 16.0 / 9.0);
    let (a, b) = (speed::rect(&w, 0).unwrap(), speed::rect(&w, 6).unwrap());
    assert_eq!(a.w, b.w);
    assert!(((b.x - a.x) / w.scale - 156.0).abs() < 1.5, "five buttons and the separator, each with the gap: {} {}", a.x, b.x);
}

#[test]
fn the_rail_shows_transport_for_a_crewed_selection_with_the_game_art() {
    let b = speed::buttons(&rail_feed(Some(40)), None);
    let [t] = b.as_slice() else { panic!("{b:?}") };
    assert_eq!(t.act, Act::Target(TargetMode::Transport));
    assert_eq!(t.sprite_names, ["b_transport"]);
    assert_eq!(t.look, Look::Ready);
    assert_eq!(t.cell, speed::RAIL_BASE + 6);
    // Crew at or below the carrier floor: shown, disabled (the sim would drop the order).
    assert_eq!(speed::buttons(&rail_feed(Some(8)), None)[0].look, Look::Locked);
    // No crew in the view, someone else's unit selected, nothing selected: no button.
    assert!(speed::buttons(&rail_feed(None), None).is_empty());
    let mut other = rail_feed(Some(40));
    other.selected = vec![EntityId(2)];
    assert!(speed::buttons(&other, None).is_empty());
    assert!(speed::buttons(&HudFeed::default(), None).is_empty());
}

#[test]
fn the_rail_button_is_drawn_at_its_place_with_its_separator_and_not_on_the_control_grid() {
    let mut app = rail_app(rail_feed(Some(40)));
    let t = cell(&mut app, |a| *a == Act::Target(TargetMode::Transport));
    let l = StockLayout::for_aspect(&layout_cfg(), 4.0 / 3.0);
    let node = app.world().get::<Node>(t).unwrap();
    let want = speed::rect(&l, 6).unwrap();
    assert_eq!((node.left, node.top, node.width, node.height), (Val::Px(want.x), Val::Px(want.y), Val::Px(want.w), Val::Px(want.h)));
    let sep = speed::rect(&l, 5).unwrap();
    let seps = find::<GuiImage>(&mut app, |_| true)
        .into_iter()
        .filter(|e| app.world().get::<Node>(*e).is_some_and(|n| n.left == Val::Px(sep.x) && n.top == Val::Px(sep.y)))
        .count();
    assert_eq!(seps, 1, "one separator at the rail's sixth position");
    // The control grid keeps its slots: Transport takes none of them (BUTTON_LOCATION_TRANSPORT is -1).
    assert!(acts(&mut app).contains(&Act::Stop));
    // No transporter selected: neither button nor separator.
    let mut none = rail_app(rail_feed(None));
    assert!(!acts(&mut none).contains(&Act::Target(TargetMode::Transport)));
}

#[test]
fn pressing_the_rail_button_arms_the_pick_and_the_click_issues_one_transport_order() {
    let mut app = rail_app(rail_feed(Some(40)));
    let t = cell(&mut app, |a| *a == Act::Target(TargetMode::Transport));
    press(&mut app, t);
    assert_eq!(app.world().resource::<HudState>().targeting, Some(TargetMode::Transport));
    // The click on a unit takes the same target gate and order as the Armada II button.
    let f = app.world().resource::<HudFeed>().clone();
    let (orders, _) = crate::ui::hud::target_order(&f, TargetMode::Transport, Some(EntityId(2)), None).expect("an enemy ship is a valid pick");
    assert!(matches!(orders.as_slice(), [Order::Transport { units, target: EntityId(2), attack: false }] if units == &[EntityId(1)]), "{orders:?}");
    // Disabled: no pick is armed, nothing is sent.
    let mut low = rail_app(rail_feed(Some(8)));
    let t = cell(&mut low, |a| *a == Act::Target(TargetMode::Transport));
    press(&mut low, t);
    assert_eq!(low.world().resource::<HudState>().targeting, None);
    assert!(low.world().resource::<CommandQueue>().0.is_empty());
}
