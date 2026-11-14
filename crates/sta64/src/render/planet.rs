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
    /// The colony's alloc picture (RND-12): alpha says how far each town has filled. Only the city layer binds it.
    #[texture(9)]
    #[sampler(10)]
    pub city: Option<Handle<Image>>,
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
    /// The city layer (mode.z 3) shares its planet's origin with the cloud shell, so the sort would tie: it is
    /// pushed back so the shell's clouds are drawn over the lights, as the engine draws them after the ground.
    fn depth_bias(&self) -> f32 {
        if self.params.mode.z > 2.5 { -4.0 } else { 0.0 }
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
    /// The ground's radius, for the city layer (RND-12).
    radius: f32,
}

/// Whether the Stock look is the one drawing planets. Apps without the profile resources (the front end's
/// own frame app) draw the Refit look, as `profile::ProfileEffects::implementation` falls back.
fn stock_scenery(profile: Option<&RenderProfile>, effects: Option<&ProfileEffects>) -> bool {
    matches!((profile, effects), (Some(p), Some(e)) if e.implementation(*p, Effect::Scenery) == RenderProfile::Stock)
}

fn refit_scenery(profile: Option<Res<RenderProfile>>, effects: Option<Res<ProfileEffects>>) -> bool {
    !stock_scenery(profile.as_deref(), effects.as_deref())
}

/// Uploads finished surface bakes under both looks (the dilithium moons take their recipe surface under the
/// Stock look too, see `rock::RockPart::moon`; gating it on the Refit planets left them waiting forever) and
/// gives the Refit planets theirs.
pub(crate) fn add_surface_systems() -> impl bevy::ecs::schedule::IntoScheduleConfigs<bevy::ecs::system::ScheduleSystem, ()> {
    (crate::render::surfaces::poll, apply_surfaces.run_if(refit_scenery)).chain()
}

pub struct PlanetPlugin;

impl Plugin for PlanetPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "planet.wesl");
        app.add_plugins(MaterialPlugin::<PlanetMaterial>::default())
            .init_resource::<crate::render::surfaces::Surfaces>()
            .init_resource::<CityRaces>()
            .add_systems(Update, (add_surface_systems(), place_glows))
            .add_systems(
                Update,
                update_cities
                    .run_if(resource_exists::<crate::sim_link::SimLink>)
                    .run_if(resource_exists::<crate::viewer::Mounted>)
                    .run_if(resource_exists::<crate::clock::AnimClock>),
            )
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
                city: None,
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
            city: None,
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
                city: None,
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
        radius,
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

// ---- Cities (RND-12) ----
//
// Rule (evidence: read from the Armada II executable, static, and from the install's data; research notes live
// outside the repository): a colonized planet gets a second pass on each ground hemisphere, the owner's race's
// development texture `PD_<cityTextureName><1|2>` times an alloc picture built from `PD_<cityTextureName>alloc<1|2>`
// (16x16, value = (R+G+B)/765) as alpha = 0.4 x clamp((P - value) k), P = population / cfgPOP_HEAVY + 1 / k, k = the
// race's `numCities`, blended over the ground and lit as the ground is. The population drawn is not the sim's: it
// eases toward it at 200 a second of simulation time and falls to 0 before a new owner's race is drawn (read from the
// executable, static). Armada I and the Fleet Ops line have no `cityTextureName`, so no layer. Stock shows exactly
// that; the Refit look adds its own night-side lights over the same layer (the original has none).

/// The people per second the drawn population moves by: `cfgPLANET_POP_MORPH_RATE`, a key no `RTS_CFG.h` sets, so the
/// executable's own 200 (read from the executable's data, static).
pub const POP_MORPH_RATE: f32 = 200.0;
/// `numCities` when a race ODF lacks the key (the executable's default).
const DEFAULT_CITY_COUNT: f32 = 10.0;
/// Where the layer sits, as a share of the ground's radius. The hemisphere meshes (`space::sphere`) are front-facing
/// from inside, so under Bevy's back-face culling a planet shows the far wall of its sphere (seen in the RND-12 frame
/// test: a layer outside the ground is hidden behind the ground's far wall except as a ring at the limb). A layer that
/// must draw over the ground therefore sits just inside it, nearer the camera on that far wall and mapped like it.
const CITY_RADIUS: f32 = 0.9985;
/// Refit's `CityLights` and `CityLightColour`.
const CITY_NIGHT: f32 = 0.8;
const CITY_COLOUR: [f32; 3] = [1.0, 0.72, 0.38];

/// What a race ODF says of its cities.
#[derive(Debug, Clone, PartialEq)]
pub struct CityRace {
    /// `cityTextureName` (`ECFR`): the stem of the race's `PD_` textures.
    pub texture: String,
    /// `numCities`: the width of the ramp each town fills over (1 / k of the population share).
    pub count: f32,
}

/// The races with cities, by race key (`name`, lower case) and by ODF stem, read once per mount.
#[derive(Resource, Debug, Default, Clone)]
pub struct CityRaces {
    by_key: std::collections::BTreeMap<String, CityRace>,
    by_stem: std::collections::BTreeMap<String, CityRace>,
}

