use std::sync::Arc;

use bevy::prelude::*;
use sta_content::{ContentDb, DefRef, GuiCfg};
use sta_sim::command::{CommandKind, Order};
use sta_sim::fake::ScriptedSource;
use sta_sim::view::PlayerView;
use sta_sim::{EntityId, Fixed, PlayerId, TeamId, WorldSource};

use super::*;
use crate::app::configure_sets;
use crate::sim_link::{CommandQueue, Selection};
use crate::ui::hud::{Armed, ExternalFeed, HudPlugin};
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::click;


const CFG: &str = "\
screenWidth = 1600
screenHeight = 1200
resourcePanelArea = 0 0 400 80
resource_1 = 220 10 80 30
resource_2 = 20 10 80 30
resource_4 = 120 10 80 30
minimapPanelArea = 0 800 400 400
minimapDisplayArea = 10 10 320 320
infoPanelArea = 400 900 500 300
infoPanelArea_1 = 400 900 500 300
infoPanelArea_2 = 400 900 500 300
infoSingleNameTextArea = 10 10 200 30
infoSingleHullBarArea = 10 50 200 12
hull = 10 70 200 30
paletteSingleButtonArea = 0 0 80 40
paletteSingleButtonGap = 5
popupPaletteXA = 1000
popupPaletteYA = 800
staticPaletteWidthA = 3
staticPaletteHeightA = 3
";

fn config() -> HudConfig {
    let (cfg, warnings) = GuiCfg::parse("synthetic_stock.cfg", CFG);
    assert!(warnings.is_empty(), "{warnings:?}");
    HudConfig {
        gui: Some(cfg.layout_auto()),
        fit_scale: false,
        ..default()
    }
}

fn content_with_build_count(build_count: usize) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut builder = ContentDb::builder()
        .profile(fake.profile())
        .features(fake.features().clone());
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    for unit in fake.units() {
        let mut unit = unit.clone();
        if unit.odf == "fx_cruiser" {
            unit.build.list = if build_count == 1 {
                vec![DefRef::new("fx_station")]
            } else {
                (0..build_count)
                    .map(|i| DefRef::new(format!("fx_build_{i:02}")))
                    .collect()
            };
        }
        builder = builder.unit(unit);
    }
    if build_count > 1 {
        let station = fake.units().find(|u| u.odf == "fx_station").unwrap();
        for i in 0..build_count {
            let mut unit = station.clone();
            unit.odf = format!("fx_build_{i:02}");
            builder = builder.unit(unit);
        }
    }
    Arc::new(builder.build().unwrap())
}

fn content() -> Arc<ContentDb> {
    content_with_build_count(1)
}

fn feed(db: &Arc<ContentDb>) -> HudFeed {
    let mut source = ScriptedSource::from_json5(
        "{ units: [{ id: 1, def: 'fx_cruiser', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] }] }",
        db,
    )
    .unwrap();
    let mut view = (*source.step(&[]).view).clone();
    view.players = vec![PlayerView { crew: None,
        id: PlayerId(1),
        team: TeamId(1),
        resources: [3000, 20, 700, 5].map(Fixed::from_int).to_vec(),
        officers: [Fixed::ZERO; 2],
        eliminated: false,
        relations: Vec::new(),
    }];
    HudFeed {
        view: Arc::new(view),
        player: PlayerId(1),
        selected: vec![EntityId(1)],
        content: Some(db.clone()), ..Default::default()
    }
}

fn app_with_content(db: Arc<ContentDb>) -> App {
    app_with_content_and_config(db, config(), UiProfile::Armada2)
}

fn app_with_content_and_config(db: Arc<ContentDb>, config: HudConfig, profile: UiProfile) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    configure_sets(&mut app);
    app.insert_resource(profile)
        .insert_resource(config)
        .insert_resource(feed(&db))
        .insert_resource(ExternalFeed)
        .insert_resource(Selection(vec![EntityId(1)]))
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(HudPlugin);
    app.update();
    app
}

fn app() -> App {
    app_with_content(content())
}

fn inherited_config(overlay: &str, base_extra: &str) -> HudConfig {
    let race = format!("#include \"interface.cfg\"\n{overlay}");
    let interface = format!("{CFG}\ninfoSingleClassTextArea = 10 100 200 30\ninfoBuildName = 20 10 200 30\ninfoBuildClass = 20 100 200 30\nshipName = 10 140 200 30\n{base_extra}");
    let (cfg, warnings) = GuiCfg::load("race.cfg", &mut |name| match name {
        "race.cfg" => Some(race.clone()),
        "interface.cfg" => Some(interface.clone()),
        _ => None,
    });
    assert!(warnings.is_empty(), "{warnings:?}");
    HudConfig {
        gui: Some(cfg.layout_auto()),
        fit_scale: false,
        ..default()
    }
}

fn stock_colour_at(app: &mut App, key: &str, top_strip: bool) -> Color {
    let expected = {
        let layout = app.world().resource::<StockState>().layout.as_ref().unwrap();
        let info = model::info(app.world().resource::<HudFeed>());
        let variant = layout.info_variant(&info);
        let mut origin = layout.info_origin(variant);
        if top_strip {
            origin += layout.point(&format!("infoBarOffset_{variant}")).unwrap_or(Vec2::ZERO);
        }
        layout.at(origin, key).unwrap()
    };
    app.world_mut()
        .query::<(&SpriteText, &ChildOf)>()
        .iter(app.world())
        .filter_map(|(text, parent)| {
            let node = app.world().get::<Node>(parent.parent())?;
            ((node.left, node.top) == (Val::Px(expected.x), Val::Px(expected.y))).then_some(text.color)
        })
        .next()
        .unwrap_or_else(|| panic!("missing SpriteText at {key}"))
}

fn resource_colour(app: &mut App, id: usize) -> Color {
    app.world_mut()
        .query::<(&info::StockText, &SpriteText)>()
        .iter(app.world())
        .find_map(|(kind, text)| matches!(kind, info::StockText::Resource(n) if *n == id).then_some(text.color))
        .unwrap()
}

fn resource_label(app: &mut App, id: usize) -> Option<(Entity, String, Color, Node)> {
    app.world_mut()
        .query::<(Entity, &info::StockText, &SpriteText, &ChildOf)>()
        .iter(app.world())
        .find_map(|(entity, kind, text, parent)| {
            matches!(kind, info::StockText::Resource(n) if *n == id).then(|| {
                (entity, text.text.clone(), text.color, app.world().get::<Node>(parent.parent()).unwrap().clone())
            })
        })
}

fn set_personnel(app: &mut App, crew: Option<Fixed>, officers: [Fixed; 2]) {
    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut view = (*feed.view).clone();
    view.players[0].crew = crew;
    view.players[0].officers = officers;
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
}

#[test]
fn personnel_uses_inherited_cfg_rectangles_colours_and_live_values() {
    let cfg = inherited_config(
        "resourcePanelArea = 13 7 400 80\nresource_0 = 17 9 73 21\nresource_0_color = 0 1 1 1\nresource_1_color = 1 1 0 1\n",
        "resource_0 = 4 5 30 10\nresource_0_color = 1 0 0 1\n",
    );
    let mut app = app_with_content_and_config(content(), cfg, UiProfile::Armada2);
    let (crew_entity, crew, crew_color, crew_node) = resource_label(&mut app, 0).unwrap();
    let (officer_entity, officers, officer_color, officer_node) = resource_label(&mut app, 1).unwrap();
    assert_eq!(crew, "");
    assert_eq!(officers, "0/0");
    assert_eq!(crew_color, Color::srgba(0.0, 1.0, 1.0, 1.0));
    assert_eq!(officer_color, Color::srgba(1.0, 1.0, 0.0, 1.0));
    // The 1600x1200 CFG on the 1920x1080 test viewport: one factor 0.9, HUD 1440 wide, centred (x + 240).
    assert_eq!((crew_node.left, crew_node.top, crew_node.width, crew_node.height),
        (Val::Px(267.0), Val::Px(14.0), Val::Px(66.0), Val::Px(19.0)));
    assert_eq!((officer_node.left, officer_node.top, officer_node.width, officer_node.height),
        (Val::Px(450.0), Val::Px(15.0), Val::Px(72.0), Val::Px(27.0)));
    set_personnel(&mut app, Some(Fixed::from_int(1200)), [Fixed::from_int(16), Fixed::from_int(600)]);
    assert_eq!(resource_label(&mut app, 0).unwrap().0, crew_entity);
    assert_eq!(resource_label(&mut app, 0).unwrap().1, "1200");
    assert_eq!(resource_label(&mut app, 1).unwrap().0, officer_entity);
    assert_eq!(resource_label(&mut app, 1).unwrap().1, "16/600");
    set_personnel(&mut app, Some(Fixed::from_int(30000)), [Fixed::from_int(16), Fixed::from_int(300)]);
    assert_eq!(resource_label(&mut app, 0).unwrap().1, "30000");
    assert_eq!(resource_label(&mut app, 1).unwrap().1, "16/300");
    set_personnel(&mut app, None, [Fixed::ZERO; 2]);
    assert_eq!(resource_label(&mut app, 0).unwrap().1, "");
    set_personnel(&mut app, Some(Fixed::ZERO), [Fixed::ZERO; 2]);
    assert_eq!(resource_label(&mut app, 0).unwrap().1, "0");
}

/// An authored sheet (own values) with the Armada I shape: no `resourcePanelArea`, three
/// panels each with a background piece, an icon and a panel-relative text area.
const A1_RESOURCES: &str = "\
screenWidth = 640
screenHeight = 480
officerPanelArea = 500 2 80 14
officerBackgroundPanel = \"res_bg_c\"
officerBackgroundPanelSize = 1
officerBackgroundPanel_0 = 0 0 80 14
officerIcon = \"res_icon_c\"
officerIconArea = 3 2 9 9
officerTextArea = 14 1 60 13
crewPanelArea = 400 2 80 14
crewBackgroundPanel = \"res_bg_b\"
crewBackgroundPanelSize = 1
crewBackgroundPanel_0 = 0 0 80 14
crewIcon = \"res_icon_b\"
crewIconArea = 3 2 9 9
crewTextArea = 14 1 60 13
dilithiumPanelArea = 300 2 80 14
dilithiumBackgroundPanel = \"res_bg_a\"
dilithiumBackgroundPanelSize = 1
dilithiumBackgroundPanel_0 = 0 0 80 14
dilithiumIcon = \"res_icon_a\"
dilithiumIconArea = 3 2 9 9
dilithiumTextArea = 14 1 60 13
";

fn single_file_config(raw: &str) -> HudConfig {
    let (cfg, _) = GuiCfg::parse("gui_fed.cfg", raw);
    HudConfig { gui: Some(cfg.layout_auto()), fit_scale: false, ..default() }
}

fn resource_label_count(app: &mut App) -> usize {
    app.world_mut().query::<&info::StockText>().iter(app.world()).count()
}

#[test]
fn armada1_separate_panels_draw_three_readouts_with_live_values() {
    let mut app = app_with_content_and_config(content(), single_file_config(A1_RESOURCES), UiProfile::Armada2);
    assert_eq!(resource_label_count(&mut app), 3);
    // 640x480 canvas on the 1920x1080 test viewport: one factor 2.25, HUD 1440 wide, centred (x + 240).
    let (_, dilithium, _, node) = resource_label(&mut app, 2).unwrap();
    assert_eq!(dilithium, "3000");
    // Panel origin (240 + 300*2.25, round(2*2.25)) plus the text area (14*2.25, round(1*2.25)).
    assert_eq!((node.left, node.top), (Val::Px(947.0), Val::Px(7.0)));
    assert_eq!(resource_label(&mut app, 0).unwrap().1, "");
    assert_eq!(resource_label(&mut app, 1).unwrap().1, "0/0");
    set_personnel(&mut app, Some(Fixed::from_int(4000)), [Fixed::from_int(7), Fixed::from_int(100)]);
    assert_eq!(resource_label(&mut app, 0).unwrap().1, "4000");
    assert_eq!(resource_label(&mut app, 1).unwrap().1, "7/100");
    // Three backgrounds and three icons are drawn as pictures.
    let pictures = app.world_mut().query::<&crate::ui::gui_sprites::GuiImage>().iter(app.world()).count();
    assert_eq!(pictures, 6);
}

