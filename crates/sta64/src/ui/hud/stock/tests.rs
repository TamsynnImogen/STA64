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
        content: Some(db.clone()),
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
        let variant = layout::info_variant(&info);
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
    let cfg = inherited_config(
        "infoSingleNameTextColor = 0 1 1 1\ninfoSingleClassTextColor = 1 1 0 1\n",
        "nameTextColor = 1 0 0 1\nclassTextColor = 0 0 1 1\ninfoTextColor = 1 0 1 1\n",
    );
    for build_count in [0, 1] {
        let mut app = app_with_content_and_config(content_with_build_count(build_count), cfg.clone(), UiProfile::FleetOps);
        let (name_key, class_key) = if build_count == 0 {
            ("infoSingleNameTextArea", "infoSingleClassTextArea")
        } else {
            ("infoBuildName", "infoBuildClass")
        };
        assert_eq!(stock_colour_at(&mut app, name_key, false), Color::srgba(0.0, 1.0, 1.0, 1.0));
        assert_eq!(stock_colour_at(&mut app, class_key, false), Color::srgba(1.0, 1.0, 0.0, 1.0));
    }

    let cfg = inherited_config("", "nameTextColor = 1 0 0 1\nclassTextColor = 0 0 1 1\ninfoTextColor = 1 0 1 1\n");
    for build_count in [0, 1] {
        let mut app = app_with_content_and_config(content_with_build_count(build_count), cfg.clone(), UiProfile::FleetOps);
        let (name_key, class_key) = if build_count == 0 {
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
    view.units.push(other);
    feed.view = Arc::new(view);
    *app.world_mut().resource_mut::<HudFeed>() = feed;
    app.update();
    app
}

#[test]
fn bare_status_values_and_crew_change_colour_with_live_pools() {
    use infobar::InfoBarField::{Hull, Shields, Crew};
    let mut app = populated_info_bar(false, "systemIconHealthyColor=0 1 0\nsystemIconLowColor=1 1 0\nsystemIconCriticalColor=1 0 0\n");
    assert_eq!(info_bar_label(&mut app, Hull).unwrap().1, "100");
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
        (Hull, "50", Color::srgb(1.0, 1.0, 0.0)),
        (Shields, "25", Color::srgb(1.0, 0.0, 0.0)),
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
fn hovered_info_bar_keeps_selected_card_and_updates_real_pools_in_place() {
    use infobar::InfoBarField::{Name, Hull, Shields, Crew, Energy};
    let mut app = populated_info_bar(true, "hull_color = 0.2 0.4 0.6 0.8\ncrew_amt_color = 0 1 1 1\nshipNameColor = 1 0 1 1\n");
    let fields = [Name, Hull, Shields, Crew, Energy];
    let ids: Vec<_> = fields.iter().map(|&field| info_bar_label(&mut app, field).unwrap().0).collect();
    assert_eq!(info_bar_label(&mut app, Name).unwrap().1, "USS Test");
    app.world_mut().resource_mut::<crate::input::hover::HoveredUnit>().0 = Some(EntityId(2));
    app.update();
    for (field, expected) in [(Name,"Construction Ship"), (Hull,"50"), (Shields,"25"), (Crew,"73"), (Energy,"3")] {
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
    assert_eq!(info_bar_label(&mut app, Name).unwrap().1, "USS Test");
    assert_eq!(info_bar_label(&mut app, Crew).unwrap().1, "150");
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

#[test]
fn middle_info_bar_and_zero_area_fields_follow_independent_cfg_axes() {
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
        .unwrap()
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
fn numeric_values_patch_existing_stock_entities() {
    let mut app = app();
    let resources = |app: &mut App| -> Vec<(Entity, usize, String)> {
        app.world_mut()
            .query::<(Entity, &info::StockText, &SpriteText)>()
            .iter(app.world())
            .map(|(e, kind, text)| {
                let info::StockText::Resource(id) = kind;
                (e, *id, text.text.clone())
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
    assert_eq!(updated.text, "60");
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
    for (key, menu) in [("move", "orders"), ("harvest", "unavailable")] {
        native.commands.insert(
            key.into(),
            NativeCommand {
                odf: format!("{key}.odf"),
                command: key.into(),
                button: key.into(),
                preferred: None,
                menu: Some(menu.into()),
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
    assert!(grouped.iter().any(|b| b.act == Act::Target(TargetMode::Move)));
    let move_button = grouped.iter().find(|b| b.act == Act::Target(TargetMode::Move)).unwrap();
    assert_eq!(move_button.tip, "GUI_MOVE_TOOLTIP");
    assert_eq!(move_button.verbose.as_deref(), Some("GUI_MOVE_VERBOSE"));
    assert!(!grouped.iter().any(|b| b.act == Act::Target(TargetMode::Mine)));
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
    // Canvas 640x480 on the 1920x1080 test viewport (factor 2.25, x + 240): dilithium panel (300,2) 80x14 -> x 915..1095.
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
        // The same condition input uses: a pointer in a margin leaves the map free.
        let in_margin = crate::ui::hud::HudPointer(state.solid.iter().any(|r| r.contains(Vec2::new(5.0, 900.0))));
        assert_eq!(in_margin, crate::ui::hud::HudPointer(false));
    }
}
