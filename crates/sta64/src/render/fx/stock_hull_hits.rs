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
//!   INTERIM: the spark sits at the hit point of the event, in the unit's frame so it follows the unit; the original
//!   puts it at the hit node of the model, which the view does not carry.
//! - **A hit on a unit already dying draws none** (read from the executable, static; not measured).
//! - **A dying hull** (`SimEvent::Dying` until `Destroyed`): one spark of the same family every
//!   [`stock_flares::DEATH_SPARK_INTERVAL`] seconds, living as long as a hit's, sharing the cap, so a few are
//!   alive at once (read from the executable, static; measured for the count). They end with the unit's removal
//!   (read from the executable, static; measured). INTERIM: the original picks a vertex of the shown model; the
//!   client takes a random point inside the unit's radius, in the unit's frame. INTERIM: the original's
//!   sparks are hidden by the hull when behind it (depth); the flares share the shots' sprite drawing.
//! - **Content.** The family's sprites are read from the mounted sprite table. A table without any of them
//!   draws nothing (not Refit's streaks) and logs once.
//! - **Damage smoke.** STA64's hull-damage smoke is its own and the original draws none (read from the
//!   executable, static; seen on the bench): off under the Stock look (D-02).

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_sim::{EntityId, SimEvent, TICK_HZ};

use super::stock_flares::{self, Anchor, Flare, FlareKind, NodeParticles};
use super::stock_shots::ShotSprites;
use crate::app::GameSet;
use crate::clock::AnimClock;
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

/// A random point inside a sphere of `radius` from three random numbers in `[0, 1)`: the stand-in for a vertex of the
/// shown model (INTERIM).
fn point_in(radius: f32, a: f32, b: f32, c: f32) -> Vec3 {
    let z = 2.0 * a - 1.0;
    let phi = std::f32::consts::TAU * b;
    let s = (1.0 - z * z).max(0.0).sqrt();
    Vec3::new(s * phi.cos(), z, s * phi.sin()) * radius * c.cbrt()
}

/// Dying units' emitters: when the next spark is due, in ticks of game time.
#[derive(Default)]
struct Emitters(BTreeMap<EntityId, f64>);

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
    mut emitters: Local<Emitters>,
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
        let radius = ents.as_ref().and_then(|m| m.0.get(id)).and_then(|e| radii.get(*e).ok()).map_or(super::impacts::DEFAULT_RADIUS, |r| r.0);
        // A long frame catches up, a few at most: the cap holds them to four anyway.
        for _ in 0..8 {
            if *next > now {
                break;
            }
            let tick = *next as u64;
            let r = |salt| super::impacts::rand(*id, tick, 0, salt);
            match pick(&sprites, r(60)) {
                Some(name) => {
                    store.spawn(&mut cmds, spark(FlareKind::DeathSpark, *id, name, radius, point_in(radius, r(61), r(62), r(63)), *next));
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
        app.add_systems(Update, emit_death_sparks.after(super::impacts::spawn_impacts).in_set(GameSet::Interpolate).distributive_run_if(resource_exists::<SimLink>));
    }
}
