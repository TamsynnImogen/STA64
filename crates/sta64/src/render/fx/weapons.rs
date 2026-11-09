//! Weapon FX (RND-09): beams, bolts, cones and muzzle flashes from
//! `SimEvent::Fired`, removed on expiry or `Hit`.
//!
//! Aligned with Armada II Refit's ordnance module (`render3d/mods/ordnance`): the
//! look is its `fx_ps` shader ported to WGSL (`quad.wesl`: the beam's core and glow
//! profile, flowing pulses, helix strands and crackle; the bolt with its tail and
//! rays; the flare; the tractor cone's fans and vapour) and its recipes
//! (`ordnance.ini`, by the weapon's ODF name, [`super::recipe`]). Refit leaves the
//! ordinary phasers and torpedoes to stock's sprites and lights them only
//! (`lights.rs`); with no sprites here they take the recipe defaults of their kind, in
//! a colour from the weapon's class and effect name, else its ODF `lightColor`.
//!
//! - **Beam** (`WeaponKind::Beam`, or a recipe of kind beam/cone): a ribbon from the
//!   shooter to the target, both followed live each frame, `duration` ticks long.
//!   Brightness rises over the recipe's `FadeIn` and falls over the last 0.25 s (the
//!   phaser lights' envelope). A flare sits at the emitter (`Emitter` half widths) and
//!   at the impact (`Impact`), as in Refit's `Fx::beam`.
//! - **Cone** (recipe `Kind=cone`: tractor, holding, assimilation beams): Refit's
//!   fans-and-vapour cone, `Spread` of the target's radius wide at the target. The
//!   view has no target radius here, so the cone is Refit's fallback shape without a
//!   target: an eighth as wide as it is long.
//! - **Projectile**: a bolt flying from the firing point to the target's (live,
//!   interpolated) position. The sim's shots are not in the view, so flight time is
//!   derived at fire time as distance / `shot_speed` (the sim hits on arrival) and
//!   progress is linear in anim time; removed on the matching `Hit` or at arrival.
//! - **Muzzle flash**: a flare at the shooter for every shot, for Refit's
//!   `MuzzleTime` (0.5 s), standing in for its muzzle light.
//! - Point-defence shots only flash.
//!
//! Shots leave the shooter's centre: the view has no hardpoint positions yet (the sim
//! has none either). The fire tick is `AnimClock::tick()` when the message is read,
//! exact at one tick per frame or fewer.
//!
//! All time is [`AnimClock`].

use bevy::prelude::*;
use sta_content::WeaponKind;
use sta_sim::command::Target;
use sta_sim::{ContentKey, EntityId, SimEvent, TICK_HZ, Tick};

use super::quad::{self, FxQuadMaterial, Look};
use super::recipe::{self, Recipe, Shape};
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::units::to_bevy;
use crate::sim_link::{ContentRes, SimLink, SimMsg};

/// Ticks a muzzle flash lasts: Refit's `MuzzleTime`, 0.5 s.
pub const FLASH_TICKS: Tick = TICK_HZ as Tick / 2;
/// Seconds a beam fades over at its end (Refit's phaser lights, `Lighting.ini`).
const BEAM_FALL: f32 = 0.25;
/// Refit's `Fx::beam` flare: core sharpness and the star's.
const FLARE_CORE: f32 = 30.0;
const FLARE_STAR_SHARP: f32 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FxKind {
    Beam,
    Projectile,
    /// The emitter's flare, the muzzle.
    Flash,
    /// The flare where a beam lands.
    Impact,
}

/// One weapon effect. Tests key on `(shooter, weapon, kind, fired)`, never
/// on the Bevy `Entity`.
#[derive(Component, Debug, Clone)]
pub struct WeaponFx {
    pub kind: FxKind,
    pub shooter: EntityId,
    pub weapon: ContentKey,
    pub target: Target,
    pub fired: Tick,
    /// Ticks the effect lives.
    pub life: f32,
    /// Where it started, in render space (projectiles).
    start: Vec3,
    /// Last known ends, kept when a unit disappears.
    from: Vec3,
    to: Vec3,
    recipe: Recipe,
    look: Look,
    seed: f32,
}

