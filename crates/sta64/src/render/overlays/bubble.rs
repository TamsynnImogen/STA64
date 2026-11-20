//! The Armada II Stock-look selection bubble (#1046): a disc under the unit and a ring around it,
//! two passes of one translucent ellipsoid.
//!
//! Rules (grades: M measured on the running original, S read from the executable, static, P stated
//! by a player, I interim: STA64's stand-in until the named item lands):
//! - Mesh: the install's `select` model through the VFS, so a mod that supplies another shape changes
//!   the bubble [S, read from data at run time]. Without it an authored ellipsoid is drawn [I].
//! - Size: the unit's shield ellipsoid semi-axes, each at least [`MIN_AXIS`], times
//!   [`FIT`] over the mesh's bounding radius [S]. The ellipsoid is the model box half-extents times
//!   [`BOX_TO_SHIELD`], centred on the unit [I: until the sim provides the ellipsoid fitted to the
//!   vertices; whether the minimum applies before or after the model scale is not established].
//! - Pass 1 (inner, scale 1): selected craft, its relation colour times the damage pulse; selected
//!   non-craft, the team colour times [`DIM`]; hovered only, black. Pass 2 (outer, scale
//!   [`OUTER_SCALE`]): hovered, the full team colour; else a selected craft, the team colour times
//!   [`DIM`]; else none [M for craft and the four states, S for non-craft]. Pass 1 and a dimmed pass 2
//!   take [`PROFILE_FACTOR`]; the full hover colour does not [M].
//! - Additive, flat, unlit. Pass 1 writes depth even when black, so the shell is hidden inside the
//!   disc and only the ring shows [M]. Only the far surface behind the hull is drawn (front faces
//!   culled) and the depth test is strict, so nothing is added over hull pixels [M].
//! - Only the inner pass pulses; the ring is steady [M]. The pulse runs on real time, also while the
//!   game is paused [S: the display tick].
//! - Follows the unit's rotation (a child of the unit root). Not drawn while a cineractive runs [S].
//!   Other cameras that see the main scene draw it too; a dedicated main-view layer is not set up [I].
//! - Left out, logged as open: the co-player blink, the warp stretch, team 0's colour in a real game.
//!
//! Active under the Armada II profile only. Armada I keeps its marker ([`super::marker`]); Fleet
//! Operations, Roots and the Remaster look keep the team annulus.
//!
//! Depth in Bevy: the material draws in the transparent phase, sorted back to front by the mesh
//! centre. Both passes share the centre, so the inner pass is sorted first by a depth bias on the
//! outer one, and its pipeline sets depth write on. Opaque hulls, drawn before, have written depth
//! (also through the depth prepass); other transparent effects that sort after an inner pass are
//! hidden where they lie behind its far surface, as they are in the original.

use std::collections::{BTreeMap, BTreeSet};

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, CompareFunction, Face, PrimitiveTopology, RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use sta_sim::view::Relation;
use sta_sim::{EntityId, WorldView};

use super::ModelBounds;
use crate::app::GameSet;
use crate::input::hover::HoveredUnit;
use crate::render::units::{HullShare, SelectRadius, TeamColour};
use crate::sim_link::{Selection, SimEntities, SimLink};
use crate::ui::profile::UiProfile;
use crate::viewer::Mounted;

