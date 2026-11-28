//! The Stock look's shield hit (#1041): a spherical cap model on the shield ellipsoid, coloured by the shield
//! level, drawn additively for the ordnance's `shieldDuration`. The Remaster look (`RenderProfile::Refit`)
//! draws the whole-shell effect of `impacts.rs`, unchanged; under Stock that shell is not drawn
//! ([`Effect::ShieldHits`]) and everything else `impacts.rs` draws still is.
//!
//! The rules and their grades (read from data: a key's presence; read from the executable, static: what
//! the code does; measured: on the original in a bench session):
//!
//! - **One cap per hit.** No limit on their number, no merging, no pruning; they coexist, each with its own
//!   colour and life (read from the executable, static; measured).
//! - **When.** The target is a craft (not a planet), the ordnance does not set `ignoreShield`, and the target's
//!   shields are up (read from the executable, static; measured). With shields down the hull hit draws
//!   the impact flare instead (`stock_flares.rs`).
//! - **Beams** (the classes `stock_shots.rs` draws): when the beam first reaches its target, once per beam,
//!   its pole at the beam's end; the pole follows the beam's end while the beam lives, then stays fixed in
//!   the target's frame; a cap shorter than the beam ends early and never comes back (measured; the following
//!   read from the executable, static, not measured on a moving target, so the moving-target pole is
//!   INTERIM). The beam's end on the shield is the shared ellipsoid rule of `stock_shots.rs`.
//! - **Every other hit with shields absorbing** (projectiles, and beams of a class with no stock drawing):
//!   at the `Hit` event, on impact (measured for torpedoes). The pole faces the side the shot came from: the
//!   reverse of a stock-drawn shot's flight direction; for a shot with no stock drawing, from the target
//!   toward the shooter (INTERIM, not established).
//! - **The model** is the default shield-hit model ([`DEFAULT_MODEL`]), found through the mounted files
//!   (a mod's replacement included): a spherical cap, drawn at its finest level of detail (INTERIM: the original
//!   switches level by distance), two-sided, additive.
//! - **Frame** (read from the executable, static; the scale measured to about 2 percent): the target's
//!   transform, then the shield ellipsoid's axes divided by the model's database radius, then the rotation
//!   that turns the cap's pole toward the hit on the ellipsoid's unit sphere. The database radius is the
//!   largest vertex distance from the centre of the model's bounding box ([`database_radius`]), so the cap
//!   stands slightly outside the ellipsoid. The ellipsoid is `impacts::shield_axes` (INTERIM, until
//!   SIM-32); its centre is the unit's (the model's box centre in the original). The roll about the pole
//!   is not established (the shortest turn from the pole is taken, INTERIM).
//! - **Colour** (read from the executable, static; the gain measured): unlit, texel times [`GAIN`] times
//!   (1 - f, f, 0), f the shield fraction, frozen for the cap's life even if the shields fall meanwhile.
//!   Projectiles take f after their damage: the view's pool after that tick. Beams take f before their
//!   damage (the original applies a beam's damage at the end of its life): under `beam_damage_at_end`, where the
//!   sim does the same, the view's pool of the side the beam lands on when the beam arrives
//!   ([`beam_side_fraction`], #1068); otherwise the view's pool plus the absorbed damage of that beam's `Hit`
//!   (INTERIM, for every profile without the switch). A beam's per-tick
//!   refresh of f while it lives is not drawn (not measured).
//! - **Blend.** Added exactly (the texture's alpha darkens nothing; measured), two-sided, depth test on
//!   (measured), depth write off (INTERIM: not established).
//! - **Animation and life.** The model's material names a texture flipbook (a sprite-table animation); its cell
//!   is picked from the game clock, the same for every cap, frozen while the game is paused, a new hit does not
//!   start at cell 0 (measured on shot sheets at one window size; the same evaluator for model-material flipbooks: read from the executable, static; the rule is [`flipbook_rect`]'s). A cap lives `shieldDuration` seconds ([`DEFAULT_DURATION`] without the key),
//!   a hard cut on the game clock, no fade (measured).
//! - **Not drawn here**: the shields-down bands (they wait for the sim's zero-shield hold, issue #1101), the
//!   per-ordnance `shieldGeometryName` (not typed) and the special-weapon shield types, the cloak rule, a beam's
//!   per-tick colour refresh. A cap is drawn only while its target is visible.

