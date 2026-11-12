//! RND-08: selection overlays on the map.
//!
//! For every selected unit that has a root in [`SimEntities`]:
//! - a team-coloured ring on the ground, sized by [`SelectRadius`];
//! - a hull bar and (when the unit has shields) a shield bar above it,
//!   billboarded to the camera, filled by [`HullShare`] and the view's
//!   shield pools;
//! - a line to its move target when [`OrderTargets`] has one. The views do
//!   not carry queued waypoints (SIM-02), so only that one target is drawn.
//!
//! Rings and bars are children of the unit root, so they go with it when
//! the unit is destroyed; [`sync_overlays`] despawns the rest when the
//! selection changes. [`OverlayCounts`] says how many of each exist (for
//! bench marks, once TOOL-07 lands). Refit's own overlays are not in this
//! repo; this follows the work package, not a port.

use std::collections::{BTreeMap, BTreeSet};

mod geometry;
#[cfg(test)]
mod gpu;
pub use geometry::{GaugeStyle, ModelBounds};

use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use sta_sim::{EntityId, UnitView};

use crate::app::GameSet;
use crate::input::orders::OrderTargets;
use crate::render::units::{HullShare, SelectRadius, TeamColour, to_bevy};
use crate::sim_link::{Selection, SimEntities, SimLink};

/// Radius used for units without a [`SelectRadius`].
const DEFAULT_RADIUS: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayKind {
    Ring,
    HullBar,
    ShieldBar,
    Line,
}

/// Marks the root entity of one overlay piece for sim unit `unit`.
#[derive(Component, Debug, Clone, Copy)]
pub struct Overlay {
    pub unit: EntityId,
    pub kind: OverlayKind,
}

/// The moving part of a bar: `share` is what it shows, 0..1.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct BarFill {
    pub share: f32,
}

/// A bar root, turned to face the camera each frame.
#[derive(Component)]
struct Billboard;

/// How many overlay pieces exist, refreshed every frame.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OverlayCounts {
    pub rings: usize,
    pub bars: usize,
    pub lines: usize,
}

#[derive(Resource)]
struct OverlayAssets {
    ring: Handle<Mesh>,
    quad: Handle<Mesh>,
    line_mesh: Handle<Mesh>,
    bar_back: Handle<StandardMaterial>,
    hull: Handle<StandardMaterial>,
    shield: Handle<StandardMaterial>,
    line: Handle<StandardMaterial>,
}

pub struct OverlaysPlugin;

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .init_resource::<GaugeStyle>()
            .init_resource::<OrderTargets>()
            .init_resource::<OverlayCounts>()
            .add_systems(Startup, make_assets.run_if(resource_exists::<Assets<Mesh>>).run_if(resource_exists::<Assets<StandardMaterial>>))
            .add_systems(
                Update,
                (sync_overlays, update_bars, update_lines, billboard.after(crate::viewer::orbit_camera), count)
                    .chain()
                    .in_set(GameSet::Render)
                    .distributive_run_if(resource_exists::<SimLink>)
                    .distributive_run_if(resource_exists::<OverlayAssets>),
            );
    }
}

fn unlit(colour: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: colour,
        unlit: true,
        cull_mode: None,
        alpha_mode: if colour.alpha() < 1.0 { AlphaMode::Blend } else { AlphaMode::Opaque },
        ..default()
    }
}

fn make_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(OverlayAssets {
        ring: meshes.add(Annulus::new(0.96, 1.0)),
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        line_mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        bar_back: mats.add(unlit(Color::srgba(0.05, 0.05, 0.05, 0.8))),
        hull: mats.add(unlit(Color::srgb(0.2, 0.9, 0.3))),
        shield: mats.add(unlit(Color::srgb(0.3, 0.6, 1.0))),
        line: mats.add(unlit(Color::srgba(0.3, 1.0, 0.4, 0.4))),
    });
}

/// All four facings' shields now over max, or `None` without shields.
pub fn shield_share(u: &UnitView) -> Option<f32> {
    let (now, max) = u.shields.iter().fold((0.0, 0.0), |(n, m), s| (n + s[0].to_f32(), m + s[1].to_f32()));
    (max > 0.0).then(|| (now / max).clamp(0.0, 1.0))
}

/// Temporary world geometry before the camera/model bounds are available.
fn bar_geometry(radius: f32) -> (f32, f32, f32) {
    (radius * 2.0, 0.4, radius + 3.0)
}

