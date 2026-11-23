//! Installed, windowless frames for the stock production and cargo status overlay.
//! Opt in with STA64_GPU_TESTS=1 STA64_STATUS_TESTS=1 and an installed GUI CFG.

use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::ui::{UiScale, UiTargetCamera};
use bevy::window::ExitCondition;
use sta_sim::economy::view::CargoView;
use sta_sim::production::view::{ProductionJobView, ProductionView};
use sta_sim::view::UnitFlags;
use sta_sim::{EntityId, Fixed, PlayerId, UnitView, WorldView};

use super::*;
use crate::ui::gui_sprites::{GuiSkin, GuiSprites};
use crate::ui::sprite_font::SpriteFontPlugin;
use crate::viewer::Mounted;

const SIZE: UVec2 = UVec2::new(1600, 1200);
const BUILDER: EntityId = EntityId(71);
const FREIGHTER: EntityId = EntityId(72);
const STATION: EntityId = EntityId(73);

fn unit(id: EntityId, def: sta_sim::ContentKey, progress: Option<Fixed>) -> UnitView {
    UnitView {
        id,
        def,
        owner: PlayerId(0),
        pos: sta_sim::Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(100), Fixed::from_int(100)],
        shields: [[Fixed::ZERO; 2]; 4],
        flags: UnitFlags::SELECTABLE,
        progress,
        crew: None,
        energy: None,
        autonomy: None,
    }
}

fn bar_width(app: &mut App, want: fn(&LiveValue) -> bool) -> f32 {
    let world = app.world_mut();
    let mut bars = world.query::<(&LiveValue, &Node)>();
    let widths: Vec<_> = bars
        .iter(world)
        .filter(|(kind, _)| want(kind))
        .map(|(_, node)| match node.width {
            Val::Px(width) => width,
            other => panic!("status bar has nonpixel width: {other:?}"),
        })
        .collect();
    assert_eq!(widths.len(), 1, "expected one visible status bar");
    widths[0]
}

fn capture(app: &mut App, target: &Handle<Image>, label: &str) -> Image {
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut()
        .spawn(Screenshot::image(target.clone()))
        .observe(move |shot: On<ScreenshotCaptured>| {
            output.lock().unwrap().push(shot.image.clone());
        });
    for _ in 0..60 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let image = frames
        .lock()
        .unwrap()
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("no status frame {label}"));
    if let Ok(dir) = std::env::var("STA64_STATUS_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        image
            .clone()
            .try_into_dynamic()
            .expect("status frame conversion")
            .save(std::path::Path::new(&dir).join(format!("status-{label}.png")))
            .unwrap();
    }
    image
}

fn changed_pixels(a: &Image, b: &Image) -> usize {
    let a = a
        .clone()
        .try_into_dynamic()
        .expect("first frame")
        .to_rgba8();
    let b = b
        .clone()
        .try_into_dynamic()
        .expect("second frame")
        .to_rgba8();
    assert_eq!(a.dimensions(), b.dimensions());
    a.pixels()
        .zip(b.pixels())
        .filter(|(a, b)| (0..3).any(|channel| a[channel].abs_diff(b[channel]) > 20))
        .count()
}

