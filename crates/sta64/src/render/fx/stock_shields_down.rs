//! The Stock look's shields-down effect (#1041): while a craft's shield pool is held at zero, a set of
//! spinning bands lies on its shield ellipsoid. The Remaster look draws nothing of this (its `ShieldsDown`
//! shell is `impacts.rs`'; the gate is [`Effect::ShieldHits`], registered by `stock_shield_hits.rs`).
//!
//! The rules and their grades (read from data: a model or key; read from the executable, static: what the
//! code does; measured: on the original in a bench session):
//!
//! - **The model** is the shields-down model ([`MODEL`]; the name STA64 looks up, no data key names it; read from
//!   the executable, static), found through the mounted files (a mod's replacement included). Each of its mesh
//!   nodes is drawn as one band, additive and two-sided, with the node's own channel and flipbook where the model has them.
//! - **Frame**: the target's transform, then the shield ellipsoid's axes divided by the model's database radius
//!   (`stock_shield_hits::database_radius`), so a model whose vertices lie at its database radius lies on the ellipsoid (read from data, measured). The
//!   ellipsoid is `impacts::shield_axes` (INTERIM, until SIM-32).
//! - **When**: the bands appear in the tick a shield pool reaches zero (the sim's `ShieldsDown` event, measured) and
//!   are removed in the tick the pool is above zero again, which the sim's zero-shield hold (feature
//!   `shield_zero_hold`, #1101) puts `shieldDelay` after the last damage taken at zero (measured). The
//!   client reads only the view: the pool at zero for the whole hold. Nothing is drawn for a craft whose
//!   definition's features lack `shield_zero_hold`: without the hold the pool is above zero on the next tick and the
//!   effect would flicker for one frame (the narrowest reading; INTERIM, the original's rule for such profiles is
//!   not established).
//! - **One effect per craft**, started by the first `ShieldsDown` event for the craft and ended when none of its pools
//!   is at zero (a craft under the hold has one pool in the original; a second simultaneous effect would add brightness
//!   nobody measured). For a mod with directional pools this is INTERIM: what the original draws per pool is not
//!   established. The effect is started by the event, never derived from the view alone: a pool can be at zero for
//!   other reasons than the hold, and what the original draws then is not established.
//! - **Spin**: the model's channels, step-keyed and evenly spaced over their period (read from data); the sense as
//!   the original draws it (measured); run on the game clock (INTERIM: the clock the original spins on is not stated).
//! - **Flipbook**: the animation the model names for each band with its own time offset (read from data; the
//!   evaluator the shots use; not measured for these bands: INTERIM).
//! - **Colour and blend**: unlit, texel times [`GAIN`] times white, added exactly (texture alpha ignored), two-sided, depth
//!   test on, depth write off. The white gain is an inference from the measured gain of the shield caps (INTERIM);
//!   depth write is INTERIM.
//! - **Visibility**: drawn only while its target is visible, as the shield caps.
//! - **Cloak**: no bands while the craft is cloaked (read from the executable, static; not measured). Cloaked is the view's
//!   `UnitView.flags` bit `UnitFlags::CLOAKED`, which the sim sets when the cloak is complete and clears when it starts
//!   to disengage; the view has no finer state, so a craft engaging or disengaging counts as not cloaked (INTERIM). The
//!   report does not say what happens to bands across a cloak: the narrowest reading is taken (INTERIM): the effect is
//!   hidden, not ended, while the craft is cloaked, and shown again if the craft decloaks with a pool still at zero. It
//!   ends only by the rules above.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use sta_assets::model::{Model, NodeKind};
use sta_sim::{EntityId, Fixed, SimEvent};
use std::collections::{BTreeSet, HashMap};

use super::impacts::shield_axes;
use super::stock_shield_hits::{GAIN, database_radius, flipbook_rect};
use super::stock_shots::{ShotParams, StockShotMaterial, StockShots};
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::model_scene::to_bevy;
use crate::render::profile::ProfileEffects;
use crate::render::sprites::SpriteTables;
use crate::sim_link::{ContentRes, SimLink, SimMsg};

/// The shields-down model STA64 looks up (no data key names it; read from the executable, static). A mod's mounted
/// files may replace it.
pub const MODEL: &str = "xshlddie";