/// With `STA64_TEST_A1_INSTALL=<Armada I root>`: every race's real GUI cfg draws the three
/// readouts (crew 0, officers 1, dilithium 2) inside the window at 4:3 and 16:9.
#[test]
fn installed_armada1_guis_draw_three_resource_readouts_inside_the_window() {
    let Some(root) = std::env::var_os("STA64_TEST_A1_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), None).unwrap();
    for cfg_name in ["gui_fed.cfg", "gui_kli.cfg", "gui_rom.cfg", "gui_bor.cfg"] {
        let layout = crate::game::gui_layout(&vfs, cfg_name).unwrap_or_else(|| panic!("{cfg_name} missing"));
        assert!(!layout.panels.contains_key("resource"), "{cfg_name} now has a resource panel");
        for aspect in [4.0_f32 / 3.0, 16.0 / 9.0] {
            let config = HudConfig { gui: Some(layout.clone()), aspect, fit_scale: false, ..default() };
            let mut app = app_with_content_and_config(content(), config, UiProfile::Armada2);
            assert_eq!(resource_label_count(&mut app), 3, "{cfg_name} at {aspect}");
            let (w, h) = (1080.0 * aspect, 1080.0);
            for id in 0..3 {
                let (_, _, _, node) = resource_label(&mut app, id).unwrap_or_else(|| panic!("{cfg_name} id {id}"));
                let (Val::Px(x), Val::Px(y), Val::Px(nw), Val::Px(nh)) = (node.left, node.top, node.width, node.height)
                else {
                    panic!("{cfg_name} id {id}: not pixel placed");
                };
                // The node sits where the cfg puts this race's text area inside its panel.
                let stock = StockLayout::new(&layout, Vec2::new(w, h));
                let prefix = ["crew", "officer", "dilithium"][id];
                let area = stock.panel(prefix).unwrap_or_else(|| panic!("{cfg_name} {prefix} panel"));
                let want = stock.at(Vec2::new(area.x, area.y), &format!("{prefix}TextArea")).unwrap();
                assert_eq!((x, y, nw, nh), (want.x, want.y, want.w, want.h), "{cfg_name} {prefix} text area at {aspect}");
                assert!(nw >= 20.0 && nh >= 8.0, "{cfg_name} id {id}: {nw}x{nh}");
                assert!(x >= 0.0 && y >= 0.0 && x + nw <= w + 0.5 && y + nh <= h, "{cfg_name} id {id}: {x},{y} {nw}x{nh} in {w}x{h}");
            }
        }
    }
}

/// The player's HUD layout choice applies live, through the real path: the `Settings` value
/// is copied into `HudConfig::fit` by `fit_screen`, the stock HUD redraws in place with
/// different rectangles, and switching back restores the exact centred layout.
#[test]
fn hud_fit_switch_redraws_the_stock_hud_live() {
    use super::layout::HudFit;
    use crate::frontend::screens::Settings;
    let aspect = 2752.0_f32 / 1152.0;
    let mut app = app_with_content_and_config(content(), HudConfig { aspect, ..config() }, UiProfile::Armada2);
    app.insert_resource(Settings::default());
    app.update();
    let snapshot = |app: &App| {
        let state = app.world().resource::<StockState>();
        let l = state.layout.as_ref().unwrap();
        (l.fit, l.hud, l.panel("minimap"), l.palette().unwrap().origin, state.solid.clone())
    };
    let label = |app: &mut App| (0..3).find_map(|id| resource_label(app, id)).expect("a resource readout").3.left;
    let centred = snapshot(&app);
    assert_eq!(centred.0, HudFit::Centred);
    assert!(centred.1.x > 0.0 && centred.1.w < 1080.0 * aspect, "{:?}", centred.1);
    let before = label(&mut app);

    app.world_mut().resource_mut::<Settings>().hud_fit = HudFit::Stretched;
    app.update();
    assert_eq!(app.world().resource::<HudConfig>().fit, HudFit::Stretched);
    let stretched = snapshot(&app);
    assert_eq!((stretched.1.x, stretched.1.y, stretched.1.h), (0.0, 0.0, 1080.0));
    assert!((stretched.1.w - 1080.0 * aspect).abs() < 0.01, "{:?}", stretched.1);
    assert_ne!(stretched.2, centred.2);
    assert_ne!(stretched.3, centred.3);
    assert_ne!(label(&mut app), before);

    app.world_mut().resource_mut::<Settings>().hud_fit = HudFit::Centred;
    app.update();
    let back = snapshot(&app);
    assert_eq!(back, centred, "switching back restores the exact centred layout");
    assert_eq!(label(&mut app), before);
}

/// Stretched mode: the HUD is the whole window, so no side margin exists; the pointer-solid
/// rectangles cover the minimap and the command cells where they are drawn, the open view
/// above the HUD stays free, and a click on a command cell still queues its order.
#[test]
fn stretched_hud_keeps_pointer_solids_and_clicks_on_its_cells() {
    use super::layout::HudFit;
    let aspect = 21.0_f32 / 9.0;
    let mut app = app_with_content_and_config(content(), HudConfig { aspect, fit: HudFit::Stretched, ..config() }, UiProfile::Armada2);
    let state = app.world().resource::<StockState>();
    let layout = state.layout.as_ref().unwrap();
    assert_eq!(layout.fit, HudFit::Stretched);
    assert_eq!(layout.hud, R::new(0.0, 0.0, 1080.0 * aspect, 1080.0));
    let map = layout.panel("minimap").unwrap();
    assert_eq!(map.x, 0.0, "panels start at the window edge");
    let first = layout.palette().unwrap().cell_rect(0, 0);
    let centre = |r: R| Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
    assert!(state.solid.iter().any(|r| r.contains(centre(map))), "minimap is pointer-solid");
    assert!(state.solid.iter().any(|r| r.contains(centre(first))), "a command cell is pointer-solid");
    for p in [Vec2::new(1080.0 * aspect / 2.0, 100.0), Vec2::new(1080.0 * aspect - 3.0, 100.0)] {
        assert!(!state.solid.iter().any(|r| r.contains(p)), "{p:?} is the open view");
    }
    // The built button sits on that rectangle, and a click on it queues the order.
    let stop = cell_rect_of(&mut app, Act::Stop);
    let origin = app.world().resource::<StockState>().layout.as_ref().unwrap().palette().unwrap();
    assert!((0..origin.columns).any(|c| (0..origin.rows).any(|r| origin.cell_rect(c, r) == stop)), "{stop:?} is a palette cell");
    let stop = cell(&mut app, Act::Stop);
    click(app.world_mut(), stop);
    app.update();
    assert_eq!(app.world().resource::<CommandQueue>().0, [CommandKind::Order(Order::Stop { units: vec![EntityId(1)] })]);
}

/// The placed rectangle of a palette button (its absolute-positioned node).
fn cell_rect_of(app: &mut App, act: Act) -> R {
    let e = cell(app, act);
    let n = app.world().get::<Node>(e).unwrap().clone();
    let (Val::Px(x), Val::Px(y), Val::Px(w), Val::Px(h)) = (n.left, n.top, n.width, n.height) else { panic!("not pixel placed") };
    R::new(x, y, w, h)
}

#[test]
fn armada1_separate_panels_respect_no_flags_and_missing_text_areas() {
    let raw = A1_RESOURCES.replace("dilithiumTextArea = 14 1 60 13", "dilithiumTextArea = 0 0 0 0") + "noCrew = 1\n";
    let mut app = app_with_content_and_config(content(), single_file_config(&raw), UiProfile::Armada2);
    assert!(resource_label(&mut app, 0).is_none());
    assert!(resource_label(&mut app, 2).is_none());
    assert!(resource_label(&mut app, 1).is_some());
}

#[test]
fn single_resource_panel_wins_over_separate_panels() {
    // A2 / FO / Roots sheets author `resourcePanelArea`; stray A1 style keys must not add readouts.
    let raw = format!("{A1_RESOURCES}resourcePanelArea = 0 0 400 80\nresource_2 = 20 10 80 30\n");
    let mut mixed = app_with_content_and_config(content(), single_file_config(&raw), UiProfile::Armada2);
    assert_eq!(resource_label_count(&mut mixed), 1);
    assert_eq!(resource_label(&mut mixed, 2).unwrap().1, "3000");
    // The shipped A2 test sheet still draws its three authored readouts unchanged.
    let mut a2 = app();
    assert_eq!(resource_label_count(&mut a2), 3);
}

#[test]
fn personnel_flags_and_zero_rectangles_suppress_only_authored_labels() {
    for extra in ["noCrew = 1\nnoOfficers = 1\n", "resource_0 = 8 8 0 20\nresource_1 = 9 9 40 0\n"] {
        let mut app = app_with_content_and_config(content(), inherited_config(extra, "resource_0 = 4 5 30 10\n"), UiProfile::Armada2);
        set_personnel(&mut app, Some(Fixed::from_int(1200)), [Fixed::from_int(16), Fixed::from_int(600)]);
        assert!(resource_label(&mut app, 0).is_none());
        assert!(resource_label(&mut app, 1).is_none());
        assert_eq!(resource_label(&mut app, 2).unwrap().1, "3000");
        assert_eq!(resource_label(&mut app, 4).unwrap().1, "700");
    }
}

/// Install-gated: the mounted STA2 Classic race CFGs determine which personnel
/// readouts exist; a live PlayerView supplies their values without rebuilding them.
#[test]
fn installed_sta2_classic_personnel_respects_race_cfg() {
    let (Some(root), Ok(mode)) = (std::env::var_os("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else {
        eprintln!("SKIP: set STA64_TEST_INSTALL and STA64_TEST_MOD=STA2 Classic");
        return;
    };
    if mode != "STA2 Classic" { return; }
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(&mode)).unwrap();
    for (cfg_name, crew_visible) in [
        ("gui_fed.cfg", true), ("gui_kli.cfg", true), ("gui_rom.cfg", true),
        ("gui_bor.cfg", true), ("gui_species8472.cfg", false),
    ] {
        let cfg = crate::game::gui_layout(&vfs, cfg_name).unwrap_or_else(|| panic!("{cfg_name} missing"));
        let layout = StockLayout::new(&cfg, Vec2::new(1920.0, 1080.0));
        assert_eq!(layout.number("noCrew").is_some_and(|n| n != 0.0), !crew_visible, "{cfg_name}");
        assert_eq!(layout.number("noOfficers").is_some_and(|n| n != 0.0), false, "{cfg_name}");
        let mut app = app_with_content_and_config(content(), HudConfig { gui: Some(cfg), ..default() }, UiProfile::Armada2);
        set_personnel(&mut app, Some(Fixed::from_int(1200)), [Fixed::from_int(16), Fixed::from_int(600)]);
        assert_eq!(resource_label(&mut app, 0).map(|r| r.1), crew_visible.then(|| "1200".to_owned()), "{cfg_name}");
        assert_eq!(resource_label(&mut app, 1).map(|r| r.1), Some("16/600".to_owned()), "{cfg_name}");
    }
}

#[test]
fn stock_resources_and_officers_format_large_fixed_values_exactly() {
    let cfg = inherited_config("resource_0 = 4 5 30 10\nresource_3 = 30 10 80 30\nresource_5 = 320 10 80 30\n", "");
    let mut app = app_with_content_and_config(content(), cfg, UiProfile::Armada2);
    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut view = (*feed.view).clone();
    view.players[0].crew = Some(Fixed::from_int(16_777_217));
    view.players[0].officers = [Fixed::from_int(16_777_217), Fixed::from_int(16_777_218)];
    view.players[0].resources = [Fixed::from_int(16_777_217); 4].to_vec();
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    for id in [0, 2, 3, 4, 5] {
        assert_eq!(resource_label(&mut app, id).unwrap().1, "16777217");
    }
    assert_eq!(resource_label(&mut app, 1).unwrap().1, "16777217/16777218");
}

#[test]
fn inherited_a2_info_colour_tints_single_and_builder_text() {
    let cfg = inherited_config("", "infoTextColor = 1 0 1 1\nshipNameColor = 0 1 0 1\nresource_2_color = 1 0 0 1\n");
    for build_count in [0, 1] {
        let mut app = app_with_content_and_config(content_with_build_count(build_count), cfg.clone(), UiProfile::Armada2);
        if build_count == 1 {
            let mut feed = app.world().resource::<HudFeed>().clone();
            let yard = feed.content.as_ref().unwrap().key("fx_yard").unwrap();
            Arc::make_mut(&mut feed.view).units[0].def = yard;
            *app.world_mut().resource_mut::<HudFeed>() = feed;
            app.update();
        }
        // The info bar follows the pointer: hover the selected ship to read its values there.
        app.world_mut().resource_mut::<crate::input::hover::HoveredUnit>().0 = Some(EntityId(1));
        app.update();
        let (name_key, class_key) = if build_count == 0 {
            ("infoSingleNameTextArea", "infoSingleClassTextArea")
        } else {
            ("infoBuildName", "infoBuildClass")
        };
        assert_eq!(stock_colour_at(&mut app, name_key, false), Color::srgba(1.0, 0.0, 1.0, 1.0));
        assert_eq!(stock_colour_at(&mut app, class_key, false), Color::srgba(1.0, 0.0, 1.0, 1.0));
        assert_eq!(stock_colour_at(&mut app, "shipName", true), Color::srgba(0.0, 1.0, 0.0, 1.0));
        assert_eq!(stock_colour_at(&mut app, "hull", true), Color::srgb(0.0, 1.0, 0.0));
        assert_eq!(resource_colour(&mut app, 2), Color::srgba(1.0, 0.0, 0.0, 1.0));
    }
}

#[test]
fn fo_separate_colours_and_legacy_overrides_take_precedence() {
    let selected = |cfg, yard| {
        let mut app = app_with_content_and_config(content(), cfg, UiProfile::FleetOps);
        if yard {
            let mut feed = app.world().resource::<HudFeed>().clone();
            let def = feed.content.as_ref().unwrap().key("fx_yard").unwrap();
            Arc::make_mut(&mut feed.view).units[0].def = def;
            *app.world_mut().resource_mut::<HudFeed>() = feed;
            app.update();
        }
        app
    };
    let cfg = inherited_config(
        "infoSingleNameTextColor = 0 1 1 1\ninfoSingleClassTextColor = 1 1 0 1\n",
        "nameTextColor = 1 0 0 1\nclassTextColor = 0 0 1 1\ninfoTextColor = 1 0 1 1\n",
    );
    for yard in [false, true] {
        let mut app = selected(cfg.clone(), yard);
        let (name_key, class_key) = if !yard {
            ("infoSingleNameTextArea", "infoSingleClassTextArea")
        } else {
            ("infoBuildName", "infoBuildClass")
        };
        assert_eq!(stock_colour_at(&mut app, name_key, false), Color::srgba(0.0, 1.0, 1.0, 1.0));
        assert_eq!(stock_colour_at(&mut app, class_key, false), Color::srgba(1.0, 1.0, 0.0, 1.0));
    }

    let cfg = inherited_config("", "nameTextColor = 1 0 0 1\nclassTextColor = 0 0 1 1\ninfoTextColor = 1 0 1 1\n");
    for yard in [false, true] {
        let mut app = selected(cfg.clone(), yard);
        let (name_key, class_key) = if !yard {
            ("infoSingleNameTextArea", "infoSingleClassTextArea")
        } else {
            ("infoBuildName", "infoBuildClass")
        };
        assert_eq!(stock_colour_at(&mut app, name_key, false), Color::srgba(1.0, 0.0, 0.0, 1.0));
        assert_eq!(stock_colour_at(&mut app, class_key, false), Color::srgba(0.0, 0.0, 1.0, 1.0));
    }
}

#[test]
fn missing_authored_colours_keep_theme_defaults() {
    let mut app = app_with_content_and_config(content(), inherited_config("", ""), UiProfile::Armada2);
    let mut feed = app.world().resource::<HudFeed>().clone();
    let yard = feed.content.as_ref().unwrap().key("fx_yard").unwrap();
    Arc::make_mut(&mut feed.view).units[0].def = yard;
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    let theme = app.world().resource::<Theme>().clone();
    assert_eq!(stock_colour_at(&mut app, "infoBuildName", false), theme.text);
    assert_eq!(stock_colour_at(&mut app, "infoBuildClass", false), theme.text_dim);
    assert_eq!(stock_colour_at(&mut app, "shipName", true), theme.text);
    assert_eq!(resource_colour(&mut app, 2), Color::srgba(0.6, 0.9, 1.0, 1.0));
}

#[test]
fn stock_resources_render_all_six_native_default_colours() {
    let cfg = inherited_config(
        "",
        "resource_0 = 4 5 30 10\nresource_3 = 30 10 80 30\nresource_5 = 320 10 80 30\n",
    );
    let mut app = app_with_content_and_config(content(), cfg, UiProfile::Armada2);
    for (id, expected) in [
        (0, Color::srgba(0.0, 1.0, 0.0, 1.0)),
        (1, Color::srgba(1.0, 1.0, 1.0, 1.0)),
        (2, Color::srgba(0.6, 0.9, 1.0, 1.0)),
        (3, Color::srgba(1.0, 1.0, 0.0, 1.0)),
        (4, Color::srgba(0.8, 0.8, 0.8, 1.0)),
        (5, Color::srgba(1.0, 0.0, 0.0, 1.0)),
    ] {
        assert_eq!(resource_label(&mut app, id).unwrap().2, expected, "resource {id}");
    }
}

#[test]
fn authored_resource_colours_override_palette_and_survive_value_updates() {
    let cfg = inherited_config(
        "resource_0_color = 1 1 1 1\n",
        "resource_0 = 4 5 30 10\nresource_3 = 30 10 80 30\nresource_5 = 320 10 80 30\nReSoUrCe_2_CoLoR = 0.25 0.5 0.75 0.4\nresource_5_color = 0 0 0 0.25\n",
    );
    let mut app = app_with_content_and_config(content(), cfg, UiProfile::Armada2);
    let expected = [
        Color::srgba(1.0, 1.0, 1.0, 1.0),
        Color::srgba(1.0, 1.0, 1.0, 1.0),
        Color::srgba(0.25, 0.5, 0.75, 0.4),
        Color::srgba(1.0, 1.0, 0.0, 1.0),
        Color::srgba(0.8, 0.8, 0.8, 1.0),
        Color::srgba(0.0, 0.0, 0.0, 0.25),
    ];
    let before: Vec<_> = (0..6)
        .map(|id| {
            let (entity, _, colour, _) = resource_label(&mut app, id).unwrap();
            assert_eq!(colour, expected[id], "resource {id}");
            entity
        })
        .collect();

    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut view = (*feed.view).clone();
    view.players[0].crew = Some(Fixed::from_int(1200));
    view.players[0].officers = [Fixed::from_int(16), Fixed::from_int(600)];
    view.players[0].resources = [1234, 6, 42, 9].map(Fixed::from_int).to_vec();
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();

    for id in 0..6 {
        let (entity, text, colour, _) = resource_label(&mut app, id).unwrap();
        assert_eq!(entity, before[id], "resource {id} entity");
        assert_eq!(colour, expected[id], "resource {id} colour");
        assert_eq!(text, ["1200", "16/600", "1234", "6", "42", "9"][id]);
    }
}

fn info_bar_label(app: &mut App, field: infobar::InfoBarField) -> Option<(Entity, String, Color, Visibility, Node)> {
    app.world_mut().query::<(Entity, &infobar::InfoBarField, &SpriteText, &Visibility, &ChildOf)>()
        .iter(app.world()).find_map(|(entity, kind, text, visibility, parent)| {
            (*kind == field).then(|| (entity, text.text.clone(), text.color, *visibility,
                app.world().get::<Node>(parent.parent()).unwrap().clone()))
        })
}

fn info_bar_content(builder: bool) -> Arc<ContentDb> {
    let base = content_with_build_count(usize::from(builder));
    let mut db = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for weapon in base.weapons() { db = db.weapon(weapon.clone()); }
    for unit in base.units() {
        let mut unit = unit.clone();
        if unit.odf == "fx_cruiser" {
            unit.raw = Arc::new(sta_odf::resolve("unit", &mut |_| Some(
                "unitName = \"Construction Ship\"\npossibleCraftNames = \"USS Test\"".into()
            )).unwrap());
        }
        db = db.unit(unit);
    }
    Arc::new(db.build().unwrap())
}

const INFO_BAR_CFG: &str = "\
shields = 110 10 50 20
crew_amt = 200 10 50 20
energy_amt = 300 10 60 20
icon_hull = 0 10 24 24
icon_shields = 80 10 24 24
icon_crew_amt = 170 10 24 24
icon_energy_amt = 270 10 24 24
infoBarOffset_0 = 11 7 0 0
infoBarOffset_1 = 22 -3 0 0
infoBarOffset_2 = 33 4 0 0
";

fn populated_info_bar(builder: bool, extra: &str) -> App {
    let cfg = inherited_config(extra, INFO_BAR_CFG);
    let mut app = app_with_content_and_config(info_bar_content(builder), cfg, UiProfile::Armada2);
    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut view = (*feed.view).clone();
    view.units[0].hull = [Fixed::from_int(100); 2];
    view.units[0].shields = [[Fixed::from_int(100); 2]; 4];
    view.units[0].crew = Some([Fixed::from_int(150); 2]);
    view.units[0].energy = Some([Fixed::from_int(600); 2]);
    let mut other = view.units[0].clone();
    other.id = EntityId(2);
    other.hull[0] = Fixed::from_int(50);
    other.shields = [[Fixed::from_int(25), Fixed::from_int(100)]; 4];
    other.crew = Some([Fixed::from_int(73), Fixed::from_int(150)]);
    other.energy = Some([Fixed::from_int(17), Fixed::from_int(600)]);
    if builder {
        view.units[0].def = feed.content.as_ref().unwrap().key("fx_yard").unwrap();
    }
    view.units.push(other);
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    app
}

#[test]
fn native_hover_percentages_and_crew_change_colour_with_live_pools() {
    use infobar::InfoBarField::{Hull, Shields, Crew};
    let mut app = populated_info_bar(false, "systemIconHealthyColor=0 1 0\nsystemIconLowColor=1 1 0\nsystemIconCriticalColor=1 0 0\n");
    // The info bar follows the pointer: hover the selected ship to read its values there.
    app.world_mut().resource_mut::<crate::input::hover::HoveredUnit>().0 = Some(EntityId(1));
    app.update();
    assert_eq!(info_bar_label(&mut app, Hull).unwrap().1, "100%");
    let crew_node = info_bar_label(&mut app, Crew).unwrap().0;
    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut view = (*feed.view).clone();
    view.units[0].hull[0] = Fixed::from_int(50);
    view.units[0].shields = [[Fixed::from_int(25), Fixed::from_int(100)]; 4];
    view.units[0].crew = Some([Fixed::from_int(37), Fixed::from_int(150)]);
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    for (field, text, colour) in [
        (Hull, "50%", Color::srgb(1.0, 1.0, 0.0)),
        (Shields, "25%", Color::srgb(1.0, 0.0, 0.0)),
        (Crew, "37", Color::srgb(1.0, 0.0, 0.0)),
    ] {
        let label = info_bar_label(&mut app, field).unwrap();
        assert_eq!(label.1, text);
        assert_eq!(label.2, colour);
    }
    assert_eq!(info_bar_label(&mut app, Crew).unwrap().0, crew_node);
}

#[test]
fn resolved_cfg_supplies_optional_selection_gauge_dimensions() {
    let mut app = populated_info_bar(false, "selectionGaugeHeight=3\nselectionGaugeGap=5\n");
    assert_eq!(*app.world().resource::<crate::render::overlays::GaugeStyle>(),
        crate::render::overlays::GaugeStyle { height: 3.0, gap: 5.0 });
    *app.world_mut().resource_mut::<UiProfile>() = UiProfile::Lcars;
    app.update();
    assert_eq!(*app.world().resource::<crate::render::overlays::GaugeStyle>(),
        crate::render::overlays::GaugeStyle::default());
}

#[test]
fn info_bar_follows_only_the_pointer_and_updates_real_pools_in_place() {
    use infobar::InfoBarField::{Name, Hull, Shields, Crew, Energy};
    let mut app = populated_info_bar(true, "hull_color = 0.2 0.4 0.6 0.8\ncrew_amt_color = 0 1 1 1\nshipNameColor = 1 0 1 1\n");
    let fields = [Name, Hull, Shields, Crew, Energy];
    let ids: Vec<_> = fields.iter().map(|&field| info_bar_label(&mut app, field).unwrap().0).collect();
    assert_eq!(info_bar_label(&mut app, Name).unwrap().1, "", "the selection alone shows nothing: the bar follows the pointer");
    app.world_mut().resource_mut::<crate::input::hover::HoveredUnit>().0 = Some(EntityId(2));
    app.update();
    for (field, expected) in [(Name,"Construction Ship"), (Hull,"50%"), (Shields,"25%"), (Crew,"73"), (Energy,"3%")] {
        let label = info_bar_label(&mut app, field).unwrap();
        assert_eq!(label.1, expected);
        assert_eq!(label.3, Visibility::Inherited, "bar must follow HUD root visibility");
    }
    assert_eq!(info_bar_label(&mut app, Hull).unwrap().2, Color::srgba(0.2,0.4,0.6,0.8));
    assert_eq!(info_bar_label(&mut app, Crew).unwrap().2, Color::srgb(0.0,1.0,1.0));
    assert_eq!(info_bar_label(&mut app, Name).unwrap().2, Color::srgb(1.0,0.0,1.0));
    assert_eq!(app.world().resource::<HudFeed>().selected, [EntityId(1)]);
    assert_eq!(app.world().resource::<Selection>().0, [EntityId(1)]);
    for (old, field) in ids.iter().zip(fields) {
        assert_eq!(*old, info_bar_label(&mut app, field).unwrap().0);
    }
    let crew = info_bar_label(&mut app, Crew).unwrap().4;
    assert_eq!((crew.left,crew.top,crew.width,crew.height),
        (Val::Px(810.0),Val::Px(823.0),Val::Px(45.0),Val::Px(18.0)));
    let icons: Vec<_> = app.world_mut().query::<(&infobar::InfoBarIcon, &crate::ui::gui_sprites::GuiImage, &Visibility)>()
        .iter(app.world()).map(|(kind,image,visibility)| (kind.0,image.color,*visibility,image.names.clone())).collect();
    assert_eq!(icons.len(),4);
    let icon = icons.iter().find(|i| i.0 == Crew).unwrap();
    assert_eq!(icon.1, Color::srgb(0.0,1.0,1.0));
    assert_eq!(icon.2, Visibility::Inherited);
    assert_eq!(icon.3, ["crew_amt"]);
    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut view = (*feed.view).clone();
    view.units[1].crew = Some([Fixed::from_int(41), Fixed::from_int(150)]);
    view.units[1].energy = None;
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    assert_eq!(info_bar_label(&mut app, Crew).unwrap().1, "41");
    assert_eq!(info_bar_label(&mut app, Energy).unwrap().3, Visibility::Hidden);
    assert_eq!(info_bar_label(&mut app, Crew).unwrap().0, ids[3]);
    app.world_mut().resource_mut::<crate::input::hover::HoveredUnit>().0 = None;
    app.update();
    assert_eq!(info_bar_label(&mut app, Name).unwrap().3, Visibility::Hidden, "off the unit the bar empties, the selection notwithstanding");
    assert_eq!(info_bar_label(&mut app, Crew).unwrap().1, "");
}

#[test]
fn no_selection_hover_uses_low_offset_and_missing_or_hidden_targets_clear_stats() {
    use infobar::InfoBarField::{Name,Crew};
    let mut app = populated_info_bar(false, "");
    let mut feed = app.world().resource::<HudFeed>().clone();
    feed.selected.clear();
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.world_mut().resource_mut::<crate::input::hover::HoveredUnit>().0 = Some(EntityId(2));
    app.update();
    assert_eq!(info_bar_label(&mut app, Name).unwrap().1,"Construction Ship");
    let node = info_bar_label(&mut app, Crew).unwrap().4;
    assert_eq!((node.left,node.top), (Val::Px(790.0),Val::Px(825.0)));
    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut view = (*feed.view).clone();
    view.visibility = vec![(feed.player,vec![EntityId(1)])];
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    assert_eq!(info_bar_label(&mut app, Name).unwrap().3, Visibility::Hidden);
    assert_eq!(info_bar_label(&mut app, Crew).unwrap().1, "");
    let mut feed = app.world().resource::<HudFeed>().clone();
    feed.selected = vec![EntityId(99)]; // No indexing panic or stale source.
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    assert_eq!(info_bar_label(&mut app, Crew).unwrap().3, Visibility::Hidden);
}

fn tiered_info_config() -> HudConfig {
    let mut raw = CFG.replace("infoPanelArea_1 = 400 900 500 300", "infoPanelArea_1 = 400 1000 500 200");
    raw.push_str("\
infoPanelArea_0 = 400 1100 500 100
infoLowBackgroundPanel = \"low_frame\"
infoLowBackgroundPanelArea = 3 4 490 96
infoLowBackgroundPanelSize = 1
infoLowBackgroundPanel_0 = 0 0 490 96
infoMiddleBackgroundPanel = \"middle_frame\"
infoMiddleBackgroundPanelArea = 5 6 480 194
infoMiddleBackgroundPanelSize = 1
infoMiddleBackgroundPanel_0 = 0 0 480 194
infoBackgroundPanel = \"high_frame\"
infoBackgroundPanelArea = 7 8 475 292
infoBackgroundPanelSize = 1
infoBackgroundPanel_0 = 0 0 475 292
infoBlackArea_0 = 12 13 440 70
infoBlackArea_1 = 12 13 440 160
infoBlackArea_2 = 12 13 440 260
infoBarOffset_0 = 10 5 0 0
infoBarOffset_1 = 20 6 0 0
infoBarOffset_2 = 30 7 0 0
shipName = 0 0 180 20
infoMultiShipIconSm_0 = 350 10 30 30
");
    for index in 0..16 {
        raw.push_str(&format!(
            "infoMultiShipIcon_{index} = {} {} 40 40\n",
            10 + (index % 8) * 55,
            70 + (index / 8) * 65,
        ));
    }
    single_file_config(&raw)
}

#[test]
fn armada2_info_frame_bar_and_hit_area_follow_selection_tier() {
    use crate::ui::gui_sprites::GuiImage;
    use crate::ui::hud::panels::GroupTile;
    use infobar::InfoBarField::Name;

    let mut app = app_with_content_and_config(content_with_build_count(1), tiered_info_config(), UiProfile::Armada2);
    let keys = {
        let feed = app.world().resource::<HudFeed>();
        let db = feed.content.as_ref().unwrap();
        ["fx_frigate", "fx_cruiser", "fx_yard", "fx_station"]
            .map(|name| db.key(name).unwrap())
    };
    {
        let mut feed = app.world().resource::<HudFeed>().clone();
        let view = Arc::make_mut(&mut feed.view);
        for id in 2..=17 {
            let mut unit = view.units[0].clone();
            unit.id = EntityId(id);
            view.units.push(unit);
        }
        view.units[0].def = keys[0];
        feed.selected.clear();
        *app.world_mut().resource_mut::<HudFeed>() = feed;
    }
    app.world_mut().resource_mut::<crate::input::hover::HoveredUnit>().0 = Some(EntityId(1));
    app.update();

    let assert_tier = |app: &mut App, tier: usize, frame_name: &str, tiles: usize| {
        let (frame_area, black_area, bar, hit) = {
            let state = app.world().resource::<StockState>();
            let layout = state.layout.as_ref().unwrap();
            let info = model::info(app.world().resource::<HudFeed>());
            assert_eq!(layout.info_variant(&info), tier);
            let area = layout.info_area(tier).unwrap();
            let frame_key = match tier {
                0 => "infoLowBackgroundPanelArea",
                1 => "infoMiddleBackgroundPanelArea",
                _ => "infoBackgroundPanelArea",
            };
            let frame_area = layout.info_at(tier, frame_key).unwrap();
            let black_area = layout.info_at(tier, &format!("infoBlackArea_{tier}")).unwrap();
            let bar = layout.info_bar_at(tier, "shipName").unwrap();
            let hit = state.solid.iter().any(|rect| rect.contains(Vec2::new(area.x + 20.0, area.y + 20.0)));
            (frame_area, black_area, bar, hit)
        };
        assert!(hit, "the drawn {frame_name} owns its pointer area");
        let parent = one::<StockInfo>(app);
        let frame = app.world_mut()
            .query::<(&GuiImage, &Node, &ChildOf)>()
            .iter(app.world())
            .find(|(image, _, child)| child.parent() == parent && image.names.iter().any(|name| name == frame_name))
            .map(|(_, node, _)| node.clone())
            .unwrap_or_else(|| panic!("missing {frame_name}"));
        assert_eq!((frame.left, frame.top, frame.width, frame.height),
            (Val::Px(frame_area.x), Val::Px(frame_area.y), Val::Px(frame_area.w), Val::Px(frame_area.h)));
        let black = app.world_mut()
            .query::<(&BackgroundColor, &Node, &ChildOf)>()
            .iter(app.world())
            .find(|(color, _, child)| child.parent() == parent && color.0 == Color::BLACK.with_alpha(0.5))
            .map(|(_, node, _)| node.clone())
            .unwrap_or_else(|| panic!("missing {frame_name} backing"));
        assert_eq!((black.left, black.top, black.width, black.height),
            (Val::Px(black_area.x), Val::Px(black_area.y), Val::Px(black_area.w), Val::Px(black_area.h)));
        let name = info_bar_label(app, Name).unwrap().4;
        assert_eq!((name.left, name.top), (Val::Px(bar.x), Val::Px(bar.y)));
        let rendered = app.world_mut().query::<&GroupTile>().iter(app.world()).count();
        assert_eq!(rendered, tiles, "{frame_name} group tiles");
    };

    assert_tier(&mut app, 0, "low_frame", 0);
    let high_only = {
        let layout = app.world().resource::<StockState>().layout.as_ref().unwrap();
        let area = layout.info_area(2).unwrap();
        Vec2::new(area.x + 20.0, area.y + 20.0)
    };
    assert!(!app.world().resource::<StockState>().solid.iter().any(|r| r.contains(high_only)),
        "the collapsed HUD leaves the space above the low frame open to the map");

    let select = |app: &mut App, ids: Vec<EntityId>, def: Option<sta_sim::ContentKey>| {
        let mut feed = app.world().resource::<HudFeed>().clone();
        if let Some(def) = def {
            Arc::make_mut(&mut feed.view).units[0].def = def;
        }
        feed.selected = ids;
        *app.world_mut().resource_mut::<HudFeed>() = feed;
        app.update();
    };
    select(&mut app, vec![EntityId(1)], None);
    assert_tier(&mut app, 1, "middle_frame", 0);
    // A craft with a build list remains in the middle tier in Armada II.
    select(&mut app, vec![EntityId(1)], Some(keys[1]));
    assert_tier(&mut app, 1, "middle_frame", 0);
    for (count, tier, frame) in [(8, 1, "middle_frame"), (9, 2, "high_frame"), (17, 2, "high_frame")] {
        select(&mut app, (1..=count).map(EntityId).collect(), None);
        assert_tier(&mut app, tier, frame, count.min(16) as usize);
        let expected = {
            let layout = app.world().resource::<StockState>().layout.as_ref().unwrap();
            layout.info_at(tier, "infoMultiShipIcon_0").unwrap()
        };
        let first = app.world_mut()
            .query::<(&GroupTile, &Node)>()
            .iter(app.world())
            .find(|(tile, _)| tile.0 == EntityId(1))
            .map(|(_, node)| node.clone())
            .unwrap();
        assert_eq!((first.left, first.top, first.width, first.height),
            (Val::Px(expected.x), Val::Px(expected.y), Val::Px(expected.w), Val::Px(expected.h)),
            "{count} selected units use the normal group geometry");
    }
    // An idle yard and a starbase keep the tall builder layout.
    assert!(app.world().resource::<HudFeed>().production.is_empty());
    for def in [keys[2], keys[3]] {
        select(&mut app, vec![EntityId(1)], Some(def));
        assert_tier(&mut app, 2, "high_frame", 0);
    }
    select(&mut app, Vec::new(), None);
    assert_tier(&mut app, 0, "low_frame", 0);
    assert!(!app.world().resource::<StockState>().solid.iter().any(|r| r.contains(high_only)));
}

#[test]
fn middle_info_bar_and_zero_area_fields_follow_the_cfg() {
    use infobar::InfoBarField::{Crew,Energy};
    let mut app = populated_info_bar(false, "icon_energy_amt = 0 0 0 0\ncrew_amt = 0 0 0 0\n");
    assert!(info_bar_label(&mut app, Crew).is_none());
    let energy = info_bar_label(&mut app, Energy).unwrap().4;
    assert_eq!((energy.left,energy.top), (Val::Px(890.0),Val::Px(816.0)));
    assert!(app.world_mut().query::<&infobar::InfoBarIcon>().iter(app.world()).all(|i| i.0 != Energy));
}

fn one<C: Component>(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<C>>()
        .single(app.world())
        .unwrap()
}

fn cell(app: &mut App, act: Act) -> Entity {
    app.world_mut()
        .query::<(Entity, &GridCell)>()
        .iter(app.world())
        .find(|(_, c)| c.0 == act)
        .unwrap_or_else(|| panic!("grid is missing {act:?}"))
        .0
}

#[test]
fn profile_and_selection_changes_preserve_root_camera_and_state() {
    let mut app = app();
    let root = one::<HudRoot>(&mut app);
    assert_eq!(
        app.world_mut()
            .query_filtered::<(), With<StockBody>>()
            .iter(app.world())
            .count(),
        1
    );
    let target = app.world_mut().spawn_empty().id();
    app.world_mut()
        .entity_mut(root)
        .insert((UiTargetCamera(target), Visibility::Hidden));
    app.world_mut().resource_mut::<HudFeed>().selected.clear();
    app.update();
    assert_eq!(one::<HudRoot>(&mut app), root);
    app.world_mut().resource_mut::<HudFeed>().selected = vec![EntityId(1)];
    app.update();
    assert_eq!(one::<HudRoot>(&mut app), root);
    let station = app
        .world()
        .resource::<HudFeed>()
        .content
        .as_ref()
        .unwrap()
        .key("fx_station")
        .unwrap();
    app.world_mut().resource_mut::<HudState>().page = Page::Build;
    app.world_mut().resource_mut::<HudState>().placing = Some(Armed {
        builder: EntityId(1),
        what: station,
        odf: "fx_station".into(),
    });
    app.update();
    assert_eq!(app.world().resource::<HudState>().page, Page::Build);
    app.world_mut().insert_resource(UiProfile::Lcars);
    app.update();
    assert_eq!(one::<HudRoot>(&mut app), root);
    assert_eq!(app.world().get::<UiTargetCamera>(root).unwrap().0, target);
    assert_eq!(app.world().get::<Visibility>(root), Some(&Visibility::Hidden));
    assert_eq!(app.world().resource::<HudState>().page, Page::Build);
    assert_eq!(
        app.world()
            .resource::<HudState>()
            .placing
            .as_ref()
            .map(|a| a.odf.as_str()),
        Some("fx_station")
    );
    assert_eq!(
        app.world_mut()
            .query_filtered::<(), With<StockBody>>()
            .iter(app.world())
            .count(),
        0
    );
    app.world_mut().insert_resource(UiProfile::FleetOps);
    app.world_mut().resource_mut::<HudConfig>().aspect = 4.0 / 3.0;
    app.update();
    assert_eq!(one::<HudRoot>(&mut app), root);
    assert_eq!(app.world().get::<UiTargetCamera>(root).unwrap().0, target);
    assert_eq!(app.world().get::<Visibility>(root), Some(&Visibility::Hidden));
    assert_eq!(app.world().resource::<HudState>().page, Page::Build);
    assert!(app.world().resource::<HudState>().placing.is_some());
    assert_eq!(
        app.world_mut()
            .query_filtered::<(), With<StockBody>>()
            .iter(app.world())
            .count(),
        1
    );
}

#[test]
fn selected_moon_draws_a_live_resource_bar_in_the_stock_panel() {
    use bevy::ecs::system::RunSystemOnce;
    use sta_sim::economy::view::NodeView;

    let mut app = app();
    let id = EntityId(1);
    {
        let mut feed = app.world_mut().resource_mut::<HudFeed>();
        let moon = feed.content.as_ref().unwrap().key("fx_moon").unwrap();
        let unit = &mut Arc::make_mut(&mut feed.view).units[0];
        unit.def = moon;
        unit.owner = sta_sim::world::NEUTRAL;
    }
    app.update();
    app.world_mut().resource_mut::<crate::ui::hud::status::StatusFeed>().nodes = vec![NodeView {
        entity: id,
        resource: 0,
        amount: Fixed::from_int(100),
        start: Fixed::from_int(400),
        infinite: false,
    }];
    app.world_mut().run_system_once(info::update).unwrap();
    let world = app.world_mut();
    let fills: Vec<_> = world.query::<(&info::StockBar, &Node)>().iter(world).collect();
    assert!(!fills.iter().any(|(bar, _)| matches!(bar, info::StockBar::Hull(_))));
    let [(width, expected)] = fills.iter().filter_map(|(bar, node)| match bar {
        info::StockBar::Node(full_width) => Some((node.width, Val::Px(*full_width * 0.25))),
        _ => None,
    }).collect::<Vec<_>>().try_into().expect("one resource bar");
    assert_eq!(width, expected);
}

#[test]
fn numeric_values_patch_existing_stock_entities() {
    let mut app = app();
    // The info bar follows the pointer: hover the selected ship to read its values there.
    app.world_mut().resource_mut::<crate::input::hover::HoveredUnit>().0 = Some(EntityId(1));
    app.update();
    let resources = |app: &mut App| -> Vec<(Entity, usize, String)> {
        app.world_mut()
            .query::<(Entity, &info::StockText, &SpriteText)>()
            .iter(app.world())
            .filter_map(|(e, kind, text)| match kind {
                info::StockText::Resource(id) => Some((e, *id, text.text.clone())),
                info::StockText::Population(_) => None,
            })
            .collect()
    };
    let old = resources(&mut app);
    assert!(old.iter().any(|(_, id, text)| *id == 2 && text == "3000"));
    assert!(old.iter().any(|(_, id, text)| *id == 4 && text == "700"));
    let hull = app
        .world_mut()
        .query::<(Entity, &infobar::InfoBarField, &SpriteText)>()
        .iter(app.world())
        .find(|(_, kind, _)| matches!(kind, infobar::InfoBarField::Hull))
        .map(|(e, _, text)| (e, text.text.clone()))
        .unwrap();
    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut view = (*feed.view).clone();
    view.players[0].resources[0] = Fixed::from_int(1234);
    view.units[0].hull[0] = Fixed::from_int(60);
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    let new = resources(&mut app);
    assert_eq!(
        old.iter().map(|(e, id, _)| (*e, *id)).collect::<Vec<_>>(),
        new.iter().map(|(e, id, _)| (*e, *id)).collect::<Vec<_>>()
    );
    assert!(new.iter().any(|(_, id, text)| *id == 2 && text == "1234"));
    let updated = app.world().get::<SpriteText>(hull.0).unwrap();
    assert_ne!(updated.text, hull.1);
    assert_eq!(updated.text, "60%");
}

#[test]
fn officers_appear_when_capacity_arrives_after_first_frame() {
    let mut app = app();
    let root = one::<HudRoot>(&mut app);
    let resource_two = app
        .world_mut()
        .query::<(Entity, &info::StockText)>()
        .iter(app.world())
        .find(|(_, kind)| matches!(kind, info::StockText::Resource(2)))
        .unwrap()
        .0;
    let mut feed = app.world().resource::<HudFeed>().clone();
    let mut view = (*feed.view).clone();
    view.players[0].officers = [Fixed::from_int(2), Fixed::from_int(6)];
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    let officers = app
        .world_mut()
        .query::<(&info::StockText, &SpriteText)>()
        .iter(app.world())
        .find_map(|(kind, text)| matches!(kind, info::StockText::Resource(1)).then_some(text.text.clone()));
    assert_eq!(officers.as_deref(), Some("2/6"));
    assert_eq!(one::<HudRoot>(&mut app), root);
    assert!(app.world().get::<info::StockText>(resource_two).is_some());
}

#[test]
fn stock_build_palette_pages_reach_every_build_option() {
    use std::collections::BTreeSet;

    let mut app = app_with_content(content_with_build_count(24));
    app.world_mut().resource_mut::<HudState>().page = Page::Build;
    app.update();
    let mut reached = BTreeSet::new();
    for page in 0..5 {
        for cell in app.world_mut().query::<&GridCell>().iter(app.world()) {
            if let Act::Build { odf, .. } = &cell.0 {
                reached.insert(odf.clone());
            }
        }
        assert!(
            app.world().resource::<CommandQueue>().0.is_empty(),
            "navigation must not issue an order"
        );
        let next = app
            .world_mut()
            .query::<(Entity, &StockNav, &WidgetState)>()
            .iter(app.world())
            .find(|(_, nav, _)| nav.0 > 0)
            .map(|(e, _, state)| (e, *state))
            .unwrap();
        if next.1 == WidgetState::Disabled {
            break;
        }
        assert!(page < 4, "paging did not terminate");
        click(app.world_mut(), next.0);
        app.update();
        app.update();
    }
    let expected: BTreeSet<_> = (0..24).map(|i| format!("fx_build_{i:02}")).collect();
    assert_eq!(reached, expected);
    let back = cell(&mut app, Act::Back);
    click(app.world_mut(), back);
    app.update();
    assert_eq!(app.world().resource::<HudState>().page, Page::Top);
    assert!(app.world().resource::<CommandQueue>().0.is_empty());
}

#[test]
fn stock_hud_build_buttons_show_station_and_ship_costs_without_authored_verbose_text() {
    let mut app = app();
    let delays = crate::ui::stock_tooltip::TipDelays::default();
    for (builder, product, cost) in [("fx_cruiser", "fx_station", "1200"), ("fx_yard", "fx_cruiser", "600")] {
        let mut feed = app.world().resource::<HudFeed>().clone();
        let mut view = (*feed.view).clone();
        view.units[0].def = feed.content.as_ref().unwrap().key(builder).unwrap();
        feed.view = Arc::new(view);
        *app.world_mut().resource_mut::<HudFeed>() = feed;
        app.world_mut().resource_mut::<super::super::HudState>().page = super::super::Page::Build;
        app.update();
        let tip = app.world_mut().query::<(&GridCell, &crate::ui::notices::Tooltip)>().iter(app.world())
            .find_map(|(cell, tip)| matches!(&cell.0, Act::Build { odf, .. } if odf == product).then_some(tip.clone()))
            .unwrap_or_else(|| panic!("missing {product} build button"));
        for rest in [delays.short, delays.verbose] {
            let text = tip.stock_text_at(rest, &delays, &|key| key.into()).unwrap();
            assert!(text.contains(cost), "{builder} -> {product} at {rest}: {text:?}");
        }
    }
}

#[test]
fn stock_grid_clicks_still_queue_orders_and_arm_targets() {
    let mut app = app();
    let stop = cell(&mut app, Act::Stop);
    click(app.world_mut(), stop);
    app.update();
    assert_eq!(
        app.world().resource::<CommandQueue>().0,
        [CommandKind::Order(Order::Stop {
            units: vec![EntityId(1)]
        })]
    );
    let move_cell = cell(&mut app, Act::Target(TargetMode::Move));
    click(app.world_mut(), move_cell);
    app.update();
    assert_eq!(app.world().resource::<HudState>().targeting, Some(TargetMode::Move));
}

#[test]
fn metadata_groups_only_available_actions() {
    let db = content();
    let feed = feed(&db);
    let mut native = NativeCommands {
        groups: vec!["orders".into(), "unavailable".into()],
        ..default()
    };
    for (key, command, menu) in [
        ("move", "GO", "orders"),
        ("attack", "ATTACK", "orders"),
        ("harvest", "MINE", "unavailable"),
    ] {
        native.commands.insert(
            key.into(),
            NativeCommand {
                odf: format!("{key}.odf"),
                command: command.into(),
                button: key.into(),
                preferred: None,
                menu: Some(menu.into()),
                source: Vec::new(),
                param: None,
                tooltip: Some("GUI_MOVE_TOOLTIP".into()),
                verbose: Some("GUI_MOVE_VERBOSE".into()),
                normal: None,
                hovered: None,
                pressed: None,
                disabled: None,
                active: None,
            },
        );
    }
    let shown = buttons(&feed, Page::Top, &native);
    assert!(shown.iter().any(|b| b.act == Act::OpenNativeMenu(0)));
    assert!(!shown.iter().any(|b| b.act == Act::OpenNativeMenu(1)));
    assert!(!shown.iter().any(|b| b.act == Act::Target(TargetMode::Mine)));
    let grouped = buttons(&feed, Page::Native(0), &native);
    assert_eq!(
        grouped.iter().map(|b| &b.act).collect::<Vec<_>>(),
        [&Act::Target(TargetMode::Move), &Act::Target(TargetMode::Attack), &Act::Back],
    );
    let move_button = grouped.iter().find(|b| b.act == Act::Target(TargetMode::Move)).unwrap();
    assert_eq!(move_button.tip, "GUI_MOVE_TOOLTIP");
    assert_eq!(move_button.verbose.as_deref(), Some("GUI_MOVE_VERBOSE"));
    assert!(!grouped.iter().any(|b| b.act == Act::Target(TargetMode::Mine)));
}

#[test]
fn transport_shows_in_the_stock_palette_only_where_the_command_data_places_it() {
    let db = content();
    let mut feed = feed(&db);
    let mut view = (*feed.view).clone();
    view.units[0].crew = Some([Fixed::from_int(40), Fixed::from_int(50)]);
    feed.view = Arc::new(view);
    let transport = |native: &NativeCommands| buttons(&feed, Page::Top, native).into_iter().find(|b| b.act == Act::Target(TargetMode::Transport));
    // No command data (an install without the command, as Armada I's Armada II-style palette): absent, though
    // the model offers its fallback cell to STA64's own layout.
    assert!(model::grid(&feed, Page::Top).iter().any(|b| b.act == Act::Target(TargetMode::Transport)));
    assert!(transport(&NativeCommands::default()).is_none());
    let record = |preferred| NativeCommand {
        odf: "transport.odf".into(), command: "TRANSPORT".into(), button: "b_transport".into(), preferred, menu: None,
        source: Vec::new(), param: None,
        tooltip: None, verbose: None, normal: None, hovered: None, pressed: None, disabled: None, active: None,
    };
    let mut native = NativeCommands::default();
    native.commands.insert("transport".into(), record(None));
    assert!(transport(&native).is_none(), "a command with no place in the data");
    native.commands.insert("transport".into(), record(Some([2, 1])));
    let b = transport(&native).expect("placed by the data");
    assert_eq!(b.sprite_names.first().map(String::as_str), Some("b_transport"));
}

#[test]
fn native_formation_choices_keep_odf_identity_and_cannot_queue_orders() {
    let db = content();
    let feed = feed(&db);
    let fallback = buttons(&feed, Page::Formations, &NativeCommands::default());
    assert!(fallback.iter().any(|b| b.act == Act::Formation(sta_sim::movement::FORM_LINE)));

    let mut native = NativeCommands::default();
    for (odf, tip, preferred) in [("zeta", "ZETA_TIP", [1, 1]), ("alpha", "ALPHA_TIP", [0, 0])] {
        native.commands.insert(odf.into(), NativeCommand {
            odf: format!("{odf}.odf"),
            command: "SET_FORMATION".into(),
            button: "same_unrelated_button".into(),
            preferred: Some(preferred),
            menu: None,
            source: Vec::new(),
            param: None,
            tooltip: Some(tip.into()),
            verbose: Some(format!("{tip}_VERBOSE")),
            normal: None,
            hovered: None,
            pressed: None,
            disabled: Some(Color::srgb(0.2, 0.3, 0.4)),
            active: None,
        });
    }
    let shown = buttons(&feed, Page::Formations, &native);
    let choices: Vec<_> = shown.iter().filter(|b| matches!(b.act, Act::NativeFormation(_))).collect();
    assert_eq!(choices.len(), 2);
    assert_eq!(choices[0].act, Act::NativeFormation("alpha".into()));
    assert_eq!(choices[0].label, "ALPHA");
    assert_eq!(choices[0].tip, "ALPHA_TIP");
    assert_eq!(choices[0].verbose.as_deref(), Some("ALPHA_TIP_VERBOSE"));
    assert_eq!(choices[0].sprite_names, ["b_alpha"]);
    assert_eq!(choices[1].act, Act::NativeFormation("zeta".into()));
    assert_eq!(choices[1].sprite_names, ["b_zeta"]);
    assert!(choices.iter().all(|b| b.look == Look::Locked && !b.keep_label));
    assert!(choices.iter().all(|b| model::command(&feed, &b.act).is_none()));
    assert_eq!(metadata(&native, &choices[0].act).unwrap().disabled, Some(Color::srgb(0.2, 0.3, 0.4)));
    assert!(shown.iter().any(|b| b.act == Act::Back));
    assert!(!shown.iter().any(|b| matches!(b.act, Act::Formation(_))));
    assert!(buttons(&feed, Page::Top, &native).iter().any(|b| b.act == Act::Stop));
}

#[test]
fn palette_cells_and_input_regions_use_cfg_geometry() {
    let app = app();
    let state = app.world().resource::<StockState>();
    let layout = state.layout.as_ref().unwrap();
    let palette = layout.palette().unwrap();
    let cells = visible_cells(&palette, layout.hud);
    assert_eq!(cells.len(), 9);
    assert_eq!((cells[0].x, cells[0].y), (1140.0, 720.0));
    assert_eq!((cells[1].x, cells[1].y), (1217.0, 720.0));
    assert_eq!((cells[3].x, cells[3].y), (1140.0, 679.0));
    assert!(state.solid.iter().any(|r| r.contains(Vec2::new(1150.0, 730.0))));
    assert!(state.solid.iter().any(|r| r.contains(Vec2::new(245.0, 5.0))));
    assert!(!state.solid.iter().any(|r| r.contains(Vec2::new(1100.0, 700.0))));
}

#[test]
fn disabled_info_variant_does_not_leave_invisible_stats_input_regions() {
    use sta_content::gui_cfg::Rect;
    let mut app = app();
    let mut config = app.world().resource::<HudConfig>().clone();
    let cfg = config.gui.as_mut().unwrap();
    cfg.panels.get_mut("info").unwrap().variants.insert(
        2,
        Rect {
            x: 400,
            y: 900,
            w: 0,
            h: 0,
        },
    );
    cfg.rects.insert(
        "hull".into(),
        Rect {
            x: 900,
            y: 100,
            w: 50,
            h: 50,
        },
    );
    app.insert_resource(config);
    app.update();
    assert!(
        !app.world()
            .resource::<StockState>()
            .solid
            .iter()
            .any(|r| r.contains(Vec2::new(1100.0, 100.0)))
    );
}

#[path = "identity_tests.rs"]
mod identity_tests;

#[test]
fn armada1_resource_readouts_are_solid_for_the_pointer() {
    let app = app_with_content_and_config(content(), single_file_config(A1_RESOURCES), UiProfile::Armada2);
    let state = app.world().resource::<StockState>();
    // Canvas 640x480 on the 1920x1080 test viewport (factor 2.25, x + 240): the dilithium, crew and
    // officer panels (80 wide, at 300, 400 and 500) cover x 915..1095, 1140..1320 and 1365..1545.
    for x in [920.0, 1150.0, 1370.0] {
        assert!(state.solid.iter().any(|r| r.contains(Vec2::new(x, 10.0))), "x {x}");
    }
    assert!(!state.solid.iter().any(|r| r.contains(Vec2::new(700.0, 10.0))));
}

#[test]
fn a_zero_resource_panel_area_hides_the_strip_instead_of_taking_the_armada1_path() {
    let raw = format!("{A1_RESOURCES}resourcePanelArea = 0 0 0 0\n");
    let mut app = app_with_content_and_config(content(), single_file_config(&raw), UiProfile::Armada2);
    assert_eq!(resource_label_count(&mut app), 0);
}

/// UI-37: on a 21:9 window the 4:3 sheet is centred; the margins beside it take world input
/// (nothing there is solid), the HUD art does, and every solid rectangle lies inside the one
/// HUD rectangle that the minimap and the palette share.
#[test]
fn margins_beside_the_centred_hud_are_not_solid() {
    for aspect in [21.0_f32 / 9.0, 32.0 / 9.0] {
        let mut cfg = config();
        cfg.aspect = aspect;
        let app = app_with_content_and_config(content(), cfg, UiProfile::Armada2);
        let state = app.world().resource::<StockState>();
        let layout = state.layout.as_ref().unwrap();
        let hud = layout.hud;
        assert_eq!((hud.w, hud.h), (1440.0, 1080.0), "{aspect}");
        assert!(hud.x > 0.0 && (hud.x - (1080.0 * aspect - hud.right())).abs() <= 1.0, "{aspect}: {hud:?}");
        for r in &state.solid {
            assert!(r.x >= hud.x && r.right() <= hud.right() + 0.5, "{aspect}: solid {r:?} leaves the HUD {hud:?}");
        }
        // Left and right margins, at the minimap's and the palette's heights.
        for y in [100.0, 700.0, 1000.0] {
            for x in [2.0, hud.x - 2.0, hud.right() + 2.0, 1080.0 * aspect - 2.0] {
                assert!(!state.solid.iter().any(|r| r.contains(Vec2::new(x, y))), "{aspect}: ({x},{y}) is a margin");
            }
        }
        // HUD art: the minimap panel's first unit and a palette cell, in the shifted positions.
        let map = layout.panel("minimap").unwrap();
        assert_eq!(map.x, hud.x, "{aspect}");
        assert!(state.solid.iter().any(|r| r.contains(Vec2::new(map.x + 5.0, map.y + 5.0))), "{aspect}");
        let cell = layout.palette().unwrap().cell_rect(0, 0);
        assert!(state.solid.iter().any(|r| r.contains(Vec2::new(cell.x + 5.0, cell.y + 5.0))), "{aspect}");
        // FO authors its minimap panel at x = -1: the placed art starts one unit left of the
        // HUD, but the pointer-solid part stops at the HUD's edge.
        {
            let mut fo = config();
            fo.aspect = aspect;
            fo.gui.as_mut().unwrap().rects.insert("minimapPanelArea".into(), sta_content::Rect { x: -1, y: 800, w: 400, h: 400 });
            let app = app_with_content_and_config(content(), fo, UiProfile::Armada2);
            let state = app.world().resource::<StockState>();
            let hud = state.layout.as_ref().unwrap().hud;
            assert_eq!(state.layout.as_ref().unwrap().panel("minimap").unwrap().x, hud.x - 1.0, "{aspect}");
            assert!(state.solid.iter().all(|r| r.x >= hud.x), "{aspect}: solid reaches into the margin");
            assert!(!state.solid.iter().any(|r| r.contains(Vec2::new(hud.x - 0.5, 1000.0))), "{aspect}");
            assert!(state.solid.iter().any(|r| r.contains(Vec2::new(hud.x + 5.0, 1000.0))), "{aspect}");
        }
        let app = {
            let mut cfg = config();
            cfg.aspect = aspect;
            app_with_content_and_config(content(), cfg, UiProfile::Armada2)
        };
        let state = app.world().resource::<StockState>();
        // The same condition input uses: a pointer in a margin leaves the map free.
        let in_margin = crate::ui::hud::HudPointer(state.solid.iter().any(|r| r.contains(Vec2::new(5.0, 900.0))));
        assert_eq!(in_margin, crate::ui::hud::HudPointer(false));
    }
}

/// UI-16, ADR 0007 T1: HUD labels come from the mod's sprite font. No HUD label is
/// plain Bevy `Text` and none opts out through `ScalableText`; the sprite font itself
/// falls back to the TTF where a mod has none.
#[test]
fn stock_hud_labels_are_sprite_text_and_never_scalable() {
    use crate::ui::gui_sprites::GuiButtonLabel;
    use crate::ui::sprite_font::ScalableText;

    let mut app = app_with_content(content_with_build_count(24));
    app.world_mut().resource_mut::<HudState>().page = Page::Build;
    app.update();
    app.update();
    let world = app.world_mut();
    assert!(world.query::<&ScalableText>().iter(world).next().is_none());
    assert_eq!(world.query::<&StockNav>().iter(world).count(), 2, "paged palette has PREV and NEXT");
    for nav in world.query_filtered::<Entity, With<StockNav>>().iter(world).collect::<Vec<_>>() {
        let kids = world.get::<Children>(nav).expect("nav label").to_vec();
        assert_eq!(kids.len(), 1);
        assert!(world.get::<SpriteText>(kids[0]).is_some());
        assert!(world.get::<Text>(kids[0]).is_none());
    }
    let labels: Vec<Entity> = world.query_filtered::<Entity, With<GuiButtonLabel>>().iter(world).collect();
    assert!(!labels.is_empty());
    for l in labels {
        assert!(world.get::<SpriteText>(l).is_some(), "button label drawn in the sprite font");
        assert!(world.get::<Text>(l).is_none());
    }
}

// UI-48: the button panel.
mod button_panel {
    use super::*;
    use crate::input::select::CameraFocus;
    use crate::ui::hud::stock::buttons::{self, PanelButton};
    use crate::ui::presentation::ObjectivesView;

    fn panel_config() -> HudConfig {
        let text = format!("{CFG}menuButtonArea = 1500 0 50 30\nidleButtonArea = 1440 0 50 30\nobjectivesButtonArea = 1380 0 50 30\nobjectivesButton = \"my_objectives\"\ncommButtonArea = 0 0 0 0\nhelpButtonArea = 1320 0 50 30\n");
        let (cfg, warnings) = GuiCfg::parse("panel.cfg", &text);
        assert!(warnings.is_empty(), "{warnings:?}");
        HudConfig { gui: Some(cfg.layout_auto()), fit_scale: false, ..default() }
    }

    fn panel_app() -> App {
        let mut app = app_with_content_and_config(content(), panel_config(), UiProfile::Armada2);
        app.insert_resource(ObjectivesView::default());
        app.update();
        app
    }

    fn panel(app: &mut App) -> Vec<(Entity, PanelButton, Vec<String>, WidgetState)> {
        let mut out: Vec<_> = app
            .world_mut()
            .query::<(Entity, &PanelButton, &crate::ui::gui_sprites::GuiButton, &WidgetState)>()
            .iter(app.world())
            .map(|(e, b, g, s)| (e, *b, g.names.clone(), *s))
            .collect();
        out.sort_by_key(|o| format!("{:?}", o.1));
        out
    }

    #[test]
    fn the_cfg_places_the_panel_buttons_with_their_sprites_and_a_zero_area_hides_one() {
        let mut app = panel_app();
        let got = panel(&mut app);
        let acts: Vec<_> = got.iter().map(|g| g.1).collect();
        assert_eq!(acts, [PanelButton::Idle, PanelButton::Menu, PanelButton::Objectives, PanelButton::Unavailable], "comm hidden, help drawn: {got:?}");
        let sprite = |b: PanelButton| got.iter().find(|g| g.1 == b).unwrap().2.clone();
        assert_eq!(sprite(PanelButton::Objectives), ["my_objectives"], "the cfg's sprite");
        assert_eq!(sprite(PanelButton::Menu), ["menu_button"], "the engine's default");
        assert_eq!(got.iter().find(|g| g.1 == PanelButton::Unavailable).unwrap().3, WidgetState::Disabled);
    }

    #[test]
    fn objectives_shows_the_objectives_and_idle_asks_the_camera_for_an_idle_ship() {
        let mut app = panel_app();
        let got = panel(&mut app);
        let find = |b: PanelButton| got.iter().find(|g| g.1 == b).unwrap().0;
        click(app.world_mut(), find(PanelButton::Objectives));
        app.update();
        assert!(app.world().resource::<ObjectivesView>().visible);
        // The test feed's cruiser stands still: it is idle.
        click(app.world_mut(), find(PanelButton::Idle));
        app.update();
        let msgs = app.world().resource::<Messages<CameraFocus>>();
        assert!(msgs.iter_current_update_messages().count() + msgs.len() > 0, "a focus request was sent");
    }

    /// The cfg of an authored button panel near the sheet's top right (own numbers, not any game's): two end
    /// pieces, a first row and a second row below it.
    fn placed_config(aspect: f32) -> HudConfig {
        sheet_config(1600, 1338, aspect)
    }

    /// The same panel on a sheet `canvas_w` units wide, its area starting at `area_x` (a wider authored sheet).
    fn sheet_config(canvas_w: i32, area_x: i32, aspect: f32) -> HudConfig {
        let cfg_text = CFG.replace("screenWidth = 1600", &format!("screenWidth = {canvas_w}"));
        let text = format!(
            "{cfg_text}buttonPanelArea = {area_x} 14 240 36\nbuttonBackgroundPanel = \"bbp\"\nbuttonBackgroundPanelArea = 0 0 240 36\nbuttonBackgroundPanelSize = 2\nbuttonBackgroundPanel_0 = 0 0 18 36\nbuttonBackgroundPanel_1 = 222 0 18 36\n\
             idleButtonArea = 28 2 32 32\nobjectivesButtonArea = 124 2 32 32\nmoreButtonArea = 172 2 32 32\nmenuButtonArea = 172 40 32 32\n"
        );
        let (cfg, warnings) = GuiCfg::parse("panel.cfg", &text);
        assert!(warnings.is_empty(), "{warnings:?}");
        HudConfig { gui: Some(cfg.layout_auto()), aspect, fit_scale: false, ..default() }
    }

    fn group_rect(app: &mut App) -> (Vec2, Vec2) {
        let (node,) = app.world_mut().query_filtered::<(&Node,), With<buttons::ButtonPanelGroup>>().single(app.world()).unwrap();
        let px = |v: Val| if let Val::Px(p) = v { p } else { panic!("{v:?}") };
        (Vec2::new(px(node.left), px(node.top)), Vec2::new(px(node.width), px(node.height)))
    }

    #[test]
    fn the_group_sits_at_the_sheets_top_right_in_the_hud_area_with_its_end_pieces_and_the_buttons_inside_it() {
        let mut app = app_with_content_and_config(content(), placed_config(4.0 / 3.0), UiProfile::Armada2);
        app.update();
        let layout = app.world().resource::<StockState>().layout.clone().unwrap();
        let (pos, size) = group_rect(&mut app);
        let panel = layout.panel("button").unwrap();
        assert_eq!((pos.x, pos.y, size.x, size.y), (panel.x, panel.y, panel.w, panel.h));
        assert!(pos.x + size.x <= layout.hud.right() && (layout.hud.right() - (pos.x + size.x)).abs() < 30.0, "at the sheet's right edge: {pos:?} {size:?}");
        let pieces = app.world_mut().query_filtered::<&Node, (With<crate::ui::gui_sprites::GuiImage>, Without<buttons::PanelButton>)>().iter(app.world()).count();
        assert!(pieces >= 2, "the two end pieces are drawn");
        // Every button is a child of the group (its area is relative to it).
        let group = app.world_mut().query_filtered::<Entity, With<buttons::ButtonPanelGroup>>().single(app.world()).unwrap();
        let n = app.world_mut().query::<(&buttons::PanelButton, &ChildOf)>().iter(app.world()).filter(|(_, c)| c.parent() == group).count();
        assert_eq!(n, 4);
    }

    #[test]
    fn the_group_is_placed_inside_the_clamped_centred_area_like_every_panel_with_its_pointer_rect_from_the_same_rect() {
        // ADR 0008 H1/H2: a 4:3 sheet and a 16:9 sheet, four window aspects, the Centred fit.
        for (canvas_w, area_x) in [(1600, 1338), (2000, 1738)] {
            for aspect in [4.0_f32 / 3.0, 16.0 / 9.0, 21.0 / 9.0, 32.0 / 9.0] {
                let mut app = app_with_content_and_config(content(), sheet_config(canvas_w, area_x, aspect), UiProfile::Armada2);
                let (layout, solid) = {
                    let state = app.world().resource::<StockState>();
                    (state.layout.clone().unwrap(), state.solid.clone())
                };
                let tag = format!("sheet {canvas_w}, window {aspect}");
                let r = layout.panel("button").unwrap();
                // Size and sheet-relative position are the sheet's (to the rounding of whole units).
                let s = layout.scale;
                assert!((r.w - 240.0 * s).abs() <= 1.0 && (r.h - 36.0 * s).abs() <= 1.0, "{tag}: size {r:?}");
                assert!(((r.x - layout.hud.x) - area_x as f32 * s).abs() <= 1.0 && ((r.y - layout.hud.y) - 14.0 * s).abs() <= 1.0, "{tag}: place {r:?}");
                assert!(r.x >= layout.hud.x && r.right() <= layout.hud.right() && r.y >= layout.hud.y && r.bottom() <= layout.hud.bottom(), "{tag}: inside the HUD area");
                // Drawn rectangle (the group node) and pointer-solid rectangle are the one placed rect.
                let (pos, size) = group_rect(&mut app);
                assert_eq!((pos.x, pos.y, size.x, size.y), (r.x, r.y, r.w, r.h), "{tag}: drawn");
                assert!(solid.contains(&r), "{tag}: pointer-solid equals the drawn rect");
                assert!(solid.iter().all(|o| o.x >= layout.hud.x - 0.5 && o.right() <= layout.hud.right() + 0.5), "{tag}: no solid outside the HUD area");
            }
        }
        // Stretched fills the window: the rect is the sheet's, scaled per axis, and solid.
        let mut cfg = placed_config(21.0 / 9.0);
        cfg.fit = crate::ui::hud::stock::layout::HudFit::Stretched;
        let app = app_with_content_and_config(content(), cfg, UiProfile::Armada2);
        let state = app.world().resource::<StockState>();
        let layout = state.layout.as_ref().unwrap();
        let r = layout.panel("button").unwrap();
        assert!((r.right() - 1578.0 * layout.scale_x).abs() <= 1.0, "stretched: the sheet's place on the whole window: {r:?}");
        assert!(state.solid.contains(&r));
    }

    #[test]
    fn a_click_on_a_shown_second_row_button_is_pointer_solid_and_a_hidden_one_is_not() {
        let mut app = app_with_content_and_config(content(), placed_config(4.0 / 3.0), UiProfile::Armada2);
        app.update();
        let more = app.world_mut().query::<(Entity, &PanelButton)>().iter(app.world()).find(|(_, p)| **p == PanelButton::More).map(|(e, _)| e).unwrap();
        let layout = app.world().resource::<StockState>().layout.clone().unwrap();
        let panel = layout.panel("button").unwrap();
        let second = layout.at(Vec2::new(panel.x, panel.y), "menuButtonArea").unwrap();
        let centre = Vec2::new(second.x + second.w / 2.0, second.y + second.h / 2.0);
        assert!(!app.world().resource::<StockState>().solid.iter().any(|r| r.contains(centre)), "hidden: the world's");
        click(app.world_mut(), more);
        app.update();
        app.update();
        assert!(app.world().resource::<StockState>().solid.iter().any(|r| r.contains(centre)), "shown: not the world's");
    }

    /// The solid rectangles with the second row open or closed, and the points of the gap between the rows
    /// and of the empty part of the second row, for a profile.
    fn gap_probe(profile: UiProfile, open: bool) -> (bool, bool) {
        let mut app = app_with_content_and_config(content(), placed_config(4.0 / 3.0), profile);
        app.update();
        if open {
            let more = app.world_mut().query::<(Entity, &PanelButton)>().iter(app.world()).find(|(_, p)| **p == PanelButton::More).map(|(e, _)| e).unwrap();
            click(app.world_mut(), more);
            app.update();
            app.update();
        }
        let state = app.world().resource::<StockState>();
        let layout = state.layout.as_ref().unwrap();
        let panel = layout.panel("button").unwrap();
        let second = layout.at(Vec2::new(panel.x, panel.y), "menuButtonArea").unwrap();
        let hit = |p: Vec2| state.solid.iter().any(|r| r.contains(p));
        // Between the rows (below the panel, above the second row) and left of the second row's buttons.
        (hit(Vec2::new(panel.x + panel.w / 2.0, panel.bottom() + 1.0)), hit(Vec2::new(panel.x + 10.0, second.y + 5.0)))
    }

    #[test]
    fn in_the_armada_ii_look_one_pointer_rect_covers_both_rows_while_the_second_row_is_open() {
        assert_eq!(gap_probe(UiProfile::Armada2, true), (true, true), "open: the gap and the empty part are the HUD's");
        assert_eq!(gap_probe(UiProfile::Armada2, false), (false, false), "closed: the first row's area only");
    }

    #[test]
    fn the_fleet_operations_look_keeps_the_buttons_own_areas_while_the_second_row_is_open() {
        assert_eq!(gap_probe(UiProfile::FleetOps, true), (false, false));
        assert_eq!(gap_probe(UiProfile::FleetOps, false), (false, false));
    }

    #[test]
    fn only_the_armada_ii_looks_group_buttons_carry_the_display_space_tint() {
        for (profile, want) in [(UiProfile::Armada2, true), (UiProfile::FleetOps, false)] {
            let mut app = app_with_content_and_config(content(), placed_config(4.0 / 3.0), profile);
            app.update();
            let tinted = app.world_mut().query_filtered::<&crate::ui::gui_sprites::DisplayTint, With<PanelButton>>().iter(app.world()).count();
            let buttons = app.world_mut().query::<&PanelButton>().iter(app.world()).count();
            assert!(buttons >= 4);
            assert_eq!(tinted, if want { buttons } else { 0 }, "{profile:?}");
            let others = app.world_mut().query_filtered::<(), (With<crate::ui::gui_sprites::DisplayTint>, Without<PanelButton>)>().iter(app.world()).count();
            assert_eq!(others, 0, "no other button carries it");
        }
        let t = buttons::GROUP_TINT;
        assert!(t.hovered.iter().chain(&t.pressed).all(|v| *v > 0.0));
    }

    #[test]
    fn the_group_overlaps_no_other_stock_panel_at_any_width() {
        for aspect in [1.0_f32, 5.0 / 4.0, 4.0 / 3.0, 16.0 / 9.0, 21.0 / 9.0] {
            let app = app_with_content_and_config(content(), placed_config(aspect), UiProfile::Armada2);
            let state = app.world().resource::<StockState>();
            let layout = state.layout.as_ref().unwrap();
            let r = layout.panel("button").unwrap();
            let overlaps = |o: R| r.x < o.right() && o.x < r.right() && r.y < o.bottom() && o.y < r.bottom();
            for name in ["resource", "minimap"] {
                assert!(!layout.panel(name).is_some_and(overlaps), "{aspect}: {name}");
            }
            assert!(!layout.info_area(0).is_some_and(overlaps), "{aspect}: info");
            assert!(r.right() <= layout.screen.x + 0.5, "{aspect}: on screen");
        }
    }

    #[test]
    fn the_second_row_starts_hidden_the_first_row_shows_and_more_expands_it_and_menu_opens_the_in_game_menu() {
        let mut app = app_with_content_and_config(content(), placed_config(4.0 / 3.0), UiProfile::Armada2);
        app.update();
        let find = |app: &mut App, b: PanelButton| {
            app.world_mut().query::<(Entity, &PanelButton)>().iter(app.world()).find(|(_, p)| **p == b).map(|(e, _)| e).unwrap()
        };
        let display = |app: &App, e: Entity| app.world().get::<Node>(e).unwrap().display;
        assert!(!app.world().resource::<buttons::ShowMore>().0, "collapsed by default");
        let (menu, more, idle, objectives) =
            (find(&mut app, PanelButton::Menu), find(&mut app, PanelButton::More), find(&mut app, PanelButton::Idle), find(&mut app, PanelButton::Objectives));
        assert_eq!(display(&app, menu), Display::None, "the second row is hidden at the start");
        // The always-visible row: drawn, inside the group, at its own cfg place (the area is relative to the group).
        let layout = app.world().resource::<StockState>().layout.clone().unwrap();
        for (e, key) in [(idle, "idleButtonArea"), (objectives, "objectivesButtonArea"), (more, "moreButtonArea")] {
            assert_eq!(display(&app, e), Display::Flex, "{key} shows when collapsed");
            let node = app.world().get::<Node>(e).unwrap();
            let r = layout.rect(key).unwrap();
            assert_eq!((node.left, node.top), (Val::Px(r.x), Val::Px(r.y)), "{key} at its stock place");
            assert_ne!(*app.world().get::<WidgetState>(e).unwrap(), WidgetState::Disabled, "{key} is enabled");
        }
        // More expands the row; the menu button then opens the menu through the real press system.
        click(app.world_mut(), more);
        app.update();
        app.update();
        assert_eq!(display(&app, menu), Display::Flex);
        assert_eq!(display(&app, idle), Display::Flex);
        click(app.world_mut(), menu);
        app.update();
        let msgs = app.world().resource::<Messages<crate::ui::ingame_menu::OpenInGameMenu>>();
        assert!(msgs.iter_current_update_messages().count() + msgs.len() > 0, "the in-game menu opens");
        // The expanded state survives a rebuild of the HUD (a layout change).
        app.world_mut().resource_mut::<HudConfig>().aspect = 21.0 / 9.0;
        app.update();
        app.update();
        let menu = find(&mut app, PanelButton::Menu);
        assert_eq!(display(&app, menu), Display::Flex);
        let more = find(&mut app, PanelButton::More);
        click(app.world_mut(), more);
        app.update();
        app.update();
        let menu = find(&mut app, PanelButton::Menu);
        assert_eq!(display(&app, menu), Display::None, "a second press collapses it again");
    }

    #[test]
    fn a_cfg_without_a_panel_area_or_background_still_gives_usable_buttons() {
        let text = format!("{CFG}idleButtonArea = 28 2 32 32\nmenuButtonArea = 172 40 32 32\n");
        let (cfg, _) = GuiCfg::parse("panel.cfg", &text);
        let config = HudConfig { gui: Some(cfg.layout_auto()), aspect: 21.0 / 9.0, fit_scale: false, ..default() };
        let mut app = app_with_content_and_config(content(), config, UiProfile::Armada2);
        app.update();
        assert_eq!(app.world_mut().query::<&PanelButton>().iter(app.world()).count(), 2);
    }

    #[test]
    fn next_idle_takes_the_idle_ships_in_turn() {
        let db = content();
        let mut feed = feed(&db);
        let mut view = (*feed.view).clone();
        let mut second = view.units[0].clone();
        second.id = EntityId(7);
        view.units.push(second);
        let mut enemy = view.units[0].clone();
        enemy.id = EntityId(9);
        enemy.owner = PlayerId(2);
        view.units.push(enemy);
        feed.view = Arc::new(view);
        let first = buttons::next_idle(&feed, None).unwrap().id;
        let then = buttons::next_idle(&feed, Some(first)).unwrap().id;
        let wrap = buttons::next_idle(&feed, Some(then)).unwrap().id;
        assert_eq!((first, then, wrap), (EntityId(1), EntityId(7), EntityId(1)), "own ships only, wrapping");
    }
}

#[test]
fn native_tara_requirements_gate_the_menu_and_dispatched_units() {
    let raw = |body: &str| Arc::new(sta_odf::resolve("test.odf", &mut |_| Some(body.to_owned())).unwrap());
    let fake = sta_content::fake::db();
    let mut builder = ContentDb::builder();
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    for def in fake.units() {
        let mut def = def.clone();
        if def.odf == "fx_cruiser" {
            def.raw = raw("alert = 1\ncombat = 1\ncan_sandd = 1\ncan_explore = 1\nshow_movement_autonomy = 1\n");
            let mut scout = def.clone();
            scout.odf = "scout_only".into();
            scout.key = sta_content::DefRef::new("scout_only").key;
            scout.raw = raw("combat = 1\ncan_sandd = 0\ncan_explore = 1\n");
            builder = builder.unit(scout);
        }
        builder = builder.unit(def);
    }
    let db = Arc::new(builder.build().unwrap());
    let mut app = app_with_content(db.clone());
    let mut source = ScriptedSource::from_json5(
        "{ units: [
          { id: 1, def: 'fx_cruiser', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
          { id: 2, def: 'scout_only', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
          { id: 3, def: 'fx_cruiser', owner: 2, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] }
        ] }",
        &db,
    ).unwrap();
    let mut view = (*source.step(&[]).view).clone();
    view.units[0].autonomy = Some([1, 0]);
    {
        let mut feed = app.world_mut().resource_mut::<HudFeed>();
        feed.view = Arc::new(view);
        feed.selected = vec![EntityId(1), EntityId(2), EntityId(3)];
    }
    {
        let mut state = app.world_mut().resource_mut::<StockState>();
        state.native.groups = vec!["ai".into(), "orders".into()];
        for (odf, command, menu, requirements) in [
            ("scout", "SCOUT", "orders", "ship can_explore"),
            ("sandd", "SEARCH_AND_DESTROY", "orders", "ship combat can_sandd"),
            ("green", "GREEN_ALERT", "ai", "alert"),
            ("red", "RED_ALERT", "ai", "alert"),
            ("movemed", "MOVE_MED", "ai", "alert show_movement_autonomy"),
            ("swhigh", "SPECIAL_WEAPON_AUTONOMY_HIGH", "ai", "alert show_sw_autonomy"),
        ] {
            state.native.commands.insert(odf.into(), NativeCommand {
                odf: format!("{odf}.odf"),
                command: command.into(),
                button: odf.into(),
                menu: Some(menu.into()),
                source: requirements.split_whitespace().map(str::to_owned).collect(),
                tooltip: Some(format!("TIP_{odf}")),
                verbose: Some(format!("VERBOSE_{odf}")),
                ..default()
            });
        }
    }
    app.update();
    let orders = cell(&mut app, Act::OpenNativeMenu(1));
    click(app.world_mut(), orders);
    app.update();
    app.update();
    let search = cell(&mut app, Act::NativeOrder("sandd".into()));
    let tooltip = app.world().get::<Tooltip>(search).unwrap();
    assert_eq!(tooltip.short, "TIP_sandd");
    assert_eq!(tooltip.verbose.as_deref(), Some("VERBOSE_sandd"));
    click(app.world_mut(), search);
    app.update();
    let scout = cell(&mut app, Act::NativeOrder("scout".into()));
    click(app.world_mut(), scout);
    app.update();
    let back = cell(&mut app, Act::Back);
    click(app.world_mut(), back);
    app.update();
    app.update();
    let ai = cell(&mut app, Act::OpenNativeMenu(0));
    click(app.world_mut(), ai);
    app.update();
    app.update();
    let red = cell(&mut app, Act::NativeOrder("red".into()));
    let medium = cell(&mut app, Act::NativeOrder("movemed".into()));
    assert!(app.world().get::<GuiActive>(red).unwrap().0);
    assert!(app.world().get::<GuiActive>(medium).unwrap().0);
    assert!(!app.world().resource::<StockState>().grid.iter().any(|b| b.act == Act::NativeOrder("swhigh".into())));
    let green = cell(&mut app, Act::NativeOrder("green".into()));
    click(app.world_mut(), green);
    app.update();
    click(app.world_mut(), medium);
    app.update();
    use sta_sim::command::{AlertState, AutonomyKind, AutonomyLevel};
    assert_eq!(app.world().resource::<CommandQueue>().0, [
        CommandKind::Order(Order::SearchAndDestroy { units: vec![EntityId(1)] }),
        CommandKind::Order(Order::Scout { units: vec![EntityId(1), EntityId(2)] }),
        CommandKind::Order(Order::Alert { units: vec![EntityId(1)], level: AlertState::Green }),
        CommandKind::Order(Order::Autonomy { units: vec![EntityId(1)], kind: AutonomyKind::Movement, level: AutonomyLevel::Medium }),
    ]);
}

// ---- the Armada II cinematic window (#1073): authored layout data only, no install numbers

/// A sheet that declares the window: the panel, the display, three frame pieces and the still.
const WINDOW_CFG: &str = "\
screenWidth = 1600
screenHeight = 1200
minimapPanelArea = 0 800 400 400
cinematicPanelArea = 1200 800 400 400
cinematicDisplayArea = 20 30 300 300
cinematicBackgroundPanel = \"cine_frame\"
cinematicBackgroundPanelSize = 3
cinematicBackgroundPanel_0 = 0 0 400 30
cinematicBackgroundPanel_1 = 0 30 20 340
cinematicBackgroundPanel_2 = 0 370 400 30
cinematicDefaultBackground = \"cine_still\"
";

/// The same sheet with the window declared off (no area, a zero size).
const OFF_CFG: &str = "\
screenWidth = 1600
screenHeight = 1200
minimapPanelArea = 0 800 400 400
cinematicPanelArea = -1 -1 0 0
cinematicDisplayArea = 0 0 0 0
cinematicBackgroundPanelSize = 0
";

fn window_app(cfg: &str, aspect: f32, fit: super::layout::HudFit, view: bool) -> App {
    let (parsed, warnings) = GuiCfg::parse("authored_window.cfg", cfg);
    assert!(warnings.is_empty(), "{warnings:?}");
    let config = HudConfig { gui: Some(parsed.layout_auto()), aspect, fit, fit_scale: false, ..default() };
    let mut app = app_with_content_and_config(content(), config, UiProfile::Armada2);
    if view {
        app.world_mut().insert_resource(crate::render::action_camera::ActionView { image: Handle::default() });
        // Rebuild the body with the picture's resource in place.
        app.world_mut().resource_mut::<HudConfig>().aspect += 0.001;
        app.update();
        app.update();
    }
    app
}

fn px(n: &Node) -> R {
    let (Val::Px(x), Val::Px(y), Val::Px(w), Val::Px(h)) = (n.left, n.top, n.width, n.height) else { panic!("px") };
    R::new(x, y, w, h)
}

fn art_named(app: &mut App, name: &str) -> Vec<(usize, R)> {
    let mut found: Vec<(usize, R)> = app
        .world_mut()
        .query::<(&crate::ui::gui_sprites::GuiImage, &Node)>()
        .iter(app.world())
        .filter(|(g, _)| g.names.first().is_some_and(|n| n == name))
        .map(|(g, n)| (g.frame, px(n)))
        .collect();
    found.sort_by_key(|f| f.0);
    found
}

fn picture_nodes(app: &mut App) -> Vec<R> {
    app.world_mut().query_filtered::<&Node, With<crate::render::action_camera::ActionPicture>>().iter(app.world()).map(px).collect()
}

#[test]
fn a_layout_that_declares_the_window_gets_panel_frame_still_and_picture_in_both_fits() {
    use super::layout::HudFit;
    for fit in HudFit::ALL {
        for aspect in [4.0_f32 / 3.0, 16.0 / 9.0, 21.0 / 9.0] {
            let mut app = window_app(WINDOW_CFG, aspect, fit, true);
            let layout = app.world().resource::<StockState>().layout.clone().unwrap();
            let panel = layout.panel("cinematic").expect("the declared panel");
            let origin = Vec2::new(panel.x, panel.y);
            let display = layout.at(origin, "cinematicDisplayArea").unwrap();
            let want: Vec<(usize, R)> = layout
                .pieces_in("cinematic", "cinematicBackgroundPanel")
                .into_iter()
                .map(|p| (p.frame, R::new(origin.x + p.rect.x, origin.y + p.rect.y, p.rect.w, p.rect.h)))
                .collect();
            assert_eq!(want.len(), 3);
            assert_eq!(art_named(&mut app, "cine_frame"), want, "{fit:?} {aspect}: frame pieces at the declared rectangles");
            assert_eq!(art_named(&mut app, "cine_still"), vec![(0, display)], "{fit:?} {aspect}: the still picture fills the display");
            assert_eq!(picture_nodes(&mut app), vec![display], "{fit:?} {aspect}: the live picture is in the display");
            // The panel group moves as a whole with the sheet: everything stays inside it.
            let inside = |r: R| r.x >= layout.hud.x - 0.5 && r.right() <= layout.hud.right() + 0.5 && r.y >= layout.hud.y - 0.5 && r.bottom() <= layout.hud.bottom() + 0.5;
            assert!(inside(panel) && inside(display), "{fit:?} {aspect}");
            // Centred keeps the picture's aspect on every window; Stretched scales per axis like every panel.
            if fit == HudFit::Centred {
                assert!((display.w / display.h - 1.0).abs() < 0.01, "{aspect}: the 1:1 display stays square");
            }
            // The window has no buttons of its own and shields the pointer.
            let buttons = app.world_mut().query::<&super::buttons::PanelButton>().iter(app.world()).filter(|b| matches!(b, super::buttons::PanelButton::Menu | super::buttons::PanelButton::Unavailable)).count();
            assert_eq!(buttons, 0, "{fit:?} {aspect}");
            assert!(app.world().resource::<StockState>().solid.contains(&panel), "{fit:?} {aspect}: the panel takes the pointer");
        }
    }
}

#[test]
fn without_the_action_view_the_window_still_draws_its_frame_and_still_but_no_picture() {
    let mut app = window_app(WINDOW_CFG, 16.0 / 9.0, super::layout::HudFit::Centred, false);
    assert_eq!(art_named(&mut app, "cine_frame").len(), 3);
    assert_eq!(art_named(&mut app, "cine_still").len(), 1);
    assert!(picture_nodes(&mut app).is_empty());
}

#[test]
fn a_layout_that_declares_the_window_off_or_not_at_all_gets_no_window() {
    use super::layout::HudFit;
    for cfg in [OFF_CFG, "screenWidth = 1600\nscreenHeight = 1200\nminimapPanelArea = 0 800 400 400\n"] {
        for fit in HudFit::ALL {
            let mut app = window_app(cfg, 16.0 / 9.0, fit, true);
            assert!(art_named(&mut app, "cine_frame").is_empty() && art_named(&mut app, "cine_still").is_empty());
            assert!(picture_nodes(&mut app).is_empty(), "{fit:?}: no live picture");
            let layout = app.world().resource::<StockState>().layout.clone().unwrap();
            assert!(layout.panel("cinematic").is_none());
        }
    }
}

#[test]
fn the_movie_panel_and_the_window_read_the_same_keys_and_the_window_goes_quiet_while_a_clip_plays() {
    use super::layout::HudFit;
    // One layout, one spec: the movie panel's display is the window's picture rectangle.
    let app = window_app(WINDOW_CFG, 16.0 / 9.0, HudFit::Centred, true);
    let layout = app.world().resource::<StockState>().layout.clone().unwrap();
    let spec = crate::ui::hud::cinematic::stock_spec(&layout).expect("the movie panel");
    let panel = layout.panel("cinematic").unwrap();
    let display = layout.at(Vec2::new(panel.x, panel.y), "cinematicDisplayArea").unwrap();
    assert_eq!((spec.panel, spec.display), (panel, display));
    // The window's camera pair goes quiet while a clip waits for or plays in a panel:
    // `render::action_camera::tests::the_cameras_go_quiet_while_a_clip_plays_in_the_panel`.
}

/// Install-gated (`STA64_TEST_A2_STOCK_INSTALL`, `STA64_TEST_ROOTS_INSTALL` or `STA64_TEST_INSTALL`,
/// read at run time, no numbers committed): for every mounted `gui_*.cfg`, the window exists exactly
/// when the layout declares a visible cinematic panel; then its frame pieces and its still picture
/// resolve in the install's GUI sprite tables. A layout that declares it off yields no window.
#[test]
fn installed_layouts_give_a_window_exactly_where_they_declare_one_and_its_art_resolves() {
    let mut ran = 0;
    for var in ["STA64_TEST_A2_STOCK_INSTALL", "STA64_TEST_ROOTS_INSTALL", "STA64_TEST_INSTALL"] {
        let Some(root) = std::env::var_os(var) else { continue };
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), None).unwrap();
        let tables: Vec<_> = vfs
            .list("sprites")
            .into_iter()
            .filter(|p| p.rsplit('/').next().is_some_and(|f| f.starts_with("gui_") && f.ends_with(".spr")))
            .map(|p| crate::ui::gui_sprites::read_table(&vfs, &p))
            .collect();
        let resolves = |name: &str, frame: usize| {
            tables.iter().any(|t| t.sprite(&format!("{name}.{frame}")).is_some() || (frame == 0 && t.sprite(name).is_some()))
        };
        for cfg in vfs.list("misc").into_iter().filter(|p| p.rsplit('/').next().is_some_and(|f| f.starts_with("gui_") && f.ends_with(".cfg"))) {
            let file = cfg.rsplit('/').next().unwrap();
            let Some(gui) = crate::game::gui_layout(&vfs, file) else { continue };
            let layout = StockLayout::for_aspect_fit(&gui, 4.0 / 3.0, super::layout::HudFit::Centred);
            let mut world = World::new();
            let root_entity = world.spawn_empty().id();
            let mut queue = bevy::ecs::world::CommandQueue::default();
            let drawn = window::spawn_a2(&mut Commands::new(&mut queue, &world), root_entity, &layout, None);
            queue.apply(&mut world);
            assert_eq!(drawn, layout.panel("cinematic").is_some(), "{var} {file}: a window exactly where the layout declares one");
            if !drawn {
                assert_eq!(world.entity(root_entity).get::<Children>().map_or(0, |c| c.len()), 0, "{var} {file}: nothing drawn");
                continue;
            }
            ran += 1;
            for piece in layout.pieces_in("cinematic", "cinematicBackgroundPanel") {
                assert!(resolves(&piece.name, piece.frame), "{var} {file}: frame piece {} does not resolve", piece.frame);
            }
            if let Some(still) = layout.name("cinematicDefaultBackground") {
                assert!(resolves(still, 0), "{var} {file}: the still picture does not resolve");
            }
        }
    }
    if ran == 0 {
        eprintln!("SKIP (or no layout declares the window): set STA64_TEST_A2_STOCK_INSTALL, STA64_TEST_ROOTS_INSTALL or STA64_TEST_INSTALL");
    }
}

