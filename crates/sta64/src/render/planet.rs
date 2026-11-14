//! Under the Stock profile (RND-23) a planet is drawn as the engine draws it: the same ground
//! hemispheres and cloud shell, each texel times the sum of the map's lights (every directional
//! light of the two the map view places, at its colour, equal weight, plus [`STOCK_AMBIENT`]) added in display (sRGB) space as
//! Direct3D 8's fixed pipeline does; no recipe surface, haze, glint or halo (`planet.wesl`, `stock`).
//! The rest of this header is the Refit look.
//!
//! Planet bodies with Armada II Refit's planet look (see `planet.wesl`): the engine's
//! two ground hemispheres and cloud shell (`space::sphere`), shaded by a material of
//! our own that lights them with the map's key and fill lights, plus the air past
//! the limb as an added halo shell.
//!
//! Settings are Refit's defaults (its `[World]` keys): ambient (0.018, 0.018, 0.027),
//! sun x1.2, fill x0.315, dusk (1, 0.55, 0.35) wrapping 0.25, haze 0.45 with exponent 3
//! in half the ground's own hue and half sky blue, glint 0.3^40, halo 1.0 reaching
//! 4% of the radius above the ground, x2 brighter looking toward the sun.

use crate::render::profile::{Effect, ProfileEffects, RenderProfile};
use crate::render::space;
use crate::render::textures::Loader;
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
/// Planet glow (Refit's lights.rs): the sun's colour x the ground's mean x 1.5, full
/// at the surface, gone at 6 radii.
const GLOW: f32 = 1.5;
const GLOW_RANGE: f32 = 6.0;
/// The map's key light (lux), which the glows are calibrated against.
pub const KEY_LUX: f32 = 6000.0;
/// Stock look: the constant term the lights are added to (the maps' own ambient is 0 on every
/// map read; not measured on the engine, tuned against the original's frames).
pub const STOCK_AMBIENT: f32 = 0.0;

/// A planet's glow light: kept on the planet's day side.
#[derive(Component)]
struct PlanetGlow {
    radius: f32,
}

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
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/planet.wesl").into_owned())
    }
    fn alpha_mode(&self) -> AlphaMode {
        self.alpha
    }
    // No prepass: under the map camera's `DepthPrepass` the opaque ground wrote its depth in the prepass but
    // drew nothing in the main pass (the planet was an empty disc with only the shell's rim around it).
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
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

/// Whether the Stock look is the one drawing planets. Apps without the profile resources (the front end's
/// own frame app) draw the Refit look, as `profile::ProfileEffects::implementation` falls back.
fn stock_scenery(profile: Option<&RenderProfile>, effects: Option<&ProfileEffects>) -> bool {
    matches!((profile, effects), (Some(p), Some(e)) if e.implementation(*p, Effect::Scenery) == RenderProfile::Stock)
}

fn refit_scenery(profile: Option<Res<RenderProfile>>, effects: Option<Res<ProfileEffects>>) -> bool {
    !stock_scenery(profile.as_deref(), effects.as_deref())
}

pub struct PlanetPlugin;

