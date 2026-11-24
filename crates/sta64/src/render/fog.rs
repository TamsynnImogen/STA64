//! RND-13: the fog of war and the shroud on the map.
//!
//! What the player has not got in sight is dimmed (the fog) and what it has never
//! had in sight is blacked out (the shroud), as Armada II Refit's overlays module
//! draws them over the terrain (`render3d/mods/overlays`, `Fog.asi`'s look; see
//! `fog.NOTICE.md`).
//!
//! - [`FogGrid`] is the data, with no renderer in it: one cell per [`CELL`] world
//!   units over the map, with samples of the distance to the local team's sensor coverage.
//!   The sign classifies its cell centre as visible or explored. The distance also
//!   lets the shader place the contour between cells. It is built from a
//!   [`WorldView`] and the content's sensor ranges, the same rule as
//!   `sta_sim::sensors` (the views carry per-unit visibility only, not areas), and
//!   answers `visible_at` / `explored_at` for any map point. The minimap (UI-05) can
//!   draw its own fog and shroud from the [`FogState`] resource without a second
//!   computation.
//! - [`FogPlugin`] keeps [`FogState`] current while [`LocalPlayer`] is `Some`, a
//!   [`FogBounds`] resource says where the map is and a `SimLink` exists, and draws
//!   the two layers: a quad over the map each, textured with the grid's mask and
//!   shaded by `fog.wesl` (a cubic B-spline of the distance samples with a smoothstep
//!   edge).
//! - A view with no visibility data is omniscient (S3): nothing is fogged.
//!
//! The sim works in the map's left-handed space; the renderer mirrors Z, so the
//! quads are placed at `-z`.

use std::sync::Arc;

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
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
use crate::ui::radar_fog::{DEFAULT_FOG, DEFAULT_SHROUD, read_pattern};
use crate::viewer::Mounted;

/// World units per grid cell, unless the map is big enough to need [`MAX_CELLS`].
pub const CELL: f32 = 128.0;
/// Most cells along one side of the grid.
pub const MAX_CELLS: usize = 512;

/// Fog opacity over unseen ground (Refit: its `fog` texture material is 0.3 translucent).
const FOG_OPACITY: f32 = 0.7;
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

/// Visibility samples for one player's map. Bytes below 128 are inside the
/// sensor contour. The bytes fade from 0 to 255 over a narrow band around it.
#[derive(Debug, Clone, PartialEq)]
pub struct FogGrid {
    /// The map's lowest corner `(x, z)` in sim space: cell `(0, 0)`'s.
    pub min: [f32; 2],
    /// A cell's side, world units.
    pub cell: f32,
    pub w: usize,
    pub h: usize,
    visible: Vec<u8>,
    explored: Vec<u8>,
    /// The last remembering update's sight, not yet in `explored`.
    pending: Vec<u8>,
}

impl FogGrid {
    /// An empty grid (nothing seen) over `min..max`.
    pub fn new(min: [f32; 2], max: [f32; 2]) -> FogGrid {
        let size = (max[0] - min[0]).max(max[1] - min[1]).max(1.0);
        let cell = CELL.max(size / MAX_CELLS as f32);
        let dim = |lo: f32, hi: f32| (((hi - lo) / cell).ceil() as usize).clamp(1, MAX_CELLS);
        let (w, h) = (dim(min[0], max[0]), dim(min[1], max[1]));
        FogGrid { min, cell, w, h, visible: vec![255; w * h], explored: vec![255; w * h], pending: vec![255; w * h] }
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
        self.cell_of(x, z).is_some_and(|i| self.visible[i] < 128)
    }

    /// Has the point ever been in sight? Off the grid: no.
    #[allow(dead_code)] // read by the minimap (UI-05) and the tests
    pub fn explored_at(&self, x: f32, z: f32) -> bool {
        self.cell_of(x, z).is_some_and(|i| self.explored[i] < 128)
    }

    /// How many cells are in sight now / were ever.
    #[allow(dead_code)] // read by the minimap (UI-05) and the tests
    pub fn counts(&self) -> (usize, usize) {
        (self.visible.iter().filter(|&&v| v < 128).count(), self.explored.iter().filter(|&&v| v < 128).count())
    }

    /// Set one cell's state (tests of the minimap).
    #[cfg(test)]
    pub fn set_cell_for_test(&mut self, cx: usize, cz: usize, visible: bool, explored: bool) {
        self.visible[cz * self.w + cx] = if visible { 0 } else { 255 };
        self.explored[cz * self.w + cx] = if explored { 0 } else { 255 };
    }

