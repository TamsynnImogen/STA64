//! Install-gated checks of the stock briefing panel on the installed games' cfg and sprites, and opt-in
//! windowless frames of it (`STA64_GPU_TESTS=1`; `STA64_UI_CAPTURE_DIR` keeps the PNGs, which are game art and
//! never committed). Installs: `STA64_TEST_A1_INSTALL`, `STA64_TEST_A2_STOCK_INSTALL`,
//! `STA64_TEST_FO327_INSTALL`, `STA64_TEST_ROOTS_INSTALL`; each is skipped when unset.

use std::path::Path;
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::{RenderPlugin, view::screenshot::{Screenshot, ScreenshotCaptured}};
use bevy::ui::UiScale;
use bevy::window::ExitCondition;
use sta_mission::api::{Objective, ObjectiveBriefing, ObjectiveState};

use super::gui_sprites::{GuiSkin, GuiSprites};
use super::hud::HudConfig;
use super::objectives_panel::load_art;
use super::presentation::{ObjectivesView, PresentationPlugin};
use super::profile::UiProfile;
use crate::app::{GameSet, configure_sets};
use crate::viewer::Mounted;

/// (label, env var, profile, cfg for 4:3, cfg for 16:9, whether the stock art is expected).
const GAMES: [(&str, &str, UiProfile, &str, &str, bool); 4] = [
    ("a1", "STA64_TEST_A1_INSTALL", UiProfile::Armada1, "gui_fed.cfg", "gui_fed.cfg", true),
    ("a2", "STA64_TEST_A2_STOCK_INSTALL", UiProfile::Armada2, "gui_fed.cfg", "gui_fed.cfg", true),
    ("fo327", "STA64_TEST_FO327_INSTALL", UiProfile::FleetOps, "gui_federation4x3.cfg", "gui_federation16x9.cfg", true),
    // Its placement is not established: not read, the panel keeps its own look (TODO.md).
    ("roots", "STA64_TEST_ROOTS_INSTALL", UiProfile::FleetOps, "gui_federation4x3.cfg", "gui_federation16x9.cfg", false),
];

fn mount(var: &str) -> Option<sta_vfs::Vfs> {
    let Ok(root) = std::env::var(var) else {
        eprintln!("SKIP: set {var} to the install");
        return None;
    };
    Some(sta_vfs::Vfs::mount_install(Path::new(&root), None).unwrap_or_else(|e| panic!("{var}: {e:?}")))
}

#[test]
fn the_installed_games_cfg_and_sprites_give_the_panel_art_where_the_rule_is_established() {
    for (label, var, _, cfg43, cfg169, expect) in GAMES {
        let Some(vfs) = mount(var) else { continue };
        for cfg in [cfg43, cfg169] {
            let gui = crate::game::gui_layout(&vfs, cfg).unwrap_or_else(|| panic!("{label}: {cfg} missing"));
            let mut images = Assets::<Image>::default();
            let mut cache = GuiSprites::default();
            let art = load_art(&gui, &vfs, &mut images, &mut cache);
            match (expect, art) {
                (true, Ok(a)) => assert!(!a.pieces.is_empty(), "{label} {cfg}"),
                (true, Err(e)) => panic!("{label} {cfg}: {}", e.why),
                (false, Ok(_)) => panic!("{label} {cfg}: the rule for this cfg is not established, yet its art was read"),
                (false, Err(e)) => assert!(!e.transient && e.why.contains("tile more than"), "{label} {cfg}: {}", e.why),
            }
        }
    }
}

fn authored_view(v: &mut ObjectivesView) {
    let goals = ["Reach the beacon", "Build a mining station and hold it until the convoy has passed through", "Keep the flagship alive"];
    let obj: Vec<Objective> = goals
        .iter()
        .enumerate()
        .map(|(i, t)| Objective { id: i as u32 + 1, title: (*t).into(), description: String::new(), state: if i == 1 { ObjectiveState::Complete } else { ObjectiveState::Active } })
        .collect();
    v.fill(
        &obj,
        Some(&ObjectiveBriefing {
            file: "authored".into(),
            title: "Test mission".into(),
            text: "This briefing was written for the test.\n\nIt has two short paragraphs, so the layout of the text area shows.".into(),
            notes: "HINT: this hint was written for the test.".into(),
        }),
    );
    v.visible = true;
}

