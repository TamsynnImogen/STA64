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
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct PlanetMaterial {
    #[uniform(0)]
    pub params: PlanetParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
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
}

pub struct PlanetPlugin;

impl Plugin for PlanetPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "planet.wgsl");
        app.add_plugins(MaterialPlugin::<PlanetMaterial>::default()).add_systems(PostUpdate, update.after(TransformSystems::Propagate));
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
                alpha: AlphaMode::Opaque,
            });
            handles.push(m.clone());
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
            alpha: if ground.is_empty() { AlphaMode::Opaque } else { AlphaMode::Blend },
        });
        handles.push(m.clone());
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
    commands.entity(parent).insert(PlanetBody { materials: handles });
    missing
}

/// Each frame: the camera, the planet's centre and the two map lights into every
/// planet material. Halo radii scale with the planet's world scale.
fn update(
    camera: Query<&GlobalTransform, With<Camera3d>>,
    lights: Query<(&GlobalTransform, &DirectionalLight)>,
    planets: Query<(&GlobalTransform, &PlanetBody)>,
    mut materials: ResMut<Assets<PlanetMaterial>>,
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
