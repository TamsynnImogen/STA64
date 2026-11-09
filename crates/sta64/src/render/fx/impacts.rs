//! Impact FX (RND-10): shield hits, hull sparks, explosions and damage smoke
//! from `SimEvent::{Hit, ShieldsDown, Destroyed}` and `HullShare`.
//!
//! Refit's `effects_{shields,explosions}.hlsl` are not in this repo, so the
//! looks are plain shapes (ellipsoid shell, spheres) shaded unlit-additive;
//! durations, counts and sizes below are this module's own and tuned by eye.
//! Particles here are tiny deterministic spheres seeded from
//! `(unit, tick, index)`: `render/particles.rs` is the SOD sprite-emitter
//! system and wants sprite tables the sim events do not name.
//!
//! - **Shield hit** (`Hit.shield > 0`, and `ShieldsDown` as a longer, brighter
//!   pulse): an ellipsoid shell around the target, pushed toward the hit
//!   facing (0 fore, 1 aft, 2 port, 3 starboard, by the heading at hit time),
//!   fading out. It follows the target.
//! - **Hull impact** (`Hit.hull > 0`): a burst of sparks from `Hit.at`.
//! - **Explosion** (`Destroyed`): a flash, an expanding shell and debris, sized
//!   by the unit's `SelectRadius` (last seen, the unit is gone from the view).
//! - **Damage smoke**: a [`SmokeEmitter`] is kept for each unit root whose
//!   [`HullShare`] is under [`SMOKE_LIGHT`] (one stream) or [`SMOKE_HEAVY`]
//!   (two); it detaches above the threshold, while cloaked, and with the root.
//!   Puffs already out run their course.
//!
//! All time is [`AnimClock`]; FX are keyed by `(kind, unit, fired, idx)`.

use bevy::prelude::*;
use sta_sim::{EntityId, SimEvent, TICK_HZ, Tick};
use std::collections::BTreeMap;

use super::weapons::FxAlpha;
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::units::{Cloaked, HullShare, SelectRadius, to_bevy};
use crate::sim_link::{SimEntities, SimEntity, SimLink, SimMsg};

/// Hull share under which a unit smokes lightly, and heavily.
pub const SMOKE_LIGHT: f32 = 0.5;
pub const SMOKE_HEAVY: f32 = 0.25;
const DEFAULT_RADIUS: f32 = 30.0;
const SPARKS: u8 = 6;
const DEBRIS: u8 = 12;
const SHIELD_TICKS: f32 = 14.0;
const SHIELD_DOWN_TICKS: f32 = 30.0;
const SPARK_TICKS: f32 = 9.0;
const FLASH_TICKS: f32 = 8.0;
const SHELL_TICKS: f32 = 30.0;
const DEBRIS_TICKS: f32 = 40.0;
const PUFF_TICKS: f32 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImpactKind {
    Shield,
    Spark,
    Flash,
    Shell,
    Debris,
    Puff,
}

/// One impact effect, positioned purely from its age.
#[derive(Component, Debug, Clone)]
pub struct ImpactFx {
    pub kind: ImpactKind,
    pub unit: EntityId,
    pub idx: u8,
    pub fired: Tick,
    pub life: f32,
    origin: Vec3,
    /// Units per second (sparks, debris, smoke).
    vel: Vec3,
    /// Render-space direction the shield shell faces.
    dir: Vec3,
    radius: f32,
    colour: Color,
}

/// Kept for a unit while it is damaged. `level` 1 light, 2 heavy.
#[derive(Component, Debug, Clone)]
pub struct SmokeEmitter {
    pub unit: EntityId,
    pub level: u8,
    last: Tick,
}

/// Last known place, heading and size of each unit seen.
#[derive(Resource, Default)]
struct Seen(BTreeMap<EntityId, (Vec3, Vec3, f32)>);

pub struct ImpactFxPlugin;

