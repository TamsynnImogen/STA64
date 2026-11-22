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
//!   the model's damage-state nodes (system nodes by offline systems).
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

/// Whose eyes the map is drawn through. `None` shows every unit in the view; `Some(p)` what
/// `WorldView::visible_to(p)` lists, plus neutral scenery (see [`is_scenery_class`]), which stays drawn unseen.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LocalPlayer(pub Option<PlayerId>);

/// While a cineractive runs the sensors do not limit what is drawn, heard and outlined: a scripted
/// shot's subject (an enemy fleet far from the player's sensors) must be in it. Grade: not measured
/// in the original; STA64's rule, taken from the fog rule already in the code (the cinematic view
/// draws no fog or shroud). It lifts sensor range, not a cloak: a cloaked unit the local player could
/// not otherwise see stays hidden (its own and allied cloaked units are in `visible_to` already).
/// Any other reveal (the fog cheat, UI-46) composes with it: with the fog lifted `visible_to` is
/// already every unit.
pub fn cinematic_reveal(cine: Option<&crate::ui::presentation::CineractiveState>) -> bool {
    cine.is_some_and(|c| c.on)
}

/// The units the client draws, hears and outlines: every unit with no local player; what
/// `visible_to(local)` lists; and, while `reveal` (a cineractive), also every unit that is not cloaked.
pub fn drawn_units(view: &WorldView, local: Option<PlayerId>, reveal: bool) -> Vec<&sta_sim::UnitView> {
    match local {
        None => view.units.iter().collect(),
        Some(p) if reveal => {
            let seen: BTreeSet<EntityId> = view.visible_to(p).into_iter().map(|u| u.id).collect();
            view.units.iter().filter(|u| seen.contains(&u.id) || !u.flags.contains(UnitFlags::CLOAKED)).collect()
        }
        Some(p) => view.visible_to(p),
    }
}

/// The team colour the unit is drawn in, from its owner.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct TeamColour(pub Color);

/// The colour the unit's strategic symbol is drawn in (an Armada I map's own table gives marks their own
/// entry); absent, the symbol follows [`TeamColour`].
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct MarkColour(pub Color);

/// Hull now over hull max, 0..1 (1 when the unit has no hull).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct HullShare(pub f32);

/// Systems of the unit that are offline, as [`systems_off_bits`]: what the model's
/// system nodes follow. A root without it has none offline.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SystemsOff(pub u8);

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

/// The system a model's system node stands for, as an index into the view's system
/// order (engines, weapons, shields, life support, sensors): the node names
/// `engines`, `target` (weapons), `shields`, `life` and `sensors`.
fn node_system(name: &str) -> Option<usize> {
    const NODES: [(&str, usize); 5] = [("engines", 0), ("target", 1), ("shields", 2), ("life", 3), ("sensors", 4)];
    NODES.iter().find(|(n, _)| name.eq_ignore_ascii_case(n)).map(|&(_, i)| i)
}

/// Bit `i` set: system `i` (view order) is offline: not enabled (held off or disabled,
/// [`sta_sim::appearance::SystemAppearance::enabled`]) or at zero health (destroyed,
/// including a script's damage edit that did not switch it off). A unit the view lists
/// no systems for has none offline.
pub fn systems_off_bits(view: &sta_sim::appearance::AppearanceView, id: EntityId) -> u8 {
    let Some(systems) = view.systems(id) else { return 0 };
    systems.iter().enumerate().fold(0, |bits, (i, s)| if s.is_some_and(|s| !s.enabled || s.health[0] <= sta_sim::Fixed::ZERO) { bits | (1 << i) } else { bits })
}

/// Whether the damage-state node `name` shows, or `None` for any other node.
///
/// - A system node (`engines`, `target`, `shields`, `life`, `sensors`) shows its
///   children while that system is offline and hides them while it is online,
///   independent of hull. Grade: read from the executable, static, for Armada II.
///   Fleet Operations, Roots and Armada I were not looked at; the rule is assumed
///   for them (interim).
/// - `damage`, the group the system nodes sit in: shown from hull share
///   [`DAMAGE_GROUP_SHARE`] down, or while any system is offline so its node can show.
///   Interim: what else a model keeps under the group is not established.
/// - `crew`: hull share below [`CREW_SHARE`]. Interim: the rule read for Armada II
///   is by crew share, not implemented here.
pub fn damage_node_shown(name: &str, share: f32, off: u8) -> Option<bool> {
    if let Some(i) = node_system(name) {
        return Some(off & (1 << i) != 0);
    }
    if name.eq_ignore_ascii_case("damage") {
        return Some(share < DAMAGE_GROUP_SHARE || off != 0);
    }
    if name.eq_ignore_ascii_case("crew") {
        return Some(share < CREW_SHARE);
    }
    None
}

/// Interim hull-share thresholds (STA64's own) of the `damage` and `crew` nodes.
const DAMAGE_GROUP_SHARE: f32 = 0.75;
const CREW_SHARE: f32 = 0.5;

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
            (sync_units, attach_models.run_if(resource_exists::<Mounted>).run_if(resource_exists::<ContentRes>), pick_bodies.run_if(resource_exists::<ContentRes>), show_damage)
                .chain()
                .before(place)
                .in_set(GameSet::Interpolate)
                .run_if(resource_exists::<SimLink>),
        )
        .add_systems(Update, place.in_set(GameSet::Interpolate).run_if(resource_exists::<SimLink>));
    crate::render::anim::add_systems(app);
}

