//! Real-GPU identity check of how sprite nodes reach the screen: the same invented scene drawn through
//! the reference path ([`super::tests::animate_reference`], `main`'s per-node `animate`) and through
//! the sprite material path of ADR 0009 ([`super::material`]: shared look materials, the colour in the
//! node's slot, the forked `sprite.wesl`), at a new time, camera place and fade every frame, from four
//! cameras. The two sets of frames must be byte-equal; a control run (reference against reference)
//! must be byte-equal too, which proves the harness itself is deterministic.
//!
//! The material path is the game's own: every node is adopted by [`SpriteMaterialPlugin`]'s observer
//! as it is spawned, [`animate`] writes slots and looks, and the game's `apply_fade` and `tint_ghost`
//! demote the cloaking unit's and the ghost's nodes to private materials (S4) and give the unit's back.
//! The reference path is the same app with adoption off, driven by `main`'s `animate`.
//!
//! The scene has: nebula-like puffs whose fades change every step while their look's colour cycles;
//! a blinking glow, blinking nodes on the flipbook sprite (one faded), flat sprites, a moon nimbus;
//! a co-located pair each of blended and of additive sprites and a coplanar overlap of cut-out ones; a cloaking unit and a placement ghost run through
//! the game's own `apply_fade` and `tint_ghost`; a sprite node spawned and one despawned mid-series;
//! and a group of puffs whose parent toggles between hidden and inherited (the Stock/Refit switch).
//! Needs `STA64_GPU_TESTS=1` and a hardware adapter; skips otherwise.
use super::tests::{BLENDS, NODES, TABLE, animate_reference, fade_of, material_of, table};
use bevy::core_pipeline::tonemapping::Tonemapping;
use super::*;
use super::material::{SpriteLooks, SpriteMaterial, SpriteMaterialPlugin, SpriteSlots, parity};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::render::storage::ShaderBuffer;
use crate::render::fx::specials::{UnitFade, apply_fade};
use crate::ui::hud::ghost::{Ghost, tint_ghost};
use sta_sim::ContentKey;
use crate::clock::{AnimClock, ClockMode};
use bevy::camera::RenderTarget;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};


fn shoot(app: &mut App, target: &Handle<Image>) -> Vec<u8> {
    let out = Arc::new(Mutex::new(None::<Image>));
    let sink = Arc::clone(&out);
    app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| {
        *sink.lock().unwrap() = Some(shot.image.clone());
    });
    for _ in 0..60 {
        app.update();
        if out.lock().unwrap().is_some() {
            break;
        }
    }
    let image = out.lock().unwrap().clone().expect("a capture");
    image.data.expect("pixels")
}

/// A small authored texture: a gradient with a hole, so blending and uv shifts show.
fn texture() -> Image {
    let mut data = Vec::new();
    for y in 0..16u32 {
        for x in 0..16u32 {
            let hole = (6..10).contains(&x) && (6..10).contains(&y);
            data.extend_from_slice(&[(x * 16) as u8, (y * 16) as u8, 200 - (x * 8) as u8, if hole { 0 } else { 255 - (y * 8) as u8 }]);
        }
    }
    Image::new(
        bevy::render::render_resource::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    )
}

/// Which animation system draws the nodes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Path {
    /// `main`'s per-node system.
    Reference,
    /// The sprite material path under test: nodes adopted onto their look's shared
    /// [`SpriteMaterial`], the colour in their slot, written by [`animate`].
    New,
    /// A deliberately broken path, a negative control: the reference system run in `Last`, after the
    /// assets' events of the frame, so its writes reach the screen one frame late. Must NOT be byte-equal.
    Late,
}

/// The camera: what the looks differ by in the post chain.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Camera {
    /// No tonemapping, no MSAA.
    Stock,
    /// Default tonemapping, 4x MSAA.
    Tonemapped4x,
    /// HDR, bloom, default tonemapping, 4x MSAA (the cinematic post chain's parts).
    HdrBloom,
    /// The game's scene camera: a depth prepass (cut-out looks are drawn in it too, through Bevy's
    /// own prepass shader), default tonemapping, 4x MSAA.
    DepthPrepass4x,
}

