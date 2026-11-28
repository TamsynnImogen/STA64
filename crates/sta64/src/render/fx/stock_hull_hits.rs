//! The Stock look at a weapon hit on a bare hull and on a dying hull (#1153), and STA64's damage smoke off
//! under it (decision D-02). The Remaster look is unchanged: Refit's hull-hit streaks (`impacts.rs`) and its smoke.
//!
//! The rules and their grades (read from data: a key's presence; read from the executable, static: what the
//! code does; measured: on the original in a bench session):
//!
//! - **A weapon hit that gets past the shields** (`Hit.hull` above zero) adds one node particle to the hit
//!   unit: one member of the spark family ([`stock_flares::SPARK_FAMILY`]), picked at random, additive, constant
//!   colour, a flipbook on the game clock, living [`stock_flares::SPARK_LIFE`] seconds (read from the
//!   executable, static). Its half size is [`stock_flares::spark_half`] of the unit's radius (the rule is read
//!   from the executable, static; the radius the original uses is not resolved: STA64 takes the unit's selection
//!   radius, INTERIM). It is drawn only while its object is visible and takes a slot of the shared cap of
//!   [`NodeParticles`] with the beams' flares (read from the executable, static). A beam's hull hit draws the
//!   impact flare as well (`stock_flares.rs`, measured). None of Refit's hull-hit streaks is drawn.
//!   INTERIM, where it is drawn: the original puts it at the hit node of the model, which the view does not
//!   carry. The sim's hit event carries the target's centre, not a hit point. A **beam** drawn the Stock way
//!   (`stock_shots.rs`) has an end on the hull (the target hardpoint the shot chose, the centre where the model
//!   has none): the spark is drawn there ([`HullHitPoints`]). A **projectile** drawn the Stock way is aimed at the
//!   same node and hits there when its distance is under a step (read from the executable, static; the node is
//!   chosen per shot by STA64's hash, INTERIM: the original's choice is not determined): its spark is drawn at that
//!   node too. A projectile not drawn the Stock way, or whose target is out of the view, is drawn at the target's
//!   centre. All stay in the unit's frame, so they follow it. The projectile's own drawing still flies to the
//!   target's centre (INTERIM), so it ends inside the hull while the spark sits on the node.
//! - **Other mods' families** draw their own per-ordnance hit effects through their own hook (not read): under
//!   Stock they get this family for now (INTERIM).
//! - **A hit on a unit already dying draws none** (read from the executable, static; not measured).
//! - **A dying hull** (`SimEvent::Dying` until `Destroyed`): one spark of the same family every
//!   [`stock_flares::DEATH_SPARK_INTERVAL`] seconds, living as long as a hit's, sharing the cap, so a few are
//!   alive at once (read from the executable, static; measured for the count). They end with the unit's removal
//!   (read from the executable, static; measured). The original picks a vertex of the shown model, and so does
//!   the client: a random vertex of the drawn hull parts, in the unit's frame (the rule as the notes grade it).
//!   Without a drawn model (a unit without a hull part) STA64's own stand-in is a random point inside the shield
//!   ellipsoid ([`super::impacts::shield_axes`]). INTERIM: the original's sparks are hidden by the hull when
//!   behind it (depth); the flares share the shots' sprite drawing.
//! - **Content.** The family's sprites are read from the mounted sprite table. A table without any of them
//!   draws nothing (not Refit's streaks) and logs once: STA64's own rule (the original's behaviour with a missing
//!   particle is not in the notes).
//! - **Damage smoke.** The threshold smoke is STA64's own; the original's damage look comes from the model's damage
//!   nodes, which STA64 already draws: off under the Stock look (D-02).

use std::collections::BTreeMap;

use bevy::ecs::system::SystemParam;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use sta_sim::{ContentKey, EntityId, SimEvent, TICK_HZ};

use super::stock_flares::{self, Anchor, Flare, FlareKind, NodeParticles};
use super::stock_shots::ShotSprites;
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::hulls::HullPart;
use crate::render::profile::{Effect, ProfileEffects, RenderProfile};
use crate::render::units::SelectRadius;
use crate::sim_link::{SimEntities, SimLink, SimMsg};

/// Whether the Stock hull-hit spark is the one drawn: under the Stock look with a stock implementation registered.
/// Without the profile resources (the windowless tests of other effects) it is not.
pub(super) fn stock_hull_hits(profile: Option<&RenderProfile>, effects: Option<&ProfileEffects>) -> bool {
    match (profile, effects) {
        (Some(p), Some(e)) => e.implementation(*p, Effect::HullHits) == RenderProfile::Stock,
        _ => false,
    }
}

