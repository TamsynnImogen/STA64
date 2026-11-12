//! Opt-in windowless GPU proof for installed stock CFG, SPR and texture assets.
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::ui::{UiScale, UiTargetCamera};
use bevy::window::ExitCondition;
use sta_content::ContentDb;
use sta_sim::fake::ScriptedSource;
use sta_sim::view::PlayerView;
use sta_sim::{EntityId, Fixed, PlayerId, TeamId, WorldSource};

use super::super::{ExternalFeed, HudConfig, HudFeed, HudPlugin, panels::HudRoot};
use super::layout::StockLayout;
use crate::app::{GameSet, configure_sets};
use crate::sim_link::Selection;
use crate::ui::gui_sprites::{GuiButton, GuiImage, GuiSkin};
use crate::ui::profile::UiProfile;
use crate::ui::radar::{RadarFeed, RadarRoot, RadarSquare};
use crate::viewer::Mounted;

#[derive(Resource)]
struct CaptureCamera(Entity);

#[allow(clippy::type_complexity)]
fn aim_roots(
    mut commands: Commands,
    camera: Res<CaptureCamera>,
    roots: Query<Entity, (Or<(With<HudRoot>, With<RadarRoot>)>, Without<UiTargetCamera>)>,
) {
    for root in &roots {
        commands.entity(root).insert(UiTargetCamera(camera.0));
    }
}

fn feed(vfs: &sta_vfs::Vfs, profile: UiProfile) -> HudFeed {
    let fake = sta_content::fake::db();
    let model = match profile {
        UiProfile::Armada2 => "fcruise1",
        UiProfile::FleetOps => "fed_akira",
        UiProfile::Lcars => unreachable!(),
    };
    let found = sta_assets::find_model(vfs, model).is_some();
    eprintln!(
        "stock GPU model {model}: {}",
        if found { "mounted" } else { "unavailable" }
    );
    let mut builder = ContentDb::builder()
        .profile(fake.profile())
        .features(fake.features().clone());
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    for unit in fake.units() {
        let mut unit = unit.clone();
        if found && unit.odf == "fx_cruiser" {
            unit.model = model.to_owned();
        }
        builder = builder.unit(unit);
    }
    let db = Arc::new(builder.build().expect("fake content with installed model name"));
    let mut source = ScriptedSource::from_json5(
        "{ units: [{ id: 1, def: 'fx_cruiser', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] }] }",
        &db,
    )
    .expect("selected fake cruiser");
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
        content: Some(db),
    }
}

fn resolved_pieces(app: &mut App, pieces: &[super::layout::Piece]) -> usize {
    app.world_mut()
        .query::<(&GuiImage, &ImageNode)>()
        .iter(app.world())
        .filter(|(image, _)| {
            pieces.iter().any(|piece| {
                image.frame == piece.frame && image.names.iter().any(|n| n.eq_ignore_ascii_case(&piece.name))
            })
        })
        .count()
}