/// The shield ellipsoid's smallest semi-axis [S].
pub const MIN_AXIS: f32 = 15.0;
/// The bubble's size over the shield ellipsoid [S].
pub const FIT: f32 = 1.2;
/// The outer pass over the inner [S, M: the ratio of the two widths].
pub const OUTER_SCALE: f32 = 1.1;
/// Model box half-extent to shield semi-axis [I: until the sim provides the fitted ellipsoid].
pub const BOX_TO_SHIELD: f32 = 1.732_050_8;
/// The dim factor of a team colour and the pulse's base level [M].
pub const DIM: f32 = 0.25;
/// The pulse's amplitude per unit of missing hull [M].
pub const PULSE_AMPLITUDE: f32 = 0.2;
/// The profile's brightness factor: the stock default of a graphics-options slider [S: its label is
/// not identified; the bench's own setting was a step higher]. Replace when settings carry it.
pub const PROFILE_FACTOR: f32 = 0.625;
/// Relation colours of a selected craft's inner pass [M for enemy and ally, S for neutral].
pub const ENEMY: Vec3 = Vec3::new(0.7, 0.0, 1.0);
pub const NEUTRAL: Vec3 = Vec3::new(0.0, 0.7, 1.0);
pub const ALLY: Vec3 = Vec3::new(0.6, 0.6, 0.6);
/// Sort bias that puts the outer pass after the inner one (view distance, tiny against any scene).
const OUTER_SORT_BIAS: f32 = 0.05;

/// Which of the two passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pass {
    Inner,
    Outer,
}

/// A bubble pass under unit `unit`'s root.
#[derive(Component, Debug, Clone, Copy)]
pub struct Bubble {
    pub unit: EntityId,
    pub pass: Pass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BubbleKey {
    depth_write: bool,
}

impl From<&BubbleMaterial> for BubbleKey {
    fn from(m: &BubbleMaterial) -> Self {
        BubbleKey { depth_write: m.inner }
    }
}

/// One flat colour, added. The inner pass writes depth; the outer one only tests it.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(BubbleKey)]
pub struct BubbleMaterial {
    /// Linear RGB.
    #[uniform(0)]
    pub colour: Vec4,
    pub inner: bool,
}

impl Material for BubbleMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/overlays/bubble.wesl").into_owned())
    }

    /// Premultiplied with alpha 0 is a pure add.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn depth_bias(&self) -> f32 {
        if self.inner { 0.0 } else { OUTER_SORT_BIAS }
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The far surface only: front faces culled.
        descriptor.primitive.cull_mode = Some(Face::Front);
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(key.bind_group_data.depth_write);
            // Reverse-Z: strictly nearer than what is in the buffer passes.
            depth.depth_compare = Some(CompareFunction::Greater);
        }
        Ok(())
    }
}

pub struct BubblePlugin;

impl Plugin for BubblePlugin {
    fn build(&self, app: &mut App) {
        // The windowless logic tests have no renderer and add the asset themselves.
        if app.get_sub_app(bevy::render::RenderApp).is_some() {
            embedded_asset!(app, "bubble.wesl");
            app.add_plugins(MaterialPlugin::<BubbleMaterial>::default());
        }
        app.init_resource::<BubbleArt>().init_resource::<Pulses>().add_systems(
            Update,
            (load_art, sync_bubbles)
                .chain()
                .in_set(GameSet::Render)
                .distributive_run_if(resource_exists::<SimLink>)
                .distributive_run_if(resource_exists::<Assets<BubbleMaterial>>)
                .distributive_run_if(resource_exists::<Assets<Mesh>>),
        );
    }
}

/// Armada II Stock look only.
pub fn wanted(profile: Option<&UiProfile>) -> bool {
    profile.is_some_and(|p| *p == UiProfile::Armada2)
}

// ---------------------------------------------------------------- rules (pure)

/// The shield ellipsoid's semi-axes from the model box [I].
pub fn shield_semi_axes(bounds: ModelBounds) -> Vec3 {
    (((bounds.max - bounds.min) * 0.5).abs() * BOX_TO_SHIELD).max(Vec3::splat(MIN_AXIS))
}

/// The inner pass's scale for a mesh of bounding radius `radius`.
pub fn inner_scale(axes: Vec3, radius: f32) -> Vec3 {
    axes * FIT / radius.max(1e-3)
}

/// The inner pass's brightness at hull share `h` and pulse phase.
pub fn pulse(h: f32, phase: f32) -> f32 {
    DIM + PULSE_AMPLITUDE * (1.0 - h) * phase.sin()
}