/// Whether STA64's damage smoke is drawn: under the Remaster look (or without the profile resources) yes; under
/// the Stock look no (D-02).
pub(super) fn damage_smoke(profile: Option<&RenderProfile>, effects: Option<&ProfileEffects>) -> bool {
    match (profile, effects) {
        (Some(p), Some(e)) => e.implementation(*p, Effect::DamageSmoke) == RenderProfile::Refit,
        _ => true,
    }
}

/// The family member `r` (0 to 1) picks among those the sprite table resolves; `None` when it resolves none.
pub fn pick(sprites: &ShotSprites, r: f32) -> Option<&'static str> {
    let have: Vec<&'static str> = stock_flares::SPARK_FAMILY.iter().copied().filter(|n| sprites.get(n).is_some()).collect();
    (!have.is_empty()).then(|| have[((r.clamp(0.0, 1.0) * have.len() as f32) as usize).min(have.len() - 1)])
}

/// One spark of `kind` on `object` (a unit of `radius`), `local` in its frame, born at `born` ticks of game time.
/// Not shown until its object's place and visibility have been read (`update_flares`).
pub fn spark(kind: FlareKind, object: EntityId, sprite: &str, radius: f32, local: Vec3, born: f64) -> Flare {
    Flare {
        kind,
        object,
        sprite: sprite.to_owned(),
        half: stock_flares::spark_half(radius),
        colour: Vec3::ONE,
        born,
        life: stock_flares::SPARK_LIFE * TICK_HZ as f32,
        anchor: Anchor::Frame(local),
        at: Vec3::ZERO,
        shown: false,
    }
}

/// What the spark of a hit needs, from `impacts.rs`: raise it if the table has the family and a slot is free.
#[allow(clippy::too_many_arguments)]
pub(super) fn raise_hit_spark(
    cmds: &mut Commands,
    store: &mut NodeParticles,
    sprites: &ShotSprites,
    logged: &mut bool,
    object: EntityId,
    radius: f32,
    local: Vec3,
    pick_r: f32,
    born: f64,
) {
    let Some(name) = pick(sprites, pick_r) else {
        if !std::mem::replace(logged, true) {
            warn!("Stock hull-hit spark: the mounted sprite table has none of the spark family; nothing is drawn");
        }
        return;
    };
    store.spawn(cmds, spark(FlareKind::HullHit, object, name, radius, local, born));
}

/// Where each beam's or projectile's hull hit of this frame struck, as the stock shot drawing knows it: `(shooter, weapon, target,
/// world point)`. Written by `stock_shots::update_shots`, read by `impacts::spawn_impacts` (which runs after it);
/// emptied at the start of every `update_shots`.
#[derive(Resource, Default, Debug)]
pub struct HullHitPoints(pub Vec<(EntityId, ContentKey, EntityId, Vec3)>);

impl HullHitPoints {
    /// Where the beam of `by` with `weapon` struck `target` this frame, if the stock beam drawing says.
    pub fn of(&self, by: EntityId, weapon: ContentKey, target: EntityId) -> Option<Vec3> {
        self.0.iter().find(|r| r.0 == by && r.1 == weapon && r.2 == target).map(|r| r.3)
    }
}

/// A random point inside the unit sphere from three random numbers in `[0, 1)`.
fn point_in_sphere(a: f32, b: f32, c: f32) -> Vec3 {
    let z = 2.0 * a - 1.0;
    let phi = std::f32::consts::TAU * b;
    let s = (1.0 - z * z).max(0.0).sqrt();
    Vec3::new(s * phi.cos(), z, s * phi.sin()) * c.cbrt()
}

/// The vertices of a unit's drawn hull parts.
#[derive(SystemParam)]
struct HullVertices<'w, 's> {
    meshes: Option<Res<'w, Assets<Mesh>>>,
    children: Query<'w, 's, &'static Children>,
    parts: Query<'w, 's, (&'static Mesh3d, &'static GlobalTransform), With<HullPart>>,
    roots: Query<'w, 's, &'static GlobalTransform>,
}

impl HullVertices<'_, '_> {
    /// The vertex `r` (0 to 1) picks among all the vertices of the hull parts under `root`, as its world offset from
    /// the root's origin. `None` without a drawn model.
    fn pick(&self, root: Entity, r: f32) -> Option<Vec3> {
        let meshes = self.meshes.as_deref()?;
        let base = self.roots.get(root).ok()?.translation();
        let parts: Vec<(&[[f32; 3]], &GlobalTransform)> = self
            .children
            .iter_descendants(root)
            .filter_map(|e| self.parts.get(e).ok())
            .filter_map(|(m, t)| match meshes.get(&m.0)?.attribute(Mesh::ATTRIBUTE_POSITION)? {
                VertexAttributeValues::Float32x3(p) => Some((p.as_slice(), t)),
                _ => None,
            })
            .collect();
        let total: usize = parts.iter().map(|p| p.0.len()).sum();
        if total == 0 {
            return None;
        }
        let mut i = ((r.clamp(0.0, 1.0) * total as f32) as usize).min(total - 1);
        for (p, t) in parts {
            if i < p.len() {
                return Some(t.transform_point(Vec3::from(p[i])) - base);
            }
            i -= p.len();
        }
        None
    }
}

