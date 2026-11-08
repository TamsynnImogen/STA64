//! Planet bodies with Armada II Refit's planet look (see `planet.wgsl`): the engine's
//! two ground hemispheres and cloud shell (`space::sphere`), shaded by a material of
//! our own that lights them with the map's key and fill lights, plus the air past
//! the limb as an added halo shell.
//!
//! Settings are Refit's defaults (its `[World]` keys): ambient (0.018, 0.018, 0.027),
//! sun x1.2, fill x0.315, dusk (1, 0.55, 0.35) wrapping 0.25, haze 0.45 with exponent 3
//! in half the ground's own hue and half sky blue, glint 0.3^40, halo 1.0 reaching
//! 4% of the radius above the ground, x2 brighter looking toward the sun.

use crate::space;
use crate::textures::Loader;
use bevy::asset::{embedded_asset, AssetPath};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

const SUN: f32 = 1.2;
const FILL: f32 = 0.315;
const AMBIENT: [f32; 3] = [0.018, 0.018, 0.027];
const DUSK: [f32; 4] = [1.0, 0.55, 0.35, 0.25];
const HAZE: f32 = 0.45;
const HAZE_POW: f32 = 3.0;
const GLINT: f32 = 0.30;
const GLINT_POW: f32 = 40.0;
const HALO: f32 = 1.0;
const HALO_HEIGHT: f32 = 0.04;
const HALO_FORWARD: f32 = 2.0;

#[derive(Clone, Copy, Default, ShaderType, Debug)]
pub struct PlanetParams {
    pub base: Vec4,
    pub mat_diffuse: Vec4,
    pub dcol0: Vec4,
    pub dcol1: Vec4,
    pub dvec0: Vec4,
    pub dvec1: Vec4,
    pub centre: Vec4,
    pub eye: Vec4,
    pub dusk: Vec4,
    pub haze: Vec4,
    pub shine: Vec4,
    pub shell: Vec4,
    pub halo: Vec4,
    pub mode: Vec4,
    pub surf_glow: Vec4,
    pub surf_cloud: Vec4,
    pub surf_drift: Vec4,
    pub surf_misc: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct PlanetMaterial {
    #[uniform(0)]
    pub params: PlanetParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    #[texture(3, dimension = "cube")]
    #[sampler(4)]
    pub albedo: Option<Handle<Image>>,
    #[texture(5, dimension = "cube")]
    #[sampler(6)]
    pub relief: Option<Handle<Image>>,
    #[texture(7, dimension = "cube")]
    #[sampler(8)]
    pub clouds: Option<Handle<Image>>,
    pub alpha: AlphaMode,
}

impl Material for PlanetMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/planet.wgsl").into_owned())
    }
    fn alpha_mode(&self) -> AlphaMode {
        self.alpha
    }
}

/// A planet's materials, updated each frame with the camera and the lights.
#[derive(Component)]
pub struct PlanetBody {
    materials: Vec<Handle<PlanetMaterial>>,
    /// The ground hemispheres' and the shell's materials, for recipe surfaces.
    ground: Vec<Handle<PlanetMaterial>>,
    shell: Option<Handle<PlanetMaterial>>,
    ground_name: String,
    atmo_name: String,
    /// A recipe surface was applied: (ground has one, atmosphere has one, drift rad/s).
    applied: Option<(bool, bool, f32)>,
}

pub struct PlanetPlugin;

impl Plugin for PlanetPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "planet.wgsl");
        app.add_plugins(MaterialPlugin::<PlanetMaterial>::default())
            .init_resource::<crate::surfaces::Surfaces>()
            .add_systems(Update, (crate::surfaces::poll, apply_surfaces).chain())
            .add_systems(PostUpdate, update.after(TransformSystems::Propagate));
    }
}

/// Mean colour of a texture's texels (for the haze hue).
fn mean_colour(loader: &Loader, name: &str) -> Option<[f32; 3]> {
    let (path, fmt) = sta_assets::find_texture(loader.vfs, name)?;
    let img = sta_assets::decode_rgba(loader.vfs, &path, fmt).ok()?;
    let mut s = [0f64; 3];
    let n = (img.width * img.height) as f64;
    for px in img.pixels.chunks_exact(4) {
        for c in 0..3 {
            s[c] += px[c] as f64;
        }
    }
    Some(s.map(|v| (v / n / 255.0) as f32))
}

/// The haze colour: half the ground's own hue, half a sky blue, times the strength.
fn haze_colour(ground: [f32; 3]) -> Vec4 {
    let sky = [0.40, 0.62, 1.00];
    let peak = ground[0].max(ground[1]).max(ground[2]);
    let g: [f32; 3] = std::array::from_fn(|i| 0.5 * if peak > 1e-3 { ground[i] / peak } else { 1.0 } + 0.5 * sky[i]);
    Vec4::new(g[0] * HAZE, g[1] * HAZE, g[2] * HAZE, HAZE_POW)
}