impl Plugin for ImpactFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Seen>().add_systems(
            Update,
            (track, spawn_impacts, smoke_attach, emit_smoke, update_impacts, attach_visuals, apply_alpha)
                .chain()
                .in_set(GameSet::Interpolate)
                .run_if(resource_exists::<SimLink>),
        );
    }
}

/// splitmix64 to a float in `[0, 1)`; one stream per `(unit, tick, idx, salt)`.
fn rand(unit: EntityId, tick: Tick, idx: u8, salt: u8) -> f32 {
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

fn track(link: Res<SimLink>, ents: Option<Res<SimEntities>>, radii: Query<&SelectRadius>, mut seen: ResMut<Seen>) {
    for u in &link.curr().units {
        let radius = ents.as_ref().and_then(|m| m.0.get(&u.id)).and_then(|e| radii.get(*e).ok()).map_or(DEFAULT_RADIUS, |r| r.0);
        let heading = Vec3::new(u.heading.0.to_f32(), 0.0, -u.heading.1.to_f32()).normalize_or(Vec3::NEG_Z);
        seen.0.insert(u.id, (to_bevy(u.pos.to_f32()), heading, radius));
    }
}

fn spawn_impacts(mut cmds: Commands, mut msgs: MessageReader<SimMsg>, clock: Res<AnimClock>, mut seen: ResMut<Seen>) {
    let tick = clock.tick();
    for SimMsg(ev) in msgs.read() {
        match ev {
            SimEvent::Hit { target, facing, shield, hull, at, .. } => {
                let (centre, heading, radius) = seen.0.get(target).copied().unwrap_or((Vec3::ZERO, Vec3::NEG_Z, DEFAULT_RADIUS));
                let dir = facing_dir(heading, *facing);
                let mk = |kind, idx, life, origin, vel, colour| {
                    (ImpactFx { kind, unit: *target, idx, fired: tick, life, origin, vel, dir, radius, colour }, FxAlpha(1.0), Transform::from_translation(origin))
                };
                if shield.to_f32() > 0.0 {
                    cmds.spawn(mk(ImpactKind::Shield, 0, SHIELD_TICKS, centre, Vec3::ZERO, Color::srgb(0.3, 0.6, 1.0)));
                }
                if hull.to_f32() > 0.0 {
                    let p = to_bevy(at.to_f32());
                    for i in 0..SPARKS {
                        let v = (dir + unit_dir(|s| rand(*target, tick, i, s)) * 0.8).normalize_or(dir) * (40.0 + 50.0 * rand(*target, tick, i, 2));
                        cmds.spawn(mk(ImpactKind::Spark, i, SPARK_TICKS, p, v, Color::srgb(1.0, 0.8, 0.3)));
                    }
                }
            }
            SimEvent::ShieldsDown { id, facing } => {
                let (centre, heading, radius) = seen.0.get(id).copied().unwrap_or((Vec3::ZERO, Vec3::NEG_Z, DEFAULT_RADIUS));
                let dir = facing_dir(heading, *facing);
                cmds.spawn((
                    ImpactFx { kind: ImpactKind::Shield, unit: *id, idx: 1, fired: tick, life: SHIELD_DOWN_TICKS, origin: centre, vel: Vec3::ZERO, dir, radius, colour: Color::srgb(0.6, 0.8, 1.0) },
                    FxAlpha(1.0),
                    Transform::from_translation(centre),
                ));
            }
            SimEvent::Destroyed { id, .. } => {
                let Some((p, _, radius)) = seen.0.remove(id) else { continue };
                let mk = |kind, idx, life, vel, colour| {
                    (ImpactFx { kind, unit: *id, idx, fired: tick, life, origin: p, vel, dir: Vec3::Y, radius, colour }, FxAlpha(1.0), Transform::from_translation(p))
                };
                cmds.spawn(mk(ImpactKind::Flash, 0, FLASH_TICKS, Vec3::ZERO, Color::WHITE));
                cmds.spawn(mk(ImpactKind::Shell, 0, SHELL_TICKS, Vec3::ZERO, Color::srgb(1.0, 0.55, 0.2)));
                for i in 0..DEBRIS {
                    let v = unit_dir(|s| rand(*id, tick, i, s)) * radius * (0.8 + 1.4 * rand(*id, tick, i, 2));
                    cmds.spawn(mk(ImpactKind::Debris, i, DEBRIS_TICKS, v, Color::srgb(1.0, 0.4, 0.1)));
                }
            }
            _ => {}
        }
    }
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
        let Some(&(p, _, radius)) = seen.0.get(&s.unit) else { continue };
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
                    origin,
                    vel: Vec3::new(r(5) * 6.0, 10.0 + 6.0 * (r(6) + 0.5), r(7) * 6.0),
                    dir: Vec3::Y,
                    radius,
                    colour: Color::srgb(0.25, 0.25, 0.25),
                },
                FxAlpha(1.0),
                Transform::from_translation(origin),
            ));
        }
    }
}

