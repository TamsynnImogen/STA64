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
//! - **Construction**: while a unit's `UnitView::progress` is `Some`, bees wait at
//!   their constructor hardpoints, travel to the next unrevealed chunk edge,
//!   weld there, and return home when the job ends. The beam, arc, hot spot and
//!   sparks share that edge target. A named sprite is used where available;
//!   procedural Refit quads remain the fallback. `construction` reveals opaque
//!   chunks; `Built` flashes at the finished station (STA64's own).
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

use std::collections::{BTreeMap, BTreeSet};

use std::sync::Arc;

use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use bevy::render::renderer::RenderDevice;
use sta_assets::model::Model;
use sta_sim::{ContentKey, EntityId, SimEvent, TICK_HZ, Tick};

use super::bees::{BeeMotion, BeePhase, BeeSpec, bee_spec};
use super::impacts::{heat_colour, rand, spark_at};
use super::quad::{self, FxQuadMaterial, Look};
use super::weapons::FxAlpha;
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::model_scene::{self, PartCache, SpawnCtx};
use crate::render::sprites;
use crate::render::sprites::SpriteTables;
use crate::render::textures::{Loader, TextureCache};
use crate::render::units::{Cloaked, SelectRadius, TeamColour, to_bevy};
use crate::sim_link::{ContentRes, SimEntities, SimEntity, SimLink, SimMsg};
use crate::viewer::Mounted;

/// Ticks a cloak or decloak takes: Refit's `CloakTime`, 2.5 s.
pub const CLOAK_TICKS: f32 = 2.5 * TICK_HZ as f32;
/// Opacity of a cloaked unit its owner still sees: Refit's `CloakHull`, 10%.
pub const CLOAKED_ALPHA: f32 = 0.1;
/// Ticks the `Built` flash lasts.
pub const FLASH_TICKS: Tick = 12;
/// Ticks the bees and beams take to fade out when the station is done.
pub const BEE_FADE_TICKS: f32 = 18.0;
/// Size of a worker bee's model, times its SOD's own.
const BEE_SCALE: f32 = 1.0;
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
    /// A worker bee, circling the station (its model hangs off this entity).
    Drone,
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
    /// Which worker bee this is (or whose spot a spark flies from); `None`: the
    /// builder's own single beam, for content that names no bees.
    pub bee: Option<u8>,
    /// Bees at the site.
    pub bees: u8,
    /// The tick its station was finished: it fades out over [`BEE_FADE_TICKS`] and goes.
    ended: Option<Tick>,
    /// The beam's width against the one the look is tuned for.
    pub beam_scale: f32,
    from: Vec3,
    to: Vec3,
    radius: f32,
    look: Look,
    sprite: Option<String>,
    textured: bool,
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
    /// Exact adjacent BuildStarted/Spawned rig bindings.
    bindings: BTreeMap<EntityId, EntityId>,
    /// Station to its effect entities.
    active: BTreeMap<EntityId, Site>,
}

struct Site {
    effects: Vec<Entity>,
    builder: Option<EntityId>,
    spec: Option<BeeSpec>,
    bees: Vec<BeeMotion>,
    returning: bool,
    closure_started: Option<f64>,
    missing_hardpoints: BTreeSet<u8>,
}

/// Constructors whose production bay stays open for working or returning bees.
#[derive(Resource, Default)]
pub struct ConstructionBays {
    pub open: BTreeSet<EntityId>,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConstructionSet;

pub struct SpecialsFxPlugin;

impl Plugin for SpecialsFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Construction>()
            .init_resource::<ConstructionBays>()
            .add_systems(
                Update,
                (
                    read_events,
                    fade_units,
                    sync_construction,
                    update_fx,
                    attach_visuals,
                    attach_bee_models
                        .run_if(resource_exists::<Mounted>)
                        .run_if(resource_exists::<ContentRes>),
                    apply_quads,
                    drive_construction_sprites,
                    apply_fade,
                )
                    .chain()
                    .in_set(GameSet::Interpolate)
                    .in_set(ConstructionSet)
                    .distributive_run_if(resource_exists::<SimLink>),
            );
    }
}

fn point(link: &SimLink, id: EntityId, alpha: f32) -> Option<Vec3> {
    link.interpolate(id, alpha).map(|(p, _)| to_bevy(p))
}

#[allow(clippy::too_many_arguments)]
fn bee_home(
    builder: EntityId,
    bee: u8,
    spec: &BeeSpec,
    link: &SimLink,
    alpha: f32,
    ents: Option<&SimEntities>,
    children: &Query<&Children>,
    named: &Query<(&Name, &GlobalTransform)>,
) -> Option<(Vec3, bool)> {
    let origin = point(link, builder, alpha)?;
    let Some(want) = spec.hardpoints.get(bee as usize) else {
        return Some((origin, false));
    };
    let found = ents.and_then(|m| m.0.get(&builder)).and_then(|root| {
        children.iter_descendants(*root).find_map(|entity| {
            let (name, transform) = named.get(entity).ok()?;
            name.as_str()
                .eq_ignore_ascii_case(want)
                .then(|| transform.translation())
        })
    });
    Some(found.map_or((origin, false), |at| (at, true)))
}