fn base_params() -> PlanetParams {
    PlanetParams {
        base: Vec4::new(AMBIENT[0], AMBIENT[1], AMBIENT[2], 0.0),
        mat_diffuse: Vec4::ONE,
        dusk: Vec4::from(DUSK),
        shine: Vec4::new(GLINT, GLINT_POW, 0.0, 1.0),
        shell: Vec4::new(1.0, 0.0, 0.0, 0.0),
        ..default()
    }
}

/// Spawn a planet body under `parent`. Returns the texture names that were not found.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PlanetMaterial>,
    loader: &mut Loader,
    ground: &str,
    atmosphere: &str,
    radius: f32,
    parent: Entity,
) -> Vec<String> {
    let mut missing = Vec::new();
    let mut handles = Vec::new();
    let mut ground_mats = Vec::new();
    let mut shell_mat = None;
    // Haze hue from both ground hemispheres, else the atmosphere, else grey.
    let means: Vec<[f32; 3]> = if ground.is_empty() {
        mean_colour(loader, atmosphere).into_iter().collect()
    } else {
        (1..=2).filter_map(|h| mean_colour(loader, &format!("{ground}{h}"))).collect()
    };
    let gc = if means.is_empty() {
        [0.5; 3]
    } else {
        std::array::from_fn(|c| means.iter().map(|m| m[c]).sum::<f32>() / means.len() as f32)
    };
    let mut params = base_params();
    params.haze = haze_colour(gc);

    if !ground.is_empty() {
        for h in 0..2 {
            let name = format!("{ground}{}", h + 1);
            let tex = loader.colour(&name, false);
            if tex.is_none() {
                missing.push(name);
            }
            let m = materials.add(PlanetMaterial {
                params: PlanetParams { mode: Vec4::ZERO, ..params },
                texture: tex,
                albedo: None,
                relief: None,
                clouds: None,
                alpha: AlphaMode::Opaque,
            });
            handles.push(m.clone());
            ground_mats.push(m.clone());
            commands.spawn((
                Mesh3d(meshes.add(space::sphere(2 * h, 2, 2, radius))),
                MeshMaterial3d(m),
                Transform::IDENTITY,
                ChildOf(parent),
            ));
        }
    }
    if !atmosphere.is_empty() {
        let tex = loader.colour(atmosphere, false);
        if tex.is_none() {
            missing.push(atmosphere.to_owned());
        }
        let shell_r = if ground.is_empty() { radius } else { radius * 1.01 };
        let m = materials.add(PlanetMaterial {
            params: PlanetParams { mode: Vec4::new(1.0, 0.0, 0.0, 0.0), ..params },
            texture: tex,
            albedo: None,
            relief: None,
            clouds: None,
            alpha: if ground.is_empty() { AlphaMode::Opaque } else { AlphaMode::Blend },
        });
        handles.push(m.clone());
        shell_mat = Some(m.clone());
        commands.spawn((
            Mesh3d(meshes.add(space::sphere(0, 4, 4, shell_r))),
            MeshMaterial3d(m),
            Transform::IDENTITY,
            ChildOf(parent),
        ));
        // The air past the limb: the shell pushed out to the top of the air, added.
        if !ground.is_empty() && HALO > 0.0 {
            let top = radius * (1.0 + HALO_HEIGHT);
            let m = materials.add(PlanetMaterial {
                params: PlanetParams {
                    mode: Vec4::new(2.0, 0.0, 0.0, 0.0),
                    halo: Vec4::new(radius, top, HALO, HALO_FORWARD),
                    ..params
                },
                texture: None,
                albedo: None,
                relief: None,
                clouds: None,
                alpha: AlphaMode::Add,
            });
            handles.push(m.clone());
            commands.spawn((
                Mesh3d(meshes.add(space::sphere(0, 4, 4, (top * 1.03).max(shell_r)))),
                MeshMaterial3d(m),
                Transform::IDENTITY,
                ChildOf(parent),
            ));
        }
    }
    commands.entity(parent).insert(PlanetBody {
        materials: handles,
        ground: ground_mats,
        shell: shell_mat,
        ground_name: ground.to_owned(),
        atmo_name: atmosphere.to_owned(),
        applied: None,
    });
    missing
}