/// What one band of the model needs.
pub struct BandGeometry {
    /// The node's name (the tests read it).
    #[cfg_attr(not(test), allow(dead_code))]
    pub name: String,
    pub mesh: Mesh,
    /// The texture's name.
    pub texture: Option<String>,
    /// The node's own transform in render space without its channel, and its parent's.
    pub parent: Transform,
    pub local: Transform,
    /// The channel: its period in seconds and the keyed transforms (render space), step-keyed.
    pub keys: Option<(f32, Vec<Transform>)>,
    /// The flipbook: the animation's id and its time offset.
    pub flipbook: Option<(String, f32)>,
}

pub struct BandsGeometry {
    pub bands: Vec<BandGeometry>,
    pub r_db: f32,
}

/// The bands of `model`: every mesh node, its channel and its flipbook.
pub fn band_geometry(model: &Model) -> Option<BandsGeometry> {
    let r_db = database_radius(model).filter(|r| *r > 0.0)?;
    let mut bands = Vec::new();
    for n in &model.nodes {
        let NodeKind::Mesh(m) = &n.kind else { continue };
        let (mut pos, mut uv, mut idx): (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) = (Vec::new(), Vec::new(), Vec::new());
        let mut remap: HashMap<(u32, u32), u32> = HashMap::new();
        let mut texture = None;
        for g in &m.groups {
            for face in &g.faces {
                let corners: Vec<Option<u32>> = face
                    .iter()
                    .map(|&(p, u)| {
                        let (pp, uu) = (m.positions.get(p as usize)?, m.uvs.get(u as usize)?);
                        Some(*remap.entry((p, u)).or_insert_with(|| {
                            pos.push([pp[0], pp[1], -pp[2]]);
                            uv.push(*uu);
                            (pos.len() - 1) as u32
                        }))
                    })
                    .collect();
                if let [Some(a), Some(b), Some(c)] = corners[..] {
                    idx.extend([a, b, c]);
                }
            }
            if texture.is_none() {
                texture = g.maps.diffuse.clone();
            }
        }
        if idx.is_empty() {
            continue;
        }
        let count = pos.len();
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; count])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; count])
            .with_inserted_indices(Indices::U32(idx));
        let parent = n.parent.map_or(Transform::IDENTITY, |p| to_bevy(&model.model_transform(p)));
        let keys = model
            .channels
            .iter()
            .find(|c| c.node.eq_ignore_ascii_case(&n.name))
            .and_then(|c| match &c.keys {
                sta_assets::model::Keys::Matrix(k) if k.len() > 1 && c.period > 0.0 => Some((c.period, k.iter().map(to_bevy).collect())),
                _ => None,
            });
        let flipbook = model.flipbooks.iter().find(|(node, _, _)| node.eq_ignore_ascii_case(&n.name)).map(|(_, a, o)| (a.clone(), *o));
        bands.push(BandGeometry { name: n.name.clone(), mesh, texture, parent, local: to_bevy(&n.transform), keys, flipbook });
    }
    (!bands.is_empty()).then_some(BandsGeometry { bands, r_db })
}

/// A band's transform at game time `t` seconds: its parent's, then its own, or the channel's key (step-keyed, the keys
/// evenly spaced over the period, looping).
pub fn band_transform(parent: &Transform, local: &Transform, keys: Option<&(f32, Vec<Transform>)>, t: f32) -> Transform {
    let own = match keys {
        Some((period, k)) => {
            let phase = (t % period) / period;
            k[((phase * k.len() as f32) as usize).min(k.len() - 1)]
        }
        None => *local,
    };
    Transform::from_matrix(parent.to_matrix() * own.to_matrix())
}

/// The effect's shared band meshes and materials, loaded once per mount (or, in the tests, authored).
#[derive(Resource, Clone)]
pub struct ShieldsDownAssets {
    pub bands: Vec<BandAsset>,
    pub r_db: f32,
}

#[derive(Clone)]
pub struct BandAsset {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StockShotMaterial>,
    pub parent: Transform,
    pub local: Transform,
    pub keys: Option<(f32, Vec<Transform>)>,
    pub flipbook: Option<(String, f32)>,
}

/// One craft's shields-down effect. Tests key on the unit.
#[derive(Component, Debug, Clone)]
pub struct ShieldsDown {
    pub unit: EntityId,
    /// Its target is visible: only then it is drawn.
    pub shown: bool,
}

#[derive(Component)]
pub struct BandPart(pub usize);

#[derive(Component)]
pub struct BandsAttached;

/// Whether the bands are hidden for a craft whose view says cloaked (the rule of the Stock look: none while cloaked).
pub fn hidden_by_cloak(cloaked: bool) -> bool {
    cloaked
}

