//! Special FX (RND-11): construction, cloak, and the gaps for repair and warp.
//!
//! Aligned with Armada II Refit: the construction welding is its `effects` module's
//! (`effects_construction.hlsl`, the worker bees' beam, arc flare, hot spot and
//! sparks, ported in `quad.wesl`; `[Bees]` and `[Construction]` constants), the cloak
//! is its `hulls` look (`ART-DIRECTION.md`, "Cloaking": the ship wavers out in bands
//! with a thin green front, and the player's own cloaked ships keep 10% of their hull).
//! Refit's `specials` module draws the special weapons' target effects (override,
//! drain, psionic), which have no sim event yet.
//!
//! - **Construction**: while a unit's `UnitView::progress` is `Some`, a bee's welding
//!   beam runs from the builder (bound from `BuildStarted`, matched to the station
//!   by content key) to a weld point on the station's hull, with the arc's flare and
//!   hot spot there and `Sparks` (3) streaks flying off it and cooling. The station
//!   hull is drawn translucent and firms up with progress ([`UnitFade::build`]);
//!   Refit welds along the new edges of the station's SOD instead. `Built` flashes at
//!   the finished station (STA64's own).
//! - **Cloak**: [`UnitFade::cloak`] moves linearly between 1 and
//!   [`CLOAKED_ALPHA`] (Refit's `CloakHull`, 0.1, for the player's own ships) over
//!   [`CLOAK_TICKS`] (Refit's `CloakTime`, 2.5 s) whenever the `Cloaked` marker (from
//!   the view's flag) changes, and `CloakChanged` adds the bands and green front,
//!   strongest mid-transition (`vis (1 - vis)`). Enemy cloaked units never reach here:
//!   RND-02 drops them from the view.
//! - **Repair, warp**: the sim has no repair or warp event or view state
//!   (SIM-14 only has cloak), so there is nothing to draw. Gap, see the PR.
//!
//! [`UnitFade`] is applied to the unit's materials (cloned per mesh on first
//! use so shared model materials are untouched). All time is [`AnimClock`].

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_sim::{ContentKey, EntityId, SimEvent, TICK_HZ, Tick};

use super::impacts::{heat_colour, rand, spark_at};
use super::quad::{self, FxQuadMaterial, Look};
use super::weapons::FxAlpha;
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::units::{Cloaked, SelectRadius, to_bevy};
use crate::sim_link::{SimEntities, SimEntity, SimLink, SimMsg};

/// Ticks a cloak or decloak takes: Refit's `CloakTime`, 2.5 s.
pub const CLOAK_TICKS: f32 = 2.5 * TICK_HZ as f32;
/// Opacity of a cloaked unit its owner still sees: Refit's `CloakHull`, 10%.
pub const CLOAKED_ALPHA: f32 = 0.1;
/// Opacity of a station at zero progress.
pub const BUILD_MIN_ALPHA: f32 = 0.15;
/// Ticks the `Built` flash lasts.
pub const FLASH_TICKS: Tick = 12;
const DEFAULT_RADIUS: f32 = 30.0;