/// The sprite material's form on the adapter of the last series ([`super::material::mode_name`]).
static MODE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Slots of the nodes adopted before the series starts (48 puffs, the unit's and the ghost's four
/// each, six blend nodes, six board puffs, the flipbook look's two blinking nodes, the doomed one):
/// the buffer is full when the series begins, so the node spawned mid-series grows it (S1, growth
/// by doubling) between two captured frames.
const SLOTS_AT_START: u32 = 48 + 4 + 4 + 6 + 6 + BLINK_GLOWS.len() as u32 + 1;

/// Nodes that blink on the flipbook sprite (the puffs' look), with the fade each carries. While
/// they are hidden the look's uv moves on with its shown nodes, and a look's uv write marks its
/// hidden nodes changed as well, which `main`'s per-node path does not: the frames show whether
/// that reaches the picture when they come back.
const BLINK_GLOWS: [(Vec3, Option<f32>); 2] = [(Vec3::new(-8.0, 12.0, 6.0), None), (Vec3::new(14.0, -6.0, 8.0), Some(0.6))];

/// The cloaking unit's fade at each step: it cloaks and decloaks, then builds up (even steps set the
/// cloak, odd ones the build), so its fade changes at every step and ends at 1.
const UNIT_FADE: [f32; 12] = [1.0, 0.8, 0.5, 0.3, 0.3, 0.6, 1.0, 1.0, 0.7, 0.2, 0.5, 1.0];

/// How many of `nodes` draw through a private `StandardMaterial` (demoted, S4).
fn demoted(app: &App, nodes: &[Entity]) -> usize {
    nodes.iter().filter(|&&e| app.world().get::<MeshMaterial3d<StandardMaterial>>(e).is_some()).count()
}

/// Frames of the scene at 12 times and camera places.
fn series(path: Path, camera: Camera) -> Option<Vec<Vec<u8>>> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    app.insert_resource(ClearColor(Color::BLACK)).insert_resource(table()).insert_resource(AnimClock::new(ClockMode::Bench { step: 0.137 }, 0));
    // The clock moves on every update, as in the game. The cloak's and the ghost's own systems run
    // after the animation, as the plugins order them (they hand nodes material copies).
    fn tick(mut clock: ResMut<AnimClock>) {
        clock.advance_idle(0.0);
    }
    app.add_systems(Update, tick.before(animate_reference).before(animate));
    // The material path's plugin, slots and observers are in every path's app, adoption turned off
    // where the path is `main`'s, so the apps differ only in what the path adopts (an observer is an
    // entity: leaving it out of one app would also shift the scene's entity ids).
    app.add_plugins(SpriteMaterialPlugin);
    let slots = SpriteSlots::with_capacity(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(), SLOTS_AT_START);
    app.insert_resource(slots);
    app.world_mut().resource_mut::<SpriteLooks>().adopt = path == Path::New;
    match path {
        Path::New => app.add_systems(Update, (animate, apply_fade, tint_ghost, face_camera).chain()),
        Path::Reference => app.add_systems(Update, (animate_reference, apply_fade, tint_ghost, face_camera).chain()),
        Path::Late => app.add_systems(Update, (apply_fade, tint_ghost, face_camera).chain()).add_systems(Last, animate_reference),
    };
    let mut target = Image::new_target_texture(160, 120, TextureFormat::Rgba8UnormSrgb, None);
    target.asset_usage = bevy::asset::RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target);
    let tex = app.world_mut().resource_mut::<Assets<Image>>().add(texture());
    let cam = app
        .world_mut()
        .spawn((Camera3d::default(), RenderTarget::from(target.clone()), Transform::from_xyz(0.0, 0.0, 60.0).looking_at(Vec3::ZERO, Vec3::Y)))
        .id();
    match camera {
        Camera::Stock => app.world_mut().entity_mut(cam).insert((Tonemapping::None, Msaa::Off)),
        Camera::Tonemapped4x => app.world_mut().entity_mut(cam).insert(Msaa::Sample4),
        Camera::HdrBloom => app.world_mut().entity_mut(cam).insert((bevy::camera::Hdr, bevy::post_process::bloom::Bloom::NATURAL, Msaa::Sample4)),
        Camera::DepthPrepass4x => app.world_mut().entity_mut(cam).insert((DepthPrepass, Msaa::Sample4)),
    };
    let quad = app.world_mut().resource_mut::<Assets<Mesh>>().add(unit_quad());
    let t = table();
    let mut puffs = Vec::new();
    for i in 0..48usize {
        let node = NODES[i % NODES.len()];
        let (mut m, mut fx, _) = material_of(&t, node);
        m.base_color_texture = Some(tex.clone());
        let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m);
        fx.material = h.clone();
        let (x, y, z) = (((i * 7) % 11) as f32 * 9.0 - 45.0, ((i * 5) % 9) as f32 * 8.0 - 32.0, ((i * 3) % 7) as f32 * 4.0 - 12.0);
        let mut e = app.world_mut().spawn((Mesh3d(quad.clone()), MeshMaterial3d(h), Transform::from_xyz(x, y, z).with_scale(Vec3::new(14.0, 14.0, 1.0)), Visibility::default(), fx));
        // Puffs and glows are billboards, the rest flat; one of them is a moon's nimbus.
        if i % 2 == 0 {
            e.insert(Billboard);
        }
        if i == 5 {
            e.insert(crate::render::rock::MoonNimbus { on: true });
        }
        puffs.push(e.id());
    }
    // A cloaking unit and a placement ghost: the game's own systems give their nodes material copies
    // (on the material path, by demoting them from their looks). Same looks as the puffs above.
    let family = |app: &mut App, root: Entity, nodes: &[usize]| -> Vec<Entity> {
        nodes
            .iter()
            .map(|&i| {
                let (mut m, mut fx, _) = material_of(&t, NODES[i % NODES.len()]);
                m.base_color_texture = Some(tex.clone());
                let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m);
                fx.material = h.clone();
                let child = app
                    .world_mut()
                    .spawn((Mesh3d(quad.clone()), MeshMaterial3d(h), Transform::from_xyz(i as f32 * 5.0 - 8.0, 0.0, 0.0).with_scale(Vec3::new(12.0, 12.0, 1.0)), Visibility::default(), fx, Billboard))
                    .id();
                app.world_mut().entity_mut(root).add_child(child);
                child
            })
            .collect()
    };
    let unit = app.world_mut().spawn((UnitFade { cloak: 1.0, build: 1.0 }, Transform::from_xyz(-20.0, 20.0, 5.0), Visibility::default())).id();
    let unit_nodes = family(&mut app, unit, &[0, 1, 2, 3]);
    let ghost = app.world_mut().spawn((Ghost::pinned(ContentKey::NONE), Transform::from_xyz(18.0, -18.0, 5.0), Visibility::default())).id();
    let ghost_nodes = family(&mut app, ghost, &[0, 1, 2, 3]);
    // One node of each blend look placed on top of another of a different look (same distance from
    // the eye: a sort tie), and two cut-out sprites in one plane.
    let spawn = |app: &mut App, node: &str, at: Vec3, size: f32, billboard: bool, parent: Option<Entity>| {
        let (mut m, mut fx, _) = material_of(&t, node);
        m.base_color_texture = Some(tex.clone());
        let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m);
        fx.material = h.clone();
        let mut e = app.world_mut().spawn((Mesh3d(quad.clone()), MeshMaterial3d(h), Transform::from_translation(at).with_scale(Vec3::new(size, size, 1.0)), Visibility::default(), fx));
        if billboard {
            e.insert(Billboard);
        }
        if let Some(p) = parent {
            e.insert(ChildOf(p));
        }
        e.id()
    };
    spawn(&mut app, BLENDS[0], Vec3::new(-30.0, -28.0, -6.0), 16.0, true, None);
    spawn(&mut app, BLENDS[1], Vec3::new(-30.0, -28.0, -6.0), 16.0, true, None);
    spawn(&mut app, BLENDS[2], Vec3::new(28.0, 24.0, -6.0), 16.0, true, None);
    spawn(&mut app, BLENDS[3], Vec3::new(28.0, 24.0, -6.0), 16.0, true, None);
    spawn(&mut app, BLENDS[4], Vec3::new(2.0, -30.0, -8.0), 18.0, false, None);
    spawn(&mut app, BLENDS[5], Vec3::new(8.0, -30.0, -8.0), 18.0, false, None);
    // A group of puffs under one parent that toggles hidden and inherited, as the nebula boards do.
    let boards = app.world_mut().spawn((Transform::from_xyz(0.0, 30.0, -4.0), Visibility::default())).id();
    let board_nodes: Vec<Entity> = (0..6).map(|i| spawn(&mut app, NODES[i % 2], Vec3::new(i as f32 * 6.0 - 15.0, (i % 3) as f32 * 4.0, 0.0), 14.0, true, Some(boards))).collect();
    // Blinking nodes on the flipbook sprite, drawn through the puffs' look.
    let mut blink_nodes = Vec::new();
    for (at, fade) in BLINK_GLOWS {
        let e = spawn(&mut app, "blinkGlow", at, 12.0, true, None);
        blink_nodes.push(e);
        if let Some(k) = fade {
            app.world_mut().entity_mut(e).insert(SpriteFade(k));
        }
    }
    // A node spawned half way and one despawned half way.
    let doomed = spawn(&mut app, NODES[0], Vec3::new(-38.0, 10.0, 0.0), 14.0, true, None);
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    let adapter = app
        .get_sub_app(bevy::render::RenderApp)
        .and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
        .or_else(|| app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())?;
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: only a software adapter ({})", adapter.name);
        return None;
    }
    eprintln!("adapter: {}", adapter.name);
    // Which form of the sprite material this adapter gets (ADR 0009 S2: bindless where supported).
    let mode = app
        .get_sub_app(bevy::render::RenderApp)
        .and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderDevice>().map(super::material::mode_name))
        .or_else(|| app.world().get_resource::<bevy::render::renderer::RenderDevice>().map(super::material::mode_name))
        .expect("a render device to tell the sprite material's form by");
    eprintln!("sprite material: {mode}");
    let _ = MODE.set(mode);
    let places = [Vec3::new(0.0, 0.0, 60.0), Vec3::new(70.0, 10.0, 40.0), Vec3::new(-90.0, 30.0, 30.0), Vec3::new(10.0, 80.0, 20.0)];
    let mut frames = Vec::new();
    // Per step, how many of the unit's nodes drew through a private material.
    let mut unit_demoted = Vec::new();
    // Per step, whether each blinking node on the flipbook sprite was hidden when the frame was taken.
    let mut blink_hidden: Vec<Vec<bool>> = Vec::new();
    for step in 0..12usize {
        // The unit cloaks and decloaks, then builds up: its fade changes at every step.
        let k = UNIT_FADE[step];
        *app.world_mut().get_mut::<UnitFade>(unit).unwrap() = if step % 2 == 0 { UnitFade { cloak: k, build: 1.0 } } else { UnitFade { cloak: 1.0, build: k } };
        if step == 4 {
            spawn(&mut app, NODES[1], Vec3::new(36.0, -4.0, 2.0), 14.0, true, None);
        }
        if step == 6 {
            app.world_mut().despawn(doomed);
        }
        *app.world_mut().get_mut::<Visibility>(boards).unwrap() = if step % 3 == 1 { Visibility::Hidden } else { Visibility::Inherited };
        for (i, &e) in board_nodes.iter().enumerate() {
            match fade_of(i + 2, step) {
                Some(k) => {
                    app.world_mut().entity_mut(e).insert(SpriteFade(k));
                }
                None => {
                    app.world_mut().entity_mut(e).remove::<SpriteFade>();
                }
            }
        }
        // The camera turns about the scene, so puffs come into view and leave it.
        let at = places[step % places.len()];
        *app.world_mut().get_mut::<Transform>(cam).unwrap() = Transform::from_translation(at).looking_at(Vec3::new(0.0, 0.0, -10.0), Vec3::Y);
        for (i, &e) in puffs.iter().enumerate() {
            let mut en = app.world_mut().entity_mut(e);
            match fade_of(i, step) {
                Some(k) => {
                    en.insert(SpriteFade(k));
                }
                None => {
                    en.remove::<SpriteFade>();
                }
            }
        }
        frames.push(shoot(&mut app, &target));
        unit_demoted.push(demoted(&app, &unit_nodes));
        blink_hidden.push(blink_nodes.iter().map(|&e| app.world().get::<Visibility>(e) == Some(&Visibility::Hidden)).collect());
    }
    if path == Path::New {
        // The material path really drew the adopted nodes: the buffer grew mid-series, one node came
        // and one went, and the nodes are on the shared look materials.
        let slots = app.world().resource::<SpriteSlots>();
        assert_eq!((slots.live(), slots.capacity()), (SLOTS_AT_START, 2 * SLOTS_AT_START), "slots after the series");
        // Looks: the puffs' two sprites (the four node ids draw two sprites) and the six blend nodes';
        // no private copy is left.
        let looks = app.world().resource::<Assets<SpriteMaterial>>().len();
        // The ghost's nodes stay demoted; the unit's are back on their looks (its fade ends at 1).
        let on_looks = app.world_mut().query::<&MeshMaterial3d<SpriteMaterial>>().iter(app.world()).count();
        assert_eq!((looks, on_looks), (2 + BLENDS.len(), SLOTS_AT_START as usize - ghost_nodes.len()), "looks and nodes drawn through them");
        assert_eq!(demoted(&app, &ghost_nodes), ghost_nodes.len(), "the ghost's nodes are on private materials");
        // The unit's nodes went to private materials while it faded and came back when it did not.
        let expected: Vec<usize> = UNIT_FADE.iter().map(|&k| if k < 1.0 { unit_nodes.len() } else { 0 }).collect();
        assert_eq!(unit_demoted, expected, "the unit's demoted nodes per step");
    }
    // The case the blinking flipbook nodes are there for: each is hidden in some step and shown again
    // in a later one, so a change of the clock step or the strobe's timing cannot drop it unnoticed.
    for (k, &e) in blink_nodes.iter().enumerate() {
        let hidden: Vec<bool> = blink_hidden.iter().map(|s| s[k]).collect();
        assert!(hidden.windows(2).any(|w| w[0] && !w[1]), "{path:?}: blinking flipbook node {k} ({e}) is never shown after being hidden: {hidden:?}");
    }
    Some(frames)
}

