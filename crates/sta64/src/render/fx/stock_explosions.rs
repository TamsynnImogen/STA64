//! The Stock look's explosion of a destroyed unit or building (#1153): the sprite-cluster fireball of the
//! unit's `fireball` definition and the shockwave ring it names. The Remaster look (`RenderProfile::Refit`)
//! draws the fireball, ring, sparks and debris of `impacts.rs`, unchanged; under Stock those are not drawn
//! for a unit whose definition resolves ([`Effect::Explosions`]).
//!
//! The definitions come from content (`ContentDb::explosion_of`, #1279); the rules and their grades (read
//! from data: a key's presence; read from the executable, static: what the code does; measured: on the
//! original in a bench session):
//!
//! - **When.** At `SimEvent::Destroyed`, where the unit was last seen (the sim removes it in the same tick;
//!   the original's dying phase waits for #1100: INTERIM, the explosion starts at removal, which is when the
//!   original's fireball starts: read from the executable, static; measured). A unit and a building share the
//!   rule (read from the executable, static). Which definition: the unit's `fireball` (the `selfDestruct`
//!   explosion is for a self-destruct, which the event does not tell: INTERIM). A unit whose definition does
//!   not resolve keeps today's effect (INTERIM: the original's default definition name is not applied yet).
//! - **Fireball.** The model `dbName` names is a cluster of sprite nodes at fixed offsets, each a camera-facing
//!   additive sprite with its own flipbook, played once and then hidden (read from the executable, static;
//!   measured: once, then hidden; the mechanism, a sprite animation played without looping, is INTERIM). Its
//!   size does not depend on the unit (measured). It lives `length` seconds (read from the executable,
//!   static); without the key it lives as long as its sprites do (INTERIM, no default is documented). It
//!   turns a fixed angle per rendered frame about a random axis, so the turn depends on the frame rate
//!   (read from the executable, static; measured as movement). Node colour animations are not played
//!   (INTERIM). No light, no camera shake (read from the executable, static).
//! - **Shockwave.** Made once when the fireball's age passes `shockwaveDelay` and the unit's radius is above
//!   `shockwaveMinimum`, if the definition names a shockwave (read from the executable, static; measured for
//!   the delay). Absent delay or minimum: zero (INTERIM, no default is documented). Its ring lies in the
//!   world plane at the unit's place: `rays` rays, each three bands between one and two ring radii, one flat
//!   and two crossed above and below the plane; `u` mirrored from ray to ray, `v` from inner to outer edge
//!   (read from the executable, static; measured). The inner edge grows by `rate` per second and the outer
//!   edge by twice that (read from the executable, static; measured). It ends where the outer edge passes
//!   the unit's radius times `radiusFactor`, fading linearly to nothing from `fadeFactor` of that distance
//!   (read from the executable, static). It is the texture `texture` names times `color`, added; the gain on
//!   the colour is not established (INTERIM: one). The radius used is the unit's `SelectRadius` (INTERIM: the
//!   original's radius is its bounding sphere, and the measurements differ from the model's own).
//! - **Not drawn.** Hull chunks (cut from the unit's own meshes), death sparks and the hull drawn through the
//!   dying phase (they wait for #1100), the hull-hit node particle, the explosion's sound and its
//!   cinematic shot. Under Stock a hull hit draws no Refit spark either (those are not the original's).

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use sta_assets::a2fmt::spr::SpriteTable;
use sta_content::{ContentDb, ContentKey, ExplosionDef, ShockwaveDef};
use sta_sim::{EntityId, TICK_HZ};

use super::stock_shots::{ShotParams, StockShotMaterial};
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::profile::{Effect, ProfileEffects, RenderProfile};
use crate::render::sprites::{Billboard, SpriteTables};

/// The fireball's turn per rendered frame, radians (read from the executable, static).
pub const SPIN_PER_FRAME: f32 = 0.01;
/// The ring's height above and below its plane for the two crossed bands (read from the executable, static).
pub const CROSSED_HEIGHT: f32 = 2.0;
/// The gain on the ring's colour (INTERIM: not established).
pub const RING_GAIN: f32 = 1.0;