/// Draws the panel with `cfg` of the mounted install on a window of `size`; with `strip_art` the cfg's piece keys are
/// removed first, which is the panel STA64 drew before the stock art (the fallback). Returns the number of
/// pictures drawn.
fn capture(vfs: sta_vfs::Vfs, label: &str, cfg: &str, profile: UiProfile, size: UVec2, strip_art: bool) -> usize {
    let mut gui = crate::game::gui_layout(&vfs, cfg).unwrap_or_else(|| panic!("{label}: {cfg} missing"));
    if strip_art {
        gui.rects.retain(|k, _| !k.to_ascii_lowercase().starts_with("objectivesbackgroundpanel_"));
        for (_, p) in gui.panels.iter_mut() {
            p.pieces.clear();
        }
    }
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::gilrs::GilrsPlugin>()
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    configure_sets(&mut app);
    app.insert_resource(profile)
        .insert_resource(profile.theme())
        .insert_resource(Mounted(vfs))
        .insert_resource(GuiSkin { table: Some("gui_federation.spr".into()) })
        .insert_resource(HudConfig { gui: Some(gui), aspect: size.x as f32 / size.y as f32, fit_scale: false, ..default() })
        .insert_resource(UiScale(size.y as f32 / 1080.0))
        .init_resource::<GuiSprites>()
        .add_plugins(super::theme::ThemePlugin)
        .add_plugins(PresentationPlugin)
        .add_systems(Update, super::gui_sprites::paint.in_set(GameSet::Ui).after(super::theme::WidgetPaint));
    let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    // A grey stand-in for the map under the panel, so the panel's own translucency shows.
    app.world_mut().spawn((Camera2d, bevy::ui::IsDefaultUiCamera, Camera { clear_color: ClearColorConfig::Custom(Color::srgb(0.2, 0.25, 0.3)), ..default() }, RenderTarget::from(target.clone()), Msaa::Off));
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    authored_view(&mut app.world_mut().resource_mut::<ObjectivesView>());
    for _ in 0..40 {
        app.update();
    }
    // The cfg's pieces; the OK button's sprite is a button.
    let roots: Vec<Entity> = app.world_mut().query_filtered::<Entity, With<super::presentation::PanelRoot>>().iter(app.world()).collect();
    let pictures = app.world_mut().query::<(&ImageNode, &ChildOf)>().iter(app.world()).filter(|(_, p)| roots.contains(&p.parent())).count();
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
        output.lock().unwrap().push(shot.image.clone());
    });
    for _ in 0..90 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let image = frames.lock().unwrap().first().cloned().expect("panel GPU capture");
    let rgba = image.try_into_dynamic().expect("panel GPU image").to_rgba8();
    let lit = rgba.pixels().filter(|p| p[0] > 120 || p[1] > 120 || p[2] > 120).count();
    assert!(lit > 500, "{label}: no visible panel text ({lit} bright pixels)");
    if let Ok(dir) = std::env::var("STA64_UI_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let which = if strip_art { "before" } else { "after" };
        rgba.save(Path::new(&dir).join(format!("objectives-{label}-{}x{}-{which}.png", size.x, size.y))).unwrap();
    }
    pictures
}

#[test]
fn the_panel_renders_without_a_window_before_and_after() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 (and STA64_UI_CAPTURE_DIR to keep the frames)");
        return;
    }
    for (label, var, profile, cfg43, cfg169, expect) in GAMES {
        for (size, cfg) in [(UVec2::new(1280, 960), cfg43), (UVec2::new(1920, 1080), cfg169)] {
            let Some(vfs) = mount(var) else { continue };
            let before = capture(vfs, label, cfg, profile, size, true);
            assert_eq!(before, 0, "{label}: the fallback draws no pictures");
            let vfs = mount(var).unwrap();
            let after = capture(vfs, label, cfg, profile, size, false);
            assert_eq!(after > 0, expect, "{label} {cfg}: {after} pictures");
        }
    }
}