fn update_impacts(mut cmds: Commands, clock: Res<AnimClock>, seen: Res<Seen>, link: Res<SimLink>, mut q: Query<(Entity, &ImpactFx, &mut Transform, &mut FxAlpha)>) {
    let now = clock.now() * TICK_HZ as f64;
    let alpha = clock.alpha();
    for (e, fx, mut tf, mut a) in &mut q {
        let age = (now - fx.fired as f64) as f32;
        if age >= fx.life {
            cmds.entity(e).despawn();
            continue;
        }
        let k = (age / fx.life).clamp(0.0, 1.0);
        let secs = age.max(0.0) / TICK_HZ as f32;
        let at = fx.origin + fx.vel * secs;
        let r = fx.radius;
        match fx.kind {
            ImpactKind::Shield => {
                let centre = link.interpolate(fx.unit, alpha).map(|(p, _)| to_bevy(p)).or_else(|| seen.0.get(&fx.unit).map(|s| s.0)).unwrap_or(fx.origin);
                // Local z runs along the facing: a shell longer than it is tall.
                *tf = Transform::from_translation(centre + fx.dir * r * 0.1).looking_to(fx.dir, Vec3::Y).with_scale(Vec3::new(r * 1.2, r * 0.8, r * 1.5) * (1.0 + 0.1 * k));
                a.0 = (if fx.idx == 1 { 1.0 } else { 0.7 }) * (1.0 - k) * (1.0 - k);
            }
            ImpactKind::Spark => {
                *tf = Transform::from_translation(at).with_scale(Vec3::splat(1.5 * (1.0 - k)));
                a.0 = 1.0 - k;
            }
            ImpactKind::Flash => {
                *tf = Transform::from_translation(at).with_scale(Vec3::splat(r * 1.5 * (1.0 - k)));
                a.0 = 1.0 - k;
            }
            ImpactKind::Shell => {
                *tf = Transform::from_translation(at).with_scale(Vec3::splat(r * (0.3 + 2.2 * k.sqrt())));
                a.0 = 0.8 * (1.0 - k);
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
fn attach_visuals(
    mut cmds: Commands,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mats: Option<ResMut<Assets<StandardMaterial>>>,
    new: Query<(Entity, &ImpactFx), Added<ImpactFx>>,
) {
    let (Some(mut meshes), Some(mut mats)) = (meshes, mats) else { return };
    if new.is_empty() {
        return;
    }
    let ball = meshes.add(Sphere::new(1.0));
    for (e, fx) in &new {
        let blend = if fx.kind == ImpactKind::Puff { AlphaMode::Blend } else { AlphaMode::Add };
        let material = mats.add(StandardMaterial { base_color: fx.colour, emissive: fx.colour.to_linear() * 3.0, unlit: true, alpha_mode: blend, ..default() });
        cmds.entity(e).insert((Mesh3d(ball.clone()), MeshMaterial3d(material)));
    }
}

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
        for f in 0..420 {
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
        assert_eq!(count(ImpactKind::Spark), 2 * SPARKS as usize);
        assert_eq!(count(ImpactKind::Flash), 1);
        assert_eq!(count(ImpactKind::Shell), 1);
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
