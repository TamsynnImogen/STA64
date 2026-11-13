//! Optional windowless proof with original synthetic hulls, art and font.
use super::*;
use crate::ui::sprite_font::{GlyphEntry, Page, SpriteFont};
use bevy::{
    camera::RenderTarget,
    render::{
        RenderPlugin,
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    window::ExitCondition,
};
use std::sync::{Arc, Mutex};

fn registry() -> sta_assets::Rgba {
    let width = 96 * 6;
    let mut pixels = vec![0; width * 7 * 4];
    for (code, rows) in [
        (b'N', [17, 25, 21, 19, 17, 17, 17]),
        (b'C', [14, 17, 16, 16, 16, 17, 14]),
        (b'-', [0, 0, 0, 31, 0, 0, 0]),
        (b'1', [4, 12, 4, 4, 4, 4, 14]),
        (b'7', [31, 1, 2, 4, 8, 8, 8]),
    ] {
        for (y, row) in rows.into_iter().enumerate() {
            for x in 0..5 {
                if row & (1 << (4 - x)) != 0 {
                    let i = (y * width + (code - b' ') as usize * 6 + x) * 4;
                    pixels[i..i + 4].copy_from_slice(&[255; 4]);
                }
            }
        }
    }
    let font = SpriteFont {
        name: "synthetic_registry".into(),
        reference: None,
        pages: vec![Page {
            texture: "original_fixture".into(),
            height: 7.0,
            glyphs: (0..96)
                .map(|i| GlyphEntry {
                    x: (i * 6) as f32,
                    y: 0.0,
                    w: 5.0,
                })
                .collect(),
        }],
    };
    text::rasterize(
        &font,
        &[sta_assets::Rgba {
            width: width as u32,
            height: 7,
            pixels,
            has_alpha: true,
        }],
        "NCC-17",
        [255; 4],
    )
    .unwrap()
}

fn hull() -> Vec<SurfaceTriangle> {
    let a = Vec3::new(-7.0, -3.0, -1.5);
    let b = Vec3::new(-7.0, 3.0, -1.5);
    let c = Vec3::new(0.0, -3.0, 0.0);
    let d = Vec3::new(0.0, 3.0, 0.0);
    let e = Vec3::new(7.0, -3.0, -1.5);
    let f = Vec3::new(7.0, 3.0, -1.5);
    [[a, b, c], [b, d, c], [c, d, e], [d, f, e]]
        .into_iter()
        .map(|positions| {
            let n = (positions[1] - positions[0])
                .cross(positions[2] - positions[0])
                .normalize();
            SurfaceTriangle {
                positions,
                normals: [n; 3],
            }
        })
        .collect()
}

fn projected_mesh(p: projection::ProjectedMesh) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, p.positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, p.uvs)
    .with_inserted_indices(Indices::U32(p.indices))
}