#[test]
fn installed_status_queue_and_cargo_render_and_clear() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1")
        || std::env::var("STA64_STATUS_TESTS").as_deref() != Ok("1")
    {
        eprintln!(
            "SKIP: set STA64_GPU_TESTS=1 STA64_STATUS_TESTS=1 STA64_TEST_INSTALL STA64_TEST_CFG"
        );
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD")
        .ok()
        .filter(|name| !name.is_empty());
    let cfg = std::env::var("STA64_TEST_CFG").expect("STA64_TEST_CFG");
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref())
        .expect("requested status test install");
    let gui =
        crate::game::gui_layout(&vfs, &cfg).unwrap_or_else(|| panic!("installed {cfg} missing"));
    let mut yard = sta_content::UnitDef::new("status_yard", "shipyard");
    yard.build
        .list
        .push(sta_content::DefRef::new("status_ship"));
    // Read native art from the opted-in install; the fixture ships remain
    // synthetic and no textures or SPR definitions are copied into the test.
    let models =
        std::env::var("STA64_STATUS_MODELS").unwrap_or_else(|_| "fcruise1,fdestroy,fscout".into());
    let models: Vec<_> = models.split(',').map(str::trim).collect();
    assert_eq!(
        models.len(),
        3,
        "STA64_STATUS_MODELS needs three model stems"
    );
    let queued_units: Vec<_> = [
        ("status_ship", "cruiser"),
        ("status_frigate", "frigate"),
        ("status_scout", "scout"),
    ]
    .into_iter()
    .zip(&models)
    .map(|((name, class), model)| {
        let mut unit = sta_content::UnitDef::new(name, class);
        unit.model = (*model).into();
        unit
    })
    .collect();
    let db = Arc::new(
        sta_content::ContentDb::builder()
            .unit(yard)
            .unit(sta_content::UnitDef::new("status_freighter", "freighter"))
            .unit(sta_content::UnitDef::new("status_station", "starbase"))
            .unit(queued_units[0].clone())
            .unit(queued_units[1].clone())
            .unit(queued_units[2].clone())
            .build()
            .unwrap(),
    );
    let feed = HudFeed {
        view: Arc::new(WorldView {
            units: vec![
                unit(BUILDER, db.key("status_yard").unwrap(), None),
                unit(FREIGHTER, db.key("status_freighter").unwrap(), None),
                unit(
                    STATION,
                    db.key("status_station").unwrap(),
                    Some(Fixed::from_f64(0.25)),
                ),
            ],
            ..Default::default()
        }),
        player: PlayerId(0),
        selected: vec![BUILDER],
        content: Some(db.clone()),
        production: Vec::new(),
        names: Default::default(),
    };
    let source = StatusFeed {
        production: vec![ProductionView {
            portal: None,
            research: None,
            officer_upgrades: None,
            builder: BUILDER,
            jobs: [
                ("status_ship", 0.4, true, true),
                ("status_frigate", 0.0, false, false),
                ("status_scout", 0.0, false, false),
            ]
            .into_iter()
            .map(|(name, progress, paid, started)| ProductionJobView {
                queue_id: None,
                def: db.key(name).unwrap(),
                progress: Fixed::from_f64(progress),
                paid,
                started,
            })
            .collect(),
            site: None,
        }],
        cargo: vec![CargoView {
            entity: FREIGHTER,
            amount: Fixed::ZERO,
            capacity: Fixed::from_int(100),
            resource: None,
        }],
            nodes: Vec::new(),
            systems: Vec::new(),
    };
    let profile =
        crate::ui::profile::resolve_profile(Some(&vfs), Default::default(), &Default::default());
    assert!(
        profile.is_stock(),
        "installed status test requires a stock HUD profile"
    );
    eprintln!("status mounted profile: {profile:?}; sheet: {cfg}");
    let aspect = SIZE.x as f32 / SIZE.y as f32;
    let config = HudConfig {
        gui: Some(gui.clone()),
        aspect,
        fit: Default::default(),
        fit_scale: false,
        log: false,
    };
    let mut stock_state = stock::StockState::default();
    stock_state.layout = Some(StockLayout::for_aspect(&gui, aspect));
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
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(profile)
        .insert_resource(profile.theme())
        .insert_resource(UiScale(SIZE.y as f32 / 1080.0))
        .insert_resource(Mounted(vfs))
        .insert_resource(GuiSkin {
            table: Some("gui_federation.spr".into()),
        })
        .init_resource::<GuiSprites>()
        .insert_resource(config)
        .insert_resource(HudLayoutRes(super::super::HudLayout::new(
            Some(&gui),
            aspect,
        )))
        .insert_resource(stock_state)
        .insert_resource(feed)
        .insert_resource(source)
        .init_resource::<StatusUiState>()
        .add_plugins(SpriteFontPlugin)
        .add_systems(
            Update,
            (refresh, update_values, crate::ui::gui_sprites::paint).chain(),
        );
    let mut target_image = Image::new_target_texture(
        SIZE.x,
        SIZE.y,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    target_image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(target_image);
    let camera = app
        .world_mut()
        .spawn((Camera2d, RenderTarget::from(target.clone()), Msaa::Off))
        .id();
    app.world_mut().spawn((
        HudRoot,
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        UiTargetCamera(camera),
    ));
    app.finish();
    app.cleanup();

    for _ in 0..4 {
        app.update();
    }
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| {
            app.get_sub_app(bevy::render::RenderApp).and_then(|render| {
                render
                    .world()
                    .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
                    .cloned()
            })
        })
        .expect("requested GPU test has no render adapter");
    eprintln!("status GPU adapter: {}", adapter.name);
    assert!(
        !crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name),
        "requested hardware status test selected software adapter {}",
        adapter.name
    );

    let mut frames = Vec::new();
    let mut queue_widths = Vec::new();
    let mut cargo_widths = Vec::new();
    let mut construction_widths = Vec::new();
    for stage in 0..10 {
        match stage {
            1 => {
                app.world_mut().resource_mut::<StatusFeed>().production[0].jobs[0].progress =
                    Fixed::from_f64(0.7)
            }
            2 => {
                app.world_mut()
                    .resource_mut::<StatusFeed>()
                    .production
                    .clear();
                app.world_mut().resource_mut::<HudFeed>().selected = vec![FREIGHTER];
            }
            3 => {
                let mut source = app.world_mut().resource_mut::<StatusFeed>();
                source.cargo[0].amount = Fixed::from_int(50);
                source.cargo[0].resource = Some(0);
            }
            4 => {
                app.world_mut().resource_mut::<StatusFeed>().cargo[0].amount = Fixed::from_int(100)
            }
            5 => {
                app.world_mut().resource_mut::<HudFeed>().selected = vec![STATION];
                *app.world_mut()
                    .resource_mut::<crate::ui::profile::UiProfile>() =
                    crate::ui::profile::UiProfile::Lcars;
                *app.world_mut().resource_mut::<Theme>() =
                    crate::ui::profile::UiProfile::Lcars.theme();
                app.world_mut().resource_mut::<HudConfig>().gui = None;
            }
            6 => {
                Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).units[2]
                    .progress = Some(Fixed::from_f64(0.7))
            }
            7 => {
                Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).units[2]
                    .progress = Some(Fixed::ONE)
            }
            8 => app.world_mut().resource_mut::<HudFeed>().selected = vec![FREIGHTER],
            9 => app.world_mut().resource_mut::<HudFeed>().selected.clear(),
            _ => {}
        }
        for _ in 0..12 {
            app.update();
        }
        let world = app.world_mut();
        let mut roots = world.query_filtered::<Entity, With<StatusRoot>>();
        assert_eq!(
            roots.iter(world).count(),
            usize::from(stage != 7 && stage != 9),
            "status root stage {stage}"
        );
        if stage == 0 || stage == 2 {
            let state = app.world().resource::<StatusUiState>();
            let shape = state.last.as_ref().expect("selected stock status shape");
            let layout = app.world().resource::<stock::StockState>();
            let layout = layout.layout.as_ref().unwrap();
            let variant = if stage == 0 { 2 } else { 1 };
            assert_eq!(
                stock::layout::info_variant(&model::info(app.world().resource::<HudFeed>())),
                variant
            );
            assert_eq!(shape.selected, if stage == 0 { BUILDER } else { FREIGHTER });
            assert!(shape.stock);
            let origin = layout.info_origin(variant);
            assert_eq!(shape.progress, layout.at(origin, "infoProgressBar"));
            assert_eq!(
                shape.cargo_bar,
                layout.at(origin, "infoSingleDilithiumBarArea")
            );
            if stage == 0 {
                let slots: Vec<_> = (0..10)
                    .filter_map(|i| {
                        layout
                            .at(origin, &format!("infoBuildQueue_{i}"))
                            .map(|r| (i, r))
                    })
                    .collect();
                assert_eq!(
                    shape.slots, slots,
                    "yard queue used the wrong stock panel origin"
                );
                assert_eq!(shape.queue.len(), 3);
            } else {
                assert!(
                    shape.cargo_bar.is_some() || shape.lcars_card.is_some(),
                    "cargo has neither native nor fallback geometry"
                );
            }
        }
        if stage <= 1 {
            let world = app.world_mut();
            let mut art = world.query::<(&GuiImage, Option<&ImageNode>)>();
            for model in &models {
                assert!(
                    art.iter(world).any(|(image, painted)| {
                        image.names.first() == Some(&format!("{model}w1")) && painted.is_some()
                    }),
                    "queued {model} has no painted native wireframe"
                );
            }
            assert!(
                art.iter(world)
                    .all(|(image, _)| { image.names.iter().all(|name| !name.starts_with("b_")) }),
                "queue must not use build-button artwork"
            );
            let native_progress = world
                .resource::<StatusUiState>()
                .last
                .as_ref()
                .unwrap()
                .progress
                .is_some();
            if native_progress {
                let layout = world.resource::<stock::StockState>();
                let sprite = layout
                    .layout
                    .as_ref()
                    .unwrap()
                    .name("infoSingleConstructionBar")
                    .unwrap_or("large_construction_bar")
                    .to_owned();
                assert!(
                    art.iter(world).any(|(image, painted)| {
                        image.names.first() == Some(&sprite) && painted.is_some()
                    }),
                    "native queue progress SPR did not resolve: {sprite}"
                );
            }
            queue_widths.push(bar_width(&mut app, |kind| {
                matches!(kind, LiveValue::QueueProgress(0, _))
            }));
        }
        if (2..=4).contains(&stage) {
            cargo_widths.push(bar_width(&mut app, |kind| {
                matches!(kind, LiveValue::Cargo(_))
            }));
        }
        if stage == 5 || stage == 6 {
            let shape = app
                .world()
                .resource::<StatusUiState>()
                .last
                .as_ref()
                .unwrap();
            assert!(!shape.stock && shape.construction && shape.lcars_card.is_some());
            construction_widths.push(bar_width(&mut app, |kind| {
                matches!(kind, LiveValue::Construction(_))
            }));
        }
        if stage == 7 || stage == 9 {
            let world = app.world_mut();
            let mut bars = world.query::<&LiveValue>();
            assert_eq!(
                bars.iter(world).count(),
                0,
                "stale bars after completion or deselection"
            );
        }
        if stage == 8 {
            assert!(bar_width(&mut app, |kind| matches!(kind, LiveValue::Cargo(_))) > 1.0);
        }
        frames.push(capture(
            &mut app,
            &target,
            &[
                "yard-queued-40",
                "yard-queued-70",
                "freighter-empty",
                "freighter-half",
                "freighter-full",
                "station-building-25",
                "station-building-70",
                "station-complete",
                "freighter-lcars-full",
                "deselected",
            ][stage],
        ));
    }
    assert!(queue_widths[1] > queue_widths[0] + 1.0);
    assert!(cargo_widths[0] <= 0.01);
    assert!(cargo_widths[1] > cargo_widths[0] + 1.0);
    assert!(cargo_widths[2] > cargo_widths[1] + 1.0);
    assert!(construction_widths[1] > construction_widths[0] + 1.0);
    for (index, pair) in frames.windows(2).enumerate() {
        let changed = changed_pixels(&pair[0], &pair[1]);
        eprintln!("status frame transition {index}: {changed} changed pixels");
        assert!(
            changed > 20,
            "status transition {index} did not visibly change"
        );
    }
}
