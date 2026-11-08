//! SOD sprite nodes (running lights, strobes, glows) and texture flipbooks, from
//! the `.spr` sprite tables, drawn the way the engine draws them:
//!
//! - a sprite node's name up to its first underscore is a `@sprite_node` id;
//! - a billboard is a camera-facing quad whose size is a half extent; any other
//!   sprite node is a quad of the full size in the node's own x-y plane;
//! - the node's `@animation` (`draw` blink, `colour` cycle) and the sprite's own
//!   flipbook play on one shared clock, so every ship's strobes blink in phase;
//! - `additive` sprites add, `alphathreshold` ones cut out at 0.5, others blend.
//!
//! Behaviour follows Armada II Refit's reading of the engine
//! (`render3d/src/mods/hulls/sprites.rs`, MIT).

use bevy::asset::RenderAssetUsages;
use bevy::math::Affine2;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use sta_assets::a2fmt::spr::SpriteTable;
use sta_assets::spr_anim::{self, SpriteBlend, Value};
use std::sync::Arc;

#[derive(Resource, Clone)]
pub struct SpriteTables(pub Arc<SpriteTable>);

/// A sprite-node quad and what animates it.
#[derive(Component)]
pub struct SpriteFx {
    pub node_anim: Option<String>,
    pub sprite: String,
    pub colour: [f32; 3],
    pub material: Handle<StandardMaterial>,
}

/// Faces the camera every frame.
#[derive(Component)]
pub struct Billboard;

/// A SOD mesh part whose texture scrolls with a sprite-table `@animation`
/// (SOD animation reference type 4).
#[derive(Component)]
pub struct UvFlipbook {
    pub animation: String,
    pub offset: f32,
    pub material: Handle<StandardMaterial>,
}

pub fn unit_quad() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[-0.5, 0.5, 0.0], [0.5, 0.5, 0.0], [0.5, -0.5, 0.0], [-0.5, -0.5, 0.0]],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
        .with_inserted_indices(Indices::U32(vec![0, 2, 1, 0, 3, 2]))
}

pub fn uv_affine(uv: [f32; 4]) -> Affine2 {
    Affine2::from_scale_angle_translation(Vec2::new(uv[2], uv[3]), 0.0, Vec2::new(uv[0], uv[1]))
}

pub fn alpha_mode(blend: SpriteBlend) -> AlphaMode {
    match blend {
        SpriteBlend::Add => AlphaMode::Add,
        SpriteBlend::Cut => AlphaMode::Mask(0.5),
        SpriteBlend::Alpha => AlphaMode::Blend,
    }
}

fn colour(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

/// Advance sprite-node animations and sprite flipbooks.
pub fn animate(
    time: Res<Time>,
    tables: Res<SpriteTables>,
    mut fx: Query<(&SpriteFx, &mut Visibility)>,
    flip: Query<&UvFlipbook>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t = time.elapsed_secs();
    let table = &tables.0;
    for (f, mut vis) in &mut fx {
        let mut col = f.colour;
        let mut shown = true;
        if let Some(a) = f.node_anim.as_deref().and_then(|a| table.animation(a)) {
            match spr_anim::value(a, t, spr_anim::LOOP) {
                Some(Value::Draw(false)) => shown = false,
                Some(Value::Colour(c)) => col = c,
                _ => {}
            }
        }
        let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if !shown {
            continue;
        }
        // Write only on change: a write re-uploads the material, and a map has
        // thousands of these, most of them static.
        if let Some(sprite) = table.sprite(&f.sprite) {
            let (c, uv) = (colour(col), uv_affine(spr_anim::sprite_uv(table, sprite, t)));
            if materials.get(&f.material).is_some_and(|m| m.base_color != c || m.uv_transform != uv)
                && let Some(mut m) = materials.get_mut(&f.material)
            {
                m.base_color = c;
                m.uv_transform = uv;
            }
        }
    }
    for f in &flip {
        if let Some(a) = table.animation(&f.animation) {
            let o = spr_anim::offset_of(spr_anim::value(a, t + f.offset, spr_anim::LOOP));
            let uv = Affine2::from_translation(Vec2::new(o[0], o[1]));
            if materials.get(&f.material).is_some_and(|m| m.uv_transform != uv)
                && let Some(mut m) = materials.get_mut(&f.material)
            {
                m.uv_transform = uv;
            }
        }
    }
}

/// Turn billboard quads to face the camera, whatever their parents' rotation.
pub fn face_camera(
    camera: Query<&GlobalTransform, With<Camera3d>>,
    parents: Query<&GlobalTransform, Without<Billboard>>,
    mut boards: Query<(&mut Transform, &ChildOf), With<Billboard>>,
) {
    let Ok(cam) = camera.single() else { return };
    let cam_rot = cam.compute_transform().rotation;
    for (mut tf, child_of) in &mut boards {
        let parent_rot = parents
            .get(child_of.parent())
            .map(|g| g.compute_transform().rotation)
            .unwrap_or(Quat::IDENTITY);
        tf.rotation = parent_rot.inverse() * cam_rot;
    }
}