#[test]
fn nodes_are_drawn_byte_equal_to_the_reference_path_on_gpu_from_four_cameras() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let _ = TABLE;
    let lit = |f: &Vec<u8>| f.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
    // Every frame of every camera is compared before the test fails, so a failure names them all.
    let mut differing = Vec::new();
    for camera in [Camera::Stock, Camera::Tonemapped4x, Camera::HdrBloom, Camera::DepthPrepass4x] {
        let Some(old) = series(Path::Reference, camera) else { return };
        // Control: the reference against itself, so a difference below is the path's, not the harness'.
        let Some(again) = series(Path::Reference, camera) else { return };
        let Some(new) = series(Path::New, camera) else { return };
        assert_eq!(old.len(), new.len());
        let mut differ = 0;
        for (n, ((a, b), c)) in old.iter().zip(&again).zip(&new).enumerate() {
            assert!(lit(a) > 300, "{camera:?} frame {n} shows the scene ({} lit pixels)", lit(a));
            assert!(a == b, "{camera:?} control frame {n} differs: {}", describe(a, b));
            if a != c {
                differ += 1;
                differing.push(format!("{camera:?} frame {n} differs from the reference: {}", describe(a, c)));
            }
        }
        eprintln!("{camera:?}: control equal, {differ} of {} frames of the material path differ", old.len());
        // The frames differ from one another (time and camera move), so equality is not an empty scene's.
        assert!(old[0] != old[5]);
    }
    let mode = MODE.get().copied().unwrap_or("unknown");
    assert!(differing.is_empty(), "sprite material ran {mode}{}:\n{}", if parity() { "" } else { ", parity mode off" }, differing.join("\n"));
    eprintln!("sprite material ran {mode}: every frame of the four cameras byte-equal to the reference");
}