#[allow(clippy::too_many_arguments)]
fn sync_overlays(
    mut commands: Commands,
    selection: Res<Selection>,
    ents: Res<SimEntities>,
    link: Res<SimLink>,
    targets: Res<OrderTargets>,
    assets: Res<OverlayAssets>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    units: Query<(Option<&SelectRadius>, Option<&TeamColour>, Option<&HullShare>)>,
    existing: Query<(Entity, &Overlay)>,
) {
    let mut want: BTreeMap<(EntityId, OverlayKind), Entity> = BTreeMap::new();
    for id in &selection.0 {
        let Some(&root) = ents.0.get(id) else { continue };
        if units.get(root).is_err() {
            continue;
        }
        want.insert((*id, OverlayKind::Ring), root);
        want.insert((*id, OverlayKind::HullBar), root);
        if link.curr().unit(*id).and_then(shield_share).is_some() {
            want.insert((*id, OverlayKind::ShieldBar), root);
        }
        if targets.0.contains_key(id) {
            want.insert((*id, OverlayKind::Line), root);
        }
    }
    let mut have: BTreeSet<(EntityId, OverlayKind)> = BTreeSet::new();
    for (e, o) in &existing {
        if !(want.contains_key(&(o.unit, o.kind)) && have.insert((o.unit, o.kind))) {
            commands.entity(e).try_despawn();
        }
    }
    for ((id, kind), root) in want {
        if have.contains(&(id, kind)) {
            continue;
        }
        let Ok((radius, colour, hull)) = units.get(root) else { continue };
        let radius = radius.map_or(DEFAULT_RADIUS, |r| r.0);
        let overlay = Overlay { unit: id, kind };
        match kind {
            OverlayKind::Ring => {
                let colour = colour.map_or(Color::srgb(0.3, 1.0, 0.4), |c| c.0);
                // Annulus faces +Z; lay it on the ground.
                commands.spawn((
                    Name::new("selection ring"),
                    overlay,
                    ChildOf(root),
                    Mesh3d(assets.ring.clone()),
                    MeshMaterial3d(mats.add(unlit(colour))),
                    Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)).with_scale(Vec3::splat(radius)),
                ));
            }
            OverlayKind::HullBar | OverlayKind::ShieldBar => {
                let (w, h, y) = bar_geometry(radius);
                let (mat, share, y) = if kind == OverlayKind::HullBar {
                    (assets.hull.clone(), hull.map_or(1.0, |s| s.0), y)
                } else {
                    (assets.shield.clone(), 1.0, y + h * 1.5)
                };
                let bar = commands.spawn((Name::new("bar"), overlay, Billboard, ChildOf(root), Transform::from_xyz(0.0, y, 0.0).with_scale(Vec3::new(w, h, 1.0)), Visibility::Hidden)).id();
                commands.spawn((ChildOf(bar), Mesh3d(assets.quad.clone()), MeshMaterial3d(assets.bar_back.clone()), Transform::default()));
                commands.spawn((
                    ChildOf(bar),
                    BarFill { share },
                    Mesh3d(assets.quad.clone()),
                    MeshMaterial3d(mat),
                    Transform::from_xyz(-(1.0 - share) * 0.5, 0.0, 0.01).with_scale(Vec3::new(share, 1.0, 1.0)),
                ));
            }
            OverlayKind::Line => {
                commands.spawn((Name::new("target line"), overlay, Mesh3d(assets.line_mesh.clone()), MeshMaterial3d(assets.line.clone()), Transform::default()));
            }
        }
    }
}

/// Set each fill from the hull share or the view's shields.
fn update_bars(
    link: Res<SimLink>,
    hulls: Query<&HullShare>,
    ents: Res<SimEntities>,
    bars: Query<(&Overlay, &Children)>,
    mut fills: Query<(&mut BarFill, &mut Transform)>,
) {
    for (o, kids) in &bars {
        let share = match o.kind {
            OverlayKind::HullBar => ents.0.get(&o.unit).and_then(|e| hulls.get(*e).ok()).map(|h| h.0),
            OverlayKind::ShieldBar => link.curr().unit(o.unit).and_then(shield_share),
            _ => continue,
        };
        let Some(share) = share else { continue };
        for k in kids.iter() {
            if let Ok((mut f, mut tf)) = fills.get_mut(k) {
                f.share = share;
                tf.translation.x = -(1.0 - share) * 0.5;
                tf.scale = Vec3::new(share, 1.0, 1.0);
            }
        }
    }
}

