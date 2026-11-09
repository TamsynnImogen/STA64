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
    material: Handle<NebulaMaterial>,
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

pub struct NebulaPlugin;

impl Plugin for NebulaPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "nebula.wesl");
        app.add_plugins(MaterialPlugin::<NebulaMaterial>::default())
            .init_resource::<NoiseVolume>()
            .add_systems(Update, poll_noise)
            .add_systems(PostUpdate, update.after(TransformSystems::Propagate));
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
        scat: Vec4::ZERO,
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
    };
    let material = materials.add(NebulaMaterial {
        params,
        noise: noise.image.clone(),
        envelope: Some(images.add(env_img)),
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
fn update(
    camera: Query<(&GlobalTransform, &Projection), crate::map::MainCamera>,
    lights: Query<(&GlobalTransform, &DirectionalLight)>,
    mut fields: Query<&mut NebulaField>,
    mut materials: ResMut<Assets<NebulaMaterial>>,
    clock: Res<crate::clock::AnimClock>,
) {
    let Ok((cam, proj)) = camera.single() else { return };
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
    for mut f in &mut fields {
        let f = &mut *f;
        let Some(mut m) = materials.get_mut(&f.material) else { continue };
        let r = &f.recipe;
        let p = &mut m.params;
        p.eye = cam.translation().extend(r.near_fade);
        p.cam_fwd = fwd.extend(0.0);
        p.box_lo.w = near;
        let share = if r.key_share >= 0.0 { r.key_share } else { KEY_SHARE };
        let g = if r.scatter >= 0.0 { r.scatter } else { SCATTER };
        p.key = key.extend(share);
        p.scat = Vec4::new(1.0 + g * g, 2.0 * g, 0.0, 0.0);
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
        let src = include_str!("nebula.wesl").replace("constants::MATERIAL_BIND_GROUP", "3");
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