// `[Bees]` (render3d/mods/effects/src/bees.rs, `read`) for the beam; `[Construction]`
// for the sparks and the arc.
const BEAM_BRIGHT: f32 = 1.3;
/// The beam's width (world units) and its minimum in pixels.
const BEAM_WIDTH: f32 = 0.9;
const BEAM_MIN_PX: f32 = 2.5;
const BEAM_COLOUR: Vec3 = Vec3::new(110.0 / 255.0, 160.0 / 255.0, 1.0);
/// The flare at the weld, in beam widths, and its minimum in pixels.
const FLARE: f32 = 3.0;
const FLARE_MIN_PX: f32 = 12.0;
const ARC_COLOUR: Vec3 = Vec3::new(120.0 / 255.0, 165.0 / 255.0, 1.0);
/// The arc flare's core and glow sharpness, rays' share and sharpness (`[Repair]`'s
/// `FlareCore`, `FlareGlow`, `FlareRays`, `FlareRaySharpness`).
const ARC_FLARE: Vec4 = Vec4::new(60.0, 3.5, 1.4, 30.0);
const HOT_COLOUR: Vec3 = Vec3::new(1.0, 120.0 / 255.0, 30.0 / 255.0);
const FLICKER_RATE: f32 = 24.0;
const STUTTER: f32 = 0.2;
const SPARKS: u8 = 3;
/// `SparkSpeed` (times the station's radius), `SparkSpread`, `SparkBrightness`,
/// `SparkWidth` (times the radius) and `SparkMinPixels`.
const SPARK_SPEED: f32 = 0.12;
const SPARK_SPREAD: f32 = 0.9;
const SPARK_BRIGHT: f32 = 2.0;
const SPARK_WIDTH: f32 = 0.002;
const SPARK_MIN_PX: f32 = 1.4;
/// Seconds a spark lives (the middle of `SparkLife` 0.3..0.9).
const SPARK_LIFE: f32 = 0.6;
/// World units one screen pixel spans per unit of distance, at the map camera's
/// field of view and a 1080 line view: what Refit's `*MinPixels` become here.
const PIXEL: f32 = 0.0008;
/// `CloakFront`'s colour, `CloakWaveBands`, `CloakWaveSpeed`, `CloakWaveTilt`.
const FRONT_COLOUR: Vec3 = Vec3::new(0.3, 1.0, 0.55);
const FRONT_BANDS: f32 = 3.0;
const FRONT_SPEED: f32 = 0.6;
const FRONT_TILT: f32 = 0.5;
const FRONT_BRIGHT: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpecialKind {
    /// The bee's welding beam from builder to station.
    Weld,
    /// The arc's flare at the weld.
    Arc,
    /// The hot spot the arc leaves on the hull.
    Hot,
    /// A spark streaking off the weld.
    Spark,
    /// Completion flash.
    BuiltFlash,
    /// Cloak or decloak bands and front.
    Shimmer,
}

/// One special effect. Tests key on `(subject, kind, born)`.
#[derive(Component, Debug, Clone)]
pub struct SpecialFx {
    pub kind: SpecialKind,
    /// The station or cloaking unit.
    pub subject: EntityId,
    pub builder: Option<EntityId>,
    pub born: Tick,
    /// Ticks it lives; `None` while the sim keeps it alive (construction).
    pub life: Option<f32>,
    /// Which of a station's sparks.
    pub idx: u8,
    from: Vec3,
    to: Vec3,
    radius: f32,
    look: Look,
}

/// Opacity of a unit's hull: cloak and construction multiply.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct UnitFade {
    pub cloak: f32,
    pub build: f32,
}

impl UnitFade {
    pub fn alpha(&self) -> f32 {
        self.cloak * self.build
    }
}

/// The unit's [`UnitFade`] is somebody else's to set (the front end's scene decloaks its
/// own ships): [`fade_units`] leaves it alone.
#[derive(Component, Debug, Clone, Copy)]
pub struct ManualFade;

/// Original handle of a material this module replaced with a per-unit clone.
#[derive(Component)]
struct FadeMat {
    orig: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
struct Construction {
    /// `BuildStarted`s not yet bound to a station.
    pending: Vec<(EntityId, ContentKey)>,
    /// Station to its effect entities.
    active: BTreeMap<EntityId, Vec<Entity>>,
}

pub struct SpecialsFxPlugin;

impl Plugin for SpecialsFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Construction>().add_systems(
            Update,
            (read_events, fade_units, sync_construction, update_fx, attach_visuals, apply_quads, apply_fade)
                .chain()
                .in_set(GameSet::Interpolate)
                .run_if(resource_exists::<SimLink>),
        );
    }
}

fn point(link: &SimLink, id: EntityId, alpha: f32) -> Option<Vec3> {
    link.interpolate(id, alpha).map(|(p, _)| to_bevy(p))
}

