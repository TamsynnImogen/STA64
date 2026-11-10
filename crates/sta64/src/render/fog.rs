//! RND-13: the fog of war and the shroud on the map.
//!
//! What the player has not got in sight is dimmed (the fog) and what it has never
//! had in sight is blacked out (the shroud), as Armada II Refit's overlays module
//! draws them over the terrain (`render3d/mods/overlays`, `Fog.asi`'s look; see
//! `fog.NOTICE.md`).
//!
//! - [`FogGrid`] is the data, with no renderer in it: one cell per [`CELL`] world
//!   units over the map, `visible` where a sensor of the local player's team reaches
//!   the cell's centre and `explored` where one ever has. It is built from a
//!   [`WorldView`] and the content's sensor ranges, the same rule as
//!   `sta_sim::sensors` (the views carry per-unit visibility only, not areas), and
//!   answers `visible_at` / `explored_at` for any map point. The minimap (UI-05) can
//!   draw its own fog and shroud from the [`FogState`] resource without a second
//!   computation.
//! - [`FogPlugin`] keeps [`FogState`] current while [`LocalPlayer`] is `Some`, a
//!   [`FogBounds`] resource says where the map is and a `SimLink` exists, and draws
//!   the two layers: a quad over the map each, textured with the grid's mask and
//!   shaded by `fog.wesl` (a cubic B-spline of the mask with a smoothstep edge, so a
//!   disc of cells reads as a circle).
//! - A view with no visibility data is omniscient (S3): nothing is fogged.
//!
//! The sim works in the map's left-handed space; the renderer mirrors Z, so the
//! quads are placed at `-z`.

use std::sync::Arc;

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset};
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, Extent3d, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use sta_content::ContentDb;
use sta_sim::sensors::DEFAULT_SENSOR;
use sta_sim::view::UnitFlags;
use sta_sim::{PlayerId, WorldView};

use crate::app::GameSet;
use crate::render::units::LocalPlayer;
use crate::sim_link::{ContentRes, SimLink};

/// World units per grid cell, unless the map is big enough to need [`MAX_CELLS`].
pub const CELL: f32 = 128.0;
/// Most cells along one side of the grid.
pub const MAX_CELLS: usize = 512;

/// Fog opacity over unseen ground (Refit: its `fog` texture material is 0.3 translucent).
const FOG_OPACITY: f32 = 0.7;
/// The fog layer's colour: a cold dark grey, as the grey of Refit's `fog` art on the bench.
const FOG_TINT: [f32; 3] = [0.004, 0.005, 0.01];
/// The edge of each layer, Refit's defaults (`FogThreshold`, `FogSoftness`, `ShroudSoftness`,
/// `FogRim`, `ShroudRim`, `RimWidth`).
const THRESHOLD: f32 = 0.5;
const FOG_SOFT: f32 = 0.2;
const SHROUD_SOFT: f32 = 0.1;
const RIM: f32 = 1.0;
const RIM_WIDTH: f32 = 0.25;
/// Sort bias of the layers among the blended things of the scene (nebula puffs, particles,
/// icons): both go after all of them, the shroud last, so what lies under fog or shroud is
/// dimmed or hidden whichever side of the map's middle it is on (Refit's `Step::Terrain`
/// draws them over the world, and the engine draws nothing it cannot see). A sort distance is
/// world units from the camera; a map is some thousands.
const FOG_SORT: f32 = 1.0e6;
const SHROUD_SORT: f32 = 2.0e6;
/// Height of the layers over the map's plane, so they do not z-fight a grid drawn at 0.
const LAYER_Y: f32 = 0.0;

/// Which layer of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// Not in sight now.
    Fog,
    /// Never in sight.
    Shroud,
}

/// Where the map is, in the sim's XZ plane: `min` and `max` corners. Whoever builds the
/// map inserts it (the bench's frames do; `MapDef::bounds` is the source).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct FogBounds {
    pub min: Vec2,
    pub max: Vec2,
}

/// The fog and shroud of the local player's view: what the minimap reads too.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct FogState {
    pub grid: FogGrid,
    /// How many times the grid has been recomputed (for tests and marks).
    pub updates: u32,
}

/// Which layers are drawn (Refit's `[Overlays] Fog`); the grid is kept either way.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FogSettings {
    pub fog: bool,
    pub shroud: bool,
}

impl Default for FogSettings {
    fn default() -> Self {
        FogSettings { fog: true, shroud: true }
    }
}