/// Switch planets to their recipe surfaces once baked (Refit's draw_body): a ground
/// recipe draws both hemispheres from the cubes and leaves the shell only its limb
/// glow; an atmosphere recipe (gas giants) draws the shell from the cubes, opaque.
fn apply_surfaces(
    vfs: Res<crate::viewer::Mounted>,
    mut surfaces: ResMut<crate::surfaces::Surfaces>,
    mut planets: Query<&mut PlanetBody>,
    mut materials: ResMut<Assets<PlanetMaterial>>,
) {
    for mut body in &mut planets {
        if body.applied.is_some() {
            continue;
        }
        let face = crate::surfaces::PLANET_FACE;
        let ground = (!body.ground_name.is_empty()).then(|| surfaces.request(&vfs.0, &body.ground_name, face)).flatten();
        let atmo = (!body.atmo_name.is_empty()).then(|| surfaces.request(&vfs.0, &body.atmo_name, face)).flatten();
        let ground_pending = !body.ground_name.is_empty() && ground.is_none() && surfaces.has_recipe(&body.ground_name);
        let atmo_pending = !body.atmo_name.is_empty() && atmo.is_none() && surfaces.has_recipe(&body.atmo_name);
        if ground_pending || atmo_pending {
            continue;
        }
        let set_surface = |m: &mut PlanetMaterial, b: &crate::surfaces::Baked| {
            let r = &b.recipe;
            m.params.mode = Vec4::new(3.0, 0.0, 0.0, 0.0);
            m.params.surf_glow = Vec4::from(r.glow);
            m.params.surf_cloud = Vec4::from(r.cloud_colour);
            m.params.surf_drift = Vec4::new(1.0, 0.0, r.cloud_shadow[0], r.cloud_shadow[1]);
            m.params.surf_misc = Vec4::new(r.depth, if b.clouds.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0);
            m.albedo = Some(b.albedo.clone());
            m.relief = Some(b.relief.clone());
            m.clouds = Some(b.clouds.clone().unwrap_or_else(|| b.relief.clone()));
        };
        let mut drift = 0.0;
        if let Some(b) = &ground {
            for h in &body.ground {
                if let Some(mut m) = materials.get_mut(h) {
                    set_surface(&mut m, b);
                }
            }
            if let Some(mut m) = body.shell.as_ref().and_then(|h| materials.get_mut(h)) {
                m.params.shell.x = 0.0;
            }
            drift = b.recipe.drift;
        }
        if let Some(b) = &atmo
            && let Some(mut m) = body.shell.as_ref().and_then(|h| materials.get_mut(h))
        {
            set_surface(&mut m, b);
            m.alpha = AlphaMode::Opaque;
            drift = b.recipe.drift;
        }
        body.applied = Some((ground.is_some(), atmo.is_some(), drift));
    }
}

/// Each frame: the camera, the planet's centre and the two map lights into every
/// planet material. Halo radii scale with the planet's world scale.
fn update(
    camera: Query<&GlobalTransform, With<Camera3d>>,
    lights: Query<(&GlobalTransform, &DirectionalLight)>,
    planets: Query<(&GlobalTransform, &PlanetBody)>,
    mut materials: ResMut<Assets<PlanetMaterial>>,
    time: Res<Time>,
) {
    let Ok(cam) = camera.single() else { return };
    // Brightest light first: the key.
    let mut ls: Vec<(Vec3, Vec3, f32)> = lights
        .iter()
        .map(|(t, l)| {
            let c = l.color.to_linear();
            (t.back().as_vec3(), Vec3::new(c.red, c.green, c.blue), l.illuminance)
        })
        .collect();
    ls.sort_by(|a, b| b.2.total_cmp(&a.2));
    let key = ls.first().copied().unwrap_or((Vec3::Y, Vec3::ONE, 1.0));
    let fill = ls.get(1).copied().unwrap_or((Vec3::NEG_Y, Vec3::ZERO, 0.0));
    for (gt, body) in &planets {
        let centre = gt.translation();
        for h in &body.materials {
            let Some(mut m) = materials.get_mut(h) else { continue };
            let p = &mut m.params;
            p.centre = centre.extend(1.0);
            p.eye = cam.translation().extend(1.0);
            p.dcol0 = (key.1 * SUN).extend(SUN);
            p.dvec0 = key.0.extend(0.0);
            p.dcol1 = (fill.1 * FILL).extend(0.0);
            p.dvec1 = fill.0.extend(0.0);
            if let Some((_, _, drift)) = body.applied {
                // The clouds drift about the pole on the clock.
                let a = drift * time.elapsed_secs();
                p.surf_drift.x = a.cos();
                p.surf_drift.y = a.sin();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// The shader must parse and validate (naga, as wgpu runs it), with Bevy's
    /// material bind group filled in.
    /// The path `embedded_asset!` registers must be the one the material asks for.
    #[test]
    fn shader_path_matches_embedded_registration() {
        let registered = bevy::asset::embedded_path!("src", "planet.wgsl");
        assert_eq!(registered.to_string_lossy().replace('\\', "/"), "sta64/planet.wgsl");
    }

    #[test]
    fn planet_shader_validates() {
        let src = include_str!("planet.wgsl").replace("#{MATERIAL_BIND_GROUP}", "3");
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
    }
}
