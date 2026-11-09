//! Weapon FX (RND-09): beams, projectiles and muzzle flashes from
//! `SimEvent::Fired`, removed on expiry or `Hit`.
//!
//! The reference mod (Refit's `render3d/mods/ordnance`) is not in this repo;
//! kinds, durations and speeds come from the weapon's `WeaponDef` in the
//! content database (`kind`, `duration`, `shot_speed`, `class`, `fx`), the
//! colours from those names. Sprites and ODF beam widths wait for CON data.
//!
//! - **Beam**: a stretched box from the shooter to the target, both followed
//!   live each frame; alpha fades 1 to 0 over `duration` ticks.
//! - **Projectile**: a small sphere flying from the firing point to the
//!   target's (live, interpolated) position. The sim's shots are not in the
//!   view, so flight time is derived at fire time as distance / `shot_speed`
//!   (the sim hits on arrival) and progress is linear in anim time; the
//!   sphere is removed on the matching `Hit` or at arrival, whichever first.
//! - **Muzzle flash**: a short-lived sphere at the shooter, for every shot.
//! - Point-defence shots only flash.
//!
//! Shots leave the shooter's centre: the view has no hardpoint positions yet
//! (the sim has none either). The fire tick is `AnimClock::tick()` when the
//! message is read, exact at one tick per frame or fewer.
//!
//! All time is [`AnimClock`].

use bevy::prelude::*;
use sta_content::WeaponKind;
use sta_sim::command::Target;
use sta_sim::{ContentKey, EntityId, SimEvent, TICK_HZ, Tick};

use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::units::to_bevy;
use crate::sim_link::{ContentRes, SimLink, SimMsg};

/// Ticks a muzzle flash lasts.
pub const FLASH_TICKS: Tick = 6;
const BEAM_WIDTH: f32 = 3.0;
const FLASH_RADIUS: f32 = 8.0;
const SHOT_RADIUS: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FxKind {
    Beam,
    Projectile,
    Flash,
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
    colour: Color,
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

fn colour_for(class: &str, fx: Option<&str>) -> Color {
    let s = format!("{class} {}", fx.unwrap_or("")).to_ascii_lowercase();
    if s.contains("disruptor") {
        Color::srgb(0.2, 1.0, 0.3)
    } else if s.contains("polaron") {
        Color::srgb(0.7, 0.4, 1.0)
    } else if s.contains("torpedo") {
        Color::srgb(0.3, 0.6, 1.0)
    } else if s.contains("phaser") || s.contains("beam") {
        Color::srgb(1.0, 0.45, 0.2)
    } else {
        Color::WHITE
    }
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
                let colour = colour_for(&def.class.0, def.fx.as_deref());
                let mk = |kind, life: f32| WeaponFx { kind, shooter: *shooter, weapon: *weapon, target: *target, fired: clock.tick(), life, start: from, from, to, colour };
                cmds.spawn((mk(FxKind::Flash, FLASH_TICKS as f32), FxAlpha(1.0), Transform::from_translation(from)));
                match def.kind {
                    WeaponKind::Beam => {
                        cmds.spawn((mk(FxKind::Beam, def.duration.max(1) as f32), FxAlpha(1.0), Transform::from_translation(from.midpoint(to))));
                    }
                    WeaponKind::Projectile => {
                        let per_tick = def.shot_speed.to_f32() / TICK_HZ as f32;
                        let flight = if per_tick > 0.0 { (from.distance(to) / per_tick).ceil().max(1.0) } else { 1.0 };
                        cmds.spawn((mk(FxKind::Projectile, flight), FxAlpha(1.0), Transform::from_translation(from)));
                    }
                    _ => {}
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

fn update_fx(mut cmds: Commands, clock: Res<AnimClock>, link: Res<SimLink>, mut q: Query<(Entity, &mut WeaponFx, &mut Transform, &mut FxAlpha)>) {
    let now_ticks = clock.now() * TICK_HZ as f64;
    let alpha = clock.alpha();
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
        match fx.kind {
            FxKind::Beam => {
                let d = fx.to - fx.from;
                let len = d.length();
                let dir = if len > 1e-4 { d / len } else { Vec3::Z };
                *tf = Transform::from_translation(fx.from.midpoint(fx.to)).looking_to(dir, Vec3::Y).with_scale(Vec3::new(BEAM_WIDTH, BEAM_WIDTH, len.max(1e-3)));
                a.0 = 1.0 - k;
            }
            FxKind::Projectile => {
                *tf = Transform::from_translation(fx.start.lerp(fx.to, k)).with_scale(Vec3::splat(SHOT_RADIUS));
                a.0 = 1.0;
            }
            FxKind::Flash => {
                *tf = Transform::from_translation(fx.from).with_scale(Vec3::splat(FLASH_RADIUS * (1.0 - k)));
                a.0 = 1.0 - k;
            }
        }
    }
}

/// Mesh and material for new effects, when the render assets exist (not in
/// windowless tests).
fn attach_visuals(
    mut cmds: Commands,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mats: Option<ResMut<Assets<StandardMaterial>>>,
    new: Query<(Entity, &WeaponFx), Added<WeaponFx>>,
) {
    let (Some(mut meshes), Some(mut mats)) = (meshes, mats) else { return };
    if new.is_empty() {
        return;
    }
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let ball = meshes.add(Sphere::new(1.0));
    for (e, fx) in &new {
        let mesh = if fx.kind == FxKind::Beam { cube.clone() } else { ball.clone() };
        let material = mats.add(StandardMaterial { base_color: fx.colour, emissive: fx.colour.to_linear() * 4.0, unlit: true, alpha_mode: AlphaMode::Add, ..default() });
        cmds.entity(e).insert((Mesh3d(mesh), MeshMaterial3d(material)));
    }
}

fn apply_alpha(mats: Option<ResMut<Assets<StandardMaterial>>>, q: Query<(&WeaponFx, &FxAlpha, &MeshMaterial3d<StandardMaterial>), Changed<FxAlpha>>) {
    let Some(mut mats) = mats else { return };
    for (fx, a, m) in &q {
        if let Some(mut mat) = mats.get_mut(&m.0) {
            mat.base_color = fx.colour.with_alpha(a.0);
            mat.emissive = fx.colour.to_linear() * (4.0 * a.0);
        }
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
        // A beam fades: its alpha only falls while it lives.
        let first = fired(FxKind::Beam).into_iter().next().unwrap();
        let alphas: Vec<f32> = a.iter().flatten().filter(|r| r.0.2 == FxKind::Beam && (r.0.1, r.0.3) == first).map(|r| f32::from_bits(r.3)).collect();
        assert!(alphas.len() > 5 && alphas.windows(2).all(|w| w[1] <= w[0]) && alphas[0] > alphas[alphas.len() - 1]);
        assert!(a.last().unwrap().is_empty(), "FX left after the timeline: {:?}", a.last());
    }
}
