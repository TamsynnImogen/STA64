//! The action camera (UI-53): the live 3D view of the Armada I stock HUD's cinematic window.
//!
//! What the Armada I manual says of the window (read from data, the install's manual): it shows what
//! happens elsewhere on the map; one click on it goes to the event, a double click switches the
//! main view to the cinematic action view, `Z` returns. The picture seen
//! in one frame of the original: a close, oblique view of the selected ship against the sky, no
//! health bar, no selection ring, no grid.
//!
//! What is implemented here, and its grade:
//! - A second camera pair renders the scene into [`ActionView::image`]; the HUD shows it inside
//!   `cinematicDisplayArea` (`ui::hud::stock::a1::action`). The pair is a sky camera (the sky is a
//!   direction-only picture, so it sits at the main camera's place with the action camera's
//!   orientation) and a scene camera. Neither is the map's [`crate::map::MainCamera`].
//! - What the window shows (a player of the originals says: the original games choose it this way;
//!   **stated by a player of the originals, not measured**): a fight the local player's ships are
//!   in, else a local construction ship that is building, else the local player's idle ships one
//!   after another, moving on from scene to scene over time. See [`pick`]. Not measured, plain
//!   values chosen here: the dwell time per scene ([`DWELL_SECS`]), the order within a tier (by
//!   unit id), pre-emption by a higher tier at once (without waiting for the dwell), framing a
//!   fight by the one local unit in it rather than the group, how long after a shot a fight lasts
//!   ([`FIGHT_SECS`]), and whether the selection plays a part (here it plays none). Only the
//!   local player's own units, which the player can see, qualify. With nothing to show the camera
//!   keeps its last pose.
//! - The framing, a fixed oblique angle at a distance that fits the unit's selection radius, is
//!   **not measured** either, and so is how the original moves the camera.
//! - Selection rings, hull and shield bars (the [`Overlay`] pieces), the strategic view's height bars (gizmos)
//!   and the map grid (`render::map_grid`) are on [`OVERLAY_LAYER`], which only the main camera draws: seen in
//!   one frame that the original's window has none of them.
//! - Not drawn in the window: nebulae (their march belongs to the main camera) and, while the
//!   main view shows a ship as a symbol (zoomed far out), its model, because the model is hidden
//!   by that handoff for every camera. Billboarded effects face the main camera. All **not
//!   measured** against the original; the fog layer is not treated specially.

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use sta_sim::EntityId;

use crate::map::{BackgroundCamera, MatchEntity};
use crate::clock::AnimClock;
use crate::render::overlays::Overlay;
use crate::render::units::SelectRadius;
use crate::render::units::LocalPlayer;
use crate::sim_link::{ContentRes, SimEntities, SimLink, SimMsg};
use sta_sim::event::SimEvent;
use sta_sim::view::UnitFlags;
use sta_sim::world::EntityKind;
use sta_sim::command::Target;
use sta_sim::Fixed;
use std::collections::{BTreeMap, BTreeSet};

/// The layer of what only the map's own camera draws: selection rings and bars, the strategic view's bars, the map grid.
pub const OVERLAY_LAYER: usize = 3;

/// Vertical field of view of the action camera (the map camera's default).
const FOV: f32 = std::f32::consts::FRAC_PI_4;
/// Where the camera looks from, relative to the subject: yaw and pitch of a fixed oblique angle.
/// Not measured.
const YAW: f32 = 0.6;
const PITCH: f32 = 0.45;
/// The distance in units of the selection radius, so that the whole unit fits with a margin.
/// Not measured.
const FIT: f32 = 1.5;
/// Smallest and largest picture the camera renders, in pixels per side.
const MIN_SIDE: u32 = 16;
const MAX_SIDE: u32 = 2048;

/// The picture the action camera renders into.
#[derive(Resource, Clone)]
pub struct ActionView {
    pub image: Handle<Image>,
}

/// The node that shows [`ActionView::image`]; its size decides the picture's.
#[derive(Component)]
pub struct ActionPicture;

