//! Ships in the map view: placement from the sim's views, and the selection
//! overlay.
//!
//! - The sim link steps the sim (`GameSet::SimLink`); each frame this places
//!   every [`SimEntity`] between the last two views at the animation clock's
//!   alpha (`GameSet::Interpolate`).
//! - [`sync_units`] diffs each new view against [`SimEntities`]: it spawns a
//!   root per new visible id (`SimEntity`, [`TeamColour`], [`HullShare`],
//!   [`Cloaked`] while cloaked, [`NeedsModel`] for units made from content),
//!   despawns the roots of ids that went (with all descendants), and
//!   updates the components that change. [`attach_models`] builds the model
//!   under a root once the game's files are mounted; [`show_damage`] shows
//!   the model's damage-state nodes by hull share.
//! - Selection rings, bars and target lines are `render/overlays.rs`.
//!
//! The sim works in the map's left-handed space; the renderer mirrors Z.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use sta_assets::model::Model;
use sta_sim::view::UnitFlags;
use sta_sim::{ContentKey, EntityId, PlayerId, WorldView};

use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::model_scene::{self, PartCache, SpawnCtx};
use crate::render::sprites::SpriteTables;
use crate::render::textures::{Loader, TextureCache};
use crate::sim_link::{ContentRes, SimEntities};
use crate::sim_link::{SimEntity, SimLink};
use crate::viewer::{Mounted, TeamColours};

mod diagnostics;

#[cfg(test)]
#[path = "units/defiant_tests.rs"]
mod defiant_tests;

/// Selection radius in world units, on the entity drawn for a sim unit.
#[derive(Component)]
pub struct SelectRadius(pub f32);

/// Whose eyes the map is drawn through. `None` shows every unit in the view;
/// `Some(p)` only what `WorldView::visible_to(p)` lists.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LocalPlayer(pub Option<PlayerId>);

/// The team colour the unit is drawn in, from its owner.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct TeamColour(pub Color);

/// Hull now over hull max, 0..1 (1 when the unit has no hull).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct HullShare(pub f32);

/// The unit is cloaked. RND-11 styles it; this only marks it.
#[derive(Component, Debug, Clone, Copy)]
pub struct Cloaked;

/// A unit root waiting for [`attach_models`] to build its model.
#[derive(Component, Debug, Clone, Copy)]
pub struct NeedsModel(pub ContentKey);

/// The unit's owner, as the view last said.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitOwner(pub PlayerId);

/// Marks roots made by [`sync_units`], so they are not adopted again.
#[derive(Component)]
pub struct FromView;

/// Hull share below which the damage-state node `name` shows, or `None` for
/// any other node. Breaches show first, then fires and venting (`crew`,
/// `life`), then the failing systems.
pub fn damage_threshold(name: &str) -> Option<f32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "damage" => 0.75,
        "crew" | "life" => 0.5,
        "shields" => 0.4,
        "engines" => 0.3,
        "sensors" => 0.25,
        "target" => 0.2,
        _ => return None,
    })
}

pub struct UnitsRenderPlugin;

impl Plugin for UnitsRenderPlugin {
    fn build(&self, app: &mut App) {
        add_view_systems(app);
        app.init_resource::<crate::render::construction::ConstructionEdges>()
            .init_resource::<crate::render::construction::ConstructionWork>()
            .add_systems(
                PostUpdate,
                crate::render::construction::sync_reveal
                    .after(TransformSystems::Propagate)
                    .run_if(resource_exists::<SimLink>),
            );
        app.add_systems(Last, diagnostics::trace);
    }
}

/// Spawn, despawn and place the units of the current view (everything but
/// the selection gizmos, which need a renderer).
pub(crate) fn add_view_systems(app: &mut App) {
    app.init_resource::<SimEntities>()
        .init_resource::<LocalPlayer>()
        .init_resource::<UnitModels>()
        .init_resource::<crate::render::moons::MoonVariants>()
        .add_systems(
            Update,
            (sync_units, attach_models.run_if(resource_exists::<Mounted>).run_if(resource_exists::<ContentRes>), show_damage)
                .chain()
                .before(place)
                .in_set(GameSet::Interpolate)
                .run_if(resource_exists::<SimLink>),
        )
        .add_systems(Update, place.in_set(GameSet::Interpolate).run_if(resource_exists::<SimLink>));
    crate::render::anim::add_systems(app);
}

