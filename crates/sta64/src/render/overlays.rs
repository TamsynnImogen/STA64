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
//! Under the Armada I stock presentation the ring is the install's own marker sprite instead, and
//! the unit under the cursor gets one too, see [`marker`] (RND-37).
//!
//! Rings and bars are children of the unit root, so they go with it when
//! the unit is destroyed; [`sync_overlays`] despawns the rest when the
//! selection changes. [`OverlayCounts`] says how many of each exist (for
//! bench marks, once TOOL-07 lands). Refit's own overlays are not in this
//! repo; this follows the work package, not a port.

use std::collections::{BTreeMap, BTreeSet};

mod geometry;
pub mod bubble;
pub mod marker;
pub mod extras;
pub mod readout;
pub mod systems;
pub use readout::armada2_look;
#[cfg(test)]
mod gpu;
pub use geometry::{GaugeStyle, ModelBounds};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use sta_sim::{EntityId, PlayerId, UnitView};

use crate::app::GameSet;
use crate::input::hover::{HoverReadout, HoveredUnit};
use crate::input::orders::OrderTargets;
use crate::render::units::{HullShare, MarkColour, SelectRadius, TeamColour, to_bevy};
use crate::sim_link::{ContentRes, Selection, SimEntities, SimLink};
use crate::ui::profile::UiProfile;
use crate::viewer::{MapTeamColours, TeamColours, owner_mark_colour};

/// Radius used for units without a [`SelectRadius`].
const DEFAULT_RADIUS: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayKind {
    Ring,
    HullBar,
    ShieldBar,
    /// The special-energy bar, Armada II readout only.
    EnergyBar,
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

/// What the Armada II hover readout needs in [`sync_overlays`], in one parameter.
#[derive(SystemParam)]
struct ReadoutInputs<'w> {
    profile: Option<Res<'w, UiProfile>>,
    readout: Res<'w, HoverReadout>,
    content: Option<Res<'w, ContentRes>>,
    art: Res<'w, readout::BarArt>,
}

/// A selection outline follows its current owner's mark colour, even on roots placed by a map.
#[derive(SystemParam)]
struct SelectionPalette<'w> {
    teams: Option<Res<'w, TeamColours>>,
    map: Option<Res<'w, MapTeamColours>>,
}

impl SelectionPalette<'_> {
    fn colour(&self, owner: Option<PlayerId>, mark: Option<&MarkColour>, hull: Option<&TeamColour>) -> Option<Color> {
        owner
            .filter(|owner| owner.is_player())
            .and_then(|owner| owner_mark_colour(self.teams.as_deref(), self.map.as_deref(), owner))
            .or_else(|| mark.map(|mark| mark.0))
            .or_else(|| hull.map(|hull| hull.0))
    }
}

/// The sprite crop of a special-energy fill, so its UV can follow the share.
#[derive(Component)]
struct EnergyCrop(readout::RowArt);