impl Plugin for PlanetPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "planet.wesl");
        app.add_plugins(MaterialPlugin::<PlanetMaterial>::default())
            .init_resource::<crate::render::surfaces::Surfaces>()
            .add_systems(Update, ((crate::render::surfaces::poll, apply_surfaces).chain().run_if(refit_scenery), place_glows))
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
        let tex_missing = tex.is_none();
        if tex_missing {
            missing.push(atmosphere.to_owned());
        }
        let shell_r = if ground.is_empty() { radius } else { radius * 1.01 };
        let m = materials.add(PlanetMaterial {
            // mode.w: 1 when the shell's texture was not found (the Stock look then draws no shell).
            params: PlanetParams { mode: Vec4::new(1.0, 0.0, 1.0, if tex_missing { 1.0 } else { 0.0 }), ..params },
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
                    mode: Vec4::new(2.0, 0.0, 2.0, 0.0),
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
    // The glow: Bevy's point lights fall off with distance squared, so it matches the
    // engine's strength at the surface and is cut off at the engine's range.
    let lum = gc[0] * 0.3 + gc[1] * 0.5 + gc[2] * 0.2;
    if lum > 0.0 && radius > 0.0 {
        let colour = Color::srgb(gc[0] / lum.max(1e-3) * 0.5, gc[1] / lum.max(1e-3) * 0.5, gc[2] / lum.max(1e-3) * 0.5);
        commands.spawn((
            PointLight {
                color: colour,
                intensity: GLOW * lum * KEY_LUX * 4.0 * std::f32::consts::PI * radius * radius,
                range: radius * GLOW_RANGE,
                radius: radius * 0.5,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::IDENTITY,
            ChildOf(parent),
            PlanetGlow { radius },
        ));
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
    mut surfaces: ResMut<crate::render::surfaces::Surfaces>,
    mut planets: Query<&mut PlanetBody>,
    mut materials: ResMut<Assets<PlanetMaterial>>,
) {
    for mut body in &mut planets {
        if body.applied.is_some() {
            continue;
        }
        let face = crate::render::surfaces::PLANET_FACE;
        let ground = (!body.ground_name.is_empty()).then(|| surfaces.request(&vfs.0, &body.ground_name, face)).flatten();
        let atmo = (!body.atmo_name.is_empty()).then(|| surfaces.request(&vfs.0, &body.atmo_name, face)).flatten();
        let ground_pending = !body.ground_name.is_empty() && ground.is_none() && surfaces.has_recipe(&body.ground_name);
        let atmo_pending = !body.atmo_name.is_empty() && atmo.is_none() && surfaces.has_recipe(&body.atmo_name);
        if ground_pending || atmo_pending {
            continue;
        }
        let set_surface = |m: &mut PlanetMaterial, b: &crate::render::surfaces::Baked| {
            let r = &b.recipe;
            m.params.mode.x = 3.0;
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
/// Keep each glow a radius out toward the key light: its day side lights ships.
fn place_glows(
    lights: Query<(&GlobalTransform, &DirectionalLight)>,
    planets: Query<&GlobalTransform, With<PlanetBody>>,
    mut glows: Query<(&PlanetGlow, &ChildOf, &mut Transform)>,
) {
    let Some(key) = lights.iter().max_by(|a, b| a.1.illuminance.total_cmp(&b.1.illuminance)).map(|(t, _)| t.back().as_vec3()) else {
        return;
    };
    for (g, child_of, mut tf) in &mut glows {
        let Ok(p) = planets.get(child_of.parent()) else { continue };
        let inv = p.compute_transform().rotation.inverse();
        let want = inv * key * g.radius;
        if tf.translation.distance(want) > 1e-3 {
            tf.translation = want;
        }
    }
}

fn update(
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    lights: Query<(&GlobalTransform, &DirectionalLight, Option<&crate::map::MapLightSource>)>,
    planets: Query<(&GlobalTransform, &PlanetBody)>,
    mut materials: ResMut<Assets<PlanetMaterial>>,
    clock: Res<crate::clock::AnimClock>,
    profile: Option<Res<RenderProfile>>,
    effects: Option<Res<ProfileEffects>>,
) {
    let stock = stock_scenery(profile.as_deref(), effects.as_deref());
    let Ok(cam) = camera.single() else { return };
    // Brightest light first: the key.
    // Stock: the lights the hulls use (`hulls::stock_light_set`); Refit: every directional light.
    let set: Vec<(Vec3, &DirectionalLight)> = if stock {
        crate::render::hulls::stock_light_set(lights.iter())
    } else {
        lights.iter().map(|(t, l, _)| (t.back().as_vec3(), l)).collect()
    };
    let mut ls: Vec<(Vec3, Vec3, f32)> = set
        .into_iter()
        .map(|(to, l)| {
            // Stock multiplies display-space values (as the engine's fixed pipeline does), so its light
            // colour is the map's own sRGB value; Refit's shader works in linear.
            let c = if stock {
                let c = l.color.to_srgba();
                Vec3::new(c.red, c.green, c.blue)
            } else {
                let c = l.color.to_linear();
                Vec3::new(c.red, c.green, c.blue)
            };
            (to, c, l.illuminance)
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
            p.mode.y = stock as u32 as f32;
            let (k, f) = if stock { (1.0, 1.0) } else { (SUN, FILL) };
            p.dcol0 = (key.1 * k).extend(k);
            p.dvec0 = key.0.extend(0.0);
            p.dcol1 = (fill.1 * f).extend(0.0);
            p.base.w = STOCK_AMBIENT;
            p.dvec1 = fill.0.extend(0.0);
            if let Some((_, _, drift)) = body.applied {
                // The clouds drift about the pole on the clock.
                let a = drift * clock.wall() as f32;
                p.surf_drift.x = a.cos();
                p.surf_drift.y = a.sin();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::*;

    /// The shader must parse and validate (naga, as wgpu runs it), with Bevy's
    /// material bind group filled in.
    /// The path `embedded_asset!` registers must be the one the material asks for.
    #[test]
    fn shader_path_matches_embedded_registration() {
        let registered = bevy::asset::embedded_path!("src", "planet.wesl");
        assert_eq!(registered.to_string_lossy().replace('\\', "/"), "sta64/render/planet.wesl");
    }

    #[test]
    fn planet_shader_validates() {
        let src = include_str!("planet.wesl").replace("constants::MATERIAL_BIND_GROUP", "3");
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
    }

    /// The cause of the "huge rings": under the map camera's `DepthPrepass` the opaque ground wrote
    /// depth in the prepass and drew nothing in the main pass. Planets take no prepass.
    #[test]
    fn planet_material_takes_no_prepass() {
        assert!(!<super::PlanetMaterial as bevy::pbr::Material>::enable_prepass());
    }

    /// `AlphaMode::Add` is Bevy's premultiplied blend (src + dst * (1 - alpha)): a halo that returns
    /// alpha 1 replaces what is behind it with black instead of adding to it.
    #[test]
    fn halo_adds_with_alpha_zero() {
        let src = include_str!("planet.wesl");
        let halo = src.split("fn halo_light").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(halo.contains("p.shine.w, 0.0)"), "{halo}");
    }

    fn run_update(profile: super::RenderProfile) -> super::PlanetParams {
        run_update_with(profile, Vec3::new(0.0, 0.0, 10.0), false)
    }

    /// `own`: the two lights are the map's (`MapLightSource`) and a stray third light (a viewer's or the menu
    /// scene's, brightest of all) is present.
    fn run_update_with(profile: super::RenderProfile, eye: Vec3, own: bool) -> super::PlanetParams {
        use super::*;
        let mut app = App::new();
        let mut effects = ProfileEffects::default();
        effects.register_stock(Effect::Scenery);
        app.insert_resource(effects)
            .insert_resource(profile)
            .insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Wall, 1))
            .init_resource::<Assets<PlanetMaterial>>()
            .add_systems(Update, update);
        let handle = app.world_mut().resource_mut::<Assets<PlanetMaterial>>().add(PlanetMaterial {
            params: base_params(),
            texture: None,
            albedo: None,
            relief: None,
            clouds: None,
            alpha: AlphaMode::Opaque,
        });
        app.world_mut().spawn((Camera3d::default(), GlobalTransform::from(Transform::from_translation(eye))));
        let light = |lux: f32, colour: Color, from: Vec3| {
            (DirectionalLight { illuminance: lux, color: colour, ..default() }, GlobalTransform::from(Transform::IDENTITY.looking_to(-from, Vec3::Y)))
        };
        let a = app.world_mut().spawn(light(6000.0, Color::srgb(1.0, 0.5, 0.25), Vec3::Y)).id();
        let b = app.world_mut().spawn(light(1500.0, Color::srgb(0.2, 0.4, 1.0), Vec3::NEG_Y)).id();
        if own {
            for e in [a, b] {
                app.world_mut().entity_mut(e).insert(crate::map::MapLightSource(true));
            }
            app.world_mut().spawn(light(9000.0, Color::WHITE, Vec3::X));
        }
        app.world_mut().spawn((
            GlobalTransform::IDENTITY,
            PlanetBody { materials: vec![handle.clone()], ground: vec![], shell: None, ground_name: String::new(), atmo_name: String::new(), applied: None },
        ));
        app.update();
        app.world().resource::<Assets<PlanetMaterial>>().get(&handle).unwrap().params
    }

    /// Stock: every map light at its own (sRGB) colour and full weight (the engine's fixed pipeline). Refit
    /// keeps its linear colours, key x1.2 and fill x0.315.
    #[test]
    fn stock_lights_planets_with_both_map_lights_at_full_weight() {
        let stock = run_update(super::RenderProfile::Stock);
        assert_eq!(stock.mode.y, 1.0);
        let (k, f) = (stock.dcol0.truncate(), stock.dcol1.truncate());
        let c = |c: Color| { let l = c.to_srgba(); Vec3::new(l.red, l.green, l.blue) };
        assert!(k.abs_diff_eq(c(Color::srgb(1.0, 0.5, 0.25)), 1e-5), "{k:?}");
        assert!(f.abs_diff_eq(c(Color::srgb(0.2, 0.4, 1.0)), 1e-5), "{f:?}");
        assert!(stock.dvec0.truncate().abs_diff_eq(Vec3::Y, 1e-5) && stock.dvec1.truncate().abs_diff_eq(Vec3::NEG_Y, 1e-5));
        let refit = run_update(super::RenderProfile::Refit);
        assert_eq!(refit.mode.y, 0.0);
        assert!((refit.dcol0.w - super::SUN).abs() < 1e-6);
        let lin = Color::srgb(0.2, 0.4, 1.0).to_linear();
        assert!((refit.dcol1.x - lin.red * super::FILL).abs() < 1e-5, "Refit keeps linear light colours");
    }

    /// The planet takes the lights the hulls take: with the map's own lights present a stray directional
    /// light does not light planets, and the direction is the hull shader's (`GlobalTransform::back`).
    #[test]
    fn stock_planet_ignores_non_map_lights_refit_keeps_all() {
        let p = run_update_with(super::RenderProfile::Stock, Vec3::new(0.0, 0.0, 10.0), true);
        assert!(p.dvec0.truncate().abs_diff_eq(Vec3::Y, 1e-5) && p.dvec1.truncate().abs_diff_eq(Vec3::NEG_Y, 1e-5), "{:?} {:?}", p.dvec0, p.dvec1);
        assert!(p.dcol0.truncate().abs_diff_eq(Vec3::new(1.0, 0.5, 0.25), 1e-5), "the stray 9000 lux light must not be the key");
        let r = run_update_with(super::RenderProfile::Refit, Vec3::new(0.0, 0.0, 10.0), true);
        assert!(r.dvec0.truncate().abs_diff_eq(Vec3::X, 1e-5), "Refit keeps every light: the 9000 lux one is the key");
    }

    /// Stock shading reads no camera: moving the camera changes only `eye`.
    #[test]
    fn stock_planet_params_do_not_depend_on_the_camera() {
        let a = run_update_with(super::RenderProfile::Stock, Vec3::new(0.0, 0.0, 10.0), true);
        let b = run_update_with(super::RenderProfile::Stock, Vec3::new(-300.0, 40.0, -90.0), true);
        assert_ne!(a.eye, b.eye);
        let mut b2 = b;
        b2.eye = a.eye;
        assert_eq!(format!("{a:?}"), format!("{b2:?}"));
        let stock = include_str!("planet.wesl").split("fn stock(").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(!stock.contains("p.eye"), "the Stock look must not read the camera");
    }
}
