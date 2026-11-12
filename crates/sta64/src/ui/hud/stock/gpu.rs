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
use sta_sim::{Fixed, PlayerId, TeamId, WorldSource};
use sta_sim::setup::{GameSetup, GameOptions, PlayerSetup, Control};

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

fn feed(vfs: &sta_vfs::Vfs, options: GameOptions) -> HudFeed {
    // Exercise the real mounted content -> shared starting sim -> HUD path.
    // Fake ship/player values cannot expose a lost crew/officer binding.
    let (index, _) = sta_content::OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).expect("installed content");
    let (races, _) = sta_content::Races::load(vfs, &index);
    let race = races.get_playable("federation")
        .or_else(|| races.get_playable("unitedfederation"))
        .or_else(|| races.playable().first().copied()).expect("playable capture faction");
    let mut map = sta_content::fake::two_bases();
    map.objects.clear();
    let db = Arc::new(db);
    let setup = GameSetup {
        seed: 71, map: Arc::new(map), content: db.clone(), options,
        players: vec![PlayerSetup { id: PlayerId(0), team: TeamId(0), race: race.name.clone(), control: Control::Human }],
    };
    let (sim, warnings) = sta_sim::Sim::from_setup_with_starts(&setup, &races);
    eprintln!("installed HUD start warnings: {}", warnings.len());
    let view = sim.view();
    let selected = view.units.iter().find(|u| u.owner == PlayerId(0)).expect("authored start unit").id;
    HudFeed { view, player: PlayerId(0), selected: vec![selected], content: Some(db) }
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

fn capture_one(vfs: sta_vfs::Vfs, profile: UiProfile, size: UVec2, cfg: &str, label: &str, options: GameOptions) {
    let layout = crate::game::gui_layout(&vfs, cfg).unwrap_or_else(|| panic!("installed {cfg} missing"));
    let stock = StockLayout::new(&layout, Vec2::new(1080.0 * size.x as f32 / size.y as f32, 1080.0));
    let resource = stock.pieces("resourcePanel");
    let minimap = stock.pieces("minimapBackgroundPanel");
    let info = stock.pieces("infoMiddleBackgroundPanel");
    assert!(
        !resource.is_empty() || !minimap.is_empty() || !info.is_empty(),
        "{label}: CFG has no authored background pieces to check"
    );
    let hud_feed = feed(&vfs, options);
    let player = hud_feed.view.player(hud_feed.player).expect("capture player");
    let whole = |value: Fixed| value.to_bits() / Fixed::ONE.to_bits();
    let crew_text = player.crew.map(|v| whole(v).to_string()).unwrap_or_default();
    let officer_text = format!("{}/{}", whole(player.officers[0]), whole(player.officers[1]));
    let selection = hud_feed.selected.clone();
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
        .insert_resource(Selection(selection))
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
    for (id, flag, expected) in [(0, "noCrew", crew_text), (1, "noOfficers", officer_text)] {
        let visible = stock.number(flag).is_none_or(|v| v == 0.0) && stock.rect(&format!("resource_{id}")).is_some();
        let actual = app.world_mut().query::<(&super::info::StockText, &crate::ui::sprite_font::SpriteText)>()
            .iter(app.world()).find_map(|(kind,text)| matches!(kind, super::info::StockText::Resource(n) if *n == id).then(|| text.text.clone()));
        assert_eq!(actual.as_deref(), visible.then_some(expected.as_str()), "{label} CFG personnel resource_{id}");
        eprintln!("{label}: resource_{id} visible={visible} expected={expected} actual={actual:?}");
    }
    // Check the final text component too: source-label tests alone cannot
    // catch a tint lost on the way to Bevy's actual renderer.
    let defaults = [
        Color::srgb(0.0, 1.0, 0.0), Color::WHITE,
        Color::srgb(0.6, 0.9, 1.0), Color::srgb(1.0, 1.0, 0.0),
        Color::srgb(0.8, 0.8, 0.8), Color::srgb(1.0, 0.0, 0.0),
    ];
    for (kind, text, children) in app.world_mut()
        .query::<(&super::info::StockText, &crate::ui::sprite_font::SpriteText, &Children)>()
        .iter(app.world())
    {
        let super::info::StockText::Resource(id) = *kind else { continue };
        let expected = stock.colour(&format!("resource_{id}_color")).unwrap_or(defaults[id]);
        assert_eq!(text.color, expected, "{label}: resource_{id} CFG/default tint");
        let rendered = children.iter().filter_map(|child| app.world().get::<TextColor>(child)).collect::<Vec<_>>();
        assert!(!rendered.is_empty(), "{label}: resource_{id} has no resolved text");
        assert!(rendered.iter().all(|colour| colour.0 == expected), "{label}: resource_{id} lost its rendered tint");
        eprintln!("{label}: resource_{id} rendered colour={expected:?}");
    }

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
    // Classic has no colour overrides. Guard the visible green/cyan/yellow
    // numerals in the real framebuffer, rather than just checking ECS state.
    if profile == UiProfile::Armada2 {
        let area = stock.panel("resource").unwrap();
        let pixel_scale = size.y as f32 / 1080.0;
        for id in [0, 2, 3] {
            let rect = stock.at(Vec2::new(area.x, area.y), &format!("resource_{id}")).unwrap();
            let rgb = stock.colour(&format!("resource_{id}_color")).unwrap_or(defaults[id]).to_srgba();
            let expected = [rgb.red, rgb.green, rgb.blue].map(|v| (v * 255.0).round() as i16);
            let mut matching = 0;
            for y in (rect.y * pixel_scale).max(0.0) as u32..((rect.y + rect.h) * pixel_scale).min(size.y as f32) as u32 {
                for x in (rect.x * pixel_scale).max(0.0) as u32..((rect.x + rect.w) * pixel_scale).min(size.x as f32) as u32 {
                    let pixel = rgba.get_pixel(x, y);
                    if (0..3).all(|channel| (pixel[channel] as i16 - expected[channel]).abs() <= 5) {
                        matching += 1;
                    }
                }
            }
            assert!(matching >= 2, "{label}: resource_{id} has no visible {expected:?} text pixels");
            eprintln!("{label}: resource_{id} colour-matched text pixels={matching}");
        }
    }
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
        capture_one(vfs, profile, size, cfg, &format!("{mode}-{ratio}"), GameOptions::default());
    }
    if profile == UiProfile::Armada2 {
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
        let mut options = GameOptions::default();
        options.starting_resources = sta_content::StartingResources::Lots;
        options.officer_limit = 1;
        capture_one(vfs, profile, UVec2::new(1280,720), "gui_fed.cfg", "a2-lots-restricted-16x9", options);
    }
}