/// The visibility of every cell of the map for one player.
#[derive(Debug, Clone, PartialEq)]
pub struct FogGrid {
    /// The map's lowest corner `(x, z)` in sim space: cell `(0, 0)`'s.
    pub min: [f32; 2],
    /// A cell's side, world units.
    pub cell: f32,
    pub w: usize,
    pub h: usize,
    visible: Vec<bool>,
    explored: Vec<bool>,
}

impl FogGrid {
    /// An empty grid (nothing seen) over `min..max`.
    pub fn new(min: [f32; 2], max: [f32; 2]) -> FogGrid {
        let size = (max[0] - min[0]).max(max[1] - min[1]).max(1.0);
        let cell = CELL.max(size / MAX_CELLS as f32);
        let dim = |lo: f32, hi: f32| (((hi - lo) / cell).ceil() as usize).clamp(1, MAX_CELLS);
        let (w, h) = (dim(min[0], max[0]), dim(min[1], max[1]));
        FogGrid { min, cell, w, h, visible: vec![false; w * h], explored: vec![false; w * h] }
    }

    /// The cell of a sim-space point, if it is on the grid.
    #[allow(dead_code)] // read by the minimap (UI-05) and the tests
    pub fn cell_of(&self, x: f32, z: f32) -> Option<usize> {
        let (cx, cz) = ((x - self.min[0]) / self.cell, (z - self.min[1]) / self.cell);
        (cx >= 0.0 && cz >= 0.0 && (cx as usize) < self.w && (cz as usize) < self.h).then(|| cz as usize * self.w + cx as usize)
    }

    /// Is the point in sight now? Off the grid: no.
    #[allow(dead_code)] // read by the minimap (UI-05) and the tests
    pub fn visible_at(&self, x: f32, z: f32) -> bool {
        self.cell_of(x, z).is_some_and(|i| self.visible[i])
    }

    /// Has the point ever been in sight? Off the grid: no.
    #[allow(dead_code)] // read by the minimap (UI-05) and the tests
    pub fn explored_at(&self, x: f32, z: f32) -> bool {
        self.cell_of(x, z).is_some_and(|i| self.explored[i])
    }

    /// How many cells are in sight now / were ever.
    #[allow(dead_code)] // read by the minimap (UI-05) and the tests
    pub fn counts(&self) -> (usize, usize) {
        (self.visible.iter().filter(|v| **v).count(), self.explored.iter().filter(|v| **v).count())
    }

    /// Set one cell's state (tests of the minimap).
    #[cfg(test)]
    pub fn set_cell_for_test(&mut self, cx: usize, cz: usize, visible: bool, explored: bool) {
        self.visible[cz * self.w + cx] = visible;
        self.explored[cz * self.w + cx] = explored;
    }

    /// Everything in sight (an omniscient view).
    fn reveal_all(&mut self) {
        self.visible.fill(true);
        self.explored.fill(true);
    }

    /// The cells whose centre is within `r` of `(x, z)` are in sight (strictly, as
    /// `sta_sim::sensors`' overlap test).
    fn reveal(&mut self, x: f32, z: f32, r: f32) {
        let c = self.cell;
        let lo = |p: f32, o: f32| (((p - r - o) / c).floor() as isize).max(0);
        let (x0, z0) = (lo(x, self.min[0]), lo(z, self.min[1]));
        let x1 = ((((x + r - self.min[0]) / c).ceil()) as isize).min(self.w as isize - 1);
        let z1 = ((((z + r - self.min[1]) / c).ceil()) as isize).min(self.h as isize - 1);
        for cz in z0..=z1 {
            let dz = self.min[1] + (cz as f32 + 0.5) * c - z;
            for cx in x0..=x1 {
                let dx = self.min[0] + (cx as f32 + 0.5) * c - x;
                if dx * dx + dz * dz < r * r {
                    self.visible[cz as usize * self.w + cx as usize] = true;
                }
            }
        }
    }

    /// Recompute what is in sight for `player` from `view`; what has been seen stays explored.
    pub fn observe(&mut self, view: &WorldView, player: PlayerId, content: Option<&ContentDb>) {
        self.visible.fill(false);
        if view.visibility.is_empty() {
            self.reveal_all();
            return;
        }
        for (x, z, r) in sensor_discs(view, player, content) {
            self.reveal(x, z, r);
        }
        for (e, v) in self.explored.iter_mut().zip(&self.visible) {
            *e |= *v;
        }
    }

