//! Impact FX (RND-10): shield hits, hull sparks, explosions and damage smoke
//! from `SimEvent::{Hit, ShieldsDown, Destroyed}` and `HullShare`.
//!
//! Aligned with Armada II Refit's `effects` module (`render3d/mods/effects`): the
//! shield hit is `effects_shields.hlsl` ported to WGSL (`shield.wgsl`: the implied
//! shell lit only about the hit, spot, ring, mist, Klingon rays, soft edge) with
//! `[Shields]`'s constants (`shield.rs`); the explosion is `effects_explosions.hlsl`
//! (`fire.wgsl`: the ray-marched fireball and the shock ring) with `[Explosions]`'s
//! (`fire.rs`); the sparks are `sparks.rs`' streaks cooling along the black-body
//! ramp (`quad.wgsl`, kind spark); the debris takes `[Debris]`'s timing and speed.
//!
//! - **Shield hit** (`Hit.shield > 0`): a shield ellipsoid around the target, lit about
//!   where the shot struck (`Hit.at`, else the facing: 0 fore, 1 aft, 2 port,
//!   3 starboard), in the faction colour of the target's ODF initial. The strength is
//!   `sqrt(damage / 60)` clamped to 0.6..1.8; a beam holds its spot for its duration
//!   and fades over 0.25 s, a shot's decays at once (Refit's `hit_now`). `ShieldsDown`
//!   is the strongest hit; Refit's lightning (`down_ps`) is not ported.
//! - **Hull impact** (`Hit.hull > 0`): `HitSparks` (6) streaks from `Hit.at`.
//! - **Explosion** (`Destroyed`): the fireball and shock ring sized by the unit's
//!   `SelectRadius` (last seen, the unit is gone from the view): the radius
//!   at least `MinRadius` (25), growing over `Growth` of its 2.5 s, `BlastSparks` (14)
//!   streaks, and chunks of `Lifetime` 4 s +- 2 flying at up to `MaxVelocity` 90.
//! - **Damage smoke**: a [`SmokeEmitter`] is kept for each unit root whose
//!   [`HullShare`] is under [`SMOKE_LIGHT`] (one stream) or [`SMOKE_HEAVY`]
//!   (two); it detaches above the threshold, while cloaked, and with the root.
//!   Puffs already out run their course. Refit has no such rule (stock's damage
//!   vents come from the SOD's damage nodes); these thresholds are STA64's own.
//!
//! The chunks and the smoke stay plain spheres: Refit's chunks break the unit's own
//! meshes (`chunks.rs`) and its smoke is the SOD emitters'.
//!
//! All time is [`AnimClock`]; FX are keyed by `(kind, unit, fired, idx)`.

use bevy::prelude::*;
use sta_content::WeaponKind;
use sta_sim::{EntityId, SimEvent, TICK_HZ, Tick};
use std::collections::BTreeMap;

use super::fire::{self, FireFrame, FireMaterial, RingFrame};
use super::quad::{self, FxQuadMaterial, Look};
use super::shield::{self, Faction, ShieldFrame, ShieldMaterial};
use super::weapons::FxAlpha;
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::units::{Cloaked, HullShare, SelectRadius, to_bevy};
use crate::sim_link::{ContentRes, SimEntities, SimEntity, SimLink, SimMsg};

/// Hull share under which a unit smokes lightly, and heavily.
pub const SMOKE_LIGHT: f32 = 0.5;
pub const SMOKE_HEAVY: f32 = 0.25;
const DEFAULT_RADIUS: f32 = 30.0;
/// `[Sparks]` `HitSparks`, `BlastSparks`, `BurstTime` (s), `SparkSpeed` (times the
/// particle's size), `SparkSpread`, `SparkLife` (s), `SparkBrightness`; the drag and
/// the tail time are `sparks.rs`' literals.
pub const HIT_SPARKS: u8 = 6;
pub const BLAST_SPARKS: u8 = 14;
const SPARK_BURST: f32 = 0.15;
const SPARK_SPEED: f32 = 4.0;
const SPARK_SPREAD: f32 = 1.0;
const SPARK_LIFE: [f32; 2] = [0.3, 0.9];
const SPARK_BRIGHT: f32 = 2.5;
const SPARK_DRAG: f32 = 0.8;
const SPARK_TAIL: f32 = 0.04;
/// A spark's half width, world units (Refit: 1.4 px).
const SPARK_HALF_WIDTH: f32 = 0.5;
/// `[Debris]`: `Lifetime`, `LifetimeDeviation` (s), `MaxVelocity`.
const DEBRIS_LIFE: f32 = 4.0;
const DEBRIS_DEV: f32 = 2.0;
const DEBRIS_SPEED: f32 = 90.0;
const DEBRIS: u8 = 12;
/// The shock ring: how long it runs, how far out (times the fireball's radius), and
/// where it starts to fade. Refit reads these from the explosion's class.
const RING_LIFE: f32 = 1.0;
const RING_REACH: f32 = 3.0;
const RING_FADE: f32 = 0.5;
const RING_COLOUR: Vec3 = Vec3::new(1.0, 0.55, 0.2);
const PUFF_TICKS: f32 = 40.0;
/// The shield ellipsoid's semi-axes in unit radii, across, up and along the heading.
const ELLIPSOID: Vec3 = Vec3::new(1.2, 0.8, 1.5);
/// The camera's near plane the march starts at.
const NEAR: f32 = 0.5;