/// Stretch each line from its unit to the unit's move target.
fn update_lines(ents: Res<SimEntities>, targets: Res<OrderTargets>, units: Query<&GlobalTransform>, mut lines: Query<(&Overlay, &mut Transform)>) {
    for (o, mut tf) in &mut lines {
        if o.kind != OverlayKind::Line {
            continue;
        }
        let (Some(from), Some(to)) = (ents.0.get(&o.unit).and_then(|e| units.get(*e).ok()), targets.0.get(&o.unit)) else { continue };
        let (a, b) = (from.translation(), to_bevy(to.to_f32()));
        let d = b - a;
        let len = d.length();
        if len < 1e-3 {
            continue;
        }
        *tf = Transform::from_translation((a + b) * 0.5).looking_to(d / len, Vec3::Y).with_scale(Vec3::new(0.3, 0.3, len));
    }
}

/// Project the model container, then keep gauge height/spacing in screen pixels.
#[allow(clippy::type_complexity)]
fn billboard(
    cam: Query<(Entity, &Camera, &Projection), crate::map::MainCamera>,
    parents: Query<(Option<&ModelBounds>, Option<&SelectRadius>), Without<Billboard>>,
    style: Res<GaugeStyle>,
    metadata: Query<(Entity, &Overlay, &ChildOf), With<Billboard>>,
    mut transforms: ParamSet<(TransformHelper, Query<(&mut Transform, &mut Visibility), With<Billboard>>)>,
    mut updates: Local<Vec<(Entity, Option<Transform>)>>,
) {
    updates.clear();
    {
        // Unit/camera local transforms already changed in Update; ordinary
        // GlobalTransform propagation happens later. Read the current hierarchy
        // so gauges do not lag behind moving or rotating actors by one frame.
        let helper = transforms.p0();
        let camera = cam.single().ok().and_then(|(entity, camera, projection)| {
            let transform = helper.compute_global_transform(entity).ok()?;
            let size = camera.logical_viewport_size()?;
            let clip = projection.get_clip_from_view() * transform.to_matrix().inverse();
            Some((size, clip, clip.inverse(), transform.compute_transform().rotation))
        });
        for (entity, overlay, p) in &metadata {
            let layout = camera.and_then(|(size, clip, inverse, rotation)| {
                let parent = helper.compute_global_transform(p.parent()).ok()?;
                let (bounds, radius) = parents.get(p.parent()).ok()?;
                let bounds = bounds.copied().unwrap_or_else(|| ModelBounds::fallback(radius.map_or(DEFAULT_RADIUS, |r| r.0)));
                let (screen, depth) = geometry::project(bounds, parent.to_matrix(), clip, size)?;
                let rect = geometry::bar_rect(screen, overlay.kind == OverlayKind::ShieldBar, *style)?;
                let world = geometry::world_bar(rect, depth, inverse, size, rotation)?;
                Some(GlobalTransform::from(world).reparented_to(&parent))
            });
            updates.push((entity, layout));
        }
    }
    let mut bars = transforms.p1();
    for (entity, layout) in updates.iter() {
        let Ok((mut tf, mut visibility)) = bars.get_mut(*entity) else { continue };
        if let Some(layout) = layout {
            *tf = *layout;
            *visibility = Visibility::Inherited;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}

fn count(mut counts: ResMut<OverlayCounts>, q: Query<&Overlay>) {
    let mut c = OverlayCounts::default();
    for o in &q {
        match o.kind {
            OverlayKind::Ring => c.rings += 1,
            OverlayKind::HullBar | OverlayKind::ShieldBar => c.bars += 1,
            OverlayKind::Line => c.lines += 1,
        }
    }
    if *counts != c {
        *counts = c;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::render::units::{LocalPlayer, add_view_systems};
    use crate::sim_link::SimLinkPlugin;
    use crate::viewer::TeamColours;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;
    use sta_sim::{Fixed, PlayerId, Vec3 as SimVec3};

    fn rig(text: &str) -> App {
        let mut app = App::new();
        let db = sta_content::ContentDb::builder().build().unwrap();
        let source = ScriptedSource::from_json5(text, &db).unwrap();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 30.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(LocalPlayer(None))
            .insert_resource(TeamColours(vec![("empty".into(), Color::WHITE), ("blue".into(), Color::srgb(0.0, 0.0, 1.0))]))
            .insert_resource(SimLink::new(Box::new(source), Box::new(LocalRouter::new()), PlayerId(1)));
        configure_sets(&mut app);
        add_view_systems(&mut app);
        app.add_plugins(OverlaysPlugin);
        app
    }

    const TWO: &str = "{ units: [
        { id: 1, owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
        { id: 2, owner: 1, hull: 100, until: 20, rails: [{ at: 0, pos: { x: 9, y: 0, z: 0 } }] },
    ] }";

    fn counts(app: &App) -> OverlayCounts {
        *app.world().resource::<OverlayCounts>()
    }

    fn select(app: &mut App, ids: &[u32]) {
        app.world_mut().resource_mut::<Selection>().0 = ids.iter().map(|i| EntityId(*i)).collect();
    }

    fn orphans(app: &mut App) -> usize {
        let ents: Vec<EntityId> = app.world().resource::<SimEntities>().0.keys().copied().collect();
        let sel = app.world().resource::<Selection>().0.clone();
        let mut q = app.world_mut().query::<&Overlay>();
        q.iter(app.world()).filter(|o| !ents.contains(&o.unit) || !sel.contains(&o.unit)).count()
    }

    #[test]
    fn counts_follow_selection() {
        let mut app = rig(TWO);
        app.update();
        assert_eq!(counts(&app), OverlayCounts::default());
        select(&mut app, &[1]);
        app.update();
        assert_eq!(counts(&app), OverlayCounts { rings: 1, bars: 1, lines: 0 });
        select(&mut app, &[1, 2]);
        app.update();
        assert_eq!(counts(&app), OverlayCounts { rings: 2, bars: 2, lines: 0 });
        select(&mut app, &[2]);
        app.update();
        assert_eq!(counts(&app), OverlayCounts { rings: 1, bars: 1, lines: 0 });
        assert_eq!(orphans(&mut app), 0);
        select(&mut app, &[]);
        app.update();
        assert_eq!(counts(&app), OverlayCounts::default());
        let mut q = app.world_mut().query::<&Overlay>();
        assert_eq!(q.iter(app.world()).count(), 0);
    }

    #[test]
    fn destroyed_units_leave_no_overlays() {
        let mut app = rig(TWO);
        select(&mut app, &[1, 2]);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(counts(&app).rings, 2);
        for _ in 0..40 {
            app.update();
        }
        assert!(app.world().resource::<SimLink>().curr().tick > 20);
        assert_eq!(counts(&app), OverlayCounts { rings: 1, bars: 1, lines: 0 });
        assert_eq!(orphans(&mut app), 0, "unit 2 is still selected but its overlays went with it");
        select(&mut app, &[1]);
        app.update();
        assert_eq!(orphans(&mut app), 0);
        // No overlay entity is left without a unit root or the line's owner.
        let mut q = app.world_mut().query::<(&Overlay, Option<&ChildOf>)>();
        for (o, parent) in q.iter(app.world()) {
            assert_eq!(o.unit, EntityId(1));
            assert!(parent.is_some() || o.kind == OverlayKind::Line);
        }
    }

    #[test]
    fn bar_fill_is_hull_share() {
        let mut app = rig(TWO);
        select(&mut app, &[1]);
        app.update();
        app.update();
        let root = app.world().resource::<SimEntities>().0[&EntityId(1)];
        // The scripted hull never changes, so set the share and run just the bar system.
        app.world_mut().entity_mut(root).insert(HullShare(0.25));
        app.world_mut().run_system_once(update_bars).unwrap();
        let mut q = app.world_mut().query::<(&BarFill, &Transform)>();
        let fills: Vec<_> = q.iter(app.world()).collect();
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].0.share, 0.25);
        assert!((fills[0].1.scale.x - 0.25).abs() < 1e-5);
        assert!((fills[0].1.translation.x + 0.75 * 0.5).abs() < 1e-5);
    }

    #[test]
    fn gauge_projection_uses_current_transforms_before_global_propagation() {
        use bevy::camera::{CameraProjection, ComputedCameraValues, RenderTargetInfo};
        let size = Vec2::new(1280.0, 720.0);
        let mut projection = PerspectiveProjection::default();
        projection.update(size.x, size.y);
        let camera = Camera {
            computed: ComputedCameraValues {
                clip_from_view: projection.get_clip_from_view(),
                target_info: Some(RenderTargetInfo { physical_size: UVec2::new(1280, 720), scale_factor: 1.0 }),
                ..default()
            }, ..default()
        };
        let mut app = App::new();
        app.init_resource::<GaugeStyle>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<bevy::input::mouse::AccumulatedMouseMotion>()
            .init_resource::<bevy::input::mouse::AccumulatedMouseScroll>()
            .add_systems(Update, (billboard.after(crate::viewer::orbit_camera), crate::viewer::orbit_camera));
        let camera_id = app.world_mut().spawn((Camera3d::default(), camera,
            Projection::Perspective(projection.clone()),
            Transform::from_xyz(0.0, 100.0, 300.0).looking_at(Vec3::ZERO, Vec3::Y),
            crate::viewer::Orbit { focus: Vec3::ZERO, yaw: 0.0, pitch: 0.5, dist: 300.0, home: (Vec3::ZERO, 300.0) })).id();
        let bounds = ModelBounds::fallback(20.0);
        let actor = app.world_mut().spawn((Transform::default(), bounds)).id();
        let bar = app.world_mut().spawn((Billboard,
            Overlay { unit: EntityId(1), kind: OverlayKind::HullBar }, ChildOf(actor),
            Transform::default(), Visibility::Hidden)).id();
        // No TransformPlugin: globals deliberately remain stale throughout.
        for (x, angle, distance) in [(0.0, 0.0, 300.0), (30.0, 0.8, 450.0)] {
            let parent = Transform::from_xyz(x, 10.0, 0.0).with_rotation(Quat::from_rotation_y(angle));
            *app.world_mut().get_mut::<Transform>(actor).unwrap() = parent;
            {
                let mut orbit = app.world_mut().get_mut::<crate::viewer::Orbit>(camera_id).unwrap();
                orbit.focus = Vec3::new(x * 0.5, 0.0, 0.0);
                orbit.yaw = angle * 0.3;
                orbit.dist = distance;
            }
            app.update();
            let camera = *app.world().get::<Transform>(camera_id).unwrap();
            assert_eq!(*app.world().get::<Visibility>(bar).unwrap(), Visibility::Inherited);
            let clip = projection.get_clip_from_view() * camera.to_matrix().inverse();
            let (screen, _) = geometry::project(bounds, parent.to_matrix(), clip, size).unwrap();
            let expected = geometry::bar_rect(screen, false, GaugeStyle::default()).unwrap();
            let local = app.world().get::<Transform>(bar).unwrap();
            let p = clip.project_point3(parent.transform_point(local.transform_point(Vec3::new(-0.5, 0.5, 0.0))));
            let actual = Vec2::new(p.x + 1.0, 1.0 - p.y) * size * 0.5;
            assert!((actual - expected.min).length() < 0.01, "current frame: {actual:?}, expected {expected:?}");
        }
    }

    #[test]
    fn ring_takes_the_team_colour_and_target_makes_a_line() {
        let mut app = rig(TWO);
        select(&mut app, &[1]);
        app.world_mut().resource_mut::<OrderTargets>().0.insert(EntityId(1), SimVec3::from_f32([30.0, 0.0, 0.0]));
        app.update();
        app.update();
        assert_eq!(counts(&app), OverlayCounts { rings: 1, bars: 1, lines: 1 });
        let mut q = app.world_mut().query::<(&Overlay, &MeshMaterial3d<StandardMaterial>)>();
        let ring = q.iter(app.world()).find(|(o, _)| o.kind == OverlayKind::Ring).unwrap().1.0.clone();
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().get(&ring).unwrap().base_color, Color::srgb(0.0, 0.0, 1.0));
        app.world_mut().resource_mut::<OrderTargets>().0.clear();
        app.update();
        assert_eq!(counts(&app).lines, 0);
    }

    #[test]
    fn shield_share_sums_facings() {
        let mut u = UnitView {
            id: EntityId(1),
            def: sta_sim::ContentKey::NONE,
            owner: PlayerId(1),
            pos: SimVec3::ZERO,
            heading: (Fixed::ONE, Fixed::ZERO),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE, Fixed::ONE],
            shields: [[Fixed::ZERO, Fixed::ZERO]; 4],
            crew: None,
            energy: None,
            flags: Default::default(),
            progress: None,
        };
        assert_eq!(shield_share(&u), None);
        u.shields = [[Fixed::from_int(5), Fixed::from_int(10)], [Fixed::from_int(10), Fixed::from_int(10)], [Fixed::ZERO, Fixed::from_int(10)], [Fixed::from_int(5), Fixed::from_int(10)]];
        assert_eq!(shield_share(&u), Some(0.5));
    }
}