/// Whether the effect is wanted for a craft under its definition's features: only with the zero-shield hold.
pub fn wanted(hold: bool, pool_now: f32, pool_max: f32) -> bool {
    hold && pool_max > 0.0 && pool_now <= 0.0
}

/// Whether any pool of the craft is at zero (a pool with a size and nothing in it).
fn any_pool_down(pools: &[[Fixed; 2]; 4]) -> bool {
    pools.iter().any(|p| p[1] > Fixed::ZERO && p[0] <= Fixed::ZERO)
}

/// Start the effect of a craft's first `ShieldsDown` event; end it when none of the craft's pools is at zero, the unit is gone,
/// or the Stock look is off.
#[allow(clippy::too_many_arguments)]
pub(super) fn track(
    mut cmds: Commands,
    mut msgs: MessageReader<SimMsg>,
    link: Res<SimLink>,
    content: Res<ContentRes>,
    ctx: StockShots,
    mut live: Query<(Entity, &mut ShieldsDown)>,
) {
    let stock = ctx.shield_hits_stock();
    let mut have: BTreeSet<EntityId> = BTreeSet::new();
    for (e, s) in &mut live {
        match link.curr().unit(s.unit) {
            Some(u) if stock && any_pool_down(&u.shields) => {
                have.insert(s.unit);
            }
            _ => cmds.entity(e).try_despawn(),
        }
    }
    for SimMsg(ev) in msgs.read() {
        let SimEvent::ShieldsDown { id, facing } = ev else { continue };
        if !stock || have.contains(id) {
            continue;
        }
        let Some(unit) = link.curr().unit(*id) else { continue };
        let Some(def) = content.0.unit(unit.def) else { continue };
        if matches!(def.class.0.as_str(), "planet" | "planet_obj") {
            continue;
        }
        let [now, max] = unit.shields[usize::from(*facing).min(3)].map(|v| v.to_f32());
        if !wanted(content.0.unit_features(def).shield_zero_hold, now, max) {
            continue;
        }
        have.insert(*id);
        cmds.spawn((ShieldsDown { unit: *id, shown: true }, Transform::IDENTITY, Visibility::Hidden));
    }
}