impl CityRaces {
    /// From the race ODFs through a layered loader (as `Races::load` reads them). Lookup follows `Races::get`: the
    /// ODF's stem first, then the playable race of that key, then any, so two definitions that share a key (an observer
    /// and the race it observes) give the playable one's answer, whatever their order in the file.
    pub fn from_loader(races: &sta_content::Races, load: &mut dyn FnMut(&str, usize) -> Option<String>) -> Self {
        let mut by_key: std::collections::BTreeMap<String, (bool, Option<CityRace>)> = std::collections::BTreeMap::new();
        let mut by_stem = std::collections::BTreeMap::new();
        for race in races.iter() {
            let city = sta_odf::resolve_layered(&race.stem, load).ok().and_then(|odf| {
                let texture = odf.str("citytexturename").map(str::trim).unwrap_or("");
                if texture.is_empty() {
                    return None;
                }
                let count = odf.f32("numcities").filter(|c| c.is_finite() && *c > 0.0).unwrap_or(DEFAULT_CITY_COUNT);
                Some(CityRace { texture: texture.to_owned(), count })
            });
            let playable = race.slot.is_some_and(|slot| slot >= 0);
            if let Some(city) = &city {
                by_stem.insert(race.stem.to_ascii_lowercase(), city.clone());
            }
            let key = race.name.to_ascii_lowercase();
            match by_key.get(&key) {
                Some((had_playable, _)) if *had_playable || !playable => {}
                _ => {
                    by_key.insert(key, (playable, city));
                }
            }
        }
        CityRaces { by_key: by_key.into_iter().filter_map(|(k, (_, c))| c.map(|c| (k, c))).collect(), by_stem }
    }

    pub fn load(vfs: &sta_vfs::Vfs, index: &sta_content::OdfIndex, races: &sta_content::Races) -> Self {
        Self::from_loader(races, &mut |name, version| index.load(vfs, name, version))
    }

    pub fn get(&self, race: &str) -> Option<&CityRace> {
        let n = race.trim().to_ascii_lowercase();
        self.by_stem.get(&n).or_else(|| self.by_key.get(&n))
    }
}

/// One alloc picture: per texel the population share at which its town is full.
#[derive(Debug, Clone, PartialEq)]
pub struct AllocArray {
    pub width: u32,
    pub height: u32,
    pub values: Vec<f32>,
}

impl AllocArray {
    /// From decoded RGBA texels: the value is (R + G + B) / 765.
    pub fn from_rgba(width: u32, height: u32, pixels: &[u8]) -> Option<AllocArray> {
        let n = (width as usize).checked_mul(height as usize)?;
        if n == 0 || n > 1 << 20 || pixels.len() < n * 4 {
            return None;
        }
        let values = pixels.chunks_exact(4).take(n).map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 765.0).collect();
        Some(AllocArray { width, height, values })
    }

    /// The picture's RGBA texels at `pop` people: white, alpha from [`city_alpha`].
    pub fn paint(&self, pop: f32, heavy: f32, count: f32) -> Vec<u8> {
        self.values.iter().flat_map(|&v| [255, 255, 255, city_alpha(v, pop, heavy, count)]).collect()
    }
}

/// A texel's alpha byte: 0.4 x clamp((P - value) k) with P = pop / heavy + 1 / k, k = `count` (read from the executable, static).
pub fn city_alpha(value: f32, pop: f32, heavy: f32, count: f32) -> u8 {
    let p = pop / heavy.max(1.0) + if count != 0.0 { 1.0 / count } else { 0.0 };
    let c = ((p - value) * count).clamp(0.0, 1.0);
    (0.4 * c * 255.9) as u8
}

/// The race and population a planet's cities are drawn at, which follow the sim's by the original's rule (read from the
/// executable, static).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawn {
    pub race: Option<String>,
    pub pop: f32,
}

impl Drawn {
    /// The first sight of a colony: draw what is there at once (decision of the project coordinator, 2026-11-14: how the
    /// original starts a map or a loaded game is not established, and a loaded save or a joined game must not show every
    /// colony filling in over about 25 s). A planet with no owner draws nothing yet, so a colony founded later still
    /// fills in.
    pub fn snap(&mut self, team: Option<&str>, actual: f32) {
        if let Some(team) = team {
            self.race = Some(team.to_owned());
            self.pop = actual;
        }
    }

    /// One step of `dt` seconds of simulation time toward the owner's race `team` and the population `actual`. With no
    /// owner (neutral, or an owner with no race) nothing moves and cities already drawn stay: the original skips the
    /// planet then, per the static reading, but whether that leaves the old cities on screen is not established.
    pub fn step(&mut self, team: Option<&str>, actual: f32, dt: f32) {
        let Some(team) = team else { return };
        if self.race.is_none() {
            self.pop = 0.0;
            self.race = Some(team.to_owned());
        }
        let step = POP_MORPH_RATE * dt;
        if self.race.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(team)) {
            if actual - self.pop < 0.0 || (actual - self.pop).abs() < step {
                self.pop = actual;
            } else {
                self.pop += step;
            }
        } else {
            self.pop -= step;
            if self.pop < 0.0 {
                self.pop = 0.0;
                self.race = Some(team.to_owned());
            }
        }
    }
}

/// Seconds of simulation time since the last call, from the clock's tick counter: nothing while the sim is paused or
/// held (a cutscene), double at double speed, 0 on the first call and when the counter went back (a new map).
fn sim_dt(last: &mut Option<sta_sim::Tick>, now: sta_sim::Tick) -> f32 {
    let dt = last.map_or(0.0, |l| now.saturating_sub(l) as f32 / sta_sim::TICK_HZ as f32);
    *last = Some(now);
    dt
}