/// The scene camera of the action view (not the map's [`crate::map::MainCamera`]).
#[derive(Component)]
pub struct ActionCamera;

/// The unit the action camera looks at.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSubject(pub Option<EntityId>);

/// Seconds one scene stays before the next of its tier. Not measured.
pub const DWELL_SECS: f32 = 8.0;
/// Seconds after a shot fired or taken that a unit counts as in a fight. Not measured.
pub const FIGHT_SECS: f32 = 6.0;

/// The three kinds of scene, in priority order (stated by a player of the originals, not measured).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// A fight the local player's ships are in.
    Fight,
    /// A local construction ship building something.
    Build,
    /// An idle local ship.
    Idle,
}

/// The local player's units that qualify for each tier, sorted by id (for the picker).
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct ActionCandidates {
    pub fight: Vec<EntityId>,
    pub build: Vec<EntityId>,
    pub idle: Vec<EntityId>,
}

impl ActionCandidates {
    fn of(&self, tier: Tier) -> &[EntityId] {
        match tier {
            Tier::Fight => &self.fight,
            Tier::Build => &self.build,
            Tier::Idle => &self.idle,
        }
    }
}

/// The scene to show: the best tier with a candidate. The current scene stays while it still
/// qualifies and nothing better has appeared, until `held` seconds reach [`DWELL_SECS`], then the
/// next unit of its tier (by id, wrapping) follows. A better tier pre-empts at once. A scene that
/// no longer qualifies is replaced at once, by the next unit after it in the best tier.
pub fn pick(c: &ActionCandidates, current: Option<(Tier, EntityId)>, held: f32) -> Option<(Tier, EntityId)> {
    let best = [Tier::Fight, Tier::Build, Tier::Idle].into_iter().find(|&t| !c.of(t).is_empty())?;
    let list = c.of(best);
    let after = |id: EntityId| list.iter().copied().find(|&x| x > id).unwrap_or(list[0]);
    match current {
        Some((tier, id)) if tier == best => {
            if list.contains(&id) {
                Some((best, if held >= DWELL_SECS { after(id) } else { id }))
            } else {
                Some((best, after(id)))
            }
        }
        _ => Some((best, list[0])),
    }
}

/// The local units a sim event puts in a fight: whoever fired, was fired at, hit or hit.
pub fn fighters(ev: &SimEvent) -> Vec<EntityId> {
    match ev {
        SimEvent::Fired { shooter, target, .. } => {
            let mut v = vec![*shooter];
            if let Target::Entity(t) = target {
                v.push(*t);
            }
            v
        }
        SimEvent::Hit { target, by, .. } => vec![*target, *by],
        _ => Vec::new(),
    }
}

/// When each unit last fired or was hit (seconds of the app clock); filtered by owner later.
#[derive(Resource, Default, Debug)]
pub struct FightLog(pub BTreeMap<EntityId, f32>);

/// The scene being shown and since when.
#[derive(Resource, Default)]
struct ActionScene {
    current: Option<(Tier, EntityId)>,
    since: f32,
}

/// Where the camera stands and what it looks at, for a subject at `at` of selection radius `radius`.
pub fn frame(at: Vec3, radius: f32) -> Transform {
    let dist = radius.max(1.0) * FIT / (FOV * 0.5).sin();
    let rot = Quat::from_rotation_y(YAW) * Quat::from_rotation_x(-PITCH);
    let eye = at + rot * Vec3::new(0.0, 0.0, dist);
    Transform::from_translation(eye).looking_at(at, Vec3::Y)
}

/// The side of the picture for a node of `size` physical pixels.
pub fn picture_size(size: Vec2) -> UVec2 {
    let side = |v: f32| (v.round() as u32).clamp(MIN_SIDE, MAX_SIDE);
    UVec2::new(side(size.x), side(size.y))
}

#[derive(Component)]
pub struct ActionSky;

/// Measurement hook of the performance runs (`bench/perf.rs`): a headless app has no HUD, so
/// this stands in for the visible picture node (its size) and says which cameras are on.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ActionBench {
    pub size: Vec2,
    pub sky: bool,
    pub scene: bool,
}