// --- #1237: the parity mark only for boards a view sees ---------------------------------------------

/// Both cameras' pictures at one update, as two captures taken in the same frame.
fn shoot_two(app: &mut App, a: &Handle<Image>, b: &Handle<Image>) -> (Vec<u8>, Vec<u8>) {
    let sinks: Vec<Arc<Mutex<Option<Image>>>> = (0..2).map(|_| Arc::new(Mutex::new(None))).collect();
    for (target, sink) in [a, b].into_iter().zip(&sinks) {
        let sink = Arc::clone(sink);
        app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| {
            *sink.lock().unwrap() = Some(shot.image.clone());
        });
    }
    for _ in 0..60 {
        app.update();
        if sinks.iter().all(|s| s.lock().unwrap().is_some()) {
            break;
        }
    }
    let take = |s: &Arc<Mutex<Option<Image>>>| s.lock().unwrap().clone().expect("a capture").data.expect("pixels");
    (take(&sinks[0]), take(&sinks[1]))
}

/// What the scene of [`view_series`] adds to its row of boards.
#[derive(Clone, Copy, Debug)]
struct Extras {
    /// Colour-cycling boards out of every view: they change state in every frame, so on `main` they
    /// are marked in every frame and the set Bevy re-queues from is large.
    crowd: usize,
    /// A blended pair and an additive pair, each at one place and size (a sort tie, in every camera).
    pairs: bool,
    /// A colour-cycling board on a parent that moves in depth and passes a constant board that it
    /// overlaps: its place among the sorted items is right only if it is re-queued (its sort centre is
    /// taken at queue time), which the parity mark does. Without the mark it draws on the wrong side.
    crossing: bool,
    /// The crowd is spawned in batches of ten, one batch per update, so the reference path's set of
    /// changed entities grows to the crowd's size while the new path's stays small (a different bucket
    /// count in a hash set keyed by entity index), and six pairs of equal-distance blended boards sit
    /// at entity ids that straddle multiples of 16 to 512.
    staged: bool,
    /// A deliberately broken form of the new path, a positive control (test builds only).
    control: material::Control,
}

impl Extras {
    const NONE: Extras = Extras { crowd: 0, pairs: false, crossing: false, staged: false, control: material::Control::None };
}

/// The boards' places along x: the row the cameras pan along.
const ROW: [f32; 17] = [-120.0, -105.0, -90.0, -75.0, -60.0, -45.0, -30.0, -15.0, 0.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0, 105.0, 120.0];
/// The panning camera's x at each step: it goes out to both ends and back, so boards come into view,
/// leave it and come back, while the second camera stays at x = 60.
const PAN: [f32; 12] = [-80.0, -40.0, 0.0, 40.0, 80.0, 120.0, 80.0, 0.0, -80.0, -40.0, 40.0, 120.0];