#[allow(clippy::too_many_arguments)]
fn fx(
    kind: SpecialKind,
    subject: EntityId,
    builder: Option<EntityId>,
    born: Tick,
    life: Option<f32>,
    idx: u8,
    at: Vec3,
    look: Look,
) -> (SpecialFx, FxAlpha, Transform) {
    (
        SpecialFx {
            kind,
            subject,
            builder,
            born,
            life,
            idx,
            bee: None,
            bees: 0,
            ended: None,
            beam_scale: 1.0,
            from: at,
            to: at,
            radius: DEFAULT_RADIUS,
            look,
            sprite: None,
            textured: false,
        },
        FxAlpha(0.0),
        Transform::from_translation(at),
    )
}

/// A construction effect that belongs to one of the site's worker bees.
#[allow(clippy::too_many_arguments)]
fn bee_fx(
    kind: SpecialKind,
    subject: EntityId,
    builder: Option<EntityId>,
    born: Tick,
    idx: u8,
    bee: u8,
    spec: &BeeSpec,
    at: Vec3,
) -> impl Bundle {
    let (mut f, a, t) = fx(kind, subject, builder, born, None, idx, at, look_for(kind));
    f.bee = Some(bee);
    f.bees = spec.count;
    f.beam_scale = spec.width_scale();
    if kind == SpecialKind::Weld {
        f.sprite.clone_from(&spec.sprite);
    }
    if kind == SpecialKind::Arc {
        f.sprite = Some("xspark".into());
    }
    (f, a, t, Visibility::default())
}

/// The model of a worker bee, built once the entity has its place.
#[derive(Component)]
struct BeeModel(String);

#[derive(Component)]
struct ConstructionSprite {
    name: String,
    material: Handle<StandardMaterial>,
    parent_fx: Entity,
}

fn look_for(kind: SpecialKind) -> Look {
    match kind {
        SpecialKind::Weld => Look::new(quad::K_WELD_BEAM, BEAM_COLOUR),
        SpecialKind::Arc => Look { p3: ARC_FLARE, ..Look::new(quad::K_ARC_FLARE, ARC_COLOUR) },
        SpecialKind::Hot => Look::new(quad::K_HOT, HOT_COLOUR),
        SpecialKind::Spark => Look::new(quad::K_SPARK, Vec3::ONE),
        SpecialKind::BuiltFlash => Look::flare(Vec3::new(0.6, 0.9, 1.0), 30.0, 3.5, 0.4, 18.0, 0.0, 20.0, 0.0),
        SpecialKind::Shimmer => Look { p3: Vec4::new(FRONT_BANDS, FRONT_SPEED, FRONT_TILT, 0.0), ..Look::new(quad::K_CLOAK, FRONT_COLOUR) },
        SpecialKind::Drone => Look::new(quad::K_FLARE, Vec3::ONE),
    }
}

fn read_events(
    mut cmds: Commands,
    mut msgs: MessageReader<SimMsg>,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    mut cons: ResMut<Construction>,
) {
    let mut immediately_started = None;
    for SimMsg(ev) in msgs.read() {
        match ev {
            SimEvent::BuildStarted { builder, what } => {
                cons.pending.push((*builder, *what));
                immediately_started = Some((*builder, *what));
            }
            SimEvent::Spawned { id } => {
                if let Some((builder, what)) = immediately_started.take() {
                    let station = link.curr().units.iter().find(|u| u.id == *id);
                    let constructor = link.curr().units.iter().find(|u| u.id == builder);
                    if station.zip(constructor).is_some_and(|(s, b)| {
                        s.progress.is_some() && s.def == what && s.owner == b.owner
                    }) {
                        cons.bindings.insert(*id, builder);
                        if let Some(i) = cons
                            .pending
                            .iter()
                            .rposition(|&(b, w)| b == builder && w == what)
                        {
                            cons.pending.remove(i);
                        }
                    }
                }
            }
            SimEvent::Built { builder, id } => {
                immediately_started = None;
                cons.pending.retain(|(b, _)| b != builder);
                if let Some(at) = point(&link, *id, clock.alpha()) {
                    cmds.spawn(fx(
                        SpecialKind::BuiltFlash,
                        *id,
                        None,
                        clock.tick(),
                        Some(FLASH_TICKS as f32),
                        0,
                        at,
                        look_for(SpecialKind::BuiltFlash),
                    ));
                }
            }
            SimEvent::CloakChanged { id, .. } => {
                immediately_started = None;
                if let Some(at) = point(&link, *id, clock.alpha()) {
                    cmds.spawn(fx(
                        SpecialKind::Shimmer,
                        *id,
                        None,
                        clock.tick(),
                        Some(CLOAK_TICKS),
                        0,
                        at,
                        look_for(SpecialKind::Shimmer),
                    ));
                }
            }
            _ => {
                immediately_started = None;
            }
        }
    }
}

