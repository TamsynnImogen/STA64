//! Real-GPU identity check of how sprite nodes reach the screen: the same invented scene drawn through
//! the reference path ([`super::tests::animate_reference`], `main`'s per-node `animate`) and through
//! the sprite material path of ADR 0009 ([`super::material`]: shared look materials, the colour in the
//! node's slot, the forked `sprite.wesl`), at a new time, camera place and fade every frame, from three
//! cameras. The two sets of frames must be byte-equal; a control run (reference against reference)
//! must be byte-equal too, which proves the harness itself is deterministic.
//!
//! Until step 2 of the ADR adopts the game's nodes, the material path is driven by a stand-in for its
//! `animate` ([`write_slots`]): the unchanged [`animate`] computes every node's colour and uv into its
//! own (no longer drawn) material, and the stand-in copies them into the node's slot and its look's
//! material with main's rule (only while shown, only on change) and parity mode (S5). The cloaking
//! unit's and the ghost's nodes stay on their own materials, as the ADR keeps them (S4).
//!
//! The scene has: nebula-like puffs whose fades change every step while their look's colour cycles;
//! a blinking glow, flat sprites, a moon nimbus; a co-located pair each of blended and of additive
//! sprites and a coplanar overlap of cut-out ones; a cloaking unit and a placement ghost run through
//! the game's own `apply_fade` and `tint_ghost`; a sprite node spawned and one despawned mid-series;
//! and a group of puffs whose parent toggles between hidden and inherited (the Stock/Refit switch).
//! Needs `STA64_GPU_TESTS=1` and a hardware adapter; skips otherwise.
use super::tests::{BLENDS, NODES, TABLE, animate_reference, fade_of, material_of, table};
use bevy::core_pipeline::tonemapping::Tonemapping;
use super::*;
use super::material::{SpriteMaterial, SpriteMaterialPlugin, SpriteSlots, linear, look_material};
use bevy::mesh::MeshTag;
use bevy::render::storage::ShaderBuffer;
use std::collections::HashMap;
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
    /// The sprite material path under test: nodes on their look's shared [`SpriteMaterial`], the
    /// colour in their slot ([`write_slots`] after [`animate`]).
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
}

/// The colour and uv an adopted node's own material held when it was last shown: what main's
/// per-node material would hold now (the ADR's `SpriteState`).
#[derive(Component)]
struct Shown {
    colour: Color,
    uv: Affine2,
}

/// The shared material of each look, by sprite name (the scene has one texture).
#[derive(Resource, Default)]
struct Looks(HashMap<String, Handle<SpriteMaterial>>);

/// Put a freshly spawned node on the sprite material path in the same frame, before any update:
/// a slot holding the colour its own material starts with, its look's shared material, and the
/// state it starts from. `own` is the material the node was spawned with.
fn adopt(app: &mut App, node: Entity, own: &StandardMaterial) {
    let world = app.world_mut();
    let sprite = world.get::<SpriteFx>(node).unwrap().sprite.clone();
    let tag = world.resource_scope(|w, mut slots: Mut<SpriteSlots>| {
        let mut buffers = w.resource_mut::<Assets<ShaderBuffer>>();
        let tag = slots.allocate(&mut buffers);
        slots.write(&mut buffers, [(&tag, linear(own.base_color))]);
        tag
    });
    let look = match world.resource::<Looks>().0.get(&sprite) {
        Some(h) => h.clone(),
        None => {
            let m = look_material(own, world.resource::<SpriteSlots>());
            let h = world.resource_mut::<Assets<SpriteMaterial>>().add(m);
            world.resource_mut::<Looks>().0.insert(sprite, h.clone());
            h
        }
    };
    world
        .entity_mut(node)
        .remove::<MeshMaterial3d<StandardMaterial>>()
        .insert((tag, MeshMaterial3d(look), Shown { colour: own.base_color, uv: own.uv_transform }));
}