/// Whether the Stock explosion is the one drawn: under the Stock look with a stock implementation registered.
/// Without the profile resources (the windowless tests of other effects) it is not.
pub(super) fn stock_explosions(profile: Option<&RenderProfile>, effects: Option<&ProfileEffects>) -> bool {
    match (profile, effects) {
        (Some(p), Some(e)) => e.implementation(*p, Effect::Explosions) == RenderProfile::Stock,
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------------------------

/// What a destroyed unit asks for: its definitions, where and how big, and when (game time in ticks).
#[derive(Debug, Clone, PartialEq)]
pub struct ExplosionRequest {
    pub unit: EntityId,
    pub at: Vec3,
    pub radius: f32,
    pub born: f64,
    pub explosion: ExplosionDef,
    pub wave: Option<ShockwaveDef>,
}

/// The request for a unit of definition `def` destroyed at `at`, or none when its `fireball` does not resolve.
pub fn request(db: &ContentDb, def: ContentKey, unit: EntityId, at: Vec3, radius: f32, born: f64) -> Option<ExplosionRequest> {
    let explosion = db.explosion_of(db.unit(def)?)?.clone();
    let wave = explosion.shockwave.as_deref().filter(|w| !w.trim().is_empty() && !w.trim().eq_ignore_ascii_case("null")).and_then(|w| db.explosions().shockwave(w)).cloned();
    Some(ExplosionRequest { unit, at, radius, born, explosion, wave })
}

/// How long the fireball lives, seconds: `length`; without the key as long as its sprites do (`sprites_end`, the
/// longest of them; INTERIM: no default is documented).
pub fn fireball_life(def: &ExplosionDef, sprites_end: f32) -> f32 {
    def.length.map_or(sprites_end, |l| l.to_f32())
}

/// Whether the shockwave is due: the definition names one, the fireball is older than `shockwaveDelay` and the
/// unit's radius is above `shockwaveMinimum` (absent keys: zero, INTERIM).
pub fn shockwave_due(def: &ExplosionDef, has_wave: bool, age: f32, radius: f32) -> bool {
    has_wave && age > def.shockwave_delay.map_or(0.0, |d| d.to_f32()) && radius > def.shockwave_minimum.map_or(0.0, |m| m.to_f32())
}

/// The ring `t` seconds after it was made: the scale of its unit mesh and the factor on its colour, or none once it
/// has ended. A wave without `rate`, `radiusFactor` or `fadeFactor` has no ring (INTERIM: no default is documented).
pub fn ring_state(wave: &ShockwaveDef, radius: f32, t: f32) -> Option<(f32, f32)> {
    let (rate, factor, fade) = (wave.rate?.to_f32(), wave.radius_factor?.to_f32(), wave.fade_factor?.to_f32());
    let outer = 2.0 * rate * t;
    let max = radius * factor;
    if outer > max {
        return None;
    }
    let k = if outer > fade * max { (max - outer) / ((1.0 - fade) * max) } else { 1.0 };
    Some((rate * t, k.clamp(0.0, 1.0)))
}

/// A sprite's texture rectangle `age` seconds into a flipbook that plays once, and the age it ends at; none after
/// it ended. A sprite with no flipbook shows its rectangle for as long as the explosion lives.
pub fn once_uv(table: &SpriteTable, sprite: &sta_assets::a2fmt::spr::Sprite, age: f32) -> Option<[f32; 4]> {
    let end = flipbook_end(table, sprite);
    if end.is_some_and(|e| age >= e) {
        return None;
    }
    // The sprite's rectangle as `sta_assets::spr_anim::sprite_uv` takes it, then the offsets of its flipbook played
    // once (the shared evaluator with the play flags that do not loop).
    let r = sprite.uv_rect().unwrap_or_else(|| {
        let rw = sprite.reference[0].unwrap_or(256.0).max(1.0);
        let rh = sprite.reference[1].unwrap_or(rw).max(1.0);
        [sprite.rect[0] / rw, sprite.rect[1] / rh, sprite.rect[2] / rw, sprite.rect[3] / rh]
    });
    let mut off = [0.0f32; 2];
    for id in &sprite.anims {
        if let Some(a) = table.animation(id) {
            let o = sta_assets::spr_anim::offset_of(sta_assets::spr_anim::value(a, age, 0));
            off = [off[0] + o[0], off[1] + o[1]];
        }
    }
    Some([r[0] + off[0], r[1] + off[1], r[2], r[3]])
}

/// The age a sprite's flipbook ends at: the longest of its animations; none for a sprite without any.
pub fn flipbook_end(table: &SpriteTable, sprite: &sta_assets::a2fmt::spr::Sprite) -> Option<f32> {
    sprite.anims.iter().filter_map(|id| table.animation(id)).map(|a| a.length).fold(None, |m: Option<f32>, l| Some(m.map_or(l, |m| m.max(l))))
}

/// The ring's unit mesh: `rays` rays about the vertical axis, each three bands between radius one and two (one
/// flat, two crossed at [`CROSSED_HEIGHT`] above and below the plane); `u` is the ray's parity, `v` runs from the inner
/// to the outer edge. Scaled by `rate * t` in the plane it is the ring of the rules above.
pub fn ring_mesh(rays: u32) -> Mesh {
    let n = rays.max(1);
    let heights = [(0.0, 0.0), (CROSSED_HEIGHT, -CROSSED_HEIGHT), (-CROSSED_HEIGHT, CROSSED_HEIGHT)];
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    for i in 0..n {
        let a = std::f32::consts::TAU * i as f32 / n as f32;
        let (s, c) = a.sin_cos();
        for (inner, outer) in heights {
            for (m, y, v) in [(1.0, inner, 0.0), (2.0, outer, 1.0)] {
                positions.push([s * m, y, c * m]);
                uvs.push([(i & 1) as f32, v]);
            }
        }
    }
    let mut indices = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        for b in 0..3 {
            let (a_in, a_out) = (i * 6 + b * 2, i * 6 + b * 2 + 1);
            let (b_in, b_out) = (j * 6 + b * 2, j * 6 + b * 2 + 1);
            indices.extend([a_in, a_out, b_out, a_in, b_out, b_in]);
        }
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

// ---------------------------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------------------------

/// The requests of this frame, from `impacts.rs`.
#[derive(Resource, Default, Debug)]
pub struct ExplosionQueue(pub Vec<ExplosionRequest>);

/// One fireball. Tests key on `(unit, born)`, never on the Bevy `Entity`.
#[derive(Component, Debug, Clone)]
pub struct StockExplosion {
    pub unit: EntityId,
    /// Game time in ticks.
    pub born: f64,
    pub radius: f32,
    pub explosion: ExplosionDef,
    pub wave: Option<ShockwaveDef>,
    /// Seconds the cluster's sprites last, once the model is attached.
    pub sprites_end: f32,
    pub ring_made: bool,
    /// The turn per frame's axis.
    axis: Vec3,
    pub visuals: bool,
}

/// One ring.
#[derive(Component, Debug, Clone)]
pub struct StockRing {
    /// The unit whose explosion made it; read by the tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub unit: EntityId,
    pub born: f64,
    pub radius: f32,
    pub wave: ShockwaveDef,
    pub visuals: bool,
}

/// A sprite node of the cluster and the rectangle it shows.
#[derive(Component, Debug, Clone)]
pub struct ExplosionSprite {
    pub sprite: String,
}

/// A cluster node: sprite, offset from the cluster's centre, size, colour, camera-facing.
#[derive(Debug, Clone)]
struct ClusterNode {
    sprite: String,
    offset: Vec3,
    size: Vec2,
    colour: Vec3,
    billboard: bool,
    texture: Option<Handle<Image>>,
}

/// What is loaded: cluster models by `dbName`, textures by name, ring meshes by ray count.
#[derive(Resource, Default)]
pub struct ExplosionAssets {
    clusters: HashMap<String, Option<Arc<Vec<ClusterNode>>>>,
    textures: HashMap<String, Option<Handle<Image>>>,
    rings: HashMap<u32, Handle<Mesh>>,
    quad: Option<Handle<Mesh>>,
}

fn secs_since(clock: &AnimClock, born: f64) -> f32 {
    ((clock.now() * TICK_HZ as f64 - born) / TICK_HZ as f64) as f32
}

fn spawn_explosions(mut cmds: Commands, mut queue: ResMut<ExplosionQueue>) {
    for r in queue.0.drain(..) {
        let a = |salt| super::impacts::rand(r.unit, r.born as u64, 0, salt) * 2.0 - 1.0;
        let axis = Vec3::new(a(60), a(61), a(62)).try_normalize().unwrap_or(Vec3::Y);
        cmds.spawn((
            StockExplosion { unit: r.unit, born: r.born, radius: r.radius, explosion: r.explosion, wave: r.wave, sprites_end: 0.0, ring_made: false, axis, visuals: false },
            Transform::from_translation(r.at),
            Visibility::default(),
        ));
    }
}

/// Turn each fireball, make its ring when due and end it at its life.
fn update_explosions(mut cmds: Commands, clock: Res<AnimClock>, mut q: Query<(Entity, &mut StockExplosion, &mut Transform)>) {
    for (e, mut x, mut tf) in &mut q {
        let age = secs_since(&clock, x.born);
        // Without `length` its sprites' end is the life, known once they are attached.
        if (x.explosion.length.is_some() || x.visuals) && age >= fireball_life(&x.explosion, x.sprites_end) {
            cmds.entity(e).try_despawn();
            continue;
        }
        tf.rotation = Quat::from_axis_angle(x.axis, SPIN_PER_FRAME) * tf.rotation;
        if !x.ring_made && shockwave_due(&x.explosion, x.wave.is_some(), age, x.radius) {
            x.ring_made = true;
            if let Some(wave) = x.wave.clone() {
                cmds.spawn((StockRing { unit: x.unit, born: clock.now() * TICK_HZ as f64, radius: x.radius, wave, visuals: false }, Transform::from_translation(tf.translation), Visibility::default()));
            }
        }
    }
}

/// Place each ring and end it where the rules end it.
fn update_rings(mut cmds: Commands, clock: Res<AnimClock>, mut rings: Query<(Entity, &StockRing, &mut Transform, Option<&MeshMaterial3d<StockShotMaterial>>)>, mut mats: Option<ResMut<Assets<StockShotMaterial>>>) {
    for (e, ring, mut tf, material) in &mut rings {
        let t = secs_since(&clock, ring.born).max(0.0);
        let Some((scale, k)) = ring_state(&ring.wave, ring.radius, t) else {
            cmds.entity(e).try_despawn();
            continue;
        };
        tf.scale = Vec3::new(scale.max(1e-4), 1.0, scale.max(1e-4));
        if let (Some(m), Some(c), Some(mats)) = (material, ring.wave.color, mats.as_mut())
            && let Some(mut mat) = mats.get_mut(&m.0)
        {
            let tint = Vec3::new(c[0].to_f32(), c[1].to_f32(), c[2].to_f32()) * (RING_GAIN * k);
            mat.params.tint = tint.extend(1.0);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Assets
// ---------------------------------------------------------------------------------------------

fn texture(assets: &mut ExplosionAssets, loader: &mut crate::render::textures::LoaderParams, name: &str) -> Option<Handle<Image>> {
    let key = name.to_ascii_lowercase();
    assets.textures.entry(key).or_insert_with(|| loader.loader().colour(name, false)).clone()
}

fn mirror(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], -p[2])
}

/// The sprite nodes of a model, with their textures.
fn cluster(model: &sta_assets::model::Model, table: &SpriteTable, assets: &mut ExplosionAssets, loader: &mut crate::render::textures::LoaderParams) -> Vec<ClusterNode> {
    let mut out = Vec::new();
    for (i, node) in model.nodes.iter().enumerate() {
        if !matches!(node.kind, sta_assets::model::NodeKind::Sprite) {
            continue;
        }
        let Some(def) = table.sprite_node(node.sprite_node_id()) else { continue };
        let Some(sprite) = table.sprite(&def.sprite) else { continue };
        out.push(ClusterNode {
            sprite: def.sprite.clone(),
            offset: mirror(model.model_transform(i).transform_point([0.0; 3])),
            size: Vec2::new(def.size[0], def.size[1]),
            colour: Vec3::from(def.colour),
            billboard: def.billboard,
            texture: texture(assets, loader, &sprite.texture),
        });
    }
    out
}

/// Give each new fireball its sprite nodes (loading the model of its `dbName` once) and each new ring its mesh and texture.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn attach_visuals(
    mut cmds: Commands,
    vfs: Res<crate::viewer::Mounted>,
    tables: Res<SpriteTables>,
    mut loader: crate::render::textures::LoaderParams,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StockShotMaterial>>,
    mut assets: ResMut<ExplosionAssets>,
    mut fireballs: Query<(Entity, &mut StockExplosion)>,
    mut rings: Query<(Entity, &mut StockRing)>,
) {
    for (e, mut x) in &mut fireballs {
        if x.visuals {
            continue;
        }
        x.visuals = true;
        let Some(name) = x.explosion.model.clone() else { continue };
        let key = name.to_ascii_lowercase();
        if !assets.clusters.contains_key(&key) {
            let nodes = match super::mining_beam::load_named_model(&vfs.0, &name, "stock explosion model") {
                Some((_, model)) => Some(Arc::new(cluster(&model, &tables.0, &mut assets, &mut loader))),
                None => {
                    warn!("stock explosions: the model {name} is not in the mounted files; the fireball is not drawn");
                    None
                }
            };
            assets.clusters.insert(key.clone(), nodes);
        }
        let Some(nodes) = assets.clusters[&key].clone() else { continue };
        let quad = assets.quad.get_or_insert_with(|| meshes.add(crate::render::sprites::unit_quad())).clone();
        let mut end = 0.0f32;
        for n in nodes.iter() {
            let Some(texture) = n.texture.clone() else { continue };
            if let Some(sprite) = tables.0.sprite(&n.sprite) {
                end = end.max(flipbook_end(&tables.0, sprite).unwrap_or(0.0));
            }
            let params = ShotParams { rect: Vec4::ZERO, flags: Vec4::ZERO, tint: n.colour.extend(1.0) };
            let scale = if n.billboard { Vec3::new(n.size.x * 2.0, n.size.y * 2.0, 1.0) } else { Vec3::new(n.size.x, n.size.y, 1.0) };
            let mut c = cmds.spawn((
                ExplosionSprite { sprite: n.sprite.clone() },
                Mesh3d(quad.clone()),
                MeshMaterial3d(mats.add(StockShotMaterial { params, texture })),
                Transform::from_translation(n.offset).with_scale(scale),
                bevy::camera::visibility::NoFrustumCulling,
                ChildOf(e),
            ));
            if n.billboard {
                c.insert(Billboard);
            }
        }
        x.sprites_end = end;
    }
    for (e, mut ring) in &mut rings {
        if ring.visuals {
            continue;
        }
        ring.visuals = true;
        let Some(name) = ring.wave.texture.clone() else { continue };
        let Some(texture) = texture(&mut assets, &mut loader, &name) else {
            warn!("stock explosions: the shockwave texture {name} is not in the mounted files; the ring is not drawn");
            continue;
        };
        let rays = ring.wave.rays.unwrap_or(0);
        if rays == 0 {
            continue;
        }
        let mesh = assets.rings.entry(rays).or_insert_with(|| meshes.add(ring_mesh(rays))).clone();
        let params = ShotParams { rect: Vec4::new(0.0, 0.0, 1.0, 1.0), flags: Vec4::ZERO, tint: Vec4::ZERO };
        cmds.entity(e).try_insert((Mesh3d(mesh), MeshMaterial3d(mats.add(StockShotMaterial { params, texture })), bevy::camera::visibility::NoFrustumCulling));
    }
}

/// Show each sprite node's flipbook cell at the fireball's age; hide it once its flipbook has played.
fn animate_sprites(
    clock: Res<AnimClock>,
    tables: Res<SpriteTables>,
    fireballs: Query<&StockExplosion>,
    mut sprites: Query<(&ExplosionSprite, &ChildOf, &MeshMaterial3d<StockShotMaterial>, &mut Visibility)>,
    mut mats: ResMut<Assets<StockShotMaterial>>,
) {
    for (s, parent, m, mut vis) in &mut sprites {
        let Ok(x) = fireballs.get(parent.parent()) else { continue };
        let age = secs_since(&clock, x.born);
        let uv = tables.0.sprite(&s.sprite).and_then(|sp| once_uv(&tables.0, sp, age));
        let want = if uv.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if let Some(uv) = uv
            && let Some(mut mat) = mats.get_mut(&m.0)
        {
            let rect = Vec4::from(uv);
            if mat.params.rect != rect {
                mat.params.rect = rect;
            }
        }
    }
}

/// Everything the drawing needs is there (a windowless app has none of it).
#[allow(clippy::too_many_arguments)]
fn visuals_ready(
    vfs: Option<Res<crate::viewer::Mounted>>,
    tables: Option<Res<SpriteTables>>,
    cache: Option<Res<crate::render::textures::TextureCache>>,
    meshes: Option<Res<Assets<Mesh>>>,
    mats: Option<Res<Assets<StockShotMaterial>>>,
    images: Option<Res<Assets<Image>>>,
) -> bool {
    vfs.is_some() && tables.is_some() && cache.is_some() && meshes.is_some() && mats.is_some() && images.is_some()
}

/// Without assets there is nothing to attach: the fireballs and rings are what the rules say and nothing more.
fn skip_visuals(mut fireballs: Query<&mut StockExplosion>, mut rings: Query<&mut StockRing>) {
    for mut x in &mut fireballs {
        x.visuals = true;
    }
    for mut r in &mut rings {
        r.visuals = true;
    }
}

pub struct StockExplosionsPlugin;

impl Plugin for StockExplosionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProfileEffects>();
        app.world_mut().resource_mut::<ProfileEffects>().register_stock(Effect::Explosions);
        app.init_resource::<ExplosionQueue>().init_resource::<ExplosionAssets>();
        app.add_systems(
            Update,
            (
                spawn_explosions,
                update_explosions,
                (attach_visuals, animate_sprites).chain().run_if(visuals_ready),
                skip_visuals.run_if(not(visuals_ready)),
                update_rings,
            )
                .chain()
                .after(super::impacts::spawn_impacts)
                .in_set(GameSet::Interpolate),
        );
    }
}

#[cfg(test)]
#[path = "stock_explosions_tests.rs"]
mod tests;