use std::collections::{BTreeSet, HashMap};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use sta_assets::model::{Model, NodeKind};
use sta_assets::spr_anim;
use sta_content::WeaponDef;
use sta_sim::command::Target;
use sta_sim::{ContentKey, EntityId, SimEvent, TICK_HZ, Tick};

use super::impacts::{facing_dir, shield_axes};
use super::stock_shots::{ShotParams, ShotPath, StockShot, StockShotMaterial, StockShots, update_shots};
use super::weapons::{point, shifted};
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::profile::{Effect, ProfileEffects};
use crate::render::sprites::SpriteTables;
use crate::sim_link::{ContentRes, SimLink, SimMsg};

/// The shield-hit model STA64 looks up when an ordnance names no other (`shieldGeometryName` is not typed yet; no
/// data key names this one; read from the executable, static). A mod's mounted files may replace it.
pub const DEFAULT_MODEL: &str = "xshldx01";
/// A cap's life in seconds when the ordnance has no `shieldDuration` (read from the executable, static).
pub const DEFAULT_DURATION: f32 = 0.8;
/// The colour gain, times texel times (1 - f, f, 0) (measured on the original; the static reading had none).
pub const GAIN: f32 = 2.0;
/// The cap's pole in render space: the model's +z, mirrored with the rest of the model.
pub const POLE: Vec3 = Vec3::NEG_Z;
/// Ticks a beam's recorded shield level is kept for the cap that will ask for it (a beam lives a few seconds).
const FRACTION_KEEP: Tick = 10 * TICK_HZ as Tick;
/// How far from a beam's fire tick its recorded level may be (the hit and the shot meet in one frame).
const FRACTION_SLACK: Tick = 2;

// ---------------------------------------------------------------------------------------------
// Rules, pure
// ---------------------------------------------------------------------------------------------

/// The shield fraction f, 0 to 1, of a pool (`[now, max]` in the view).
pub fn shield_fraction(now: f32, max: f32) -> f32 {
    if max > 0.0 { (now / max).clamp(0.0, 1.0) } else { 0.0 }
}

/// A beam's fraction before its own damage: the pool the view shows after the tick plus what the beam's hit
/// took from it (INTERIM, for every profile where the sim does not apply a beam's damage at the end of its life: without
/// `beam_damage_at_end`, #1068).
pub fn beam_fraction_before(now: f32, absorbed: f32, max: f32) -> f32 {
    shield_fraction(now + absorbed.max(0.0), max)
}

/// The shield fraction of the side of `target` a beam from `shooter` lands on, in `view`: the side the sim names in the
/// beam's `Hit` (`combat::weapons::facing`, centre to centre). `None` when either is not in the view.
pub fn beam_side_fraction(view: &sta_sim::WorldView, shooter: EntityId, target: EntityId) -> Option<f32> {
    let (s, t) = (view.unit(shooter)?, view.unit(target)?);
    let side = usize::from(sta_sim::combat::weapons::facing(t.heading, t.pos, s.pos)).min(3);
    let [now, max] = t.shields[side].map(|v| v.to_f32());
    Some(shield_fraction(now, max))
}

/// The cap's colour factor: [`GAIN`] times (1 - f, f, 0).
pub fn cap_colour(f: f32) -> Vec3 {
    let f = f.clamp(0.0, 1.0);
    GAIN * Vec3::new(1.0 - f, f, 0.0)
}

/// A cap's life in ticks: the ordnance's `shieldDuration` (seconds), else the default.
pub fn cap_life_ticks(def: &WeaponDef) -> f32 {
    def.base.shield_duration.map_or(DEFAULT_DURATION, |d| d.to_f32()) * TICK_HZ as f32
}

/// Whether a cap born at `born` ticks of game time and living `life` ticks is still drawn at `now`: a hard
/// cut, no fade.
pub fn cap_live(now: f64, born: f64, life: f32) -> bool {
    now - born < f64::from(life)
}

/// The direction in the ellipsoid's unit-sphere space (the frame of `rot`, axes `axes`) of the world vector
/// `v` from the ellipsoid's centre: where on the ellipsoid a line along `v` leaves it.
pub fn dir_toward(rot: Quat, axes: Vec3, v: Vec3) -> Vec3 {
    ((rot.inverse() * v) / axes).try_normalize().unwrap_or(POLE)
}