    #[cfg(test)]
    pub(crate) fn set_sensor_for_test(&mut self, centre: Vec2, radius: f32) {
        self.reveal(centre.x, centre.y, radius);
        self.explored.copy_from_slice(&self.visible);
    }

    /// Everything in sight (an omniscient view).
    fn reveal_all(&mut self) {
        self.visible.fill(0);
        self.explored.fill(0);
    }

    /// Stamp signed distances to the sensor's circle near its edge. Binary
    /// texels encode only which side a centre fell on, so filtering those can
    /// still put the contour as much as half a cell away from the radius.
    fn reveal(&mut self, x: f32, z: f32, r: f32) {
        let c = self.cell;
        // Include all the samples the cubic filter uses along the contour.
        let band = 2.0 * c;
        let outer = r + band;
        let outer2 = outer * outer;
        let inner = (r - band).max(0.0);
        let inner2 = inner * inner;
        let r2 = r * r;
        let lo = |p: f32, o: f32| (((p - outer - o) / c).floor() as isize).max(0);
        let hi = |p: f32, o: f32, cells: usize| {
            (((p + outer - o) / c).ceil() as isize).min(cells as isize - 1)
        };
        let (x0, z0) = (lo(x, self.min[0]), lo(z, self.min[1]));
        let (x1, z1) = (hi(x, self.min[0], self.w), hi(z, self.min[1], self.h));
        for cz in z0..=z1 {
            let dz = self.min[1] + (cz as f32 + 0.5) * c - z;
            for cx in x0..=x1 {
                let dx = self.min[0] + (cx as f32 + 0.5) * c - x;
                let d2 = dx * dx + dz * dz;
                if d2 >= outer2 {
                    continue;
                }
                let cell = &mut self.visible[cz as usize * self.w + cx as usize];
                if *cell == 0 || d2 < inner2 {
                    *cell = 0;
                    continue;
                }
                let edge = ((d2.sqrt() - r) / band * 127.5 + 127.5).round() as u8;
                // Keep the strict centre test, even when quantization rounds
                // a centre just inside the radius to the other side.
                let edge = if d2 < r2 { edge.min(127) } else { edge.max(128) };
                *cell = (*cell).min(edge);
            }
        }
    }

    /// Recompute what is in sight for `player` from `view`; what has been seen stays explored.
    #[cfg(test)]
    pub fn observe(&mut self, view: &WorldView, player: PlayerId, content: Option<&ContentDb>) {
        self.observe_with(view, player, content, true);
    }

    /// As [`observe`](Self::observe); with `remember` false (a cutscene runs) this update's sight is
    /// not remembered. Interim rule, grade: stated by a player (2026-11-22), not measured on the
    /// original: sight gained during a cutscene does not become explored ground. Whether the
    /// original does this by a general rule or by a mission script call is not established (the
    /// script call for the shroud stays unwired).
    ///
    /// The client learns that a cutscene began one system later than the view that came with it, so
    /// an update's sight is held back and folded into the explored memory by the next update that
    /// still runs outside a cutscene; an update inside one drops it. An omniscient view stays fully
    /// explored.
    pub fn observe_with(&mut self, view: &WorldView, player: PlayerId, content: Option<&ContentDb>, remember: bool) {
        self.visible.fill(255);
        if view.visibility.is_empty() {
            self.reveal_all();
            self.pending.fill(255);
            return;
        }
        for (x, z, r) in sensor_discs(view, player, content) {
            self.reveal(x, z, r);
        }
        if remember {
            for (e, p) in self.explored.iter_mut().zip(&self.pending) {
                *e = (*e).min(*p);
            }
            self.pending.copy_from_slice(&self.visible);
        } else {
            self.pending.fill(255);
        }
    }

    /// This grid as the player sees it: what is in sight counts as explored (the memory takes it in
    /// one update later, see [`observe_with`](Self::observe_with)), and UI-46's lifting (presentation
    /// only; `self` keeps the real memory). Lifting the shroud counts every cell as explored; lifting
    /// the fog counts every explored cell as in sight, so with both lifted everything is in sight,
    /// and with the fog alone the unexplored ground stays black.
    pub fn lifted(&self, fog: bool, shroud: bool) -> FogGrid {
        let mut g = self.clone();
        for (e, v) in g.explored.iter_mut().zip(&g.visible) {
            *e = (*e).min(*v);
        }
        if shroud {
            g.explored.fill(0);
        }
        if fog {
            for (v, e) in g.visible.iter_mut().zip(&g.explored) {
                *v = (*v).min(*e);
            }
        }
        g
    }