/// What the view says of planet `id`: its owner's race (None for a neutral planet or an owner with no race) and its
/// population (0 without a colony row).
pub fn colony_state(view: &sta_sim::WorldView, id: sta_sim::EntityId) -> Option<(Option<String>, f32)> {
    let unit = view.unit(id)?;
    let race = (unit.owner != sta_sim::world::NEUTRAL)
        .then(|| {
            view.attrs
                .seats
                .iter()
                .find(|s| s.player == unit.owner)
                .and_then(|s| s.race.clone())
                .or_else(|| view.attrs.player_races.iter().find(|(p, _)| *p == unit.owner).map(|(_, r)| r.clone()))
        })
        .flatten();
    let pop = view.colonies.iter().find(|c| c.id == id).map_or(0.0, |c| c.population.to_f32());
    Some((race, pop))
}

/// Marks a planet body as the sim planet `.0`, which gives it cities.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanetColony(pub sta_sim::EntityId);

struct CityLayer {
    material: Handle<PlanetMaterial>,
    entity: Entity,
    alloc: Option<Handle<Image>>,
    /// What this layer's picture was last painted as: the race and the texels, so an unchanged repaint uploads nothing.
    shown: Option<(String, Vec<u8>)>,
}

/// The city layers of a [`PlanetColony`] planet and what they are drawn at.
#[derive(Component)]
pub struct PlanetCities {
    layers: Vec<CityLayer>,
    drawn: Drawn,
    /// The colony state has been seen once (the first sight snaps, see [`Drawn::snap`]).
    seen: bool,
    /// What the layers show now: race and the population's bits.
    painted: Option<(String, u32)>,
}

/// Alloc arrays by texture name (`PD_ECFRalloc1`), `None` for a miss. Cleared when the mount changes.
#[derive(Default)]
struct AllocCache(std::collections::HashMap<String, Option<AllocArray>>);

impl AllocCache {
    fn get(&mut self, vfs: &sta_vfs::Vfs, name: &str) -> Option<&AllocArray> {
        self.0
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| {
                let (path, fmt) = sta_assets::find_texture(vfs, name)?;
                let img = sta_assets::decode_rgba(vfs, &path, fmt).map_err(|e| warn!("{path}: {e}")).ok()?;
                AllocArray::from_rgba(img.width, img.height, &img.pixels)
            })
            .as_ref()
    }
}

/// The alloc picture's sampler: the original's filtering of it is not established; linear is used (the frame test of
/// RND-12 compares it with nearest).
fn alloc_image(a: &AllocArray, data: Vec<u8>) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let mut img = Image::new(
        Extent3d { width: a.width, height: a.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..alloc_filter()
    });
    img
}

fn alloc_filter() -> bevy::image::ImageSamplerDescriptor {
    #[cfg(test)]
    if gpu::NEAREST.load(std::sync::atomic::Ordering::Relaxed) {
        return bevy::image::ImageSamplerDescriptor::nearest();
    }
    bevy::image::ImageSamplerDescriptor::linear()
}