/// What the series saw of each board: per step, whether the board is visible in any view, in the
/// panning camera, and in the fixed one (Bevy's own visible-entity lists).
struct Seen {
    any: Vec<Vec<bool>>,
    in_a: Vec<Vec<bool>>,
    in_b: Vec<Vec<bool>>,
    /// The boards under the parent that is hidden in some steps.
    grouped: Vec<usize>,
}

/// Frames of the panning camera and of the fixed camera at 12 steps, and what each saw.
fn view_series(path: Path, camera: Camera, extras: Extras) -> Option<(Vec<Vec<u8>>, Vec<Vec<u8>>, Seen)> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    app.insert_resource(ClearColor(Color::BLACK)).insert_resource(table()).insert_resource(AnimClock::new(ClockMode::Bench { step: 0.137 }, 0));
    // The clock moves once per step, in the update the capture is asked for (the frame that is
    // captured), not once per update: how many updates pass before a capture arrives depends on the
    // machine's load, and would shift the time of every later step.
    #[derive(Resource)]
    struct Step(bool);
    fn tick(mut clock: ResMut<AnimClock>, mut step: ResMut<Step>) {
        if std::mem::take(&mut step.0) {
            clock.advance_idle(0.0);
        }
    }
    app.insert_resource(Step(false));
    app.add_systems(Update, tick.before(animate_reference).before(animate));
    app.add_plugins(SpriteMaterialPlugin);
    let slots = SpriteSlots::with_capacity(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(), 16);
    app.insert_resource(slots);
    app.world_mut().resource_mut::<SpriteLooks>().adopt = path == Path::New;
    app.world_mut().resource_mut::<material::ParityMarks>().control = extras.control;
    match path {
        Path::New => app.add_systems(Update, (animate, apply_fade, tint_ghost).chain()),
        Path::Reference => app.add_systems(Update, (animate_reference, apply_fade, tint_ghost).chain()),
        Path::Late => unreachable!("no late path here"),
    };
    let mut targets = Vec::new();
    for order in 0..2 {
        let mut t = Image::new_target_texture(160, 120, TextureFormat::Rgba8UnormSrgb, None);
        t.asset_usage = bevy::asset::RenderAssetUsages::RENDER_WORLD;
        targets.push(app.world_mut().resource_mut::<Assets<Image>>().add(t));
        let cam = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                bevy::camera::Camera { order, ..default() },
                RenderTarget::from(targets[order as usize].clone()),
                Transform::from_xyz(if order == 0 { PAN[0] } else { 60.0 }, 0.0, 60.0),
            ))
            .id();
        match camera {
            Camera::Stock => app.world_mut().entity_mut(cam).insert((Tonemapping::None, Msaa::Off)),
            Camera::Tonemapped4x => app.world_mut().entity_mut(cam).insert(Msaa::Sample4),
            Camera::HdrBloom => app.world_mut().entity_mut(cam).insert((bevy::camera::Hdr, bevy::post_process::bloom::Bloom::NATURAL, Msaa::Sample4)),
            Camera::DepthPrepass4x => app.world_mut().entity_mut(cam).insert((DepthPrepass, Msaa::Sample4)),
        };
    }
    let cams: Vec<Entity> = app.world_mut().query_filtered::<Entity, With<Camera3d>>().iter(app.world()).collect();
    // The panning camera is the one of order 0.
    let pan = *cams.iter().find(|&&c| app.world().get::<bevy::camera::Camera>(c).unwrap().order == 0).unwrap();
    let still = *cams.iter().find(|&&c| app.world().get::<bevy::camera::Camera>(c).unwrap().order == 1).unwrap();
    let tex = app.world_mut().resource_mut::<Assets<Image>>().add(texture());
    let quad = app.world_mut().resource_mut::<Assets<Mesh>>().add(unit_quad());
    let t = table();
    let spawn = |app: &mut App, node: &str, at: Vec3, size: f32, parent: Option<Entity>| {
        let (mut m, mut fx, _) = material_of(&t, node);
        m.base_color_texture = Some(tex.clone());
        let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m);
        fx.material = h.clone();
        let mut e = app.world_mut().spawn((Mesh3d(quad.clone()), MeshMaterial3d(h), Transform::from_translation(at).with_scale(Vec3::new(size, size, 1.0)), Visibility::default(), fx, Billboard));
        if let Some(p) = parent {
            e.insert(ChildOf(p));
        }
        e.id()
    };
    // The row: every blend mode, a colour that cycles or holds, places along x, each at its own depth
    // (no two boards at one distance), overlapping their neighbours.
    const KINDS: [&str; 8] = ["puffA", "blA", "adA", "cutB", "puffB", "blB", "adB", "cutA"];
    let mut boards = Vec::new();
    for (i, &x) in ROW.iter().enumerate() {
        boards.push(spawn(&mut app, KINDS[i % KINDS.len()], Vec3::new(x, (i % 3) as f32 * 5.0 - 5.0, -3.0 + 0.37 * i as f32), 20.0, None));
    }
    // Boards under one parent that is hidden in some steps (inside the fixed camera's view).
    let group = app.world_mut().spawn((Transform::from_xyz(60.0, 12.0, 4.0), Visibility::default())).id();
    let mut grouped = Vec::new();
    for (i, kind) in ["puffA", "adA", "blA", "puffB"].into_iter().enumerate() {
        grouped.push(boards.len());
        boards.push(spawn(&mut app, kind, Vec3::new(i as f32 * 9.0 - 14.0, (i % 2) as f32 * 6.0, 0.21 * i as f32), 14.0, Some(group)));
    }
    // A colour-cycling board on a parent moving in depth, passing a constant board it overlaps. Both
    // are in the fixed camera's view and, from step 3 on, in the panning camera's.
    let mover = if extras.crossing {
        spawn(&mut app, "blB", Vec3::new(48.0, -14.0, 0.0), 18.0, None);
        let carrier = app.world_mut().spawn((Transform::from_xyz(0.0, 0.0, -8.2), Visibility::default())).id();
        let dim = spawn(&mut app, "blC", Vec3::new(51.0, -13.0, 0.0), 18.0, Some(carrier));
        app.world_mut().entity_mut(dim).insert(SpriteFade(0.5));
        Some(carrier)
    } else {
        None
    };
    // Six pairs of equal-distance blended boards whose entity ids straddle multiples of 16 to 512 (staged
    // runs only): one board takes the last index below the multiple, the other the first at it.
    if extras.staged {
        for (k, modulus) in [16u32, 32, 64, 128, 256, 512].into_iter().enumerate() {
            let at = Vec3::new(28.0 + 11.0 * k as f32, -20.0, 1.5);
            let first = loop {
                let e = app.world_mut().spawn_empty().id();
                if e.index_u32() % modulus == modulus - 1 {
                    break e;
                }
            };
            let (mut m, mut fx, _) = material_of(&t, "blA");
            m.base_color_texture = Some(tex.clone());
            let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m);
            fx.material = h.clone();
            app.world_mut().entity_mut(first).insert((Mesh3d(quad.clone()), MeshMaterial3d(h), Transform::from_translation(at).with_scale(Vec3::new(10.0, 10.0, 1.0)), Visibility::default(), fx, Billboard));
            let dim = spawn(&mut app, "blC", at, 10.0, None);
            app.world_mut().entity_mut(dim).insert(SpriteFade(0.5));
        }
    }
    // The first board of each co-located pair, then the colour-cycling boards in no view, then the
    // second board of each pair: the crowd sits between the pair's entity ids, so the pair's relative
    // place in a hash set of changed entities moves with the crowd's size.
    if extras.pairs {
        spawn(&mut app, "blA", Vec3::new(62.0, -14.0, 1.5), 16.0, None);
        spawn(&mut app, "adA", Vec3::new(22.0, 14.0, 1.5), 16.0, None);
    }
    let crowd_at = |i: usize| Vec3::new((i % 40) as f32 * 6.0 - 120.0, 300.0 + (i / 40) as f32 * 6.0, -3.0 + 0.011 * i as f32);
    if !extras.staged {
        for i in 0..extras.crowd {
            spawn(&mut app, KINDS[i % 3], crowd_at(i), 20.0, None);
        }
    }
    if extras.pairs {
        // Both blended boards cycle in colour, so both are re-queued in every frame; the second is
        // dimmed by a fade, so the order they are drawn in shows in the picture.
        let dim = spawn(&mut app, "blC", Vec3::new(62.0, -14.0, 1.5), 16.0, None);
        app.world_mut().entity_mut(dim).insert(SpriteFade(0.5));
        let dim = spawn(&mut app, "adC", Vec3::new(22.0, 14.0, 1.5), 16.0, None);
        app.world_mut().entity_mut(dim).insert(SpriteFade(0.5));
    }
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    let adapter = app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()).or_else(|| app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())?;
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: only a software adapter ({})", adapter.name);
        return None;
    }
    eprintln!("adapter: {}", adapter.name);
    if extras.staged {
        for batch in 0..extras.crowd / 10 {
            for i in batch * 10..batch * 10 + 10 {
                spawn(&mut app, KINDS[i % 3], crowd_at(i), 20.0, None);
            }
            app.update();
        }
        for _ in 0..4 {
            app.update();
        }
    }
    let mut seen = Seen { any: Vec::new(), in_a: Vec::new(), in_b: Vec::new(), grouped };
    let (mut frames_a, mut frames_b) = (Vec::new(), Vec::new());
    for step in 0..PAN.len() {
        *app.world_mut().get_mut::<Transform>(pan).unwrap() = Transform::from_xyz(PAN[step], 0.0, 60.0);
        *app.world_mut().get_mut::<Transform>(still).unwrap() = Transform::from_xyz(60.0, 0.0, 60.0);
        *app.world_mut().get_mut::<Visibility>(group).unwrap() = if matches!(step, 4..=6 | 9) { Visibility::Hidden } else { Visibility::Inherited };
        if let Some(carrier) = mover {
            // Behind the constant board until step 5, in front of it from step 6.
            *app.world_mut().get_mut::<Transform>(carrier).unwrap() = Transform::from_xyz(0.0, 0.0, -8.2 + 1.5 * step as f32);
        }
        // Fades move every step on the blended and additive boards (a cut-out board that fades is
        // demoted, which the other test covers).
        for (i, &e) in boards.iter().enumerate() {
            if i < ROW.len() && KINDS[i % KINDS.len()].starts_with("cut") {
                continue;
            }
            match fade_of(i, step) {
                Some(k) => {
                    app.world_mut().entity_mut(e).insert(SpriteFade(k));
                }
                None => {
                    app.world_mut().entity_mut(e).remove::<SpriteFade>();
                }
            }
        }
        app.world_mut().resource_mut::<Step>().0 = true;
        let (a, b) = shoot_two(&mut app, &targets[0], &targets[1]);
        frames_a.push(a);
        frames_b.push(b);
        let in_view = |app: &App, cam: Entity| -> Vec<bool> {
            let list = app.world().get::<bevy::camera::visibility::VisibleEntities>(cam).map(|v| v.get(std::any::TypeId::of::<Mesh3d>()).to_vec()).unwrap_or_default();
            boards.iter().map(|e| list.contains(e)).collect()
        };
        seen.in_a.push(in_view(&app, pan));
        seen.in_b.push(in_view(&app, still));
        seen.any.push(boards.iter().map(|&e| app.world().get::<ViewVisibility>(e).is_some_and(|v| v.get())).collect());
    }
    Some((frames_a, frames_b, seen))
}

