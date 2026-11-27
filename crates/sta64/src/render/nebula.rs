//! Nebulae as volumes of gas (Armada II Refit's Nebulae port, see `nebula.wesl`).
//!
//! A nebula class with a recipe (`nebulae/<odf>.ini` through the VFS, else the ones
//! bundled from Refit) is drawn as one field per map: an envelope over the map plane
//! built from where the class's nebulae sit, and a box around it whose back faces run
//! the march. Its stock billboards are not drawn. The noise volume is shared by all
//! classes and made once, on a background thread.

use bevy::asset::{embedded_asset, AssetPath, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, Extent3d, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use crate::render::nebula_lowres::NebulaLowRes;
use bevy::camera::visibility::RenderLayers;
use sta_surface::nebula::{envelope, noise_volume, volume_mips, Lightning, NebulaRecipe};
use std::sync::{Arc, Mutex};

/// The noise volume's size (Refit's default).
const NOISE: usize = 128;
/// Refit's defaults when a recipe leaves them unset.
const KEY_SHARE: f32 = 0.8;
const SCATTER: f32 = 0.2;

const BUNDLED: &[(&str, &str)] = &[
    ("fluidicnebula", include_str!("../../../sta_surface/nebulae/fluidicnebula.ini")),
    ("generatednebula", include_str!("../../../sta_surface/nebulae/generatednebula.ini")),
    ("latinum", include_str!("../../../sta_surface/nebulae/latinum.ini")),
    ("mnebula10", include_str!("../../../sta_surface/nebulae/mnebula10.ini")),
    ("mnebula12", include_str!("../../../sta_surface/nebulae/mnebula12.ini")),
    ("mnebula6", include_str!("../../../sta_surface/nebulae/mnebula6.ini")),
    ("mnebula7", include_str!("../../../sta_surface/nebulae/mnebula7.ini")),
    ("mnebula8", include_str!("../../../sta_surface/nebulae/mnebula8.ini")),
    ("mnebula9", include_str!("../../../sta_surface/nebulae/mnebula9.ini")),
];

/// The recipe for a nebula class, if it has one.
pub fn recipe(vfs: &sta_vfs::Vfs, odf: &str) -> Option<NebulaRecipe> {
    let key = odf.to_ascii_lowercase();
    if let Some(Ok(b)) = vfs.read(&format!("nebulae/{key}.ini")) {
        return Some(NebulaRecipe::parse(&String::from_utf8_lossy(&b)));
    }
    BUNDLED.iter().find(|(n, _)| *n == key).map(|(_, t)| NebulaRecipe::parse(t))
}

/// The Key's shadow map as `shadows.rs` fitted it, into the march's parameters (none: no shadow).
pub fn apply_shadow(p: &mut NebulaParams, shadow: Option<&crate::render::shadows::KeyShadow>) {
    match shadow {
        Some(s) => {
            p.sh0 = s.rows[0];
            p.sh1 = s.rows[1];
            p.sh2 = s.rows[2];
            p.shp = s.params;
        }
        None => p.shp = Vec4::ZERO,
    }
}

#[derive(Clone, Copy, Default, ShaderType, Debug)]
pub struct NebulaParams {
    pub eye: Vec4,
    pub gas_a: Vec4,
    pub gas_b: Vec4,
    pub glow: Vec4,
    pub noise: Vec4,
    pub seed: Vec4,
    pub env: Vec4,
    pub vert: Vec4,
    pub shape: Vec4,
    pub knots: Vec4,
    pub light: Vec4,
    pub fine: Vec4,
    pub flow: Vec4,
    pub fil: Vec4,
    pub fringe: Vec4,
    pub core: Vec4,
    pub opac: Vec4,
    pub key: Vec4,
    pub scat: Vec4,
    pub box_lo: Vec4,
    pub box_hi: Vec4,
    pub cam_fwd: Vec4,
    pub pulse: Vec4,
    pub ring: Vec4,
    pub rpatch: Vec4,
    pub ring_a: Vec4,
    pub ring_b: Vec4,
    pub ring_c: Vec4,
    pub ring_d: Vec4,
    pub swirl: Vec4,
    pub flash0: Vec4,
    pub flash1: Vec4,
    pub flash2: Vec4,
    pub flash3: Vec4,
    pub flash_c: Vec4,
    /// The Key's shadow map (`shadows.rs`): world to u, v, depth along the Key, and
    /// x 1 with a map, y the depth bias, z the penumbra, w a texel.
    pub sh0: Vec4,
    pub sh1: Vec4,
    pub sh2: Vec4,
    pub shp: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct NebulaMaterial {
    #[uniform(0)]
    pub params: NebulaParams,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub noise: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub envelope: Option<Handle<Image>>,
    /// The Key's shadow map, read when `params.shp.x` says there is one.
    #[texture(5)]
    pub shadow: Option<Handle<Image>>,
}

impl Material for NebulaMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/nebula.wesl").into_owned())
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    /// The box's back faces, no depth test or write: the march finds the scene's
    /// depth itself, so the gas in front of a ship still shows.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = Some(Face::Front);
        // The module has a second entry point (the low-resolution march, `nebula_lowres.rs`).
        if let Some(f) = descriptor.fragment.as_mut() {
            f.entry_point = Some("fragment".into());
        }
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_compare = Some(CompareFunction::Always);
            ds.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// Where the background thread leaves the noise volume's levels.
type Slot = Arc<Mutex<Option<Vec<Vec<u8>>>>>;

/// The shared noise volume, made once on a background thread.
#[derive(Resource, Default)]
pub struct NoiseVolume {
    pending: Option<Slot>,
    pub image: Option<Handle<Image>>,
}

/// A class's field on this map.
#[derive(Component)]
pub struct NebulaField {
    pub(super) material: Handle<NebulaMaterial>,
    recipe: NebulaRecipe,
    reach: f32,
    /// The envelope's extent, for the edge-on dimming's path limit.
    span: f32,
    /// The domes' height, the lightning's vertical spread.
    dome: f32,
    /// Where the class's nebulae sit (Bevy space), for the lightning to strike among.
    centres: Vec<[f32; 3]>,
    /// The nebula radius the envelope was built for.
    radius: f32,
    /// Which of `centres` are drawn (#1146: a nebula never seen is not); all, until the states say otherwise.
    shown: Vec<f32>,
    /// Each centre's region of the map, found once (`gate_fields`); `None` until then.
    region: Option<Vec<Option<usize>>>,
    /// The centres drawn (`shown`), for the lightning to strike among.
    lit: Vec<[f32; 3]>,
    lightning: Lightning,
    /// The wall clock at the last update, for the lightning's step.
    last: f32,
}

/// A field whose box the low-resolution path stands in for (`nebula_lowres.rs`).
#[derive(Component)]
struct LowResGas;

pub struct NebulaPlugin;

impl Plugin for NebulaPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "nebula.wesl");
        app.add_plugins((MaterialPlugin::<NebulaMaterial>::default(), crate::render::nebula_lowres::NebulaLowResPlugin))
            .init_resource::<NoiseVolume>()
            .add_systems(Update, (poll_noise, gate_fields.run_if(resource_exists::<crate::render::fog_objects::FogRegions>)))
            // `fade_boards` must run every frame, with no run condition: it reads change ticks and
            // removal events since its last run, and a removal it misses (a puff losing its stored
            // fade) would leave a frame skipped that the walk of every board would not skip.
            .add_systems(PostUpdate, (fade_boards, update.after(crate::render::shadows::ShadowSet))
                .after(TransformSystems::Propagate)
                .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate));
    }
}

/// A stock nebula drawn as billboards: its puffs dim
/// with the eye's nearness and with the nebulae in front of it (RND-05).
#[derive(Component)]
pub struct NebulaBoards {
    /// The nebula's reach, in world units.
    pub radius: f32,
}

/// The near fade's distance, in world units: a puff is gone within a quarter of it
/// (or of its own half size, if larger) and whole past a quarter more than the whole.
const BOARD_NEAR: f32 = 60.0;
/// How much of a puff's light a nebula covering all of it takes away.
const OCCLUDE: f32 = 0.5;

/// A puff's near fade: 0 at the eye, 1 once the eye is clear of it. The same ramp as
/// the gas's `NearFade` (`nebula.wesl`), over the puff's half size where that is wider.
pub fn near_fade(dist: f32, half_size: f32) -> f32 {
    let nf = BOARD_NEAR.max(half_size);
    ((dist - nf * 0.25) / nf).clamp(0.0, 1.0)
}

/// Each nebula's occlusion intensity, 1 when nothing stands before it: the nebulae
/// nearer the eye whose discs (by angle) cover its centre's direction thin it, each by
/// `OCCLUDE` times how much of its disc they cover. The rule STA64 follows is that stock thins a
/// nebula where another nebula hides it (evidence grade: read from the executable, static); the
/// exact falloff is not known to us. `(centre, radius)` per nebula; an eye inside a nebula is
/// not occluded by it.
/// (The board fade keeps an [`Occlusion`] between frames; this is the one-off form the tests use.)
#[cfg(test)]
pub fn occlusion(eye: Vec3, nebulae: &[(Vec3, f32)]) -> Vec<f32> {
    let mut out = Vec::new();
    Occlusion::default().run(eye, nebulae, &mut out);
    out
}

/// Half the angular margin (radians) by which `occlusion`'s pair test keeps discs that may touch.
const PAIR_MARGIN: f64 = 5e-4;

/// `occlusion`'s index: discs whose half-angle (with the margin) is at most `GRID_MAX_HALF` sit
/// on grids over two components (x, z) of their directions, one grid per size class.
const GRID_MAX_HALF: f64 = 0.6;
/// The smallest size class's bound on the half-angle; each class doubles it, up to `GRID_MAX_HALF`.
const GRID_MIN_HALF: f64 = 1.0 / 512.0;
/// Size classes.
const GRID_LEVELS: usize = 10;
/// Most cells along an axis of a class's grid.
const GRID_CELLS: usize = 128;
/// A grid covers direction components from `-GRID_OFFSET` to `GRID_OFFSET`; a disc whose
/// direction has a component beyond `GRID_REACH` (or one that is not finite) is on no grid.
const GRID_OFFSET: f64 = 1.5;
const GRID_REACH: f32 = 1.25;
/// What a grid query keeps beyond the pair test's bound on the chord: far wider than the rounding
/// of that bound, of the cell index and of the directions' f32 differences.
const GRID_PAD: f64 = 1e-5;

/// The eyes one candidate list serves lie within this distance of the eye it was made at.
const LIST_REACH: f64 = 30.0;
/// Calls of `Occlusion::run` (the board fade calls it only when the eye or the visible nebulae
/// changed, so not every frame) since the last list was made before another is made for an eye
/// outside its ball; until then the grids are queried.
const LIST_MIN_AGE: u32 = 4;
/// Half of the margin (radians) that a list's half-angle carries beyond the largest angular radius
/// of its disc and the drift of its direction. This is a correctness margin, not a speed setting:
/// the pair test keeps pairs up to `PAIR_MARGIN` beyond the discs' angular radii (half-angle sum),
/// and a list must hold every pair the pair test keeps, so it has to exceed `PAIR_MARGIN` by the
/// rounding of the f32 directions, of the arcsines and of the chord (`LIST_ROUNDING`).
const LIST_SLACK: f64 = 1.5e-3;
/// An allowance, in radians of half-angle, for that rounding (about 1e-6 in all).
const LIST_ROUNDING: f64 = 1e-4;
const _: () = assert!(LIST_SLACK >= PAIR_MARGIN + LIST_ROUNDING);

/// A size class's bound on the half-angle.
fn class_bound(l: usize) -> f64 {
    (GRID_MIN_HALF * (1u32 << l) as f64).min(GRID_MAX_HALF)
}

/// One size class's grid: the discs in each cell (by the x and z of their directions), in disc
/// order.
#[derive(Default)]
struct Grid {
    /// Cells along an axis, and a cell's width.
    cells: usize,
    size: f64,
    /// `items[start[c]..start[c + 1]]` are cell `c`'s discs.
    start: Vec<u32>,
    items: Vec<u32>,
    used: bool,
}

/// The candidate lists of [`Occlusion`] and what they were made for.
#[derive(Default)]
struct Lists {
    ready: bool,
    eye: Vec3,
    /// The nebulae (centre, radius) the lists were made for, by bits.
    key: Vec<[u32; 4]>,
    age: u32,
    /// Per disc: has a list (else it runs the pair loop over every disc).
    listed: Vec<bool>,
    /// `items[start[i]..start[i + 1]]` are disc i's partners, ascending.
    start: Vec<u32>,
    items: Vec<u32>,
    /// Calls the lists served.
    #[cfg(test)]
    served: u32,
}