/// How fast the pulse phase advances, radians per second.
pub fn phase_rate(h: f32) -> f32 {
    std::f32::consts::TAU * (2.0 * (1.0 - h) + 0.2 * h)
}

/// The colour a relation gives the inner pass of a selected craft.
pub fn relation_colour(r: Relation) -> Vec3 {
    match r {
        Relation::Ally => ALLY,
        Relation::Enemy => ENEMY,
        _ => NEUTRAL,
    }
}

/// How `owner`'s units stand to `player`: its own and allied ones ally, other teams enemy, ownerless neutral.
pub fn relation(view: &WorldView, player: sta_sim::PlayerId, owner: sta_sim::PlayerId) -> Relation {
    if owner == player {
        return Relation::Ally;
    }
    let Some(me) = view.player(player) else { return Relation::Neutral };
    if let Some((_, r)) = me.relations.iter().find(|(p, _)| *p == owner) {
        return *r;
    }
    match view.player(owner) {
        Some(o) if o.team == me.team => Relation::Ally,
        Some(_) => Relation::Enemy,
        None => Relation::Neutral,
    }
}

/// What a unit shows: the display colour (before the linear conversion) of each pass, if it exists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub inner: Option<Vec3>,
    pub outer: Option<Vec3>,
}

/// The four states [M]; `craft` false takes the non-craft rule [S]. `pulse` is [`pulse`] at the current phase.
pub fn look(selected: bool, hovered: bool, craft: bool, relation: Relation, team: Vec3, pulse: f32) -> Look {
    let inner = if selected {
        Some(if craft { relation_colour(relation) * pulse } else { team * DIM } * PROFILE_FACTOR)
    } else if hovered {
        Some(Vec3::ZERO)
    } else {
        None
    };
    let outer = if hovered {
        Some(team)
    } else if selected && craft {
        Some(team * DIM * PROFILE_FACTOR)
    } else {
        None
    };
    Look { inner, outer }
}

// ---------------------------------------------------------------- mesh

/// The bubble's mesh and its bounding radius (the farthest vertex from the origin).
#[derive(Resource, Default)]
pub struct BubbleArt {
    pub mesh: Option<Handle<Mesh>>,
    pub radius: f32,
    /// True while the authored ellipsoid stands in for the install's model.
    pub interim: bool,
    tried_for_mount: bool,
}

/// The install's bubble model flattened into one triangle list in render space (the legacy Z flip
/// applied), wound outward whatever the file's winding, with its bounding radius.
pub fn mesh_from_model(model: &sta_assets::model::Model) -> Option<(Mesh, f32)> {
    let mut hidden: BTreeSet<usize> = model.coarse_lods().into_iter().collect();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    'nodes: for (i, node) in model.nodes.iter().enumerate() {
        let Some(mesh) = node.mesh() else { continue };
        let mut up = Some(i);
        while let Some(n) = up {
            if hidden.contains(&n) {
                continue 'nodes;
            }
            up = model.nodes[n].parent;
        }
        let w = model.model_transform(i);
        let base = positions.len() as u32;
        positions.extend(mesh.positions.iter().map(|p| {
            let q = w.transform_point(*p);
            [q[0], q[1], -q[2]]
        }));
        for g in &mesh.groups {
            for f in &g.faces {
                if f.iter().any(|(p, _)| *p as usize >= mesh.positions.len()) {
                    continue;
                }
                indices.extend(f.iter().map(|(p, _)| base + p));
            }
        }
    }
    hidden.clear();
    if indices.len() < 3 {
        return None;
    }
    let radius = positions.iter().map(|p| Vec3::from(*p).length()).fold(0.0, f32::max);
    if !(radius.is_finite() && radius > 0.0) {
        return None;
    }
    // Signed volume: negative means the faces point inward; flip them.
    let volume: f32 = indices
        .chunks_exact(3)
        .map(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(positions[i as usize]));
            a.dot(b.cross(c))
        })
        .sum();
    if volume < 0.0 {
        for t in indices.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
    }
    indices.truncate(indices.len() / 3 * 3);
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    Some((mesh, radius))
}