#[allow(clippy::too_many_arguments)]
fn fx(kind: SpecialKind, subject: EntityId, builder: Option<EntityId>, born: Tick, life: Option<f32>, idx: u8, at: Vec3, look: Look) -> impl Bundle {
    (SpecialFx { kind, subject, builder, born, life, idx, from: at, to: at, radius: DEFAULT_RADIUS, look }, FxAlpha(0.0), Transform::from_translation(at))
}

fn look_for(kind: SpecialKind) -> Look {
    match kind {
        SpecialKind::Weld => Look::new(quad::K_WELD_BEAM, BEAM_COLOUR),
        SpecialKind::Arc => Look { p3: ARC_FLARE, ..Look::new(quad::K_ARC_FLARE, ARC_COLOUR) },
        SpecialKind::Hot => Look::new(quad::K_HOT, HOT_COLOUR),
        SpecialKind::Spark => Look::new(quad::K_SPARK, Vec3::ONE),
        SpecialKind::BuiltFlash => Look::flare(Vec3::new(0.6, 0.9, 1.0), 30.0, 3.5, 0.4, 18.0, 0.0, 20.0, 0.0),
        SpecialKind::Shimmer => Look { p3: Vec4::new(FRONT_BANDS, FRONT_SPEED, FRONT_TILT, 0.0), ..Look::new(quad::K_CLOAK, FRONT_COLOUR) },
    }
}

fn read_events(mut cmds: Commands, mut msgs: MessageReader<SimMsg>, clock: Res<AnimClock>, link: Res<SimLink>, mut cons: ResMut<Construction>) {
    for SimMsg(ev) in msgs.read() {
        match ev {
            SimEvent::BuildStarted { builder, what } => cons.pending.push((*builder, *what)),
            SimEvent::Built { id, .. } => {
                if let Some(at) = point(&link, *id, clock.alpha()) {
                    cmds.spawn(fx(SpecialKind::BuiltFlash, *id, None, clock.tick(), Some(FLASH_TICKS as f32), 0, at, look_for(SpecialKind::BuiltFlash)));
                }
            }
            SimEvent::CloakChanged { id, .. } => {
                if let Some(at) = point(&link, *id, clock.alpha()) {
                    cmds.spawn(fx(SpecialKind::Shimmer, *id, None, clock.tick(), Some(CLOAK_TICKS), 0, at, look_for(SpecialKind::Shimmer)));
                }
            }
            _ => {}
        }
    }
}

/// The beam, arc, hot spot and sparks for every station with progress; the station's
/// build fade.
fn sync_construction(
    mut cmds: Commands,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    mut cons: ResMut<Construction>,
    mut fades: Query<(&SimEntity, &mut UnitFade)>,
) {
    let curr = link.curr().clone();
    let building: BTreeMap<EntityId, (f32, ContentKey)> =
        curr.units.iter().filter_map(|u| u.progress.map(|p| (u.id, (p.to_f32().clamp(0.0, 1.0), u.def)))).collect();
    let gone: Vec<EntityId> = cons.active.keys().filter(|id| !building.contains_key(id)).copied().collect();
    for id in gone {
        for e in cons.active.remove(&id).unwrap() {
            cmds.entity(e).despawn();
        }
    }
    for (&id, &(_, def)) in &building {
        if cons.active.contains_key(&id) {
            continue;
        }
        let slot = cons.pending.iter().position(|(_, what)| *what == def);
        let builder = slot.map(|i| cons.pending.remove(i).0);
        let at = point(&link, id, clock.alpha()).unwrap_or(Vec3::ZERO);
        let mut spawned = Vec::new();
        for kind in [SpecialKind::Weld, SpecialKind::Arc, SpecialKind::Hot] {
            spawned.push(cmds.spawn(fx(kind, id, builder, clock.tick(), None, 0, at, look_for(kind))).id());
        }
        for i in 0..SPARKS {
            spawned.push(cmds.spawn(fx(SpecialKind::Spark, id, builder, clock.tick(), None, i, at, look_for(SpecialKind::Spark))).id());
        }
        cons.active.insert(id, spawned);
    }
    for (e, mut f) in &mut fades {
        let build = building.get(&e.0).map_or(1.0, |(p, _)| BUILD_MIN_ALPHA + (1.0 - BUILD_MIN_ALPHA) * p);
        if f.build != build {
            f.build = build;
        }
    }
}