/// `occlusion` with its working memory, which the board fade keeps between frames.
///
/// The result is the same, bit for bit, as the pair loop over every pair (`occlusion_reference` in
/// the tests). Each nebula's intensity is a product, in ascending index order, over the nebulae
/// before it; the pair test of #1235 leaves out pairs whose factor is exactly 1 (discs further
/// apart than their angular radii plus 2e-3 rad). Here, for each disc, an index proposes a
/// superset of the partners that test keeps; the partners the pair loop would pass over (itself,
/// those not nearer the eye, those holding the eye, those the pair test leaves out) are dropped by
/// the pair loop's own conditions, and the product runs over the rest in ascending index order with
/// the pair loop's own code. A partner the index does not propose is one the pair test leaves out
/// (see [`Occlusion::partners`]). So the factors multiplied, and their order, are the pair loop's:
/// what is left out is a factor of exactly 1 (or a pair the loop skips), which leaves a product
/// unchanged.
///
/// Discs whose half-angle is negative, above `GRID_MAX_HALF` or not a number, or whose direction is
/// not a finite vector near the unit sphere (an eye at a centre, inside a nebula or next to it, a
/// radius that is not a number, a centre at infinity) are on no grid: they are proposed to every
/// disc, and a disc on no grid runs the pair loop over every disc.
///
/// Candidate lists. While the eye moves within `LIST_REACH` of where it was, and the nebulae are
/// the same (by bits), the grids are not queried again. At the eye `e0` a list is made for each
/// disc that is regular there (a finite radius of at least 0, its centre between `4 LIST_REACH`
/// and 1e6 away): every partner whose directions, seen from `e0`, are closer than the discs'
/// largest angular radii for any eye in the ball, plus how far a direction turns in the ball (`asin
/// (LIST_REACH / distance)`), plus `4 LIST_SLACK` (6e-3 rad), which is wider than the pair test's 2e-3 rad by far more than any
/// rounding (`LIST_SLACK` is a correctness margin; a compile-time assertion keeps it above
/// `PAIR_MARGIN` plus `LIST_ROUNDING`). For an eye `e` in the ball, two directions from `e` are at least that angle
/// less the two drifts apart, so a pair not in the list is one the pair test leaves out at `e`
/// (the same argument as for the grids). The pair loop's own conditions are applied to the list at
/// `e`, in ascending index order, so the result is the pair loop's, bit for bit. A disc not regular
/// at `e0` is in every list and runs the pair loop over every disc. Outside the ball, a list is made
/// anew if `LIST_MIN_AGE` calls of `run` have passed since the last, else the grids are queried as before.
#[derive(Default)]
pub struct Occlusion {
    lists: Lists,
    spare: Vec<(f64, f64, f64)>,
    /// Per disc: direction, distance, angular radius (as the pair loop has them).
    discs: Vec<(Vec3, f32, f32)>,
    /// Per disc: half the angular radius plus the margin, its sine and its cosine (in f64).
    half: Vec<(f64, f64, f64)>,
    /// Per disc: its size class, `GRID_LEVELS` for none.
    level: Vec<u8>,
    /// The discs on no grid, in order.
    wide: Vec<u32>,
    grids: Vec<Grid>,
    fill: Vec<u32>,
    /// One disc's partners.
    found: Vec<u32>,
    /// Deliberate faults, for the tests to show they would see them: 1 queries the grids without
    /// the class bound (too small a range), 2 multiplies in descending order, 3, 4 and 5 make the candidate
    /// lists without the drift of directions and the slack, without the slack, without the drift.
    #[cfg(test)]
    fault: u8,
}

impl Occlusion {
    /// Each nebula's intensity into `out` (see `occlusion`).
    pub fn run(&mut self, eye: Vec3, nebulae: &[(Vec3, f32)], out: &mut Vec<f32>) {
        let n = nebulae.len();
        self.load(eye, nebulae);
        let key_same = self.lists.ready && self.lists.key.len() == n && self.lists.key.iter().zip(nebulae).all(|(k, &(c, r))| *k == [c.x.to_bits(), c.y.to_bits(), c.z.to_bits(), r.to_bits()]);
        let reach = {
            let (e0, e) = (self.lists.eye, eye);
            let (x, y, z) = (e.x as f64 - e0.x as f64, e.y as f64 - e0.y as f64, e.z as f64 - e0.z as f64);
            (x * x + y * y + z * z).sqrt()
        };
        let mut serve = key_same && reach <= LIST_REACH;
        self.lists.age = self.lists.age.saturating_add(1);
        if !serve && (!key_same || self.lists.age >= LIST_MIN_AGE) && eye.to_array().iter().all(|c| c.is_finite()) {
            self.make_lists(eye, nebulae);
            serve = true;
        }
        out.clear();
        if serve {
            #[cfg(test)]
            {
                self.lists.served += 1;
            }
            for i in 0..n {
                if !self.lists.listed[i] {
                    out.push(self.product(i, 0..n, nebulae));
                    continue;
                }
                let js = &self.lists.items[self.lists.start[i] as usize..self.lists.start[i + 1] as usize];
                #[cfg(test)]
                if self.fault == 2 {
                    out.push(self.product(i, js.iter().rev().map(|&j| j as usize), nebulae));
                    continue;
                }
                out.push(self.product(i, js.iter().map(|&j| j as usize), nebulae));
            }
            return;
        }
        self.grids();
        let mut found = std::mem::take(&mut self.found);
        for i in 0..n {
            if self.level[i] as usize == GRID_LEVELS {
                out.push(self.product(i, 0..n, nebulae));
                continue;
            }
            self.partners::<false>(i, nebulae, &mut found);
            found.sort_unstable();
            #[cfg(test)]
            if self.fault == 2 {
                found.reverse();
            }
            out.push(self.product(i, found.iter().map(|&j| j as usize), nebulae));
        }
        self.found = found;
    }

    /// The discs and half-angles of the nebulae seen from `eye`.
    fn load(&mut self, eye: Vec3, nebulae: &[(Vec3, f32)]) {
        // Each nebula's disc once (direction, distance, angular radius), not once per pair: the
        // disc costs a square root and an arcsine.
        self.discs.clear();
        self.discs.extend(nebulae.iter().map(|&(c, r)| {
            let d = c - eye;
            let dist = d.length().max(1e-3);
            (d / dist, dist, (r / dist).min(1.0).asin())
        }));
        // For the pair test (in f64): half the disc's angular radius plus a margin, with its sine
        // and cosine, so that two discs' bound on their directions' chord costs no sine per pair.
        // A disc whose distance was clamped has no unit direction and is never skipped.
        self.half.clear();
        self.half.extend(nebulae.iter().zip(&self.discs).map(|(&(c, _), &(_, _, a))| {
            let h = if (c - eye).length() >= 1e-3 { a as f64 * 0.5 + PAIR_MARGIN } else { std::f64::consts::PI };
            (h, h.sin(), h.cos())
        }));
    }

    /// The half-angle a list gives a regular disc of radius `r` whose centre is `d0` from the eye the
    /// list is made at: half the largest angular radius for any eye in the ball plus half the drift
    /// of its direction, plus `LIST_SLACK`.
    fn list_half(&self, r: f32, d0: f64) -> f64 {
        let a_max = (r as f64 / (d0 - LIST_REACH)).min(1.0).asin();
        let drift = (LIST_REACH / d0).asin();
        let slack = LIST_SLACK;
        // Deliberate faults: 3 leaves out the drift and the slack, 4 the slack, 5 the drift.
        #[cfg(test)]
        let (drift, slack) = match self.fault {
            3 => (0.0, 0.0),
            4 => (drift, 0.0),
            5 => (0.0, slack),
            _ => (drift, slack),
        };
        (a_max + drift) * 0.5 + slack
    }

    /// The candidate lists for the eyes within `LIST_REACH` of `eye` (see [`Occlusion`]), made with
    /// the discs of `eye` in `self.discs`.
    fn make_lists(&mut self, eye: Vec3, nebulae: &[(Vec3, f32)]) {
        let n = nebulae.len();
        let rho = LIST_REACH;
        let mut inflated = std::mem::take(&mut self.spare);
        inflated.clear();
        let mut listed = std::mem::take(&mut self.lists.listed);
        listed.clear();
        for &(c, r) in nebulae {
            let (x, y, z) = (c.x as f64 - eye.x as f64, c.y as f64 - eye.y as f64, c.z as f64 - eye.z as f64);
            let d0 = (x * x + y * y + z * z).sqrt();
            let safe = r.is_finite() && r >= 0.0 && d0 >= 4.0 * rho && d0 <= 1.0e6;
            listed.push(safe);
            let h = if safe { self.list_half(r, d0) } else { std::f64::consts::PI };
            inflated.push((h, h.sin(), h.cos()));
        }
        self.lists.listed = listed;
        std::mem::swap(&mut self.half, &mut inflated);
        self.grids();
        let mut found = std::mem::take(&mut self.found);
        let mut start = std::mem::take(&mut self.lists.start);
        let mut items = std::mem::take(&mut self.lists.items);
        start.clear();
        items.clear();
        start.push(0);
        for i in 0..n {
            if self.lists.listed[i] {
                self.partners::<true>(i, nebulae, &mut found);
                found.sort_unstable();
                items.extend_from_slice(&found);
            }
            start.push(items.len() as u32);
        }
        self.found = found;
        std::mem::swap(&mut self.half, &mut inflated);
        self.spare = inflated;
        let lists = &mut self.lists;
        lists.start = start;
        lists.items = items;
        lists.key.clear();
        lists.key.extend(nebulae.iter().map(|&(c, r)| [c.x.to_bits(), c.y.to_bits(), c.z.to_bits(), r.to_bits()]));
        lists.eye = eye;
        lists.ready = true;
        lists.age = 0;
    }

    /// Whether the pair loop passes over partner j of disc i without a factor: j is i, is not
    /// nearer the eye, holds the eye, or the pair test leaves it out.
    fn skips<const GEOM: bool>(&self, i: usize, j: usize, nebulae: &[(Vec3, f32)]) -> bool {
        let ((di, dist_i, _), (dj, dist_j, _)) = (self.discs[i], self.discs[j]);
        if j == i || (!GEOM && (dist_j >= dist_i || dist_j < nebulae[j].1)) {
            return true;
        }
        // Discs whose directions are further apart than their angular radii plus 2e-3 rad cover
        // nothing of each other: their factor would be exactly 1 (the cover clamps to 0), so they
        // are skipped without the arccosine. Each half-angle h is half the disc's angular radius plus
        // PAIR_MARGIN (5e-4 rad), so the half-angle sum carries 2 PAIR_MARGIN = 1e-3 rad, which is
        // 2e-3 rad on the angle between the directions; such discs' chord is longer than
        // 2 sin(hi + hj). The 2e-3 rad are far wider than the rounding of the directions and of the
        // arccosine.
        let ((hi, si, ci), (hj, sj, cj)) = (self.half[i], self.half[j]);
        if hi + hj < std::f64::consts::FRAC_PI_2 {
            let bound = 2.0 * (si * cj + ci * sj);
            let (dx, dy, dz) = ((di.x - dj.x) as f64, (di.y - dj.y) as f64, (di.z - dj.z) as f64);
            if dx * dx + dy * dy + dz * dz > bound * bound {
                return true;
            }
        }
        false
    }

    /// Disc i's intensity over the partners `js`, in ascending order: the pair loop's body.
    fn product(&self, i: usize, js: impl Iterator<Item = usize>, nebulae: &[(Vec3, f32)]) -> f32 {
        let (di, _, ai) = self.discs[i];
        let mut k = 1.0f32;
        for j in js {
            if self.skips::<false>(i, j, nebulae) {
                continue;
            }
            let (dj, _, aj) = self.discs[j];
            let theta = di.dot(dj).clamp(-1.0, 1.0).acos();
            let cover = ((aj + ai - theta) / (2.0 * ai).max(1e-4)).clamp(0.0, 1.0);
            k *= 1.0 - OCCLUDE * cover;
        }
        k
    }

    /// The size classes and their grids.
    fn grids(&mut self) {
        self.level.clear();
        self.wide.clear();
        for (i, (&(d, _, _), &(h, _, _))) in self.discs.iter().zip(&self.half).enumerate() {
            let on = (0.0..=GRID_MAX_HALF).contains(&h) && d.to_array().iter().all(|c| c.abs() <= GRID_REACH);
            let level = if on { (0..GRID_LEVELS).find(|&l| h <= class_bound(l)).unwrap_or(GRID_LEVELS) } else { GRID_LEVELS };
            if level == GRID_LEVELS {
                self.wide.push(i as u32);
            }
            self.level.push(level as u8);
        }
        self.grids.resize_with(GRID_LEVELS, Grid::default);
        let cell = |x: f32, size: f64| ((x as f64 + GRID_OFFSET) / size).floor() as usize;
        for (l, g) in self.grids.iter_mut().enumerate() {
            g.used = self.level.iter().any(|&v| v as usize == l);
            if !g.used {
                continue;
            }
            // Cells about as wide as the query of a disc of the class.
            let width = 2.0 * (2.0 * class_bound(l)).sin() + GRID_PAD;
            g.cells = ((2.0 * GRID_OFFSET / width).floor() as usize).clamp(1, GRID_CELLS);
            g.size = 2.0 * GRID_OFFSET / g.cells as f64;
            let (cells, size) = (g.cells, g.size);
            let index = |d: Vec3| cell(d.x, size) * cells + cell(d.z, size);
            g.start.clear();
            g.start.resize(cells * cells + 1, 0);
            for (i, &(d, _, _)) in self.discs.iter().enumerate() {
                if self.level[i] as usize == l {
                    g.start[index(d) + 1] += 1;
                }
            }
            for c in 0..cells * cells {
                g.start[c + 1] += g.start[c];
            }
            g.items.clear();
            g.items.resize(g.start[cells * cells] as usize, 0);
            self.fill.clear();
            self.fill.extend_from_slice(&g.start);
            for (i, &(d, _, _)) in self.discs.iter().enumerate() {
                if self.level[i] as usize == l {
                    let c = index(d);
                    g.items[self.fill[c] as usize] = i as u32;
                    self.fill[c] += 1;
                }
            }
        }
    }