/// The authored stand-in [I]: a unit sphere.
fn interim_mesh() -> Mesh {
    Sphere::new(1.0).mesh().uv(32, 18)
}

fn load_art(mut art: ResMut<BubbleArt>, profile: Option<Res<UiProfile>>, mounted: Option<Res<Mounted>>, mut meshes: ResMut<Assets<Mesh>>) {
    if !wanted(profile.as_deref()) {
        return;
    }
    if art.mesh.is_none() {
        art.mesh = Some(meshes.add(interim_mesh()));
        art.radius = 1.0;
        art.interim = true;
    }
    let Some(mounted) = mounted else { return };
    if mounted.is_changed() {
        art.tried_for_mount = false;
    }
    if art.tried_for_mount {
        return;
    }
    art.tried_for_mount = true;
    let loaded = sta_assets::find_model(&mounted.0, "select").and_then(|(path, fmt)| match sta_assets::load_model(&mounted.0, &path, fmt) {
        Ok(m) => mesh_from_model(&m),
        Err(err) => {
            warn!("{err}");
            None
        }
    });
    match loaded {
        Some((mesh, radius)) => {
            art.mesh = Some(meshes.add(mesh));
            art.radius = radius;
            art.interim = false;
        }
        None => {
            art.mesh = Some(meshes.add(interim_mesh()));
            art.radius = 1.0;
            art.interim = true;
        }
    }
}

// ---------------------------------------------------------------- systems

/// Damage pulse phases by unit, advanced on real time.
#[derive(Resource, Default)]
pub struct Pulses(pub BTreeMap<EntityId, f32>);