/// Diff the current view against [`SimEntities`].
#[allow(clippy::too_many_arguments)]
pub fn sync_units(
    mut commands: Commands,
    link: Res<SimLink>,
    local: Res<LocalPlayer>,
    teams: Option<Res<TeamColours>>,
    mut map: ResMut<SimEntities>,
    existing: Query<(Entity, &SimEntity), Without<FromView>>,
    mut live: Query<(&mut HullShare, &mut UnitOwner, Has<Cloaked>), With<FromView>>,
    mut seen: Local<Option<(Arc<WorldView>, Option<PlayerId>)>>,
) {
    // Entities the map view spawned for its own ships belong to the link too.
    for (e, u) in &existing {
        map.0.entry(u.0).or_insert(e);
    }
    let curr = link.curr();
    if seen.as_ref().is_some_and(|(v, p)| Arc::ptr_eq(v, curr) && *p == local.0) {
        return;
    }
    *seen = Some((curr.clone(), local.0));

    let want: BTreeMap<EntityId, &sta_sim::UnitView> = match local.0 {
        Some(p) => curr.visible_to(p),
        None => curr.units.iter().collect(),
    }
    .into_iter()
    .map(|u| (u.id, u))
    .collect();

    // Gone: not in the view, or no longer visible.
    let gone: Vec<EntityId> = map.0.keys().filter(|id| !want.contains_key(id)).copied().collect();
    for id in gone {
        if let Some(e) = map.0.remove(&id) {
            commands.entity(e).try_despawn();
        }
    }
    let neutral = Color::WHITE;
    let colour = |owner: PlayerId| teams.as_ref().and_then(|t| t.0.get(owner.0 as usize)).map(|t| t.1).unwrap_or(neutral);
    let mut stale: BTreeSet<EntityId> = BTreeSet::new();
    for (id, u) in want {
        let share = hull_share(u);
        let cloaked = u.flags.contains(UnitFlags::CLOAKED);
        if let Some(&e) = map.0.get(&id) {
            if let Ok((mut h, mut o, was_cloaked)) = live.get_mut(e) {
                if h.0 != share {
                    h.0 = share;
                }
                if o.0 != u.owner {
                    o.0 = u.owner;
                    commands.entity(e).insert(TeamColour(colour(u.owner)));
                }
                if cloaked != was_cloaked {
                    if cloaked {
                        commands.entity(e).insert(Cloaked);
                    } else {
                        commands.entity(e).remove::<Cloaked>();
                    }
                }
            } else if commands.get_entity(e).is_err() {
                stale.insert(id);
            }
            continue;
        }
        let pos = to_bevy(u.pos.to_f32());
        let h = Vec2::new(u.heading.0.to_f32(), u.heading.1.to_f32());
        let rot = Transform::IDENTITY.looking_to(Vec3::new(h.x, 0.0, -h.y), Vec3::Y).rotation;
        let mut e = commands.spawn((
            Name::new(format!("unit {}", id.0)),
            FromView,
            SimEntity(id),
            UnitOwner(u.owner),
            TeamColour(colour(u.owner)),
            HullShare(share),
            Transform::from_translation(pos).with_rotation(rot),
            Visibility::default(),
        ));
        if cloaked {
            e.insert(Cloaked);
        }
        if u.def != ContentKey::NONE {
            e.insert(NeedsModel(u.def));
        }
        map.0.insert(id, e.id());
    }
    if !stale.is_empty() {
        for id in stale {
            map.0.remove(&id);
        }
        *seen = None;
    }
}

fn hull_share(u: &sta_sim::UnitView) -> f32 {
    let max = u.hull[1].to_f32();
    if max > 0.0 { (u.hull[0].to_f32() / max).clamp(0.0, 1.0) } else { 1.0 }
}

/// Models loaded for the units, by SOD name.
#[derive(Resource, Default)]
pub struct UnitModels {
    models: std::collections::HashMap<String, Option<(String, Arc<Model>)>>,
    appearance: std::collections::HashMap<ContentKey, Arc<crate::render::appearance::policy::AppearancePolicy>>,
}