    /// Disc i's partners, unordered: the discs on no grid and the grids' discs near its direction,
    /// less those the pair loop passes over ([`Occlusion::skips`]).
    ///
    /// Why a disc left out here is one the pair test leaves out: disc i (half-angle `hi`) asks each
    /// class `l` (bound `H`, every disc j of it with `hj <= H`) for the cells within
    /// `B = 2 sin(hi + H) + GRID_PAD` of its direction along x and along z. As `hi + hj <=
    /// 2 GRID_MAX_HALF < pi/2`, the pair test runs for the pair, with the bound
    /// `2 (sin hi cos hj + cos hi sin hj)`, which is `2 sin(hi + hj) <= 2 sin(hi + H)` up to a
    /// rounding far below the pad. A disc in a cell outside the range along x (or z) has an x (or z)
    /// further than `B` from disc i's: the cell index rounds monotonically in the component, and `B`
    /// is subtracted from or added to disc i's component before its cell is taken. Its f32
    /// difference along that axis is then longer than the bound by most of the pad, and so is the
    /// root of the test's sum of squares: the test leaves the pair out.
    fn partners<const GEOM: bool>(&self, i: usize, nebulae: &[(Vec3, f32)], found: &mut Vec<u32>) {
        found.clear();
        found.extend(self.wide.iter().copied().filter(|&j| !self.skips::<GEOM>(i, j as usize, nebulae)));
        let (d, hi) = (self.discs[i].0, self.half[i].0);
        for (l, g) in self.grids.iter().enumerate() {
            if !g.used {
                continue;
            }
            let b = 2.0 * (hi + class_bound(l)).sin() + GRID_PAD;
            #[cfg(test)]
            let b = if self.fault == 1 { 2.0 * hi.sin() } else { b };
            let range = |x: f32| {
                let lo = (((x as f64 - b) + GRID_OFFSET) / g.size).floor().max(0.0) as usize;
                let hi = ((((x as f64 + b) + GRID_OFFSET) / g.size).floor().max(0.0) as usize).min(g.cells - 1);
                lo..=hi
            };
            let zs = range(d.z);
            for cx in range(d.x) {
                let row = cx * g.cells;
                let items = &g.items[g.start[row + zs.start()] as usize..g.start[row + zs.end() + 1] as usize];
                found.extend(items.iter().copied().filter(|&j| !self.skips::<GEOM>(i, j as usize, nebulae)));
            }
        }
    }
}

/// One puff's fade step: the value its eye distance, half size and nebula's occlusion give,
/// written when it is more than a thousandth off the stored one (or inserted when there is none).
/// Returns whether a fade was inserted.
fn fade_puff(
    commands: &mut Commands,
    e: Entity,
    g: &GlobalTransform,
    t: &Transform,
    fade: Option<Mut<crate::render::sprites::SpriteFade>>,
    eye: Vec3,
    occ: f32,
) -> bool {
    // A billboard quad's scale is twice its half size.
    let v = near_fade(g.translation().distance(eye), t.scale.x * 0.5) * occ;
    match fade {
        Some(mut f) => {
            if (f.0 - v).abs() > 1e-3 {
                f.0 = v;
            }
            false
        }
        None => {
            commands.entity(e).insert(crate::render::sprites::SpriteFade(v));
            true
        }
    }
}

/// What [`fade_boards`] keeps between frames: each billboard nebula's puffs, and the inputs of
/// the last pass that wrote the fades.
///
/// A pass is a pure function of the eye, the visible nebulae (centre and radius, in order), and
/// each puff's world position, local scale and stored fade; the occlusion, a function of the first
/// two, is kept with them and computed again only when they change. After a pass every stored fade is
/// within a thousandth of the value its inputs give (or was just set to it), so a second pass over
/// unchanged inputs writes nothing: it is skipped. A change to any of these inputs runs the whole
/// pass, exactly as before. The one exception is a puff under two billboard nebulae at once (one
/// nested in the other): its two writes per pass are not at rest, so with such a puff every frame
/// runs the pass.
#[derive(Default)]
struct BoardCache {
    /// Each nebula's puffs (descendants with a sprite node), in the order a hierarchy walk
    /// from the nebula meets them.
    puffs: bevy::ecs::entity::EntityHashMap<Vec<Entity>>,
    /// The nebulae and all their descendants, puff or not, to tell which changes concern them.
    members: bevy::ecs::entity::EntityHashSet,
    /// A puff is under two nebulae.
    shared: bool,
    /// The lists must be walked again (first frame, a change to a nebula's hierarchy).
    stale: bool,
    /// The last pass's eye and visible nebulae (centre and radius), by bits, and the occlusion
    /// they gave.
    eye: Option<[u32; 3]>,
    visible: Vec<(Entity, [u32; 3], u32, u32)>,
    occ: Vec<f32>,
    /// The occlusion's working memory.
    index: Occlusion,
    /// The last pass queued a fade insert: the next one runs whatever changed.
    inserted: bool,
}

impl BoardCache {
    fn rebuild(&mut self, roots: impl Iterator<Item = Entity>, children: &Query<&Children>, is_puff: impl Fn(Entity) -> bool) {
        self.puffs.clear();
        self.members.clear();
        self.shared = false;
        let mut seen = bevy::ecs::entity::EntityHashSet::default();
        for root in roots {
            self.members.insert(root);
            let mut list = Vec::new();
            for e in children.iter_descendants(root) {
                self.members.insert(e);
                if is_puff(e) {
                    if !seen.insert(e) {
                        self.shared = true;
                    }
                    list.push(e);
                }
            }
            self.puffs.insert(root, list);
        }
        self.stale = false;
    }
}

/// An entity that became a sprite node or got a transform.
type Gained = Or<(Added<crate::render::sprites::SpriteFx>, Added<GlobalTransform>, Added<Transform>)>;

/// Changes since the last run that may move the puff lists. Read every run, as change ticks and
/// removal events do not wait.
#[derive(bevy::ecs::system::SystemParam)]
struct BoardChanges<'w, 's> {
    /// A hierarchy change under a nebula.
    children: Query<'w, 's, Entity, Changed<Children>>,
    gained: Query<'w, 's, Entity, Gained>,
    lost_children: RemovedComponents<'w, 's, Children>,
    lost_fx: RemovedComponents<'w, 's, crate::render::sprites::SpriteFx>,
    lost_global: RemovedComponents<'w, 's, GlobalTransform>,
    lost_local: RemovedComponents<'w, 's, Transform>,
    lost_boards: RemovedComponents<'w, 's, NebulaBoards>,
    lost_fade: RemovedComponents<'w, 's, crate::render::sprites::SpriteFade>,
}

impl BoardChanges<'_, '_> {
    /// Whether a nebula's hierarchy changed (`.0`) and whether a puff lost its stored fade (`.1`).
    fn read(&mut self, members: &bevy::ecs::entity::EntityHashSet) -> (bool, bool) {
        let m = |e: Entity| members.contains(&e);
        // Every removal reader is read in full, so none carries old events into the next run.
        let mut structure = self.lost_children.read().fold(false, |a, e| a | m(e));
        structure |= self.lost_fx.read().fold(false, |a, e| a | m(e));
        structure |= self.lost_global.read().fold(false, |a, e| a | m(e));
        structure |= self.lost_local.read().fold(false, |a, e| a | m(e));
        structure |= self.lost_boards.read().count() > 0;
        structure |= self.children.iter().any(m) || self.gained.iter().any(m);
        let fade = self.lost_fade.read().fold(false, |a, e| a | m(e));
        (structure, fade)
    }
}

type Puffs<'w, 's> =
    Query<'w, 's, (&'static GlobalTransform, &'static Transform, Option<&'static mut crate::render::sprites::SpriteFade>), With<crate::render::sprites::SpriteFx>>;
/// A sprite node whose position, scale or stored fade changed since the last run (by anyone but
/// the fade itself, whose own writes a system does not see).
type MovedPuffs<'w, 's> = Query<
    'w,
    's,
    Entity,
    (
        With<crate::render::sprites::SpriteFx>,
        Or<(Changed<GlobalTransform>, Changed<Transform>, Changed<crate::render::sprites::SpriteFade>)>,
    ),
>;

/// Dim each billboard nebula's puffs. The values and writes are exactly those of walking every
/// visible nebula's hierarchy each frame; the walk and the occlusion are kept in [`BoardCache`] and
/// a frame whose inputs did not change, which would write nothing, is skipped (#1235). Nebulae out
/// of the view are not skipped: a stored fade only follows its value by steps of more than a
/// thousandth, so what a skipped puff would hold on coming back depends on every frame it missed.
fn fade_boards(
    mut commands: Commands,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    nebulae: Query<(Entity, &GlobalTransform, &NebulaBoards, &InheritedVisibility, Option<&ChildOf>)>,
    out_of_sight: Query<(), With<crate::render::fog_objects::OutOfSight>>,
    children: Query<&Children>,
    mut puffs: ParamSet<(MovedPuffs, Puffs)>,
    mut changes: BoardChanges,
    mut cache: Local<BoardCache>,
) {
    let cache = &mut *cache;
    let (structure, lost_fade) = changes.read(&cache.members);
    let moved = lost_fade || puffs.p0().iter().any(|e| cache.members.contains(&e));
    // A nebula the lists do not know yet (spawned, or given its marker).
    if structure || nebulae.iter().any(|(e, ..)| !cache.puffs.contains_key(&e)) {
        cache.stale = true;
    }
    let Ok(cam) = camera.single() else {
        // Nothing is written; the next pass runs whatever changes meanwhile.
        cache.eye = None;
        return;
    };
    let mut puffs = puffs.p1();
    if cache.stale {
        cache.rebuild(nebulae.iter().map(|(e, ..)| e), &children, |e| puffs.contains(e));
        cache.eye = None;
    }
    let eye = cam.translation();
    let list: Vec<_> = nebulae.iter().filter(|(_, _, _, v, _)| v.get()).map(|(e, g, b, _, _)| (e, g.translation(), b.radius)).collect();
    // A nebula out of sight is a copy at a reduced intensity (#1146; `fog_objects::ghost::NEBULA`, interim): its
    // puffs' light is scaled by it, on top of the occlusion.
    let dims: Vec<f32> = nebulae
        .iter()
        .filter(|(_, _, _, v, _)| v.get())
        .map(|(.., parent)| if parent.is_some_and(|p| out_of_sight.contains(p.parent())) { crate::render::fog_objects::ghost::NEBULA } else { 1.0 })
        .collect();
    // The occlusion is a pure function of the eye and the visible nebulae (in order): kept while
    // they do not change.
    let visible: Vec<(Entity, [u32; 3], u32, u32)> = list.iter().zip(&dims).map(|(&(e, c, r), d)| (e, c.to_array().map(f32::to_bits), r.to_bits(), d.to_bits())).collect();
    let eye_bits = eye.to_array().map(f32::to_bits);
    let same = cache.eye == Some(eye_bits) && cache.visible == visible;
    if same && !moved && !cache.shared && !cache.inserted {
        return;
    }
    if !same {
        let discs: Vec<(Vec3, f32)> = list.iter().map(|&(_, c, r)| (c, r)).collect();
        cache.index.run(eye, &discs, &mut cache.occ);
    }
    let occ = std::mem::take(&mut cache.occ);
    let mut inserted = false;
    for ((&(root, _, _), &occ), &dim) in list.iter().zip(&occ).zip(&dims) {
        let Some(list) = cache.puffs.get(&root) else { continue };
        for &e in list {
            let Ok((g, t, fade)) = puffs.get_mut(e) else { continue };
            inserted |= fade_puff(&mut commands, e, g, t, fade, eye, occ * dim);
        }
    }
    cache.occ = occ;
    cache.eye = Some(eye_bits);
    cache.visible = visible;
    cache.inserted = inserted;
}

fn poll_noise(mut nv: ResMut<NoiseVolume>, mut images: ResMut<Assets<Image>>, mut fields: Query<&NebulaField>, mut mats: ResMut<Assets<NebulaMaterial>>) {
    if nv.image.is_none() && nv.pending.is_none() {
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        std::thread::spawn(move || {
            let levels = volume_mips(NOISE, noise_volume(NOISE));
            if let Ok(mut o) = out.lock() {
                *o = Some(levels);
            }
        });
        nv.pending = Some(slot);
    }
    if let Some(levels) = nv.pending.as_ref().and_then(|s| s.lock().ok().and_then(|mut o| o.take())) {
        let s = NOISE as u32;
        let mut img = Image::new(
            Extent3d {
                width: s,
                height: s,
                depth_or_array_layers: s,
            },
            TextureDimension::D3,
            levels[0].clone(),
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.texture_descriptor.mip_level_count = levels.len() as u32;
        img.data = Some(levels.concat());
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            address_mode_w: ImageAddressMode::Repeat,
            ..ImageSamplerDescriptor::linear()
        });
        nv.image = Some(images.add(img));
        nv.pending = None;
    }
    // Hand the volume to fields made before it was ready.
    if let Some(h) = &nv.image {
        for f in &mut fields {
            if let Some(m) = mats.get(&f.material)
                && m.noise.is_none()
                && let Some(mut m) = mats.get_mut(&f.material)
            {
                m.noise = Some(h.clone());
            }
        }
    }
}

