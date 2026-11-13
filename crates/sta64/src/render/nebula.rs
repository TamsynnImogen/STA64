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
            .add_systems(Update, (poll_noise, fade_boards))
            .add_systems(PostUpdate, update.after(TransformSystems::Propagate).after(crate::render::shadows::ShadowSet));
    }
}

/// A stock nebula drawn as billboards (a class without a gas recipe): its puffs dim
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

/// Dim each billboard nebula's puffs.
fn fade_boards(
    mut commands: Commands,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    nebulae: Query<(Entity, &GlobalTransform, &NebulaBoards)>,
    children: Query<&Children>,
    mut puffs: Query<(&GlobalTransform, &Transform, Option<&mut crate::render::sprites::SpriteFade>), With<crate::render::sprites::SpriteFx>>,
) {
    let Ok(cam) = camera.single() else { return };
    let eye = cam.translation();
    let list: Vec<_> = nebulae.iter().map(|(e, g, b)| (e, g.translation(), b.radius)).collect();
    let occ = occlusion(eye, &list.iter().map(|&(_, c, r)| (c, r)).collect::<Vec<_>>());
    for (&(root, _, _), occ) in list.iter().zip(occ) {
        for e in children.iter_descendants(root) {
            let Ok((g, t, fade)) = puffs.get_mut(e) else { continue };
            // A billboard quad's scale is twice its half size.
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
) {
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
    ));
}

/// Each frame: the camera, the key light, the flow's drift and the edge-on dimming.
#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    camera: Query<(Entity, &GlobalTransform, &Projection, Has<NebulaLowRes>), crate::map::MainCamera>,
    lights: Query<(&GlobalTransform, &DirectionalLight)>,
    shadow: Option<Res<crate::render::shadows::KeyShadow>>,
    quality: Option<Res<crate::render::quality::RenderQuality>>,
    mut fields: Query<(Entity, &mut NebulaField, Has<LowResGas>)>,
    mut materials: ResMut<Assets<NebulaMaterial>>,
    clock: Res<crate::clock::AnimClock>,
) {
    let Ok((cam_entity, cam, proj, has_low)) = camera.single() else { return };
    // The march at a fraction of the view's size needs the depth prepass to stop at the
    // scene and a perspective view to find its rays.
    let divisor = quality.as_deref().map_or(1, |q| q.nebula_scale);
    let low = divisor > 1 && quality.as_deref().is_none_or(|q| q.depth_prepass) && matches!(proj, Projection::Perspective(_)) && !fields.is_empty();
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
    let fwd = cam.forward().as_vec3();
    let t = clock.wall() as f32;
    for (entity, mut f, has) in &mut fields {
        // While the low-resolution path draws the gas, the box is on no camera's layers.
        if low && !has {
            commands.entity(entity).insert((LowResGas, RenderLayers::none()));
        } else if !low && has {
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
        p.scat = Vec4::new(1.0 + g * g, 2.0 * g, quality.as_deref().map_or(16, |q| q.nebula_steps) as f32, 0.0);
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