/// Build the model of every root that [`NeedsModel`], in the owner's team
/// colour, with the damage-state nodes present (hidden by [`show_damage`]).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn attach_models(
    mut commands: Commands,
    pending: Query<(Entity, &NeedsModel, &TeamColour, Option<&UnitOwner>, Option<&SimEntity>,
    )>,
    mut moons: ResMut<crate::render::moons::MoonVariants>,
    mut silhouettes: Option<ResMut<crate::render::strategic::Silhouettes>>,
    vfs: Res<Mounted>,
    content: Res<ContentRes>,
    tables: Res<SpriteTables>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: ResMut<TextureCache>,
    mut parts: ResMut<PartCache>,
    mut models: ResMut<UnitModels>,
    link: Res<SimLink>,
    device: Option<Res<RenderDevice>>,
) {
    let formats = device.map(|d| CompressedImageFormats::from_features(d.features())).unwrap_or(CompressedImageFormats::BC);
    for (e, need, team, owner, sim) in &pending {
        commands.entity(e).remove::<NeedsModel>();
        let Some(def) = content.0.unit(need.0) else { continue;
        };
        // A planet's body is the map view's (`map/mod.rs` builds it from the map's object); a sim
        // planet with no map object (a bench scene's) has none, and its SOD sphere would be an
        // untextured dark ball hiding the map's planet drawn at the same place.
        if def.class.0.eq_ignore_ascii_case("planet") {
            continue;
        }
        // A resource moon's marker is drawn as the moon the game swaps it for (RND-16).
        let def = moons.moon_for(&vfs.0, &content.0, def, sim.map_or(0, |s| s.0.0 as u64)).unwrap_or(def);
        let key = def.model.to_ascii_lowercase().trim_end_matches(".sod").to_owned();
        let found = models
            .models
            .entry(key.clone())
            .or_insert_with(|| {
                sta_assets::find_model(&vfs.0, &key).and_then(|(path, fmt)| {
                    match sta_assets::load_model(&vfs.0, &path, fmt) {
                    Ok(m) => Some((path, Arc::new(m))),
                    Err(err) => {
                        warn!("{err}");
                        None
                        }
                    }
                })
            })
            .clone();
        let Some((path, model)) = found else {
            warn!("unit model unavailable: id={:?} odf={} model={key}", sim.map(|s| s.0), def.odf);
            continue;
        };
        let scale = def.scale.to_f32();
        // What the map view gives the ships it draws itself: a click radius from the
        // model's bounds, and for team-owned things a strategic icon of the right
        // shape (the mod's `<model>_sv` sprite, else a silhouette of the model). Units
        // that come into sight, or are placed at the start, are drawn here.
        let model_scale = if scale > 0.0 { scale } else { 1.0 };
        let bounds = model.bounds();
        if let Some((lo, hi)) = bounds {
            commands.entity(e).insert(crate::render::overlays::ModelBounds::from_legacy(lo, hi, model_scale));
        }
        let radius = bounds.map(|(lo, hi)| (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5).unwrap_or(20.0) * model_scale;
        let centre = bounds.map(|(lo, hi)| crate::render::strategic::centre_for_bounds(lo, hi, model_scale)).unwrap_or_default();
        let mobile = def.movement.max_speed > sta_sim::Fixed::ZERO;
        let owned = owner.is_some_and(|o| o.0 != sta_sim::world::NEUTRAL);
        // A resource (a moon) is neutral and never moves, but is clicked for its info and
        // as a mining target, so it gets a click radius too: of the moon the marker was swapped for.
        if mobile || owned || def.resource.is_some() {
            commands.entity(e).insert(SelectRadius(radius.max(5.0)));
        }
        let class = def.class.0.as_str();
        if owned && !matches!(class, "background_obj" | "latinum" | "asteroidfield" | "nebula_obj" | "planet" | "wormhole_obj") {
            let mut icon = crate::render::strategic::Strategic::new(crate::render::strategic::shape_for(class, mobile), team.0, radius,
            ).with_centre(centre);
            let sv = tables.0.sprite(&format!("{}_sv", def.model));
            let tex = sv.and_then(|sprite| {
                let tex = (Loader { vfs: &vfs.0, cache: &mut cache, images: &mut images, formats,
                }).colour(&sprite.texture, false)?;
                Some((tex, sta_assets::spr_anim::sprite_uv(&tables.0, sprite, 0.0)))
            });
            if let Some((tex, uv)) = tex {
                icon = icon.with_image(tex, Some(uv));
            } else if let Some(img) = silhouettes.as_mut().and_then(|s| s.get(&path, &model, &mut images)) {
                icon = icon.with_image(img, None);
            }
            commands.entity(e).insert(icon);
        }
        let mut ctx = SpawnCtx {
            commands: &mut commands,
            meshes: &mut meshes,
            materials: &mut materials,
            textures: Loader { vfs: &vfs.0, cache: &mut cache, images: &mut images, formats,
            },
            team: team.0,
            borg: false,
            sprites: &tables.0,
            damaged: true,
            normal_maps: true,
            parts: Some(&mut parts),
            model_key: path,
            unlit: false,
            scale: if scale > 0.0 { scale } else { 1.0 },
        };
        let progress = sim
            .and_then(|unit| link.curr().unit(unit.0))
            .and_then(|unit| unit.progress)
            .map(|progress| progress.to_f32().clamp(0.0, 1.0));
        let stats = if progress.is_some_and(|progress| progress < 1.0) {
            let chunks = construction_chunk_count(def);
            model_scene::spawn_construction(&mut ctx, &model, e, chunks, 0.0)
        } else {
            model_scene::spawn(&mut ctx, &model, e)
        };
        let policy = models.appearance.entry(need.0).or_insert_with(|| {
            crate::render::appearance::ModelInstance::new(need.0, model.clone(), model_scale, def).policy
        }).clone();
        commands.entity(e).insert(crate::render::appearance::ModelInstance { def: need.0, model: model.clone(), scale: model_scale, policy });
        debug!("unit model attached: id={:?} odf={} model={key} meshes={} triangles={} missing_textures={:?}",
            sim.map(|s| s.0), def.odf, stats.meshes, stats.triangles, stats.missing_textures);
    }
}

fn construction_chunk_count(def: &sta_content::UnitDef) -> u16 {
    const DEFAULT: u16 = 24;
    let Some(raw) = def.raw.raw("constructionChunks") else {
        return DEFAULT;
    };
    match def.raw.i64("constructionChunks") {
        Some(value) if (1..=256).contains(&value) => value as u16,
        Some(value) => {
            warn!(
                "{}: constructionChunks={raw} is outside 1..=256; clamping",
                def.odf
            );
            value.clamp(1, 256) as u16
        }
        None => {
            warn!(
                "{}: constructionChunks={raw} is invalid; using default {DEFAULT}",
                def.odf
            );
            DEFAULT
        }
    }
}

/// Show each damage-state node of a unit's model once its hull share is below
/// the node's threshold ([`damage_threshold`]).
#[allow(clippy::type_complexity)]
pub fn show_damage(
    units: Query<(Entity, &HullShare), Or<(Changed<HullShare>, Changed<Children>)>>,
    children: Query<&Children>,
    mut nodes: Query<(&Name, &mut Visibility), Without<model_scene::NativeNode>>,
) {
    for (unit, share) in &units {
        for d in children.iter_descendants(unit) {
            let Ok((name, mut vis)) = nodes.get_mut(d) else { continue };
            let Some(limit) = damage_threshold(name) else { continue };
            let want = if share.0 < limit { Visibility::Inherited } else { Visibility::Hidden };
            if *vis != want {
                *vis = want;
            }
        }
    }
}

pub fn to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], -p[2])
}