/// A nebula of a class's field that the local player's side has never had in sight is not drawn
/// (#1146; rule and grade: `render::fog_objects`). The field is one box for the whole class, so the
/// nebula is cleared from the envelope the gas is marched through, and the lightning strikes among
/// the nebulae drawn only. A nebula in sight, or remembered (out-of-sight looks are a later step),
/// is as before. A centre that is no region of the map is always drawn, as is every nebula when there is
/// no fog grid (the states then read as in sight). Runs only when the states changed or a field is new,
/// and finds each centre's region once.
fn gate_fields(
    regions: Res<crate::render::fog_objects::FogRegions>,
    sights: Res<crate::render::fog_objects::RegionSights>,
    mut fields: Query<&mut NebulaField>,
    materials: Res<Assets<NebulaMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    use crate::render::fog_objects::Sight;
    // A field that has not found its regions yet is a new one.
    if !sights.is_changed() && !regions.is_changed() && !fields.iter().any(|f| f.region.is_none()) {
        return;
    }
    for mut f in &mut fields {
        if f.region.is_none() {
            let found = f.centres.iter().map(|c| regions.find(sta_content::RegionKind::Nebula, c[0], -c[2], REGION_MATCH).map(|r| r.id)).collect();
            f.region = Some(found);
        }
        // 1 in sight, the out-of-sight factor for a nebula seen before (a still, thinner copy; interim, see
        // `fog_objects::ghost::NEBULA`), 0 never seen.
        let shown: Vec<f32> = f
            .region
            .iter()
            .flatten()
            .map(|r| match r.map_or(Sight::Visible, |id| sights.get(id)) {
                Sight::Never => 0.0,
                Sight::Remembered => crate::render::fog_objects::ghost::NEBULA,
                Sight::Visible => 1.0,
            })
            .collect();
        if shown == f.shown {
            continue;
        }
        let Some(handle) = materials.get(&f.material).and_then(|m| m.envelope.clone()) else { continue };
        let env = sta_surface::nebula::envelope_weighted(&f.recipe, f.radius, &f.centres, &shown);
        let Some(mut img) = images.get_mut(&handle) else { continue };
        img.data = Some(env.pixels);
        // The lightning strikes among the nebulae in sight only: a copy out of sight does not animate.
        f.lit = f.centres.iter().zip(&shown).filter(|(_, s)| **s >= 1.0).map(|(c, _)| *c).collect();
        f.shown = shown;
    }
}

/// How near (world units) a field's centre must be to a region's to be that region: both come from
/// the one map object.
const REGION_MATCH: f32 = 2.0;

/// Spawn one class's field: its nebulae at `pos` (Bevy space), each `radius`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_field(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<NebulaMaterial>,
    images: &mut Assets<Image>,
    noise: &NoiseVolume,
    recipe: NebulaRecipe,
    radius: f32,
    pos: &[[f32; 3]],
    parent: Entity,
) -> Entity {
    let radius = if recipe.radius > 10.0 && recipe.radius < 5000.0 { recipe.radius } else { radius };
    let reach = radius * recipe.extent;
    let env = envelope(&recipe, radius, pos);
    let mut env_img = Image::new(
        Extent3d {
            width: env.w,
            height: env.h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        env.pixels.clone(),
        TextureFormat::Rgba8Unorm,
        // Kept in the main world too: the places not yet seen are cleared from it later (`gate_fields`).
        RenderAssetUsages::default(),
    );
    env_img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    });
    let dome = reach * recipe.height;
    let half = dome * (1.0 + recipe.lumps) + env.ydev;
    let [x0, z0, x1, z1] = env.rect;
    let lo = Vec3::new(x0, env.ey - half, z0);
    let hi = Vec3::new(x1, env.ey + half, z1);
    let r = &recipe;
    // A class's lightning is its own sequence, the same on every run of a seeded clock.
    let seed = pos.iter().flatten().fold(0x811c_9dc5u32, |h, x| (h ^ x.to_bits()).wrapping_mul(0x0100_0193));
    let params = NebulaParams {
        eye: Vec4::new(0.0, 0.0, 0.0, r.near_fade),
        gas_a: Vec3::from(r.gas_a).extend(r.brightness),
        gas_b: Vec3::from(r.gas_b).extend(r.hue_scale),
        glow: Vec3::from(r.glow).extend(r.gamma),
        noise: Vec4::new(r.inv_tile, r.warp, r.coverage, r.softness),
        seed: Vec3::from(r.seed).extend(r.detail),
        env: Vec4::new(x0, z0, 1.0 / (x1 - x0), 1.0 / (z1 - z0)),
        vert: Vec4::new(env.ey, dome, 0.0, 1.0 / reach),
        shape: Vec4::new(r.ridge, r.hue_mix, r.warp_scale, r.lumps),
        knots: Vec4::from(r.knots),
        light: Vec4::from(r.light),
        fine: Vec4::new(r.fine_detail, r.fuzz, r.flow, r.flow_scale),
        flow: Vec4::new(0.0, 0.0, 0.0, r.edge_warp * reach),
        fil: Vec4::new(r.filaments[0], r.filaments[1], r.filaments[2], dome * 0.8),
        fringe: Vec4::new(r.fringe[0], r.fringe[1], 0.0, 0.0),
        core: Vec4::new(r.core[0], r.core[1], 0.0, 0.0),
        opac: Vec4::new(r.opacity / reach, env.ydev, 0.0, 0.0),
        key: Vec4::ZERO,
        scat: Vec4::new(0.0, 0.0, 16.0, 0.0),
        box_lo: lo.extend(1.0),
        box_hi: hi.extend(0.0),
        cam_fwd: Vec4::NEG_Z,
        pulse: Vec4::new(r.pulse, 0.0, if r.lightning > 0.0 { 1.0 } else { 0.0 }, 0.0),
        ring: Vec4::new(r.ring[0], r.ring[1], r.ring[2], 0.0),
        rpatch: Vec4::from(r.ring_patch),
        ring_a: Vec3::from(r.ring_cols[0]).extend(0.0),
        ring_b: Vec3::from(r.ring_cols[1]).extend(0.0),
        ring_c: Vec3::from(r.ring_cols[2]).extend(0.0),
        ring_d: Vec3::from(r.ring_cols[3]).extend(0.0),
        swirl: Vec4::new(r.swirl, r.swirl_scale, 0.0, 0.0),
        flash0: Vec4::ZERO,
        flash1: Vec4::ZERO,
        flash2: Vec4::ZERO,
        flash3: Vec4::ZERO,
        flash_c: Vec3::from(r.lightning_colour).extend(1.0 / (r.lightning_size * reach).powi(2).max(1e-6)),
        sh0: Vec4::ZERO,
        sh1: Vec4::ZERO,
        sh2: Vec4::ZERO,
        shp: Vec4::ZERO,
    };
    let material = materials.add(NebulaMaterial {
        params,
        noise: noise.image.clone(),
        envelope: Some(images.add(env_img)),
        shadow: None,
    });
    let centre = (lo + hi) * 0.5;
    let size = hi - lo;
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(centre),
        ChildOf(parent),
        bevy::camera::visibility::NoFrustumCulling,
        NebulaField {
            material,
            recipe,
            reach,
            span: (x1 - x0) + (z1 - z0),
            dome,
            centres: pos.to_vec(),
            radius,
            shown: vec![1.0; pos.len()],
            region: None,
            lit: pos.to_vec(),
            lightning: Lightning::new(seed),
            last: -1.0,
        },
        Visibility::default(),
    )).id()
}