/// Dying units' emitters: when the next spark is due, in ticks of game time.
#[derive(Resource, Default)]
struct Emitters(BTreeMap<EntityId, f64>);

/// The units that have begun to die (`Dying`) and are not yet gone: a hit on one draws no spark.
#[derive(Resource, Default, Debug)]
pub struct DyingUnits(pub std::collections::BTreeSet<EntityId>);

/// A match or mission starting or ending: the next sim's ids begin again, so the dying and their emitters go.
fn forget(mut dying: ResMut<DyingUnits>, mut emitters: ResMut<Emitters>) {
    dying.0.clear();
    emitters.0.clear();
}

/// The sparks of dying hulls: one every [`stock_flares::DEATH_SPARK_INTERVAL`] seconds from `Dying` until the
/// unit is gone.
#[allow(clippy::too_many_arguments)]
fn emit_death_sparks(
    mut cmds: Commands,
    mut msgs: MessageReader<SimMsg>,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    profile: Option<Res<RenderProfile>>,
    effects: Option<Res<ProfileEffects>>,
    sprites: Option<Res<ShotSprites>>,
    store: Option<ResMut<NodeParticles>>,
    ents: Option<Res<SimEntities>>,
    radii: Query<&SelectRadius>,
    hulls: HullVertices,
    mut emitters: ResMut<Emitters>,
    mut logged: Local<bool>,
) {
    let stock = stock_hull_hits(profile.as_deref(), effects.as_deref());
    let now = clock.now() * TICK_HZ as f64;
    for SimMsg(ev) in msgs.read() {
        match ev {
            SimEvent::Dying { id, .. } if stock => {
                emitters.0.insert(*id, now);
            }
            SimEvent::Destroyed { id, .. } => {
                emitters.0.remove(id);
            }
            _ => {}
        }
    }
    let (Some(sprites), Some(mut store)) = (sprites, store) else { return };
    let every = f64::from(stock_flares::DEATH_SPARK_INTERVAL) * f64::from(TICK_HZ);
    let view = link.curr();
    emitters.0.retain(|id, _| stock && view.unit(*id).is_some());
    for (id, next) in emitters.0.iter_mut() {
        let root = ents.as_ref().and_then(|m| m.0.get(id)).copied();
        let radius = root.and_then(|e| radii.get(e).ok()).map_or(super::impacts::DEFAULT_RADIUS, |r| r.0);
        let heading = view.unit(*id).map_or(Vec3::NEG_Z, |u| Vec3::new(u.heading.0.to_f32(), 0.0, -u.heading.1.to_f32()).normalize_or(Vec3::NEG_Z));
        // A long frame catches up, a few at most: the cap holds them to four anyway.
        for _ in 0..8 {
            if *next > now {
                break;
            }
            let tick = *next as u64;
            let r = |salt| super::impacts::rand(*id, tick, 0, salt);
            match pick(&sprites, r(60)) {
                Some(name) => {
                    // A vertex of the drawn model; without one, a point inside the shield ellipsoid (STA64's own).
                    let local = root.and_then(|e| hulls.pick(e, r(61))).map_or_else(
                        || point_in_sphere(r(61), r(62), r(63)) * super::impacts::shield_axes(heading, radius).1,
                        |offset| stock_flares::to_local(heading, offset),
                    );
                    store.spawn(&mut cmds, spark(FlareKind::DeathSpark, *id, name, radius, local, *next));
                }
                None => {
                    if !std::mem::replace(&mut *logged, true) {
                        warn!("Stock death sparks: the mounted sprite table has none of the spark family; nothing is drawn");
                    }
                    break;
                }
            }
            *next += every;
        }
        if *next <= now {
            *next = now + every;
        }
    }
}

#[cfg(test)]
#[path = "stock_hull_hits_tests.rs"]
mod tests;

/// Registers the stock hull-hit spark and the switch-off of STA64's damage smoke, and runs the death sparks.
pub struct StockHullHitsPlugin;

impl Plugin for StockHullHitsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProfileEffects>();
        {
            let mut effects = app.world_mut().resource_mut::<ProfileEffects>();
            effects.register_stock(Effect::HullHits);
            effects.register_stock(Effect::DamageSmoke);
        }
        app.init_resource::<HullHitPoints>().init_resource::<DyingUnits>().init_resource::<Emitters>();
        app.add_systems(OnEnter(crate::app::AppState::Loading), forget).add_systems(OnExit(crate::app::AppState::InGame), forget);
        app.add_systems(Update, emit_death_sparks.after(super::impacts::spawn_impacts).in_set(GameSet::Interpolate).distributive_run_if(resource_exists::<SimLink>));
    }
}