/// The click radius of a planet or a resource node from its definition, for every sim entity
/// that has none: the entity [`sync_units`] makes when the object comes back into sight is
/// not the one the map view made (which went when the object left it), and
/// [`attach_models`] gives a radius only to what it draws a model for (a planet's body is
/// the map's, a latinum nebula has no model), so without this such an object cannot be picked
/// after it was out of sight once. A model-derived radius, given first, stays. Runs once per new
/// entity (`Added<SimEntity>`): a definition does not change in place, so the radius follows the
/// definition the entity was spawned with, and an entity that gets none is not rescanned.
#[allow(clippy::type_complexity)]
fn pick_bodies(mut commands: Commands, link: Res<SimLink>, content: Res<ContentRes>, bare: Query<(Entity, &SimEntity), (Added<SimEntity>, Without<SelectRadius>)>) {
    for (e, id) in &bare {
        let Some(r) = link.curr().unit(id.0).and_then(|u| content.0.unit(u.def)).and_then(crate::map::ghost_pick_radius) else { continue };
        commands.entity(e).insert(SelectRadius(r));
    }
}

/// What `sync_units` last drew: the view, the local player and whether a cineractive revealed the view.
type SeenKey = (Arc<WorldView>, Option<PlayerId>, bool);

/// The view-spawned units `sync_units` updates in place.
type LiveUnits = (&'static mut HullShare, &'static mut UnitOwner, Has<Cloaked>, Option<&'static mut SystemsOff>);

/// Diff the current view against [`SimEntities`].
#[allow(clippy::too_many_arguments)]
pub fn sync_units(
    mut commands: Commands,
    link: Res<SimLink>,
    local: Res<LocalPlayer>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    teams: Option<Res<TeamColours>>,
    map_teams: Option<Res<crate::viewer::MapTeamColours>>,
    mut map: ResMut<SimEntities>,
    existing: Query<(Entity, &SimEntity), Without<FromView>>,
    mut live: Query<LiveUnits, With<FromView>>,
    mut placed: Query<(Option<&mut HullShare>, Option<&mut SystemsOff>), Without<FromView>>,
    mut seen: Local<Option<SeenKey>>,
    content: Option<Res<ContentRes>>,
) {
    // Entities the map view spawned for its own ships belong to the link too.
    for (e, u) in &existing {
        map.0.entry(u.0).or_insert(e);
    }
    let curr = link.curr();
    // While a cineractive runs the sensors do not limit the view (`drawn_units`); the next frame after
    // it ends the local player's sight applies again and the revealed enemies are despawned below.
    let reveal = cinematic_reveal(cine.as_deref());
    if seen.as_ref().is_some_and(|(v, p, r)| Arc::ptr_eq(v, curr) && *p == local.0 && *r == reveal) {
        return;
    }
    *seen = Some((curr.clone(), local.0, reveal));

    let mut want: BTreeMap<EntityId, &sta_sim::UnitView> = drawn_units(curr, local.0, reveal).into_iter().map(|u| (u.id, u)).collect();
    // A neutral planet or backdrop is scenery and stays drawn outside the sensors. Grade: that the original
    // draws planets regardless of sensors is not measured, inferred from the stock look. Clicks, hover, right-click orders, the order
    // cursor and HUD targeting follow `visible_to` (`input::select::target_seen`), so a kept planet cannot be
    // picked or ordered while unseen.
    if local.0.is_some()
        && let Some(content) = &content
    {
        for u in &curr.units {
            if u.owner == sta_sim::world::NEUTRAL && content.0.unit(u.def).is_some_and(|d| is_scenery_class(&d.class.0)) {
                want.entry(u.id).or_insert(u);
            }
        }
    }

    // Gone: not in the view, or neither visible to the local player nor neutral scenery.
    let gone: Vec<EntityId> = map.0.keys().filter(|id| !want.contains_key(id)).copied().collect();
    for id in gone {
        if let Some(e) = map.0.remove(&id) {
            commands.entity(e).try_despawn();
        }
    }
    // Seat n takes palette entry n + 1 (entry 0 is the neutral one): `TeamColours::owner_colour`.
    // Unowned units (moons, props, derelicts) stay plain white, as they always were here.
    let colour = |owner: PlayerId| {
        if owner == sta_sim::world::NEUTRAL {
            return Color::WHITE;
        }
        // An Armada I map's own team table, where one applies (`MapTeamColours`): seat n is team n + 1.
        crate::viewer::owner_hull_colour(teams.as_deref(), map_teams.as_deref(), owner).unwrap_or(Color::WHITE)
    };
    let mark = |owner: PlayerId| {
        if owner == sta_sim::world::NEUTRAL {
            return Color::WHITE;
        }
        crate::viewer::owner_mark_colour(teams.as_deref(), map_teams.as_deref(), owner).unwrap_or(Color::WHITE)
    };
    let mut stale: BTreeSet<EntityId> = BTreeSet::new();
    for (id, u) in want {
        let share = hull_share(u);
        let off = systems_off_bits(&curr.appearance, id);
        let cloaked = u.flags.contains(UnitFlags::CLOAKED);
        if let Some(&e) = map.0.get(&id) {
            if let Ok((mut h, mut o, was_cloaked, was_off)) = live.get_mut(e) {
                if h.0 != share {
                    h.0 = share;
                }
                match was_off {
                    Some(mut was) => {
                        if was.0 != off {
                            was.0 = off;
                        }
                    }
                    None => {
                        commands.entity(e).insert(SystemsOff(off));
                    }
                }
                if o.0 != u.owner {
                    o.0 = u.owner;
                    commands.entity(e).insert((TeamColour(colour(u.owner)), MarkColour(mark(u.owner))));
                }
                if cloaked != was_cloaked {
                    if cloaked {
                        commands.entity(e).insert(Cloaked);
                    } else {
                        commands.entity(e).remove::<Cloaked>();
                    }
                }
            } else if let Ok((was_hull, was_off)) = placed.get_mut(e) {
                // A ship the map view drew for its own object: its damage state follows the
                // view too (it is the same unit), or its system and hull nodes, hull bar and
                // smoke would never change.
                match was_hull {
                    Some(mut was) => {
                        if was.0 != share {
                            was.0 = share;
                        }
                    }
                    None => {
                        commands.entity(e).insert(HullShare(share));
                    }
                }
                match was_off {
                    Some(mut was) => {
                        if was.0 != off {
                            was.0 = off;
                        }
                    }
                    None => {
                        commands.entity(e).insert(SystemsOff(off));
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
            MarkColour(mark(u.owner)),
            HullShare(share),
            SystemsOff(off),
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

/// Classes of neutral things that are the map's scenery: always drawn, never an icon (`ODF classLabel`, read from data).
pub fn is_scenery_class(class: &str) -> bool {
    ["planet_obj", "planet", "background_obj"].iter().any(|c| class.eq_ignore_ascii_case(c))
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
pub(crate) fn attach_models(
    mut commands: Commands,
    pending: Query<(Entity, &NeedsModel, &TeamColour, Option<&UnitOwner>, Option<&SimEntity>, Option<&crate::render::construction::ConstructionPreview>, Option<&crate::render::build_visuals::BuildVisual>, Option<&MarkColour>,
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
    for (e, need, team, owner, sim, preview, build_visual, mark) in &pending {
        commands.entity(e).remove::<NeedsModel>();
        let Some(def) = content.0.unit(need.0) else { continue;
        };
        // A planet's body is the map view's (`map/mod.rs` builds it from the map's object); a sim
        // planet with no map object (a bench scene's) has none, and its SOD sphere would be an
        // untextured dark ball hiding the map's planet drawn at the same place.
        // Scenery never gets a strategic icon (also an A2 `planet` re-created from the view, which has no model here).
        if is_scenery_class(&def.class.0) {
            commands.entity(e).insert(crate::render::strategic::NoIcon);
        }
        if def.class.0.eq_ignore_ascii_case("planet") {
            continue;
        }
        // A resource moon's marker is drawn as the moon the game swaps it for (RND-16).
        let def = moons.moon_for(&vfs.0, &content.0, def, sim.map_or(0, |s| s.0.0 as u64)).unwrap_or(def);
        let override_model = build_visual.and_then(crate::render::build_visuals::BuildVisual::model_override);
        let key = override_model.unwrap_or(&def.model).to_ascii_lowercase().trim_end_matches(".sod").to_owned();
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
        let scale = if override_model.is_some() { 1.0 } else { def.scale.to_f32() };
        // What the map view gives the ships it draws itself: a click radius from the
        // model's bounds, and for team-owned things a strategic icon of the right
        // shape (the mod's `<model>_sv` sprite, else a silhouette of the model). Units
        // that come into sight, or are placed at the start, are drawn here.
        let model_scale = if scale > 0.0 { scale } else { 1.0 };
        let bounds = model.bounds();
        if let Some((lo, hi)) = bounds {
            commands.entity(e).insert(crate::render::overlays::ModelBounds::from_legacy(lo, hi, model_scale));
        }
        let radius = bounds.map(|(lo, hi)| (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5).unwrap_or(if def.resource.is_some() { 0.0 } else { 20.0 }) * model_scale;
        let centre = bounds.map(|(lo, hi)| crate::render::strategic::centre_for_bounds(lo, hi, model_scale)).unwrap_or_default();
        let mobile = def.movement.max_speed > sta_sim::Fixed::ZERO;
        let owned = owner.is_some_and(|o| o.0 != sta_sim::world::NEUTRAL);
        // A resource (a moon) is neutral and never moves, but is clicked for its info and
        // as a mining target, so it gets a click radius too: of the moon the marker was swapped for.
        if build_visual.is_none() && (mobile || owned || def.resource.is_some()) {
            let radius = if def.resource.is_some() { crate::map::resource_pick_radius(radius) } else { radius };
            commands.entity(e).insert(SelectRadius(radius.max(5.0)));
        }
        let class = def.class.0.as_str();
        if owned && !matches!(class, "background_obj" | "latinum" | "asteroidfield" | "nebula_obj" | "planet" | "wormhole_obj") {
            let mut icon = crate::render::strategic::Strategic::new(crate::render::strategic::shape_for(class, mobile), mark.map_or(team.0, |m| m.0), radius,
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
            damaged: build_visual.is_none(),
            normal_maps: true,
            parts: Some(&mut parts),
            model_key: path,
            unlit: false,
            scale: if scale > 0.0 { scale } else { 1.0 },
            diffuse_tint: false,
        };
        let progress = preview.map(|p| p.0).or_else(|| sim
            .and_then(|unit| link.curr().unit(unit.0))
            .and_then(|unit| unit.progress)
            .map(|progress| progress.to_f32().clamp(0.0, 1.0)));
        let stats = if progress.is_some_and(|progress| progress < 1.0) {
            let chunks = construction_chunk_count(def);
            model_scene::spawn_construction(&mut ctx, &model, e, chunks, 0.0)
        } else {
            // A resource rock's model turns on a pivot below the unit, which `place` leaves alone.
            let under = if sim.is_some() && def.is_evolver() {
                ctx.commands.spawn((
                    crate::render::build_visuals::EmbryoMesh, Transform::IDENTITY, Visibility::Inherited, ChildOf(e),
                )).id()
            } else {
                crate::render::spin::rock_pivot(ctx.commands, e, &def.class.0, def.raw.f32("rotationSpeed"), sim.map_or(0, |s| s.0.0 as u64))
            };
            model_scene::spawn(&mut ctx, &model, under)
        };
        if override_model.is_some() {
            commands.entity(e).insert(crate::render::build_visuals::CocoonMesh);
            continue;
        }
        if def.class.0.eq_ignore_ascii_case("planet_obj") {
            model_scene::spawn_planet_nimbus(&mut ctx, &def.model, e);
        }
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

/// Show each damage-state node of a unit's model by [`damage_node_shown`]: the system
/// nodes (and the emitters under them) by the unit's offline systems, the `damage` and
/// `crew` nodes by hull share (interim).
#[allow(clippy::type_complexity)]
pub fn show_damage(
    units: Query<(Entity, &HullShare, Option<&SystemsOff>), Or<(Changed<HullShare>, Changed<SystemsOff>, Changed<Children>)>>,
    children: Query<&Children>,
    mut nodes: Query<(&Name, &mut Visibility), Without<model_scene::NativeNode>>,
) {
    for (unit, share, off) in &units {
        let off = off.map_or(0, |o| o.0);
        for d in children.iter_descendants(unit) {
            let Ok((name, mut vis)) = nodes.get_mut(d) else { continue };
            let Some(show) = damage_node_shown(name, share.0, off) else { continue };
            let want = if show { Visibility::Inherited } else { Visibility::Hidden };
            if *vis != want {
                *vis = want;
            }
        }
    }
}

/// An authored `oq` group contains numbered quarters `oq1`, `oq2`, etc.
/// Only those exact numbered descendants follow completed Armada I base upgrades.
pub(super) fn officer_quarter_number(model: &Model, index: usize) -> Option<u32> {
    let node = model.nodes.get(index)?;
    let name = node.name.to_ascii_lowercase();
    let suffix = name.strip_prefix("oq")?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = suffix.parse::<u32>().ok().filter(|number| *number > 0)?;
    let mut parent = node.parent;
    while let Some(index) = parent {
        let ancestor = model.nodes.get(index)?;
        if ancestor.name.eq_ignore_ascii_case("oq") {
            return Some(number);
        }
        parent = ancestor.parent;
    }
    None
}

pub fn to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], -p[2])
}

pub(super) fn research_rotation(axes: [sta_sim::Vec3; 3]) -> Quat {
    let [_, y, z] = axes.map(|v| to_bevy(v.to_f32()));
    // The model's local +Z is mirrored as well as the world axes.
    Transform::IDENTITY.looking_to(z, y).rotation
}

/// Place each unit between its last two ticks.
pub(crate) fn place(link: Res<SimLink>, clock: Res<AnimClock>, mut q: Query<(&SimEntity, &mut Transform)>) {
    let alpha = clock.alpha().clamp(0.0, 1.0);
    let pod_axes: std::collections::BTreeMap<_, _> = link.production().iter()
        .filter_map(|row| row.research.as_ref())
        .flat_map(|research| research.pods.iter())
        .map(|pod| (pod.entity, pod.axes))
        .collect();
    for (u, mut tf) in &mut q {
        let Some((pos, h)) = link.interpolate(u.0, alpha) else { continue };
        tf.translation = to_bevy(pos);
        // The model's front (+Z in the files) is Bevy's -Z after the mirror, which
        // is what `looking_to` points along the heading.
        tf.rotation = pod_axes.get(&u.0).copied().map_or_else(
            || Transform::IDENTITY.looking_to(Vec3::new(h[0], 0.0, -h[1]), Vec3::Y).rotation,
            research_rotation,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn officer_quarters_match_only_exact_numbered_descendants_of_oq() {
        use sta_assets::a2fmt::Matrix34;
        use sta_assets::model::{Node, NodeKind, SourceFormat};

        let names = ["root", "oq", "oq1", "oq2", "oq3_extra", "oq0", "oqx", "other", "oq4", "OQ5"];
        let parents = [None, Some(0), Some(1), Some(2), Some(1), Some(1), Some(1), Some(0), Some(7), Some(2)];
        let model = Model {
            format: SourceFormat::Sod,
            version: 1.93,
            materials: vec![],
            nodes: names.iter().zip(parents).map(|(name, parent)| Node {
                name: (*name).into(), parent, children: vec![],
                transform: Matrix34::IDENTITY, kind: NodeKind::Null,
            }).collect(),
            channels: vec![], flipbooks: vec![], warnings: vec![], pbr: None,
        };
        let got: Vec<_> = (0..model.nodes.len()).map(|i| officer_quarter_number(&model, i)).collect();
        assert_eq!(got, [None, None, Some(1), Some(2), None, None, None, None, None, Some(5)]);
    }

    #[test]
    fn research_rotation_follows_a_tilted_legacy_socket() {
        let axes = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]]
            .map(sta_sim::Vec3::from_f32);
        let rotation = research_rotation(axes);
        assert!((rotation * Vec3::NEG_Z).abs_diff_eq(Vec3::NEG_Y, 0.0001));
        assert!((rotation * Vec3::Y).abs_diff_eq(Vec3::NEG_Z, 0.0001));
    }
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
                ("empty".into(), Color::srgb(0.5, 0.5, 0.5)),
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

    /// A map's own team colours reach hulls and marks through `sync_units` while the resource is there, and
    /// a unit drawn after it is gone (the match ended: `clear_match`) takes the palette's colours.
    #[test]
    fn the_map_team_colours_apply_while_present_and_not_after_they_are_removed() {
        use crate::viewer::MapTeamColours;
        let mut app = app_with(
            scripted(
                "{ units: [
                    { id: 1, owner: 0, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
                    { id: 2, owner: 0, hull: 100, rails: [{ at: 20, pos: { x: 5, y: 0, z: 7 } }] },
                ] }",
            ),
            None,
        );
        // Seat 0 is team 1: hull entry 2 (`red`), mark entry 3 (`blue`); the palette alone gives it `blue`.
        app.insert_resource(MapTeamColours(vec![[0, 0], [2, 3]]));
        for _ in 0..3 {
            app.update();
        }
        let map = app.world().resource::<SimEntities>().0.clone();
        assert_eq!(app.world().get::<TeamColour>(map[&EntityId(1)]).unwrap().0, Color::srgb(1.0, 0.0, 0.0));
        assert_eq!(app.world().get::<MarkColour>(map[&EntityId(1)]).unwrap().0, Color::srgb(0.0, 0.0, 1.0));
        app.world_mut().remove_resource::<MapTeamColours>();
        for _ in 0..40 {
            app.update();
        }
        let map = app.world().resource::<SimEntities>().0.clone();
        let late = map.get(&EntityId(2)).copied().expect("the second unit appeared");
        assert_eq!(app.world().get::<TeamColour>(late).unwrap().0, Color::srgb(0.0, 0.0, 1.0), "the palette's entry for seat 0");
        assert_eq!(app.world().get::<MarkColour>(late).unwrap().0, Color::srgb(0.0, 0.0, 1.0));
    }

    #[test]
    fn scripted_spawn_and_destroy() {
        let mut app = app_with(
            scripted(
                "{ units: [
                    { id: 1, owner: 0, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
                    { id: 2, owner: 1, hull: 100, until: 10, rails: [{ at: 4, pos: { x: 5, y: 0, z: 7 } }] },
                    { id: 3, owner: 253, hull: 100, rails: [{ at: 0, pos: { x: 2, y: 0, z: 2 } }] },
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
                assert_eq!(app.world().get::<TeamColour>(map[&EntityId(3)]).unwrap().0, Color::WHITE, "unowned: plain white, as on main (the palette entry 0 is grey here)");
                // Seat 0 is the first team (entry 1), not the neutral entry 0.
                assert_ne!(app.world().get::<TeamColour>(map[&EntityId(1)]).unwrap().0, Color::srgb(0.5, 0.5, 0.5));
            }
        }
        let (e2, c, g) = child.expect("unit 2 never appeared");
        assert!(app.world().get_entity(e2).is_err() && app.world().get_entity(c).is_err() && app.world().get_entity(g).is_err());
        assert_eq!(app.world().resource::<SimEntities>().0.len(), 2);
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
        WorldView { tick, units, visibility, ..WorldView::default() }
    }

    /// One view of unit 1 at `hull` of 100, with system `off` (view order) offline, if any.
    fn view_with_system(tick: u64, hull: i32, off: Option<usize>) -> WorldView {
        use sta_sim::appearance::SystemAppearance;
        let mut v = view(tick, vec![unit(1, 1, hull, UnitFlags::default())], Vec::new());
        let systems = std::array::from_fn(|i| {
            let on = off != Some(i);
            Some(SystemAppearance { enabled: on, health: [if on { Fixed::from_int(20) } else { Fixed::ZERO }, Fixed::from_int(20)] })
        });
        v.appearance.systems = vec![(EntityId(1), systems)];
        v
    }

    #[test]
    fn system_nodes_follow_the_offline_system_not_the_hull() {
        // Hull 100 with every system online; hull 10 online; hull 100 with engines offline;
        // the same with weapons offline; engines back online (repaired).
        let steps: [(i32, Option<usize>); 5] = [(100, None), (10, None), (100, Some(0)), (100, Some(1)), (100, None)];
        let views = steps.iter().enumerate().map(|(i, &(h, off))| view_with_system(i as u64, h, off)).collect();
        let mut app = app_with(Box::new(Views(views, 0)), None);
        app.update();
        let root = app.world().resource::<SimEntities>().0[&EntityId(1)];
        let names = ["damage", "crew", "engines", "target", "shields", "life", "sensors", "hull"];
        let nodes: Vec<(&str, Entity)> = names
            .into_iter()
            .map(|n| (n, app.world_mut().spawn((Name::new(n), Visibility::Hidden, ChildOf(root))).id()))
            .collect();
        let mut seen = BTreeSet::new();
        for _ in 0..20 {
            app.update();
            let tick = app.world().resource::<SimLink>().curr().tick as usize;
            let (hull, off) = steps[tick.min(steps.len() - 1)];
            seen.insert(tick);
            for (name, e) in &nodes {
                let shown = *app.world().get::<Visibility>(*e).unwrap() != Visibility::Hidden;
                let want = match *name {
                    "engines" => off == Some(0),
                    "target" => off == Some(1),
                    "shields" | "life" | "sensors" | "hull" => false,
                    "damage" => hull < 75 || off.is_some(),
                    _ => hull < 50,
                };
                assert_eq!(shown, want, "node {name} at hull {hull}, offline system {off:?}");
            }
        }
        assert!(seen.iter().max().is_some_and(|&t| t >= steps.len() - 1), "the last step was reached: {seen:?}");
    }

    /// A ship the map view drew for its own object (a `SimEntity` root that `sync_units` did not
    /// make) follows its unit's offline systems and hull like any other: a derelict scout placed by a
    /// map showed no venting because only `sync_units`' own roots had the state.
    #[test]
    fn a_ship_the_map_view_drew_shows_its_offline_systems_and_hull_too() {
        let views = vec![view_with_system(0, 100, None), view_with_system(1, 40, Some(0)), view_with_system(2, 40, None)];
        let mut app = app_with(Box::new(Views(views, 0)), None);
        let root = app.world_mut().spawn((SimEntity(EntityId(1)), Visibility::default())).id();
        let engines = app.world_mut().spawn((Name::new("engines"), Visibility::Hidden, ChildOf(root))).id();
        let damage = app.world_mut().spawn((Name::new("damage"), Visibility::Hidden, ChildOf(root))).id();
        let mut seen = BTreeSet::new();
        for _ in 0..20 {
            app.update();
            let tick = app.world().resource::<SimLink>().curr().tick;
            seen.insert(tick);
            assert_eq!(app.world().resource::<SimEntities>().0[&EntityId(1)], root, "the map view's root stays the unit's root");
            let shown = |app: &App, e| *app.world().get::<Visibility>(e).unwrap() != Visibility::Hidden;
            assert_eq!(shown(&app, engines), tick == 1, "engines node at tick {tick}");
            assert_eq!(shown(&app, damage), tick >= 1, "damage node at tick {tick}");
            let share = app.world().get::<HullShare>(root).map(|h| h.0);
            assert_eq!(share, Some(if tick == 0 { 1.0 } else { 0.4 }), "hull share at tick {tick}");
        }
        assert!(seen.contains(&2), "the last step was reached: {seen:?}");
    }

    #[test]
    fn every_offline_system_shows_its_node_and_a_model_without_nodes_shows_nothing() {
        let names = ["engines", "target", "shields", "life", "sensors"];
        for (i, name) in names.iter().enumerate() {
            assert_eq!(damage_node_shown(name, 1.0, 0), Some(false), "{name} online");
            assert_eq!(damage_node_shown(name, 1.0, 1 << i), Some(true), "{name} offline");
            assert_eq!(damage_node_shown(&name.to_uppercase(), 1.0, 1 << i), Some(true), "{name} any case");
            for (j, other) in names.iter().enumerate().filter(|&(j, _)| j != i) {
                assert_eq!(damage_node_shown(other, 1.0, 1 << i), Some(false), "{other} (system {j}) with only system {i} offline");
            }
        }
        for other in ["hull", "geometry", "hp01", "borg", ""] {
            assert_eq!(damage_node_shown(other, 0.0, 0b11111), None, "{other:?} is not a damage node");
        }
        // Offline is "not enabled" or "zero health": destroyed and switched off (health 0), disabled or
        // held off (health full), and zero health that was never switched off all count; a system
        // the unit lacks and a damaged but working one do not.
        use sta_sim::appearance::{AppearanceView, SystemAppearance};
        let sys = |enabled, hp| Some(SystemAppearance { enabled, health: [Fixed::from_int(hp), Fixed::from_int(20)] });
        let view = AppearanceView { systems: vec![(EntityId(1), [sys(false, 0), sys(false, 20), sys(true, 0), None, sys(true, 7)])], ..AppearanceView::default() };
        assert_eq!(systems_off_bits(&view, EntityId(1)), 0b00111);
        assert_eq!(systems_off_bits(&view, EntityId(2)), 0);
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

    #[test]
    fn a_cineractive_draws_the_enemy_fleet_and_it_is_gone_the_next_frame_after_it_ends() {
        use crate::ui::presentation::CineractiveState;
        let all = vec![unit(1, 1, 100, UnitFlags::default()), unit(2, 2, 100, UnitFlags::default()), unit(3, 2, 100, UnitFlags::default())];
        // Player 1 sees only its own unit 1; 2 and 3 belong to the enemy and are out of its sensors.
        let v = view(0, all, vec![(PlayerId(1), vec![EntityId(1)]), (PlayerId(2), vec![EntityId(2), EntityId(3)])]);
        let mut app = app_with(Box::new(Views(vec![v], 0)), Some(PlayerId(1)));
        app.insert_resource(CineractiveState::default());
        let ids = |app: &mut App| roots(app).into_iter().map(|r| r.0.0).collect::<BTreeSet<_>>();
        app.update();
        app.update();
        assert_eq!(ids(&mut app), BTreeSet::from([1]), "the enemy is out of sight in play");
        app.world_mut().resource_mut::<CineractiveState>().on = true;
        app.update();
        assert_eq!(ids(&mut app), BTreeSet::from([1, 2, 3]), "a cineractive shows the enemy fleet its shot is about");
        app.world_mut().resource_mut::<CineractiveState>().on = false;
        app.update();
        assert_eq!(ids(&mut app), BTreeSet::from([1]), "and it is gone again the next frame after the cineractive ends");
        consistent_with_visible(&mut app);
    }

    #[test]
    fn a_cineractive_lifts_sensor_range_not_a_cloak() {
        let all = vec![
            unit(1, 1, 100, UnitFlags::CLOAKED),
            unit(2, 2, 100, UnitFlags::default()),
            unit(3, 2, 100, UnitFlags::default()),
            unit(4, 2, 100, UnitFlags::CLOAKED),
            unit(5, 2, 100, UnitFlags::CLOAKED),
        ];
        // Player 1 sees its own cloaked unit 1, the enemy 2 and, by a sensor, the enemy cloaked 5.
        let v = view(0, all, vec![(PlayerId(1), vec![EntityId(1), EntityId(2), EntityId(5)])]);
        let ids = |local: Option<PlayerId>, reveal: bool| drawn_units(&v, local, reveal).into_iter().map(|u| u.id.0).collect::<Vec<_>>();
        assert_eq!(ids(Some(PlayerId(1)), false), [1, 2, 5], "in play: what the sensors list");
        assert_eq!(ids(Some(PlayerId(1)), true), [1, 2, 3, 5], "a cineractive adds the unseen enemy, not the unseen cloaked one (4)");
        assert_eq!(ids(None, true), [1, 2, 3, 4, 5], "with no local player everything is drawn, as before");
    }

    #[test]
    fn a_cineractive_while_the_fog_is_lifted_keeps_the_revealed_units() {
        use crate::ui::presentation::CineractiveState;
        let all = vec![unit(1, 1, 100, UnitFlags::default()), unit(2, 2, 100, UnitFlags::default()), unit(3, 2, 100, UnitFlags::default())];
        let v = view(0, all, vec![(PlayerId(1), vec![EntityId(1)]), (PlayerId(2), vec![EntityId(2), EntityId(3)])]);
        let mut app = app_with(Box::new(Views(vec![v], 0)), Some(PlayerId(1)));
        app.insert_resource(CineractiveState::default());
        app.world_mut().resource_mut::<SimLink>().set_fog_lifted(true);
        let ids = |app: &mut App| roots(app).into_iter().map(|r| r.0.0).collect::<BTreeSet<_>>();
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(ids(&mut app), BTreeSet::from([1, 2, 3]), "the fog cheat shows every unit");
        app.world_mut().resource_mut::<CineractiveState>().on = true;
        app.update();
        app.update();
        assert_eq!(ids(&mut app), BTreeSet::from([1, 2, 3]));
        app.world_mut().resource_mut::<CineractiveState>().on = false;
        app.update();
        app.update();
        assert_eq!(ids(&mut app), BTreeSet::from([1, 2, 3]), "the cineractive's end does not despawn what the cheat reveals");
    }

    fn consistent_with_visible(app: &mut App) {
        let map: BTreeSet<EntityId> = app.world().resource::<SimEntities>().0.keys().copied().collect();
        let ents: BTreeSet<EntityId> = roots(app).into_iter().map(|r| r.0).collect();
        assert_eq!(map, ents, "no orphan entity outlives its map entry");
    }

    fn defs() -> Arc<sta_content::ContentDb> {
        let mut planet = sta_content::UnitDef::new("pb_test", "planet");
        planet.planet_role = Some(sta_content::PlanetRole::Producer);
        planet.resource = Some(0);
        planet.radius = Fixed::from_int(130);
        let mut nebula = sta_content::UnitDef::new("latinum_test", "latinum");
        nebula.resource = Some(1);
        let mut gate = sta_content::UnitDef::new("gate_test", "wormhole_obj");
        gate.radius = Fixed::from_int(60);
        let mut ship = sta_content::UnitDef::new("ship_test", "frigate");
        ship.radius = Fixed::from_int(20);
        Arc::new(sta_content::ContentDb::builder().unit(planet).unit(nebula).unit(gate).unit(ship).build().unwrap())
    }

    fn on(db: &sta_content::ContentDb, id: u32, def: &str) -> UnitView {
        UnitView { def: db.key(def).unwrap(), ..unit(id, 0, 100, UnitFlags::default()) }
    }

    fn radius(app: &mut App, id: u32) -> Option<(Entity, Option<f32>)> {
        let mut q = app.world_mut().query::<(Entity, &SimEntity, Option<&SelectRadius>)>();
        q.iter(app.world()).find(|(_, s, _)| s.0 == EntityId(id)).map(|(e, _, r)| (e, r.map(|r| r.0)))
    }

    fn run_to(app: &mut App, tick: u64) {
        for _ in 0..200 {
            app.update();
            if app.world().resource::<SimLink>().curr().tick >= tick {
                return;
            }
        }
        panic!("never reached tick {tick}");
    }

    /// An object that leaves the local team's view is despawned, and the entity made when it is
    /// seen again must have its click radius: `attach_models` (which runs here, with a mount
    /// that has no models) gives none to a planet (its body is the map's) or to a node whose
    /// model is missing, and a gate has none from its model on a re-sighting either.
    #[test]
    fn a_planet_or_node_seen_again_can_still_be_picked() {
        let db = defs();
        let all = vec![on(&db, 1, "pb_test"), on(&db, 2, "latinum_test"), on(&db, 3, "ship_test"), on(&db, 4, "gate_test")];
        let seen = |ids: &[u32]| vec![(PlayerId(1), ids.iter().map(|&i| EntityId(i)).collect())];
        let views = vec![view(0, all.clone(), seen(&[1, 2, 3, 4])), view(1, all.clone(), seen(&[3])), view(2, all.clone(), seen(&[1, 2, 3, 4]))];
        let mut app = app_with(Box::new(Views(views, 0)), Some(PlayerId(1)));
        app.add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<Image>()
            .init_resource::<TextureCache>()
            .init_resource::<PartCache>()
            .insert_resource(SpriteTables(Arc::new(Default::default())))
            .insert_resource(Mounted(sta_vfs::Vfs::default()))
            .insert_resource(ContentRes(db.clone()));
        let want = [(1, Some(130.0)), (2, Some(crate::map::FIELD_RADIUS)), (3, None), (4, Some(60.0))];
        run_to(&mut app, 0);
        let first: Vec<_> = want.iter().map(|&(id, r)| (radius(&mut app, id).unwrap(), r)).collect();
        for ((_, got), want) in &first {
            // The ship's radius would come from its model; the mount has none.
            assert_eq!(got, want, "first sight");
        }
        run_to(&mut app, 1);
        for id in [1, 2, 4] {
            assert!(radius(&mut app, id).is_none(), "object {id} left the view: despawned");
        }
        run_to(&mut app, 2);
        for (&(id, r), ((old, _), _)) in want.iter().zip(&first) {
            let (now, got) = radius(&mut app, id).unwrap();
            if id != 3 {
                assert_ne!(now, *old, "object {id} is a new entity");
            }
            assert_eq!(got, r, "object {id} seen again");
        }
    }

    /// A radius an entity already has (the model's, given by `attach_models` first) is kept.
    #[test]
    fn a_model_radius_wins_over_the_definitions() {
        let db = defs();
        let views = vec![view(0, vec![on(&db, 1, "pb_test"), on(&db, 2, "latinum_test")], Vec::new())];
        let mut app = app_with(Box::new(Views(views, 0)), None);
        app.insert_resource(ContentRes(db.clone()));
        let (a, b) = (app.world_mut().spawn((SimEntity(EntityId(1)), SelectRadius(7.0))).id(), app.world_mut().spawn((SimEntity(EntityId(2)), SelectRadius(9.0))).id());
        let c = app.world_mut().spawn(SimEntity(EntityId(2))).id();
        app.update();
        assert_eq!(app.world().get::<SelectRadius>(a).map(|r| r.0), Some(7.0));
        assert_eq!(app.world().get::<SelectRadius>(b).map(|r| r.0), Some(9.0));
        assert_eq!(app.world().get::<SelectRadius>(c).map(|r| r.0), Some(crate::map::FIELD_RADIUS), "a bare one gets the definition's");
    }

    /// A neutral planet outside every sensor stays drawn (the original shows its planets wherever the camera
    /// looks); a neutral ship outside them does not.
    #[test]
    fn neutral_scenery_is_drawn_outside_sensor_range_and_a_neutral_ship_is_not() {
        use sta_sim::world::NEUTRAL;
        let db = sta_content::ContentDb::builder()
            .unit(sta_content::UnitDef::new("t_backdrop", "planet_obj"))
            .unit(sta_content::UnitDef::new("t_ship", "wingman"))
            .build()
            .unwrap();
        let mut planet = unit(2, NEUTRAL.0, 100, UnitFlags::default());
        planet.def = db.key("t_backdrop").unwrap();
        let mut ship = unit(3, NEUTRAL.0, 100, UnitFlags::default());
        ship.def = db.key("t_ship").unwrap();
        let v = view(0, vec![unit(1, 1, 100, UnitFlags::default()), planet, ship], vec![(PlayerId(1), vec![EntityId(1)])]);
        let mut app = app_with(Box::new(Views(vec![v], 0)), Some(PlayerId(1)));
        app.insert_resource(ContentRes(Arc::new(db)));
        for _ in 0..3 {
            app.update();
        }
        let ids = roots(&mut app).into_iter().map(|r| r.0.0).collect::<BTreeSet<_>>();
        assert_eq!(ids, BTreeSet::from([1, 2]), "the planet stays, the unseen neutral ship does not");
    }

    #[test]
    fn scenery_classes_are_the_planet_and_backdrop_classes() {
        assert!(is_scenery_class("planet_obj") && is_scenery_class("Planet") && is_scenery_class("background_obj"));
        assert!(!is_scenery_class("wingman") && !is_scenery_class("station") && !is_scenery_class("asteroid"));
    }

    /// A view-drawn planet gets no strategic arrow: with the default radius it would turn into a speck and hide
    /// its model from the zoom where its projected size drops under the symbol threshold.
    #[test]
    fn view_drawn_scenery_gets_no_strategic_icon() {
        let mut app = App::new();
        app.add_systems(Update, crate::render::strategic::ensure_icons);
        let ship = app.world_mut().spawn((FromView, SimEntity(EntityId(1)))).id();
        let planet = app.world_mut().spawn((FromView, SimEntity(EntityId(2)), crate::render::strategic::NoIcon)).id();
        app.update();
        assert!(app.world().get::<crate::render::strategic::Strategic>(ship).is_some());
        assert!(app.world().get::<crate::render::strategic::Strategic>(planet).is_none());
    }

    /// The real game's path: the map view's own entity (a `SimEntity` without `FromView`) for a neutral planet outside
    /// every sensor survives `sync_units` and stays the one entity of that id; a neutral ship outside them is despawned.
    #[test]
    fn the_map_views_own_planet_entity_survives_while_unseen() {
        use sta_sim::world::NEUTRAL;
        let db = sta_content::ContentDb::builder()
            .unit(sta_content::UnitDef::new("t_backdrop", "planet_obj"))
            .unit(sta_content::UnitDef::new("t_ship", "wingman"))
            .build()
            .unwrap();
        let mut planet = unit(2, NEUTRAL.0, 100, UnitFlags::default());
        planet.def = db.key("t_backdrop").unwrap();
        let mut ship = unit(3, NEUTRAL.0, 100, UnitFlags::default());
        ship.def = db.key("t_ship").unwrap();
        let v = view(0, vec![unit(1, 1, 100, UnitFlags::default()), planet, ship], vec![(PlayerId(1), vec![EntityId(1)])]);
        let mut app = app_with(Box::new(Views(vec![v], 0)), Some(PlayerId(1)));
        app.insert_resource(ContentRes(Arc::new(db)));
        let map_planet = app.world_mut().spawn(SimEntity(EntityId(2))).id();
        let map_ship = app.world_mut().spawn(SimEntity(EntityId(3))).id();
        for _ in 0..5 {
            app.update();
        }
        assert!(app.world().get_entity(map_planet).is_ok(), "the map's planet entity was despawned");
        assert_eq!(app.world().resource::<SimEntities>().0.get(&EntityId(2)), Some(&map_planet));
        assert_eq!(roots(&mut app).iter().filter(|r| r.0 == EntityId(2)).count(), 1, "a second entity was made for the planet");
        assert!(app.world().get_entity(map_ship).is_err(), "an unseen neutral ship stays drawn");
    }

    /// An A2 `planet` re-created from the view (the map view draws its body, so attach_models skips it) gets `NoIcon`
    /// before that skip, and `ensure_icons` leaves it without an arrow; a ship of the same kind of entity does get one.
    #[test]
    fn a_view_drawn_a2_planet_gets_no_icon_even_though_it_has_no_model() {
        use bevy::ecs::system::RunSystemOnce;
        let db = sta_content::ContentDb::builder().unit(sta_content::UnitDef::new("t_planet", "planet")).build().unwrap();
        let key = db.key("t_planet").unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<Image>()
            .insert_resource(Mounted(sta_vfs::Vfs::default()))
            .insert_resource(ContentRes(Arc::new(db)))
            .insert_resource(SpriteTables(Arc::new(sta_assets::a2fmt::spr::SpriteTable::default())))
            .init_resource::<crate::render::moons::MoonVariants>()
            .init_resource::<TextureCache>()
            .init_resource::<PartCache>()
            .init_resource::<UnitModels>()
            .insert_resource(SimLink::new(Box::new(Views(vec![view(0, Vec::new(), Vec::new())], 0)), Box::new(LocalRouter::new()), PlayerId(1)));
        let planet = app.world_mut().spawn((FromView, SimEntity(EntityId(2)), NeedsModel(key), TeamColour(Color::WHITE))).id();
        app.world_mut().run_system_once(attach_models).unwrap();
        app.world_mut().flush();
        app.world_mut().run_system_once(crate::render::strategic::ensure_icons).unwrap();
        app.world_mut().flush();
        assert!(app.world().get::<crate::render::strategic::NoIcon>(planet).is_some());
        assert!(app.world().get::<crate::render::strategic::Strategic>(planet).is_none());
    }
}