/// Place each effect on its craft's ellipsoid and show it only while the craft is visible.
#[allow(clippy::too_many_arguments)]
pub(super) fn place(
    mut cmds: Commands,
    offsets: Option<Res<crate::render::mining::VisualOffsets>>,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    ctx: StockShots,
    assets: Option<Res<ShieldsDownAssets>>,
    visibility: Query<&InheritedVisibility>,
    mut q: Query<(Entity, &mut ShieldsDown, &mut Transform, &mut Visibility)>,
) {
    let alpha = clock.alpha();
    for (e, mut s, mut tf, mut vis) in &mut q {
        let Some(body) = ctx.body(&link, s.unit, alpha, &offsets, None) else {
            cmds.entity(e).try_despawn();
            continue;
        };
        s.shown = ctx.object_visible(s.unit, &visibility) && !hidden_by_cloak(link.curr().unit(s.unit).is_some_and(|u| u.flags.contains(sta_sim::view::UnitFlags::CLOAKED)));
        let want = if s.shown { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        let Some(r_db) = assets.as_ref().map(|a| a.r_db) else { continue };
        let (rot, axes) = shield_axes(body.heading, body.radius);
        *tf = Transform { translation: body.centre, rotation: rot, scale: axes / r_db.max(1e-3) };
    }
}

/// Give each effect its bands.
pub(super) fn attach_visuals(mut cmds: Commands, assets: Res<ShieldsDownAssets>, q: Query<Entity, (With<ShieldsDown>, Without<BandsAttached>)>) {
    for e in &q {
        for (i, b) in assets.bands.iter().enumerate() {
            cmds.spawn((BandPart(i), Mesh3d(b.mesh.clone()), MeshMaterial3d(b.material.clone()), Transform::IDENTITY, bevy::camera::visibility::NoFrustumCulling, ChildOf(e)));
        }
        cmds.entity(e).insert(BandsAttached);
    }
}

/// Spin every band on the game clock and set the flipbook cell of each band's shared material.
pub(super) fn animate(
    clock: Res<AnimClock>,
    tables: Option<Res<SpriteTables>>,
    assets: Res<ShieldsDownAssets>,
    mut parts: Query<(&BandPart, &mut Transform)>,
    mut mats: ResMut<Assets<StockShotMaterial>>,
) {
    let t = clock.now() as f32;
    for (p, mut tf) in &mut parts {
        if let Some(b) = assets.bands.get(p.0) {
            *tf = band_transform(&b.parent, &b.local, b.keys.as_ref(), t);
        }
    }
    let Some(tables) = tables else { return };
    for b in &assets.bands {
        let rect = flipbook_rect(&tables.0, b.flipbook.as_ref().map(|(a, o)| (a.as_str(), *o)), t);
        if mats.get(&b.material).is_some_and(|m| m.params.rect != rect)
            && let Some(mut m) = mats.get_mut(&b.material)
        {
            m.params.rect = rect;
        }
    }
}

/// The band assets built from `geo` with `texture_of` giving each band's texture.
pub fn build_assets(geo: BandsGeometry, texture_of: &mut dyn FnMut(&str) -> Option<Handle<Image>>, meshes: &mut Assets<Mesh>, mats: &mut Assets<StockShotMaterial>) -> Option<ShieldsDownAssets> {
    let mut bands = Vec::new();
    for b in geo.bands {
        let texture = texture_of(b.texture.as_deref()?)?;
        let params = ShotParams { rect: Vec4::new(0.0, 0.0, 1.0, 1.0), flags: Vec4::ZERO, tint: Vec4::new(GAIN, GAIN, GAIN, 1.0) };
        bands.push(BandAsset { mesh: meshes.add(b.mesh), material: mats.add(StockShotMaterial { params, texture }), parent: b.parent, local: b.local, keys: b.keys, flipbook: b.flipbook });
    }
    Some(ShieldsDownAssets { bands, r_db: geo.r_db })
}

#[derive(Resource, Default)]
pub struct DownLoadState {
    pub tried: bool,
}

/// The mounted game or mod changed: the bands of the old files go and the load may be tried again.
pub(super) fn reset_on_remount(mut cmds: Commands, mut state: ResMut<DownLoadState>) {
    state.tried = false;
    cmds.remove_resource::<ShieldsDownAssets>();
}

/// Load the bands' model and textures from the mounted files, once per mount, when the first effect is wanted.
pub(super) fn load_assets(
    mut cmds: Commands,
    vfs: Res<crate::viewer::Mounted>,
    mut loader: crate::render::textures::LoaderParams,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StockShotMaterial>>,
    mut state: ResMut<DownLoadState>,
) {
    if std::mem::replace(&mut state.tried, true) {
        return;
    }
    let Some((_, model)) = super::mining_beam::load_named_model(&vfs.0, MODEL, "shields-down model") else {
        warn!("stock shields down: the model {MODEL} is not in the mounted files; nothing is drawn");
        return;
    };
    let Some(geo) = band_geometry(&model) else {
        warn!("stock shields down: the model {MODEL} has no geometry; nothing is drawn");
        return;
    };
    let mut tex = |name: &str| loader.loader().colour(name, false);
    match build_assets(geo, &mut tex, &mut meshes, &mut mats) {
        Some(a) => cmds.insert_resource(a),
        None => warn!("stock shields down: a texture of {MODEL} is not in the mounted files; nothing is drawn"),
    }
}

pub struct StockShieldsDownPlugin;

impl Plugin for StockShieldsDownPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProfileEffects>();
        app.init_resource::<DownLoadState>();
        app.add_systems(Update, reset_on_remount.run_if(resource_exists_and_changed::<crate::viewer::Mounted>).before(load_assets).in_set(GameSet::Interpolate));
        app.add_systems(
            Update,
            (
                (track, place).chain(),
                (
                    load_assets
                        .run_if(not(resource_exists::<ShieldsDownAssets>))
                        .run_if(any_with_component::<ShieldsDown>)
                        .run_if(resource_exists::<crate::viewer::Mounted>)
                        .run_if(resource_exists::<crate::render::textures::TextureCache>)
                        .run_if(resource_exists::<Assets<Mesh>>)
                        .run_if(resource_exists::<Assets<Image>>)
                        .run_if(resource_exists::<Assets<StockShotMaterial>>),
                    attach_visuals.run_if(resource_exists::<ShieldsDownAssets>),
                    animate.run_if(resource_exists::<ShieldsDownAssets>).run_if(resource_exists::<Assets<StockShotMaterial>>),
                )
                    .chain(),
            )
                .chain()
                .in_set(GameSet::Interpolate)
                .distributive_run_if(resource_exists::<SimLink>)
                .distributive_run_if(resource_exists::<ContentRes>),
        );
    }
}

#[cfg(test)]
#[path = "stock_shields_down_tests.rs"]
mod tests;
