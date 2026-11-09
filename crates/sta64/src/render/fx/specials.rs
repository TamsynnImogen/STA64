//! Special FX (RND-11): construction, cloak, and the gaps for repair and warp.
//!
//! - **Construction**: while a unit's `UnitView::progress` is `Some`, a weld
//!   beam runs from the builder (bound from `BuildStarted`, matched to the
//!   station by content key) to the station, with a spark orbiting it. The
//!   station hull is drawn translucent and firms up with progress
//!   ([`UnitFade::build`]). `Built` flashes at the finished station.
//! - **Cloak**: [`UnitFade::cloak`] moves linearly between 1 and
//!   [`CLOAKED_ALPHA`] over [`CLOAK_TICKS`] whenever the `Cloaked` marker
//!   (from the view's flag) changes, and `CloakChanged` adds a shimmer flash.
//!   Enemy cloaked units never reach here: RND-02 drops them from the view.
//! - **Repair, warp**: the sim has no repair or warp event or view state
//!   (SIM-14 only has cloak), so there is nothing to draw. Gap, see the PR.
//!
//! [`UnitFade`] is applied to the unit's materials (cloned per mesh on first
//! use so shared model materials are untouched). All time is [`AnimClock`].

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_sim::{ContentKey, EntityId, SimEvent, TICK_HZ, Tick};

use super::weapons::FxAlpha;
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::units::{Cloaked, to_bevy};
use crate::sim_link::{SimEntity, SimLink, SimMsg};

/// Ticks a cloak or decloak takes.
pub const CLOAK_TICKS: f32 = 30.0;
/// Opacity of a cloaked unit its owner still sees.
pub const CLOAKED_ALPHA: f32 = 0.3;
/// Opacity of a station at zero progress.
pub const BUILD_MIN_ALPHA: f32 = 0.15;
/// Ticks the `Built` flash lasts.
pub const FLASH_TICKS: Tick = 12;
const WELD_WIDTH: f32 = 2.0;
const SPARK_RADIUS: f32 = 4.0;
const SPARK_ORBIT: f32 = 30.0;
const FLASH_RADIUS: f32 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpecialKind {
    /// Beam from builder to station.
    Weld,
    /// Spark orbiting the station.
    Spark,
    /// Completion flash.
    BuiltFlash,
    /// Cloak or decloak shimmer.
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
    from: Vec3,
    to: Vec3,
    colour: Color,
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

/// Original handle of a material this module replaced with a per-unit clone.
#[derive(Component)]
struct FadeMat {
    orig: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
struct Construction {
    /// `BuildStarted`s not yet bound to a station.
    pending: Vec<(EntityId, ContentKey)>,
    /// Station to its (weld, spark) entities.
    active: BTreeMap<EntityId, [Entity; 2]>,
}

pub struct SpecialsFxPlugin;

impl Plugin for SpecialsFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Construction>().add_systems(
            Update,
            (read_events, fade_units, sync_construction, update_fx, attach_visuals, apply_alpha, apply_fade)
                .chain()
                .in_set(GameSet::Interpolate)
                .run_if(resource_exists::<SimLink>),
        );
    }
}

fn point(link: &SimLink, id: EntityId, alpha: f32) -> Option<Vec3> {
    link.interpolate(id, alpha).map(|(p, _)| to_bevy(p))
}

fn fx(kind: SpecialKind, subject: EntityId, builder: Option<EntityId>, born: Tick, life: Option<f32>, at: Vec3, colour: Color) -> impl Bundle {
    (SpecialFx { kind, subject, builder, born, life, from: at, to: at, colour }, FxAlpha(0.0), Transform::from_translation(at))
}

fn read_events(mut cmds: Commands, mut msgs: MessageReader<SimMsg>, clock: Res<AnimClock>, link: Res<SimLink>, mut cons: ResMut<Construction>) {
    for SimMsg(ev) in msgs.read() {
        match ev {
            SimEvent::BuildStarted { builder, what } => cons.pending.push((*builder, *what)),
            SimEvent::Built { id, .. } => {
                if let Some(at) = point(&link, *id, clock.alpha()) {
                    cmds.spawn(fx(SpecialKind::BuiltFlash, *id, None, clock.tick(), Some(FLASH_TICKS as f32), at, Color::srgb(0.6, 0.9, 1.0)));
                }
            }
            SimEvent::CloakChanged { id, on } => {
                if let Some(at) = point(&link, *id, clock.alpha()) {
                    let c = if *on { Color::srgb(0.5, 0.7, 1.0) } else { Color::srgb(0.8, 0.9, 1.0) };
                    cmds.spawn(fx(SpecialKind::Shimmer, *id, None, clock.tick(), Some(CLOAK_TICKS), at, c));
                }
            }
            _ => {}
        }
    }
}

