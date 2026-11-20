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

/// Dims a sprite node's colour (a nebula puff's near fade and occlusion, RND-05).
#[derive(Component)]
pub struct SpriteFade(pub f32);

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

/// A private copy of a sprite node's material, kept while its fade is not exactly 1 (a nebula puff
/// near the eye or behind another nebula): the shared material of its look carries the unfaded colour.
#[derive(Component)]
pub struct SpritePrivate(pub Handle<StandardMaterial>);

/// What decides a sprite node's material content, so equal nodes can share one material.
#[derive(Hash, PartialEq, Eq)]
pub struct ShareKey {
    texture: Option<AssetId<Image>>,
    alpha: (u8, u32),
    sprite: String,
    anim: Option<String>,
    colour: [u32; 3],
}

fn alpha_key(a: AlphaMode) -> (u8, u32) {
    match a {
        AlphaMode::Opaque => (0, 0),
        AlphaMode::Mask(c) => (1, c.to_bits()),
        AlphaMode::Blend => (2, 0),
        AlphaMode::Premultiplied => (3, 0),
        AlphaMode::AlphaToCoverage => (4, 0),
        AlphaMode::Add => (5, 0),
        AlphaMode::Multiply => (6, 0),
    }
}

fn colour_bits(c: [f32; 3]) -> [u32; 3] {
    c.map(f32::to_bits)
}

/// Sprite nodes with the same texture, blend, sprite, animation and colour draw the same picture
/// at every moment (colour and uv are functions of those and the clock): they share one material,
/// so one write a frame serves all of them (a map can hold very many nebula puffs). A node whose
/// fade is not 1 takes a private copy while it is ([`animate`]); the copies stay with their node
/// after the fade returns to 1 (one material per node that was ever faded). The cache holds one id
/// per look and only grows with the number of looks.
///
/// Runs on a node once, when it is added. A node whose [`MeshMaterial3d`] is already some other
/// system's copy (a placement ghost's tinted copy) keeps it; its `material` still becomes the shared
/// one, and the look is read from and registered as `material`, never from that copy.
pub fn share_materials(
    mut added: Query<(&mut SpriteFx, &mut MeshMaterial3d<StandardMaterial>), Added<SpriteFx>>,
    // The ids of the shared materials, not handles: the cache does not keep a look alive that no
    // node uses any more (a dead id fails `get_strong_handle`, and the next node makes a new look).
    mut known: Local<std::collections::HashMap<ShareKey, AssetId<StandardMaterial>>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (mut f, mut mm) in &mut added {
        let Some(m) = materials.get(&f.material) else { continue };
        let key = ShareKey {
            texture: m.base_color_texture.as_ref().map(|h| h.id()),
            alpha: alpha_key(m.alpha_mode),
            sprite: f.sprite.clone(),
            anim: f.node_anim.clone(),
            colour: colour_bits(f.colour),
        };
        let alive = known.get(&key).and_then(|id| materials.get_strong_handle(*id));
        match alive {
            Some(shared) => {
                if mm.0 == f.material {
                    mm.0 = shared.clone();
                }
                f.material = shared;
            }
            None => {
                known.insert(key, f.material.id());
            }
        }
    }
}

/// Advance sprite-node visibility, colour and uv, and sprite flipbooks.
///
/// The colour and uv go into the node's [`SpriteFx::material`], shared by all nodes of a look
/// ([`share_materials`]), once per look and frame, in the same frame as the other animation so
/// extraction sees them at once. A node whose [`SpriteFade`] is not exactly 1 draws with a private
/// copy ([`SpritePrivate`]) that gets the faded colour. The node's [`MeshMaterial3d`] is only
/// swapped between its shared material and its private copy: when it holds anything else (the copy
/// the cloak and build fade or the placement ghost gave it) the node is left alone, and a faded
/// node writes nothing, as that copy is what draws.
#[allow(clippy::type_complexity)]
pub fn animate(
    mut commands: Commands,
    clock: Res<crate::clock::AnimClock>,
    tables: Res<SpriteTables>,
    mut fx: Query<(
        Entity,
        &SpriteFx,
        Option<&SpriteFade>,
        Option<&SpritePrivate>,
        Option<&crate::render::rock::MoonNimbus>,
        &mut Visibility,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
    flip: Query<&UvFlipbook>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t = clock.wall() as f32;
    let table = &tables.0;
    // Per look and frame: the colour its animation gives and whether it is drawn at all.
    let mut looks: std::collections::HashMap<(&str, Option<&str>, [u32; 3]), (bool, [f32; 3])> = std::collections::HashMap::new();
    for (e, f, fade, private, nimbus, mut vis, mut mm) in &mut fx {
        let (drawn, col) = *looks.entry((f.sprite.as_str(), f.node_anim.as_deref(), colour_bits(f.colour))).or_insert_with(|| {
            let mut col = f.colour;
            let mut drawn = true;
            if let Some(a) = f.node_anim.as_deref().and_then(|a| table.animation(a)) {
                match spr_anim::value(a, t, spr_anim::LOOP) {
                    Some(Value::Draw(false)) => drawn = false,
                    Some(Value::Colour(c)) => col = c,
                    _ => {}
                }
            }
            (drawn, col)
        });
        // A moon's nimbus is drawn under the Stock look only.
        let shown = drawn && nimbus.is_none_or(|n| n.on);
        let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if !shown {
            continue;
        }
        let Some(sprite) = table.sprite(&f.sprite) else { continue };
        let k = fade.map_or(1.0, |k| k.0);
        let own = mm.0 == f.material || private.is_some_and(|p| mm.0 == p.0);
        let target = if k == 1.0 {
            f.material.clone()
        } else if !own {
            continue;
        } else if let Some(p) = private {
            p.0.clone()
        } else {
            let Some(base) = materials.get(&f.material).cloned() else { continue };
            let h = materials.add(base);
            commands.entity(e).insert(SpritePrivate(h.clone()));
            h
        };
        if own && mm.0 != target {
            mm.0 = target.clone();
        }
        // Write only on change: a write re-uploads the material.
        let (c, uv) = (colour(if k == 1.0 { col } else { col.map(|c| c * k) }), uv_affine(spr_anim::sprite_uv(table, sprite, t)));
        if materials.get(&target).is_some_and(|m| m.base_color != c || m.uv_transform != uv)
            && let Some(mut m) = materials.get_mut(&target)
        {
            m.base_color = c;
            m.uv_transform = uv;
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
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
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

#[cfg(test)]
mod tests;
#[cfg(test)]
mod gpu_tests;