/// Opacity 0..1, applied to the material when there is one.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct FxAlpha(pub f32);

pub struct WeaponFxPlugin;

impl Plugin for WeaponFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (spawn_fx, update_fx, attach_visuals, apply_alpha)
                .chain()
                .in_set(GameSet::Interpolate)
                .run_if(resource_exists::<SimLink>)
                .run_if(resource_exists::<ContentRes>),
        );
    }
}

/// A colour from the weapon's class and effect name, as the stock effects read.
fn name_colour(class: &str, fx: Option<&str>) -> Option<Vec3> {
    let s = format!("{class} {}", fx.unwrap_or("")).to_ascii_lowercase();
    Some(if s.contains("disruptor") {
        Vec3::new(0.2, 1.0, 0.3)
    } else if s.contains("polaron") {
        Vec3::new(0.7, 0.4, 1.0)
    } else if s.contains("torpedo") {
        Vec3::new(0.3, 0.6, 1.0)
    } else if s.contains("phaser") || s.contains("beam") {
        Vec3::new(1.0, 0.45, 0.2)
    } else {
        return None;
    })
}

/// The recipe a weapon draws with: Refit's, by ODF name, else its kind's defaults in
/// the weapon's colour (class and effect name, else the ODF's `lightColor`: three
/// numbers, 0..1 or 0..255).
fn recipe_for(def: &sta_content::WeaponDef) -> Recipe {
    if let Some(r) = recipe::bundled().get(&def.odf.to_ascii_lowercase()) {
        return r.clone();
    }
    let shape = if def.kind == WeaponKind::Beam { Shape::Beam } else { Shape::Bolt };
    let mut r = Recipe::new(shape);
    let light: Vec<f32> = def.raw.list("lightColor").iter().filter_map(|t| t.trim_end_matches(['f', 'F', ';']).parse().ok()).collect();
    let odf = match light[..] {
        [r, g, b, ..] if r.max(g).max(b) > 0.0 => Some(Vec3::new(r, g, b) / if r.max(g).max(b) > 1.0 { 255.0 } else { 1.0 }),
        _ => None,
    };
    if let Some(c) = name_colour(&def.class.0, def.fx.as_deref()).or(odf) {
        r.col = c;
        r.core = c.lerp(Vec3::ONE, 0.85);
    }
    r
}

fn point(link: &SimLink, id: EntityId, alpha: f32) -> Option<Vec3> {
    link.interpolate(id, alpha).map(|(p, _)| to_bevy(p))
}

fn target_point(link: &SimLink, t: &Target, alpha: f32) -> Option<Vec3> {
    match t {
        Target::Entity(id) => point(link, *id, alpha),
        Target::Point(p) => Some(to_bevy(p.to_f32())),
        _ => None,
    }
}

/// A per-shot seed in 0..10.23, as Refit's `(hash & 1023) * 0.01`.
fn shot_seed(shooter: EntityId, weapon: ContentKey, tick: Tick) -> f32 {
    let mut h = 0x811c_9dc5u32;
    for x in [shooter.0, weapon.0, tick as u32] {
        h = (h ^ x).wrapping_mul(0x0100_0193);
        h ^= h >> 15;
    }
    (h & 1023) as f32 * 0.01
}