/// Place each unit between its last two ticks.
pub(crate) fn place(link: Res<SimLink>, clock: Res<AnimClock>, mut q: Query<(&SimEntity, &mut Transform)>) {
    let alpha = clock.alpha().clamp(0.0, 1.0);
    for (u, mut tf) in &mut q {
        let Some((pos, h)) = link.interpolate(u.0, alpha) else { continue };
        tf.translation = to_bevy(pos);
        // The model's front (+Z in the files) is Bevy's -Z after the mirror, which
        // is what `looking_to` points along the heading.
        tf.rotation = Transform::IDENTITY.looking_to(Vec3::new(h[0], 0.0, -h[1]), Vec3::Y).rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::sim_link::{CommandQueue, SimLinkPlugin};
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::command::{CommandKind, LocalRouter, Order};
    use sta_sim::{EntityId, Fixed, MoveStats, PlayerId, Sim, Vec3 as SimVec3};
    use std::time::Duration;

    /// A ship at the origin facing +Z, ordered to `to` through the command
    /// queue, drawn by `place` from the sim link on the animation clock.
    fn rig(to: [f32; 3]) -> (App, EntityId, Entity) {
        let mut sim = Sim::new(1);
        let stats = MoveStats::from_f32(140.0, 100.0, 1.0, 1.0, 1.0);
        let id = sim.spawn(1, SimVec3::ZERO, (Fixed::ZERO, Fixed::ONE), stats);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0,
            )))
            .add_plugins(SimLinkPlugin)
            .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId::BENCH,
            ))
            .add_systems(Update, place.in_set(GameSet::Interpolate));
        configure_sets(&mut app);
        app.world_mut().resource_mut::<CommandQueue>().0
            .push(CommandKind::Order(Order::Move {
            units: vec![id],
            to: SimVec3::from_f32(to),
            queue: false,
        }));
        let e = app.world_mut().spawn((SimEntity(id), Transform::IDENTITY)).id();
        (app, id, e)
    }

    /// A ship cruising in a straight line must move the same distance every
    /// rendered frame (sim at 30 Hz, frames at 60 Hz, interpolated between ticks).
    #[test]
    fn straight_line_motion_is_even_per_frame() {
        let (mut app, _, e) = rig([0.0, 0.0, 100_000.0]);
        let mut zs = Vec::new();
        for _ in 0..180 {
            app.update();
            zs.push(app.world().get::<Transform>(e).unwrap().translation.z);
        }
        // Skip the first second (acceleration), then compare frame steps.
        let steps: Vec<f32> = zs.windows(2).skip(60).map(|w| (w[1] - w[0]).abs()).collect();
        let (lo, hi) = steps.iter().fold((f32::MAX, 0f32), |(l, h), &s| (l.min(s), h.max(s)));
        let expect = 140.0 / 60.0;
        assert!(lo > expect * 0.8 && hi < expect * 1.2, "uneven frame steps: min {lo}, max {hi}, expected ~{expect}");
    }

    /// With uneven frame times the on-screen speed (step / frame time) must stay
    /// steady too.
    #[test]
    fn motion_follows_uneven_frames() {
        let (mut app, _, e) = rig([0.0, 0.0, 100_000.0]);
        let frames = [0.007, 0.023, 0.016, 0.031, 0.011, 0.019, 0.042, 0.009];
        let mut speeds = Vec::new();
        let mut last = 0.0f32;
        for i in 0..400 {
            let dt = frames[i % frames.len()];
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(dt)));
            app.update();
            let z = app.world().get::<Transform>(e).unwrap().translation.z;
            if i > 120 {
                speeds.push((z - last).abs() / dt as f32);
            }
            last = z;
        }
        let (lo, hi) = speeds.iter().fold((f32::MAX, 0f32), |(l, h), &s| (l.min(s), h.max(s)));
        assert!(lo > 140.0 * 0.85 && hi < 140.0 * 1.15, "on-screen speed varies: {lo}..{hi}");
    }

    /// A move order through the command queue is stepped by the sim link and
    /// drawn between the last two views, ending on the target (mirrored in Z).
    #[test]
    fn sim_link_drives_a_unit_to_its_target_with_interpolation() {
        let target = [30.0, 0.0, 200.0];
        let (mut app, id, e) = rig(target);
        let mut between = 0;
        let mut arrived_at = None;
        for frame in 0..600 {
            app.update();
            let link = app.world().resource::<SimLink>();
            let (prev, curr) = (link.prev().unit(id).unwrap().pos.to_f32(), link.curr().unit(id).unwrap().pos.to_f32());
            let alpha = app.world().resource::<AnimClock>().alpha();
            let at = app.world().get::<Transform>(e).unwrap().translation;
            // On the segment between the views, at alpha along it.
            let want = to_bevy(prev).lerp(to_bevy(curr), alpha);
            assert!(at.distance(want) < 1e-3, "frame {frame}: drawn {at}, expected {want}");
            if prev != curr && alpha > 0.1 && alpha < 0.9 {
                between += 1;
                assert!(at.distance(to_bevy(prev)) > 1e-4 && at.distance(to_bevy(curr)) > 1e-4, "frame {frame}: drawn on a view, not between");
            }
            if curr == target && arrived_at.is_none() {
                arrived_at = Some(frame);
            }
            if arrived_at.is_some_and(|f| frame > f + 6) {
                break;
            }
        }
        assert!(arrived_at.is_some(), "never reached the target");
        assert!(between > 10, "only {between} frames drawn between two views");
        assert_eq!(app.world().get::<Transform>(e).unwrap().translation, to_bevy(target));
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::sim_link::SimLinkPlugin;
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::command::{Command, LocalRouter};
    use sta_sim::fake::ScriptedSource;
    use sta_sim::view::{TickOutput, UnitView};
    use sta_sim::{Fixed, Vec3 as SimVec3, WorldSource};
    use std::time::Duration;

    fn app_with(source: Box<dyn WorldSource>, local: Option<PlayerId>) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 30.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(LocalPlayer(local))
            .insert_resource(TeamColours(vec![
                ("empty".into(), Color::WHITE),
                ("blue".into(), Color::srgb(0.0, 0.0, 1.0)),
                ("red".into(), Color::srgb(1.0, 0.0, 0.0)),
            ]))
            .insert_resource(SimLink::new(source, Box::new(LocalRouter::new()), PlayerId(1)));
        configure_sets(&mut app);
        add_view_systems(&mut app);
        app
    }

    fn scripted(text: &str) -> Box<dyn WorldSource> {
        let db = sta_content::ContentDb::builder().build().unwrap();
        Box::new(ScriptedSource::from_json5(text, &db).unwrap())
    }

    fn roots(app: &mut App) -> Vec<(EntityId, Entity)> {
        let mut q = app.world_mut().query::<(Entity, &SimEntity)>();
        q.iter(app.world()).map(|(e, s)| (s.0, e)).collect()
    }

    fn consistent(app: &mut App) {
        let live: BTreeSet<EntityId> = app.world().resource::<SimLink>().curr().units.iter().map(|u| u.id).collect();
        let map: BTreeSet<EntityId> = app.world().resource::<SimEntities>().0.keys().copied().collect();
        let ents: Vec<EntityId> = roots(app).into_iter().map(|r| r.0).collect();
        assert_eq!(map, live);
        assert_eq!(ents.len(), live.len(), "orphan or duplicate SimEntity entities");
        assert_eq!(ents.into_iter().collect::<BTreeSet<_>>(), live);
    }

    #[test]
    fn scripted_spawn_and_destroy() {
        let mut app = app_with(
            scripted(
                "{ units: [
                    { id: 1, owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
                    { id: 2, owner: 2, hull: 100, until: 10, rails: [{ at: 4, pos: { x: 5, y: 0, z: 7 } }] },
                ] }",
            ),
            None,
        );
        let mut child = None;
        for _ in 0..30 {
            app.update();
            consistent(&mut app);
            let map = app.world().resource::<SimEntities>().0.clone();
            if let Some(&e2) = map.get(&EntityId(2))
                && child.is_none()
            {
                let c = app.world_mut().spawn(ChildOf(e2)).id();
                let g = app.world_mut().spawn(ChildOf(c)).id();
                child = Some((e2, c, g));
                assert_eq!(app.world().get::<TeamColour>(e2).unwrap().0, Color::srgb(1.0, 0.0, 0.0));
                assert_eq!(app.world().get::<TeamColour>(map[&EntityId(1)]).unwrap().0, Color::srgb(0.0, 0.0, 1.0));
            }
        }
        let (e2, c, g) = child.expect("unit 2 never appeared");
        assert!(app.world().get_entity(e2).is_err() && app.world().get_entity(c).is_err() && app.world().get_entity(g).is_err());
        assert_eq!(app.world().resource::<SimEntities>().0.len(), 1);
    }

    #[test]
    fn churn_over_a_thousand_ticks_leaves_no_orphans() {
        let mut units = String::new();
        for i in 1..=60u32 {
            let at = i * 13 % 400;
            let until = at + 20 + i * 7 % 150;
            units += &format!("{{ id: {i}, owner: {}, hull: 50, until: {until}, rails: [{{ at: {at}, pos: {{ x: 0, y: 0, z: {i} }} }}] }},", i % 3);
        }
        // A second wave so spawns and removals keep happening past tick 600.
        for i in 61..=120u32 {
            let at = 500 + i * 11 % 450;
            units += &format!("{{ id: {i}, owner: 1, hull: 50, until: {}, rails: [{{ at: {at}, pos: {{ x: 1, y: 0, z: 1 }} }}] }},", at + 30);
        }
        let mut app = app_with(scripted(&format!("{{ units: [{units}] }}")), None);
        let mut peak = 0;
        for _ in 0..1100 {
            app.update();
            consistent(&mut app);
            peak = peak.max(app.world().resource::<SimEntities>().0.len());
        }
        assert!(app.world().resource::<SimLink>().curr().tick >= 1000);
        assert!(peak > 10, "no churn happened");
    }

    /// A fixed list of views, one per tick, the last repeating.
    struct Views(Vec<WorldView>, usize);
    impl WorldSource for Views {
        fn step(&mut self, _: &[Command]) -> TickOutput {
            self.1 = (self.1 + 1).min(self.0.len() - 1);
            TickOutput { view: Arc::new(self.0[self.1].clone()), events: Vec::new() }
        }
        fn view(&self) -> Arc<WorldView> {
            Arc::new(self.0[self.1].clone())
        }
    }

    fn unit(id: u32, owner: u8, hull: i32, flags: UnitFlags) -> UnitView {
        UnitView {
            id: EntityId(id),
            def: ContentKey::NONE,
            owner: PlayerId(owner),
            pos: SimVec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::from_int(hull), Fixed::from_int(100)],
            shields: [[Fixed::ZERO; 2]; 4],
            crew: None,
            energy: None, autonomy: None,
            flags,
            progress: None,
        }
    }

    fn view(tick: u64, units: Vec<UnitView>, visibility: Vec<(PlayerId, Vec<EntityId>)>) -> WorldView {
        WorldView { tick, digest: 0, units, players: Vec::new(), visibility, perceived_owners: Vec::new(), appearance: Default::default(), attrs: Default::default(), colonies: Vec::new() }
    }

    #[test]
    fn damage_nodes_switch_at_hull_thresholds() {
        let hulls = [100, 80, 70, 45, 35, 25, 10, 60];
        let views = hulls.iter().enumerate().map(|(i, &h)| view(i as u64, vec![unit(1, 1, h, UnitFlags::default())], Vec::new())).collect();
        let mut app = app_with(Box::new(Views(views, 0)), None);
        app.update();
        let root = app.world().resource::<SimEntities>().0[&EntityId(1)];
        let names = ["damage", "crew", "engines", "target", "hull"];
        let nodes: Vec<(&str, Entity)> = names
            .into_iter()
            .map(|n| (n, app.world_mut().spawn((Name::new(n), Visibility::Hidden, ChildOf(root))).id()))
            .collect();
        let mut checked = 0;
        for _ in 0..20 {
            app.update();
            let tick = app.world().resource::<SimLink>().curr().tick as usize;
            let share = hulls[tick.min(hulls.len() - 1)] as f32 / 100.0;
            for (name, e) in &nodes {
                let shown = *app.world().get::<Visibility>(*e).unwrap() != Visibility::Hidden;
                let want = damage_threshold(name).is_some_and(|t| share < t);
                assert_eq!(shown, want, "node {name} at hull share {share}");
            }
            checked += 1;
        }
        assert_eq!(checked, 20);
        // Repaired: back to hidden (last view is 60%: only `damage` stays shown).
        assert!(damage_threshold("hull").is_none());
    }

    #[test]
    fn only_what_the_local_player_sees_is_drawn_and_cloaks_are_marked() {
        let all = vec![unit(1, 1, 100, UnitFlags::CLOAKED), unit(2, 2, 100, UnitFlags::default()), unit(3, 2, 100, UnitFlags::default())];
        let v0 = view(0, all, vec![(PlayerId(1), vec![EntityId(1), EntityId(2)]), (PlayerId(2), vec![EntityId(2), EntityId(3)])]);
        let mut v1 = v0.clone();
        v1.tick = 1;
        v1.visibility[0].1 = vec![EntityId(1)];
        let mut app = app_with(Box::new(Views(vec![v0, v1], 0)), Some(PlayerId(1)));
        let ids = |app: &mut App| roots(app).into_iter().map(|r| r.0.0).collect::<BTreeSet<_>>();
        app.update();
        assert_eq!(ids(&mut app), BTreeSet::from([1, 2]));
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(app.world().resource::<SimLink>().curr().tick, 1);
        assert_eq!(ids(&mut app), BTreeSet::from([1]));
        let e = app.world().resource::<SimEntities>().0[&EntityId(1)];
        assert!(app.world().get::<Cloaked>(e).is_some());
    }
}