/// Each frame: the camera, the key light, the flow's drift and the edge-on dimming.
#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    camera: Query<(Entity, &GlobalTransform, &Projection, Has<NebulaLowRes>, &Camera), crate::map::MainCamera>,
    lights: Query<(&GlobalTransform, &DirectionalLight)>,
    shadow: Option<Res<crate::render::shadows::KeyShadow>>,
    quality: Option<Res<crate::render::quality::RenderQuality>>,
    mut fields: Query<(Entity, &mut NebulaField, Has<LowResGas>, &InheritedVisibility)>,
    mut materials: ResMut<Assets<NebulaMaterial>>,
    clock: Res<crate::clock::AnimClock>,
) {
    let Ok((cam_entity, cam, proj, has_low, camera_info)) = camera.single() else { return };
    let cam_size = camera_info.physical_target_size();
    // The march at a fraction of the view's size needs the depth prepass to stop at the
    // scene and a perspective view to find its rays.
    let divisor = match quality.as_deref().map_or(0, |q| q.nebula_scale) {
        0 => cam_size.map_or(1, |s| crate::render::quality::auto_nebula_scale(s.x, s.y)),
        d => d,
    };
    let low = divisor > 1 && quality.as_deref().is_none_or(|q| q.depth_prepass) && matches!(proj, Projection::Perspective(_)) && fields.iter().any(|(_, _, _, visible)| visible.get());
    if low {
        commands.entity(cam_entity).insert(NebulaLowRes { divisor });
    } else if has_low {
        commands.entity(cam_entity).remove::<NebulaLowRes>();
    }
    let near = match proj {
        Projection::Perspective(p) => p.near,
        _ => 1.0,
    };
    let key = lights
        .iter()
        .max_by(|a, b| a.1.illuminance.total_cmp(&b.1.illuminance))
        .map(|(t, _)| t.back().as_vec3())
        .unwrap_or(Vec3::Y);
    let steps = match quality.as_deref().map_or(0, |q| q.nebula_steps) {
        0 => crate::render::quality::default_nebula_steps(if low { divisor } else { 1 }),
        n => n,
    };
    let fwd = cam.forward().as_vec3();
    let t = clock.wall() as f32;
    for (entity, mut f, has, visible) in &mut fields {
        // While the low-resolution path draws the gas, the box is on no camera's layers.
        if low && visible.get() && !has {
            commands.entity(entity).insert((LowResGas, RenderLayers::none()));
        } else if (!low || !visible.get()) && has {
            commands.entity(entity).remove::<(LowResGas, RenderLayers)>();
        }
        let f = &mut *f;
        let Some(mut m) = materials.get_mut(&f.material) else { continue };
        if let Some(s) = shadow.as_deref()
            && m.shadow.as_ref() != Some(&s.image)
        {
            m.shadow = Some(s.image.clone());
        }
        let r = &f.recipe;
        let p = &mut m.params;
        p.eye = cam.translation().extend(r.near_fade);
        p.cam_fwd = fwd.extend(0.0);
        p.box_lo.w = near;
        let share = if r.key_share >= 0.0 { r.key_share } else { KEY_SHARE };
        let g = if r.scatter >= 0.0 { r.scatter } else { SCATTER };
        p.key = key.extend(share);
        // Ships' shadows through the gas (RND-15).
        apply_shadow(p, shadow.as_deref());
        p.scat = Vec4::new(1.0 + g * g, 2.0 * g, steps as f32, 0.0);
        p.flow.x = (t * r.flow_speed) % 100.0;
        // Edge-on dimming: a long path through a thin field adds up; dim it unless
        // the gas's own opacity already does.
        let rf = 2.8 * f.reach;
        let path = if fwd.y.abs() > 1e-3 { (2.0 * p.vert.y / fwd.y.abs()).min(f.span) } else { f.span };
        let s = if path > rf && r.opacity <= 0.0 { (rf / path).powf(r.edge_dim) } else { 1.0 };
        p.vert.w = s / f.reach;
        // Motion on the clock: the core's pulse, the ring's colours, the swirl's circles.
        p.pulse.y = r.pulse_phase(t);
        p.ring.w = r.ring_phase(t);
        let [s1, s2] = r.swirl_phases(t);
        p.swirl.z = s1;
        p.swirl.w = s2;
        // Lightning: the flashes now (none on the first frame or after a long gap).
        if r.lightning > 0.0 {
            let dt = if f.last >= 0.0 && t > f.last { t - f.last } else { 0.0 };
            f.last = t;
            let lit = std::mem::take(&mut f.lit);
            let fl = f.lightning.step(r, &lit, f.reach, f.dome, t, dt);
            f.lit = lit;
            p.flash0 = Vec4::from(fl[0]);
            p.flash1 = Vec4::from(fl[1]);
            p.flash2 = Vec4::from(fl[2]);
            p.flash3 = Vec4::from(fl[3]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1146: a nebula of the class's field that was never seen is cleared from the envelope; the one in sight is not.
    #[test]
    fn gate_fields_clears_the_nebulae_never_seen_from_the_envelope() {
        use crate::render::fog_objects::{FogRegions, RegionSights, Sight, update_region_sights};
        use sta_content::{MapDef, Region, RegionKind, RegionShape};
        let v = |x: i32, z: i32| sta_sim::Vec3 { x: sta_sim::Fixed::from_int(x), y: sta_sim::Fixed::ZERO, z: sta_sim::Fixed::from_int(z) };
        let mut map = MapDef::empty();
        for x in [0, 2000] {
            map.regions.push(Region { kind: RegionKind::Nebula, odf: "n".into(), shape: RegionShape::Sphere { centre: v(x, 100), radius: sta_sim::Fixed::from_int(300) } });
        }
        let recipe = NebulaRecipe::parse("[Nebula]\nExtent=1.3\nHeight=3\n");
        // Bevy space: z is mirrored.
        let centres = vec![[0.0, 0.0, -100.0], [2000.0, 0.0, -100.0]];
        let env = sta_surface::nebula::envelope(&recipe, 300.0, &centres);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<Assets<NebulaMaterial>>().init_resource::<Assets<Image>>();
        let image = Image::new(Extent3d { width: env.w, height: env.h, depth_or_array_layers: 1 }, TextureDimension::D2, env.pixels.clone(), TextureFormat::Rgba8Unorm, RenderAssetUsages::default());
        let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let material = app.world_mut().resource_mut::<Assets<NebulaMaterial>>().add(NebulaMaterial { params: NebulaParams::default(), noise: None, envelope: Some(handle.clone()), shadow: None });
        let field = app
            .world_mut()
            .spawn(NebulaField {
                material,
                recipe,
                reach: 390.0,
                span: 1.0,
                dome: 1.0,
                centres: centres.clone(),
                radius: 300.0,
                shown: vec![1.0; 2],
                region: None,
                lit: centres.clone(),
                lightning: Lightning::new(1),
                last: -1.0,
            })
            .id();
        app.insert_resource(FogRegions::of_map(&map))
            .init_resource::<RegionSights>()
            .insert_resource(crate::render::units::LocalPlayer(Some(sta_sim::PlayerId(1))));
        app.add_systems(Update, (update_region_sights, gate_fields).chain());
        // No grid yet: every nebula is drawn (the same rule as the gates and `sync_units`).
        app.update();
        assert_eq!(app.world().get::<NebulaField>(field).unwrap().shown, vec![1.0, 1.0]);
        assert_eq!(app.world().get::<NebulaField>(field).unwrap().region, Some(vec![Some(0), Some(1)]), "each centre's region found once");
        // A grid that has seen only the second nebula.
        let mut grid = crate::render::fog::FogGrid::new([-4096.0, -4096.0], [4096.0, 4096.0]);
        let c = grid.cell_of(2000.0, 100.0).unwrap();
        grid.set_cell_for_test(c % grid.w, c / grid.w, true, true);
        app.insert_resource(crate::render::fog::FogState { grid, updates: 1 });
        app.update();
        assert_eq!(app.world().resource::<RegionSights>().get(0), Sight::Never);
        assert_eq!(app.world().get::<NebulaField>(field).unwrap().shown, vec![0.0, 1.0]);
        let want = sta_surface::nebula::envelope_masked(&app.world().get::<NebulaField>(field).unwrap().recipe, 300.0, &centres, &[false, true]);
        let got = app.world().resource::<Assets<Image>>().get(&handle).unwrap().data.clone().unwrap();
        assert_eq!(got, want.pixels);
        assert_ne!(got, env.pixels);
        assert_eq!(app.world().get::<NebulaField>(field).unwrap().lit, vec![centres[1]], "the lightning strikes among the nebulae drawn");
        // A frame with nothing changed does no work: a marker written into the image stays.
        app.world_mut().resource_mut::<Assets<Image>>().get_mut(&handle).unwrap().data = Some(vec![7; got.len()]);
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Assets<Image>>().get(&handle).unwrap().data.clone().unwrap(), vec![7; got.len()], "no recompute without a change");
        // The second nebula goes out of sight (seen before, not now): a dimmed, still copy; the first stays hidden.
        let mut grid = crate::render::fog::FogGrid::new([-4096.0, -4096.0], [4096.0, 4096.0]);
        grid.set_cell_for_test(c % grid.w, c / grid.w, false, true);
        app.insert_resource(crate::render::fog::FogState { grid, updates: 2 });
        app.update();
        let ghost = crate::render::fog_objects::ghost::NEBULA;
        assert_eq!(app.world().get::<NebulaField>(field).unwrap().shown, vec![0.0, ghost]);
        assert!(app.world().get::<NebulaField>(field).unwrap().lit.is_empty(), "no lightning in a copy out of sight");
        let want = sta_surface::nebula::envelope_weighted(&app.world().get::<NebulaField>(field).unwrap().recipe, 300.0, &centres, &[0.0, ghost]);
        assert_eq!(app.world().resource::<Assets<Image>>().get(&handle).unwrap().data.clone().unwrap(), want.pixels);
        // Everything in sight again: the envelope is the full one.
        let all = crate::render::fog::FogGrid::new([-4096.0, -4096.0], [4096.0, 4096.0]).lifted(true, true);
        app.insert_resource(crate::render::fog::FogState { grid: all, updates: 3 });
        app.update();
        assert_eq!(app.world().get::<NebulaField>(field).unwrap().shown, vec![1.0, 1.0]);
        assert_eq!(app.world().resource::<Assets<Image>>().get(&handle).unwrap().data.clone().unwrap(), env.pixels);
    }

    /// The per-pair form the occlusion had before the discs were computed once.
    fn occlusion_reference(eye: Vec3, nebulae: &[(Vec3, f32)]) -> Vec<f32> {
        let disc = |c: Vec3, r: f32| {
            let d = c - eye;
            let dist = d.length().max(1e-3);
            (d / dist, dist, (r / dist).min(1.0).asin())
        };
        nebulae
            .iter()
            .enumerate()
            .map(|(i, &(ci, ri))| {
                let (di, dist_i, ai) = disc(ci, ri);
                let mut k = 1.0f32;
                for (j, &(cj, rj)) in nebulae.iter().enumerate() {
                    let (dj, dist_j, aj) = disc(cj, rj);
                    if j == i || dist_j >= dist_i || dist_j < rj {
                        continue;
                    }
                    let theta = di.dot(dj).clamp(-1.0, 1.0).acos();
                    let cover = ((aj + ai - theta) / (2.0 * ai).max(1e-4)).clamp(0.0, 1.0);
                    k *= 1.0 - OCCLUDE * cover;
                }
                k
            })
            .collect()
    }

    #[test]
    fn occlusion_is_the_same_as_the_per_pair_form_on_a_crowd() {
        // A fixed pseudo-random crowd, an eye outside and one inside some of the discs.
        let mut x = 0x2545f491u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x % 10_000) as f32 / 10_000.0
        };
        let crowd: Vec<(Vec3, f32)> = (0..150).map(|_| (Vec3::new(next() * 6000.0, next() * 100.0, next() * 6000.0), 60.0 + next() * 600.0)).collect();
        // Eyes outside, inside some discs, and exactly at a nebula's centre (distance clamped to 1e-3).
        for eye in [Vec3::new(3000.0, 1200.0, -500.0), Vec3::new(1500.0, 40.0, 2500.0), crowd[7].0, crowd[100].0] {
            let (new, old) = (occlusion(eye, &crowd), occlusion_reference(eye, &crowd));
            assert_eq!(new.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), old.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        }
        // A dense crowd of small and large nebulae (many discs touching, some just), seen from
        // eyes above, level with and among them, near and far: the pairs skipped without the
        // arccosine change no bit.
        let crowd: Vec<(Vec3, f32)> =
            (0..700).map(|_| (Vec3::new(next() * 20000.0, next() * 400.0 - 200.0, next() * 20000.0), 20.0 + next() * next() * 900.0)).collect();
        let mut dimmed = 0;
        for n in 0..40 {
            let eye = match n % 4 {
                0 => Vec3::new(next() * 20000.0, 200.0 + next() * 4000.0, next() * 20000.0),
                1 => Vec3::new(next() * 20000.0, next() * 10.0, next() * 20000.0),
                2 => crowd[n * 13].0 + Vec3::new(next() * 50.0, next() * 50.0, next() * 50.0),
                _ => Vec3::new(-30000.0 + next() * 80000.0, next() * 30000.0, -30000.0 + next() * 80000.0),
            };
            let (new, old) = (occlusion(eye, &crowd), occlusion_reference(eye, &crowd));
            assert_eq!(new.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), old.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "eye {eye}");
            dimmed += old.iter().filter(|&&k| k < 1.0).count();
        }
        assert!(dimmed > 1000, "{dimmed}");
        // Discs placed at their touching angle and a little either side of it, in steps finer and
        // coarser than the pair test's margin.
        for s in -400..=400 {
            let (ra, rb) = (120.0f32, 45.0f32);
            let touch = (ra / 1000.0).asin() + (rb / 500.0).asin();
            let theta = touch + s as f32 * 1e-5;
            let pair = [(Vec3::new(1000.0, 0.0, 0.0), ra), (Vec3::new(theta.cos(), 0.0, theta.sin()) * 500.0, rb)];
            let (new, old) = (occlusion(Vec3::ZERO, &pair), occlusion_reference(Vec3::ZERO, &pair));
            assert_eq!(new.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), old.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "offset {s}");
        }
    }

    /// A crowd of hundreds of nebulae with degenerate ones among them (coincident discs, a radius
    /// of 0, negative, infinite or not a number, a centre not a number, at infinity or too far for
    /// its distance to be finite), seen from an eye that moves through and around it, at the
    /// centre of one, inside others and far off: the index changes no bit of the pair loop's
    /// result. Deliberate faults in the index (too small a query, the wrong order) do.
    #[test]
    fn indexed_occlusion_is_the_pair_loops_for_a_moving_eye_and_degenerate_nebulae() {
        let mut x = 0x9e37_79b9u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x % 100_000) as f32 / 100_000.0
        };
        let mut crowd: Vec<(Vec3, f32)> = (0..700)
            .map(|_| (Vec3::new(next() * 20000.0 - 10000.0, next() * 300.0 - 150.0, next() * 20000.0 - 10000.0), 15.0 + next() * next() * 800.0))
            .collect();
        let p = crowd[17].0;
        crowd.extend([
            (p, crowd[17].1),
            (p, 40.0),
            (Vec3::new(120.0, 0.0, 80.0), 0.0),
            (Vec3::new(-300.0, 10.0, 500.0), -50.0),
            (Vec3::new(800.0, 0.0, -900.0), f32::INFINITY),
            (Vec3::new(-800.0, 0.0, 900.0), f32::NAN),
            (Vec3::new(f32::NAN, 0.0, 0.0), 100.0),
            (Vec3::new(f32::INFINITY, 0.0, 100.0), 100.0),
            (Vec3::new(3.0e38, 0.0, -3.0e38), 100.0),
            (Vec3::new(0.0, -2000.0, 0.0), 1.0e6),
        ]);
        let path = |f: usize| {
            let t = f as f32 / 100.0;
            Vec3::new(-11000.0 + 22000.0 * t, 60.0 + 1500.0 * (t * 9.0).sin().max(0.0), 3000.0 * (t * 5.0).sin())
        };
        let bits = |v: &[f32]| v.iter().map(|k| k.to_bits()).collect::<Vec<_>>();
        let (mut dimmed, mut wide_seen, mut eyes) = (0, 0, Vec::new());
        for f in 0..100 {
            eyes.push(path(f));
        }
        // At a nebula's centre, inside the coincident pair, and at a point of the path.
        eyes.extend([p, p + Vec3::new(5.0, 3.0, -4.0), crowd[400].0, Vec3::new(0.0, 25000.0, 0.0)]);
        let mut index = Occlusion::default();
        let mut out = Vec::new();
        for (n, &eye) in eyes.iter().enumerate() {
            let old = occlusion_reference(eye, &crowd);
            index.run(eye, &crowd, &mut out);
            assert_eq!(bits(&out), bits(&old), "eye {n} {eye}");
            dimmed += old.iter().filter(|&&k| k < 1.0).count();
            wide_seen += index.wide.len();
        }
        assert!(dimmed > 10_000 && wide_seen > 500, "{dimmed} {wide_seen}");
        // The same crowd through the faults: each changes some bit.
        for fault in [1, 2] {
            let mut bad = Occlusion { fault, ..Default::default() };
            let differ = eyes.iter().filter(|&&eye| {
                bad.run(eye, &crowd, &mut out);
                bits(&out) != bits(&occlusion_reference(eye, &crowd))
            });
            assert!(differ.count() > 10, "fault {fault} goes unseen");
        }
    }

    /// A small xorshift generator for the stress scenes.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.next()
        }

        fn unit(&mut self) -> [f64; 3] {
            let z = self.range(-1.0, 1.0);
            let a = self.range(0.0, std::f64::consts::TAU);
            let s = (1.0 - z * z).sqrt();
            [s * a.cos(), z, s * a.sin()]
        }
    }

    fn normalised(v: [f64; 3]) -> [f64; 3] {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    }

    /// Nebulae seen from `eye` that stress the index: directions spread over the sphere, in tight
    /// clusters, at the poles, and with x on the boundaries of a size class's cells; half-angles
    /// at a class bound and a hair either side; distances over four decades; and a few degenerate
    /// ones (a radius of 0, below 0, infinite, not a number or denormal, a centre at the eye).
    fn stress_scene(seed: u64, eye: Vec3) -> Vec<(Vec3, f32)> {
        let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        for _ in 0..4 {
            r.next();
        }
        let n = 60 + (r.next() * 200.0) as usize;
        let axis = r.unit();
        let spread = 10f64.powf(r.range(-6.0, -0.3));
        let mut out = Vec::new();
        for _ in 0..n {
            let mode = (seed + (r.next() * 3.0) as u64) % 4;
            let d = match mode {
                0 => r.unit(),
                1 => {
                    let o = r.unit();
                    normalised([axis[0] + spread * o[0], axis[1] + spread * o[1], axis[2] + spread * o[2]])
                }
                2 => {
                    let sign = if r.next() < 0.5 { 1.0 } else { -1.0 };
                    normalised([r.range(-1e-3, 1e-3), sign, r.range(-1e-3, 1e-3)])
                }
                _ => {
                    let l = 1 + (r.next() * 7.0) as usize;
                    let bound = class_bound(l);
                    let cells = ((2.0 * GRID_OFFSET / (2.0 * (2.0 * bound).sin() + GRID_PAD)).floor() as usize).clamp(1, GRID_CELLS);
                    let size = 2.0 * GRID_OFFSET / cells as f64;
                    let k = (cells as f64 * r.range(0.2, 0.8)).floor();
                    let x = -GRID_OFFSET + k * size;
                    let z = r.range(-0.5, 0.5);
                    let y = (1.0 - x * x - z * z).max(0.0).sqrt() * if r.next() < 0.5 { 1.0 } else { -1.0 };
                    [x, y, z]
                }
            };
            let h = if r.next() < 0.4 {
                let l = (r.next() * GRID_LEVELS as f64) as usize;
                class_bound(l) + [0.0, 1e-12, -1e-12, 1e-7, -1e-7][(r.next() * 5.0) as usize]
            } else {
                10f64.powf(r.range(-3.0, -0.15))
            };
            let dist = 10f64.powf(r.range(0.5, 4.0));
            let radius = (dist * (2.0 * (h - PAIR_MARGIN)).max(0.0).sin()) as f32;
            let c = eye + Vec3::new(d[0] as f32, d[1] as f32, d[2] as f32) * dist as f32;
            out.push((c, radius));
        }
        out.extend([
            (eye, 10.0),
            (eye + Vec3::new(1e-4, 0.0, 0.0), 10.0),
            (eye + Vec3::new(300.0, 20.0, 0.0), 0.0),
            (eye + Vec3::new(-300.0, 20.0, 40.0), -5.0),
            (eye + Vec3::new(0.0, -200.0, 500.0), f32::INFINITY),
            (eye + Vec3::new(50.0, 0.0, 500.0), f32::NAN),
            (eye + Vec3::new(80.0, 10.0, 900.0), 1e-40),
            (Vec3::new(3.0e38, 0.0, -3.0e38), 100.0),
        ]);
        out
    }

    /// The index offers, for every disc on a grid, every partner the pair loop keeps, and none
    /// twice (the bit comparisons would pass if a partner it missed happened to give a factor of 1).
    /// Scenes with clustered and polar directions, directions on cell boundaries and half-angles at
    /// the class bounds.
    #[test]
    fn the_grids_offer_every_partner_the_pair_loop_keeps_once() {
        let (mut kept, mut at_bound, mut polar, mut on_grid) = (0, 0, 0, 0);
        let mut found = Vec::new();
        for seed in 0..300u64 {
            let eye = Vec3::new((seed % 7) as f32 * 3.0, (seed % 5) as f32, 0.0);
            let scene = stress_scene(seed, eye);
            let n = scene.len();
            let mut ix = Occlusion::default();
            ix.load(eye, &scene);
            ix.grids();
            let mut mark = vec![false; n];
            for i in 0..n {
                if ix.level[i] as usize == GRID_LEVELS {
                    continue;
                }
                on_grid += 1;
                let h = ix.half[i].0;
                at_bound += (0..GRID_LEVELS).any(|l| (h - class_bound(l)).abs() < 1e-6) as u32;
                polar += (ix.discs[i].0.y.abs() > 0.999) as u32;
                ix.partners::<false>(i, &scene, &mut found);
                mark.fill(false);
                for &j in &found {
                    assert!(!mark[j as usize], "seed {seed}: disc {i} is offered {j} twice");
                    mark[j as usize] = true;
                }
                for j in 0..n {
                    if !ix.skips::<false>(i, j, &scene) {
                        assert!(mark[j], "seed {seed}: the pair loop keeps {j} for disc {i}, the index does not offer it");
                        kept += 1;
                    }
                }
            }
        }
        assert!(kept > 5_000 && at_bound > 1_000 && polar > 500 && on_grid > 10_000, "{kept} {at_bound} {polar} {on_grid}");
    }

    /// With the eye moving by a few units a frame (and now and then jumping), the candidate lists
    /// give the pair loop's result by bits, and every partner the pair loop keeps is in the list of
    /// a disc that has one. A list too narrow by its drift and margin is seen.
    #[test]
    fn candidate_lists_give_the_pair_loops_result_for_a_slowly_moving_eye() {
        let bits = |v: &[f32]| v.iter().map(|k| k.to_bits()).collect::<Vec<_>>();
        let (mut served, mut kept, mut dimmed, mut narrow_seen) = (0, 0, 0, 0);
        let mut out = Vec::new();
        for seed in 0..24u64 {
            let scene = stress_scene(seed, Vec3::ZERO);
            let n = scene.len();
            let mut r = Rng(seed + 77);
            let mut eye = Vec3::new(r.range(-20.0, 20.0) as f32, r.range(-20.0, 20.0) as f32, r.range(-20.0, 20.0) as f32);
            let mut ix = Occlusion::default();
            let mut bad = Occlusion { fault: 3, ..Default::default() };
            for step in 0..70 {
                eye += Vec3::new(r.range(-4.0, 4.0) as f32, r.range(-2.0, 2.0) as f32, r.range(-4.0, 4.0) as f32);
                if step == 41 {
                    eye += Vec3::new(90.0, 5.0, -60.0);
                }
                let old = occlusion_reference(eye, &scene);
                let before = ix.lists.served;
                ix.run(eye, &scene, &mut out);
                assert_eq!(bits(&out), bits(&old), "seed {seed} step {step} eye {eye}");
                dimmed += old.iter().filter(|&&k| k < 1.0).count();
                if ix.lists.served > before {
                    served += 1;
                    for i in (0..n).filter(|&i| ix.lists.listed[i]) {
                        let js = &ix.lists.items[ix.lists.start[i] as usize..ix.lists.start[i + 1] as usize];
                        assert!(js.windows(2).all(|w| w[0] < w[1]), "list of {i} is not ascending");
                        for j in (0..n).filter(|&j| !ix.skips::<false>(i, j, &scene)) {
                            assert!(js.contains(&(j as u32)), "seed {seed} step {step}: the pair loop keeps {j} for disc {i}, its list lacks it");
                            kept += 1;
                        }
                    }
                }
                bad.run(eye, &scene, &mut out);
                narrow_seen += (bits(&out) != bits(&old)) as u32;
            }
        }
        assert!(served > 24 * 40 && kept > 5_000 && dimmed > 5_000, "{served} {kept} {dimmed}");
        assert!(narrow_seen > 5, "a list without its drift and margin goes unseen");
    }

    /// A direction at angle `theta` from `u` (unit vectors in f64), turned about a random axis.
    fn turned(r: &mut Rng, u: [f64; 3], theta: f64) -> [f64; 3] {
        let w = r.unit();
        let dot = w[0] * u[0] + w[1] * u[1] + w[2] * u[2];
        let w = normalised([w[0] - dot * u[0], w[1] - dot * u[1], w[2] - dot * u[2]]);
        [0, 1, 2].map(|k| u[k] * theta.cos() + w[k] * theta.sin())
    }

    fn at(e: Vec3, d: [f64; 3], dist: f64) -> Vec3 {
        e + Vec3::new(d[0] as f32, d[1] as f32, d[2] as f32) * dist as f32
    }

    /// Whether `ix`, after serving an eye from lists, has every partner the pair loop keeps in the
    /// list of each disc that has one; and the result by bits equal to the pair loop's.
    struct Served {
        contained: bool,
        bits_equal: bool,
    }

    /// Lists made at `e0` for `scene`, then the eye `e` (within the ball) served from them.
    fn serve_at(fault: u8, e0: Vec3, e: Vec3, scene: &[(Vec3, f32)]) -> Served {
        let mut ix = Occlusion { fault, ..Default::default() };
        let mut out = Vec::new();
        ix.run(e0, scene, &mut out);
        let before = ix.lists.served;
        ix.run(e, scene, &mut out);
        assert!(ix.lists.served > before, "the eye {e} is not served from the lists made at {e0}");
        let mut contained = true;
        for i in (0..scene.len()).filter(|&i| ix.lists.listed[i]) {
            let js = &ix.lists.items[ix.lists.start[i] as usize..ix.lists.start[i + 1] as usize];
            contained &= (0..scene.len()).filter(|&j| !ix.skips::<false>(i, j, scene)).all(|j| js.contains(&(j as u32)));
        }
        let bits_equal = out.iter().map(|k| k.to_bits()).eq(occlusion_reference(e, scene).iter().map(|k| k.to_bits()));
        Served { contained, bits_equal }
    }

    /// Pairs the pair test keeps only by its margin (their discs do not overlap, so their factor
    /// is exactly 1), far away so that the lists' drift is small: a list made without the slack
    /// misses them, a list with it never does. The result by bits cannot show a missing slack,
    /// as those pairs give a factor of 1: the containment is what shows it.
    #[test]
    fn a_list_needs_its_slack_for_pairs_the_pair_test_keeps_by_its_margin() {
        let mut r = Rng(0x51ac);
        let (mut missed_without, mut trials) = (0, 0);
        for _ in 0..600 {
            let e0 = Vec3::new(r.range(-40.0, 40.0) as f32, r.range(-40.0, 40.0) as f32, r.range(-40.0, 40.0) as f32);
            let (di, dj) = (r.range(1e5, 4e5), r.range(1e5, 4e5));
            let (ai, aj) = (r.range(0.005, 0.1), r.range(0.005, 0.1));
            let (ri, rj) = ((di * ai.sin()) as f32, (dj * aj.sin()) as f32);
            let u = r.unit();
            // Between the discs' touching angle and the pair test's margin beyond it.
            let theta = ai + aj + r.range(0.2e-3, 1.8e-3);
            let v = turned(&mut r, u, theta);
            let scene = [(at(e0, u, di), ri), (at(e0, v, dj), rj)];
            let edge = |r: &mut Rng| {
                let n = r.unit();
                e0 + Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32) * (LIST_REACH * 0.9999) as f32
            };
            for e in [e0, edge(&mut r)] {
                let ok = serve_at(0, e0, e, &scene);
                assert!(ok.contained && ok.bits_equal);
                missed_without += !serve_at(4, e0, e, &scene).contained as u32;
                trials += 1;
            }
        }
        assert!(missed_without > trials / 10, "a list without its slack goes unseen: {missed_without} of {trials}");
    }

    /// An eye at the edge of the ball (0.9999 of the reach) reached by a sideways move, perpendicular
    /// to both directions or along their plane, with a near and a far disc whose directions turn by
    /// very different angles: the result is the pair loop's, every kept partner is in the list, and
    /// a list made without the drift term loses pairs whose discs overlap (seen in the bits as well).
    #[test]
    fn a_list_needs_its_drift_for_an_eye_at_the_edge_of_the_ball() {
        let mut r = Rng(0xd41f7);
        let (mut missed_without, mut differs_without, mut dimmed) = (0, 0, 0);
        for _ in 0..600 {
            let e0 = Vec3::new(r.range(-40.0, 40.0) as f32, r.range(-40.0, 40.0) as f32, r.range(-40.0, 40.0) as f32);
            // One disc near and wide, the other far: moving the eye in their plane turns them by
            // very different angles.
            let (di, dj) = (r.range(150.0, 250.0), r.range(800.0, 3000.0));
            let (ai, aj) = (r.range(0.1, 0.5), r.range(0.02, 0.1));
            let (ri, rj) = ((di * ai.sin()) as f32, (dj * aj.sin()) as f32);
            let u = r.unit();
            let theta = r.range(0.0, ai + aj + 0.3);
            let v = turned(&mut r, u, theta);
            let scene = [(at(e0, u, di), ri), (at(e0, v, dj), rj)];
            let cross = normalised([u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]);
            // In the plane of the two directions, along the way from the first to the second.
            let dot = u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
            let tangent = normalised([v[0] - dot * u[0], v[1] - dot * u[1], v[2] - dot * u[2]]);
            for (n, sign) in [(cross, 1.0), (cross, -1.0), (tangent, 1.0), (tangent, -1.0)] {
                let e = e0 + Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32) * (sign * LIST_REACH * 0.9999) as f32;
                let ok = serve_at(0, e0, e, &scene);
                assert!(ok.contained && ok.bits_equal, "{e0} {e} {scene:?}");
                dimmed += occlusion_reference(e, &scene).iter().filter(|&&k| k < 1.0).count();
                let bad = serve_at(5, e0, e, &scene);
                missed_without += !bad.contained as u32;
                differs_without += !bad.bits_equal as u32;
            }
        }
        assert!(dimmed > 100 && missed_without > 20 && differs_without > 5, "{dimmed} {missed_without} {differs_without}");
    }

    /// Pairs placed just inside, exactly on and just outside the angle a list reaches: a pair inside
    /// is in the list, one outside is not (the list is no wider than its bound), the one on it gives
    /// the pair loop's result.
    #[test]
    fn a_list_holds_a_pair_just_inside_its_bound_and_not_one_just_outside() {
        let mut r = Rng(0xb0u64);
        let probe = Occlusion::default();
        let (mut inside, mut outside) = (0, 0);
        for _ in 0..400 {
            let e0 = Vec3::new(r.range(-40.0, 40.0) as f32, r.range(-40.0, 40.0) as f32, r.range(-40.0, 40.0) as f32);
            let (di, dj) = (r.range(150.0, 5000.0), r.range(150.0, 5000.0));
            let (ai, aj) = (r.range(0.01, 0.4), r.range(0.01, 0.4));
            let (ri, rj) = ((di * ai.sin()) as f32, (dj * aj.sin()) as f32);
            let u = r.unit();
            // The bound: twice the sum of the two lists' half-angles, from the discs as stored.
            let place = |v: [f64; 3]| [(at(e0, u, di), ri), (at(e0, v, dj), rj)];
            let probe_scene = place(u);
            let d0 = |c: Vec3| {
                let (x, y, z) = (c.x as f64 - e0.x as f64, c.y as f64 - e0.y as f64, c.z as f64 - e0.z as f64);
                (x * x + y * y + z * z).sqrt()
            };
            let bound = 2.0 * (probe.list_half(ri, d0(probe_scene[0].0)) + probe.list_half(rj, d0(probe_scene[1].0)));
            for (scale, want) in [(1.0 - 1e-4, Some(true)), (1.0, None), (1.0 + 1e-4, Some(false))] {
                let v = turned(&mut r, u, bound * scale);
                let scene = place(v);
                let mut ix = Occlusion::default();
                let mut out = Vec::new();
                ix.run(e0, &scene, &mut out);
                let listed = ix.lists.items[ix.lists.start[0] as usize..ix.lists.start[1] as usize].contains(&1);
                match want {
                    Some(true) => {
                        assert!(listed, "a pair just inside the bound is not listed");
                        inside += 1;
                    }
                    Some(false) => {
                        assert!(!listed, "a pair just outside the bound is listed");
                        outside += 1;
                    }
                    None => {}
                }
                assert_eq!(out.iter().map(|k| k.to_bits()).collect::<Vec<_>>(), occlusion_reference(e0, &scene).iter().map(|k| k.to_bits()).collect::<Vec<_>>());
            }
        }
        assert!(inside == 400 && outside == 400);
    }

    #[test]
    fn near_fade_ramps_from_a_quarter_to_a_quarter_past_the_whole() {
        assert_eq!(near_fade(10.0, 5.0), 0.0);
        assert_eq!(near_fade(15.0, 5.0), 0.0);
        assert!((near_fade(45.0, 5.0) - 0.5).abs() < 1e-5);
        assert_eq!(near_fade(80.0, 5.0), 1.0);
        // A large puff fades over its own size.
        assert!((near_fade(250.0, 400.0) - 0.375).abs() < 1e-5);
    }

    #[test]
    fn a_nebula_behind_another_is_thinned_and_the_front_one_is_not() {
        let eye = Vec3::ZERO;
        let o = occlusion(eye, &[(Vec3::new(0.0, 0.0, -400.0), 100.0), (Vec3::new(0.0, 0.0, -1000.0), 100.0)]);
        assert_eq!(o[0], 1.0);
        assert!((o[1] - 0.5).abs() < 0.05, "{o:?}");
        // Off to one side: untouched.
        let o = occlusion(eye, &[(Vec3::new(0.0, 0.0, -400.0), 100.0), (Vec3::new(900.0, 0.0, -400.0), 100.0)]);
        assert_eq!(o, vec![1.0, 1.0]);
        // The eye inside one does not dim what lies beyond.
        let o = occlusion(eye, &[(Vec3::new(0.0, 0.0, -50.0), 100.0), (Vec3::new(0.0, 0.0, -1000.0), 100.0)]);
        assert_eq!(o[1], 1.0);
    }

    /// The board fade as it was before #1235: every visible nebula's hierarchy walked each frame,
    /// with the occlusion's pair loop over every pair.
    fn fade_boards_reference(
        mut commands: Commands,
        camera: Query<&GlobalTransform, crate::map::MainCamera>,
        nebulae: Query<(Entity, &GlobalTransform, &NebulaBoards, &InheritedVisibility)>,
        children: Query<&Children>,
        mut puffs: Puffs,
    ) {
        let Ok(cam) = camera.single() else { return };
        let eye = cam.translation();
        let list: Vec<_> = nebulae.iter().filter(|(_, _, _, v)| v.get()).map(|(e, g, b, _)| (e, g.translation(), b.radius)).collect();
        let occ = occlusion_reference(eye, &list.iter().map(|&(_, c, r)| (c, r)).collect::<Vec<_>>());
        for (&(root, _, _), occ) in list.iter().zip(occ) {
            for e in children.iter_descendants(root) {
                let Ok((g, t, fade)) = puffs.get_mut(e) else { continue };
                let v = near_fade(g.translation().distance(eye), t.scale.x * 0.5) * occ;
                match fade {
                    Some(mut f) => {
                        if (f.0 - v).abs() > 1e-3 {
                            f.0 = v;
                        }
                    }
                    None => {
                        commands.entity(e).insert(crate::render::sprites::SpriteFade(v));
                    }
                }
            }
        }
    }

    /// `sprites::face_camera` as it was before #1235: the rotation written every frame.
    fn face_camera_reference(
        camera: Query<&GlobalTransform, crate::map::MainCamera>,
        parents: Query<&GlobalTransform, Without<crate::render::sprites::Billboard>>,
        mut boards: Query<(&mut Transform, &ChildOf), With<crate::render::sprites::Billboard>>,
    ) {
        let Ok(cam) = camera.single() else { return };
        let cam_rot = cam.compute_transform().rotation;
        for (mut tf, child_of) in &mut boards {
            let parent_rot = parents.get(child_of.parent()).map(|g| g.compute_transform().rotation).unwrap_or(Quat::IDENTITY);
            tf.rotation = parent_rot.inverse() * cam_rot;
        }
    }

    fn puff(w: &mut World, parent: Entity, at: Vec3, half: f32) -> Entity {
        w.spawn((
            Transform::from_translation(at).with_scale(Vec3::new(half * 2.0, half * 2.0, 1.0)),
            crate::render::sprites::SpriteFx { node_anim: None, sprite: String::new(), colour: [1.0; 3], material: Handle::default() },
            crate::render::sprites::Billboard,
            ChildOf(parent),
        ))
        .id()
    }

    /// A nebula at `at` of `radius`: two nodes (one turned), each with a few puffs of varied size.
    fn nebula(w: &mut World, at: Vec3, radius: f32, seed: u32) -> Entity {
        let root = w.spawn((Transform::from_translation(at), NebulaBoards { radius }, InheritedVisibility::VISIBLE)).id();
        for n in 0..2u32 {
            let node = w
                .spawn((Transform::from_xyz(n as f32 * 70.0 - 35.0, 10.0, 0.0).with_rotation(Quat::from_rotation_y(n as f32 * 0.7)), ChildOf(root)))
                .id();
            for k in 0..5u32 {
                let s = (seed * 7 + n * 5 + k) as f32;
                puff(w, node, Vec3::new((s * 37.0) % 90.0 - 45.0, (s * 13.0) % 30.0 - 15.0, (s * 53.0) % 90.0 - 45.0), 20.0 + (s * 11.0) % 70.0);
            }
        }
        root
    }

    fn fade_app(reference: bool) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin));
        if reference {
            app.add_systems(Update, face_camera_reference)
                .add_systems(PostUpdate, fade_boards_reference.after(TransformSystems::Propagate));
        } else {
            app.add_systems(Update, crate::render::sprites::face_camera)
                .add_systems(PostUpdate, fade_boards.after(TransformSystems::Propagate));
        }
        let w = app.world_mut();
        w.spawn((Camera3d::default(), Transform::from_xyz(0.0, 900.0, 1400.0).looking_at(Vec3::ZERO, Vec3::Y)));
        nebula(w, Vec3::new(0.0, 0.0, 0.0), 160.0, 1);
        nebula(w, Vec3::new(0.0, 0.0, -600.0), 200.0, 2);
        nebula(w, Vec3::new(500.0, 0.0, 100.0), 150.0, 3);
        nebula(w, Vec3::new(-450.0, 40.0, -250.0), 180.0, 4);
        app
    }

    /// #1146: a billboard nebula whose gate is out of sight has its puffs' light scaled by the out-of-sight factor
    /// (a still copy at a reduced intensity); in sight it is as before, and the change is written on the next pass.
    #[test]
    fn a_billboard_nebula_out_of_sight_has_its_puffs_dimmed_and_in_sight_unchanged() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin));
        app.add_systems(Update, crate::render::sprites::face_camera).add_systems(PostUpdate, fade_boards.after(TransformSystems::Propagate));
        let w = app.world_mut();
        w.spawn((Camera3d::default(), Transform::from_xyz(0.0, 900.0, 1400.0).looking_at(Vec3::ZERO, Vec3::Y)));
        let gate = w.spawn((Transform::IDENTITY, Visibility::Inherited)).id();
        let a = nebula(w, Vec3::ZERO, 160.0, 1);
        let b = nebula(w, Vec3::new(900.0, 0.0, 0.0), 160.0, 1);
        w.entity_mut(a).insert(ChildOf(gate));
        let fades = |app: &mut App, root: Entity| -> Vec<f32> {
            let w = app.world_mut();
            let mut q = w.query::<(&crate::render::sprites::SpriteFade, &ChildOf)>();
            let nodes: Vec<Entity> = w.query::<(Entity, &ChildOf)>().iter(w).filter(|(_, c)| c.parent() == root).map(|(e, _)| e).collect();
            let mut v: Vec<f32> = q.iter(w).filter(|(_, c)| nodes.contains(&c.parent())).map(|(f, _)| f.0).collect();
            v.sort_by(f32::total_cmp);
            v
        };
        for _ in 0..3 {
            app.update();
        }
        let (live_a, live_b) = (fades(&mut app, a), fades(&mut app, b));
        assert!(!live_a.is_empty() && live_a.iter().any(|f| *f > 0.0));
        assert_eq!(live_a, live_b, "two copies of one nebula in sight read the same");
        app.world_mut().entity_mut(gate).insert(crate::render::fog_objects::OutOfSight);
        for _ in 0..3 {
            app.update();
        }
        let ghost = crate::render::fog_objects::ghost::NEBULA;
        let dimmed = fades(&mut app, a);
        let sum = |v: &[f32]| v.iter().sum::<f32>();
        assert!((sum(&dimmed) - sum(&live_a) * ghost).abs() < 0.01 * live_a.len() as f32, "dimmed {dimmed:?} against {live_a:?}");
        assert_eq!(fades(&mut app, b), live_b, "the nebula in sight is unchanged");
        app.world_mut().entity_mut(gate).remove::<crate::render::fog_objects::OutOfSight>();
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(fades(&mut app, a), live_a, "back in sight: as before");
    }

    /// The same scripted change, frame by frame, to either app (entities are spawned in the same
    /// order in both, so their ids agree).
    fn script(w: &mut World, frame: u32) {
        let cam = w.query_filtered::<Entity, With<Camera>>().single(w).ok();
        let roots: Vec<Entity> = {
            let mut q = w.query_filtered::<Entity, (With<NebulaBoards>, Without<ChildOf>)>();
            let mut r: Vec<_> = q.iter(w).collect();
            r.sort();
            r
        };
        let puffs: Vec<Entity> = {
            let mut q = w.query_filtered::<Entity, With<crate::render::sprites::SpriteFx>>();
            let mut p: Vec<_> = q.iter(w).collect();
            p.sort();
            p
        };
        let set_cam = |w: &mut World, t: Transform| {
            if let Some(c) = cam {
                *w.get_mut::<Transform>(c).unwrap() = t;
            }
        };
        let start = Transform::from_xyz(0.0, 900.0, 1400.0).looking_at(Vec3::ZERO, Vec3::Y);
        match frame {
            // Still, then panned without turning, into the first nebula.
            0..20 => {}
            20..40 => set_cam(w, start.with_translation(start.translation.lerp(Vec3::new(0.0, 20.0, 60.0), (frame - 19) as f32 / 20.0))),
            // Turning while it moves through the nebula.
            40..60 => {
                let p = Vec3::new(0.0, 20.0, 60.0).lerp(Vec3::new(-20.0, 10.0, -500.0), (frame - 39) as f32 / 20.0);
                set_cam(w, Transform::from_translation(p).looking_at(Vec3::new(300.0, 0.0, -900.0), Vec3::Y));
            }
            // Drifting by less than the write threshold a frame.
            80..100 => {
                let t = *w.get::<Transform>(cam.unwrap()).unwrap();
                set_cam(w, t.with_translation(t.translation + Vec3::new(0.02, 0.0, 0.03)));
            }
            // Out again, turning.
            130..145 => {
                let p = Vec3::new(-20.0, 10.0, -500.0).lerp(Vec3::new(800.0, 600.0, 900.0), (frame - 129) as f32 / 15.0);
                set_cam(w, Transform::from_translation(p).looking_at(Vec3::ZERO, Vec3::Y));
            }
            _ => {}
        }
        match frame {
            // A nebula leaves the view while the camera moves, and comes back.
            30 => {
                w.entity_mut(roots[1]).insert(InheritedVisibility::HIDDEN);
            }
            50 => {
                w.entity_mut(roots[1]).insert(InheritedVisibility::VISIBLE);
            }
            65 => {
                w.entity_mut(roots[3]).insert(InheritedVisibility::HIDDEN);
            }
            // A nebula moves, a puff grows, others' fades are written, dropped, spawned, despawned.
            70 => w.get_mut::<Transform>(roots[2]).unwrap().translation.x -= 300.0,
            75 => {
                w.entity_mut(roots[3]).insert(InheritedVisibility::VISIBLE);
            }
            // (In still frames, so only these changes can start a pass.)
            103 => w.get_mut::<Transform>(puffs[3]).unwrap().scale *= 3.0,
            107 => w.get_mut::<crate::render::sprites::SpriteFade>(puffs[4]).unwrap().0 = 0.123,
            113 => {
                w.entity_mut(puffs[5]).remove::<crate::render::sprites::SpriteFade>();
            }
            // The node above the puff nearest the eye moves (in a still frame with no nested nebula,
            // so only this change can start a pass): its puffs' world positions change, their own
            // transforms do not.
            127 => {
                let eye = w.get::<Transform>(cam.unwrap()).unwrap().translation;
                let d = |w: &World, e: Entity| w.get::<GlobalTransform>(e).unwrap().translation().distance(eye);
                let near = puffs.iter().copied().min_by(|&a, &b| d(w, a).total_cmp(&d(w, b))).unwrap();
                let node = w.get::<ChildOf>(near).unwrap().parent();
                w.get_mut::<Transform>(node).unwrap().translation.y += 40.0;
            }
            100 => {
                let node = w.get::<ChildOf>(puffs[0]).unwrap().parent();
                puff(w, node, Vec3::new(5.0, 5.0, 5.0), 33.0);
            }
            105 => w.entity_mut(puffs[6]).despawn(),
            110 => {
                nebula(w, Vec3::new(-100.0, 0.0, -450.0), 120.0, 5);
            }
            // A nebula nested in another (its puffs under both), then not.
            115 => {
                let node = w.get::<ChildOf>(puffs[0]).unwrap().parent();
                w.entity_mut(node).insert((NebulaBoards { radius: 90.0 }, InheritedVisibility::VISIBLE));
            }
            125 => {
                let node = w.get::<ChildOf>(puffs[0]).unwrap().parent();
                w.entity_mut(node).remove::<NebulaBoards>();
            }
            // No main camera for two frames, with a change in between.
            145 => {
                w.entity_mut(cam.unwrap()).insert(crate::render::action_camera::ActionCamera);
            }
            146 => w.get_mut::<Transform>(puffs[7]).unwrap().scale *= 0.5,
            147 => {
                w.entity_mut(cam.unwrap()).remove::<crate::render::action_camera::ActionCamera>();
            }
            _ => {}
        }
    }

    type Seen = Vec<(Entity, Option<u32>, [u32; 4], [u32; 3])>;

    fn seen(w: &mut World) -> Seen {
        let mut q = w.query_filtered::<(Entity, Option<&crate::render::sprites::SpriteFade>, &Transform, &GlobalTransform), With<crate::render::sprites::SpriteFx>>();
        let mut v: Seen = q
            .iter(w)
            .map(|(e, f, t, g)| (e, f.map(|f| f.0.to_bits()), t.rotation.to_array().map(f32::to_bits), g.translation().to_array().map(f32::to_bits)))
            .collect();
        v.sort();
        v
    }

    /// The kept lists and the skipped frames give, every frame and for every puff, the same
    /// stored fade (by bits) and the same transforms as the walk of every board each frame: still,
    /// panned, turning and drifting cameras, nebulae leaving and entering the view, and puffs and
    /// nebulae that change, appear, vanish or nest.
    #[test]
    fn kept_board_lists_give_the_walks_fades_every_frame() {
        let (mut old, mut new) = (fade_app(true), fade_app(false));
        let mut faded = 0;
        let mut last = Seen::new();
        for frame in 0..160 {
            script(old.world_mut(), frame);
            script(new.world_mut(), frame);
            old.update();
            new.update();
            let (a, b) = (seen(old.world_mut()), seen(new.world_mut()));
            assert_eq!(a, b, "frame {frame}");
            faded += a.iter().filter(|s| s.1.is_some_and(|f| f != 1.0f32.to_bits())).count();
            // The node's move (in a still frame) rewrites some stored fade: a pass that missed it
            // would be seen.
            if frame == 127 {
                assert!(a.iter().zip(&last).any(|(n, o)| n.0 == o.0 && n.1 != o.1), "the node's move writes no fade");
            }
            last = a;
        }
        // The script does dim puffs (near fade and occlusion), so the comparison is not vacuous.
        assert!(faded > 500, "{faded}");
    }

    /// The eye of [`crowd_app`] at a frame: through and around the crowd, turning, at a nebula's
    /// centre at frame 37; still from frame 150 on, inside that nebula.
    fn crowd_eye(frame: u32) -> Transform {
        let f = frame.min(150) as f32;
        let at = Vec3::new(-2600.0 + 34.0 * f, 40.0 + 25.0 * (f * 0.21).sin(), 700.0 * (f * 0.043).sin());
        let at = match frame {
            37 => CROWD_CENTRE,
            150.. => CROWD_CENTRE + Vec3::new(60.0, 10.0, 40.0),
            _ => at,
        };
        Transform::from_translation(at).looking_at(at + Vec3::new((f * 0.05).cos(), -0.2, (f * 0.05).sin()), Vec3::Y)
    }

    /// The nebula the eye of [`crowd_app`] stands at the centre of at frame 37.
    const CROWD_CENTRE: Vec3 = Vec3::new(-1342.0, 31.0, 900.0);

    /// A nebula of one node and three puffs.
    fn small_nebula(w: &mut World, at: Vec3, radius: f32, seed: u32) -> Entity {
        let root = w.spawn((Transform::from_translation(at), NebulaBoards { radius }, InheritedVisibility::VISIBLE)).id();
        let node = w.spawn((Transform::from_xyz(0.0, 5.0, 0.0).with_rotation(Quat::from_rotation_y(seed as f32 * 0.3)), ChildOf(root))).id();
        for k in 0..3u32 {
            let s = (seed * 3 + k) as f32;
            puff(w, node, Vec3::new((s * 37.0) % 120.0 - 60.0, (s * 13.0) % 30.0 - 15.0, (s * 53.0) % 120.0 - 60.0), 15.0 + (s * 11.0) % 80.0);
        }
        root
    }

    /// Hundreds of nebulae over a few thousand units, many overlapping, and degenerate ones:
    /// two at the same centre, radii of 0, below 0, infinite and not a number, a centre not a
    /// number and one at infinity.
    fn crowd_app(reference: bool) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin));
        if reference {
            app.add_systems(Update, face_camera_reference).add_systems(PostUpdate, fade_boards_reference.after(TransformSystems::Propagate));
        } else {
            app.add_systems(Update, crate::render::sprites::face_camera).add_systems(PostUpdate, fade_boards.after(TransformSystems::Propagate));
        }
        let w = app.world_mut();
        w.spawn((Camera3d::default(), crowd_eye(0)));
        let mut x = 0x6a09_e667u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x % 100_000) as f32 / 100_000.0
        };
        for k in 0..300 {
            let at = Vec3::new(next() * 6000.0 - 3000.0, next() * 160.0 - 80.0, next() * 3000.0 - 1500.0);
            small_nebula(w, at, 30.0 + next() * next() * 500.0, k);
        }
        small_nebula(w, CROWD_CENTRE, 260.0, 300);
        small_nebula(w, CROWD_CENTRE, 260.0, 301);
        small_nebula(w, Vec3::new(-200.0, 0.0, 300.0), 0.0, 302);
        small_nebula(w, Vec3::new(400.0, 10.0, -200.0), -80.0, 303);
        small_nebula(w, Vec3::new(900.0, 0.0, 100.0), f32::INFINITY, 304);
        small_nebula(w, Vec3::new(-900.0, 0.0, -100.0), f32::NAN, 305);
        small_nebula(w, Vec3::new(f32::NAN, 0.0, 0.0), 120.0, 306);
        small_nebula(w, Vec3::new(f32::INFINITY, 0.0, 50.0), 120.0, 307);
        app
    }

    /// With hundreds of nebulae, degenerate ones among them, and an eye that moves and turns every
    /// frame through and around them (inside some, at one's centre), then stands while a node
    /// between a nebula and its puffs moves: every puff's stored fade, rotation and world position,
    /// every frame, by bits, as the walk of every board with the pair loop over every pair.
    #[test]
    fn kept_board_lists_give_the_walks_fades_with_a_crowd_and_a_moving_eye() {
        let (mut old, mut new) = (crowd_app(true), crowd_app(false));
        let (mut faded, mut changed) = (0, 0);
        let mut last = Seen::new();
        for frame in 0..180 {
            for app in [&mut old, &mut new] {
                let w = app.world_mut();
                let cam = w.query_filtered::<Entity, With<Camera>>().single(w).unwrap();
                *w.get_mut::<Transform>(cam).unwrap() = crowd_eye(frame);
                let roots: Vec<Entity> = {
                    let mut q = w.query_filtered::<Entity, With<NebulaBoards>>();
                    let mut r: Vec<_> = q.iter(w).collect();
                    r.sort();
                    r
                };
                match frame {
                    // Out of the view and back while the eye moves.
                    60 => {
                        w.entity_mut(roots[40]).insert(InheritedVisibility::HIDDEN);
                    }
                    75 => {
                        w.entity_mut(roots[40]).insert(InheritedVisibility::VISIBLE);
                    }
                    // The node under the nebula nearest the standing eye moves, in a still frame.
                    160 => {
                        let eye = crowd_eye(160).translation;
                        let near = roots
                            .iter()
                            .copied()
                            .filter(|&e| w.get::<GlobalTransform>(e).unwrap().translation().is_finite())
                            .min_by(|&a, &b| {
                                let d = |e: Entity| w.get::<GlobalTransform>(e).unwrap().translation().distance(eye);
                                d(a).total_cmp(&d(b))
                            })
                            .unwrap();
                        let node = w.get::<Children>(near).unwrap()[0];
                        w.get_mut::<Transform>(node).unwrap().translation += Vec3::new(30.0, 20.0, -10.0);
                    }
                    _ => {}
                }
            }
            old.update();
            new.update();
            let (a, b) = (seen(old.world_mut()), seen(new.world_mut()));
            assert_eq!(a, b, "frame {frame}");
            faded += a.iter().filter(|s| s.1.is_some_and(|f| f != 1.0f32.to_bits())).count();
            let writes = a.iter().zip(&last).filter(|(n, o)| n.1 != o.1).count();
            changed += writes;
            if frame == 160 {
                assert!(writes > 0, "the node's move writes no fade");
            }
            last = a;
        }
        // Fades are dimmed and rewritten while the eye moves, so the comparison is not vacuous.
        assert!(faded > 20_000 && changed > 2_000, "{faded} {changed}");
    }
    /// Evaluates a conditional-translation attribute's condition (`A && !B`).
    fn holds(cond: &str, defs: &[&str]) -> bool {
        cond.split("&&").all(|t| {
            let t = t.trim();
            match t.strip_prefix('!') {
                Some(n) => !defs.contains(&n.trim()),
                None => defs.contains(&t),
            }
        })
    }

    /// The shader as Bevy's WESL loader would hand it to naga for these shader defs:
    /// `@if(cond)` keeps or drops the declaration, statement or `{ }` block after it,
    /// and `constants::MATERIAL_BIND_GROUP` is filled in.
    fn variant(defs: &[&str]) -> String {
        variant_of(include_str!("nebula.wesl"), defs)
    }

    fn variant_of(src: &str, defs: &[&str]) -> String {
        let src = src.replace("constants::MATERIAL_BIND_GROUP", "3");
        let lines: Vec<&str> = src.lines().collect();
        let indent_of = |l: &str| l.len() - l.trim_start().len();
        let mut out = String::new();
        let mut i = 0;
        while i < lines.len() {
            let t = lines[i].trim();
            if let Some(cond) = t.strip_prefix("@if(").and_then(|r| r.strip_suffix(')')) {
                let keep = holds(cond, defs);
                i += 1;
                let start = i;
                if lines[i].trim() == "{" {
                    while !(lines[i].trim() == "}" && indent_of(lines[i]) == indent_of(lines[start])) {
                        i += 1;
                    }
                }
                if keep {
                    for l in &lines[start..=i] {
                        out.push_str(l);
                        out.push('\n');
                    }
                }
                i += 1;
                continue;
            }
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
        }
        out
    }

    #[test]
    fn nebula_shader_validates_in_every_variant() {
        for defs in [&[][..], &["DEPTH_PREPASS"], &["DEPTH_PREPASS", "MULTISAMPLED"]] {
            let src = variant(defs);
            let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{defs:?}: {}", e.emit_to_string(&src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{defs:?}: {e:?}"));
        }
    }

    #[test]
    fn low_resolution_shader_validates_in_every_variant() {
        for defs in [&[][..], &["MULTISAMPLED"]] {
            let src = variant_of(include_str!("nebula_lowres.wesl"), defs);
            let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{defs:?}: {}", e.emit_to_string(&src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{defs:?}: {e:?}"));
        }
    }

    #[test]
    fn shader_path_matches_embedded_registration() {
        let registered = bevy::asset::embedded_path!("src", "nebula.wesl");
        assert_eq!(registered.to_string_lossy().replace('\\', "/"), "sta64/render/nebula.wesl");
    }

    #[test]
    fn bundled_recipes_parse() {
        for (name, text) in super::BUNDLED {
            let r = sta_surface::nebula::NebulaRecipe::parse(text);
            assert!(r.inv_tile > 0.0 && r.extent > 0.0, "{name}");
        }
    }
}