/// A bar of the Armada II hover readout: additive, its colour follows the readout fade and its share.
#[derive(Component)]
struct ReadoutBar;

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
    readout_back: Handle<StandardMaterial>,
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
            .init_resource::<HoverReadout>()
            .init_resource::<readout::BarArt>()
            .init_resource::<systems::SystemArt>()
            .init_resource::<marker::MarkerArt>()
            .add_plugins(bubble::BubblePlugin)
            .add_systems(Startup, make_assets.run_if(resource_exists::<Assets<Mesh>>).run_if(resource_exists::<Assets<StandardMaterial>>))
            .add_systems(
                Update,
                (crate::input::hover::tick_readout, marker::load_art, readout::load_art, systems::load_art, sync_overlays, marker::orient, update_bars, update_lines, billboard.after(crate::viewer::orbit_camera), extras::sync_extras, systems::sync_icons, count)
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

/// The readout's bars add their colour to the frame (the original's bar material is additive).
fn additive(colour: Color) -> StandardMaterial {
    StandardMaterial { base_color: colour, unlit: true, cull_mode: None, alpha_mode: AlphaMode::Add, ..default() }
}

fn make_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(OverlayAssets {
        ring: meshes.add(Annulus::new(0.96, 1.0)),
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        line_mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        bar_back: mats.add(unlit(Color::srgba(0.05, 0.05, 0.05, 0.8))),
        readout_back: mats.add(additive(Color::srgb(readout::BACKGROUND, readout::BACKGROUND, readout::BACKGROUND))),
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

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_overlays(
    mut commands: Commands,
    selection: Res<Selection>,
    hovered: Option<Res<HoveredUnit>>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    art: Res<marker::MarkerArt>,
    inputs: ReadoutInputs,
    palette: SelectionPalette,
    ents: Res<SimEntities>,
    link: Res<SimLink>,
    targets: Res<OrderTargets>,
    assets: Res<OverlayAssets>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    units: Query<(Option<&SelectRadius>, Option<&MarkColour>, Option<&TeamColour>, Option<&HullShare>)>,
    existing: Query<(Entity, &Overlay, Has<marker::Marker>, Has<ReadoutBar>, Option<&MeshMaterial3d<StandardMaterial>>)>,
) {
    let mut want: BTreeMap<(EntityId, OverlayKind), Entity> = BTreeMap::new();
    let ReadoutInputs { profile, readout, content, art: bar_art } = inputs;
    let readout_look = armada2_look(profile.as_deref());
    let cutscene = crate::ui::presentation::cutscene_running(cine);
    let ring_colour = |id, mark, hull| {
        palette.colour(link.curr().unit(id).map(|unit| unit.owner), mark, hull).unwrap_or(Color::srgb(0.3, 1.0, 0.4))
    };
    // The Armada I marker also shows under the hovered unit, selected or not, except while a
    // cineractive runs: hover tracking goes on then and ships sweeping under the resting cursor
    // would flicker markers. Only this overlay is gated; hover for the HUD is unchanged. A
    // selection kept through the cutscene keeps its marker.
    if art.art.is_some()
        && !cutscene
        && let Some(id) = hovered.as_ref().and_then(|h| h.0)
        && let Some(&root) = ents.0.get(&id)
    {
        want.insert((id, OverlayKind::Ring), root);
    }
    for id in &selection.0 {
        let Some(&root) = ents.0.get(id) else { continue };
        if units.get(root).is_err() {
            continue;
        }
        // The Armada II look shows the selection as the bubble (see [`bubble`]) instead of the team
        // annulus.
        if !readout_look {
            want.insert((*id, OverlayKind::Ring), root);
            want.insert((*id, OverlayKind::HullBar), root);
            if link.curr().unit(*id).and_then(shield_share).is_some() {
                want.insert((*id, OverlayKind::ShieldBar), root);
            }
        }
        if targets.0.contains_key(id) {
            want.insert((*id, OverlayKind::Line), root);
        }
    }
    // Armada II: bars are a hover readout, not a permanent overlay (see [`readout`]). Selection alone
    // shows none, and the team annulus gives way to the selection bubble.
    // Interim: none while a cineractive runs (not established for the original).
    if readout_look && !cutscene && readout.bars_up() {
        let candidates: BTreeSet<EntityId> = selection.0.iter().copied().chain(readout.unit).collect();
        for id in candidates {
            let (Some(&root), Some(u)) = (ents.0.get(&id), link.curr().unit(id)) else { continue };
            if units.get(root).is_err() {
                continue;
            }
            let selected = selection.0.contains(&id);
            if !readout::shows(&readout, id, u.owner, selected, readout::is_starbase(content.as_deref(), u.def)) {
                continue;
            }
            let [shield, hull, energy] = readout::shares(u);
            for (kind, share) in [(OverlayKind::ShieldBar, shield), (OverlayKind::HullBar, hull), (OverlayKind::EnergyBar, energy)] {
                if share.is_some() {
                    want.insert((id, kind), root);
                }
            }
        }
    }
    let mut have: BTreeSet<(EntityId, OverlayKind)> = BTreeSet::new();
    for (e, o, is_marker, is_readout, material) in &existing {
        let is_bar = matches!(o.kind, OverlayKind::HullBar | OverlayKind::ShieldBar | OverlayKind::EnergyBar);
        let stale = (o.kind == OverlayKind::Ring && is_marker != art.art.is_some()) || (is_bar && is_readout != readout_look);
        let key = (o.unit, o.kind);
        let Some(&root) = want.get(&key).filter(|_| !stale && have.insert(key)) else {
            commands.entity(e).try_despawn();
            continue;
        };
        if o.kind == OverlayKind::Ring
            && !is_marker
            && let Some(material) = material
            && let Ok((_, mark, hull, _)) = units.get(root)
        {
            let colour = ring_colour(o.unit, mark, hull);
            if mats.get(&material.0).is_some_and(|m| m.base_color != colour)
                && let Some(mut m) = mats.get_mut(&material.0)
            {
                m.base_color = colour;
            }
        }
    }
    for ((id, kind), root) in want {
        if have.contains(&(id, kind)) {
            continue;
        }
        let Ok((radius, mark, team, hull)) = units.get(root) else { continue };
        let radius = radius.map_or(DEFAULT_RADIUS, |r| r.0);
        let overlay = Overlay { unit: id, kind };
        match kind {
            OverlayKind::Ring if art.art.is_some() => {
                let a = art.art.as_ref().unwrap();
                commands.spawn((
                    Name::new("selection marker"),
                    overlay,
                    marker::Marker { radius },
                    ChildOf(root),
                    Mesh3d(assets.quad.clone()),
                    MeshMaterial3d(a.material.clone()),
                    Transform::default(),
                ));
            }
            OverlayKind::Ring => {
                let colour = ring_colour(id, mark, team);
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
            OverlayKind::HullBar | OverlayKind::ShieldBar | OverlayKind::EnergyBar if readout_look => {
                // Placeholder geometry and colour: the billboard and `update_bars` set both each frame.
                let row = match kind {
                    OverlayKind::ShieldBar => 0,
                    OverlayKind::HullBar => 1,
                    _ => 2,
                };
                let art = bar_art.rows[row].as_ref();
                let fill = commands
                    .spawn((
                        BarFill { share: 1.0 },
                        Mesh3d(assets.quad.clone()),
                        MeshMaterial3d(mats.add(readout::fill_material(art))),
                        Transform::from_xyz(0.0, 0.0, 0.01),
                    ))
                    .id();
                if kind == OverlayKind::EnergyBar && let Some(a) = art {
                    commands.entity(fill).insert(EnergyCrop(a.clone()));
                }
                let bar = commands.spawn((Name::new("readout bar"), overlay, Billboard, ReadoutBar, ChildOf(root), Transform::default(), Visibility::Hidden)).id();
                commands.spawn((ChildOf(bar), Mesh3d(assets.quad.clone()), MeshMaterial3d(assets.readout_back.clone()), Transform::default()));
                commands.entity(fill).insert(ChildOf(bar));
            }
            OverlayKind::HullBar | OverlayKind::ShieldBar | OverlayKind::EnergyBar => {
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

/// Set each fill from the hull share or the view's shields; the readout's bars also take their colour.
fn update_bars(
    link: Res<SimLink>,
    readout: Res<HoverReadout>,
    hulls: Query<&HullShare>,
    ents: Res<SimEntities>,
    bars: Query<(&Overlay, &Children, Has<ReadoutBar>)>,
    mut fills: Query<(&mut BarFill, &mut Transform, &MeshMaterial3d<StandardMaterial>, Option<&EnergyCrop>)>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    for (o, kids, is_readout) in &bars {
        let share = match o.kind {
            OverlayKind::HullBar => ents.0.get(&o.unit).and_then(|e| hulls.get(*e).ok()).map(|h| h.0),
            OverlayKind::ShieldBar => link.curr().unit(o.unit).and_then(shield_share),
            OverlayKind::EnergyBar => link.curr().unit(o.unit).and_then(|u| readout::shares(u)[2]),
            _ => continue,
        };
        let Some(share) = share else { continue };
        let f = readout.fade;
        let colour = match o.kind {
            OverlayKind::ShieldBar => readout::shield_colour(share, f),
            OverlayKind::HullBar => readout::hull_colour(share, f),
            _ => readout::energy_colour(f),
        };
        for k in kids.iter() {
            if let Ok((mut fill, mut tf, material, crop)) = fills.get_mut(k) {
                fill.share = share;
                tf.translation.x = -(1.0 - share) * 0.5;
                tf.scale = Vec3::new(share, 1.0, 1.0);
                if is_readout && let Some(mut m) = mats.get_mut(&material.0) {
                    let c = Color::srgb(colour[0], colour[1], colour[2]);
                    if m.base_color != c {
                        m.base_color = c;
                    }
                    if let Some(crop) = crop {
                        let uv = readout::energy_uv(&crop.0, share);
                        if m.uv_transform != uv {
                            m.uv_transform = uv;
                        }
                    }
                }
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
                let row = match overlay.kind {
                    OverlayKind::ShieldBar => -1,
                    OverlayKind::EnergyBar => 1,
                    _ => 0,
                };
                let rect = geometry::bar_row_rect(screen, row, *style)?;
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
            OverlayKind::HullBar | OverlayKind::ShieldBar | OverlayKind::EnergyBar => c.bars += 1,
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
    use crate::ui::sprite_font::SpriteText;
    use crate::app::configure_sets;
    use crate::render::units::{LocalPlayer, add_view_systems};
    use crate::sim_link::SimLinkPlugin;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;
    use sta_sim::{Fixed, Vec3 as SimVec3};

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
        { id: 1, owner: 0, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
        { id: 2, owner: 0, hull: 100, until: 20, rails: [{ at: 0, pos: { x: 9, y: 0, z: 0 } }] },
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

    const MIXED: &str = "{ units: [
        { id: 1, owner: 0, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
        { id: 2, owner: 0, hull: 100, rails: [{ at: 0, pos: { x: 9, y: 0, z: 0 } }] },
        { id: 3, owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 18, y: 0, z: 0 } }] },
    ] }";

    fn bar_units(app: &mut App) -> Vec<(EntityId, OverlayKind)> {
        let mut q = app.world_mut().query::<&Overlay>();
        let mut v: Vec<_> = q.iter(app.world()).filter(|o| matches!(o.kind, OverlayKind::HullBar | OverlayKind::ShieldBar | OverlayKind::EnergyBar)).map(|o| (o.unit, o.kind)).collect();
        v.sort();
        v
    }

    fn hover(app: &mut App, unit: Option<(u32, u8)>, fade: f32) {
        *app.world_mut().resource_mut::<HoverReadout>() = HoverReadout { unit: unit.map(|u| EntityId(u.0)), team: unit.map(|u| PlayerId(u.1)), fade };
    }

    #[test]
    fn armada_ii_bars_are_a_hover_readout_and_selection_alone_shows_none() {
        let mut app = rig(MIXED);
        app.update();
        app.world_mut().insert_resource(UiProfile::Armada2);
        select(&mut app, &[1, 3]);
        sync(&mut app);
        assert_eq!(counts_now(&mut app), OverlayCounts { rings: 0, bars: 0, lines: 0 }, "selected only: no bars, and no team annulus (the bubble shows the selection)");
        // Resting on unit 2 (team 0) for less than a second: nothing yet.
        hover(&mut app, Some((2, 0)), 0.5);
        sync(&mut app);
        assert!(bar_units(&mut app).is_empty());
        // Past the threshold: the hovered unit and the selected unit of its team, not the other team's.
        hover(&mut app, Some((2, 0)), 0.51);
        sync(&mut app);
        assert_eq!(bar_units(&mut app), vec![(EntityId(1), OverlayKind::HullBar), (EntityId(2), OverlayKind::HullBar)]);
        // Hovering an enemy of the selection: bars on the enemy only.
        hover(&mut app, Some((3, 1)), 0.9);
        select(&mut app, &[1]);
        sync(&mut app);
        assert_eq!(bar_units(&mut app), vec![(EntityId(3), OverlayKind::HullBar)]);
        // Nothing hovered: nothing, whatever the fade.
        hover(&mut app, None, 1.0);
        sync(&mut app);
        assert!(bar_units(&mut app).is_empty());
        let mut q = app.world_mut().query::<&Overlay>();
        assert_eq!(q.iter(app.world()).filter(|o| o.kind == OverlayKind::Ring).count(), 0, "no team annulus under the bubble look");
    }

    #[test]
    fn no_readout_bars_during_a_cineractive_and_none_for_a_neutral_hover() {
        let mut app = rig(MIXED);
        app.update();
        app.world_mut().insert_resource(UiProfile::Armada2);
        select(&mut app, &[1]);
        hover(&mut app, Some((2, 0)), 1.0);
        app.world_mut().insert_resource(crate::ui::presentation::CineractiveState { on: true, skippable: true });
        sync(&mut app);
        assert!(bar_units(&mut app).is_empty(), "interim: no readout bars in a cutscene");
        app.world_mut().insert_resource(crate::ui::presentation::CineractiveState::default());
        sync(&mut app);
        assert_eq!(bar_units(&mut app).len(), 2);
        // A neutral hover carries no team in the readout: nothing shows, selected units included.
        *app.world_mut().resource_mut::<HoverReadout>() = HoverReadout { unit: Some(EntityId(2)), team: None, fade: 1.0 };
        sync(&mut app);
        assert!(bar_units(&mut app).is_empty());
    }

    #[test]
    fn the_group_number_exists_only_in_the_armada_ii_look() {
        use crate::input::select::ControlGroups;
        for profile in [None, Some(UiProfile::Lcars), Some(UiProfile::Armada1), Some(UiProfile::FleetOps), Some(UiProfile::Armada2)] {
            let mut app = rig(MIXED);
            app.update();
            if let Some(p) = profile {
                app.world_mut().insert_resource(p);
            }
            app.init_resource::<ControlGroups>();
            let stale = app.world_mut().spawn((extras::GroupNumber { unit: EntityId(1) }, Node::default(), SpriteText { text: "1".into(), size: 16.0, color: Color::WHITE })).id();
            select(&mut app, &[1]);
            hover(&mut app, Some((1, 0)), 1.0);
            app.world_mut().run_system_once(extras::sync_extras).unwrap();
            app.world_mut().flush();
            // Without a camera nothing can be placed: the stale node goes in every look, and none appears.
            assert!(app.world().get_entity(stale).is_err(), "{profile:?}");
            let mut q = app.world_mut().query::<&extras::GroupNumber>();
            assert_eq!(q.iter(app.world()).count(), 0, "{profile:?}");
        }
    }

    #[test]
    fn the_group_number_draws_at_the_anchor_in_the_armada_ii_look() {
        use crate::input::select::ControlGroups;
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo, Viewport};
        let mut app = rig(MIXED);
        app.update();
        app.world_mut().insert_resource(UiProfile::Armada2);
        let mut groups = ControlGroups::default();
        groups.groups[2] = vec![EntityId(1)];
        app.insert_resource(groups);
        let size = UVec2::new(1280, 720);
        let camera_tf = Transform::from_xyz(0.0, 0.0, 60.0).looking_at(Vec3::ZERO, Vec3::Y);
        app.world_mut().spawn((
            Camera3d::default(),
            camera_tf,
            Camera {
                computed: ComputedCameraValues { target_info: Some(RenderTargetInfo { physical_size: size, scale_factor: 1.0 }), ..default() },
                viewport: Some(Viewport { physical_size: size, ..default() }),
                ..default()
            },
        ));
        select(&mut app, &[1]);
        let run = |app: &mut App| {
            app.world_mut().run_system_once(extras::sync_extras).unwrap();
            app.world_mut().flush();
        };
        run(&mut app);
        let mut q = app.world_mut().query::<(&extras::GroupNumber, &Node, &SpriteText)>();
        let found: Vec<_> = q.iter(app.world()).map(|(g, n, t)| (g.unit, n.left, n.top, t.text.clone())).collect();
        assert_eq!(found.len(), 1, "{found:?}");
        // The anchor from the same projection the bars use.
        let root = app.world().resource::<SimEntities>().0[&EntityId(1)];
        let radius = app.world().get::<SelectRadius>(root).map_or(DEFAULT_RADIUS, |r| r.0);
        let bounds = app.world().get::<ModelBounds>(root).copied().unwrap_or_else(|| ModelBounds::fallback(radius));
        // The camera keeps its spawned projection (no aspect update system runs in this app).
        let clip = Projection::default().get_clip_from_view() * camera_tf.to_matrix().inverse();
        let world = app.world().get::<GlobalTransform>(root).map_or(Mat4::IDENTITY, |g| g.to_matrix());
        let (b, _) = geometry::project(bounds, world, clip, Vec2::new(1280.0, 720.0)).unwrap();
        let at = extras::group_anchor(b);
        assert_eq!((found[0].0, found[0].3.as_str()), (EntityId(1), "3"));
        assert_eq!((found[0].1, found[0].2), (Val::Px(at.x), Val::Px(at.y)));
        // Unselected and not hovered: gone. Selected but in no group: none.
        select(&mut app, &[]);
        run(&mut app);
        assert_eq!(q.iter(app.world()).count(), 0);
        select(&mut app, &[2]);
        run(&mut app);
        assert_eq!(q.iter(app.world()).count(), 0, "in no group");
    }

    #[test]
    fn the_other_looks_keep_their_permanent_bars() {
        for profile in [None, Some(UiProfile::Lcars), Some(UiProfile::Armada1), Some(UiProfile::FleetOps)] {
            let mut app = rig(MIXED);
            app.update();
            if let Some(p) = profile {
                app.world_mut().insert_resource(p);
            }
            hover(&mut app, Some((2, 0)), 1.0);
            select(&mut app, &[1, 3]);
            sync(&mut app);
            assert_eq!(bar_units(&mut app), vec![(EntityId(1), OverlayKind::HullBar), (EntityId(3), OverlayKind::HullBar)], "{profile:?}: selected units, no hover rule");
            hover(&mut app, None, 0.0);
            sync(&mut app);
            assert_eq!(bar_units(&mut app).len(), 2, "{profile:?}: the fade does not matter");
        }
    }

    fn give_marker_art(app: &mut App) {
        app.world_mut().resource_mut::<marker::MarkerArt>().art = Some(marker::Art { material: Handle::default() });
    }

    fn sync(app: &mut App) {
        app.world_mut().run_system_once(sync_overlays).unwrap();
        app.world_mut().flush();
    }

    fn markers(app: &mut App) -> Vec<EntityId> {
        let mut q = app.world_mut().query::<(&Overlay, &marker::Marker)>();
        let mut ids: Vec<_> = q.iter(app.world()).map(|(o, _)| o.unit).collect();
        ids.sort();
        ids
    }

    #[test]
    fn marker_shows_under_selected_and_hovered_units_when_the_art_exists() {
        let mut app = rig(TWO);
        app.update();
        app.world_mut().insert_resource(HoveredUnit(Some(EntityId(2))));
        sync(&mut app);
        assert!(markers(&mut app).is_empty(), "no art, no marker: other profiles keep their look");
        select(&mut app, &[1]);
        give_marker_art(&mut app);
        sync(&mut app);
        assert_eq!(markers(&mut app), vec![EntityId(1), EntityId(2)], "selected and hovered");
        // The hovered, unselected unit gets the marker only, no bars.
        assert_eq!(counts_now(&mut app), OverlayCounts { rings: 2, bars: 1, lines: 0 });
        app.world_mut().insert_resource(HoveredUnit(Some(EntityId(1))));
        sync(&mut app);
        assert_eq!(markers(&mut app), vec![EntityId(1)], "selected and hovered at once is still one marker");
        app.world_mut().insert_resource(HoveredUnit(None));
        select(&mut app, &[]);
        sync(&mut app);
        assert!(markers(&mut app).is_empty());
    }

    #[test]
    fn no_hover_marker_during_a_cineractive_but_a_kept_selection_keeps_its_marker() {
        let mut app = rig(TWO);
        app.update();
        select(&mut app, &[1]);
        give_marker_art(&mut app);
        app.world_mut().insert_resource(HoveredUnit(Some(EntityId(2))));
        app.world_mut().insert_resource(crate::ui::presentation::CineractiveState { on: true, skippable: true });
        sync(&mut app);
        assert_eq!(markers(&mut app), vec![EntityId(1)], "selected only: no hover marker in a cutscene");
        app.world_mut().insert_resource(crate::ui::presentation::CineractiveState::default());
        sync(&mut app);
        assert_eq!(markers(&mut app), vec![EntityId(1), EntityId(2)], "hover marker is back when it ends");
    }

    #[test]
    fn losing_the_art_brings_the_plain_ring_back() {
        let mut app = rig(TWO);
        app.update();
        select(&mut app, &[1]);
        give_marker_art(&mut app);
        sync(&mut app);
        assert_eq!(markers(&mut app).len(), 1);
        app.world_mut().resource_mut::<marker::MarkerArt>().art = None;
        sync(&mut app);
        assert!(markers(&mut app).is_empty());
        assert_eq!(counts_now(&mut app).rings, 1, "the team ring is drawn again");
    }

    fn counts_now(app: &mut App) -> OverlayCounts {
        app.world_mut().run_system_once(count).unwrap();
        counts(app)
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
    fn fleet_ops_ring_tracks_the_current_seat_palette_without_root_colours() {
        const SECOND_SEAT: &str = "{ units: [
            { id: 1, owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
        ] }";
        let mut app = rig(SECOND_SEAT);
        let red = Color::srgb(0.9, 0.1, 0.1);
        let blue = Color::srgb(0.1, 0.3, 0.9);
        app.insert_resource(UiProfile::FleetOps);
        app.insert_resource(TeamColours(vec![("mpcolor01".into(), Color::WHITE), ("mpcolor02".into(), red)]));
        select(&mut app, &[1]);
        app.update();
        app.update();
        let root = app.world().resource::<SimEntities>().0[&EntityId(1)];
        app.world_mut().entity_mut(root).remove::<(MarkColour, TeamColour)>();
        let mut q = app.world_mut().query::<(Entity, &Overlay, &MeshMaterial3d<StandardMaterial>)>();
        let (ring, _, material) = q.iter(app.world()).find(|(_, o, _)| o.kind == OverlayKind::Ring).unwrap();
        let material = material.0.clone();
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().get(&material).unwrap().base_color, red);

        app.world_mut().resource_mut::<TeamColours>().0[1].1 = blue;
        app.update();
        assert_eq!(app.world().get::<MeshMaterial3d<StandardMaterial>>(ring).unwrap().0, material);
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().get(&material).unwrap().base_color, blue);
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
            energy: None, autonomy: None,
            flags: Default::default(),
            progress: None,
        };
        assert_eq!(shield_share(&u), None);
        u.shields = [[Fixed::from_int(5), Fixed::from_int(10)], [Fixed::from_int(10), Fixed::from_int(10)], [Fixed::ZERO, Fixed::from_int(10)], [Fixed::from_int(5), Fixed::from_int(10)]];
        assert_eq!(shield_share(&u), Some(0.5));
    }
}