/// The two transforms that place a cap: the ellipsoid frame (the centre, the rotation, the axes over the model's
/// database radius) and, as its child, the turn of the pole toward `dir` (unit-sphere space). The world
/// matrix is their product; it can shear, which a single transform cannot hold, hence two.
pub fn cap_frame(centre: Vec3, rot: Quat, axes: Vec3, r_db: f32, dir: Vec3) -> (Transform, Transform) {
    let frame = Transform { translation: centre, rotation: rot, scale: axes / r_db.max(1e-3) };
    (frame, Transform::from_rotation(Quat::from_rotation_arc(POLE, dir.try_normalize().unwrap_or(POLE))))
}

/// The flipbook's texture offset, as a material rectangle `(u, v, 1, 1)` added to the model's own
/// coordinates, at game time `t` seconds for the animation `anim` (its time offset `offset`), by the sprite table's
/// own evaluator (`spr_anim::value`), the one the shots use. No animation or an unknown one: the model's own
/// coordinates.
pub fn flipbook_rect(table: &sta_assets::a2fmt::spr::SpriteTable, anim: Option<(&str, f32)>, t: f32) -> Vec4 {
    let off = anim.and_then(|(id, offset)| table.animation(id).map(|a| spr_anim::offset_of(spr_anim::value(a, t + offset, spr_anim::LOOP)))).unwrap_or([0.0, 0.0]);
    Vec4::new(off[0], off[1], 1.0, 1.0)
}

// ---------------------------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------------------------

/// What the cap needs from its model.
pub struct CapGeometry {
    /// The finest level of detail, in render space (z mirrored, as every model's).
    pub mesh: Mesh,
    /// The first diffuse texture of its groups (a texture name).
    pub texture: Option<String>,
    /// The texture flipbook: the animation's id and its time offset.
    pub flipbook: Option<(String, f32)>,
    pub r_db: f32,
}

/// The database radius: the largest distance of any vertex of any mesh from the centre of the model's
/// bounding box (read from the executable, static; data: the model's vertices).
pub fn database_radius(model: &Model) -> Option<f32> {
    let (min, max) = model.bounds()?;
    let c = (Vec3::from(min) + Vec3::from(max)) * 0.5;
    let mut r2 = 0.0f32;
    for (i, n) in model.nodes.iter().enumerate() {
        if let Some(m) = n.mesh() {
            let w = model.model_transform(i);
            for p in &m.positions {
                r2 = r2.max((Vec3::from(w.transform_point(*p)) - c).length_squared());
            }
        }
    }
    Some(r2.sqrt())
}

/// The cap's geometry from `model`: its finest level of detail only (INTERIM: LOD by distance is not read).
pub fn cap_geometry(model: &Model) -> Option<CapGeometry> {
    let r_db = database_radius(model).filter(|r| *r > 0.0)?;
    // A node is hidden when it, or an ancestor, is a coarser level of detail.
    let coarse: BTreeSet<usize> = model.coarse_lods().into_iter().collect();
    let hidden = |mut i: usize| {
        for _ in 0..=model.nodes.len() {
            if coarse.contains(&i) {
                return true;
            }
            match model.nodes[i].parent {
                Some(p) => i = p,
                None => return false,
            }
        }
        false
    };
    let (mut pos, mut uv, mut idx): (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) = (Vec::new(), Vec::new(), Vec::new());
    let (mut texture, mut flipbook) = (None, None);
    for (i, n) in model.nodes.iter().enumerate() {
        let NodeKind::Mesh(m) = &n.kind else { continue };
        if hidden(i) {
            continue;
        }
        let w = model.model_transform(i);
        let mut remap: HashMap<(u32, u32), u32> = HashMap::new();
        let mut used = false;
        for g in &m.groups {
            for face in &g.faces {
                let corners: Vec<Option<u32>> = face
                    .iter()
                    .map(|&(p, u)| {
                        let (pp, uu) = (m.positions.get(p as usize)?, m.uvs.get(u as usize)?);
                        Some(*remap.entry((p, u)).or_insert_with(|| {
                            let q = w.transform_point(*pp);
                            pos.push([q[0], q[1], -q[2]]);
                            uv.push(*uu);
                            (pos.len() - 1) as u32
                        }))
                    })
                    .collect();
                if let [Some(a), Some(b), Some(c)] = corners[..] {
                    idx.extend([a, b, c]);
                    used = true;
                }
            }
            if texture.is_none() {
                texture = g.maps.diffuse.clone();
            }
        }
        if used && flipbook.is_none() {
            flipbook = model.flipbooks.iter().find(|(node, _, _)| node.eq_ignore_ascii_case(&n.name)).map(|(_, a, o)| (a.clone(), *o));
        }
    }
    if idx.is_empty() {
        return None;
    }
    // The first flipbook of the model when none belongs to a drawn node.
    let flipbook = flipbook.or_else(|| model.flipbooks.first().map(|(_, a, o)| (a.clone(), *o)));
    let n = pos.len();
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; n])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; n])
        .with_inserted_indices(Indices::U32(idx));
    Some(CapGeometry { mesh, texture, flipbook, r_db })
}

