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
            .add_systems(Update, poll_noise)
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
/// `OCCLUDE` times how much of its disc they cover. Stock's `sCullOccludedNebula` thins
/// "where one hides another"; its exact rule is not known to us. `(centre, radius)` per
/// nebula; an eye inside a nebula is not occluded by it.
pub fn occlusion(eye: Vec3, nebulae: &[(Vec3, f32)]) -> Vec<f32> {
    // Each nebula's disc once (direction, distance, angular radius), not once per pair: the
    // pair loop below is O(n^2) and the disc costs a square root and an arcsine.
    let discs: Vec<(Vec3, f32, f32)> = nebulae
        .iter()
        .map(|&(c, r)| {
            let d = c - eye;
            let dist = d.length().max(1e-3);
            (d / dist, dist, (r / dist).min(1.0).asin())
        })
        .collect();
    // For the pair test below, per disc (in f64): half its angular radius plus a margin, with its
    // sine and cosine, so that two discs' bound on their directions' chord costs no sine per pair.
    // A disc whose distance was clamped has no unit direction and is never skipped.
    let half: Vec<(f64, f64, f64)> = nebulae
        .iter()
        .zip(&discs)
        .map(|(&(c, _), &(_, _, a))| {
            let h = if (c - eye).length() >= 1e-3 { a as f64 * 0.5 + PAIR_MARGIN } else { std::f64::consts::PI };
            (h, h.sin(), h.cos())
        })
        .collect();
    discs
        .iter()
        .enumerate()
        .map(|(i, &(di, dist_i, ai))| {
            let mut k = 1.0f32;
            for (j, &(dj, dist_j, aj)) in discs.iter().enumerate() {
                if j == i || dist_j >= dist_i || dist_j < nebulae[j].1 {
                    continue;
                }
                // Discs whose directions are further apart than their angular radii plus twice the
                // margin cover nothing of each other: their factor below is exactly 1 (the cover
                // clamps to 0), so it is skipped without the arccosine. Their chord is then longer
                // than 2 sin((ai + aj) / 2 + margin); the margin (1e-3 rad) is far wider than the
                // rounding of the directions and of the arccosine.
                let ((hi, si, ci), (hj, sj, cj)) = (half[i], half[j]);
                if hi + hj < std::f64::consts::FRAC_PI_2 {
                    let bound = 2.0 * (si * cj + ci * sj);
                    let (dx, dy, dz) = ((di.x - dj.x) as f64, (di.y - dj.y) as f64, (di.z - dj.z) as f64);
                    if dx * dx + dy * dy + dz * dz > bound * bound {
                        continue;
                    }
                }
                let theta = di.dot(dj).clamp(-1.0, 1.0).acos();
                let cover = ((aj + ai - theta) / (2.0 * ai).max(1e-4)).clamp(0.0, 1.0);
                k *= 1.0 - OCCLUDE * cover;
            }
            k
        })
        .collect()
}

/// Half the angular margin (radians) by which [`occlusion`]'s pair test keeps discs that may touch.
const PAIR_MARGIN: f64 = 5e-4;

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
    visible: Vec<(Entity, [u32; 3], u32)>,
    occ: Vec<f32>,
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
    nebulae: Query<(Entity, &GlobalTransform, &NebulaBoards, &InheritedVisibility)>,
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
    let list: Vec<_> = nebulae.iter().filter(|(_, _, _, v)| v.get()).map(|(e, g, b, _)| (e, g.translation(), b.radius)).collect();
    // The occlusion is a pure function of the eye and the visible nebulae (in order): kept while
    // they do not change.
    let visible: Vec<(Entity, [u32; 3], u32)> = list.iter().map(|&(e, c, r)| (e, c.to_array().map(f32::to_bits), r.to_bits())).collect();
    let eye_bits = eye.to_array().map(f32::to_bits);
    let same = cache.eye == Some(eye_bits) && cache.visible == visible;
    if same && !moved && !cache.shared && !cache.inserted {
        return;
    }
    if !same {
        cache.occ = occlusion(eye, &list.iter().map(|&(_, c, r)| (c, r)).collect::<Vec<_>>());
    }
    let occ = std::mem::take(&mut cache.occ);
    let mut inserted = false;
    for (&(root, _, _), &occ) in list.iter().zip(&occ) {
        let Some(list) = cache.puffs.get(&root) else { continue };
        for &e in list {
            let Ok((g, t, fade)) = puffs.get_mut(e) else { continue };
            inserted |= fade_puff(&mut commands, e, g, t, fade, eye, occ);
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
        RenderAssetUsages::RENDER_WORLD,
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
            let fl = f.lightning.step(r, &f.centres, f.reach, f.dome, t, dt);
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

    /// The board fade as it was before #1235: every visible nebula's hierarchy walked each frame.
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
        let occ = occlusion(eye, &list.iter().map(|&(_, c, r)| (c, r)).collect::<Vec<_>>());
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
        for frame in 0..160 {
            script(old.world_mut(), frame);
            script(new.world_mut(), frame);
            old.update();
            new.update();
            let (a, b) = (seen(old.world_mut()), seen(new.world_mut()));
            assert_eq!(a, b, "frame {frame}");
            faded += a.iter().filter(|s| s.1.is_some_and(|f| f != 1.0f32.to_bits())).count();
        }
        // The script does dim puffs (near fade and occlusion), so the comparison is not vacuous.
        assert!(faded > 500, "{faded}");
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