/// Every case the parity mark has to get right is in the scene's steps: a board comes into view, leaves
/// it and comes back; the cameras see different boards; boards under a hidden parent are not seen.
fn assert_cases_are_exercised(seen: &Seen) {
    let steps = seen.any.len();
    let n = seen.any[0].len();
    let col = |v: &[Vec<bool>], i: usize| -> Vec<bool> { (0..steps).map(|s| v[s][i]).collect() };
    let entered = (0..n).filter(|&i| col(&seen.any, i).windows(2).any(|w| !w[0] && w[1])).count();
    let re_entered = (0..n).filter(|&i| col(&seen.any, i).windows(3).any(|w| w[0] && !w[1] && w[2])).count();
    let only_a = (0..steps).any(|s| (0..n).any(|i| seen.in_a[s][i] && !seen.in_b[s][i]));
    let only_b = (0..steps).any(|s| (0..n).any(|i| seen.in_b[s][i] && !seen.in_a[s][i]));
    let both = (0..steps).any(|s| (0..n).any(|i| seen.in_a[s][i] && seen.in_b[s][i]));
    let hidden_by_parent = [4usize, 5, 6, 9].iter().all(|&s| seen.grouped.iter().all(|&i| !seen.any[s][i])) && seen.grouped.iter().any(|&i| seen.any[3][i]);
    let never = (0..n).filter(|&i| col(&seen.any, i).iter().all(|&v| !v)).count();
    eprintln!("cases: {entered} boards entered a view, {re_entered} left and came back, only panning {only_a}, only fixed {only_b}, both {both}, hidden by a parent {hidden_by_parent}, never seen {never}");
    assert!(entered >= 6 && re_entered >= 3 && only_a && only_b && both && hidden_by_parent, "the scene must reach every case");
}