/// The stand-in for step 2's `animate` on adopted nodes, after the unchanged [`animate`] has written
/// each node's colour and uv into its own material: while the node is shown and its values changed
/// (main's rule), the new linear colour goes into its slot, the uv into its look's material if that
/// differs, and the node's material component is marked changed (parity mode, S5), so the nodes
/// re-queued are main's. All slot writes touch the buffer once.
fn write_slots(
    slots: Res<SpriteSlots>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    own: Res<Assets<StandardMaterial>>,
    mut looks: ResMut<Assets<SpriteMaterial>>,
    mut nodes: Query<(&SpriteFx, &MeshTag, &Visibility, &mut Shown, &mut MeshMaterial3d<SpriteMaterial>)>,
) {
    let mut writes = Vec::new();
    for (f, tag, vis, mut last, mut look) in &mut nodes {
        if *vis == Visibility::Hidden {
            continue;
        }
        let Some(m) = own.get(&f.material) else { continue };
        if m.base_color == last.colour && m.uv_transform == last.uv {
            continue;
        }
        if m.base_color != last.colour {
            writes.push((tag.clone(), linear(m.base_color)));
        }
        if looks.get(&look.0).is_some_and(|l| l.base.uv_transform != m.uv_transform)
            && let Some(mut l) = looks.get_mut(&look.0)
        {
            l.base.uv_transform = m.uv_transform;
        }
        *last = Shown { colour: m.base_color, uv: m.uv_transform };
        look.set_changed();
    }
    slots.write(&mut buffers, writes.iter().map(|(t, c)| (t, *c)));
}