/// The cap's mesh, texture and flipbook, once loaded (or, in the tests, authored).
#[derive(Resource, Clone)]
pub struct ShieldCapAssets {
    pub mesh: Handle<Mesh>,
    pub texture: Handle<Image>,
    pub flipbook: Option<(String, f32)>,
    pub r_db: f32,
}

// ---------------------------------------------------------------------------------------------
// Caps
// ---------------------------------------------------------------------------------------------

/// Where a cap came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapOrigin {
    /// A stock-drawn beam, fired at this tick: one cap per beam; its pole follows the beam's end while it lives.
    Beam { fired: Tick },
    /// A `Hit` event: at the event, the pole fixed.
    Event,
}

/// A cap to make, from the two places that decide it (the beam's arrival in `stock_shots.rs`, the hit event here).
#[derive(Debug, Clone, PartialEq)]
pub struct CapRequest {
    pub target: EntityId,
    pub shooter: EntityId,
    pub weapon: ContentKey,
    /// The pole, in the ellipsoid's unit-sphere space.
    pub dir: Vec3,
    /// Game time in ticks.
    pub born: f64,
    pub origin: CapOrigin,
    /// The shield fraction; a beam's is looked up from its recorded `Hit`.
    pub fraction: Option<f32>,
}

#[derive(Resource, Default, Debug)]
pub struct ShieldCapQueue(pub Vec<CapRequest>);

/// What each beam's `Hit` left of the shield before its damage: `(shooter, weapon, target, tick, f)`.
#[derive(Resource, Default, Debug)]
pub struct BeamFractions(pub Vec<(EntityId, ContentKey, EntityId, Tick, f32)>);

impl BeamFractions {
    /// The level recorded for the beam fired at `fired`: the nearest record of its shooter, weapon and target.
    pub fn before(&self, shooter: EntityId, weapon: ContentKey, target: EntityId, fired: Tick) -> Option<f32> {
        self.0.iter().filter(|r| r.0 == shooter && r.1 == weapon && r.2 == target && r.3.abs_diff(fired) <= FRACTION_SLACK).min_by_key(|r| r.3.abs_diff(fired)).map(|r| r.4)
    }
}

/// One shield hit. Tests key on `(target, weapon, born)`, never on the Bevy `Entity`.
#[derive(Component, Debug, Clone)]
pub struct ShieldCap {
    pub target: EntityId,
    pub shooter: EntityId,
    pub weapon: ContentKey,
    /// A stock beam's fire tick while its end moves the pole.
    pub follows: Option<Tick>,
    /// Game time it began, in ticks.
    pub born: f64,
    /// Ticks it lives.
    pub life: f32,
    /// The shield fraction it was made with, frozen (the colour is made from it; the tests read it).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fraction: f32,
    /// Linear RGB factor: gain times (1 - f, f, 0), frozen.
    pub colour: Vec3,
    /// The pole, in the ellipsoid's unit-sphere space (the target's frame).
    pub dir: Vec3,
    /// Its target is visible: only then it is drawn.
    pub shown: bool,
}

/// The visual child of a cap and the material it was given.
#[derive(Component)]
pub struct CapMesh;

#[derive(Component)]
pub struct CapVisual(pub Entity);