    /// One distance byte per cell, row by row from the map's low Z. The shader
    /// interpolates the contour at 127.5 between the clear and covered samples.
    pub fn mask(&self, layer: Layer) -> Vec<u8> {
        match layer {
            Layer::Fog => self.visible.clone(),
            Layer::Shroud => self.explored.clone(),
        }
    }
}

/// The discs `(x, z, radius)` that `player`'s team sees through: each of its units
/// that has a sensor (`UnitDef::sensor`, or [`DEFAULT_SENSOR`] for units with no
/// definition; not projectiles, which the view marks by not being selectable).
pub fn sensor_discs(view: &WorldView, player: PlayerId, content: Option<&ContentDb>) -> Vec<(f32, f32, f32)> {
    let Some(me) = view.player(player) else { return Vec::new() };
    // Sight is shared between mutual allies only (SIM-63: the world view's
    // per-player relations, which carry mission-set relations).
    let ally = |from: &sta_sim::view::PlayerView, to: PlayerId| {
        from.id == to
            || match from.relations.iter().find(|&&(id, _)| id == to) {
                Some(&(_, r)) => r == sta_sim::view::Relation::Ally,
                // A view without relation rows (hand-made, legacy): same team.
                None => view.player(to).is_some_and(|t| t.team == from.team),
            }
    };
    let upgrades = content.map(|db| sta_sim::ship_upgrades::ShipUpgrades::from_defs(db,
        view.units.iter()
            .filter(|u| u.hull[0] > sta_sim::Fixed::ZERO && !u.flags.contains(UnitFlags::DYING)
                && u.progress.is_none_or(|p| p >= sta_sim::Fixed::ONE))
            .map(|u| (u.owner, u.def))
    ));
    view.units
        .iter()
        .filter(|u| view.player(u.owner).is_some_and(|o| ally(me, o.id) && ally(o, player)))
        .filter_map(|u| {
            let r = match content.and_then(|c| c.unit(u.def)) {
                Some(def) => def.sensor.to_f32(),
                None if u.flags.contains(UnitFlags::SELECTABLE) => DEFAULT_SENSOR as f32,
                None => 0.0,
            };
            let r = r * upgrades.as_ref().map_or(1.0, |upgrades| upgrades.sensor_multiplier(u.owner).to_f32());
            (r > 0.0).then(|| (u.pos.x.to_f32(), u.pos.z.to_f32(), r))
        })
        .collect()
}

#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct FogParams {
    /// rgb multiplies the layer's art (white); w its opacity.
    pub tint: Vec4,
    /// xy the art's uv per unit of the layer's uv, zw its offset: one art texel per cell, the grid centred in it.
    pub pat: Vec4,
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
    /// The layer's art: the mounted mod's `Gfog` / `Gshroud`, or one texel of the stock colour.
    #[texture(3)]
    #[sampler(4)]
    pub pattern: Handle<Image>,
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

/// The layers' art: `Gfog` and `Gshroud` of the mounted mod (decoded by
/// `ui::radar_fog::read_pattern`, the radar's loader), each a 1 x 1 texel of Fleet Ops'
/// colour where the data has none. `size` is the textures' sides in texels.
#[derive(Resource, Debug, Clone)]
pub struct FogPatterns {
    pub fog: (Handle<Image>, Vec2),
    pub shroud: (Handle<Image>, Vec2),
    /// Bumped when the textures are replaced, so the layers are rebuilt.
    generation: u32,
}

impl FogPatterns {
    fn of(&self, layer: Layer) -> &(Handle<Image>, Vec2) {
        match layer {
            Layer::Fog => &self.fog,
            Layer::Shroud => &self.shroud,
        }
    }
}

/// An art texture: repeating, sRGB, opaque (the layer's opacity is the layer's own).
fn pattern_image(vfs: Option<&sta_vfs::Vfs>, name: &str, default: [u8; 3]) -> (Image, Vec2) {
    let (w, h, mut px) = match vfs.and_then(|v| read_pattern(v, name)) {
        Some(p) if p.width > 0 && p.height > 0 => (p.width, p.height, p.pixels),
        _ => (1, 1, vec![default[0], default[1], default[2], 255]),
    };
    for a in px.chunks_exact_mut(4) {
        a[3] = 255;
    }
    let mut img = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    (img, Vec2::new(w as f32, h as f32))
}