fn spawn_fx(
    mut cmds: Commands,
    mut msgs: MessageReader<SimMsg>,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    content: Res<ContentRes>,
    live: Query<(Entity, &WeaponFx)>,
) {
    let alpha = clock.alpha();
    for SimMsg(ev) in msgs.read() {
        match ev {
            SimEvent::Fired { shooter, weapon, target, .. } => {
                let Some(def) = content.0.weapon(*weapon) else { continue };
                let Some(from) = point(&link, *shooter, alpha) else { continue };
                let to = target_point(&link, target, alpha).unwrap_or(from);
                let recipe = recipe_for(def);
                let seed = shot_seed(*shooter, *weapon, clock.tick());
                let mk = |kind, life: f32, look: Look| WeaponFx {
                    kind,
                    shooter: *shooter,
                    weapon: *weapon,
                    target: *target,
                    fired: clock.tick(),
                    life,
                    start: from,
                    from,
                    to,
                    recipe: recipe.clone(),
                    look,
                    seed,
                };
                // Refit's emitter flare: `Fx::flare` at the start, `Emitter` half widths.
                let flare = Look::flare(recipe.col, FLARE_CORE, 3.5, recipe.star, FLARE_STAR_SHARP, 0.0, 20.0, seed);
                cmds.spawn((mk(FxKind::Flash, FLASH_TICKS as f32, flare), FxAlpha(1.0), Transform::from_translation(from)));
                let per_tick = def.shot_speed.to_f32() / TICK_HZ as f32;
                let flight = if per_tick > 0.0 { (from.distance(to) / per_tick).ceil().max(1.0) } else { 1.0 };
                match def.kind {
                    WeaponKind::PointDefence => {}
                    WeaponKind::Projectile if recipe.shape == Shape::Bolt => {
                        cmds.spawn((mk(FxKind::Projectile, flight, Look::new(quad::K_BOLT, recipe.col)), FxAlpha(0.0), Transform::from_translation(from)));
                    }
                    // A beam weapon runs its ribbon for `duration`; a recipe of beam or
                    // cone kind on a shot the sim resolves on arrival, for its flight.
                    _ => {
                        let life = if def.kind == WeaponKind::Beam { def.duration.max(1) as f32 } else { flight };
                        let kind = if recipe.shape == Shape::Cone { quad::K_CONE } else { quad::K_BEAM };
                        cmds.spawn((mk(FxKind::Beam, life, Look::new(kind, recipe.col)), FxAlpha(0.0), Transform::from_translation(from.midpoint(to))));
                        cmds.spawn((mk(FxKind::Impact, life, flare), FxAlpha(0.0), Transform::from_translation(to)));
                    }
                }
            }
            SimEvent::Hit { target, by, weapon, .. } => {
                // The earliest matching shot in flight has arrived.
                let hit = live
                    .iter()
                    .filter(|(_, f)| f.kind == FxKind::Projectile && f.shooter == *by && f.weapon == *weapon && f.target == Target::Entity(*target))
                    .min_by_key(|(_, f)| f.fired);
                if let Some((e, _)) = hit {
                    cmds.entity(e).despawn();
                }
            }
            _ => {}
        }
    }
}

/// The beam's brightness envelope: up over `fade_in` seconds, down over the last
/// [`BEAM_FALL`].
fn envelope(age: f32, life: f32, fade_in: f32) -> f32 {
    let t = TICK_HZ as f32;
    ((age / t) / fade_in).clamp(0.0, 1.0) * (((life - age) / t) / BEAM_FALL).clamp(0.0, 1.0)
}