fn ticks(secs: f32) -> f32 {
    secs * TICK_HZ as f32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImpactKind {
    Shield,
    Spark,
    /// The fireball.
    Fire,
    /// The shock ring.
    Ring,
    Debris,
    Puff,
}

/// One impact effect, positioned purely from its age.
#[derive(Component, Debug, Clone)]
pub struct ImpactFx {
    pub kind: ImpactKind,
    pub unit: EntityId,
    /// Which of a burst (the tests' key; nothing else reads it).
    #[allow(dead_code)]
    pub idx: u8,
    pub fired: Tick,
    /// Ticks it lives, counted from `fired` (sparks start `delay` ticks late).
    pub life: f32,
    delay: f32,
    origin: Vec3,
    /// Units per second (sparks, debris, smoke).
    vel: Vec3,
    /// Render-space direction from the shield's centre to where it was hit.
    dir: Vec3,
    heading: Vec3,
    radius: f32,
    colour: Color,
    faction: Faction,
    strength: f32,
    /// A beam's hit: seconds it is held.
    held: Option<f32>,
    /// Per-effect seed, 0..1.
    seed: Vec3,
    look: Look,
}

/// Kept for a unit while it is damaged. `level` 1 light, 2 heavy.
#[derive(Component, Debug, Clone)]
pub struct SmokeEmitter {
    pub unit: EntityId,
    pub level: u8,
    last: Tick,
}

/// Last known place, heading, size and faction of each unit seen.
#[derive(Clone, Copy)]
struct SeenUnit {
    pos: Vec3,
    heading: Vec3,
    radius: f32,
    faction: Faction,
}

impl Default for SeenUnit {
    fn default() -> SeenUnit {
        SeenUnit { pos: Vec3::ZERO, heading: Vec3::NEG_Z, radius: DEFAULT_RADIUS, faction: Faction::of_odf("") }
    }
}

#[derive(Resource, Default)]
struct Seen(BTreeMap<EntityId, SeenUnit>);

pub struct ImpactFxPlugin;

impl Plugin for ImpactFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Seen>().add_systems(
            Update,
            (track, spawn_impacts, smoke_attach, emit_smoke, update_impacts, attach_visuals, apply_alpha, apply_shields, apply_fire, apply_quads)
                .chain()
                .in_set(GameSet::Interpolate)
                .run_if(resource_exists::<SimLink>),
        );
    }
}