    /// One byte per cell, row by row from the map's low Z: 255 where the layer hides
    /// the cell, 0 where it shows it. This is the texture `fog.wesl` reads.
    pub fn mask(&self, layer: Layer) -> Vec<u8> {
        let shown = match layer {
            Layer::Fog => &self.visible,
            Layer::Shroud => &self.explored,
        };
        shown.iter().map(|s| if *s { 0 } else { 255 }).collect()
    }
}

/// The discs `(x, z, radius)` that `player`'s team sees through: each of its units
/// that has a sensor (`UnitDef::sensor`, or [`DEFAULT_SENSOR`] for units with no
/// definition; not projectiles, which the view marks by not being selectable).
pub fn sensor_discs(view: &WorldView, player: PlayerId, content: Option<&ContentDb>) -> Vec<(f32, f32, f32)> {
    let Some(team) = view.player(player).map(|p| p.team) else { return Vec::new() };
    view.units
        .iter()
        .filter(|u| view.player(u.owner).is_some_and(|o| o.team == team))
        .filter_map(|u| {
            let r = match content.and_then(|c| c.unit(u.def)) {
                Some(def) => def.sensor.to_f32(),
                None if u.flags.contains(UnitFlags::SELECTABLE) => DEFAULT_SENSOR as f32,
                None => 0.0,
            };
            (r > 0.0).then(|| (u.pos.x.to_f32(), u.pos.z.to_f32(), r))
        })
        .collect()
}

#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct FogParams {
    /// rgb the layer's colour; w its opacity.
    pub tint: Vec4,
    /// x the mask level of the edge, y its softness, z the rim's strength, w 1 / the rim's width.
    pub edge: Vec4,
}

/// One layer: the grid's mask over the map.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FogMaterial {
    #[uniform(0)]
    pub params: FogParams,
    #[texture(1)]
    #[sampler(2)]
    pub mask: Handle<Image>,
    /// Sort bias: [`FOG_SORT`] for the fog, [`SHROUD_SORT`] for the shroud.
    pub bias: f32,
}

impl Material for FogMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/fog.wesl").into_owned())
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    /// The two layers share a plane, so their sort distance ties, and a tie is broken by
    /// an order that varies from run to run; blends do not commute. The shroud goes over
    /// the fog, as in Refit's `Step::Terrain` (fog, then shroud).
    fn depth_bias(&self) -> f32 {
        self.bias
    }
    /// As stock: the depth test always passes and nothing is written, so the layer
    /// covers whatever is drawn under it, on both sides.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None::<Face>;
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_compare = Some(CompareFunction::Always);
            ds.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// A layer's quad and the mask image it samples.
#[derive(Component, Debug, Clone)]
pub struct FogLayer {
    pub layer: Layer,
    mask: Handle<Image>,
}

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "fog.wesl");
        app.init_resource::<FogSettings>().add_plugins(MaterialPlugin::<FogMaterial>::default()).add_systems(
            Update,
            update_fog
                .in_set(GameSet::Render)
                .run_if(resource_exists::<SimLink>)
                .run_if(resource_exists::<FogBounds>)
                .run_if(resource_exists::<Assets<FogMaterial>>),
        );
    }
}

fn mask_image(grid: &FogGrid, layer: Layer) -> Image {
    let mut img = Image::new(
        Extent3d { width: grid.w as u32, height: grid.h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        grid.mask(layer),
        TextureFormat::R8Unorm,
        RenderAssetUsages::default(),
    );
    img.sampler = ImageSampler::linear();
    img
}

/// A quad over the grid, in render space; uv `(u, v)` with `v` running with sim Z, so
/// texel row `r` is the cells at `z = min.z + r * cell`.
fn layer_mesh(grid: &FogGrid) -> Mesh {
    let (x0, z0) = (grid.min[0], grid.min[1]);
    let (x1, z1) = (x0 + grid.w as f32 * grid.cell, z0 + grid.h as f32 * grid.cell);
    let p = |x: f32, z: f32| [x, LAYER_Y, -z];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![p(x0, z0), p(x1, z0), p(x1, z1), p(x0, z1)])
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
        .with_inserted_indices(Indices::U32(vec![0, 2, 1, 0, 3, 2]))
}

fn params(layer: Layer) -> FogParams {
    match layer {
        Layer::Fog => FogParams {
            tint: Vec4::new(FOG_TINT[0], FOG_TINT[1], FOG_TINT[2], FOG_OPACITY),
            edge: Vec4::new(THRESHOLD, FOG_SOFT, RIM, 1.0 / RIM_WIDTH),
        },
        Layer::Shroud => FogParams {
            tint: Vec4::new(0.0, 0.0, 0.0, 1.0),
            edge: Vec4::new(THRESHOLD, SHROUD_SOFT, RIM, 1.0 / RIM_WIDTH),
        },
    }
}

