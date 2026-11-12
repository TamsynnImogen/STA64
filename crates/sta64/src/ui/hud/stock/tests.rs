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
    view.players = vec![PlayerView {
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
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    configure_sets(&mut app);
    app.insert_resource(UiProfile::Armada2)
        .insert_resource(config())
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
            .filter_map(|(e, kind, text)| match kind {
                info::StockText::Resource(id) => Some((e, *id, text.text.clone())),
                _ => None,
            })
            .collect()
    };
    let old = resources(&mut app);
    assert!(old.iter().any(|(_, id, text)| *id == 2 && text == "3000"));
    assert!(old.iter().any(|(_, id, text)| *id == 4 && text == "700"));
    let hull = app
        .world_mut()
        .query::<(Entity, &info::StockText, &SpriteText)>()
        .iter(app.world())
        .find(|(_, kind, _)| matches!(kind, info::StockText::Hull))
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
    assert_eq!(updated.text, "60/100");
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
                tooltip: None,
                verbose: None,
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
    assert!(!grouped.iter().any(|b| b.act == Act::Target(TargetMode::Mine)));
}

#[test]
fn palette_cells_and_input_regions_use_cfg_geometry() {
    let app = app();
    let state = app.world().resource::<StockState>();
    let layout = state.layout.as_ref().unwrap();
    let palette = layout.palette().unwrap();
    let cells = visible_cells(&palette, layout.screen);
    assert_eq!(cells.len(), 9);
    assert_eq!((cells[0].x, cells[0].y), (1200.0, 720.0));
    assert_eq!((cells[1].x, cells[1].y), (1302.0, 720.0));
    assert_eq!((cells[3].x, cells[3].y), (1200.0, 679.0));
    assert!(state.solid.iter().any(|r| r.contains(Vec2::new(1210.0, 730.0))));
    assert!(state.solid.iter().any(|r| r.contains(Vec2::new(5.0, 5.0))));
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