/// Slots of the nodes adopted before the series starts: the buffer is full when the series begins,
/// so the node spawned mid-series grows it (S1, growth by doubling) between two captured frames.
const SLOTS_AT_START: u32 = 48 + 6 + 6 + 1;

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
    // The material path's plugin, slots and observer are in every path's app, idle where no node is
    // adopted, so the apps differ only in what the path adopts (an observer is an entity: leaving it
    // out of one app would also shift the scene's entity ids).
    app.add_plugins(SpriteMaterialPlugin).init_resource::<Looks>();
    let slots = SpriteSlots::with_capacity(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(), SLOTS_AT_START);
    app.insert_resource(slots);
    // A despawned node's slot goes back, as step 2's adoption will do.
    app.add_observer(|remove: On<Remove<MeshTag>>, tags: Query<&MeshTag>, mut slots: ResMut<SpriteSlots>| {
        if let Ok(tag) = tags.get(remove.entity) {
            slots.release(tag);
        }
    });
    match path {
        Path::New => app.add_systems(Update, (animate, write_slots, apply_fade, tint_ghost, face_camera).chain()),
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
    };
    let quad = app.world_mut().resource_mut::<Assets<Mesh>>().add(unit_quad());
    let t = table();
    let mut puffs = Vec::new();
    for i in 0..48usize {
        let node = NODES[i % NODES.len()];
        let (mut m, mut fx, _) = material_of(&t, node);
        m.base_color_texture = Some(tex.clone());
        let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m.clone());
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
        let e = e.id();
        if path == Path::New {
            adopt(&mut app, e, &m);
        }
        puffs.push(e);
    }
    // A cloaking unit and a placement ghost: their nodes are given material copies by the game's own
    // systems. Same look as the puffs above (so, shared when sharing is on).
    let family = |app: &mut App, root: Entity, nodes: &[usize]| {
        for &i in nodes {
            let (mut m, mut fx, _) = material_of(&t, NODES[i % NODES.len()]);
            m.base_color_texture = Some(tex.clone());
            let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m);
            fx.material = h.clone();
            let child = app
                .world_mut()
                .spawn((Mesh3d(quad.clone()), MeshMaterial3d(h), Transform::from_xyz(i as f32 * 5.0 - 8.0, 0.0, 0.0).with_scale(Vec3::new(12.0, 12.0, 1.0)), Visibility::default(), fx, Billboard))
                .id();
            app.world_mut().entity_mut(root).add_child(child);
        }
    };
    let unit = app.world_mut().spawn((UnitFade { cloak: 1.0, build: 1.0 }, Transform::from_xyz(-20.0, 20.0, 5.0), Visibility::default())).id();
    family(&mut app, unit, &[0, 1, 2, 3]);
    let ghost = app.world_mut().spawn((Ghost::pinned(ContentKey::NONE), Transform::from_xyz(18.0, -18.0, 5.0), Visibility::default())).id();
    family(&mut app, ghost, &[0, 1, 2, 3]);
    // One node of each blend look placed on top of another of a different look (same distance from
    // the eye: a sort tie), and two cut-out sprites in one plane.
    let spawn = |app: &mut App, node: &str, at: Vec3, size: f32, billboard: bool, parent: Option<Entity>| {
        let (mut m, mut fx, _) = material_of(&t, node);
        m.base_color_texture = Some(tex.clone());
        let h = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(m.clone());
        fx.material = h.clone();
        let mut e = app.world_mut().spawn((Mesh3d(quad.clone()), MeshMaterial3d(h), Transform::from_translation(at).with_scale(Vec3::new(size, size, 1.0)), Visibility::default(), fx));
        if billboard {
            e.insert(Billboard);
        }
        if let Some(p) = parent {
            e.insert(ChildOf(p));
        }
        let e = e.id();
        if path == Path::New {
            adopt(app, e, &m);
        }
        e
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
    let places = [Vec3::new(0.0, 0.0, 60.0), Vec3::new(70.0, 10.0, 40.0), Vec3::new(-90.0, 30.0, 30.0), Vec3::new(10.0, 80.0, 20.0)];
    let mut frames = Vec::new();
    for step in 0..12usize {
        // The unit cloaks and decloaks, then builds up: its fade changes at every step.
        let k = [1.0, 0.8, 0.5, 0.3, 0.3, 0.6, 1.0, 1.0, 0.7, 0.2, 0.5, 1.0][step];
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
    }
    if path == Path::New {
        // The material path really drew the adopted nodes: the buffer grew mid-series, one node came
        // and one went, and the nodes are on the shared look materials.
        let slots = app.world().resource::<SpriteSlots>();
        assert_eq!((slots.live(), slots.capacity()), (SLOTS_AT_START, 2 * SLOTS_AT_START), "slots after the series");
        let looks = app.world().resource::<Looks>().0.len();
        let on_looks = app.world_mut().query::<&MeshMaterial3d<SpriteMaterial>>().iter(app.world()).count();
        // Looks: the puffs' two sprites (the four node ids draw two sprites) and the six blend nodes'.
        assert_eq!((looks, on_looks), (2 + BLENDS.len(), SLOTS_AT_START as usize), "looks and adopted nodes");
    }
    Some(frames)
}

#[test]
fn nodes_are_drawn_byte_equal_to_the_reference_path_on_gpu_from_three_cameras() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let _ = TABLE;
    let lit = |f: &Vec<u8>| f.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
    // Every frame of every camera is compared before the test fails, so a failure names them all.
    let mut differing = Vec::new();
    for camera in [Camera::Stock, Camera::Tonemapped4x, Camera::HdrBloom] {
        let Some(old) = series(Path::Reference, camera) else { return };
        // Control: the reference against itself, so a difference below is the path's, not the harness'.
        let Some(again) = series(Path::Reference, camera) else { return };
        let Some(new) = series(Path::New, camera) else { return };
        assert_eq!(old.len(), new.len());
        for (n, ((a, b), c)) in old.iter().zip(&again).zip(&new).enumerate() {
            assert!(lit(a) > 300, "{camera:?} frame {n} shows the scene ({} lit pixels)", lit(a));
            assert!(a == b, "{camera:?} control frame {n} differs: {}", describe(a, b));
            if a != c {
                differing.push(format!("{camera:?} frame {n} differs from the reference: {}", describe(a, c)));
            }
        }
        // The frames differ from one another (time and camera move), so equality is not an empty scene's.
        assert!(old[0] != old[5]);
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
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