/// Recompute the grid when the view, the player or the map changes; spawn the layers
/// once, then rewrite their masks.
#[allow(clippy::too_many_arguments)]
fn update_fog(
    mut commands: Commands,
    link: Res<SimLink>,
    local: Res<LocalPlayer>,
    bounds: Res<FogBounds>,
    content: Option<Res<ContentRes>>,
    settings: Res<FogSettings>,
    state: Option<ResMut<FogState>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut layers: Query<(Entity, &FogLayer, &mut Visibility)>,
    mut seen: Local<Option<(Arc<WorldView>, Option<PlayerId>)>>,
) {
    let Some(player) = local.0 else {
        // No local player: the whole map is shown.
        for (_, _, mut v) in &mut layers {
            *v = Visibility::Hidden;
        }
        return;
    };
    let curr = link.curr();
    let fresh = state.is_none() || !seen.as_ref().is_some_and(|(v, p)| Arc::ptr_eq(v, curr) && *p == local.0);
    let map_changed = state.as_ref().is_some_and(|s| s.grid.min != [bounds.min.x, bounds.min.y]);
    if fresh || map_changed {
        *seen = Some((curr.clone(), local.0));
        let (mut grid, updates) = match state.as_deref() {
            Some(s) if !map_changed => (s.grid.clone(), s.updates),
            _ => (FogGrid::new([bounds.min.x, bounds.min.y], [bounds.max.x, bounds.max.y]), 0),
        };
        grid.observe(curr, player, content.as_deref().map(|c| &*c.0));
        let respawn = state.is_none() || map_changed;
        let new = FogState { grid, updates: updates + 1 };
        if respawn {
            for (e, l, _) in &layers {
                images.remove(&l.mask);
                commands.entity(e).despawn();
            }
            for layer in [Layer::Fog, Layer::Shroud] {
                let mask = images.add(mask_image(&new.grid, layer));
                let mat = materials.add(FogMaterial { params: params(layer), mask: mask.clone(), bias: if layer == Layer::Shroud { SHROUD_SORT } else { FOG_SORT } });
                commands.spawn((
                    Name::new(format!("fog layer {layer:?}")),
                    FogLayer { layer, mask },
                    Mesh3d(meshes.add(layer_mesh(&new.grid))),
                    MeshMaterial3d(mat),
                    Transform::default(),
                    Visibility::Inherited,
                ));
            }
        } else {
            for (_, l, _) in &layers {
                if let Some(mut img) = images.get_mut(&l.mask) {
                    img.data = Some(new.grid.mask(l.layer));
                }
            }
        }
        commands.insert_resource(new);
    }
    for (_, l, mut v) in &mut layers {
        let on = match l.layer {
            Layer::Fog => settings.fog,
            Layer::Shroud => settings.shroud,
        };
        let want = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::view::{PlayerView, UnitFlags, UnitView};
    use sta_sim::{ContentKey, EntityId, Fixed, TeamId};

    fn fx(v: i32) -> Fixed {
        Fixed::from_int(v)
    }

    fn unit(id: u32, owner: u8, x: i32, z: i32) -> UnitView {
        UnitView {
            id: EntityId(id),
            def: ContentKey::NONE,
            owner: PlayerId(owner),
            pos: sta_sim::Vec3 { x: fx(x), y: fx(0), z: fx(z) },
            heading: (fx(0), fx(1)),
            speed: fx(0),
            hull: [fx(100), fx(100)],
            shields: [[fx(0), fx(0)]; 4],
            flags: UnitFlags::SELECTABLE,
            progress: None,
        }
    }

    fn player(id: u8, team: u8) -> PlayerView {
        PlayerView { id: PlayerId(id), team: TeamId(team), resources: vec![], officers: [fx(0); 2], eliminated: false, relations: vec![] }
    }

    fn view(units: Vec<UnitView>) -> WorldView {
        WorldView {
            players: vec![player(1, 1), player(2, 2)],
            units,
            // Not empty, so the view is not omniscient; what `visible_to` says does not matter here.
            visibility: vec![(PlayerId(1), vec![]), (PlayerId(2), vec![])],
            ..Default::default()
        }
    }

    fn grid() -> FogGrid {
        FogGrid::new([-4096.0, -4096.0], [4096.0, 4096.0])
    }

    #[test]
    fn a_sensor_clears_a_disc_of_cells_around_it() {
        let mut g = grid();
        g.observe(&view(vec![unit(1, 1, 0, 0)]), PlayerId(1), None);
        // The default sensor reaches 800: inside is seen, outside is not.
        assert!(g.visible_at(0.0, 0.0) && g.visible_at(700.0, 0.0) && g.visible_at(0.0, -700.0));
        assert!(!g.visible_at(900.0, 0.0) && !g.visible_at(0.0, 900.0));
        assert!(!g.visible_at(600.0, 600.0), "the corner of the square is outside the disc");
        assert!(!g.visible_at(1.0e6, 0.0), "off the map is never seen");
    }

    #[test]
    fn enemy_sensors_do_not_clear_my_fog() {
        let mut g = grid();
        g.observe(&view(vec![unit(1, 2, 0, 0)]), PlayerId(1), None);
        assert_eq!(g.counts(), (0, 0));
    }

    #[test]
    fn allies_share_vision() {
        let mut v = view(vec![unit(1, 2, 0, 0)]);
        v.players[1].team = TeamId(1);
        let mut g = grid();
        g.observe(&v, PlayerId(1), None);
        assert!(g.visible_at(0.0, 0.0));
    }

    #[test]
    fn what_was_seen_stays_explored_when_the_sensor_moves_on() {
        let mut g = grid();
        g.observe(&view(vec![unit(1, 1, 0, 0)]), PlayerId(1), None);
        g.observe(&view(vec![unit(1, 1, 3000, 0)]), PlayerId(1), None);
        assert!(!g.visible_at(0.0, 0.0) && g.explored_at(0.0, 0.0));
        assert!(g.visible_at(3000.0, 0.0) && g.explored_at(3000.0, 0.0));
        let m = g.mask(Layer::Fog);
        let s = g.mask(Layer::Shroud);
        assert_eq!(m.len(), g.w * g.h);
        let at = |x: f32, z: f32| g.cell_of(x, z).unwrap();
        assert_eq!((m[at(0.0, 0.0)], s[at(0.0, 0.0)]), (255, 0), "fogged but not shrouded");
        assert_eq!((m[at(3000.0, 0.0)], s[at(3000.0, 0.0)]), (0, 0), "in sight");
        assert_eq!((m[at(-3000.0, 0.0)], s[at(-3000.0, 0.0)]), (255, 255), "never seen");
    }

    #[test]
    fn a_view_without_visibility_data_shows_everything() {
        let mut g = grid();
        g.observe(&WorldView::default(), PlayerId(1), None);
        assert_eq!(g.counts(), (g.w * g.h, g.w * g.h));
    }

    #[test]
    fn a_player_the_view_does_not_know_sees_nothing() {
        let mut g = grid();
        g.observe(&view(vec![unit(1, 1, 0, 0)]), PlayerId(9), None);
        assert_eq!(g.counts(), (0, 0));
    }

    #[test]
    fn projectiles_have_no_sensor_but_content_ranges_are_used() {
        let mut p = unit(1, 1, 0, 0);
        p.flags = UnitFlags::default();
        assert!(sensor_discs(&view(vec![p]), PlayerId(1), None).is_empty());
        let db = sta_content::fake::db();
        let key = (0..64).map(ContentKey).find(|k| db.unit(*k).is_some_and(|d| d.sensor > Fixed::ZERO)).expect("fake content has a sensor");
        let mut u = unit(2, 1, 5, 6);
        u.def = key;
        let d = sensor_discs(&view(vec![u]), PlayerId(1), Some(&db));
        assert_eq!(d, vec![(5.0, 6.0, db.unit(key).unwrap().sensor.to_f32())]);
    }

    #[test]
    fn big_maps_coarsen_the_cell_instead_of_growing_the_grid() {
        let g = FogGrid::new([0.0, 0.0], [102_400.0, 51_200.0]);
        assert_eq!((g.w, g.h), (MAX_CELLS, MAX_CELLS / 2));
        assert_eq!(g.cell, 200.0);
        let small = FogGrid::new([0.0, 0.0], [1000.0, 300.0]);
        assert_eq!((small.cell, small.w, small.h), (CELL, 8, 3));
    }

    #[test]
    fn fog_shader_validates() {
        crate::render::fx::validate_wgsl(include_str!("fog.wesl"));
    }
}