pub struct ActionCameraPlugin;

impl Plugin for ActionCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActionSubject>()
            .init_resource::<ActionCandidates>()
            .init_resource::<ActionScene>()
            .init_resource::<FightLog>()
            .add_message::<SimMsg>()
            .add_systems(Startup, make_view)
            .add_systems(PostUpdate, grid_layer.run_if(resource_exists::<bevy::gizmos::config::GizmoConfigStore>))
            .add_systems(
                PostUpdate,
                (layer_overlays, (log_fights, gather.run_if(picture_wanted)).chain().run_if(resource_exists::<SimLink>), subject, sync_cameras)
                    .chain()
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
    }
}

/// Whether a picture of the action view can be seen (or a perf run stands in for one): the
/// candidates are gathered and the cameras run only then, so a profile with no such window pays nothing.
fn picture_wanted(pictures: Query<&InheritedVisibility, With<ActionPicture>>, bench: Option<Res<ActionBench>>) -> bool {
    bench.is_some() || pictures.iter().any(|v| v.get())
}

/// The strategic view's height bars are gizmos: they go to [`OVERLAY_LAYER`] too (once).
fn grid_layer(mut store: ResMut<bevy::gizmos::config::GizmoConfigStore>, mut done: Local<bool>) {
    if !*done {
        store.config_mut::<bevy::gizmos::config::DefaultGizmoConfigGroup>().0.render_layers = RenderLayers::layer(OVERLAY_LAYER);
        *done = true;
    }
}