/// Record each beam's shield level and make the caps of the hits the beam's arrival does not make.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_hits(
    mut msgs: MessageReader<SimMsg>,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    content: Res<ContentRes>,
    ctx: StockShots,
    offsets: Option<Res<crate::render::mining::VisualOffsets>>,
    shots: Query<(Entity, &StockShot)>,
    mut fractions: ResMut<BeamFractions>,
    mut queue: ResMut<ShieldCapQueue>,
) {
    let tick = clock.tick();
    let alpha = clock.alpha();
    fractions.0.retain(|r| r.3 + FRACTION_KEEP > tick);
    let mut used: BTreeSet<Entity> = BTreeSet::new();
    let hits: Vec<&SimEvent> = msgs.read().map(|m| &m.0).filter(|e| matches!(e, SimEvent::Hit { .. })).collect();
    // What every hit of this batch (one sim tick when the game runs at its speed) took from each pool: in the original
    // none of the beams has landed yet, so the level before is the pool plus ALL of it, not only the beam's own.
    // INTERIM with #1068; a frame that runs several ticks adds back the earlier ticks' hits too.
    let mut taken: std::collections::BTreeMap<(EntityId, u8), f32> = std::collections::BTreeMap::new();
    for ev in &hits {
        if let SimEvent::Hit { target, facing, shield, .. } = ev {
            *taken.entry((*target, (*facing).min(3))).or_default() += shield.to_f32().max(0.0);
        }
    }
    for ev in hits {
        let SimEvent::Hit { target, by, weapon, facing, shield, .. } = ev else { continue };
        let absorbed = shield.to_f32();
        if !ctx.shield_hits_stock() || absorbed <= 0.0 {
            continue;
        }
        let Some(def) = content.0.weapon(*weapon) else { continue };
        if def.base.ignore_shield.unwrap_or(false) {
            continue;
        }
        let Some(unit) = link.curr().unit(*target) else { continue };
        let Some(body) = ctx.body(&link, *target, alpha, &offsets, None) else { continue };
        if body.planet {
            continue;
        }
        let [now, max] = unit.shields[usize::from(*facing).min(3)].map(|v| v.to_f32());
        if ctx.stock_path(def) == Some(ShotPath::Ribbon) {
            // Made when the beam reaches its target (`stock_shots.rs`). Under `beam_damage_at_end` this hit comes at the
            // end of the beam's life, long after the cap took its level from the view (`spawn_caps`): nothing to record
            // (#1068). Otherwise its level is what this hit left plus what it took (INTERIM).
            if ctx.lands_at_end(def) {
                continue;
            }
            fractions.0.push((*by, *weapon, *target, tick, beam_fraction_before(now, taken[&(*target, (*facing).min(3))], max)));
            continue;
        }
        // The side the shot came from: the reverse of a stock-drawn shot's flight, else toward the shooter (INTERIM).
        let shot = shots
            .iter()
            .filter(|(e, s)| !used.contains(e) && s.path != ShotPath::Ribbon && s.shooter == *by && s.weapon == *weapon && s.target == Target::Entity(*target))
            .min_by_key(|(_, s)| s.fired);
        let toward = match shot {
            Some((e, s)) => {
                used.insert(e);
                Some(-s.flight_dir())
            }
            None => point(&link, *by, alpha).map(|p| p + shifted(&offsets, *by) - body.centre),
        };
        let toward = toward.filter(|v| v.length_squared() > 1e-6).unwrap_or_else(|| facing_dir(body.heading, *facing));
        let (rot, axes) = shield_axes(body.heading, body.radius);
        queue.0.push(CapRequest {
            target: *target,
            shooter: *by,
            weapon: *weapon,
            dir: dir_toward(rot, axes, toward),
            born: clock.now() * TICK_HZ as f64,
            origin: CapOrigin::Event,
            fraction: Some(shield_fraction(now, max)),
        });
    }
}