/// Half size of something `px` pixels across at `dist`, at least `world`.
fn at_least_px(world: f32, px: f32, dist: f32) -> f32 {
    world.max(px * PIXEL * dist * 0.5)
}

/// Refit's stutter: the brightness drops now and then, a new value each flicker.
fn flicker(now: f64, seed: u8) -> f32 {
    let step = (now * FLICKER_RATE as f64).floor() as u32;
    let h = step.wrapping_mul(2654435761).wrapping_add(seed as u32 * 40503) >> 8 & 0xffff;
    1.0 - STUTTER * (h as f32 / 65535.0)
}

#[allow(clippy::type_complexity)]
fn update_fx(
    mut cmds: Commands,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    ents: Option<Res<SimEntities>>,
    radii: Query<&SelectRadius>,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    mut q: Query<(Entity, &mut SpecialFx, &mut Transform, &mut FxAlpha)>,
) {
    let now = clock.now();
    let now_ticks = now * TICK_HZ as f64;
    let alpha = clock.alpha();
    let eye = camera.single().ok().map(|c| c.translation());
    for (e, mut f, mut tf, mut a) in &mut q {
        let age = (now_ticks - f.born as f64) as f32;
        let k = match f.life {
            Some(life) if age >= life => {
                cmds.entity(e).despawn();
                continue;
            }
            Some(life) => (age / life).clamp(0.0, 1.0),
            None => 0.0,
        };
        if let Some(p) = f.builder.and_then(|b| point(&link, b, alpha)) {
            f.from = p;
        }
        let Some(subject) = point(&link, f.subject, alpha) else {
            // Subject gone: nothing left to draw.
            a.0 = 0.0;
            continue;
        };
        f.to = subject;
        f.radius = ents.as_ref().and_then(|m| m.0.get(&f.subject)).and_then(|e| radii.get(*e).ok()).map_or(DEFAULT_RADIUS, |r| r.0);
        // The weld point: on the station's hull, on the side the builder is on.
        let weld = match f.builder {
            Some(_) => f.to + (f.from - f.to).normalize_or(Vec3::Y) * f.radius * 0.6,
            None => f.to,
        };
        let live = if f.builder.is_some() { 1.0 } else { 0.0 };
        let dist = eye.map_or(100.0, |c| c.distance(weld));
        let hw = at_least_px(BEAM_WIDTH * 0.5, BEAM_MIN_PX, dist);
        let seed = f.idx;
        match f.kind {
            SpecialKind::Weld => {
                let from = if f.builder.is_some() { f.from } else { weld };
                *tf = quad::ribbon(eye, from, weld, hw, 0.0);
                // The crackle's phase moves on with each flicker.
                f.look.seed = (now * FLICKER_RATE as f64).floor() as f32 * 0.137 % 1.0;
                a.0 = live * flicker(now, seed);
            }
            SpecialKind::Arc => {
                let h = at_least_px(FLARE * hw, FLARE_MIN_PX, dist);
                *tf = quad::billboard(eye, weld, Vec2::splat(h));
                // The rays turn to a new angle with each flicker.
                f.look.p4.x = (now * FLICKER_RATE as f64).floor() as f32 * 2.399;
                a.0 = live * flicker(now, seed);
            }
            SpecialKind::Hot => {
                let h = at_least_px(FLARE * hw, FLARE_MIN_PX, dist) * 0.5;
                *tf = quad::billboard(eye, weld, Vec2::splat(h));
                a.0 = live;
            }
            SpecialKind::Spark => {
                // Each spark runs its own cycle, staggered, from the weld: a new
                // direction every time round.
                let t = now as f32 / SPARK_LIFE + seed as f32 / SPARKS as f32;
                let n = t.floor();
                let cycle = n as Tick;
                let at = (t - n) * SPARK_LIFE;
                let out = (weld - f.to).normalize_or(Vec3::Y);
                let r = |s: u8| rand(f.subject, cycle, seed, s);
                let (ya, aa) = (r(0) * 2.0 - 1.0, r(1) * std::f32::consts::TAU);
                let ring = (1.0 - ya * ya).sqrt();
                let dir = (out + Vec3::new(ring * aa.cos(), ya, ring * aa.sin()) * SPARK_SPREAD).normalize_or(out);
                let speed = SPARK_SPEED * f.radius * (0.3 + 0.7 * r(2));
                let w = at_least_px(SPARK_WIDTH * f.radius, SPARK_MIN_PX, dist);
                match spark_at(weld, dir, speed, at, SPARK_LIFE) {
                    Some((tail, head, kh)) if f.builder.is_some() => {
                        *tf = quad::ribbon(eye, tail, head, w, w);
                        f.look.col = heat_colour(kh);
                        f.look.len = (head - tail).length() / w;
                        a.0 = kh;
                    }
                    _ => {
                        *tf = Transform::from_translation(weld).with_scale(Vec3::ZERO);
                        a.0 = 0.0;
                    }
                }
            }
            SpecialKind::BuiltFlash => {
                *tf = quad::billboard(eye, f.to, Vec2::splat(f.radius * (0.3 + 0.7 * k)));
                a.0 = 1.0 - k;
            }
            SpecialKind::Shimmer => {
                *tf = quad::billboard(eye, f.to, Vec2::splat(f.radius));
                // Strongest mid-transition: vis (1 - vis), scaled to 1.
                a.0 = 4.0 * k * (1.0 - k);
            }
        }
    }
}