/// #1237: the parity mark is applied after visibility, to boards some view sees. From two cameras,
/// one panning along a row of boards (boards enter its view, leave it and come back) and one fixed
/// (it sees boards the other does not, and boards the other sees too), with boards under a parent that
/// is hidden for some steps and a crowd of colour-cycling boards no camera sees, every frame of both
/// cameras is byte-equal to the reference path's, which marks every board whose state changes. The
/// scene has no two boards at one distance: the tie case is [`co_located_boards_on_gpu`].
#[test]
fn boards_a_view_sees_are_drawn_byte_equal_to_the_reference_path_from_two_cameras_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let extras = Extras { crowd: 60, crossing: true, ..Extras::NONE };
    let lit = |f: &Vec<u8>| f.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
    let mut differing = Vec::new();
    for camera in [Camera::Stock, Camera::Tonemapped4x, Camera::HdrBloom, Camera::DepthPrepass4x] {
        let Some((oa, ob, os)) = view_series(Path::Reference, camera, extras) else { return };
        let Some((ra, rb, _)) = view_series(Path::Reference, camera, extras) else { return };
        let Some((na, nb, ns)) = view_series(Path::New, camera, extras) else { return };
        assert_cases_are_exercised(&os);
        assert!(os.any == ns.any && os.in_a == ns.in_a && os.in_b == ns.in_b, "{camera:?}: the two paths saw different boards");
        let mut differ = 0;
        for n in 0..oa.len() {
            for (name, old, again, new) in [("panning", &oa, &ra, &na), ("fixed", &ob, &rb, &nb)] {
                assert!(lit(&old[n]) > 300, "{camera:?} {name} frame {n} shows the scene ({} lit pixels)", lit(&old[n]));
                assert!(old[n] == again[n], "{camera:?} {name} control frame {n} differs: {}", describe(&old[n], &again[n]));
                if old[n] != new[n] {
                    differ += 1;
                    differing.push(format!("{camera:?} {name} frame {n} differs from the reference: {}", describe(&old[n], &new[n])));
                }
            }
        }
        eprintln!("{camera:?}: control equal, {differ} of {} frames differ", 2 * oa.len());
        assert!(oa[0] != oa[5] && ob[0] != ob[5], "the frames move");
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
}

/// The tie case of #1237, which the amended S5 does not remove: two boards at exactly one distance
/// that are both re-queued in one frame take their place in the phase from the order in which Bevy's set
/// of changed entities iterates, and that set holds fewer entities than on the reference path (only
/// those a view sees). Nothing in Bevy 0.20.1 makes the order the same (the phase keeps equal-distance
/// items in insertion order, and insertion follows the iteration of a hash set whose size and layout
/// differ), so this documents what happens and asserts nothing about the order: the pairs (both boards
/// of each pair re-queued in every frame, one of them dimmed so the order shows; the entity ids of each
/// pair's boards separated by a crowd of unseen, colour-cycling boards of growing size, so each pair's
/// places in the set move) are drawn, and per crowd size the test prints how many frames differ from
/// the reference path. It prints nothing for a crowd size where the reference path does not repeat
/// itself: the tie order is not repeatable even on the reference path under load (the specialisation
/// check collects its entities from several threads), which is the same hash-set accident. The test
/// fails only if a frame shows no pair.
#[test]
fn co_located_boards_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    for camera in [Camera::Stock, Camera::DepthPrepass4x] {
        for crowd in [0usize, 7, 31, 60, 150, 300, 700, 2000] {
            let extras = Extras { crowd, pairs: true, ..Extras::NONE };
            let Some((oa, ob, _)) = view_series(Path::Reference, camera, extras) else { return };
            let Some((ra, rb, _)) = view_series(Path::Reference, camera, extras) else { return };
            let Some((na, nb, _)) = view_series(Path::New, camera, extras) else { return };
            let lit = |f: &Vec<u8>| f.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
            assert!(oa.iter().chain(&ob).all(|f| lit(f) > 300), "{camera:?}, crowd {crowd}: a frame shows no scene");
            if !(oa == ra && ob == rb) {
                eprintln!("TIE {camera:?} crowd {crowd}: the reference path does not repeat itself here (tie order is not repeatable under load); not compared");
                continue;
            }
            let mut lines = Vec::new();
            for n in 0..oa.len() {
                for (name, old, new) in [("panning", &oa, &na), ("fixed", &ob, &nb)] {
                    if old[n] != new[n] {
                        lines.push(format!("{name} frame {n}: {}", describe(&old[n], &new[n])));
                    }
                }
            }
            eprintln!("TIE {camera:?} crowd {crowd}: {} of {} frames differ{}{}", lines.len(), 2 * oa.len(), if lines.is_empty() { "" } else { ": " }, lines.join("; "));
        }
    }
}