/// Weld and spark for every station with progress; the station's build fade.
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
        let weld = cmds.spawn(fx(SpecialKind::Weld, id, builder, clock.tick(), None, at, Color::srgb(1.0, 0.8, 0.3))).id();
        let spark = cmds.spawn(fx(SpecialKind::Spark, id, builder, clock.tick(), None, at, Color::srgb(1.0, 0.95, 0.7))).id();
        cons.active.insert(id, [weld, spark]);
    }
    for (e, mut f) in &mut fades {
        let build = building.get(&e.0).map_or(1.0, |(p, _)| BUILD_MIN_ALPHA + (1.0 - BUILD_MIN_ALPHA) * p);
        if f.build != build {
            f.build = build;
        }
    }
}

fn update_fx(mut cmds: Commands, clock: Res<AnimClock>, link: Res<SimLink>, mut q: Query<(Entity, &mut SpecialFx, &mut Transform, &mut FxAlpha)>) {
    let now = clock.now();
    let now_ticks = now * TICK_HZ as f64;
    let alpha = clock.alpha();
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
        match f.kind {
            SpecialKind::Weld => {
                let (from, to) = if f.builder.is_some() { (f.from, f.to) } else { (f.to, f.to) };
                let d = to - from;
                let len = d.length();
                let dir = if len > 1e-4 { d / len } else { Vec3::Z };
                *tf = Transform::from_translation(from.midpoint(to)).looking_to(dir, Vec3::Y).with_scale(Vec3::new(WELD_WIDTH, WELD_WIDTH, len.max(1e-3)));
                // Flicker, deterministic in anim time.
                a.0 = if f.builder.is_some() { 0.6 + 0.4 * (now as f32 * 40.0).sin().abs() } else { 0.0 };
            }
            SpecialKind::Spark => {
                let ang = now as f32 * 9.0;
                let off = Vec3::new(ang.cos(), 0.3 * (ang * 1.7).sin(), ang.sin()) * SPARK_ORBIT;
                *tf = Transform::from_translation(f.to + off).with_scale(Vec3::splat(SPARK_RADIUS));
                a.0 = 0.7 + 0.3 * (now as f32 * 55.0).sin().abs();
            }
            SpecialKind::BuiltFlash => {
                *tf = Transform::from_translation(f.to).with_scale(Vec3::splat(FLASH_RADIUS * (0.3 + 0.7 * k)));
                a.0 = 1.0 - k;
            }
            SpecialKind::Shimmer => {
                *tf = Transform::from_translation(f.to).with_scale(Vec3::splat(FLASH_RADIUS * (0.5 + 0.5 * k)));
                a.0 = 0.6 * (1.0 - k);
            }
        }
    }
}

#[allow(clippy::type_complexity)]
/// Move each unit's cloak fade toward its target at a fixed rate.
fn fade_units(mut cmds: Commands, clock: Res<AnimClock>, mut last: Local<Option<f64>>, mut q: Query<(Entity, Has<Cloaked>, Option<&mut UnitFade>), With<SimEntity>>) {
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
    mats: Option<ResMut<Assets<StandardMaterial>>>,
    new: Query<(Entity, &SpecialFx), Added<SpecialFx>>,
) {
    let (Some(mut meshes), Some(mut mats)) = (meshes, mats) else { return };
    if new.is_empty() {
        return;
    }
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let ball = meshes.add(Sphere::new(1.0));
    for (e, f) in &new {
        let mesh = if f.kind == SpecialKind::Weld { cube.clone() } else { ball.clone() };
        let material = mats.add(StandardMaterial { base_color: f.colour, emissive: f.colour.to_linear() * 4.0, unlit: true, alpha_mode: AlphaMode::Add, ..default() });
        cmds.entity(e).insert((Mesh3d(mesh), MeshMaterial3d(material)));
    }
}

fn apply_alpha(mats: Option<ResMut<Assets<StandardMaterial>>>, q: Query<(&SpecialFx, &FxAlpha, &MeshMaterial3d<StandardMaterial>), Changed<FxAlpha>>) {
    let Some(mut mats) = mats else { return };
    for (f, a, m) in &q {
        if let Some(mut mat) = mats.get_mut(&m.0) {
            mat.base_color = f.colour.with_alpha(a.0);
            mat.emissive = f.colour.to_linear() * (4.0 * a.0);
        }
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
        assert_eq!(kinds(SpecialKind::Spark).len(), 1);
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