#[allow(clippy::type_complexity)]
fn update_fx(
    mut cmds: Commands,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    mut q: Query<(Entity, &mut WeaponFx, &mut Transform, &mut FxAlpha)>,
) {
    let now_ticks = clock.now() * TICK_HZ as f64;
    let alpha = clock.alpha();
    let eye = camera.single().ok().map(|c| c.translation());
    for (e, mut fx, mut tf, mut a) in &mut q {
        let age = (now_ticks - fx.fired as f64) as f32;
        if age >= fx.life {
            cmds.entity(e).despawn();
            continue;
        }
        let k = (age / fx.life).clamp(0.0, 1.0);
        if let Some(p) = point(&link, fx.shooter, alpha) {
            fx.from = p;
        }
        if let Some(p) = target_point(&link, &fx.target, alpha) {
            fx.to = p;
        }
        let r = fx.recipe.clone();
        match fx.kind {
            FxKind::Beam => {
                let d = fx.to - fx.from;
                let len = d.length().max(1e-3);
                let hw = r.width * 0.5;
                if r.shape == Shape::Cone {
                    // Refit's no-target cone: an eighth as wide as long, `Spread` of it.
                    let tr = (len * 0.125).max(r.width);
                    let hw1 = (r.spread * tr).max(hw);
                    let s_end = 1.0 + r.beyond * tr / len;
                    let end = fx.from + d / len * (len * s_end);
                    *tf = quad::ribbon(eye, fx.from, end, hw1, 0.0);
                    fx.look.len = len / hw1;
                    fx.look.col2 = Vec3::new(s_end, 0.0, 0.0);
                    fx.look.extra = hw / hw1;
                    fx.look.p3 = Vec4::new(r.fans[0].clamp(0.0, 4.0), r.fans[1].clamp(0.0, 1.0), r.fans[2], r.edge_px);
                    fx.look.p4 = Vec4::new(r.vapour[0].clamp(0.0, 1.0), r.vapour[1], r.vapour[2], r.fall);
                } else {
                    // Capped at both ends by a half width, as `Fx::beam`.
                    *tf = quad::ribbon(eye, fx.from, fx.to, hw, hw);
                    fx.look.len = len / hw;
                    fx.look.col2 = r.core;
                    fx.look.p3 = Vec4::new(r.core_w, r.flow[0], r.flow[1], r.flow[2]);
                    fx.look.p4 = Vec4::new(r.helix[0], r.helix[1], r.helix[2], r.crackle);
                }
                fx.look.seed = fx.seed;
                a.0 = envelope(age, fx.life, r.fade_in);
            }
            FxKind::Projectile => {
                let at = fx.start.lerp(fx.to, k);
                let dir = (fx.to - fx.start).normalize_or(Vec3::Z);
                let rad = r.size;
                *tf = quad::ribbon(eye, at - dir * rad * r.tail, at + dir * rad, rad, 0.0);
                fx.look.len = r.tail;
                fx.look.col2 = r.core;
                fx.look.seed = fx.seed;
                fx.look.p3 = Vec4::new(18.0, 3.0, r.rays, clock.now() as f32 * r.spin + fx.seed);
                a.0 = ((age / TICK_HZ as f32) / r.fade_in).clamp(0.0, 1.0);
            }
            FxKind::Flash => {
                let h = if r.shape == Shape::Bolt { r.size * 1.5 } else { r.emitter * r.width * 0.5 };
                *tf = quad::billboard(eye, fx.from, Vec2::splat(h));
                a.0 = (1.0 - k) * (1.0 - k);
            }
            FxKind::Impact => {
                let h = if r.shape == Shape::Bolt { r.size } else { r.impact * r.width * 0.5 };
                *tf = quad::billboard(eye, fx.to, Vec2::splat(h));
                a.0 = envelope(age, fx.life, r.fade_in);
            }
        }
    }
}

/// Mesh and material for new effects, when the render assets exist (not in
/// windowless tests).
fn attach_visuals(
    mut cmds: Commands,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mats: Option<ResMut<Assets<FxQuadMaterial>>>,
    mut quad_mesh: Local<Option<Handle<Mesh>>>,
    new: Query<(Entity, &WeaponFx), Added<WeaponFx>>,
) {
    let (Some(mut meshes), Some(mut mats)) = (meshes, mats) else { return };
    if new.is_empty() {
        return;
    }
    let mesh = quad_mesh.get_or_insert_with(|| meshes.add(quad::rectangle())).clone();
    for (e, fx) in &new {
        cmds.entity(e).insert((Mesh3d(mesh.clone()), MeshMaterial3d(mats.add(quad::material(&fx.look)))));
    }
}