#[allow(clippy::type_complexity)]
/// Move each unit's cloak fade toward its target at a fixed rate.
fn fade_units(mut cmds: Commands, clock: Res<AnimClock>, mut last: Local<Option<f64>>, mut q: Query<(Entity, Has<Cloaked>, Option<&mut UnitFade>), (With<SimEntity>, Without<ManualFade>)>) {
    let now = clock.now();
    let dt = (now - last.unwrap_or(now)).max(0.0) as f32;
    *last = Some(now);
    let step = dt * TICK_HZ as f32 / CLOAK_TICKS * (1.0 - CLOAKED_ALPHA);
    for (e, cloaked, fade) in &mut q {
        let target = if cloaked { CLOAKED_ALPHA } else { 1.0 };
        match fade {
            // A unit seen already cloaked starts cloaked.
            None => {
                cmds.entity(e).insert(UnitFade { cloak: target, build: 1.0 });
            }
            Some(mut f) => {
                let next = if f.cloak < target { (f.cloak + step).min(target) } else { (f.cloak - step).max(target) };
                if next != f.cloak {
                    f.cloak = next;
                }
            }
        }
    }
}

fn attach_visuals(
    mut cmds: Commands,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mats: Option<ResMut<Assets<FxQuadMaterial>>>,
    mut quad_mesh: Local<Option<Handle<Mesh>>>,
    new: Query<(Entity, &SpecialFx), Added<SpecialFx>>,
) {
    let (Some(mut meshes), Some(mut mats)) = (meshes, mats) else { return };
    if new.is_empty() {
        return;
    }
    let mesh = quad_mesh.get_or_insert_with(|| meshes.add(quad::rectangle())).clone();
    for (e, f) in &new {
        cmds.entity(e).insert((Mesh3d(mesh.clone()), MeshMaterial3d(mats.add(quad::material(&f.look)))));
    }
}

fn apply_quads(clock: Res<AnimClock>, mats: Option<ResMut<Assets<FxQuadMaterial>>>, q: Query<(&SpecialFx, &FxAlpha, &MeshMaterial3d<FxQuadMaterial>)>) {
    let Some(mut mats) = mats else { return };
    let t = quad::shader_clock(clock.now());
    for (f, a, m) in &q {
        // `[Construction] Brightness` 1.2 for the arc and hot spot, `[Bees]`' 1.3 for the beam.
        let bright = match f.kind {
            SpecialKind::Weld => BEAM_BRIGHT * a.0,
            SpecialKind::Arc | SpecialKind::Hot => 1.2 * a.0,
            SpecialKind::Spark => SPARK_BRIGHT * a.0 * (0.5 + 0.5 * a.0),
            SpecialKind::BuiltFlash => a.0,
            SpecialKind::Shimmer => FRONT_BRIGHT * 0.5 * a.0,
        };
        quad::drive(&mut mats, &m.0, &f.look, bright, t);
    }
}