fn make_view(mut commands: Commands, images: Option<ResMut<Assets<Image>>>) {
    let Some(mut images) = images else { return };
    let mut image = Image::new_target_texture(MIN_SIDE, MIN_SIDE, TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    // A resized picture is drawn afresh every frame, so the old texture is not worth copying; the
    // copy also needs `COPY_SRC`, which a render target made here does not have, and the failed
    // copy is a validation error that makes the renderer quit.
    image.copy_on_resize = false;
    commands.insert_resource(ActionView { image: images.add(image) });
}

/// Rings, bars and lines of the selection overlays go to [`OVERLAY_LAYER`], with their children.
fn layer_overlays(mut commands: Commands, added: Query<Entity, Added<Overlay>>) {
    for e in &added {
        // `Propagate` reaches the descendants only: the piece itself needs the layer too.
        commands.entity(e).try_insert((RenderLayers::layer(OVERLAY_LAYER), bevy::app::Propagate(RenderLayers::layer(OVERLAY_LAYER))));
    }
}

/// The animation clock's time in seconds (it stands while the sim is paused, so does the cycling).
fn anim_now(clock: Option<&AnimClock>) -> f32 {
    clock.map_or(0.0, |c| c.now() as f32)
}

/// Note every unit a shot touched.
fn log_fights(mut msgs: MessageReader<SimMsg>, clock: Option<Res<AnimClock>>, mut log: ResMut<FightLog>) {
    let now = anim_now(clock.as_deref());
    for SimMsg(ev) in msgs.read() {
        for id in fighters(ev) {
            log.0.insert(id, now);
        }
    }
    // Trimmed here, not in `gather`, which runs only while a picture shows.
    trim_log(&mut log.0, now);
}

/// Forget units that last fought more than [`FIGHT_SECS`] before `now`.
fn trim_log(log: &mut BTreeMap<EntityId, f32>, now: f32) {
    log.retain(|_, t| now - *t <= FIGHT_SECS);
}

/// The local player's visible, living units for each tier, from the client's view of the world.
fn gather(link: Res<SimLink>, local: Option<Res<LocalPlayer>>, content: Option<Res<ContentRes>>, clock: Option<Res<AnimClock>>, mut log: ResMut<FightLog>, mut out: ResMut<ActionCandidates>) {
    let now = anim_now(clock.as_deref());
    trim_log(&mut log.0, now);
    let player = local.and_then(|l| l.0).unwrap_or(link.player());
    let view = link.curr();
    let craft = |def| content.as_ref().and_then(|c| c.0.unit(def)).is_none_or(|d| EntityKind::of_def(d) == EntityKind::Craft);
    let own: BTreeSet<EntityId> = view
        .visible_to(player)
        .into_iter()
        .filter(|u| u.owner == player && !u.flags.contains(UnitFlags::DYING) && craft(u.def))
        .map(|u| u.id)
        .collect();
    let fight: BTreeSet<EntityId> = log.0.keys().copied().filter(|id| own.contains(id)).collect();
    let build: BTreeSet<EntityId> = link
        .production()
        .iter()
        .filter(|r| own.contains(&r.builder) && r.jobs.first().zip(r.site.as_ref()).is_some_and(|(j, s)| j.def == s.def))
        .map(|r| r.builder)
        .collect();
    let idle: BTreeSet<EntityId> = own
        .iter()
        .copied()
        .filter(|id| view.unit(*id).is_some_and(|u| u.speed == Fixed::ZERO && u.progress.is_none() && !u.flags.contains(UnitFlags::BUILDING)))
        .collect();
    let next = ActionCandidates { fight: fight.into_iter().collect(), build: build.into_iter().collect(), idle: idle.into_iter().collect() };
    if *out != next {
        *out = next;
    }
}

/// Choose the subject from the candidates: [`pick`], re-evaluated every frame.
fn subject(candidates: Res<ActionCandidates>, entities: Option<Res<SimEntities>>, clock: Option<Res<AnimClock>>, mut scene: ResMut<ActionScene>, mut subject: ResMut<ActionSubject>, mut log: ResMut<FightLog>) {
    // A match ended (its entities are gone): no id carries into the next one.
    if entities.as_ref().is_none_or(|e| e.0.is_empty()) {
        if subject.0.is_some() {
            subject.0 = None;
        }
        scene.current = None;
        log.0.clear();
        return;
    }
    let now = anim_now(clock.as_deref());
    let next = pick(&candidates, scene.current, now - scene.since);
    if next != scene.current {
        scene.current = next;
        scene.since = now;
    }
    let id = next.map(|(_, id)| id);
    if subject.0 != id {
        subject.0 = id;
    }
}

/// Keep the camera pair while the match runs, switched on only while the HUD shows the picture, and aim it at the subject.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn sync_cameras(
    mut commands: Commands,
    view: Option<Res<ActionView>>,
    mut images: ResMut<Assets<Image>>,
    pictures: Query<(&ComputedNode, &InheritedVisibility), With<ActionPicture>>,
    subject: Res<ActionSubject>,
    entities: Option<Res<SimEntities>>,
    bodies: Query<(&GlobalTransform, Option<&SelectRadius>), (Without<ActionCamera>, Without<ActionSky>)>,
    main: Query<(&GlobalTransform, &Projection), (crate::map::MainCamera, Without<ActionSky>)>,
    mut scene: Query<(&mut Transform, &mut GlobalTransform, &mut Camera), (With<ActionCamera>, Without<ActionSky>)>,
    mut sky: Query<(&mut Transform, &mut GlobalTransform, &mut Camera), (With<ActionSky>, Without<ActionCamera>)>,
    bench: Option<Res<ActionBench>>,
    menu: Option<Res<crate::ui::ingame_menu::InGameMenu>>,
    movies: Option<Res<crate::media::Movies>>,
) {
    // Only while the picture can be seen: a hidden match UI (loading, a cineractive) costs no passes.
    // Not drawn while the in-game menu is over it or a mission clip plays in the panel: nobody sees it.
    let covered = bench.is_none() && (menu.is_some_and(|m| m.open) || movies.is_some_and(|m| m.panel_busy()));
    let size = pictures.iter().filter(|(_, v)| v.get() && !covered).map(|(n, _)| n.size()).find(|s| s.x > 0.0 && s.y > 0.0).or(bench.as_ref().map(|b| b.size));
    let (Some(view), Some(size), Ok((main_tf, projection))) = (view, size, main.single()) else {
        // The cameras stay, switched off: showing the picture again costs neither a black frame
        // nor a new target.
        for (.., mut cam) in scene.iter_mut().chain(sky.iter_mut()) {
            if cam.is_active {
                cam.is_active = false;
            }
        }
        return;
    };
    for (.., mut cam) in scene.iter_mut() {
        let on = bench.as_ref().is_none_or(|b| b.scene);
        if cam.is_active != on {
            cam.is_active = on;
        }
    }
    for (.., mut cam) in sky.iter_mut() {
        let on = bench.as_ref().is_none_or(|b| b.sky);
        if cam.is_active != on {
            cam.is_active = on;
        }
    }
    let want = picture_size(size);
    if let Some(img) = images.get(&view.image)
        && (img.width(), img.height()) != (want.x, want.y)
        && let Some(mut img) = images.get_mut(&view.image)
    {
        img.resize(bevy::render::render_resource::Extent3d { width: want.x, height: want.y, depth_or_array_layers: 1 });
    }
    let target = subject.0.and_then(|id| entities.as_ref()?.0.get(&id)).and_then(|&e| bodies.get(e).ok());
    let mut pose = target.map(|(gt, radius)| frame(gt.translation(), radius.map_or(20.0, |r| r.0)));
    if pose.is_none() && bench.is_some() {
        // Measurement only: look at the ground point the map camera looks at.
        let f = main_tf.forward().as_vec3();
        if f.y < -0.01 {
            pose = Some(frame(main_tf.translation() + f * (-main_tf.translation().y / f.y), 30.0));
        }
    }
    if scene.is_empty() {
        let projection = match projection {
            Projection::Perspective(p) => Projection::Perspective(PerspectiveProjection { fov: FOV, ..p.clone() }),
            other => other.clone(),
        };
        let eye = pose.unwrap_or_else(|| Transform::from_translation(main_tf.translation()));
        commands.spawn((
            MatchEntity,
            ActionCamera,
            Camera3d::default(),
            // A small picture: no multisampling (the quality settings skip these cameras). The post
            // writer gives this camera the main view's tonemapper (none in Stock) and nothing else.
            Msaa::Off,
            // No dithering: it would show as noise in a few hundred pixels.
            bevy::core_pipeline::tonemapping::DebandDither::Disabled,
            Camera { order: -9, clear_color: ClearColorConfig::None, ..default() },
            RenderTarget::from(view.image.clone()),
            projection.clone(),
            eye,
            GlobalTransform::from(eye),
        ));
        commands.spawn((
            MatchEntity,
            ActionSky,
            BackgroundCamera,
            Camera3d::default(),
            Msaa::Off,
            bevy::core_pipeline::tonemapping::DebandDither::Disabled,
            Camera { order: -10, ..default() },
            RenderTarget::from(view.image.clone()),
            RenderLayers::layer(crate::map::BACKGROUND_LAYER),
            projection,
            eye,
            GlobalTransform::from(eye),
        ));
        return;
    }
    // With no subject the camera keeps its last pose.
    if let Some(pose) = pose {
        // `GlobalTransform` too: this runs after propagation, and the render of this frame reads it.
        for (mut t, mut g, _) in &mut scene {
            *t = pose;
            *g = GlobalTransform::from(pose);
        }
        // The sky is a direction-only picture drawn from the map camera's place.
        for (mut t, mut g, _) in &mut sky {
            *t = Transform { translation: main_tf.translation(), ..pose };
            *g = GlobalTransform::from(*t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[u32]) -> Vec<EntityId> {
        v.iter().map(|&i| EntityId(i)).collect()
    }
    fn cands(fight: &[u32], build: &[u32], idle: &[u32]) -> ActionCandidates {
        ActionCandidates { fight: ids(fight), build: ids(build), idle: ids(idle) }
    }

    #[test]
    fn each_tier_is_chosen_in_priority_order() {
        assert_eq!(pick(&cands(&[5], &[3], &[1]), None, 0.0), Some((Tier::Fight, EntityId(5))));
        assert_eq!(pick(&cands(&[], &[3, 4], &[1]), None, 0.0), Some((Tier::Build, EntityId(3))));
        assert_eq!(pick(&cands(&[], &[], &[2, 7]), None, 0.0), Some((Tier::Idle, EntityId(2))));
    }

    #[test]
    fn a_higher_tier_pre_empts_at_once_and_the_scene_falls_back_down_when_it_ends() {
        let idle_now = Some((Tier::Idle, EntityId(1)));
        assert_eq!(pick(&cands(&[], &[3], &[1]), idle_now, 0.1), Some((Tier::Build, EntityId(3))), "a build pre-empts an idle scene");
        let building = Some((Tier::Build, EntityId(3)));
        assert_eq!(pick(&cands(&[9], &[3], &[1]), building, 0.1), Some((Tier::Fight, EntityId(9))), "a fight pre-empts a build");
        let fight = Some((Tier::Fight, EntityId(9)));
        assert_eq!(pick(&cands(&[], &[3], &[1]), fight, 0.1), Some((Tier::Build, EntityId(3))), "the fight ends: back to the build");
        assert_eq!(pick(&cands(&[], &[], &[1]), building, 0.1), Some((Tier::Idle, EntityId(1))), "the build ends: back to idle");
    }

    #[test]
    fn idle_ships_are_cycled_after_the_dwell_in_id_order_and_wrap() {
        let c = cands(&[], &[], &[2, 5, 9]);
        let at = |id, held| pick(&c, Some((Tier::Idle, EntityId(id))), held);
        assert_eq!(at(2, DWELL_SECS - 0.1), Some((Tier::Idle, EntityId(2))), "held until the dwell is over");
        assert_eq!(at(2, DWELL_SECS), Some((Tier::Idle, EntityId(5))));
        assert_eq!(at(5, DWELL_SECS), Some((Tier::Idle, EntityId(9))));
        assert_eq!(at(9, DWELL_SECS), Some((Tier::Idle, EntityId(2))), "wraps");
        let one = cands(&[], &[], &[4]);
        assert_eq!(pick(&one, Some((Tier::Idle, EntityId(4))), 99.0), Some((Tier::Idle, EntityId(4))), "a single ship stays");
    }

    #[test]
    fn a_scene_that_stops_qualifying_is_replaced_at_once_by_the_next_unit() {
        let c = cands(&[], &[], &[2, 9]);
        assert_eq!(pick(&c, Some((Tier::Idle, EntityId(5))), 0.0), Some((Tier::Idle, EntityId(9))), "5 is gone, 9 follows");
        let c = cands(&[], &[], &[2, 5]);
        assert_eq!(pick(&c, Some((Tier::Idle, EntityId(9))), 0.0), Some((Tier::Idle, EntityId(2))), "9 left sight, wraps");
        let f = cands(&[6, 8], &[], &[2]);
        assert_eq!(pick(&f, Some((Tier::Fight, EntityId(6))), DWELL_SECS), Some((Tier::Fight, EntityId(8))), "fights cycle too");
    }

    #[test]
    fn nothing_is_chosen_when_the_player_has_no_units() {
        assert_eq!(pick(&cands(&[], &[], &[]), Some((Tier::Idle, EntityId(1))), 0.0), None);
        assert_eq!(pick(&cands(&[], &[], &[]), None, 0.0), None);
    }

    #[test]
    fn the_fight_log_forgets_old_fights_without_a_picture() {
        let mut log = BTreeMap::from([(EntityId(1), 0.0), (EntityId(2), 10.0)]);
        trim_log(&mut log, FIGHT_SECS + 1.0);
        assert_eq!(log.keys().copied().collect::<Vec<_>>(), vec![EntityId(2)]);
        // `log_fights` itself (the system that runs in every profile) calls it.
        let mut app = App::new();
        app.add_message::<SimMsg>().init_resource::<FightLog>().add_systems(Update, log_fights);
        app.world_mut().resource_mut::<FightLog>().0.insert(EntityId(9), -100.0);
        app.world_mut().write_message(SimMsg(SimEvent::Hit { target: EntityId(2), by: EntityId(7), weapon: sta_sim::ContentKey::NONE, facing: 0, shield: Fixed::ZERO, hull: Fixed::ZERO, at: Default::default() }));
        app.update();
        let keys: Vec<_> = app.world().resource::<FightLog>().0.keys().copied().collect();
        assert_eq!(keys, vec![EntityId(2), EntityId(7)], "the old entry is gone, the new ones stay");
    }

    #[test]
    fn shots_put_both_ends_in_a_fight() {
        let hit = SimEvent::Hit { target: EntityId(2), by: EntityId(7), weapon: sta_sim::ContentKey::NONE, facing: 0, shield: Fixed::ZERO, hull: Fixed::ZERO, at: Default::default() };
        assert_eq!(fighters(&hit), vec![EntityId(2), EntityId(7)]);
        let shot = SimEvent::Fired { shooter: EntityId(3), weapon: sta_sim::ContentKey::NONE, hardpoint: 0, target: Target::Entity(EntityId(4)) };
        assert_eq!(fighters(&shot), vec![EntityId(3), EntityId(4)]);
        let at_a_point = SimEvent::Fired { shooter: EntityId(3), weapon: sta_sim::ContentKey::NONE, hardpoint: 0, target: Target::None };
        assert_eq!(fighters(&at_a_point), vec![EntityId(3)]);
        assert!(fighters(&SimEvent::Spawned { id: EntityId(1) }).is_empty());
    }

    #[test]
    fn the_frame_looks_at_the_subject_from_a_distance_that_grows_with_its_radius() {
        let at = Vec3::new(100.0, 5.0, -40.0);
        let near = frame(at, 10.0);
        let far = frame(at, 40.0);
        assert!((near.forward().as_vec3().dot((at - near.translation).normalize()) - 1.0).abs() < 1e-4);
        assert!((far.translation - at).length() > 3.9 * (near.translation - at).length());
        assert!(near.translation.y > at.y, "from above");
    }

    /// #1073: the window is quiet (its cameras switched off) while a mission clip waits for or plays
    /// in the movie panel, which shares the window's layout keys, and wakes when the clip is over.
    #[test]
    fn the_cameras_go_quiet_while_a_clip_plays_in_the_panel() {
        use crate::media::{MovieId, MovieTarget, Movies, PlayMovie};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .init_resource::<ActionSubject>()
            .init_resource::<Movies>()
            .insert_resource(ActionView { image: Handle::default() })
            .add_systems(Update, sync_cameras);
        app.world_mut().spawn((Camera3d::default(), Transform::default(), GlobalTransform::default(), Projection::default()));
        app.world_mut().spawn((ActionPicture, ComputedNode { size: Vec2::new(200.0, 100.0), ..default() }, InheritedVisibility::VISIBLE));
        let active = |app: &mut App| -> Vec<bool> {
            app.world_mut().query_filtered::<&Camera, Or<(With<ActionCamera>, With<ActionSky>)>>().iter(app.world()).map(|c| c.is_active).collect()
        };
        app.update();
        app.update();
        assert_eq!(active(&mut app), [true, true], "the picture is seen: the pair runs");
        let target = MovieTarget::Panel(Handle::default());
        app.world_mut().resource_mut::<Movies>().queue_for_test(PlayMovie { id: MovieId(1), name: String::new(), target, skippable: false });
        app.update();
        assert_eq!(active(&mut app), [false, false], "a clip in the panel: quiet");
        app.world_mut().resource_mut::<Movies>().stop_for_test();
        app.update();
        assert_eq!(active(&mut app), [true, true], "the clip is over: the picture runs again");
    }

    #[test]
    fn the_picture_follows_the_node_and_stays_within_bounds() {
        assert_eq!(picture_size(Vec2::new(226.4, 160.0)), UVec2::new(226, 160));
        assert_eq!(picture_size(Vec2::new(1.0, 99999.0)), UVec2::new(MIN_SIDE, MAX_SIDE));
    }
}

#[cfg(test)]
#[path = "action_camera_gpu.rs"]
mod gpu;