fn apply_alpha(clock: Res<AnimClock>, mats: Option<ResMut<Assets<FxQuadMaterial>>>, q: Query<(&WeaponFx, &FxAlpha, &MeshMaterial3d<FxQuadMaterial>)>) {
    let Some(mut mats) = mats else { return };
    let t = quad::shader_clock(clock.now());
    for (fx, a, m) in &q {
        // Refit's flares are drawn at 0.6 of the beam's brightness.
        let share = if matches!(fx.kind, FxKind::Flash | FxKind::Impact) { 0.6 } else { 1.0 };
        let flicker = fx.recipe.flicker_at(t, fx.seed);
        quad::drive(&mut mats, &m.0, &fx.look, fx.recipe.bright * share * a.0 * flicker, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::sim_link::SimLinkPlugin;
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::PlayerId;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;
    use std::time::Duration;

    type Row = ((u32, u32, FxKind, Tick), [u32; 3], [u32; 3], u32);

    fn timeline(db: &sta_content::ContentDb) -> String {
        let beam = db.key("fx_beam").unwrap().0;
        let torp = db.key("fx_torpedo").unwrap().0;
        let mut ev = String::new();
        // SIM-04's cadence: beam every 45 ticks (15 drawn), torpedo every 90,
        // arriving after 600 / 400 s = 90 ticks.
        for t in (45..=180).step_by(45) {
            ev += &format!("{{ at: {t}, event: {{ Fired: {{ shooter: 1, weapon: {beam}, hardpoint: 0, target: {{ Entity: 2 }} }} }} }},");
        }
        for t in [90, 180] {
            ev += &format!("{{ at: {t}, event: {{ Fired: {{ shooter: 1, weapon: {torp}, hardpoint: 1, target: {{ Entity: 2 }} }} }} }},");
            ev += &format!(
                "{{ at: {}, event: {{ Hit: {{ target: 2, by: 1, weapon: {torp}, facing: 0, shield: 0, hull: 60, at: {{ x: 0, y: 0, z: 600 }} }} }} }},",
                t + 90
            );
        }
        format!(
            "{{ units: [
                {{ id: 1, def: \"fx_cruiser\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}, {{ at: 300, pos: {{ x: 50, y: 0, z: 30 }} }}] }},
                {{ id: 2, def: \"fx_frigate\", owner: 1, hull: 400, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 600 }} }}] }},
            ], events: [{ev}] }}"
        )
    }

    fn run() -> Vec<Vec<Row>> {
        let db = sta_content::fake::db();
        let src = ScriptedSource::from_json5(&timeline(&db), &db).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 45.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(ContentRes(std::sync::Arc::new(db)))
            .insert_resource(SimLink::new(Box::new(src), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_plugins(WeaponFxPlugin);
        configure_sets(&mut app);
        let mut frames = Vec::new();
        for _ in 0..420 {
            app.update();
            let mut q = app.world_mut().query::<(&WeaponFx, &Transform, &FxAlpha)>();
            let mut rows: Vec<Row> = q
                .iter(app.world())
                .map(|(f, t, a)| {
                    let b = |v: Vec3| v.to_array().map(f32::to_bits);
                    ((f.shooter.0, f.weapon.0, f.kind, f.fired), b(t.translation), b(t.scale), a.0.to_bits())
                })
                .collect();
            rows.sort_by_key(|r| r.0);
            frames.push(rows);
        }
        frames
    }

    #[test]
    fn firing_timeline_fx_are_identical_across_runs() {
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        let fired = |k: FxKind| a.iter().flatten().filter(|r| r.0.2 == k).map(|r| (r.0.1, r.0.3)).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(fired(FxKind::Beam).len(), 4);
        assert_eq!(fired(FxKind::Projectile).len(), 2);
        assert_eq!(fired(FxKind::Flash).len(), 6);
        // A flare lands with each beam.
        assert_eq!(fired(FxKind::Impact).len(), 4);
        // A beam rises over its fade-in and falls over its last quarter second: the
        // alpha climbs to a single peak of 1 and drops back.
        let first = fired(FxKind::Beam).into_iter().next().unwrap();
        let alphas: Vec<f32> = a.iter().flatten().filter(|r| r.0.2 == FxKind::Beam && (r.0.1, r.0.3) == first).map(|r| f32::from_bits(r.3)).collect();
        let peak = alphas.iter().cloned().fold(0.0, f32::max);
        let at = alphas.iter().position(|&x| x == peak).unwrap();
        assert!(alphas.len() > 5 && peak > 0.99 && at > 0 && at < alphas.len() - 1);
        assert!(alphas[..=at].windows(2).all(|w| w[1] >= w[0]) && alphas[at..].windows(2).all(|w| w[1] <= w[0]));
        assert!(alphas[0] < peak && alphas[alphas.len() - 1] < peak);
        assert!(a.last().unwrap().is_empty(), "FX left after the timeline: {:?}", a.last());
    }
}