fn linear(display: Vec3) -> Vec4 {
    let c = Color::srgb(display.x, display.y, display.z).to_linear();
    Vec4::new(c.red, c.green, c.blue, 0.0)
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn sync_bubbles(
    mut commands: Commands,
    profile: Option<Res<UiProfile>>,
    selection: Res<Selection>,
    hovered: Option<Res<HoveredUnit>>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    time: Option<Res<Time<Real>>>,
    art: Res<BubbleArt>,
    ents: Res<SimEntities>,
    link: Res<SimLink>,
    mut pulses: ResMut<Pulses>,
    mut mats: ResMut<Assets<BubbleMaterial>>,
    units: Query<(Option<&ModelBounds>, Option<&SelectRadius>, Option<&TeamColour>, Option<&HullShare>)>,
    mut existing: Query<(Entity, &Bubble, &MeshMaterial3d<BubbleMaterial>, &mut Mesh3d, &mut Transform)>,
) {
    let mut want: BTreeMap<(EntityId, Pass), (Entity, Vec3, Vec3)> = BTreeMap::new(); // root, scale, colour
    if wanted(profile.as_deref()) && !crate::ui::presentation::cutscene_running(cine) && art.mesh.is_some() {
        let dt = time.map_or(0.0, |t| t.delta_secs());
        let hover = hovered.as_ref().and_then(|h| h.0);
        let ids: BTreeSet<EntityId> = selection.0.iter().copied().chain(hover).collect();
        let view = link.curr();
        pulses.0.retain(|id, _| selection.0.contains(id));
        for id in ids {
            let (Some(&root), Some(unit)) = (ents.0.get(&id), view.unit(id)) else { continue };
            let Ok((bounds, radius, team, hull)) = units.get(root) else { continue };
            let (selected, hovered) = (selection.0.contains(&id), hover == Some(id));
            let h = hull.map_or(1.0, |s| s.0).clamp(0.0, 1.0);
            let phase = pulses.0.entry(id).or_insert(0.0);
            if selected {
                *phase = (*phase + phase_rate(h) * dt) % std::f32::consts::TAU;
            }
            let team = team.map_or(Vec3::splat(0.9), |c| {
                let c = c.0.to_srgba();
                Vec3::new(c.red, c.green, c.blue)
            });
            let l = look(selected, hovered, true, relation(view, link.player(), unit.owner), team, pulse(h, *phase));
            let bounds = bounds.copied().unwrap_or_else(|| ModelBounds::fallback(radius.map_or(super::DEFAULT_RADIUS, |r| r.0)));
            let scale = inner_scale(shield_semi_axes(bounds), art.radius);
            if let Some(c) = l.inner {
                want.insert((id, Pass::Inner), (root, scale, c));
            }
            if let Some(c) = l.outer {
                want.insert((id, Pass::Outer), (root, scale * OUTER_SCALE, c));
            }
        }
    }
    let mut have: BTreeSet<(EntityId, Pass)> = BTreeSet::new();
    for (e, b, mat, mut mesh, mut tf) in &mut existing {
        let key = (b.unit, b.pass);
        let Some(&(_, scale, colour)) = want.get(&key) else {
            commands.entity(e).try_despawn();
            continue;
        };
        if !have.insert(key) {
            commands.entity(e).try_despawn();
            continue;
        }
        if tf.scale != scale {
            tf.scale = scale;
        }
        if let Some(handle) = art.mesh.as_ref()
            && mesh.0 != *handle
        {
            mesh.0 = handle.clone();
        }
        let colour = linear(colour);
        if mats.get(&mat.0).is_some_and(|m| m.colour != colour)
            && let Some(mut m) = mats.get_mut(&mat.0)
        {
            m.colour = colour;
        }
    }
    let Some(handle) = art.mesh.as_ref() else { return };
    for ((id, pass), (root, scale, colour)) in want {
        if have.contains(&(id, pass)) {
            continue;
        }
        commands.spawn((
            Name::new(if pass == Pass::Inner { "selection bubble" } else { "selection bubble ring" }),
            Bubble { unit: id, pass },
            ChildOf(root),
            Mesh3d(handle.clone()),
            MeshMaterial3d(mats.add(BubbleMaterial { colour: linear(colour), inner: pass == Pass::Inner })),
            Transform::from_scale(scale),
        ));
    }
}

#[cfg(test)]
mod gpu_tests;

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Vec3 = Vec3::splat(0.9);

    #[test]
    fn four_states_give_these_passes() {
        let l = |s, h| look(s, h, true, Relation::Ally, WHITE, DIM);
        assert_eq!(l(false, false), Look { inner: None, outer: None }, "neither: nothing");
        let sel = l(true, false);
        assert_eq!(sel.inner, Some(ALLY * DIM * PROFILE_FACTOR));
        assert_eq!(sel.outer, Some(WHITE * DIM * PROFILE_FACTOR), "selected: a dimmed ring, brighter than the disc");
        assert!(sel.outer.unwrap().x > sel.inner.unwrap().x);
        let hov = l(false, true);
        assert_eq!(hov.inner, Some(Vec3::ZERO), "hovered only: a black inner pass (it still writes depth)");
        assert_eq!(hov.outer, Some(WHITE), "full team colour, no profile factor");
        let both = l(true, true);
        assert_eq!(both.inner, sel.inner);
        assert_eq!(both.outer, Some(WHITE));
    }

    #[test]
    fn relation_colours_and_independent_ring() {
        let enemy = look(true, false, true, Relation::Enemy, Vec3::new(1.0, 0.0, 0.0), DIM);
        assert_eq!(enemy.inner, Some(ENEMY * DIM * PROFILE_FACTOR), "violet disc");
        assert_eq!(enemy.outer, Some(Vec3::new(1.0, 0.0, 0.0) * DIM * PROFILE_FACTOR), "the ring keeps the team colour");
        assert_eq!(look(true, false, true, Relation::Neutral, WHITE, DIM).inner, Some(NEUTRAL * DIM * PROFILE_FACTOR));
    }

    #[test]
    fn a_selected_non_craft_has_a_team_coloured_disc_and_no_dim_ring() {
        let l = look(true, false, false, Relation::Enemy, Vec3::new(0.0, 0.0, 1.0), 0.9);
        assert_eq!(l.inner, Some(Vec3::new(0.0, 0.0, 1.0) * DIM * PROFILE_FACTOR), "no relation colour, no pulse");
        assert_eq!(l.outer, None);
        assert_eq!(look(true, true, false, Relation::Ally, WHITE, 0.0).outer, Some(WHITE));
    }

    #[test]
    fn the_pulse_is_steady_at_full_hull_and_swings_with_damage() {
        assert_eq!(pulse(1.0, 1.2), DIM);
        let h = 0.556;
        let period = std::f32::consts::TAU / phase_rate(h);
        assert!((period - 1.0).abs() < 0.02, "about one second at this hull: {period}");
        let (hi, lo) = (pulse(h, std::f32::consts::FRAC_PI_2), pulse(h, -std::f32::consts::FRAC_PI_2));
        assert!((hi - lo - 2.0 * PULSE_AMPLITUDE * (1.0 - h)).abs() < 1e-6);
        assert!((hi - DIM - 0.0888).abs() < 1e-3);
    }

    #[test]
    fn size_follows_the_model_box_with_a_minimum() {
        let b = ModelBounds { min: Vec3::new(-20.0, -12.0, -10.0), max: Vec3::new(20.0, 12.0, 30.0) };
        let axes = shield_semi_axes(b);
        assert!((axes - Vec3::new(20.0, 12.0, 20.0) * BOX_TO_SHIELD).length() < 1e-4, "half the box times the factor, centred on the unit");
        let small = shield_semi_axes(ModelBounds::fallback(2.0));
        assert_eq!(small, Vec3::splat(MIN_AXIS), "the minimum");
        let s = inner_scale(Vec3::splat(30.0), 2.0);
        assert!((s - Vec3::splat(18.0)).length() < 1e-4, "axis times 1.2 over the mesh radius");
    }

    #[test]
    fn only_the_armada_two_stock_look_draws_it() {
        assert!(wanted(Some(&UiProfile::Armada2)));
        assert!(!wanted(Some(&UiProfile::Armada1)));
        assert!(!wanted(Some(&UiProfile::FleetOps)));
        assert!(!wanted(Some(&UiProfile::Lcars)));
        assert!(!wanted(None));
    }

    #[test]
    fn a_closed_model_is_wound_outward_and_measured() {
        // An authored tetrahedron wound inward: the loader flips it.
        let model = sta_assets::model::Model {
            format: sta_assets::model::SourceFormat::Sod,
            version: 1.0,
            materials: vec![],
            nodes: vec![sta_assets::model::Node {
                name: "m".into(),
                parent: None,
                children: vec![],
                transform: sta_assets::a2fmt::Matrix34::IDENTITY,
                kind: sta_assets::model::NodeKind::Mesh(sta_assets::model::Mesh {
                    positions: vec![[2.0, 0.0, 0.0], [-1.0, 1.0, 1.0], [-1.0, -1.0, 1.0], [-1.0, 0.0, -1.0]],
                    uvs: vec![[0.0, 0.0]],
                    groups: vec![sta_assets::model::Group {
                        material: String::new(),
                        blend: String::new(),
                        maps: Default::default(),
                        faces: vec![[(0, 0), (1, 0), (2, 0)], [(0, 0), (2, 0), (3, 0)], [(0, 0), (3, 0), (1, 0)], [(1, 0), (3, 0), (2, 0)]],
                    }],
                    two_sided: false,
                }),
            }],
            channels: vec![],
            flipbooks: vec![],
            warnings: vec![],
            pbr: None,
        };
        let (mesh, radius) = mesh_from_model(&model).expect("a mesh");
        assert!((radius - 2.0).abs() < 1e-5);
        let Some(Indices::U32(idx)) = mesh.indices().cloned() else { panic!("indices") };
        let p: Vec<Vec3> = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap().iter().map(|v| Vec3::from(*v)).collect();
        let vol: f32 = idx.chunks_exact(3).map(|t| p[t[0] as usize].dot(p[t[1] as usize].cross(p[t[2] as usize]))).sum();
        assert!(vol > 0.0, "outward");
    }

    /// Install-gated (`STA64_TEST_A2_INSTALL`, an Armada II root): the bubble model loads from the
    /// player's files and has a bounding radius.
    #[test]
    fn the_installed_bubble_model_loads() {
        let Some(root) = std::env::var_os("STA64_TEST_A2_INSTALL") else {
            eprintln!("SKIP: set STA64_TEST_A2_INSTALL to an Armada II root");
            return;
        };
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), None).expect("mount");
        let (path, fmt) = sta_assets::find_model(&vfs, "select").expect("the bubble model");
        let model = sta_assets::load_model(&vfs, &path, fmt).expect("loads");
        let (mesh, radius) = mesh_from_model(&model).expect("a mesh");
        assert!(radius > 0.0 && mesh.count_vertices() >= 4);
    }

    // The ECS rig: sync with authored data, time stepped by hand.
    mod ecs {
        use super::*;
        use sta_sim::command::{Command, LocalRouter};
        use sta_sim::{ContentKey, Fixed, PlayerId, TickOutput, UnitView, WorldSource};
        use bevy::time::TimeUpdateStrategy;
        use std::sync::Arc;

        struct Frozen(Arc<WorldView>);
        impl WorldSource for Frozen {
            fn view(&self) -> Arc<WorldView> {
                self.0.clone()
            }
            fn step(&mut self, _: &[Command]) -> TickOutput {
                TickOutput { view: self.0.clone(), events: Vec::new() }
            }
        }

        fn unit(id: u32, owner: u8) -> UnitView {
            UnitView {
                id: EntityId(id),
                def: ContentKey::NONE,
                owner: PlayerId(owner),
                pos: sta_sim::Vec3::ZERO,
                heading: (Fixed::ONE, Fixed::ZERO),
                speed: Fixed::ZERO,
                hull: [Fixed::from_int(1), Fixed::from_int(1)],
                shields: [[Fixed::ZERO, Fixed::ZERO]; 4],
                crew: None,
                energy: None,
                autonomy: None,
                flags: Default::default(),
                progress: None,
            }
        }

        fn rig(profile: UiProfile) -> (App, Entity) {
            let view = Arc::new(WorldView { units: vec![unit(1, 1)], ..Default::default() });
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_plugins(AssetPlugin::default())
                .init_asset::<Mesh>()
                .init_asset::<BubbleMaterial>()
                .insert_resource(profile)
                .init_resource::<Selection>()
                .init_resource::<SimEntities>()
                .init_resource::<BubbleArt>()
                .init_resource::<Pulses>()
                .insert_resource(SimLink::new(Box::new(Frozen(view)), Box::new(LocalRouter::new()), PlayerId(1)));
            let root = app
                .world_mut()
                .spawn((Transform::default(), TeamColour(Color::srgb(0.9, 0.9, 0.9)), HullShare(1.0), ModelBounds::fallback(20.0)))
                .id();
            app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(1), root);
            app.add_systems(Update, (load_art, sync_bubbles).chain());
            (app, root)
        }

        fn passes(app: &mut App) -> Vec<(Pass, Vec3)> {
            let mut q = app.world_mut().query::<(&Bubble, &Transform)>();
            let mut v: Vec<_> = q.iter(app.world()).map(|(b, t)| (b.pass, t.scale)).collect();
            v.sort_by_key(|(p, _)| *p);
            v
        }

        #[test]
        fn the_passes_exist_for_the_four_states_and_only_under_armada_two() {
            let (mut app, _) = rig(UiProfile::Armada2);
            app.update();
            assert!(passes(&mut app).is_empty(), "neither");
            app.world_mut().resource_mut::<Selection>().0 = vec![EntityId(1)];
            app.update();
            let sel = passes(&mut app);
            assert_eq!(sel.iter().map(|p| p.0).collect::<Vec<_>>(), [Pass::Inner, Pass::Outer], "selected: both, the ring dim");
            assert!((sel[1].1 / sel[0].1 - Vec3::splat(OUTER_SCALE)).length() < 1e-5, "the ring is 1.1 times the disc");
            app.world_mut().resource_mut::<Selection>().0.clear();
            app.world_mut().insert_resource(HoveredUnit(Some(EntityId(1))));
            app.update();
            assert_eq!(passes(&mut app).len(), 2, "hovered only: black disc and white ring");
            app.world_mut().insert_resource(HoveredUnit(None));
            app.update();
            assert!(passes(&mut app).is_empty());
            for other in [UiProfile::Armada1, UiProfile::FleetOps, UiProfile::Lcars] {
                let (mut app, _) = rig(other);
                app.world_mut().resource_mut::<Selection>().0 = vec![EntityId(1)];
                app.world_mut().insert_resource(HoveredUnit(Some(EntityId(1))));
                app.update();
                assert!(passes(&mut app).is_empty(), "{other:?} keeps its own markers");
            }
        }

        #[test]
        fn no_bubble_during_a_cineractive() {
            let (mut app, _) = rig(UiProfile::Armada2);
            app.world_mut().resource_mut::<Selection>().0 = vec![EntityId(1)];
            app.world_mut().insert_resource(crate::ui::presentation::CineractiveState { on: true, skippable: false });
            app.update();
            assert!(passes(&mut app).is_empty());
            app.world_mut().insert_resource(crate::ui::presentation::CineractiveState::default());
            app.update();
            assert_eq!(passes(&mut app).len(), 2);
        }

        #[test]
        fn the_inner_colour_pulses_on_the_time_stepped_by_hand() {
            let (mut app, root) = rig(UiProfile::Armada2);
            app.world_mut().entity_mut(root).insert(HullShare(0.556));
            app.world_mut().resource_mut::<Selection>().0 = vec![EntityId(1)];
            app.insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::ZERO));
            app.update();
            let inner = |app: &mut App| {
                let mut q = app.world_mut().query::<(&Bubble, &MeshMaterial3d<BubbleMaterial>)>();
                let m = q.iter(app.world()).find(|(b, _)| b.pass == Pass::Inner).map(|(_, m)| m.0.clone()).unwrap();
                app.world().resource::<Assets<BubbleMaterial>>().get(&m).unwrap().colour.x
            };
            // A quarter period in, the pulse is at its crest; three quarters in, at its trough.
            let quarter = std::time::Duration::from_secs_f32(0.25 / (phase_rate(0.556) / std::f32::consts::TAU));
            let at = |app: &mut App, d| {
                app.insert_resource(TimeUpdateStrategy::ManualDuration(d));
                app.update();
                inner(app)
            };
            let crest = at(&mut app, quarter);
            let trough = at(&mut app, quarter * 2);
            assert!(crest > trough, "{crest} {trough}");
            let to_display = |lin: f32| Color::linear_rgb(lin, 0.0, 0.0).to_srgba().red;
            let swing = to_display(crest) - to_display(trough);
            let expected = ALLY.x * 2.0 * PULSE_AMPLITUDE * (1.0 - 0.556) * PROFILE_FACTOR;
            assert!((swing - expected).abs() < 0.01, "{swing} against {expected}");
        }
    }
}
