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
#[require(SpriteKeys)]
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

/// A sprite node's animation and sprite, resolved to [`animate`]'s per-frame values, and whether it
/// had a fade and a nimbus at its last pass (a removed component is not a change Bevy reports).
/// Every sprite node gets one with its [`SpriteFx`]; only [`animate`] reads or writes it.
#[derive(Component, Default, Debug)]
pub struct SpriteKeys {
    /// `(animation, sprite)` indices into [`Played`], once resolved from the node's names.
    keys: Option<(Option<u32>, u32)>,
    /// The [`Played::generation`] the indices belong to.
    generation: u32,
    /// `(fade, nimbus)` present at the node's last pass.
    had: (bool, bool),
    /// Full passes so far, so tests can see that nodes were passed over.
    #[cfg(test)]
    pub(super) passes: u32,
}

/// An animation's value or a sprite's uv this frame, with the bits it had in the frame before.
struct Entry<T> {
    name: String,
    now: T,
    bits: [u32; 7],
    /// The bits differ from the last run's (or the entry is new this frame).
    moved: bool,
}

/// What every node of an animation or sprite shares in a frame: [`spr_anim::value`] and
/// [`spr_anim::sprite_uv`] are functions of the table, the name and the time only, so each is
/// evaluated once per name and frame instead of once per node (#1236).
#[derive(Default)]
pub struct Played {
    anim_ix: std::collections::HashMap<String, u32>,
    /// The value `main` matches on: `None` when the table has no such animation or it gives none.
    anims: Vec<Entry<Option<Value>>>,
    sprite_ix: std::collections::HashMap<String, u32>,
    /// `None` when the table has no such sprite.
    sprites: Vec<Entry<Option<Affine2>>>,
    /// Counts the tables seen: a new one clears the names, and indices of an older one are resolved
    /// again.
    generation: u32,
}

/// The bits of a value as an [`Entry`] keeps them, with a tag for the variant.
fn value_bits(v: Option<Value>) -> [u32; 7] {
    match v {
        None => [0; 7],
        Some(Value::Draw(b)) => [1, u32::from(b), 0, 0, 0, 0, 0],
        Some(Value::Colour(c)) => [2, c[0].to_bits(), c[1].to_bits(), c[2].to_bits(), 0, 0, 0],
        Some(Value::Offset(o)) => [3, o[0].to_bits(), o[1].to_bits(), 0, 0, 0, 0],
        Some(Value::U(x)) => [4, x.to_bits(), 0, 0, 0, 0, 0],
        Some(Value::V(x)) => [5, x.to_bits(), 0, 0, 0, 0, 0],
        Some(Value::Width(x)) => [6, x.to_bits(), 0, 0, 0, 0, 0],
    }
}

/// The bits of a uv as an [`Entry`] keeps them, with a tag for `None`.
fn uv_bits(uv: Option<Affine2>) -> [u32; 7] {
    let mut b = [0; 7];
    if let Some(a) = uv {
        b[0] = 1;
        b[1..].copy_from_slice(&a.to_cols_array().map(f32::to_bits));
    }
    b
}

impl Played {
    fn anim_now(table: &SpriteTable, name: &str, t: f32) -> Option<Value> {
        table.animation(name).and_then(|a| spr_anim::value(a, t, spr_anim::LOOP))
    }

    fn uv_now(table: &SpriteTable, name: &str, t: f32) -> Option<Affine2> {
        table.sprite(name).map(|s| uv_affine(spr_anim::sprite_uv(table, s, t)))
    }

    /// This frame's values of every known name. A new table (`all`) drops every name instead: every
    /// node runs in that frame anyway and resolves its names again, so names only an earlier table
    /// had are not played on.
    fn refresh(&mut self, table: &SpriteTable, t: f32, all: bool) {
        if all {
            *self = Self { generation: self.generation.wrapping_add(1), ..Self::default() };
            return;
        }
        for e in &mut self.anims {
            e.now = Self::anim_now(table, &e.name, t);
            let bits = value_bits(e.now);
            e.moved = all || bits != e.bits;
            e.bits = bits;
        }
        for e in &mut self.sprites {
            e.now = Self::uv_now(table, &e.name, t);
            let bits = uv_bits(e.now);
            e.moved = all || bits != e.bits;
            e.bits = bits;
        }
    }

    /// A node's indices, its names added (with this frame's value, moved) when new.
    fn resolve(&mut self, table: &SpriteTable, t: f32, f: &SpriteFx) -> (Option<u32>, u32) {
        let anim = f.node_anim.as_deref().map(|name| match self.anim_ix.get(name) {
            Some(&i) => i,
            None => {
                let now = Self::anim_now(table, name, t);
                let i = self.anims.len() as u32;
                self.anims.push(Entry { name: name.to_owned(), now, bits: value_bits(now), moved: true });
                self.anim_ix.insert(name.to_owned(), i);
                i
            }
        });
        let sprite = match self.sprite_ix.get(&f.sprite) {
            Some(&i) => i,
            None => {
                let now = Self::uv_now(table, &f.sprite, t);
                let i = self.sprites.len() as u32;
                self.sprites.push(Entry { name: f.sprite.clone(), now, bits: uv_bits(now), moved: true });
                self.sprite_ix.insert(f.sprite.clone(), i);
                i
            }
        };
        (anim, sprite)
    }
}