/// Positive control of the two-camera test above (H1 of the review of #1269): the new path with the
/// parity mark taken out, and with it a frame late, must NOT be byte-equal to the reference path in the
/// scene where a colour-cycling board on a moving parent passes a constant board, while the new path
/// with the mark must be. Without this, the byte-equality tests could pass with the mark removed.
#[test]
fn the_new_path_without_the_mark_or_with_it_late_differs_from_the_reference_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let base = Extras { crowd: 60, crossing: true, ..Extras::NONE };
    for camera in [Camera::Stock, Camera::DepthPrepass4x] {
        let Some((oa, ob, _)) = view_series(Path::Reference, camera, base) else { return };
        let Some((ra, rb, _)) = view_series(Path::Reference, camera, base) else { return };
        assert!(oa == ra && ob == rb, "{camera:?}: the reference does not repeat");
        let count = |a: &[Vec<u8>], b: &[Vec<u8>]| a.iter().zip(b).filter(|(x, y)| x != y).count();
        let mut line = Vec::new();
        for (name, control, must_differ) in [("mark", material::Control::None, false), ("no mark", material::Control::Off, true), ("late mark", material::Control::Late, true)] {
            let Some((na, nb, _)) = view_series(Path::New, camera, Extras { control, ..base }) else { return };
            let differing = count(&oa, &na) + count(&ob, &nb);
            line.push(format!("{name}: {differing} of {} frames differ", oa.len() + ob.len()));
            assert_eq!(differing > 0, must_differ, "{camera:?} {name}: {differing} frames differ from the reference");
        }
        eprintln!("{camera:?}: {}", line.join("; "));
    }
}

/// The tie case with the reference path's changed set grown past the new path's (case (a) of the
/// review): the crowd is spawned in batches, so on the reference path the set of changed entities grows to
/// the crowd's size (every cycling board is marked, seen or not) while on the new path it stays at the
/// seen boards, and six pairs of equal-distance blended boards sit at entity ids that straddle multiples
/// of 16 to 512. Documents how many frames differ; asserts nothing about the order (it is not defined
/// where the bucket counts differ), only that the scene is drawn.
#[test]
fn staged_co_located_pairs_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let lit = |f: &Vec<u8>| f.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
    for camera in [Camera::Stock, Camera::DepthPrepass4x] {
        for crowd in [0usize, 80, 400, 1200] {
            let extras = Extras { crowd, staged: true, ..Extras::NONE };
            let Some((oa, ob, _)) = view_series(Path::Reference, camera, extras) else { return };
            let Some((ra, rb, _)) = view_series(Path::Reference, camera, extras) else { return };
            let Some((na, nb, _)) = view_series(Path::New, camera, extras) else { return };
            assert!(oa.iter().chain(&ob).all(|f| lit(f) > 300), "{camera:?}, crowd {crowd}: a frame shows no scene");
            let repeats = oa == ra && ob == rb;
            let differing = oa.iter().zip(&na).filter(|(x, y)| x != y).count() + ob.iter().zip(&nb).filter(|(x, y)| x != y).count();
            eprintln!("STAGED {camera:?} crowd {crowd}: reference repeats {repeats}, {differing} of {} frames differ from the reference", oa.len() + ob.len());
        }
    }
}

/// Where two frames of [`series`] differ: how many pixels, their box, the largest channel difference
/// and a few of them.
fn describe(a: &[u8], b: &[u8]) -> String {
    const W: usize = 160;
    let px: Vec<(usize, &[u8], &[u8])> = a.chunks(4).zip(b.chunks(4)).enumerate().filter(|(_, (p, q))| p != q).map(|(i, (p, q))| (i, p, q)).collect();
    if px.is_empty() {
        return "equal".into();
    }
    let (mut x0, mut x1, mut y0, mut y1) = (W, 0, usize::MAX, 0);
    let mut most = 0;
    for &(i, p, q) in &px {
        let (x, y) = (i % W, i / W);
        (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        most = most.max(p.iter().zip(q).map(|(u, v)| u.abs_diff(*v)).max().unwrap_or(0));
    }
    let samples: Vec<String> = px.iter().step_by((px.len() / 4).max(1)).take(4).map(|(i, p, q)| format!("({}, {}) {p:?} against {q:?}", i % W, i / W)).collect();
    format!("{} pixels in x {x0}..={x1}, y {y0}..={y1}, largest channel difference {most}; {}", px.len(), samples.join(", "))
}

/// Negative control: the harness fails when the picture differs. The reference system run one frame
/// late must give frames that are not byte-equal to the reference's.
#[test]
fn a_path_that_writes_one_frame_late_is_not_byte_equal_to_the_reference_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let Some(old) = series(Path::Reference, Camera::Stock) else { return };
    let Some(late) = series(Path::Late, Camera::Stock) else { return };
    let differing = old.iter().zip(&late).filter(|(a, b)| a != b).count();
    eprintln!("late path: {differing} of {} frames differ", old.len());
    assert!(differing >= old.len() / 2, "only {differing} frames differ: the harness cannot see a one-frame lag");
}