/// Turn the queued requests into caps.
pub(super) fn spawn_caps(mut cmds: Commands, mut queue: ResMut<ShieldCapQueue>, fractions: Res<BeamFractions>, content: Res<ContentRes>, link: Res<SimLink>) {
    for r in queue.0.drain(..) {
        let Some(def) = content.0.weapon(r.weapon) else { continue };
        let at_end = sta_sim::combat::weapons::lands_at_end(content.0.features(), def);
        let f = r
            .fraction
            .or_else(|| match r.origin {
                // Under `beam_damage_at_end` the sim has not applied the beam's damage when the cap is made: the pool of
                // the side the hit will land on, as the view shows it now, is the level before it (#1068).
                CapOrigin::Beam { .. } if at_end => beam_side_fraction(link.real_curr(), r.shooter, r.target),
                CapOrigin::Beam { fired } => fractions.before(r.shooter, r.weapon, r.target, fired),
                CapOrigin::Event => None,
            })
            // No record of the beam's hit (it was not delivered): the pool as the view shows it.
            .unwrap_or_else(|| {
                link.curr().unit(r.target).map_or(0.0, |u| {
                    let (now, max) = u.shields.iter().fold((0.0, 0.0), |a, s| (a.0 + s[0].to_f32(), a.1 + s[1].to_f32()));
                    shield_fraction(now, max)
                })
            });
        cmds.spawn((
            ShieldCap {
                target: r.target,
                shooter: r.shooter,
                weapon: r.weapon,
                follows: match r.origin {
                    CapOrigin::Beam { fired } => Some(fired),
                    CapOrigin::Event => None,
                },
                born: r.born,
                life: cap_life_ticks(def),
                fraction: f,
                colour: cap_colour(f),
                dir: r.dir,
                shown: true,
            },
            Transform::IDENTITY,
            Visibility::Hidden,
        ));
    }
}

