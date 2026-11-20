//! Real-GPU identity check of the sprite nodes' materials: the same invented scene (nebula-like
//! puffs with colour cycles and fades, a blinking glow, a flat sprite, a moon nimbus) drawn through
//! the per-node path ([`super::tests::animate_reference`]) and through the shared
//! one, at a new time and camera place every frame, with a cloaking unit and a placement ghost whose
//! nodes the game's own systems give material copies; the two sets of frames must be byte-equal.
//! Needs `STA64_GPU_TESTS=1` and a hardware adapter; skips otherwise.
use super::tests::{NODES, TABLE, animate_reference, fade_of, material_of, table};
use super::*;
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

static SERIAL: Mutex<()> = Mutex::new(());

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

/// Frames of the scene at `steps` times and camera places; `shared` selects the new path.
fn series(shared: bool) -> Option<Vec<Vec<u8>>> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    app.insert_resource(ClearColor(Color::BLACK)).insert_resource(table()).insert_resource(AnimClock::new(ClockMode::Bench { step: 0.137 }, 0));
    // The clock moves on every update, as in the game. The ghost's and the fade's own systems run
    // as in the game (they hand nodes material copies); the ghost's first, the worst case for sharing.
    fn tick(mut clock: ResMut<AnimClock>) {
        clock.advance_idle(0.0);
    }
    app.add_systems(Update, tick.before(animate_reference).before(animate));
    if shared {
        app.add_systems(Update, (tint_ghost, share_materials, animate, apply_fade, face_camera).chain());
    } else {
        app.add_systems(Update, (tint_ghost, animate_reference, apply_fade, face_camera).chain());
    }
    let mut target = Image::new_target_texture(160, 120, TextureFormat::Rgba8UnormSrgb, None);
    target.asset_usage = bevy::asset::RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target);
    let tex = app.world_mut().resource_mut::<Assets<Image>>().add(texture());
    let cam = app
        .world_mut()
        .spawn((Camera3d::default(), Msaa::Off, RenderTarget::from(target.clone()), Transform::from_xyz(0.0, 0.0, 60.0).looking_at(Vec3::ZERO, Vec3::Y)))
        .id();
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
    Some(frames)
}

#[test]
fn nodes_drawn_through_shared_materials_are_byte_equal_to_the_per_node_path_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _ = TABLE;
    let Some(old) = series(false) else { return };
    let Some(new) = series(true) else { return };
    // Control: the per-node path against itself, so a difference below is the sharing's, not the harness'.
    let Some(again) = series(false) else { return };
    for (n, (a, b)) in old.iter().zip(&again).enumerate() {
        let mx = a.iter().zip(&new[n]).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        let px = a.chunks(4).zip(new[n].chunks(4)).filter(|(p, q)| p != q).count();
        let first = a.chunks(4).zip(new[n].chunks(4)).position(|(p, q)| p != q).unwrap_or(0);
        eprintln!("frame {n}: max diff {mx}, {px} px of {}, first at ({}, {}) old {:?} new {:?}", a.len() / 4, first % 160, first / 160, &a[first * 4..first * 4 + 4], &new[n][first * 4..first * 4 + 4]);
        eprintln!("control frame {n}: {} bytes differ; vs shared: {}", a.iter().zip(b).filter(|(x, y)| x != y).count(), a.iter().zip(&new[n]).filter(|(x, y)| x != y).count());
    }
    assert_eq!(old.len(), new.len());
    let lit = |f: &Vec<u8>| f.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
    for (n, (a, b)) in old.iter().zip(&new).enumerate() {
        assert!(lit(a) > 300, "frame {n} shows the scene ({} lit pixels)", lit(a));
        assert!(a == b, "frame {n} differs: {} bytes of {}", a.iter().zip(b).filter(|(x, y)| x != y).count(), a.len());
    }
    // The frames differ from one another (time and camera move), so equality is not an empty scene's.
    assert!(old[0] != old[5]);
}