/// Whether a node's state equals itself. A colour or uv with a NaN does not, so `main`'s comparison
/// finds it changed in every frame where the node is shown (slot written, look rewritten for a NaN
/// uv, change mark): such a node must run every frame, as on `main`.
#[allow(clippy::eq_op)]
fn settled(s: &material::SpriteState) -> bool {
    s.colour == s.colour && s.uv == s.uv
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
///
/// Work skipped (#1236), with what makes each skip exact:
/// - Each animation's value and each sprite's uv are computed once per frame ([`Played`]): they are
///   functions of the table, the name and the time, the same for every node.
/// - An adopted node is passed over in a frame where none of its inputs changed since the last run:
///   its animation's value and its sprite's uv (by bits), its `SpriteFx`, `SpriteFade`, `MoonNimbus`,
///   `Visibility`, `SpriteState` and look component (by Bevy's change ticks, which a system does not
///   see for its own writes), whether it has a fade and a nimbus, and the table. Its last pass ran on
///   these same inputs (each frame they changed it ran), and a pass leaves its outputs where a second
///   pass on the same inputs writes nothing: the visibility asked for, the state and slot at the
///   colour and uv (written only on change), the look component swapped or marked only on change.
///   The one output a node shares is its look's uv, which every shown node of the look sets to the
///   same value: a shown node passed over found it current at its last pass, and only a pass with
///   another uv for the sprite could have moved it since, which would have made the node's uv move.
///   So every write, slot, look and change mark of a frame is `main`'s. A node that is not adopted
///   runs every frame, as its own material may be edited by others; so does a node whose stored
///   colour or uv holds a NaN ([`settled`]), which `main` finds changed in every frame. A new table
///   runs every node and drops the names of the old one from [`Played`].
/// - Not skipped: nodes whose ancestors are hidden. Whether they are drawn this frame is known only
///   after `PostUpdate`'s visibility propagation, after this system, and a node that comes into view
///   must already hold this frame's slot, state and change mark; their blink must also be current for
///   that propagation. They cost what any node with the same inputs costs.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn animate(
    clock: Res<crate::clock::AnimClock>,
    tables: Res<SpriteTables>,
    mut fx: Query<(
        Ref<SpriteFx>,
        Option<Ref<SpriteFade>>,
        Option<Ref<crate::render::rock::MoonNimbus>>,
        &mut Visibility,
        Option<&mut material::SpriteState>,
        Option<&mut MeshMaterial3d<material::SpriteMaterial>>,
        &mut SpriteKeys,
    )>,
    flip: Query<&UvFlipbook>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut looks: Option<ResMut<Assets<material::SpriteMaterial>>>,
    slots: Option<Res<material::SpriteSlots>>,
    buffers: Option<ResMut<Assets<bevy::render::storage::ShaderBuffer>>>,
    mut writes: Local<Vec<(bevy::mesh::MeshTag, [f32; 4])>>,
    mut played: Local<Played>,
) {
    let t = clock.wall() as f32;
    let table = &tables.0;
    let parity = material::parity();
    let all = tables.is_changed();
    played.refresh(table, t, all);
    writes.clear();
    for (f, fade, nimbus, mut vis, state, mut drawn, mut keys) in &mut fx {
        let resolved = keys.keys.filter(|_| !f.is_changed() && keys.generation == played.generation);
        let (anim, sprite) = match resolved {
            Some(k) => k,
            None => {
                let k = played.resolve(table, t, &f);
                keys.keys = Some(k);
                keys.generation = played.generation;
                k
            }
        };
        let had = (fade.is_some(), nimbus.is_some());
        let quiet = state.is_some()
            && resolved.is_some()
            && !all
            && keys.had == had
            && !fade.as_ref().is_some_and(|k| k.is_changed())
            && !nimbus.as_ref().is_some_and(|n| n.is_changed())
            && !vis.is_changed()
            && !state.as_ref().is_some_and(|s| s.is_changed())
            && !drawn.as_ref().is_some_and(|d| d.is_changed())
            && !anim.is_some_and(|i| played.anims[i as usize].moved)
            && !played.sprites[sprite as usize].moved
            && state.as_deref().is_some_and(settled);
        if quiet {
            continue;
        }
        if keys.had != had {
            keys.had = had;
        }
        #[cfg(test)]
        {
            keys.passes += 1;
        }
        let mut col = f.colour;
        // A moon's nimbus is drawn under the Stock look only.
        let mut shown = nimbus.as_ref().is_none_or(|n| n.on);
        match anim.and_then(|i| played.anims[i as usize].now) {
            Some(Value::Draw(false)) => shown = false,
            Some(Value::Colour(c)) => col = c,
            _ => {}
        }
        if let Some(k) = &fade {
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
        let Some(uv) = played.sprites[sprite as usize].now else { continue };
        let c = colour(col);
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