#[test]
fn armada2_yard_personnel_and_emblem_use_builder_rectangles() {
    use crate::ui::gui_sprites::GuiImage;
    use crate::ui::hud::readouts::{Readout, ReadoutTarget};

    let base = content();
    let mut db = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for weapon in base.weapons() {
        db = db.weapon(weapon.clone());
    }
    for def in base.units() {
        let mut def = def.clone();
        if def.odf == "fx_yard" {
            def.build.officers = Fixed::from_int(10);
            def.race = "federation".into();
        }
        db = db.unit(def);
    }
    let db = Arc::new(db.build().unwrap());
    let cfg = inherited_config("", "\
infoBuildCrew = 30 30 60 24
infoBuildCrewIcon = 5 30 24 24
infoBuildOfficers = 30 55 60 24
infoBuildOfficersIcon = 5 55 24 24
infoBuildEnergy = 30 80 60 24
infoBuildEnergyIcon = 5 80 24 24
infoBuildShieldBar = 5 105 80 8
infoProgressBar = 210 280 210 10
infoSingleRaceIconDisplayArea = 405 20 80 80
");
    let mut app = app_with_content_and_config(db.clone(), cfg, UiProfile::Armada2);
    let mut feed = app.world().resource::<HudFeed>().clone();
    let unit = &mut Arc::make_mut(&mut feed.view).units[0];
    unit.def = db.key("fx_yard").unwrap();
    unit.crew = Some([Fixed::from_int(150); 2]);
    unit.energy = Some([Fixed::from_int(7), Fixed::from_int(10)]);
    unit.hull = [Fixed::from_int(50), Fixed::from_int(100)];
    unit.shields = [[Fixed::from_int(100); 2]; 4];
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    let person_label = |app: &mut App, value: &str| {
        app.world_mut().query::<(Entity, &SpriteText, &ChildOf, &Visibility, &selection_readouts::SelectionReadout)>()
            .iter(app.world())
            .find(|(_, text, _, vis, _)| text.text == value && **vis == Visibility::Inherited)
            .map(|(entity, _, parent, _, _)| (entity, app.world().get::<Node>(parent.parent()).unwrap().clone()))
            .unwrap()
    };
    for (key, count) in [("infoBuildCrew", "150"), ("infoBuildOfficers", "10"), ("infoBuildEnergy", "70%")] {
        let (_, placement) = person_label(&mut app, count);
        let layout = app.world().resource::<StockState>().layout.as_ref().unwrap();
        let rect = layout.at(layout.info_origin(2), key).unwrap();
        assert_eq!((placement.left, placement.top), (Val::Px(rect.x), Val::Px(rect.y)));
    }
    assert!(app.world_mut().query::<&GuiImage>().iter(app.world())
        .any(|image| image.names.iter().any(|name| name == "federation_icon")));
    assert!(app.world().resource::<HudFeed>().production.is_empty());
    assert!(app.world_mut().query::<&GuiImage>().iter(app.world())
        .any(|image| image.names.iter().any(|name| name == "large_construction_bar")));
    assert!(app.world_mut()
        .query::<(&info::StockBar, &Node, &BackgroundColor)>()
        .iter(app.world())
        .any(|(bar, node, color)| matches!(bar, info::StockBar::NativeHealth(w) if node.width == Val::Px(*w * 0.5))
            && color.0 == Color::srgb(0.0, 1.0, 0.0)),
        "stock health gauge shows half hull with a full-shield tint");

    assert_eq!(app.world_mut().query::<&ReadoutTarget>().iter(app.world())
        .filter(|ReadoutTarget(_, kind)| matches!(kind, Readout::Officers)).count(), 2);

    let (officer, _) = person_label(&mut app, "10");
    let (crew, _) = person_label(&mut app, "150");
    let mut feed = app.world().resource::<HudFeed>().clone();
    let unit = &mut Arc::make_mut(&mut feed.view).units[0];
    unit.crew.as_mut().unwrap()[0] = Fixed::from_int(73);
    unit.energy = None;
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    assert_eq!(person_label(&mut app, "73").0, crew);
    assert_eq!(person_label(&mut app, "10").0, officer);
    assert!(app.world_mut().query::<(&SpriteText, &Visibility, &selection_readouts::SelectionReadout)>()
        .iter(app.world()).any(|(text, vis, _)| text.text == "70%" && *vis == Visibility::Hidden));
}

#[test]
fn roots_hosted_classic_uses_medium_card_readouts_and_live_values() {
    use crate::ui::gui_sprites::GuiImage;
    use crate::ui::hud::readouts::{Readout, ReadoutTarget};

    let base = content();
    let mut db = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for weapon in base.weapons() {
        db = db.weapon(weapon.clone());
    }
    for def in base.units() {
        let mut def = def.clone();
        if def.odf == "fx_cruiser" {
            def.build.officers = Fixed::from_int(4);
            def.build.list.clear();
            let mut no_officers = def.clone();
            no_officers.odf = "fx_cruiser_no_officers".into();
            no_officers.build.officers = Fixed::ZERO;
            db = db.unit(no_officers);
        }
        db = db.unit(def);
    }
    let db = Arc::new(db.build().unwrap());
    let cfg = inherited_config("", "\
infoCrewNum = 37 35 57 24
infoCrewNumIcon = 8 35 24 24
infoOfficers = 37 65 57 24
infoOfficersIcon = 8 65 24 24
infoEnergy = 37 95 57 24
infoEnergyIcon = 8 95 24 24
");
    let mut app = app_with_content_and_config(db.clone(), cfg, UiProfile::FleetOps);
    let mut feed = app.world().resource::<HudFeed>().clone();
    let unit = &mut Arc::make_mut(&mut feed.view).units[0];
    unit.crew = Some([Fixed::from_int(150); 2]);
    unit.energy = Some([Fixed::from_int(2), Fixed::from_int(3)]);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();

    let rect = |app: &App, key| {
        let layout = app.world().resource::<StockState>().layout.as_ref().unwrap();
        layout.at(layout.info_origin(1), key).unwrap()
    };
    let label = |app: &mut App, key| {
        let r = rect(app, key);
        app.world_mut()
            .query::<(Entity, &SpriteText, &ChildOf, &Visibility, &selection_readouts::SelectionReadout)>()
            .iter(app.world())
            .find_map(|(entity, text, parent, vis, _)| {
                let node = app.world().get::<Node>(parent.parent())?;
                (node.left == Val::Px(r.x) && node.top == Val::Px(r.y))
                    .then(|| (entity, text.text.clone(), *vis))
            })
            .unwrap_or_else(|| panic!("missing selection label for {key}"))
    };
    let icon = |app: &mut App, key| {
        let r = rect(app, key);
        app.world_mut()
            .query::<(&GuiImage, &Node, &Visibility, &selection_readouts::SelectionReadout)>()
            .iter(app.world())
            .find_map(|(image, node, vis, _)| {
                (node.left == Val::Px(r.x) && node.top == Val::Px(r.y))
                    .then(|| (image.names.clone(), *vis))
            })
            .unwrap_or_else(|| panic!("missing selection icon for {key}"))
    };

    let (crew_id, crew, crew_vis) = label(&mut app, "infoCrewNum");
    assert_eq!((crew, crew_vis), ("150".into(), Visibility::Inherited));
    assert_eq!(label(&mut app, "infoOfficers").1, "4");
    let (energy_id, energy, energy_vis) = label(&mut app, "infoEnergy");
    assert_eq!((energy, energy_vis), ("67%".into(), Visibility::Inherited));
    assert_eq!(icon(&mut app, "infoCrewNumIcon"), (vec!["crew_amt".into()], Visibility::Inherited));
    assert_eq!(icon(&mut app, "infoOfficersIcon"), (vec!["officers".into()], Visibility::Inherited));
    assert_eq!(icon(&mut app, "infoEnergyIcon"), (vec!["energy_amt".into()], Visibility::Inherited));
    for readout in [Readout::Crew, Readout::Officers, Readout::Energy] {
        assert_eq!(app.world_mut().query::<&ReadoutTarget>().iter(app.world())
            .filter(|ReadoutTarget(id, found)| *id == EntityId(1) &&
                std::mem::discriminant(found) == std::mem::discriminant(&readout))
            .count(), 2);
    }

    let mut feed = app.world().resource::<HudFeed>().clone();
    let unit = &mut Arc::make_mut(&mut feed.view).units[0];
    unit.crew.as_mut().unwrap()[0] = Fixed::from_int(73);
    unit.energy.as_mut().unwrap()[0] = Fixed::ZERO;
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    assert_eq!(label(&mut app, "infoCrewNum"), (crew_id, "73".into(), Visibility::Inherited));
    assert_eq!(label(&mut app, "infoEnergy"), (energy_id, "0%".into(), Visibility::Inherited));

    let mut feed = app.world().resource::<HudFeed>().clone();
    let unit = &mut Arc::make_mut(&mut feed.view).units[0];
    unit.crew = None;
    unit.energy = None;
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    assert_eq!(label(&mut app, "infoCrewNum").2, Visibility::Hidden);
    assert_eq!(icon(&mut app, "infoCrewNumIcon").1, Visibility::Hidden);
    assert_eq!(label(&mut app, "infoEnergy").2, Visibility::Hidden);
    assert_eq!(icon(&mut app, "infoEnergyIcon").1, Visibility::Hidden);
    assert_eq!(label(&mut app, "infoOfficers").2, Visibility::Inherited);

    let mut feed = app.world().resource::<HudFeed>().clone();
    let unit = &mut Arc::make_mut(&mut feed.view).units[0];
    unit.crew = Some([Fixed::from_int(73), Fixed::from_int(150)]);
    unit.def = db.key("fx_cruiser_no_officers").unwrap();
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    assert_eq!(label(&mut app, "infoCrewNum").2, Visibility::Inherited);
    assert_eq!(icon(&mut app, "infoCrewNumIcon").1, Visibility::Inherited);
    assert_eq!(label(&mut app, "infoOfficers").2, Visibility::Hidden);
    assert_eq!(icon(&mut app, "infoOfficersIcon").1, Visibility::Hidden);
}