/// The beam, arc, hot spot and sparks for every station with progress.
#[allow(clippy::too_many_arguments)]
fn sync_construction(
    mut cmds: Commands,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    mut cons: ResMut<Construction>,
    mut bays: ResMut<ConstructionBays>,
    content: Option<Res<ContentRes>>,
    edges: Option<Res<crate::render::construction::ConstructionEdges>>,
    ents: Option<Res<SimEntities>>,
    children: Query<&Children>,
    named: Query<(&Name, &GlobalTransform)>,
    mut fades: Query<(&SimEntity, &mut UnitFade)>,
    mut fxs: Query<&mut SpecialFx>,
) {
    let curr = link.curr().clone();
    cons.pending.retain(|(builder, _)| curr.units.iter().any(|u| u.id == *builder));
    let building: BTreeMap<EntityId, (ContentKey, sta_sim::PlayerId)> = curr
        .units
        .iter()
        .filter_map(|u| u.progress.map(|_| (u.id, (u.def, u.owner))))
        .collect();
    // Legacy/no-ODF effects keep their short fade; bees fly home first.
    let gone: Vec<EntityId> = cons
        .active
        .iter()
        .filter(|(id, site)| !building.contains_key(id) && site.bees.is_empty())
        .map(|(id, _)| *id)
        .collect();
    for id in gone {
        let site = cons.active.remove(&id).unwrap();
        for e in site.effects {
            match fxs.get_mut(e) {
                Ok(mut f) => f.ended = Some(clock.tick()),
                Err(_) => cmds.entity(e).try_despawn(),
            }
        }
    }
    for (&id, &(def, owner)) in &building {
        if cons.active.contains_key(&id) {
            continue;
        }
        // A BuildStarted has no station id. Match by definition and owner, then
        // nearest surviving builder; EntityId is the stable tie-break.
        let exact = cons.bindings.remove(&id);
        let slot = if exact.is_some() {
            None
        } else {
            cons.pending
                .iter()
                .enumerate()
                .filter_map(|(i, &(builder, what))| {
                    if what != def
                        || curr
                            .units
                            .iter()
                            .find(|u| u.id == builder)
                            .is_none_or(|u| u.owner != owner)
                    {
                        return None;
                    }
                    Some((
                        i,
                        builder,
                        point(&link, builder, clock.alpha())
                            .unwrap_or(Vec3::ZERO)
                            .distance_squared(
                                point(&link, id, clock.alpha()).unwrap_or(Vec3::ZERO),
                            ),
                    ))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2).then_with(|| a.1.cmp(&b.1)))
                .map(|v| v.0)
        };
        let builder = exact.or_else(|| slot.map(|i| cons.pending.remove(i).0));
        if let Some(builder) = builder {
            let old: Vec<EntityId> = cons
                .active
                .iter()
                .filter(|(old_id, site)| {
                    site.builder == Some(builder)
                        && (site.returning || !building.contains_key(old_id))
                })
                .map(|(id, _)| *id)
                .collect();
            for old_id in old {
                if let Some(site) = cons.active.remove(&old_id) {
                    for entity in site.effects {
                        cmds.entity(entity).try_despawn();
                    }
                }
            }
        }
        let at = point(&link, id, clock.alpha()).unwrap_or(Vec3::ZERO);
        let mut spawned = Vec::new();
        // The builder's ODF says how many bees there are and what they look like.
        let spec = builder
            .and_then(|b| curr.units.iter().find(|u| u.id == b))
            .and_then(|b| content.as_ref()?.0.unit(b.def))
            .and_then(|d| bee_spec(&d.raw));
        if let Some(spec) = spec.as_ref().filter(|s| s.count > 0) {
            for bee in 0..spec.count {
                for kind in [SpecialKind::Weld, SpecialKind::Arc, SpecialKind::Hot] {
                    spawned.push(
                        cmds.spawn(bee_fx(kind, id, builder, clock.tick(), 0, bee, spec, at))
                            .id(),
                    );
                }
                if let Some(model) = &spec.model {
                    spawned.push(
                        cmds.spawn((
                            bee_fx(
                                SpecialKind::Drone,
                                id,
                                builder,
                                clock.tick(),
                                0,
                                bee,
                                spec,
                                at,
                            ),
                            BeeModel(model.clone()),
                        ))
                        .id(),
                    );
                }
            }
            for i in 0..SPARKS {
                spawned.push(
                    cmds.spawn(bee_fx(
                        SpecialKind::Spark,
                        id,
                        builder,
                        clock.tick(),
                        i,
                        i % spec.count,
                        spec,
                        at,
                    ))
                    .id(),
                );
            }
        } else if spec.is_none() {
            for kind in [SpecialKind::Weld, SpecialKind::Arc, SpecialKind::Hot] {
                spawned.push(
                    cmds.spawn(fx(
                        kind,
                        id,
                        builder,
                        clock.tick(),
                        None,
                        0,
                        at,
                        look_for(kind),
                    ))
                    .id(),
                );
            }
            for i in 0..SPARKS {
                spawned.push(
                    cmds.spawn(fx(
                        SpecialKind::Spark,
                        id,
                        builder,
                        clock.tick(),
                        None,
                        i,
                        at,
                        look_for(SpecialKind::Spark),
                    ))
                    .id(),
                );
            }
        }
        let bees = spec.as_ref().map_or_else(Vec::new, |s| {
            (0..s.count)
                .map(|bee| {
                    let home = builder
                        .and_then(|builder| {
                            bee_home(
                                builder,
                                bee,
                                s,
                                &link,
                                clock.alpha(),
                                ents.as_deref(),
                                &children,
                                &named,
                            )
                        })
                        .map_or(at, |(home, _)| home);
                    BeeMotion::new(clock.now(), home)
                })
                .collect()
        });
        cons.active.insert(
            id,
            Site {
                effects: spawned,
                builder,
                spec,
                bees,
                returning: false,
                closure_started: None,
                missing_hardpoints: BTreeSet::new(),
            },
        );
    }
    bays.open.clear();
    let mut finished = Vec::new();
    for (&id, site) in &mut cons.active {
        if !building.contains_key(&id) {
            site.returning = true;
        }
        if point(&link, id, clock.alpha()).is_none() {
            site.returning = true;
        }
        let targets = edges
            .as_ref()
            .and_then(|e| e.sites.get(&id))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if let (Some(builder), Some(spec)) = (site.builder, site.spec.as_ref()) {
            if point(&link, builder, clock.alpha()).is_none() {
                finished.push(id);
                continue;
            }
            for (i, bee) in site.bees.iter_mut().enumerate() {
                let (home, found) = bee_home(
                    builder,
                    i as u8,
                    spec,
                    &link,
                    clock.alpha(),
                    ents.as_deref(),
                    &children,
                    &named,
                )
                .unwrap();
                if !found && !spec.hardpoints.is_empty() && site.missing_hardpoints.insert(i as u8)
                {
                    warn!(
                        "constructor {builder:?}: worker bee {i} hardpoint {:?} missing; using origin",
                        spec.hardpoints.get(i)
                    );
                }
                bee.update(clock.now(), home, targets, i as u8, spec, site.returning);
            }
            if site.returning && site.bees.iter().all(|b| b.phase == BeePhase::Closing) {
                let since = *site.closure_started.get_or_insert(clock.now());
                if clock.now() >= since + f64::from(spec.closure) {
                    for bee in &mut site.bees {
                        bee.phase = BeePhase::Done;
                    }
                }
            }
            if site.bees.iter().any(|b| {
                matches!(
                    b.phase,
                    BeePhase::Waiting
                        | BeePhase::Traveling
                        | BeePhase::Welding
                        | BeePhase::Returning
                )
            }) {
                bays.open.insert(builder);
            }
        }
        if site.returning && site.bees.iter().all(|b| b.phase == BeePhase::Done) {
            finished.push(id);
        }
    }
    for id in finished {
        if let Some(site) = cons.active.remove(&id) {
            for e in site.effects {
                cmds.entity(e).try_despawn();
            }
        }
    }
    // Partial stations are cut by geometry; no global transparency ramp.
    for (_, mut fade) in &mut fades {
        if fade.build != 1.0 {
            fade.build = 1.0;
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

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_fx(
    mut cmds: Commands,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    cons: Res<Construction>,
    edges: Option<Res<crate::render::construction::ConstructionEdges>>,
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
        let gone = match f.ended {
            Some(t) => ((now_ticks - t as f64) as f32 / BEE_FADE_TICKS).clamp(0.0, 1.0),
            None => 0.0,
        };
        if gone >= 1.0 {
            cmds.entity(e).try_despawn();
            continue;
        }
        let k = match f.life {
            Some(life) if age >= life => {
                cmds.entity(e).try_despawn();
                continue;
            }
            Some(life) => (age / life).clamp(0.0, 1.0),
            None => 0.0,
        };
        if let Some(p) = f.builder.and_then(|b| point(&link, b, alpha)) {
            f.from = p;
        }
        let motion = f.bee.and_then(|bee| {
            cons.active
                .get(&f.subject)
                .and_then(|site| site.bees.get(bee as usize))
        });
        if let Some(subject) = point(&link, f.subject, alpha) {
            f.to = subject;
        } else if motion.is_none() {
            a.0 = 0.0;
            continue;
        }
        f.radius = ents
            .as_ref()
            .and_then(|m| m.0.get(&f.subject))
            .and_then(|e| radii.get(*e).ok())
            .map_or(DEFAULT_RADIUS, |r| r.0);
        let target = motion.map(|bee| bee.edge).or_else(|| {
            edges
                .as_ref()
                .and_then(|e| e.sites.get(&f.subject))
                .and_then(|edges| edges.first())
                .copied()
        });
        let weld = target.unwrap_or(f.to);
        // Construction effects that outlive the build fade out over their last ticks.
        let fade = match f.kind {
            SpecialKind::BuiltFlash | SpecialKind::Shimmer => 1.0,
            _ => 1.0 - gone,
        };
        let working = motion.is_some_and(|bee| bee.phase == BeePhase::Welding);
        let live = if f.builder.is_some() && target.is_some() && (f.bee.is_none() || working) {
            fade
        } else {
            0.0
        };
        let beam_live = if cons
            .active
            .get(&f.subject)
            .and_then(|site| site.spec.as_ref())
            .is_some_and(|spec| spec.width <= 0.0)
        {
            0.0
        } else {
            live
        };
        let dist = eye.map_or(100.0, |c| c.distance(weld));
        let authored_width = cons
            .active
            .get(&f.subject)
            .and_then(|site| site.spec.as_ref())
            .map(|s| s.width * 0.5);
        let hw = at_least_px(
            authored_width.unwrap_or(BEAM_WIDTH * 0.5 * f.beam_scale),
            BEAM_MIN_PX,
            dist,
        );
        let seed = f.idx;
        match f.kind {
            SpecialKind::Weld => {
                let from = match (motion, f.builder) {
                    (Some(bee), _) => bee.at,
                    (None, Some(_)) => f.from,
                    (None, None) => weld,
                };
                *tf = quad::ribbon(eye, from, weld, hw, 0.0);
                // The crackle's phase moves on with each flicker.
                f.look.seed = (now * FLICKER_RATE as f64).floor() as f32 * 0.137 % 1.0;
                a.0 = beam_live * flicker(now, seed);
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
                let dir = (out + Vec3::new(ring * aa.cos(), ya, ring * aa.sin()) * SPARK_SPREAD)
                    .normalize_or(out);
                let speed = SPARK_SPEED * f.radius * (0.3 + 0.7 * r(2));
                let w = at_least_px(SPARK_WIDTH * f.radius, SPARK_MIN_PX, dist);
                match spark_at(weld, dir, speed, at, SPARK_LIFE) {
                    Some((tail, head, kh)) if live > 0.0 => {
                        *tf = quad::ribbon(eye, tail, head, w, w);
                        f.look.col = heat_colour(kh);
                        f.look.len = (head - tail).length() / w;
                        a.0 = kh * fade;
                    }
                    _ => {
                        *tf = Transform::from_translation(weld).with_scale(Vec3::ZERO);
                        a.0 = 0.0;
                    }
                }
            }
            SpecialKind::Drone => {
                // Its model is a child; this entity is the bee's place and heading.
                if let Some(bee) = motion {
                    *tf = Transform::from_translation(bee.at)
                        .looking_to(bee.forward(), Vec3::Y)
                        .with_scale(Vec3::splat(BEE_SCALE * fade.max(0.001)));
                }
                a.0 = 1.0;
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
                cmds.entity(e).try_insert(UnitFade { cloak: target, build: 1.0 });
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

#[allow(clippy::too_many_arguments)]
fn attach_visuals(
    mut cmds: Commands,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mats: Option<ResMut<Assets<FxQuadMaterial>>>,
    standards: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
    texture_cache: Option<ResMut<TextureCache>>,
    tables: Option<Res<SpriteTables>>,
    mounted: Option<Res<Mounted>>,
    device: Option<Res<RenderDevice>>,
    mut quad_mesh: Local<Option<Handle<Mesh>>>,
    mut sprite_mesh: Local<Option<Handle<Mesh>>>,
    mut new: Query<(Entity, &mut SpecialFx), Added<SpecialFx>>,
) {
    let (Some(mut meshes), Some(mut mats)) = (meshes, mats) else {
        return;
    };
    let mut new = new
        .iter_mut()
        .filter(|(_, f)| f.kind != SpecialKind::Drone)
        .peekable();
    if new.peek().is_none() {
        return;
    }
    let mesh = quad_mesh
        .get_or_insert_with(|| meshes.add(quad::rectangle()))
        .clone();
    let mut sprite_loader = match (mounted, texture_cache, images) {
        (Some(mounted), Some(cache), Some(images)) => Some((mounted, cache, images)),
        _ => None,
    };
    let mut standards = standards;
    for (e, mut f) in new {
        cmds.entity(e).try_insert((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mats.add(quad::material(&f.look))),
        ));
        let Some(name) = f.sprite.as_ref() else {
            continue;
        };
        let (Some(table), Some((mounted, cache, images)), Some(standards)) =
            (tables.as_ref(), sprite_loader.as_mut(), standards.as_mut())
        else {
            continue;
        };
        let Some(sprite) = table.0.sprite(name) else {
            continue;
        };
        let formats = device
            .as_ref()
            .map(|d| CompressedImageFormats::from_features(d.features()))
            .unwrap_or(CompressedImageFormats::BC);
        let Some(texture) = (Loader {
            vfs: &mounted.0,
            cache,
            images,
            formats,
        })
        .colour(&sprite.texture, false) else {
            continue;
        };
        let material = standards.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(texture),
            uv_transform: sprites::uv_affine(sta_assets::spr_anim::sprite_uv(
                &table.0, sprite, 0.0,
            )),
            alpha_mode: sprites::alpha_mode(sta_assets::spr_anim::sprite_blend(&sprite.material)),
            unlit: true,
            double_sided: true,
            cull_mode: None::<Face>,
            ..default()
        });
        let smesh = sprite_mesh
            .get_or_insert_with(|| meshes.add(sprites::unit_quad()))
            .clone();
        cmds.spawn((
            Mesh3d(smesh),
            MeshMaterial3d(material.clone()),
            Transform::from_scale(Vec3::splat(2.0)),
            Visibility::default(),
            ChildOf(e),
            ConstructionSprite {
                name: name.clone(),
                material,
                parent_fx: e,
            },
        ));
        f.textured = true;
    }
}

fn drive_construction_sprites(
    clock: Res<AnimClock>,
    tables: Option<Res<SpriteTables>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    parents: Query<(&SpecialFx, &FxAlpha)>,
    mut sprites: Query<(&ConstructionSprite, &mut Visibility)>,
) {
    let (Some(tables), Some(mut materials)) = (tables, materials) else {
        return;
    };
    let now = clock.now() as f32;
    for (child, mut visibility) in &mut sprites {
        let Ok((fx, alpha)) = parents.get(child.parent_fx) else {
            continue;
        };
        if !fx.textured {
            continue;
        }
        let shown = alpha.0 > 0.0;
        *visibility = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let Some(sprite) = tables.0.sprite(&child.name) else {
            continue;
        };
        let uv = sprites::uv_affine(sta_assets::spr_anim::sprite_uv(&tables.0, sprite, now));
        let color = Color::srgba(1.0, 1.0, 1.0, alpha.0.clamp(0.0, 1.0));
        if let Some(mut material) = materials.get_mut(&child.material)
            && (material.uv_transform != uv || material.base_color != color)
        {
            material.uv_transform = uv;
            material.base_color = color;
        }
    }
}

fn apply_quads(
    clock: Res<AnimClock>,
    mats: Option<ResMut<Assets<FxQuadMaterial>>>,
    q: Query<(&SpecialFx, &FxAlpha, &MeshMaterial3d<FxQuadMaterial>)>,
) {
    let Some(mut mats) = mats else { return };
    let t = quad::shader_clock(clock.now());
    for (f, a, m) in &q {
        // `[Construction] Brightness` 1.2 for the arc and hot spot, `[Bees]`' 1.3 for the beam.
        let bright = match f.kind {
            SpecialKind::Weld => BEAM_BRIGHT * a.0,
            SpecialKind::Arc | SpecialKind::Hot => 1.2 * a.0,
            SpecialKind::Spark => SPARK_BRIGHT * a.0 * (0.5 + 0.5 * a.0),
            SpecialKind::BuiltFlash => a.0,
            SpecialKind::Drone => 0.0,
            SpecialKind::Shimmer => FRONT_BRIGHT * 0.5 * a.0,
        };
        quad::drive(
            &mut mats,
            &m.0,
            &f.look,
            if f.textured { 0.0 } else { bright },
            t,
        );
    }
}

/// Build the model of each worker bee in the owner's team colour (shared parts, so a
/// dozen bees cost one set of meshes).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn attach_bee_models(
    mut commands: Commands,
    pending: Query<(Entity, &BeeModel, &SpecialFx), Added<BeeModel>>,
    teams: Query<&TeamColour>,
    ents: Option<Res<SimEntities>>,
    mut known: Local<std::collections::HashMap<String, Option<(String, Arc<Model>)>>>,
    vfs: Res<Mounted>,
    tables: Res<SpriteTables>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: ResMut<TextureCache>,
    mut parts: ResMut<PartCache>,
    device: Option<Res<RenderDevice>>,
) {
    let formats = device.map(|d| CompressedImageFormats::from_features(d.features())).unwrap_or(CompressedImageFormats::BC);
    for (e, bee, f) in &pending {
        let key = bee.0.to_ascii_lowercase().trim_end_matches(".sod").to_owned();
        let found = known
            .entry(key.clone())
            .or_insert_with(|| {
                sta_assets::find_model(&vfs.0, &key).and_then(|(path, fmt)| sta_assets::load_model(&vfs.0, &path, fmt).ok().map(|m| (path, Arc::new(m))))
            })
            .clone();
        let Some((path, model)) = found else { continue };
        let team = ents.as_ref().and_then(|m| m.0.get(&f.subject)).and_then(|s| teams.get(*s).ok()).map_or(Color::WHITE, |t| t.0);
        let mut ctx = SpawnCtx {
            commands: &mut commands,
            meshes: &mut meshes,
            materials: &mut materials,
            textures: Loader { vfs: &vfs.0, cache: &mut cache, images: &mut images, formats },
            team,
            borg: false,
            sprites: &tables.0,
            damaged: false,
            normal_maps: true,
            parts: Some(&mut parts),
            model_key: path,
            unlit: false,
            scale: 1.0,
        };
        model_scene::spawn(&mut ctx, &model, e);
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
                    cmds.entity(d).try_remove::<FadeMat>();
                }
                continue;
            }
            if saved.is_none() {
                let Some(copy) = mats.get(&m.0).cloned() else { continue };
                let orig = std::mem::replace(&mut m.0, mats.add(copy));
                cmds.entity(d).try_insert(FadeMat { orig });
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
    use sta_sim::fake::ScriptedSource;
    use sta_sim::view::UnitFlags;
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
            TickOutput {
                view: patch(o.view),
                events: o.events,
            }
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

    struct BeeSource(ScriptedSource);

    fn bee_view(view: Arc<WorldView>) -> Arc<WorldView> {
        let mut view = (*view).clone();
        for unit in &mut view.units {
            let active = match unit.id.0 { 3 | 4 => (29..90).contains(&view.tick), 5 => (110..150).contains(&view.tick), _ => false };
            if active { unit.progress = Some(Fixed::from_int((view.tick % 100) as i32) / Fixed::from_int(100)); }
        }
        Arc::new(view)
    }

    impl WorldSource for BeeSource {
        fn step(&mut self, commands: &[sta_sim::command::Command]) -> TickOutput {
            let output = self.0.step(commands);
            TickOutput { view: bee_view(output.view), events: output.events }
        }
        fn view(&self) -> Arc<WorldView> { bee_view(self.0.view()) }
    }

    fn bee_app() -> App {
        let mut constructor = sta_content::UnitDef::new("test_constructor", "constructor");
        let raw = "numberOfWorkerBees = 1\nworkerBeeName = \"empty\"\nworkerBeeHardpoints = \"bot1\"\nweldingBeamSprite = \"weldbeam\"\ntimeUntilTakeoff = 1\ntimeWelding = 0.5\ntimeUntilClosure = 2\n";
        constructor.raw = Arc::new(sta_odf::resolve("test_constructor", &mut |_| Some(raw.into())).unwrap());
        let station = sta_content::UnitDef::new("test_station", "starbase");
        let db = sta_content::ContentDb::builder().unit(constructor).unit(station).build().unwrap();
        let what = db.key("test_station").unwrap().0;
        let script = format!("{{ units: [
            {{ id: 1, def: 'test_constructor', owner: 0, hull: 100, rails: [{{at:0,pos:{{x:0,y:0,z:0}}}}] }},
            {{ id: 2, def: 'test_constructor', owner: 0, hull: 100, rails: [{{at:0,pos:{{x:100,y:0,z:0}}}}] }},
            {{ id: 3, def: 'test_station', owner: 0, hull: 100, rails: [{{at:0,pos:{{x:100,y:0,z:0}}}}] }},
            {{ id: 4, def: 'test_station', owner: 0, hull: 100, rails: [{{at:0,pos:{{x:0,y:0,z:0}}}}] }},
            {{ id: 5, def: 'test_station', owner: 0, hull: 100, rails: [{{at:0,pos:{{x:30,y:0,z:0}}}}] }}
        ], events: [
            {{at:29,event:{{BuildStarted:{{builder:1,what:{what}}}}}}}, {{at:29,event:{{Spawned:{{id:3}}}}}},
            {{at:29,event:{{BuildStarted:{{builder:2,what:{what}}}}}}}, {{at:29,event:{{Spawned:{{id:4}}}}}},
            {{at:110,event:{{BuildStarted:{{builder:1,what:{what}}}}}}}, {{at:110,event:{{Spawned:{{id:5}}}}}}
        ] }}");
        let source = BeeSource(ScriptedSource::from_json5(&script, &db).unwrap());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 45.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(SimLink::new(Box::new(source), Box::new(LocalRouter::new()), PlayerId(0)))
            .insert_resource(ContentRes(Arc::new(db)))
            .init_resource::<crate::render::construction::ConstructionEdges>()
            .add_plugins(SpecialsFxPlugin);
        add_view_systems(&mut app);
        configure_sets(&mut app);
        for _ in 0..3 { app.update(); }
        for builder in [EntityId(1), EntityId(2)] {
            let root = app.world().resource::<SimEntities>().0[&builder];
            app.world_mut().spawn((Name::new("bot1"), Transform::from_xyz(10.0, 0.0, 0.0), ChildOf(root)));
        }
        let mut edges = app.world_mut().resource_mut::<crate::render::construction::ConstructionEdges>();
        edges.sites.insert(EntityId(3), vec![Vec3::new(30.0, 0.0, 0.0)]);
        edges.sites.insert(EntityId(4), vec![Vec3::new(130.0, 0.0, 0.0)]);
        edges.sites.insert(EntityId(5), vec![Vec3::new(30.0, 0.0, 0.0)]);
        app
    }

    fn to_tick(app: &mut App, tick: Tick) {
        for _ in 0..1000 {
            if app.world().resource::<SimLink>().curr().tick >= tick { return; }
            app.update();
        }
        panic!("script did not reach tick {tick}");
    }

    #[test]
    fn parallel_bees_use_exact_bindings_edges_and_return_before_closure() {
        let mut app = bee_app();
        to_tick(&mut app, 30);
        let sites = &app.world().resource::<Construction>().active;
        assert_eq!(sites[&EntityId(3)].builder, Some(EntityId(1)), "exact event beats nearer constructor");
        assert_eq!(sites[&EntityId(4)].builder, Some(EntityId(2)));
        assert!((sites[&EntityId(3)].bees[0].at.x - 10.0).abs() < 0.1, "bot1 transformed home");
        assert!((sites[&EntityId(4)].bees[0].at.x - 110.0).abs() < 0.1);
        assert_eq!(sites[&EntityId(3)].bees[0].phase, BeePhase::Waiting);
        assert!(app.world().resource::<ConstructionBays>().open.contains(&EntityId(1)));
        let mut welded = false;
        while app.world().resource::<SimLink>().curr().tick < 90 {
            app.update();
            let sites = &app.world().resource::<Construction>().active;
            if sites[&EntityId(3)].bees[0].phase == BeePhase::Welding {
                let world = app.world_mut();
                let mut query = world.query::<(&SpecialFx, &Transform, &FxAlpha)>();
                // The arc shares the current geometry edge, with the beam lit only while welding.
                assert!(query.iter(world).any(|(fx, transform, alpha)| fx.subject == EntityId(3)
                    && fx.kind == SpecialKind::Arc && transform.translation.distance(Vec3::new(30.0,0.0,0.0)) < 0.01 && alpha.0 > 0.0));
                welded = true;
                break;
            }
        }
        assert!(welded, "bee never welded at the published edge");
        to_tick(&mut app, 91);
        let sites = &app.world().resource::<Construction>().active;
        assert!(sites[&EntityId(3)].returning);
        assert!(sites[&EntityId(3)].bees[0].phase != BeePhase::Done);
        let world = app.world_mut();
        let mut query = world.query::<(&SpecialFx, &FxAlpha)>();
        assert!(query.iter(world).filter(|(fx, _)| fx.subject == EntityId(3) && fx.kind == SpecialKind::Weld)
            .all(|(_, alpha)| alpha.0 == 0.0), "return must extinguish beams");
        to_tick(&mut app, 105);
        let sites = &app.world().resource::<Construction>().active;
        assert_eq!(sites[&EntityId(3)].bees[0].phase, BeePhase::Closing);
        assert!(!app.world().resource::<ConstructionBays>().open.contains(&EntityId(1)));
        to_tick(&mut app, 111);
        let sites = &app.world().resource::<Construction>().active;
        assert!(!sites.contains_key(&EntityId(3)), "new job supersedes old closure");
        assert_eq!(sites[&EntityId(5)].builder, Some(EntityId(1)));
        assert_eq!(sites[&EntityId(5)].bees.len(), 1);
        to_tick(&mut app, 160);
        assert!(!app.world().resource::<Construction>().active.contains_key(&EntityId(4)), "closure expires");
    }

    #[test]
    fn specials_timeline_is_identical_across_runs_and_expires() {
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        let kinds = |k: SpecialKind| {
            a.iter()
                .flat_map(|f| &f.0)
                .filter(|r| r.0.1 == k)
                .map(|r| r.0.2)
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(kinds(SpecialKind::Weld).len(), 1);
        assert_eq!(kinds(SpecialKind::Arc).len(), 1);
        assert_eq!(kinds(SpecialKind::Hot).len(), 1);
        assert_eq!(kinds(SpecialKind::Spark).len(), 1);
        // Refit's `Sparks`: three streaks to a station.
        let sparks = a
            .iter()
            .map(|f| f.0.iter().filter(|r| r.0.1 == SpecialKind::Spark).count())
            .max()
            .unwrap();
        assert_eq!(sparks, SPARKS as usize);
        assert_eq!(kinds(SpecialKind::BuiltFlash).len(), 1);
        assert_eq!(kinds(SpecialKind::Shimmer).len(), 2);
        assert!(a.last().unwrap().0.is_empty(), "FX left after the timeline");
        // Weld is alive only while building (90 ticks at 1.5 frames each) and its fade-out
        // after (BEE_FADE_TICKS).
        let weld_frames = a
            .iter()
            .filter(|f| f.0.iter().any(|r| r.0.1 == SpecialKind::Weld))
            .count();
        let fade = (BEE_FADE_TICKS * 1.5) as usize;
        assert!(
            (129 + fade - 3..=141 + fade + 3).contains(&weld_frames),
            "{weld_frames}"
        );

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

        // Partial station triangles remain opaque while chunks are revealed.
        let st: Vec<f32> = a
            .iter()
            .map(|f| f32::from_bits(f.2))
            .filter(|x| !x.is_nan())
            .collect();
        assert!(!st.is_empty());
        assert!(st.iter().all(|&x| x == 1.0));
    }
}