/// Write [`UnitFade`] into the unit's mesh materials.
fn apply_fade(
    mut cmds: Commands,
    mats: Option<ResMut<Assets<StandardMaterial>>>,
    roots: Query<(Entity, &UnitFade), Changed<UnitFade>>,
    children: Query<&Children>,
    mut meshes: Query<(&mut MeshMaterial3d<StandardMaterial>, Option<&FadeMat>)>,
) {
    let Some(mut mats) = mats else { return };
    for (root, fade) in &roots {
        let a = fade.alpha();
        for d in children.iter_descendants(root) {
            let Ok((mut m, saved)) = meshes.get_mut(d) else { continue };
            if a >= 1.0 {
                if let Some(s) = saved {
                    m.0 = s.orig.clone();
                    cmds.entity(d).remove::<FadeMat>();
                }
                continue;
            }
            if saved.is_none() {
                let Some(copy) = mats.get(&m.0).cloned() else { continue };
                let orig = std::mem::replace(&mut m.0, mats.add(copy));
                cmds.entity(d).insert(FadeMat { orig });
            }
            if let Some(mut mat) = mats.get_mut(&m.0) {
                mat.base_color = mat.base_color.with_alpha(a);
                mat.alpha_mode = AlphaMode::Blend;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::render::units::add_view_systems;
    use crate::sim_link::SimLinkPlugin;
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::command::LocalRouter;
    use sta_sim::view::UnitFlags;
    use sta_sim::fake::ScriptedSource;
    use sta_sim::{Fixed, PlayerId, TickOutput, WorldSource, WorldView};
    use std::sync::Arc;
    use std::time::Duration;

    /// A scripted source whose view also carries build progress (unit 3,
    /// ticks 30..120) and the cloak flag (unit 1, ticks 150..240), which the
    /// JSON timeline cannot say.
    struct Patched(ScriptedSource);

    fn patch(v: Arc<WorldView>) -> Arc<WorldView> {
        let mut v = (*v).clone();
        let t = v.tick;
        for u in &mut v.units {
            if u.id.0 == 3 && (30..120).contains(&t) {
                u.progress = Some(Fixed::from_int((t - 30) as i32) / Fixed::from_int(90));
            }
            if u.id.0 == 1 && (150..240).contains(&t) {
                u.flags = UnitFlags(u.flags.0 | UnitFlags::CLOAKED.0);
            }
        }
        Arc::new(v)
    }

    impl WorldSource for Patched {
        fn step(&mut self, c: &[sta_sim::command::Command]) -> TickOutput {
            let o = self.0.step(c);
            TickOutput { view: patch(o.view), events: o.events }
        }
        fn view(&self) -> Arc<WorldView> {
            patch(self.0.view())
        }
    }

    fn timeline(db: &sta_content::ContentDb) -> String {
        let what = db.key("fx_frigate").unwrap().0;
        format!(
            "{{ units: [
                {{ id: 1, def: \"fx_cruiser\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }},
                {{ id: 3, def: \"fx_frigate\", owner: 0, hull: 400, rails: [{{ at: 0, pos: {{ x: 100, y: 0, z: 200 }} }}] }},
            ], events: [
                {{ at: 29, event: {{ BuildStarted: {{ builder: 1, what: {what} }} }} }},
                {{ at: 120, event: {{ Built: {{ builder: 1, id: 3 }} }} }},
                {{ at: 150, event: {{ CloakChanged: {{ id: 1, on: true }} }} }},
                {{ at: 240, event: {{ CloakChanged: {{ id: 1, on: false }} }} }},
            ] }}"
        )
    }

    type Row = ((u32, SpecialKind, Tick), [u32; 3], [u32; 3], u32);
    /// Rows, then the cloak alpha of unit 1 and the build alpha of unit 3 (bits).
    type Frame = (Vec<Row>, u32, u32);

    fn run() -> Vec<Frame> {
        let db = sta_content::fake::db();
        let src = Patched(ScriptedSource::from_json5(&timeline(&db), &db).unwrap());
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 45.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(SimLink::new(Box::new(src), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_plugins(SpecialsFxPlugin);
        add_view_systems(&mut app);
        configure_sets(&mut app);
        let mut frames = Vec::new();
        for _ in 0..500 {
            app.update();
            let w = app.world_mut();
            let mut q = w.query::<(&SpecialFx, &Transform, &FxAlpha)>();
            let mut rows: Vec<Row> = q
                .iter(w)
                .map(|(f, t, a)| {
                    let b = |v: Vec3| v.to_array().map(f32::to_bits);
                    ((f.subject.0, f.kind, f.born), b(t.translation), b(t.scale), a.0.to_bits())
                })
                .collect();
            rows.sort_by_key(|r| r.0);
            let mut u = w.query::<(&SimEntity, &UnitFade)>();
            let mut fade = |id: u32, cloak: bool| {
                u.iter(w).find(|(s, _)| s.0.0 == id).map_or(f32::NAN, |(_, f)| if cloak { f.cloak } else { f.build }).to_bits()
            };
            let (c, s) = (fade(1, true), fade(3, false));
            frames.push((rows, c, s));
        }
        frames
    }

    #[test]
    fn specials_timeline_is_identical_across_runs_and_expires() {
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        let kinds = |k: SpecialKind| a.iter().flat_map(|f| &f.0).filter(|r| r.0.1 == k).map(|r| r.0.2).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(kinds(SpecialKind::Weld).len(), 1);
        assert_eq!(kinds(SpecialKind::Arc).len(), 1);
        assert_eq!(kinds(SpecialKind::Hot).len(), 1);
        assert_eq!(kinds(SpecialKind::Spark).len(), 1);
        // Refit's `Sparks`: three streaks to a station.
        let sparks = a.iter().map(|f| f.0.iter().filter(|r| r.0.1 == SpecialKind::Spark).count()).max().unwrap();
        assert_eq!(sparks, SPARKS as usize);
        assert_eq!(kinds(SpecialKind::BuiltFlash).len(), 1);
        assert_eq!(kinds(SpecialKind::Shimmer).len(), 2);
        assert!(a.last().unwrap().0.is_empty(), "FX left after the timeline");
        // Weld is alive only while building (90 ticks at 1.5 frames each).
        let weld_frames = a.iter().filter(|f| f.0.iter().any(|r| r.0.1 == SpecialKind::Weld)).count();
        assert!((129..=141).contains(&weld_frames), "{weld_frames}");

        // Cloak: down to CLOAKED_ALPHA, then back up, each leg monotonic.
        let cloak: Vec<f32> = a.iter().map(|f| f32::from_bits(f.1)).collect();
        let min = cloak.iter().cloned().fold(f32::MAX, f32::min);
        assert!((min - CLOAKED_ALPHA).abs() < 1e-5);
        assert_eq!(cloak[0], 1.0);
        assert_eq!(*cloak.last().unwrap(), 1.0);
        let lo = cloak.iter().position(|&x| x == min).unwrap();
        let hi = cloak.iter().rposition(|&x| x == min).unwrap();
        assert!(cloak[..=lo].windows(2).all(|w| w[1] <= w[0]));
        assert!(cloak[hi..].windows(2).all(|w| w[1] >= w[0]));
        assert!(cloak[lo..=hi].iter().all(|&x| x == min));

        // Station firms up with progress and ends opaque.
        let st: Vec<f32> = a.iter().map(|f| f32::from_bits(f.2)).filter(|x| !x.is_nan()).collect();
        assert!(st.iter().cloned().fold(f32::MAX, f32::min) < 0.2);
        let from = st.iter().position(|&x| x < 1.0).unwrap();
        assert!(st[from..].windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(*st.last().unwrap(), 1.0);
    }
}