fn capture_one(vfs: sta_vfs::Vfs, profile: UiProfile, size: UVec2, cfg: &str, label: &str) {
    let layout = crate::game::gui_layout(&vfs, cfg).unwrap_or_else(|| panic!("installed {cfg} missing"));
    let stock = StockLayout::new(&layout, Vec2::new(1080.0 * size.x as f32 / size.y as f32, 1080.0));
    let resource = stock.pieces("resourcePanel");
    let minimap = stock.pieces("minimapBackgroundPanel");
    let info = stock.pieces("infoMiddleBackgroundPanel");
    assert!(
        !resource.is_empty() || !minimap.is_empty() || !info.is_empty(),
        "{label}: CFG has no authored background pieces to check"
    );
    let hud_feed = feed(&vfs, profile);
    let radar_feed = RadarFeed {
        view: hud_feed.view.clone(),
        player: hud_feed.player,
        selected: hud_feed.selected.clone(),
        content: hud_feed.content.clone(),
        camera: None,
    };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    configure_sets(&mut app);
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(profile)
        .insert_resource(profile.theme())
        .insert_resource(Mounted(vfs))
        .insert_resource(GuiSkin {
            table: Some("gui_federation.spr".into()),
        })
        .insert_resource(HudConfig {
            gui: Some(layout),
            aspect: size.x as f32 / size.y as f32,
            fit_scale: false,
            log: false,
        })
        .insert_resource(UiScale(size.y as f32 / 1080.0))
        .insert_resource(hud_feed)
        .insert_resource(radar_feed)
        .insert_resource(ExternalFeed)
        .insert_resource(Selection(vec![EntityId(1)]))
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(HudPlugin)
        .add_systems(Update, aim_roots.before(GameSet::Ui));

    let mut image = Image::new_target_texture(
        size.x,
        size.y,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera {
                order: 1,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderTarget::from(target.clone()),
            Msaa::Off,
        ))
        .id();
    app.insert_resource(CaptureCamera(camera));
    app.finish();
    app.cleanup();
    for _ in 0..16 {
        app.update();
    }

    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| {
            app.get_sub_app(bevy::render::RenderApp).and_then(|r| {
                r.world()
                    .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
                    .cloned()
            })
        });
    let Some(adapter) = adapter else {
        eprintln!("SKIP {label}: no render adapter");
        return;
    };
    eprintln!("{label} GPU adapter: {}", adapter.name);
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP {label}: hardware adapter unavailable");
        return;
    }

    let roots = app
        .world_mut()
        .query_filtered::<&UiTargetCamera, With<HudRoot>>()
        .iter(app.world())
        .map(|target| target.0)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 1, "{label}: one HUD root");
    assert_eq!(roots[0], camera, "{label}: HUD targets capture camera");
    let radar_roots = app
        .world_mut()
        .query_filtered::<&UiTargetCamera, With<RadarRoot>>()
        .iter(app.world())
        .map(|target| target.0)
        .collect::<Vec<_>>();
    assert_eq!(radar_roots.len(), 1, "{label}: one radar root");
    assert_eq!(radar_roots[0], camera, "{label}: radar targets capture camera");
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<RadarSquare>>()
            .iter(app.world())
            .count(),
        1
    );
    for (name, pieces) in [("resource", &resource), ("minimap", &minimap), ("info", &info)] {
        if !pieces.is_empty() {
            assert!(
                resolved_pieces(&mut app, pieces) > 0,
                "{label}: no {name} atlas art resolved"
            );
        }
    }
    let commands = app
        .world_mut()
        .query::<(&GuiButton, &ImageNode)>()
        .iter(app.world())
        .count();
    assert!(commands > 0, "{label}: no command button art resolved");
    let resources = app
        .world_mut()
        .query::<&super::info::StockText>()
        .iter(app.world())
        .filter(|text| matches!(text, super::info::StockText::Resource(_)))
        .count();
    assert!(resources > 0, "{label}: no live resource text");

    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut()
        .spawn(Screenshot::image(target))
        .observe(move |shot: On<ScreenshotCaptured>| {
            output.lock().unwrap().push(shot.image.clone());
        });
    for _ in 0..60 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let shot = frames
        .lock()
        .unwrap()
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("{label}: no GPU capture"));
    let dynamic = shot.try_into_dynamic().expect("GPU image conversion");
    let rgba = dynamic.to_rgba8();
    // A2's first two resource atlas pieces meet at canvas x=256. On a wide
    // viewport Auto letterboxes both pieces and leaves a black gap here; native
    // independent-axis drawing must fill this authored join.
    if profile == UiProfile::Armada2 && size.x as f32 / size.y as f32 > 1.5 {
        let x = (256.0 * size.x as f32 / stock.canvas.x).round() as u32;
        let y = (38.0 * size.y as f32 / stock.canvas.y).round() as u32;
        for x in x.saturating_sub(3)..=(x + 3).min(size.x - 1) {
            let pixel = rgba.get_pixel(x, y.min(size.y - 1));
            assert!(
                pixel[0].max(pixel[1]).max(pixel[2]) > 20,
                "{label}: black gap at resource atlas join ({x},{y})"
            );
        }
    }
    let coloured = rgba.pixels().filter(|p| p[0] > 20 || p[1] > 20 || p[2] > 20).count();
    eprintln!("{label}: {commands} command images, {resources} resource fields, {coloured} nonblack pixels");
    assert!(coloured > 500, "{label}: HUD did not visibly render");
    if let Ok(dir) = std::env::var("STA64_HUD_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        dynamic
            .save(std::path::Path::new(&dir).join(format!("stock-{label}.png")))
            .unwrap();
    }
}

#[test]
fn installed_stock_hud_renders_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") || std::env::var("STA64_HUD_TESTS").as_deref() != Ok("1")
    {
        eprintln!(
            "SKIP: set STA64_GPU_TESTS=1 STA64_HUD_TESTS=1 STA64_TEST_INSTALL STA64_HUD_PROFILE=a2|fo and optional STA64_TEST_MOD"
        );
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let profile = match std::env::var("STA64_HUD_PROFILE").as_deref() {
        Ok("a2") => UiProfile::Armada2,
        Ok("fo") => UiProfile::FleetOps,
        _ => panic!("set STA64_HUD_PROFILE=a2 or fo for the mounted install"),
    };
    for (size, ratio) in [(UVec2::new(1024, 768), "4x3"), (UVec2::new(1280, 720), "16x9")] {
        let cfg = match (profile, ratio) {
            (UiProfile::Armada2, _) => "gui_fed.cfg",
            (UiProfile::FleetOps, "16x9") => "gui_federation16x9.cfg",
            (UiProfile::FleetOps, _) => "gui_federation.cfg",
            _ => unreachable!(),
        };
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref())
            .expect("installed stock VFS");
        let mode = if profile == UiProfile::FleetOps { "fo4" } else { "a2" };
        capture_one(vfs, profile, size, cfg, &format!("{mode}-{ratio}"));
    }
}