#[test]
#[ignore = "requires windowless Vulkan; no game install or GUI"]
fn projected_image_and_registry_text_draw_on_curved_hulls_and_hide_with_receiver() {
    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic);
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
            .disable::<bevy::winit::WinitPlugin>(),
    );
    app.insert_resource(ClearColor(Color::BLACK));
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            512,
            256,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, -28.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let triangles = hull();
    let surface = projection::ProjectedMesh {
        positions: triangles
            .iter()
            .flat_map(|t| t.positions.map(|v| v.to_array()))
            .collect(),
        normals: triangles
            .iter()
            .flat_map(|t| t.normals.map(|v| v.to_array()))
            .collect(),
        // Deliberately unusable source UVs: projector must provide its own.
        uvs: vec![[9.0, -4.0]; 12],
        indices: (0..12).collect(),
    };
    let hull_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(projected_mesh(surface));
    let projected = project_surface(
        &triangles,
        &Projector {
            receiver_to_projector: Mat4::IDENTITY,
            width: 10.0,
            height: 4.0,
            depth: 4.0,
            facing_threshold: 0.15,
            normal_bias: 0.003,
            flip_u: false,
        },
    )
    .unwrap();
    assert!(
        projected.indices.len() > 12,
        "must clip across both angled faces"
    );
    let decal_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(projected_mesh(projected));
    let lettering = project_surface(
        &triangles,
        &Projector {
            receiver_to_projector: Mat4::IDENTITY,
            width: 10.0,
            height: 4.0,
            depth: 4.0,
            facing_threshold: 0.15,
            normal_bias: 0.003,
            flip_u: true,
        },
    )
    .unwrap();
    let lettering_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(projected_mesh(lettering));
    let mut art = sta_assets::Rgba {
        width: 16,
        height: 8,
        pixels: vec![0; 16 * 8 * 4],
        has_alpha: true,
    };
    for y in 0..8 {
        for x in 0..16 {
            if x == y * 2 || x + 1 == y * 2 || x == 15 - y * 2 {
                let i = (y * 16 + x) * 4;
                art.pixels[i..i + 4].copy_from_slice(&[255; 4]);
            }
        }
    }
    let images = [art, registry()];
    let mut roots = Vec::new();
    let mut decals = Vec::new();
    for (index, pixels) in images.into_iter().enumerate() {
        let root = app
            .world_mut()
            .spawn((
                Transform::from_xyz(if index == 0 { -9.0 } else { 9.0 }, 0.0, 0.0),
                Visibility::Inherited,
            ))
            .id();
        roots.push(root);
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: if index == 0 {
                    Color::srgb(0.03, 0.06, 0.65)
                } else {
                    Color::srgb(0.03, 0.55, 0.06)
                },
                unlit: true,
                ..default()
            });
        app.world_mut().spawn((
            Mesh3d(hull_mesh.clone()),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            ChildOf(root),
        ));
        let texture = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(upload(&sta_assets::pixels::generate_mips(&pixels), true));
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color_texture: Some(texture),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            });
        decals.push(
            app.world_mut()
                .spawn((
                    Mesh3d(if index == 0 {
                        decal_mesh.clone()
                    } else {
                        lettering_mesh.clone()
                    }),
                    MeshMaterial3d(material),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    ChildOf(root),
                ))
                .id(),
        );
    }
    app.finish();
    app.cleanup();
    let frames = Arc::new(Mutex::new(Vec::new()));
    for stage in ["base", "projected", "receiver-hidden"] {
        if stage == "projected" {
            for e in &decals {
                app.world_mut().entity_mut(*e).insert(Visibility::Inherited);
            }
        }
        if stage == "receiver-hidden" {
            app.world_mut()
                .entity_mut(roots[0])
                .insert(Visibility::Hidden);
        }
        for _ in 0..12 {
            app.update();
        }
        let before = frames.lock().unwrap().len();
        let sink = frames.clone();
        app.world_mut()
            .spawn(Screenshot::image(target.clone()))
            .observe(move |shot: On<ScreenshotCaptured>| {
                sink.lock().unwrap().push((stage, shot.image.clone()));
            });
        for _ in 0..120 {
            app.update();
            if frames.lock().unwrap().len() > before {
                break;
            }
        }
        assert!(
            frames.lock().unwrap().len() > before,
            "missing {stage} frame"
        );
    }
    let mut counts = HashMap::new();
    for (stage, image) in frames.lock().unwrap().iter() {
        let dynamic = image.clone().try_into_dynamic().unwrap();
        let rgba = dynamic.to_rgba8();
        let blue = rgba
            .pixels()
            .filter(|p| {
                p[2] > 60 && p[2] > p[0].saturating_add(30) && p[2] > p[1].saturating_add(30)
            })
            .count();
        let green = rgba
            .pixels()
            .filter(|p| {
                p[1] > 60 && p[1] > p[0].saturating_add(30) && p[1] > p[2].saturating_add(30)
            })
            .count();
        let white = rgba
            .pixels()
            .filter(|p| p[0] > 150 && p[1] > 150 && p[2] > 150)
            .count();
        eprintln!("projected hull {stage}: blue={blue} green={green} registry/art={white}");
        counts.insert(*stage, (blue, green, white));
        if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
            std::fs::create_dir_all(&dir).unwrap();
            dynamic
                .save(std::path::Path::new(&dir).join(format!("projected-hulls-{stage}.png")))
                .unwrap();
        }
    }
    assert!(counts["base"].0 > 100 && counts["base"].1 > 100);
    assert_eq!(counts["base"].2, 0);
    assert!(
        counts["projected"].2 > 40,
        "image/text projection absent: {counts:?}"
    );
    assert_eq!(
        counts["receiver-hidden"].0, 0,
        "receiver or projected art floats after hide"
    );
    assert_eq!(counts["receiver-hidden"].1, counts["projected"].1);
    assert!(
        counts["receiver-hidden"].2 > 10 && counts["receiver-hidden"].2 < counts["projected"].2
    );
}