/// Place every cap on its target, move a beam's cap with the beam's end, end the old ones, hide those of a hidden target.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn update_caps(
    mut cmds: Commands,
    offsets: Option<Res<crate::render::mining::VisualOffsets>>,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    ctx: StockShots,
    assets: Option<Res<ShieldCapAssets>>,
    visibility: Query<&InheritedVisibility>,
    shots: Query<&StockShot>,
    mut caps: Query<(Entity, &mut ShieldCap, &mut Transform, &mut Visibility, Option<&CapVisual>), Without<CapMesh>>,
    mut turns: Query<&mut Transform, With<CapMesh>>,
    warp: super::warp_flash::WarpStretch,
) {
    let now = clock.now() * TICK_HZ as f64;
    let alpha = clock.alpha();
    for (e, mut cap, mut tf, mut vis, visual) in &mut caps {
        // Its target gone, or its life run out (a hard cut): it ends.
        let body = ctx.body(&link, cap.target, alpha, &offsets, None).filter(|_| cap_live(now, cap.born, cap.life));
        let Some(body) = body else {
            cmds.entity(e).try_despawn();
            continue;
        };
        let (rot, axes) = shield_axes(body.heading, body.radius);
        if let Some(fired) = cap.follows {
            let beam = shots.iter().find(|s| s.path == ShotPath::Ribbon && s.shooter == cap.shooter && s.weapon == cap.weapon && s.fired == fired && s.target == Target::Entity(cap.target));
            if let Some(b) = beam {
                cap.dir = dir_toward(rot, axes, b.beam_end_now() - body.centre);
            }
        }
        cap.shown = ctx.object_visible(cap.target, &visibility);
        let want = if cap.shown { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        let Some(r_db) = assets.as_ref().map(|a| a.r_db) else { continue };
        // A craft in warp: the cap stretches with the hull along its motion (read from the executable, static; INTERIM:
        // the hull's uniform stretch).
        let (frame, turn) = cap_frame(body.centre, rot, Vec3::new(axes.x, axes.y, axes.z * warp.of_stock(cap.target)), r_db, cap.dir);
        *tf = frame;
        if let Some(CapVisual(child)) = visual
            && let Ok(mut t) = turns.get_mut(*child)
        {
            *t = turn;
        }
    }
}

/// Give each cap its mesh, a material of its own colour and its texture.
pub(super) fn attach_visuals(
    mut cmds: Commands,
    assets: Res<ShieldCapAssets>,
    caps: Query<(Entity, &ShieldCap), Without<CapVisual>>,
    mut mats: ResMut<Assets<StockShotMaterial>>,
) {
    for (e, cap) in &caps {
        let params = ShotParams { rect: Vec4::new(0.0, 0.0, 1.0, 1.0), flags: Vec4::ZERO, tint: cap.colour.extend(1.0) };
        let material = mats.add(StockShotMaterial { params, texture: assets.texture.clone() });
        let child = cmds.spawn((CapMesh, Mesh3d(assets.mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY, bevy::camera::visibility::NoFrustumCulling, ChildOf(e))).id();
        cmds.entity(e).insert(CapVisual(child));
    }
}

/// The flipbook cell: one for every cap, from the game clock.
pub(super) fn animate_caps(clock: Res<AnimClock>, tables: Option<Res<SpriteTables>>, assets: Res<ShieldCapAssets>, caps: Query<&MeshMaterial3d<StockShotMaterial>, With<CapMesh>>, mut mats: ResMut<Assets<StockShotMaterial>>) {
    let Some(tables) = tables else { return };
    let rect = flipbook_rect(&tables.0, assets.flipbook.as_ref().map(|(a, o)| (a.as_str(), *o)), clock.now() as f32);
    for h in &caps {
        if mats.get(&h.0).is_some_and(|m| m.params.rect != rect)
            && let Some(mut m) = mats.get_mut(&h.0)
        {
            m.params.rect = rect;
        }
    }
}

/// Whether the model load was tried for the mounted files.
#[derive(Resource, Default)]
pub struct CapLoadState {
    pub tried: bool,
}

/// The mounted game or mod changed: the cap of the old files goes and the load may be tried again, a failed one too.
pub(super) fn reset_on_remount(mut cmds: Commands, mut state: ResMut<CapLoadState>) {
    state.tried = false;
    cmds.remove_resource::<ShieldCapAssets>();
}

/// Load the cap's model and texture from the mounted files, once per mount, when the first cap is wanted.
pub(super) fn load_assets(
    mut cmds: Commands,
    vfs: Res<crate::viewer::Mounted>,
    mut loader: crate::render::textures::LoaderParams,
    mut meshes: ResMut<Assets<Mesh>>,
    mut state: ResMut<CapLoadState>,
) {
    if std::mem::replace(&mut state.tried, true) {
        return;
    }
    let Some((_, model)) = super::mining_beam::load_named_model(&vfs.0, DEFAULT_MODEL, "shield hit model") else {
        warn!("stock shield hits: the model {DEFAULT_MODEL} is not in the mounted files; no cap is drawn");
        return;
    };
    let Some(geo) = cap_geometry(&model) else {
        warn!("stock shield hits: the model {DEFAULT_MODEL} has no geometry; no cap is drawn");
        return;
    };
    let Some(texture) = geo.texture.as_deref().and_then(|t| loader.loader().colour(t, false)) else {
        warn!("stock shield hits: the texture of {DEFAULT_MODEL} is not in the mounted files; no cap is drawn");
        return;
    };
    cmds.insert_resource(ShieldCapAssets { mesh: meshes.add(geo.mesh), texture, flipbook: geo.flipbook, r_db: geo.r_db });
}

pub struct StockShieldHitsPlugin;

impl Plugin for StockShieldHitsPlugin {
    fn build(&self, app: &mut App) {
        crate::render::safe_mode::register_mesh_guard(app);
        app.init_resource::<ProfileEffects>();
        app.world_mut().resource_mut::<ProfileEffects>().register_stock(Effect::ShieldHits);
        app.init_resource::<ShieldCapQueue>().init_resource::<BeamFractions>().init_resource::<CapLoadState>();
        // A new mount drops the cap of the old files; `.chain()` below runs the reload after it.
        app.add_systems(Update, reset_on_remount.run_if(resource_exists_and_changed::<crate::viewer::Mounted>).before(load_assets).in_set(GameSet::Interpolate));
        app.add_systems(
            Update,
            (
                collect_hits.before(update_shots),
                (spawn_caps, update_caps).chain().after(update_shots),
                (
                    load_assets
                        .run_if(not(resource_exists::<ShieldCapAssets>))
                        .run_if(any_with_component::<ShieldCap>)
                        .run_if(resource_exists::<crate::viewer::Mounted>)
                        .run_if(resource_exists::<crate::render::textures::TextureCache>)
                        .run_if(resource_exists::<Assets<Mesh>>)
                        .run_if(resource_exists::<Assets<Image>>),
                    attach_visuals.run_if(resource_exists::<ShieldCapAssets>).run_if(resource_exists::<Assets<StockShotMaterial>>),
                    animate_caps.run_if(resource_exists::<ShieldCapAssets>).run_if(resource_exists::<Assets<StockShotMaterial>>),
                )
                    .chain()
                    .after(update_caps),
            )
                .in_set(GameSet::Interpolate)
                .distributive_run_if(resource_exists::<SimLink>)
                .distributive_run_if(resource_exists::<ContentRes>),
        );
    }
}

#[cfg(test)]
#[path = "stock_shield_hits_tests.rs"]
mod tests;
