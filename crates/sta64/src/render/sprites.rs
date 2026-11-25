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
    /// The node's own material, as spawned. Adoption (ADR 0009, [`material`]) builds the node's look
    /// from it and then drops it (`Handle::default()`); a node that is not adopted keeps it.
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

/// Advance sprite-node animations and sprite flipbooks.
///
/// An adopted node (ADR 0009) keeps `main`'s rule, only while shown and only on change, with its
/// colour going to its slot and its uv to its look's shared material (written at most once per look
/// and frame, as every shown node of a look has the same uv); with parity mode (S5) its material
/// component is marked changed whenever its own material would have been written on `main`. A node
/// that is not adopted (an app without rendering) has its own material written, as on `main`.
///
/// A look's uv write re-queues every node drawn through it. For a node drawn in the frame before,
/// that is exactly when its own uv changes on `main` (it kept the look current itself). A node coming
/// back from hidden can find its look moved on while its own last uv equals the current one, so it is
/// re-queued where `main` would not re-queue it; Bevy queues newly visible entities first and once
/// (`iter_to_queue`, read from the Bevy 0.20.1 source), so its place in the queue is the same either
/// way. `tests::adopted_nodes_are_re_queued_in_the_frames_mains_own_materials_change` holds this.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn animate(
    clock: Res<crate::clock::AnimClock>,
    tables: Res<SpriteTables>,
    mut fx: Query<(
        &SpriteFx,
        Option<&SpriteFade>,
        Option<&crate::render::rock::MoonNimbus>,
        &mut Visibility,
        Option<&mut material::SpriteState>,
        Option<&mut MeshMaterial3d<material::SpriteMaterial>>,
    )>,
    flip: Query<&UvFlipbook>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut looks: Option<ResMut<Assets<material::SpriteMaterial>>>,
    slots: Option<Res<material::SpriteSlots>>,
    buffers: Option<ResMut<Assets<bevy::render::storage::ShaderBuffer>>>,
    mut writes: Local<Vec<(bevy::mesh::MeshTag, [f32; 4])>>,
) {
    let t = clock.wall() as f32;
    let table = &tables.0;
    let parity = material::parity();
    writes.clear();
    for (f, fade, nimbus, mut vis, state, mut drawn) in &mut fx {
        let mut col = f.colour;
        // A moon's nimbus is drawn under the Stock look only.
        let mut shown = nimbus.is_none_or(|n| n.on);
        if let Some(a) = f.node_anim.as_deref().and_then(|a| table.animation(a)) {
            match spr_anim::value(a, t, spr_anim::LOOP) {
                Some(Value::Draw(false)) => shown = false,
                Some(Value::Colour(c)) => col = c,
                _ => {}
            }
        }
        if let Some(k) = fade {
            col = col.map(|c| c * k.0);
        }
        let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if !shown {
            continue;
        }
        // Write only on change: a write re-uploads the material or the slot buffer, and a
        // map has thousands of these, most of them static.
        let Some(sprite) = table.sprite(&f.sprite) else { continue };
        let (c, uv) = (colour(col), uv_affine(spr_anim::sprite_uv(table, sprite, t)));
        let Some(mut st) = state else {
            if materials.get(&f.material).is_some_and(|m| m.base_color != c || m.uv_transform != uv)
                && let Some(mut m) = materials.get_mut(&f.material)
            {
                m.base_color = c;
                m.uv_transform = uv;
            }
            continue;
        };
        let Some(looks) = looks.as_deref_mut() else { continue };
        // The look shows the uv of its shown nodes; the first of them this frame writes it.
        if looks.get(&st.look).is_some_and(|l| l.base.uv_transform != uv)
            && let Some(mut l) = looks.get_mut(&st.look)
        {
            l.base.uv_transform = uv;
        }
        if c != st.colour || uv != st.uv {
            if c != st.colour {
                writes.push((st.tag.clone(), material::linear(c)));
            }
            st.colour = c;
            st.uv = uv;
            if let Some(d) = drawn.as_mut() {
                if d.0 != st.look {
                    // A node spawned on a private copy (another uv than its look's) joins its look
                    // only in a frame where its own material would change on `main`, so the swap
                    // re-queues it when `main` does. Until then the copy keeps showing the node's
                    // uv, which has not changed; from now on the look shows it (written above).
                    d.0 = st.look.clone();
                } else if parity {
                    d.set_changed();
                }
            }
        }
    }
    if let (Some(slots), Some(mut buffers)) = (slots, buffers) {
        slots.write(&mut buffers, writes.iter().map(|(tag, c)| (tag, *c)));
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
        // Written only when its bits change, so a still camera leaves the boards' transforms
        // unchanged and systems that watch them (the nebula board fade) can rest. The stored value
        // is the same either way.
        let rot = parent_rot.inverse() * cam_rot;
        if tf.rotation.to_array().map(f32::to_bits) != rot.to_array().map(f32::to_bits) {
            tf.rotation = rot;
        }
    }
}

pub mod material;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod gpu_tests;