/// Each frame: give a tagged planet its two city layers, step what is drawn toward the colony and paint the layers.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_cities(
    mut commands: Commands,
    link: Res<crate::sim_link::SimLink>,
    races: Res<CityRaces>,
    content: Option<Res<crate::sim_link::ContentRes>>,
    vfs: Option<Res<crate::viewer::Mounted>>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
    clock: Res<crate::clock::AnimClock>,
    mut cache: ResMut<crate::render::textures::TextureCache>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PlanetMaterial>>,
    mut planets: Query<(Entity, &PlanetColony, &mut PlanetBody, Option<&mut PlanetCities>)>,
    mut layers: Query<&mut Visibility>,
    mut allocs: Local<AllocCache>,
    mut last_tick: Local<Option<sta_sim::Tick>>,
) {
    let Some(vfs) = vfs else { return };
    // The easing runs on simulation time (the sim's ticks run since the last frame), not wall time: it stands while
    // the game is paused or a cutscene holds the sim and follows the game speed.
    let dt = sim_dt(&mut last_tick, clock.tick());
    // A new content mount: the cached alloc pictures and what the layers show are stale.
    let remounted = vfs.is_changed();
    if remounted {
        allocs.0.clear();
    }
    let view = link.curr().clone();
    let heavy = content.as_ref().and_then(|c| c.0.rules().population()).map_or(5000.0, |p| p.levels[3].to_f32());
    let formats = device
        .map(|d| bevy::image::CompressedImageFormats::from_features(d.features()))
        .unwrap_or(bevy::image::CompressedImageFormats::BC);
    for (parent, colony, mut body, cities) in &mut planets {
        let state = colony_state(&view, colony.0);
        let Some(mut cities) = cities else {
            if state.is_some() && body.ground.len() == 2 && body.radius > 0.0 {
                let mut state = PlanetCities { layers: Vec::new(), drawn: Drawn::default(), seen: false, painted: None };
                for h in 0..2 {
                    let params = PlanetParams {
                        mode: Vec4::new(0.0, 0.0, 3.0, 0.0),
                        shine: Vec4::new(GLINT, GLINT_POW, CITY_NIGHT, 1.0),
                        surf_glow: Vec4::new(CITY_COLOUR[0], CITY_COLOUR[1], CITY_COLOUR[2], 0.0),
                        ..base_params()
                    };
                    let material = materials.add(PlanetMaterial {
                        params,
                        texture: None,
                        albedo: None,
                        relief: None,
                        clouds: None,
                        city: None,
                        alpha: AlphaMode::Blend,
                    });
                    body.materials.push(material.clone());
                    let entity = commands
                        .spawn((
                            Mesh3d(meshes.add(space::sphere(2 * h, 2, 2, body.radius * CITY_RADIUS))),
                            MeshMaterial3d(material.clone()),
                            Transform::IDENTITY,
                            Visibility::Hidden,
                            ChildOf(parent),
                        ))
                        .id();
                    state.layers.push(CityLayer { material, entity, alloc: None, shown: None });
                }
                commands.entity(parent).insert(state);
            }
            continue;
        };
        // The view no longer has the planet (it left the view, or lost its row): hide the layers and forget what was
        // drawn, so a later first sight snaps again.
        let Some((team, actual)) = state else {
            for layer in &mut cities.layers {
                if let Ok(mut v) = layers.get_mut(layer.entity) {
                    *v = Visibility::Hidden;
                }
                layer.shown = None;
            }
            cities.drawn = Drawn::default();
            cities.seen = false;
            cities.painted = None;
            continue;
        };
        if remounted {
            cities.painted = None;
            for layer in &mut cities.layers {
                layer.shown = None;
            }
        }
        if !cities.seen {
            cities.seen = true;
            cities.drawn.snap(team.as_deref(), actual);
        } else {
            cities.drawn.step(team.as_deref(), actual, dt);
        }
        let want = cities.drawn.race.as_deref().and_then(|r| races.get(r).map(|c| (r.to_owned(), c.clone())));
        let shown = want.filter(|_| cities.drawn.pop > 0.0);
        let key = shown.as_ref().map(|(r, _)| (r.clone(), cities.drawn.pop.to_bits()));
        if key == cities.painted {
            continue;
        }
        cities.painted = key;
        let mut loader = crate::render::textures::Loader { vfs: &vfs.0, cache: &mut cache, images: &mut images, formats };
        for h in 0..cities.layers.len() {
            let mut ready = false;
            if let Some((race_key, race)) = &shown {
                let pd = loader.colour(&format!("PD_{}{}", race.texture, h + 1), false);
                let alloc = allocs.get(&vfs.0, &format!("PD_{}alloc{}", race.texture, h + 1)).cloned();
                if let (Some(pd), Some(alloc)) = (pd, alloc) {
                    let data = alloc.paint(cities.drawn.pop, heavy, race.count);
                    let unchanged = cities.layers[h].alloc.is_some() && cities.layers[h].shown.as_ref().is_some_and(|(r, d)| r == race_key && *d == data);
                    if !unchanged {
                        let handle = match cities.layers[h].alloc.clone() {
                            Some(handle) => {
                                if let Some(mut img) = loader.images.get_mut(&handle) {
                                    img.data = Some(data.clone());
                                }
                                handle
                            }
                            None => {
                                let handle = loader.images.add(alloc_image(&alloc, data.clone()));
                                cities.layers[h].alloc = Some(handle.clone());
                                handle
                            }
                        };
                        if let Some(mut m) = materials.get_mut(&cities.layers[h].material) {
                            m.texture = Some(pd);
                            m.city = Some(handle);
                        }
                        cities.layers[h].shown = Some((race_key.clone(), data));
                    }
                    ready = true;
                }
            }
            if !ready {
                cities.layers[h].shown = None;
            }
            if let Ok(mut v) = layers.get_mut(cities.layers[h].entity) {
                *v = if ready { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
    }
}

#[cfg(test)]
#[path = "planet_gpu.rs"]
mod gpu;

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
            city: None,
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
            PlanetBody { materials: vec![handle.clone()], ground: vec![], shell: None, ground_name: String::new(), atmo_name: String::new(), applied: None, radius: 1.0 },
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

    // ---- Cities (RND-12) ----

    /// The pass's alpha (read from the executable, static): 0.4 x clamp((P - value) k), P = pop / heavy + 1 / k. An empty planet shows nothing
    /// for the stock alloc values (the smallest is 1/34, above 1/k for k = 35), a full one 0x66 everywhere.
    #[test]
    fn city_alpha_follows_the_engines_formula() {
        let (heavy, k) = (5000.0, 35.0);
        assert_eq!(super::city_alpha(1.0 / 34.0, 0.0, heavy, k), 0);
        assert_eq!(super::city_alpha(1.0, heavy, heavy, k), 0x66);
        assert_eq!(super::city_alpha(1.0, heavy * 3.0, heavy, k), 0x66, "above heavy it clamps");
        // Half full: a town whose value is 0.5 is fully on, one at 0.6 is off.
        assert_eq!(super::city_alpha(0.5, 2500.0, heavy, k), 0x66);
        assert_eq!(super::city_alpha(0.6, 2500.0, heavy, k), 0);
        // Inside the ramp 1/k wide below the value's share.
        let mid = super::city_alpha(0.5 + 0.5 / k, 2500.0, heavy, k);
        assert!(mid > 0 && mid < 0x66, "{mid}");
    }

    #[test]
    fn alloc_value_is_the_mean_of_the_channels() {
        let a = super::AllocArray::from_rgba(2, 1, &[255, 255, 255, 255, 0, 0, 0, 255]).unwrap();
        assert_eq!(a.values, vec![1.0, 0.0]);
        assert!(super::AllocArray::from_rgba(0, 1, &[]).is_none());
        assert!(super::AllocArray::from_rgba(2, 2, &[0; 4]).is_none());
        let px = a.paint(5000.0, 5000.0, 35.0);
        assert_eq!(px.len(), 8);
        assert_eq!(&px[..3], &[255, 255, 255]);
        assert_eq!(px[3], 0x66, "a full colony shows its town of value 1");
    }

    /// The drawn population (read from the executable, static) rises 200 a second, snaps down, and a new owner's race waits for the
    /// old one to fade out.
    #[test]
    fn drawn_population_eases_as_the_engine_does() {
        let mut d = super::Drawn::default();
        d.step(None, 1000.0, 1.0);
        assert_eq!(d, super::Drawn::default(), "a neutral planet draws nothing new");
        d.step(Some("fx"), 1000.0, 1.0);
        assert_eq!((d.race.as_deref(), d.pop), (Some("fx"), 200.0));
        for _ in 0..3 {
            d.step(Some("fx"), 1000.0, 1.0);
        }
        assert_eq!(d.pop, 800.0);
        d.step(Some("fx"), 1000.0, 1.0);
        assert_eq!(d.pop, 1000.0);
        d.step(Some("fx"), 100.0, 0.01);
        assert_eq!(d.pop, 100.0, "above the real population it snaps down");
        d.step(Some("other"), 5000.0, 0.25);
        assert_eq!((d.race.as_deref(), d.pop), (Some("fx"), 50.0), "the old race fades first");
        d.step(Some("other"), 5000.0, 0.25);
        assert_eq!((d.race.as_deref(), d.pop), (Some("fx"), 0.0), "the swap needs the population below 0");
        d.step(Some("other"), 5000.0, 0.25);
        assert_eq!((d.race.as_deref(), d.pop), (Some("other"), 0.0));
        d.step(Some("other"), 5000.0, 0.25);
        assert_eq!(d.pop, 50.0);
    }

    fn view_with(owner: sta_sim::PlayerId, pop: Option<i32>, race: Option<&str>) -> sta_sim::WorldView {
        use sta_sim::{EntityId, Fixed};
        let mut v = sta_sim::WorldView::default();
        v.units.push(sta_sim::UnitView {
            id: EntityId(5),
            def: sta_sim::ContentKey::NONE,
            owner,
            pos: sta_sim::Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4],
            crew: None,
            energy: None,
            autonomy: None,
            flags: sta_sim::view::UnitFlags::SELECTABLE,
            progress: None,
        });
        if let Some(p) = pop {
            v.colonies.push(sta_sim::colony::ColonyView { id: EntityId(5), population: Fixed::from_int(p), max_population: Fixed::from_int(5000), colonizer: None });
        }
        if let Some(r) = race {
            v.attrs.seats.push(sta_sim::attrs::SeatView { player: owner, race: Some(r.into()), ..Default::default() });
        }
        v
    }

    /// The selection logic: an owned, populated planet gives its owner's race and population; an uncolonized one has
    /// population 0; a neutral one no race; a planet the view does not hold nothing.
    #[test]
    fn colony_state_reads_owner_race_and_population() {
        use sta_sim::{EntityId, PlayerId};
        let v = view_with(PlayerId(1), Some(1000), Some("fx"));
        assert_eq!(super::colony_state(&v, EntityId(5)), Some((Some("fx".into()), 1000.0)));
        let v = view_with(PlayerId(1), None, Some("fx"));
        assert_eq!(super::colony_state(&v, EntityId(5)), Some((Some("fx".into()), 0.0)));
        let v = view_with(sta_sim::world::NEUTRAL, None, None);
        assert_eq!(super::colony_state(&v, EntityId(5)), Some((None, 0.0)));
        assert_eq!(super::colony_state(&v, EntityId(6)), None);
    }

    /// The races with cities come from the race ODFs' `cityTextureName` and `numCities`; a race without the key
    /// (Armada I, the Fleet Ops line) has none; the key's absence of `numCities` means 10.
    #[test]
    fn city_races_come_from_the_race_odfs() {
        let (races, _) = sta_content::Races::load_with(&mut |name, version| {
            if version != 0 {
                return None;
            }
            match name.trim_end_matches(".odf") {
                "races" => Some("numberofraces=3\nrace0=a\nrace1=b\nrace2=c".into()),
                "a" => Some("name=Alpha\ncityTextureName=\"ECFR\"\nnumCities=35.0".into()),
                "b" => Some("name=beta\ncityTextureName=\"BORG\"".into()),
                "c" => Some("name=gamma\ncityTextureName=\"\"".into()),
                _ => None,
            }
        });
        let cities = super::CityRaces::from_loader(&races, &mut |name, version| {
            if version != 0 {
                return None;
            }
            match name.trim_end_matches(".odf") {
                "a" => Some("name=Alpha\ncityTextureName=\"ECFR\"\nnumCities=35.0".into()),
                "b" => Some("name=beta\ncityTextureName=\"BORG\"".into()),
                "c" => Some("name=gamma\ncityTextureName=\"\"".into()),
                _ => None,
            }
        });
        assert_eq!(cities.get("ALPHA"), Some(&super::CityRace { texture: "ECFR".into(), count: 35.0 }));
        assert_eq!(cities.get("beta"), Some(&super::CityRace { texture: "BORG".into(), count: 10.0 }));
        assert_eq!(cities.get("gamma"), None);
    }

    /// The city layer is a third mesh per hemisphere (mode.z 3) in both looks, sorted behind the clouds.
    #[test]
    fn city_shader_has_both_looks() {
        let src = include_str!("planet.wesl");
        assert!(src.contains("fn stock_city") && src.contains("fn city_night") && src.contains("if p.mode.z > 2.5"));
        let m = super::PlanetMaterial {
            params: super::PlanetParams { mode: Vec4::new(0.0, 0.0, 3.0, 0.0), ..default() },
            texture: None,
            albedo: None,
            relief: None,
            clouds: None,
            city: None,
            alpha: AlphaMode::Blend,
        };
        assert!(<super::PlanetMaterial as bevy::pbr::Material>::depth_bias(&m) < 0.0);
    }

    // ---- The city layers in a running app (RND-12) ----

    /// An uncompressed top-left 32-bit TGA of our own, for the test mount.
    fn tga(w: u16, h: u16, rgba: &[u8]) -> Vec<u8> {
        let mut out = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.extend([w as u8, (w >> 8) as u8, h as u8, (h >> 8) as u8, 32, 0x28]);
        for px in rgba.chunks_exact(4) {
            out.extend([px[2], px[1], px[0], px[3]]);
        }
        out
    }

    /// A mount holding race `TEST`'s textures: a white development picture and a 2x2 alloc picture whose texels have the
    /// values 1.0, 140/255, 0.8 and 0.0.
    fn city_mount(tag: &str) -> sta_vfs::Vfs {
        let dir = std::env::temp_dir().join(format!("sta64-rnd12-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("textures")).unwrap();
        let white = [255u8; 16];
        let alloc: Vec<u8> = [255u8, 140, 204, 0].iter().flat_map(|&v| [v, v, v, 255]).collect();
        for h in 1..=2 {
            std::fs::write(dir.join(format!("textures/PD_TEST{h}.tga")), tga(2, 2, &white)).unwrap();
            std::fs::write(dir.join(format!("textures/PD_TESTalloc{h}.tga")), tga(2, 2, &alloc)).unwrap();
        }
        let mut vfs = sta_vfs::Vfs::default();
        vfs.push_dir("test".into(), dir).unwrap();
        vfs
    }

    fn test_races() -> super::CityRaces {
        let (races, _) = sta_content::Races::load_with(&mut |name, version| match (name.trim_end_matches(".odf"), version) {
            ("races", 0) => Some("numberofraces=1\nrace0=a".into()),
            ("a", 0) => Some("name=fx\ncityTextureName=\"TEST\"\nnumCities=10.0".into()),
            _ => None,
        });
        super::CityRaces::from_loader(&races, &mut |name, version| match (name.trim_end_matches(".odf"), version) {
            ("a", 0) => Some("name=fx\ncityTextureName=\"TEST\"\nnumCities=10.0".into()),
            _ => None,
        })
    }

    struct Held(std::sync::Arc<sta_sim::WorldView>);

    impl sta_sim::WorldSource for Held {
        fn view(&self) -> std::sync::Arc<sta_sim::WorldView> {
            self.0.clone()
        }
        fn step(&mut self, _: &[sta_sim::command::Command]) -> sta_sim::TickOutput {
            sta_sim::TickOutput { view: self.0.clone(), events: Vec::new() }
        }
    }

    fn city_app(tag: &str, first: sta_sim::WorldView) -> App {
        use super::*;
        let mut app = App::new();
        app.add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Image>()
            .init_asset::<Mesh>()
            .init_asset::<PlanetMaterial>()
            .init_resource::<crate::render::textures::TextureCache>()
            .insert_resource(test_races())
            .insert_resource(crate::viewer::Mounted(city_mount(tag)))
            .insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Wall, 1))
            .insert_resource(crate::sim_link::SimLink::new(Box::new(Held(std::sync::Arc::new(first))), Box::new(sta_sim::command::LocalRouter::new()), sta_sim::PlayerId(0)))
            .add_systems(Update, update_cities);
        let mut materials = app.world_mut().resource_mut::<Assets<PlanetMaterial>>();
        let ground: Vec<_> = (0..2)
            .map(|_| {
                materials.add(PlanetMaterial { params: base_params(), texture: None, albedo: None, relief: None, clouds: None, city: None, alpha: AlphaMode::Opaque })
            })
            .collect();
        app.world_mut().spawn((
            PlanetColony(sta_sim::EntityId(5)),
            PlanetBody { materials: ground.clone(), ground, shell: None, ground_name: String::new(), atmo_name: String::new(), applied: None, radius: 1.0 },
        ));
        app.update();
        app
    }

    fn feed(app: &mut App, view: sta_sim::WorldView) {
        app.world_mut().resource_mut::<crate::sim_link::SimLink>().feed(std::sync::Arc::new(view));
    }

    /// One frame in which the sim ran `ticks` ticks.
    fn frame(app: &mut App, ticks: u32) {
        let mut clock = app.world_mut().resource_mut::<crate::clock::AnimClock>();
        clock.advance_idle(1.0 / 30.0 * ticks.max(1) as f64);
        for _ in 0..ticks {
            clock.ticked();
        }
        app.update();
    }

    fn drawn(app: &mut App) -> super::Drawn {
        app.world_mut().query::<&super::PlanetCities>().single(app.world()).unwrap().drawn.clone()
    }

    fn layer_entities(app: &mut App) -> Vec<Entity> {
        app.world_mut().query::<&super::PlanetCities>().single(app.world()).unwrap().layers.iter().map(|l| l.entity).collect()
    }

    fn layer_image(app: &mut App, h: usize) -> Option<Handle<Image>> {
        app.world_mut().query::<&super::PlanetCities>().single(app.world()).unwrap().layers[h].alloc.clone()
    }

    fn visible(app: &App, e: Entity) -> bool {
        *app.world().get::<Visibility>(e).unwrap() == Visibility::Inherited
    }

    fn fx(pop: i32) -> sta_sim::WorldView {
        view_with(sta_sim::PlayerId(1), Some(pop), Some("fx"))
    }

    /// The first sight of a colony draws it as it is (the coordinator's decision: a loaded save or a joined game does
    /// not fill in); the layers are made, hidden until painted, and painted with the alloc picture's texels for that
    /// population.
    #[test]
    fn a_colony_seen_for_the_first_time_is_drawn_as_it_is() {
        let mut app = city_app("first", fx(2500));
        let layers = layer_entities(&mut app);
        assert_eq!(layers.len(), 2);
        assert!(layers.iter().all(|&e| !visible(&app, e)), "hidden until painted");
        frame(&mut app, 0);
        assert_eq!(drawn(&mut app), super::Drawn { race: Some("fx".into()), pop: 2500.0 }, "no fill-in at first sight");
        assert!(layers.iter().all(|&e| visible(&app, e)));
        // P = 2500 / 5000 + 1 / 10 = 0.6: value 1.0 and 0.8 are off, 140/255 is mid-ramp, 0 is fully on (0x66).
        for h in 0..2 {
            let handle = layer_image(&mut app, h).unwrap();
            let img = app.world().resource::<Assets<Image>>().get(&handle).unwrap();
            let data = img.data.clone().unwrap();
            assert_eq!(data, vec![255, 255, 255, 0, 255, 255, 255, 52, 255, 255, 255, 0, 255, 255, 255, 0x66]);
            let layer = app.world_mut().query::<&super::PlanetCities>().single(app.world()).unwrap().layers[h].material.clone();
            let m = app.world().resource::<Assets<super::PlanetMaterial>>();
            assert!(m.get(&layer).unwrap().texture.is_some() && m.get(&layer).unwrap().city.is_some());
        }
    }

    /// A colony founded while the planet is watched fills in at 200 a second of simulation time.
    #[test]
    fn a_colony_founded_while_watched_fills_in_gradually() {
        let mut app = city_app("founded", fx(0));
        frame(&mut app, 0);
        assert_eq!(drawn(&mut app).pop, 0.0);
        let layers = layer_entities(&mut app);
        assert!(layers.iter().all(|&e| !visible(&app, e)), "nothing to show at population 0");
        feed(&mut app, fx(1000));
        frame(&mut app, 30);
        assert_eq!(drawn(&mut app).pop, 200.0);
        assert!(layers.iter().all(|&e| visible(&app, e)));
        frame(&mut app, 30);
        assert_eq!(drawn(&mut app).pop, 400.0);
    }

    /// An unowned planet seen first, colonized later: still no snap, it fills in.
    #[test]
    fn a_neutral_planet_colonized_later_fills_in() {
        let mut app = city_app("neutral", view_with(sta_sim::world::NEUTRAL, None, None));
        frame(&mut app, 0);
        assert_eq!(drawn(&mut app), super::Drawn::default());
        feed(&mut app, fx(3000));
        frame(&mut app, 30);
        assert_eq!(drawn(&mut app).pop, 200.0, "no snap: it rises by one step");
        frame(&mut app, 30);
        assert_eq!(drawn(&mut app).pop, 400.0);
    }

    /// Simulation time, not wall time: no rise while the sim is paused (wall time runs on), double speed rises twice
    /// as fast per wall second.
    #[test]
    fn the_drawn_population_follows_simulation_time() {
        let mut app = city_app("simtime", fx(0));
        frame(&mut app, 0);
        feed(&mut app, fx(4000));
        for _ in 0..5 {
            frame(&mut app, 0);
        }
        assert_eq!(drawn(&mut app).pop, 0.0, "paused: five wall frames of one tick's length each, no rise");
        // A wall second at normal speed (30 ticks) and at double speed (60 ticks).
        frame(&mut app, 30);
        assert_eq!(drawn(&mut app).pop, 200.0);
        frame(&mut app, 60);
        assert_eq!(drawn(&mut app).pop, 600.0, "double speed rises twice as fast");
    }

    /// A planet that is gone from the view loses its lights, and its next first sight snaps again.
    #[test]
    fn a_planet_that_leaves_the_view_hides_its_cities() {
        let mut app = city_app("gone", fx(2500));
        frame(&mut app, 0);
        let layers = layer_entities(&mut app);
        assert!(layers.iter().all(|&e| visible(&app, e)));
        feed(&mut app, sta_sim::WorldView::default());
        frame(&mut app, 0);
        assert!(layers.iter().all(|&e| !visible(&app, e)));
        assert_eq!(drawn(&mut app), super::Drawn::default());
        feed(&mut app, fx(4000));
        frame(&mut app, 0);
        assert_eq!(drawn(&mut app).pop, 4000.0, "snaps again");
        assert!(layers.iter().all(|&e| visible(&app, e)));
    }

    /// A repaint whose texels are the same uploads nothing; a different one does.
    #[test]
    fn an_unchanged_picture_is_not_uploaded_again() {
        let mut app = city_app("upload", fx(2500));
        frame(&mut app, 0);
        let handle = layer_image(&mut app, 0).unwrap();
        let sentinel = vec![7u8; 16];
        app.world_mut().resource_mut::<Assets<Image>>().get_mut(&handle).unwrap().data = Some(sentinel.clone());
        feed(&mut app, fx(2501));
        frame(&mut app, 1);
        assert_eq!(drawn(&mut app).pop, 2501.0);
        assert_eq!(app.world().resource::<Assets<Image>>().get(&handle).unwrap().data, Some(sentinel), "same texels: skipped");
        feed(&mut app, fx(1000));
        frame(&mut app, 1);
        assert_ne!(app.world().resource::<Assets<Image>>().get(&handle).unwrap().data.as_deref(), Some(&[7u8; 16][..]), "other texels: uploaded");
    }

    /// The remount drops the cached alloc pictures.
    #[test]
    fn a_new_mount_repaints_from_the_new_pictures() {
        let mut app = city_app("remount", fx(2500));
        frame(&mut app, 0);
        let handle = layer_image(&mut app, 0).unwrap();
        let sentinel = vec![7u8; 16];
        app.world_mut().resource_mut::<Assets<Image>>().get_mut(&handle).unwrap().data = Some(sentinel);
        app.world_mut().resource_mut::<crate::viewer::Mounted>().set_changed();
        frame(&mut app, 0);
        assert_ne!(app.world().resource::<Assets<Image>>().get(&handle).unwrap().data.as_deref(), Some(&[7u8; 16][..]));
    }

    /// Races sharing a key answer as `Races::get` does: the playable one, whatever the file order.
    #[test]
    fn a_shared_race_key_prefers_the_playable_definition() {
        let load = |name: &str, version: usize| -> Option<String> {
            if version != 0 {
                return None;
            }
            match name.trim_end_matches(".odf") {
                "races" => Some("numberofraces=2\nrace0=observer\nrace1=federation".into()),
                "observer" => Some("name=federation\ncityTextureName=\"OBS\"".into()),
                "federation" => Some("name=federation\ninstantActionSlot=2\ncityTextureName=\"ECFR\"\nnumCities=35.0".into()),
                _ => None,
            }
        };
        let (races, _) = sta_content::Races::load_with(&mut |n, v| load(n, v));
        let cities = super::CityRaces::from_loader(&races, &mut |n, v| load(n, v));
        assert_eq!(cities.get("federation").map(|c| c.texture.as_str()), Some("ECFR"));
        assert_eq!(cities.get("observer").map(|c| c.texture.as_str()), Some("OBS"), "the stem first");
    }

    #[test]
    fn the_first_sight_snaps_only_an_owned_planet() {
        let mut d = super::Drawn::default();
        d.snap(None, 900.0);
        assert_eq!(d, super::Drawn::default());
        d.snap(Some("fx"), 900.0);
        assert_eq!((d.race.as_deref(), d.pop), (Some("fx"), 900.0));
    }

    #[test]
    fn simulation_time_is_the_ticks_since_the_last_frame() {
        let mut last = None;
        assert_eq!(super::sim_dt(&mut last, 100), 0.0);
        assert_eq!(super::sim_dt(&mut last, 100), 0.0, "paused");
        assert!((super::sim_dt(&mut last, 130) - 1.0).abs() < 1e-6);
        assert_eq!(super::sim_dt(&mut last, 5), 0.0, "a new map starts the counter again");
        assert!((super::sim_dt(&mut last, 35) - 1.0).abs() < 1e-6);
    }

    /// The hemisphere meshes are front-facing from inside (Bevy culls back faces, so a planet draws the far wall of its
    /// sphere), which is why the city layer sits just inside the ground (`CITY_RADIUS`). If the winding is ever fixed,
    /// this test fails: move the layer outside the ground then.
    #[test]
    fn hemisphere_meshes_face_inward_and_the_city_layer_sits_inside_the_ground() {
        use bevy::mesh::{Indices, VertexAttributeValues};
        let mesh = crate::render::space::sphere(0, 2, 2, 1.0);
        let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("positions") };
        let Some(Indices::U32(idx)) = mesh.indices() else { panic!("indices") };
        let mut inward = 0;
        let mut outward = 0;
        for t in idx.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(pos[i as usize]));
            let n = (b - a).cross(c - a);
            if n.length() < 1e-9 {
                continue;
            }
            // Counter-clockwise is front in Bevy: the front's normal is `n`.
            if n.dot(a + b + c) < 0.0 {
                inward += 1;
            } else {
                outward += 1;
            }
        }
        assert!(inward > 0 && outward == 0, "inward {inward}, outward {outward}");
        const { assert!(super::CITY_RADIUS < 1.0) };
    }
}