/// splitmix64 to a float in `[0, 1)`; one stream per `(unit, tick, idx, salt)`.
pub(super) fn rand(unit: EntityId, tick: Tick, idx: u8, salt: u8) -> f32 {
    let mut z = ((unit.0 as u64) << 40) ^ (tick << 8) ^ idx as u64 ^ ((salt as u64) << 32);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

fn unit_dir(r: impl Fn(u8) -> f32) -> Vec3 {
    let (y, a) = (r(0) * 2.0 - 1.0, r(1) * std::f32::consts::TAU);
    let s = (1.0 - y * y).sqrt();
    Vec3::new(s * a.cos(), y, s * a.sin())
}

fn facing_dir(heading: Vec3, facing: u8) -> Vec3 {
    let right = heading.cross(Vec3::Y).normalize_or(Vec3::X);
    match facing {
        0 => heading,
        1 => -heading,
        2 => -right,
        _ => right,
    }
}

/// Refit's `weldfx::heat_colour`: a spark's colour as it cools, 1 white-hot to 0.
pub(super) fn heat_colour(k: f32) -> Vec3 {
    let k = k.clamp(0.0, 1.0);
    if k > 0.66 {
        let f = (k - 0.66) / 0.34;
        Vec3::new(1.0, 0.62 + 0.33 * f, 0.22 + 0.58 * f)
    } else if k > 0.33 {
        let f = (k - 0.33) / 0.33;
        Vec3::new(1.0, 0.30 + 0.32 * f, 0.06 + 0.16 * f)
    } else {
        let f = k / 0.33;
        Vec3::new(0.55 + 0.45 * f, 0.08 + 0.22 * f, 0.02 + 0.04 * f)
    }
}

/// Refit's `weldfx::spark_at`: tail and head of a spark `age` seconds into `life`,
/// and its heat (1 down to 0).
pub(super) fn spark_at(from: Vec3, dir: Vec3, speed: f32, age: f32, life: f32) -> Option<(Vec3, Vec3, f32)> {
    if age <= 0.0 || age >= life {
        return None;
    }
    let travel = |t: f32| speed * t * (1.0 - 0.5 * SPARK_DRAG * t / life);
    let tail_t = if age > SPARK_TAIL { age - SPARK_TAIL } else { 0.0 };
    Some((from + dir * travel(tail_t), from + dir * travel(age), 1.0 - age / life))
}

fn track(link: Res<SimLink>, content: Option<Res<ContentRes>>, ents: Option<Res<SimEntities>>, radii: Query<&SelectRadius>, mut seen: ResMut<Seen>) {
    for u in &link.curr().units {
        let radius = ents.as_ref().and_then(|m| m.0.get(&u.id)).and_then(|e| radii.get(*e).ok()).map_or(DEFAULT_RADIUS, |r| r.0);
        let heading = Vec3::new(u.heading.0.to_f32(), 0.0, -u.heading.1.to_f32()).normalize_or(Vec3::NEG_Z);
        let faction = content.as_ref().and_then(|c| c.0.unit(u.def)).map_or(Faction::of_odf(""), |d| Faction::of_odf(&d.odf));
        seen.0.insert(u.id, SeenUnit { pos: to_bevy(u.pos.to_f32()), heading, radius, faction });
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_impacts(mut cmds: Commands, mut msgs: MessageReader<SimMsg>, clock: Res<AnimClock>, content: Option<Res<ContentRes>>, mut seen: ResMut<Seen>) {
    let tick = clock.tick();
    for SimMsg(ev) in msgs.read() {
        match ev {
            SimEvent::Hit { target, weapon, facing, shield, hull, at, .. } => {
                let s = seen.0.get(target).copied().unwrap_or_default();
                let at = to_bevy(at.to_f32());
                let dir = (at - s.pos).try_normalize().unwrap_or_else(|| facing_dir(s.heading, *facing));
                let mk = |kind, idx, life, origin, vel, colour| {
                    (
                        ImpactFx {
                            kind,
                            unit: *target,
                            idx,
                            fired: tick,
                            life,
                            delay: 0.0,
                            origin,
                            vel,
                            dir,
                            heading: s.heading,
                            radius: s.radius,
                            colour,
                            faction: s.faction,
                            strength: 1.0,
                            held: None,
                            seed: Vec3::ZERO,
                            look: Look::new(quad::K_SPARK, Vec3::ONE),
                        },
                        FxAlpha(1.0),
                        Transform::from_translation(origin),
                    )
                };
                if shield.to_f32() > 0.0 {
                    let held = content.as_ref().and_then(|c| c.0.weapon(*weapon)).filter(|d| d.kind == WeaponKind::Beam).map(|d| d.duration as f32 / TICK_HZ as f32);
                    let secs = held.map_or(shield::keep_time(), |h| h + shield::FADE_OUT);
                    let (mut b, a, t) = mk(ImpactKind::Shield, 0, ticks(secs), s.pos, Vec3::ZERO, Color::WHITE);
                    b.strength = shield::strength(shield.to_f32());
                    b.held = held;
                    b.seed = Vec3::splat(rand(*target, tick, 0, 20));
                    cmds.spawn((b, a, t));
                }
                if hull.to_f32() > 0.0 {
                    let size = (s.radius * 0.14).max(4.0);
                    for i in 0..HIT_SPARKS {
                        cmds.spawn(spark(*target, tick, i, at, dir, size, s));
                    }
                }
            }
            SimEvent::ShieldsDown { id, facing } => {
                let s = seen.0.get(id).copied().unwrap_or_default();
                let dir = facing_dir(s.heading, *facing);
                cmds.spawn((
                    ImpactFx {
                        kind: ImpactKind::Shield,
                        unit: *id,
                        idx: 1,
                        fired: tick,
                        life: ticks(shield::keep_time()),
                        delay: 0.0,
                        origin: s.pos,
                        vel: Vec3::ZERO,
                        dir,
                        heading: s.heading,
                        radius: s.radius,
                        colour: Color::WHITE,
                        faction: s.faction,
                        strength: 1.8,
                        held: None,
                        seed: Vec3::splat(rand(*id, tick, 1, 20)),
                        look: Look::new(quad::K_SPARK, Vec3::ONE),
                    },
                    FxAlpha(1.0),
                    Transform::from_translation(s.pos),
                ));
            }
            SimEvent::Destroyed { id, .. } => {
                let Some(s) = seen.0.remove(id) else { continue };
                let p = s.pos;
                let mk = |kind, idx, life, vel, colour| {
                    (
                        ImpactFx {
                            kind,
                            unit: *id,
                            idx,
                            fired: tick,
                            life,
                            delay: 0.0,
                            origin: p,
                            vel,
                            dir: Vec3::Y,
                            heading: s.heading,
                            radius: s.radius,
                            colour,
                            faction: s.faction,
                            strength: 1.0,
                            held: None,
                            seed: Vec3::new(rand(*id, tick, 0, 10), rand(*id, tick, 0, 11), rand(*id, tick, 0, 12)),
                            look: Look::new(quad::K_SPARK, Vec3::ONE),
                        },
                        FxAlpha(1.0),
                        Transform::from_translation(p),
                    )
                };
                cmds.spawn(mk(ImpactKind::Fire, 0, ticks(fire::DEFAULT_LIFE), Vec3::ZERO, Color::WHITE));
                cmds.spawn(mk(ImpactKind::Ring, 0, ticks(RING_LIFE), Vec3::ZERO, Color::WHITE));
                let size = (s.radius * 0.14).max(4.0);
                for i in 0..BLAST_SPARKS {
                    let out = unit_dir(|k| rand(*id, tick, i, 30 + k));
                    cmds.spawn(spark(*id, tick, i, p, out, size, s));
                }
                for i in 0..DEBRIS {
                    let v = unit_dir(|k| rand(*id, tick, i, k)) * DEBRIS_SPEED * (0.25 + 0.75 * rand(*id, tick, i, 2));
                    let life = ticks(DEBRIS_LIFE + DEBRIS_DEV * (2.0 * rand(*id, tick, i, 6) - 1.0));
                    cmds.spawn(mk(ImpactKind::Debris, i, life, v, Color::srgb(1.0, 0.4, 0.1)));
                }
            }
            _ => {}
        }
    }
}

/// One spark of a burst, as Refit's `sparks.rs` draws them: a random start inside the
/// burst, a life of 0.3 to 0.9 s, a direction spread about `out`, a speed in `size`s.
fn spark(unit: EntityId, tick: Tick, i: u8, origin: Vec3, out: Vec3, size: f32, s: SeenUnit) -> impl Bundle {
    let te = rand(unit, tick, i, 40) * SPARK_BURST;
    let life = SPARK_LIFE[0] + (SPARK_LIFE[1] - SPARK_LIFE[0]) * rand(unit, tick, i, 41);
    let dir = (out + unit_dir(|k| rand(unit, tick, i, 42 + k)) * SPARK_SPREAD).normalize_or(out);
    let speed = SPARK_SPEED * size * (0.3 + 0.7 * rand(unit, tick, i, 45));
    (
        ImpactFx {
            kind: ImpactKind::Spark,
            unit,
            idx: i,
            fired: tick,
            life: ticks(te + life),
            delay: ticks(te),
            origin,
            vel: dir * speed,
            dir,
            heading: s.heading,
            radius: s.radius,
            colour: Color::WHITE,
            faction: s.faction,
            strength: 1.0,
            held: None,
            seed: Vec3::ZERO,
            look: Look::new(quad::K_SPARK, Vec3::ONE),
        },
        FxAlpha(0.0),
        Transform::from_translation(origin),
    )
}

fn smoke_level(share: f32) -> u8 {
    if share < SMOKE_HEAVY {
        2
    } else if share < SMOKE_LIGHT {
        1
    } else {
        0
    }
}

/// Attach, retune and detach emitters as `HullShare` crosses the thresholds.
fn smoke_attach(
    mut cmds: Commands,
    clock: Res<AnimClock>,
    units: Query<(&SimEntity, &HullShare, Has<Cloaked>)>,
    mut emitters: Query<(Entity, &mut SmokeEmitter)>,
) {
    let want: BTreeMap<EntityId, u8> = units.iter().map(|(u, h, cloaked)| (u.0, if cloaked { 0 } else { smoke_level(h.0) })).collect();
    for (e, mut s) in &mut emitters {
        match want.get(&s.unit).copied().unwrap_or(0) {
            0 => cmds.entity(e).despawn(),
            l if l != s.level => s.level = l,
            _ => {}
        }
    }
    let have: Vec<EntityId> = emitters.iter().map(|(_, s)| s.unit).collect();
    for (id, level) in want {
        if level > 0 && !have.contains(&id) {
            cmds.spawn(SmokeEmitter { unit: id, level, last: clock.tick() });
        }
    }
}

fn emit_smoke(mut cmds: Commands, clock: Res<AnimClock>, seen: Res<Seen>, mut emitters: Query<&mut SmokeEmitter>) {
    let tick = clock.tick();
    for mut s in &mut emitters {
        let every = if s.level >= 2 { 3 } else { 6 };
        if tick < s.last + every {
            continue;
        }
        s.last = tick;
        let Some(&u) = seen.0.get(&s.unit) else { continue };
        let (p, radius) = (u.pos, u.radius);
        for i in 0..s.level {
            let r = |salt| rand(s.unit, tick, i, salt) - 0.5;
            let origin = p + Vec3::new(r(3), 0.0, r(4)) * radius * 0.8;
            cmds.spawn((
                ImpactFx {
                    kind: ImpactKind::Puff,
                    unit: s.unit,
                    idx: i,
                    fired: tick,
                    life: PUFF_TICKS,
                    delay: 0.0,
                    origin,
                    vel: Vec3::new(r(5) * 6.0, 10.0 + 6.0 * (r(6) + 0.5), r(7) * 6.0),
                    dir: Vec3::Y,
                    heading: u.heading,
                    radius,
                    colour: Color::srgb(0.25, 0.25, 0.25),
                    faction: u.faction,
                    strength: 1.0,
                    held: None,
                    seed: Vec3::ZERO,
                    look: Look::new(quad::K_SPARK, Vec3::ONE),
                },
                FxAlpha(1.0),
                Transform::from_translation(origin),
            ));
        }
    }
}

/// Age of an effect in seconds, from its fire tick.
fn age_secs(clock: &AnimClock, fx: &ImpactFx) -> f32 {
    ((clock.now() * TICK_HZ as f64 - fx.fired as f64) as f32 - fx.delay) / TICK_HZ as f32
}

/// The shield's place now: centre (the unit's, live), heading, ellipsoid.
fn shield_frame(fx: &ImpactFx, centre: Vec3) -> (Vec3, Quat, Vec3, Vec3) {
    let rot = Transform::IDENTITY.looking_to(fx.heading, Vec3::Y).rotation;
    let scale = ELLIPSOID * fx.radius;
    // Where the hit struck, on the unit sphere of the ellipsoid.
    let local = (rot.inverse() * fx.dir) / scale;
    (centre, rot, scale, local.normalize_or(Vec3::NEG_Z))
}

#[allow(clippy::type_complexity)]
fn update_impacts(
    mut cmds: Commands,
    clock: Res<AnimClock>,
    seen: Res<Seen>,
    link: Res<SimLink>,
    mut q: Query<(Entity, &mut ImpactFx, &mut Transform, &mut FxAlpha)>,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
) {
    let alpha = clock.alpha();
    let eye = camera.single().ok().map(|c| c.translation());
    for (e, mut fx, mut tf, mut a) in &mut q {
        let age = age_secs(&clock, &fx);
        let ticks_age = age * TICK_HZ as f32 + fx.delay;
        if ticks_age >= fx.life {
            cmds.entity(e).despawn();
            continue;
        }
        let k = (ticks_age / fx.life).clamp(0.0, 1.0);
        let secs = age.max(0.0);
        let at = fx.origin + fx.vel * secs;
        let r = fx.radius;
        match fx.kind {
            ImpactKind::Shield => {
                let centre = link.interpolate(fx.unit, alpha).map(|(p, _)| to_bevy(p)).or_else(|| seen.0.get(&fx.unit).map(|s| s.pos)).unwrap_or(fx.origin);
                let (centre, rot, scale, _) = shield_frame(&fx, centre);
                *tf = Transform { translation: centre, rotation: rot, scale };
                // The glow's share of its strength: down from 1 as the hit fades.
                let (_, _, _, glow) = shield::hit_now(fx.strength, secs, fx.held);
                a.0 = glow / fx.strength;
            }
            ImpactKind::Spark => {
                // Not yet started: nothing to draw.
                match spark_at(fx.origin, fx.dir, fx.vel.length(), age, (fx.life - fx.delay) / TICK_HZ as f32) {
                    Some((tail, head, kh)) => {
                        *tf = quad::ribbon(eye, tail, head, SPARK_HALF_WIDTH, SPARK_HALF_WIDTH);
                        fx.look.col = heat_colour(kh);
                        fx.look.len = (head - tail).length() / SPARK_HALF_WIDTH;
                        a.0 = kh;
                    }
                    None => {
                        *tf = Transform::from_translation(fx.origin).with_scale(Vec3::ZERO);
                        a.0 = 0.0;
                    }
                }
            }
            ImpactKind::Fire => {
                let rf = fire::full_radius(r);
                let life = fx.life / TICK_HZ as f32;
                let rn = fire::radius_now(rf, (secs / life).min(1.0));
                *tf = Transform::from_translation(at).with_scale(Vec3::splat(fire::cover_radius(rf, rn, secs)));
                a.0 = 1.0 - k;
            }
            ImpactKind::Ring => {
                let mx = fire::full_radius(r) * RING_REACH;
                let rr = mx * k;
                *tf = Transform::from_translation(at).with_scale(Vec3::splat((rr * 1.03).max(0.01)));
                a.0 = 1.0 - k;
            }
            ImpactKind::Debris => {
                *tf = Transform::from_translation(at).with_scale(Vec3::splat((r * 0.08).max(1.0) * (1.0 - k)));
                a.0 = 1.0 - k;
            }
            ImpactKind::Puff => {
                *tf = Transform::from_translation(at).with_scale(Vec3::splat((r * 0.1).max(1.5) * (1.0 + 2.0 * k)));
                a.0 = 0.5 * (1.0 - k);
            }
        }
    }
}

/// Mesh and material for new effects, when render assets exist.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn attach_visuals(
    mut cmds: Commands,
    meshes: Option<ResMut<Assets<Mesh>>>,
    std_mats: Option<ResMut<Assets<StandardMaterial>>>,
    shield_mats: Option<ResMut<Assets<ShieldMaterial>>>,
    fire_mats: Option<ResMut<Assets<FireMaterial>>>,
    quad_mats: Option<ResMut<Assets<FxQuadMaterial>>>,
    mut handles: Local<Option<(Handle<Mesh>, Handle<Mesh>)>>,
    new: Query<(Entity, &ImpactFx), Added<ImpactFx>>,
) {
    let (Some(mut meshes), Some(mut std_mats), Some(mut shield_mats), Some(mut fire_mats), Some(mut quad_mats)) = (meshes, std_mats, shield_mats, fire_mats, quad_mats) else { return };
    if new.is_empty() {
        return;
    }
    let (ball, rect) = handles.get_or_insert_with(|| (meshes.add(Sphere::new(1.0)), meshes.add(quad::rectangle()))).clone();
    for (e, fx) in &new {
        match fx.kind {
            ImpactKind::Shield => {
                cmds.entity(e).insert((Mesh3d(ball.clone()), MeshMaterial3d(shield_mats.add(ShieldMaterial { params: default() }))));
            }
            ImpactKind::Fire | ImpactKind::Ring => {
                cmds.entity(e).insert((Mesh3d(ball.clone()), MeshMaterial3d(fire_mats.add(FireMaterial { params: default() }))));
            }
            ImpactKind::Spark => {
                cmds.entity(e).insert((Mesh3d(rect.clone()), MeshMaterial3d(quad_mats.add(quad::material(&fx.look)))));
            }
            ImpactKind::Debris | ImpactKind::Puff => {
                let blend = if fx.kind == ImpactKind::Puff { AlphaMode::Blend } else { AlphaMode::Add };
                let material = std_mats.add(StandardMaterial { base_color: fx.colour, emissive: fx.colour.to_linear() * 3.0, unlit: true, alpha_mode: blend, ..default() });
                cmds.entity(e).insert((Mesh3d(ball.clone()), MeshMaterial3d(material)));
            }
        }
    }
}

/// The chunks' and the smoke's plain materials.
fn apply_alpha(mats: Option<ResMut<Assets<StandardMaterial>>>, q: Query<(&ImpactFx, &FxAlpha, &MeshMaterial3d<StandardMaterial>), Changed<FxAlpha>>) {
    let Some(mut mats) = mats else { return };
    for (fx, a, m) in &q {
        if let Some(mut mat) = mats.get_mut(&m.0) {
            mat.base_color = fx.colour.with_alpha(a.0);
            if fx.kind != ImpactKind::Puff {
                mat.emissive = fx.colour.to_linear() * (3.0 * a.0);
            }
        }
    }
}

fn apply_shields(
    clock: Res<AnimClock>,
    seen: Res<Seen>,
    link: Res<SimLink>,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    mats: Option<ResMut<Assets<ShieldMaterial>>>,
    q: Query<(&ImpactFx, &MeshMaterial3d<ShieldMaterial>)>,
) {
    let (Some(mut mats), Ok(cam)) = (mats, camera.single()) else { return };
    let alpha = clock.alpha();
    for (fx, m) in &q {
        let centre = link.interpolate(fx.unit, alpha).map(|(p, _)| to_bevy(p)).or_else(|| seen.0.get(&fx.unit).map(|s| s.pos)).unwrap_or(fx.origin);
        let (centre, rot, scale, unit) = shield_frame(fx, centre);
        let frame = ShieldFrame {
            eye: cam.translation(),
            clock: clock.now(),
            faction: fx.faction,
            centre,
            rot,
            scale,
            unit,
            seed: fx.seed.x,
            now: shield::hit_now(fx.strength, age_secs(&clock, fx), fx.held),
            brightness: 1.0,
        };
        let params = frame.params();
        if mats.get(&m.0).is_some_and(|x| x.params != params)
            && let Some(mut x) = mats.get_mut(&m.0)
        {
            x.params = params;
        }
    }
}

fn apply_fire(
    clock: Res<AnimClock>,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    mats: Option<ResMut<Assets<FireMaterial>>>,
    q: Query<(&ImpactFx, &MeshMaterial3d<FireMaterial>)>,
) {
    let (Some(mut mats), Ok(cam)) = (mats, camera.single()) else { return };
    for (fx, m) in &q {
        let age = age_secs(&clock, fx).max(0.0);
        let life = fx.life / TICK_HZ as f32;
        let params = match fx.kind {
            ImpactKind::Fire => FireFrame { eye: cam.translation(), forward: cam.forward().as_vec3(), near: NEAR, centre: fx.origin, rf: fire::full_radius(fx.radius), age, life, seed: fx.seed }.params(),
            _ => {
                let mx = fire::full_radius(fx.radius) * RING_REACH;
                RingFrame {
                    eye: cam.translation(),
                    forward: cam.forward().as_vec3(),
                    near: NEAR,
                    centre: fx.origin,
                    up: Vec3::Y,
                    radius: mx * (age / life).min(1.0),
                    max: mx,
                    age,
                    colour: RING_COLOUR,
                    seed: fx.seed.x,
                    fade: RING_FADE,
                }
                .params()
            }
        };
        if mats.get(&m.0).is_some_and(|x| x.params != params)
            && let Some(mut x) = mats.get_mut(&m.0)
        {
            x.params = params;
        }
    }
}

fn apply_quads(clock: Res<AnimClock>, mats: Option<ResMut<Assets<FxQuadMaterial>>>, q: Query<(&ImpactFx, &FxAlpha, &MeshMaterial3d<FxQuadMaterial>)>) {
    let Some(mut mats) = mats else { return };
    let t = quad::shader_clock(clock.now());
    for (fx, a, m) in &q {
        // Refit's spark: bright with its heat, more than linearly.
        quad::drive(&mut mats, &m.0, &fx.look, SPARK_BRIGHT * a.0 * (0.5 + 0.5 * a.0), t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::sim_link::{ContentRes, SimLinkPlugin};
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::PlayerId;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;
    use std::time::Duration;

    type Row = ((ImpactKind, u32, Tick, u8), [u32; 3], [u32; 3], u32);

    fn timeline(db: &sta_content::ContentDb) -> String {
        let torp = db.key("fx_torpedo").unwrap().0;
        let hit = |at: u32, facing: u8, shield: u32, hull: u32| {
            format!("{{ at: {at}, event: {{ Hit: {{ target: 2, by: 1, weapon: {torp}, facing: {facing}, shield: {shield}, hull: {hull}, at: {{ x: 0, y: 0, z: 590 }} }} }} }},")
        };
        let ev = hit(20, 0, 30, 0) + &hit(40, 2, 30, 0) + &hit(60, 1, 10, 20) + &hit(100, 3, 0, 40) + "{ at: 80, event: { ShieldsDown: { id: 2, facing: 1 } } },";
        format!(
            "{{ units: [
                {{ id: 1, def: \"fx_cruiser\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }},
                {{ id: 2, def: \"fx_frigate\", owner: 1, hull: 400, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 600 }} }}, {{ at: 300, pos: {{ x: 30, y: 0, z: 600 }} }}], until: 200 }},
            ], events: [{ev}] }}"
        )
    }

    /// Frames of FX rows, and per frame the emitter levels.
    fn run() -> (Vec<Vec<Row>>, Vec<Vec<u8>>) {
        let db = sta_content::fake::db();
        let src = ScriptedSource::from_json5(&timeline(&db), &db).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 45.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(ContentRes(std::sync::Arc::new(db)))
            .insert_resource(SimLink::new(Box::new(src), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_plugins(ImpactFxPlugin);
        configure_sets(&mut app);
        let root = app.world_mut().spawn((SimEntity(EntityId(2)), HullShare(1.0))).id();
        let (mut frames, mut smoke) = (Vec::new(), Vec::new());
        for f in 0..900 {
            // Stand-in for the unit view: share falls with time, root goes with the unit.
            if app.world().get_entity(root).is_ok() {
                if app.world().resource::<SimLink>().curr().unit(EntityId(2)).is_none() && f > 10 {
                    app.world_mut().entity_mut(root).despawn();
                } else {
                    let share = match f {
                        0..=80 => 1.0,
                        81..=120 => 0.4,
                        121..=150 => 0.2,
                        151..=170 => 0.4,
                        171..=180 => 0.8,
                        _ => 0.2,
                    };
                    app.world_mut().entity_mut(root).insert(HullShare(share));
                }
            }
            app.update();
            let mut q = app.world_mut().query::<(&ImpactFx, &Transform, &FxAlpha)>();
            let mut rows: Vec<Row> = q
                .iter(app.world())
                .map(|(f, t, a)| {
                    let b = |v: Vec3| v.to_array().map(f32::to_bits);
                    ((f.kind, f.unit.0, f.fired, f.idx), b(t.translation), b(t.scale), a.0.to_bits())
                })
                .collect();
            rows.sort_by_key(|r| r.0);
            frames.push(rows);
            let mut e = app.world_mut().query::<&SmokeEmitter>();
            smoke.push(e.iter(app.world()).map(|s| s.level).collect());
        }
        (frames, smoke)
    }

    #[test]
    fn impact_timeline_is_identical_across_runs_and_cleans_up() {
        let ((a, sa), (b, sb)) = (run(), run());
        assert_eq!(a, b);
        assert_eq!(sa, sb);
        let count = |k: ImpactKind| a.iter().flatten().filter(|r| r.0.0 == k).map(|r| (r.0.2, r.0.3)).collect::<std::collections::BTreeSet<_>>().len();
        // Shield hits at 20, 40, 60 plus the shields-down pulse; sparks from the two hull hits.
        assert_eq!(count(ImpactKind::Shield), 4);
        assert_eq!(count(ImpactKind::Spark), 2 * HIT_SPARKS as usize + BLAST_SPARKS as usize);
        assert_eq!(count(ImpactKind::Fire), 1);
        assert_eq!(count(ImpactKind::Ring), 1);
        assert_eq!(count(ImpactKind::Debris), DEBRIS as usize);
        assert!(count(ImpactKind::Puff) > 5);
        // A shield shell fades monotonically.
        let first = a.iter().flatten().find(|r| r.0.0 == ImpactKind::Shield).map(|r| r.0).unwrap();
        let alphas: Vec<f32> = a.iter().flatten().filter(|r| r.0 == first).map(|r| f32::from_bits(r.3)).collect();
        assert!(alphas.len() > 5 && alphas.windows(2).all(|w| w[1] <= w[0]));
        // Smoke: none, light, heavy, light, none, then heavy until the unit dies, then gone.
        let mut levels: Vec<u8> = sa.iter().map(|v| v.first().copied().unwrap_or(0)).collect();
        levels.dedup();
        assert_eq!(levels, vec![0, 1, 2, 1, 0, 2, 0]);
        // Everything drains after the destruction.
        assert!(a.last().unwrap().is_empty(), "FX left: {:?}", a.last());
        assert!(sa.last().unwrap().is_empty());
    }
}