/// (Re)load the art when the mounted install changes (or once, with the fallback colours).
fn load_patterns(
    mut commands: Commands,
    vfs: Option<Res<Mounted>>,
    old: Option<Res<FogPatterns>>,
    mut images: ResMut<Assets<Image>>,
) {
    if old.is_some() && !vfs.as_ref().is_some_and(|v| v.is_changed()) {
        return;
    }
    let vfs = vfs.as_ref().map(|v| &v.0);
    let (fog, fog_size) = pattern_image(vfs, "Gfog", DEFAULT_FOG);
    let (shroud, shroud_size) = pattern_image(vfs, "Gshroud", DEFAULT_SHROUD);
    commands.insert_resource(FogPatterns {
        fog: (images.add(fog), fog_size),
        shroud: (images.add(shroud), shroud_size),
        generation: old.map_or(0, |o| o.generation + 1),
    });
}

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "fog.wesl");
        app.add_plugins(crate::render::reveal::RevealPlugin);
        app.init_resource::<FogSettings>().add_plugins(MaterialPlugin::<FogMaterial>::default()).add_systems(
            Update,
            (load_patterns.run_if(resource_exists::<Assets<Image>>), update_fog)
                .chain()
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

fn params(layer: Layer, grid: &FogGrid, art: Vec2) -> FogParams {
    // One art texel per cell, the grid centred in the art (stock's sprite is as large as its grid).
    let grid_texels = Vec2::new(grid.w as f32, grid.h as f32);
    let pat = grid_texels / art;
    let off = (Vec2::ONE - pat) * 0.5;
    let pat = Vec4::new(pat.x, pat.y, off.x, off.y);
    match layer {
        Layer::Fog => FogParams {
            tint: Vec4::new(1.0, 1.0, 1.0, FOG_OPACITY),
            pat,
            edge: Vec4::new(THRESHOLD, FOG_SOFT, RIM, 1.0 / RIM_WIDTH),
        },
        Layer::Shroud => FogParams {
            tint: Vec4::new(1.0, 1.0, 1.0, 1.0),
            pat,
            edge: Vec4::new(THRESHOLD, SHROUD_SOFT, RIM, 1.0 / RIM_WIDTH),
        },
    }
}

/// What `update_fog` remembers between frames (one `Local`: a system takes at most 16 parameters).
#[derive(Default)]
struct Memo {
    seen: Option<(Arc<WorldView>, Option<PlayerId>)>,
    art_generation: Option<u32>,
    /// What the sim's sensors have shown so far, whatever is lifted (UI-46); `FogState` holds the lifted copy.
    memory: Option<FogGrid>,
    shown_reveal: Option<(bool, bool)>,
    shown_cutscene: Option<bool>,
}

type MatchFog<'w> = (
    Option<Res<'w, crate::game::net_match::NetGame>>,
    Option<Res<'w, crate::game::flow::ActiveMatch>>,
    Option<Res<'w, crate::editor::Editing>>,
);

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
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    patterns: Option<Res<FogPatterns>>,
    state: Option<ResMut<FogState>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut layers: Query<(Entity, &FogLayer, &mut Visibility)>,
    reveal: Option<Res<crate::render::reveal::Reveal>>,
    (net, active, editor): MatchFog<'_>,
    mut memo: Local<Memo>,
) {
    let Memo { seen, art_generation, memory, shown_reveal, shown_cutscene } = &mut *memo;
    let cutscene = cine.as_deref().is_some_and(|c| c.on);
    let Some(patterns) = patterns else { return };
    let Some(player) = local.0 else {
        // No local player: the whole map is shown.
        for (_, _, mut v) in &mut layers {
            *v = Visibility::Hidden;
        }
        return;
    };
    let curr = link.real_curr();
    let lifted = if net.is_some() {
        (false, false)
    } else if editor.is_some() {
        (true, true)
    } else {
        let (fog, shroud) = active.as_deref().map_or((false, false), |active| active.fog.lifted());
        (fog || reveal.as_deref().is_some_and(|r| r.fog), shroud || reveal.as_deref().is_some_and(|r| r.shroud))
    };
    let fresh = state.is_none() || *shown_reveal != Some(lifted) || *shown_cutscene != Some(cutscene) || *art_generation != Some(patterns.generation) || !seen.as_ref().is_some_and(|(v, p)| Arc::ptr_eq(v, curr) && *p == local.0);
    let map_changed = memory.as_ref().is_some_and(|m| m.min != [bounds.min.x, bounds.min.y]);
    if fresh || map_changed {
        *seen = Some((curr.clone(), local.0));
        *shown_reveal = Some(lifted);
        *shown_cutscene = Some(cutscene);
        let (mut grid, updates) = match (state.as_deref(), memory.take()) {
            (Some(s), Some(m)) if !map_changed => (m, s.updates),
            _ => (FogGrid::new([bounds.min.x, bounds.min.y], [bounds.max.x, bounds.max.y]), 0),
        };
        // Editing is presentation-only: leave both explored and pending sight intact.
        // In particular, observe_with(..., false) would discard the last play frame's
        // pending sight when entering the editor.
        if editor.is_none() {
            // During a cutscene the layers are hidden anyway and the memory does not take
            // the sight in; see `FogGrid::observe_with`.
            grid.observe_with(curr, player, content.as_deref().map(|c| &*c.0), !cutscene);
        }
        let shown = grid.lifted(lifted.0, lifted.1);
        *memory = Some(grid);
        let art_changed = *art_generation != Some(patterns.generation);
        *art_generation = Some(patterns.generation);
        let respawn = state.is_none() || map_changed || art_changed;
        let new = FogState { grid: shown, updates: updates + 1 };
        if respawn {
            for (e, l, _) in &layers {
                images.remove(&l.mask);
                commands.entity(e).despawn();
            }
            for layer in [Layer::Fog, Layer::Shroud] {
                let mask = images.add(mask_image(&new.grid, layer));
                let mat = materials.add(FogMaterial { params: params(layer, &new.grid, patterns.of(layer).1), mask: mask.clone(), pattern: patterns.of(layer).0.clone(), bias: if layer == Layer::Shroud { SHROUD_SORT } else { FOG_SORT } });
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
        // Not in a cineractive: the cinematic view draws no fog or shroud (Refit, static).
        let on = on && !cutscene;
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
            crew: None,
            energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE,
            progress: None,
        }
    }

    fn player(id: u8, team: u8) -> PlayerView {
        PlayerView { crew: None, id: PlayerId(id), team: TeamId(team), resources: vec![], officers: [fx(0); 2], eliminated: false, relations: vec![] }
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

    // The shader's four bilinear taps implement these separable cubic weights.
    fn reconstructed_mask(g: &FogGrid, mask: &[u8], pos: Vec2) -> f32 {
        fn weights(f: f32) -> [f32; 4] {
            let f2 = f * f;
            let f3 = f2 * f;
            [
                (1.0 - f).powi(3) / 6.0,
                (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0,
                (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0,
                f3 / 6.0,
            ]
        }
        let texel = (pos - Vec2::from_array(g.min)) / g.cell - Vec2::splat(0.5);
        let base = texel.floor();
        let [u, v] = (texel - base).to_array();
        let mut value = 0.0;
        for (row, wz) in weights(v).into_iter().enumerate() {
            for (col, wx) in weights(u).into_iter().enumerate() {
                let x = base.x as usize + col - 1;
                let z = base.y as usize + row - 1;
                value += f32::from(mask[z * g.w + x]) * wx * wz;
            }
        }
        value
    }

    #[test]
    fn fog_contour_follows_a_sensor_radius_between_grid_centres() {
        let radius = DEFAULT_SENSOR as f32;
        // Neither sensor position is aligned to the grid. Test both sides of
        // a small step so motion smaller than a cell is represented too.
        for (x, z) in [(37, -83), (54, -83)] {
            let mut g = grid();
            g.observe(&view(vec![unit(1, 1, x, z)]), PlayerId(1), None);
            let shown = g.lifted(false, false);
            let mask = shown.mask(Layer::Fog);
            assert_eq!(mask, shown.mask(Layer::Shroud));
            let centre = Vec2::new(x as f32, z as f32);
            for i in 0..64 {
                let direction = Vec2::from_angle(i as f32 * std::f32::consts::TAU / 64.0);
                let inner = centre + direction * (radius - g.cell / 12.0);
                let outer = centre + direction * (radius + g.cell / 12.0);
                assert!(reconstructed_mask(&g, &mask, inner) < 127.5, "inside at angle {i}");
                assert!(reconstructed_mask(&g, &mask, outer) > 127.5, "outside at angle {i}");
            }
        }
    }

    #[test]
    fn a_cell_centre_exactly_on_the_sensor_radius_stays_hidden() {
        let mut g = grid();
        g.reveal(64.0, 64.0, 128.0);
        assert!(g.visible_at(64.0, 64.0));
        assert!(!g.visible_at(192.0, 64.0));
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
    fn mission_relations_decide_shared_vision_mutual_ally_only() {
        use sta_sim::view::Relation;
        let mut v = view(vec![unit(1, 2, 0, 0)]);
        v.players[0].relations = vec![(PlayerId(2), Relation::Ally)];
        v.players[1].relations = vec![(PlayerId(1), Relation::Enemy)];
        let mut g = grid();
        g.observe(&v, PlayerId(1), None);
        assert_eq!(g.counts(), (0, 0), "one-sided Ally shares nothing");
        v.players[1].relations = vec![(PlayerId(1), Relation::Ally)];
        g.observe(&v, PlayerId(1), None);
        assert!(g.visible_at(0.0, 0.0), "mutual Ally shares sight across teams");
    }

    #[test]
    fn what_was_seen_stays_explored_when_the_sensor_moves_on() {
        let mut g = grid();
        g.observe(&view(vec![unit(1, 1, 0, 0)]), PlayerId(1), None);
        g.observe(&view(vec![unit(1, 1, 3000, 0)]), PlayerId(1), None);
        // The picture is the shown copy: the memory takes the newest sight in one update later.
        let g = g.lifted(false, false);
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
    fn sight_during_a_cutscene_is_not_remembered() {
        let (home, away) = (view(vec![unit(1, 1, 0, 0)]), view(vec![unit(1, 1, 3000, 0)]));
        let mut g = grid();
        g.observe(&home, PlayerId(1), None);
        g.observe(&home, PlayerId(1), None);
        let before = g.counts().1;
        g.observe_with(&away, PlayerId(1), None, false);
        g.observe_with(&away, PlayerId(1), None, false);
        assert!(g.visible_at(3000.0, 0.0), "what is in sight is still in sight");
        assert!(!g.explored_at(3000.0, 0.0));
        assert_eq!(g.counts().1, before, "the explored set is unchanged");
        assert!(g.lifted(false, false).explored_at(3000.0, 0.0), "the shown copy has what is in sight");
        g.observe(&home, PlayerId(1), None);
        g.observe(&home, PlayerId(1), None);
        assert!(!g.explored_at(3000.0, 0.0), "after the cutscene only what is seen from where the units are counts");
    }

    #[test]
    fn the_update_that_still_reads_the_cutscene_as_off_is_not_remembered() {
        // The view that moves the units arrives with the cutscene start; the first update still sees the state off.
        let (home, away) = (view(vec![unit(1, 1, 0, 0)]), view(vec![unit(1, 1, 3000, 0)]));
        let mut g = grid();
        g.observe(&home, PlayerId(1), None);
        g.observe(&away, PlayerId(1), None);
        g.observe_with(&away, PlayerId(1), None, false);
        g.observe_with(&away, PlayerId(1), None, false);
        assert!(!g.explored_at(3000.0, 0.0));
        assert!(g.explored_at(0.0, 0.0));
    }

    #[test]
    fn ground_passed_in_one_update_outside_a_cutscene_is_explored_by_the_next() {
        let (home, away) = (view(vec![unit(1, 1, 0, 0)]), view(vec![unit(1, 1, 3000, 0)]));
        let mut g = grid();
        g.observe(&home, PlayerId(1), None);
        g.observe(&away, PlayerId(1), None);
        g.observe(&home, PlayerId(1), None);
        assert!(g.explored_at(3000.0, 0.0));
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

    fn texel(img: &Image) -> [u8; 3] {
        let d = img.data.as_ref().unwrap();
        [d[0], d[1], d[2]]
    }

    #[test]
    fn without_data_the_layers_use_the_stock_colours() {
        let (shroud, size) = pattern_image(None, "Gshroud", DEFAULT_SHROUD);
        assert_eq!((texel(&shroud), size), (DEFAULT_SHROUD, Vec2::ONE));
        let (fog, _) = pattern_image(None, "Gfog", DEFAULT_FOG);
        assert_eq!(texel(&fog), DEFAULT_FOG);
    }

    /// Install-gated: the shroud's art is the mounted mod's own `Gshroud.tga`.
    #[test]
    fn the_shroud_art_comes_from_the_mods_gshroud() {
        let roots = std::path::Path::new("/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots");
        let Ok(vfs) = sta_vfs::Vfs::mount_install(roots, Some("Fleet Ops 4.0")) else { return };
        let (img, size) = pattern_image(Some(&vfs), "Gshroud", [255, 0, 255]);
        assert_ne!(texel(&img), [255, 0, 255], "the fallback colour is not used when the mod has the texture");
        let mean = crate::ui::radar_fog::mean_colour(img.data.as_ref().unwrap()).unwrap();
        assert_eq!(mean, [30, 36, 50]);
        assert!(size.x >= 1.0 && size.y >= 1.0);
    }

    #[test]
    fn the_art_is_one_texel_per_cell_with_the_grid_centred() {
        let g = FogGrid::new([0.0, 0.0], [4.0 * CELL, 2.0 * CELL]);
        let p = params(Layer::Shroud, &g, Vec2::splat(8.0)).pat;
        assert_eq!(p, Vec4::new(0.5, 0.25, 0.25, 0.375));
    }

    #[test]
    fn fog_shader_validates() {
        crate::render::fx::validate_wgsl(include_str!("fog.wesl"));
    }

    // The real systems in their real order: the view and the cutscene start arrive in one frame,
    // `update_fog` (Render) runs before `presentation::consume` (Ui) turns the state on.
    mod system {
        use super::*;
        use crate::sim_link::SimMsg;
        use crate::ui::presentation::{CineractiveState, PresentationPlugin};
        use sta_sim::command::LocalRouter;
        use sta_sim::present::Presentation;
        use sta_sim::source::WorldSource;
        use sta_sim::{SimEvent, TickOutput};
        use std::sync::Mutex;

        type Next = Arc<Mutex<Option<TickOutput>>>;

        struct Scripted(Next, Arc<WorldView>);

        impl WorldSource for Scripted {
            fn step(&mut self, _: &[sta_sim::command::Command]) -> TickOutput {
                let out = self.0.lock().unwrap().take().expect("a tick is only taken when one is queued");
                self.1 = out.view.clone();
                out
            }

            fn view(&self) -> Arc<WorldView> {
                self.1.clone()
            }
        }

        #[derive(Resource)]
        struct Feed(Next);

        fn drive(mut link: ResMut<SimLink>, feed: Res<Feed>, mut out: MessageWriter<SimMsg>) {
            if feed.0.lock().unwrap().is_some()
                && let Some(events) = link.tick()
            {
                out.write_batch(events.into_iter().map(SimMsg));
            }
        }

        struct Rig {
            app: App,
            next: Next,
            tick: u64,
        }

        impl Rig {
            fn new() -> Rig {
                let next: Next = Arc::default();
                let first = Arc::new(view(vec![]));
                let mut app = App::new();
                app.add_plugins(MinimalPlugins).init_resource::<ButtonInput<KeyCode>>().add_plugins(PresentationPlugin);
                crate::app::configure_sets(&mut app);
                app.init_resource::<Assets<Image>>()
                    .init_resource::<Assets<Mesh>>()
                    .init_resource::<Assets<FogMaterial>>()
                    .init_resource::<FogSettings>()
                    .insert_resource(FogPatterns { fog: (Handle::default(), Vec2::ONE), shroud: (Handle::default(), Vec2::ONE), generation: 0 })
                    .insert_resource(FogBounds { min: Vec2::splat(-4096.0), max: Vec2::splat(4096.0) })
                    .insert_resource(LocalPlayer(Some(PlayerId(1))))
                    .insert_resource(Feed(next.clone()))
                    .insert_resource(SimLink::new(Box::new(Scripted(next.clone(), first)), Box::new(LocalRouter::new()), PlayerId(1)))
                    .add_systems(Update, (drive.in_set(GameSet::SimLink), update_fog.in_set(GameSet::Render)));
                Rig { app, next, tick: 0 }
            }

            /// One frame; a new view and events arrive in it when given.
            fn frame(&mut self, units: Option<Vec<UnitView>>, cine: Option<bool>) {
                if units.is_some() || cine.is_some() {
                    self.tick += 1;
                    let mut v = match units {
                        Some(u) => view(u),
                        None => (*self.app.world().resource::<SimLink>().real_curr().clone()).clone(),
                    };
                    v.tick = self.tick;
                    let events = cine.map(|on| SimEvent::Present(Presentation::Cineractive { on, skippable: false })).into_iter().collect();
                    *self.next.lock().unwrap() = Some(TickOutput { view: Arc::new(v), events });
                }
                self.app.update();
            }

            fn explored(&self, x: f32) -> bool {
                self.app.world().resource::<FogState>().grid.explored_at(x, 0.0)
            }

            fn seen(&self, x: f32) -> bool {
                self.app.world().resource::<FogState>().grid.visible_at(x, 0.0)
            }

            fn updates(&self) -> u32 {
                self.app.world().resource::<FogState>().updates
            }

            fn cutscene(&self) -> bool {
                self.app.world().resource::<CineractiveState>().on
            }
        }

        #[test]
        fn editor_shows_whole_map_without_changing_play_exploration() {
            let at = |x| Some(vec![unit(1, 1, x, 0)]);
            let mut rig = Rig::new();
            rig.frame(at(0), None); // The last play sight is still pending.
            rig.app.world_mut().insert_resource(crate::editor::Editing::default());
            rig.frame(at(3000), None);
            let shown = &rig.app.world().resource::<FogState>().grid;
            assert_eq!(shown.counts(), (shown.w * shown.h, shown.w * shown.h), "3D and radar share the clear grid");
            assert!(shown.mask(Layer::Fog).iter().all(|&v| v == 0));
            assert!(shown.mask(Layer::Shroud).iter().all(|&v| v == 0));
            rig.app.world_mut().remove_resource::<crate::editor::Editing>();
            rig.frame(at(-3000), None);
            assert!(rig.explored(0.0), "sight pending when play was paused survives editing");
            assert!(!rig.explored(3000.0) && !rig.seen(3000.0), "editor-only sight is forgotten");
            assert!(rig.seen(-3000.0), "normal sensor rules resume");
        }

        #[test]
        fn starting_in_editor_does_not_seed_play_exploration() {
            let mut rig = Rig::new();
            rig.app.world_mut().insert_resource(crate::editor::Editing::default());
            rig.frame(Some(vec![unit(1, 1, 3000, 0)]), None);
            assert_eq!(rig.app.world().resource::<FogState>().grid.counts().0,
                rig.app.world().resource::<FogState>().grid.w * rig.app.world().resource::<FogState>().grid.h);
            rig.app.world_mut().remove_resource::<crate::editor::Editing>();
            rig.frame(Some(vec![unit(1, 1, 0, 0)]), None);
            assert!(!rig.explored(3000.0) && !rig.seen(3000.0));
            assert!(rig.seen(0.0));
        }

        #[test]
        fn a_cutscene_started_by_the_same_frame_as_the_units_moving_uncovers_nothing() {
            let at = |x| Some(vec![unit(1, 1, x, 0)]);
            let mut rig = Rig::new();
            // Normal play at home.
            rig.frame(at(0), None);
            rig.frame(at(0), None);
            assert!(rig.explored(0.0) && !rig.explored(3000.0));
            // One frame: the units are placed far away and the cutscene starts.
            rig.frame(at(3000), Some(true));
            assert!(rig.cutscene(), "consume turned the state on after update_fog ran");
                // The cutscene runs on: units move again.
            rig.frame(at(-3000), None);
            assert!(rig.seen(-3000.0) && rig.explored(-3000.0), "what is in sight is drawn as before");
            assert!(!rig.explored(3000.0), "ground seen only in the frame the cutscene began is not explored");
            // The end, no unit moved and no new view: the state flip recomputes.
            let updates = rig.updates();
            rig.frame(None, Some(false));
            rig.frame(None, None);
            assert!(!rig.cutscene());
            assert!(rig.updates() > updates, "the end of the cutscene rebuilds the picture");
            assert!(rig.explored(-3000.0), "the end positions count");
            assert!(!rig.explored(3000.0) && !rig.seen(3000.0));
            // A second cutscene straight after the first behaves the same.
            rig.frame(at(1500), Some(true));
            rig.frame(at(-1500), None);
            assert!(!rig.explored(1500.0), "second cutscene: nothing uncovered");
            assert!(rig.explored(-1500.0) && rig.seen(-1500.0));
            rig.frame(None, Some(false));
            rig.frame(None, None);
            rig.frame(at(-1500), None);
            assert!(rig.explored(-1500.0) && !rig.explored(1500.0) && !rig.explored(3000.0));
            assert!(rig.explored(0.0), "what was explored before stays explored");
        }
    }
}
