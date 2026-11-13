//! Rocks with recipe surfaces: mesh parts whose texture names a recipe (the
//! asteroids' `Master*`, the dilithium moons' `Mdmoon`) switch to Armada II Refit's
//! rock look (`rock.wesl`) once their surface is baked.
//!
//! Settings are Refit's defaults: ambient (0.05, 0.05, 0.07), specular 0.7 with
//! exponent 24 (times the texture's brightness and the recipe's Gloss), rim
//! (0.12, 0.14, 0.20) with exponent 3, the key x1.1; the moons' crack light is stock's
//! nimbus colour (0, 0.5, 1) x MoonCracks 4, its core whitening x1.

use bevy::asset::{embedded_asset, AssetPath};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

const AMBIENT: [f32; 3] = [0.05, 0.05, 0.07];
const SPEC: f32 = 0.7;
const SPEC_POW: f32 = 24.0;
const RIM: [f32; 3] = [0.12, 0.14, 0.20];
const RIM_POW: f32 = 3.0;
const SUN: f32 = 1.1;
const CRACKS: f32 = 4.0;
const CRACK_CORE: f32 = 1.0;
/// How deep the moons' cracks are, in the mesh's radii (Refit's `MoonCrackDepth`), and
/// the exponent of the light's fade up their walls (`MoonCrackFalloff`).
const CRACK_DEPTH: f32 = 0.02;
const CRACK_FALLOFF: f32 = 3.0;
/// Whether the round nimbus (`oreglow`) is drawn behind a moon (Refit's `MoonNimbus`,
/// off: the light shows in the cracks instead).
pub const MOON_NIMBUS: bool = false;

/// The material name of a dilithium moon's glow shell.
pub fn is_moon_shell(material: &str) -> bool {
    material.eq_ignore_ascii_case("glowdmoon")
}

/// The glow shell as Refit draws it by default (`MoonShell=dark`): dark and opaque, so
/// the rock's cut-outs show black where the cracks' light is not. Stock draws it added
/// in the moon's colour; the cracks' own light stands in for that now.
pub fn shell_material(two_sided: bool) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::BLACK,
        unlit: true,
        double_sided: two_sided,
        cull_mode: if two_sided { None } else { Some(bevy::render::render_resource::Face::Back) },
        ..default()
    }
}

#[derive(Clone, Copy, Default, ShaderType, Debug)]
pub struct RockParams {
    pub amb: Vec4,
    pub shine: Vec4,
    pub eye: Vec4,
    pub rim: Vec4,
    pub ddir0: Vec4,
    pub dcol0: Vec4,
    pub ddir1: Vec4,
    pub dcol1: Vec4,
    pub row0: Vec4,
    pub row1: Vec4,
    pub row2: Vec4,
    pub ctr: Vec4,
    pub glow: Vec4,
    pub misc: Vec4,
    pub ext: Vec4,
    pub core: Vec4,
    pub spill: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct RockMaterial {
    #[uniform(0)]
    pub params: RockParams,
    #[texture(1, dimension = "cube")]
    #[sampler(2)]
    pub albedo: Option<Handle<Image>>,
    #[texture(3, dimension = "cube")]
    #[sampler(4)]
    pub relief: Option<Handle<Image>>,
}

impl Material for RockMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/rock.wesl").into_owned())
    }
}

/// A mesh part that takes a recipe surface once baked: the recipe's name, and the
/// mesh's centre (Bevy local space) and 1 / its extent per axis (local units).
#[derive(Component, Clone)]
pub struct RockPart {
    pub recipe: String,
    pub centre: Vec3,
    pub inv_extent: Vec3,
}

/// The part's rock material, shared by every part of the same mesh and recipe (the
/// instance's axes and centre come from its own transform in `rock.wesl`, so a tumbling rock
/// rewrites nothing and the rocks of a mesh draw together).
#[derive(Component)]
struct RockMat;

/// The rock materials made so far, by recipe and the mesh's centre and extents.
#[derive(Resource, Default)]
struct RockMaterials(std::collections::HashMap<MaterialKey, Handle<RockMaterial>>);

type MaterialKey = (String, [u32; 3], [u32; 3]);

/// What a part's material depends on: parts with the same key share one (the per-instance
/// axes and centre are not part of it, they come from the instance's transform).
fn material_key(part: &RockPart) -> MaterialKey {
    (part.recipe.clone(), part.centre.to_array().map(f32::to_bits), part.inv_extent.to_array().map(f32::to_bits))
}

pub struct RockPlugin;

impl Plugin for RockPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "rock.wesl");
        app.add_plugins(MaterialPlugin::<RockMaterial>::default())
            .init_resource::<RockMaterials>()
            .add_systems(Update, apply.after(crate::render::surfaces::poll))
            .add_systems(PostUpdate, update.after(TransformSystems::Propagate));
    }
}

/// Swap waiting parts to their rock material when the surface is ready.
fn apply(
    mut commands: Commands,
    vfs: Res<crate::viewer::Mounted>,
    mut surfaces: ResMut<crate::render::surfaces::Surfaces>,
    parts: Query<(Entity, &RockPart), Without<RockMat>>,
    mut materials: ResMut<Assets<RockMaterial>>,
    mut shared: ResMut<RockMaterials>,
) {
    for (e, part) in &parts {
        let Some(b) = surfaces.request(&vfs.0, &part.recipe, crate::render::surfaces::ROCK_FACE) else { continue };
        let key = material_key(part);
        if let Some(m) = shared.0.get(&key) {
            commands.entity(e).insert((RockMat, MeshMaterial3d(m.clone()))).remove::<MeshMaterial3d<StandardMaterial>>();
            continue;
        }
        let r = &b.recipe;
        let cut = r.cut > 0.0;
        let params = RockParams {
            amb: Vec3::from(AMBIENT).extend(1.0),
            ctr: part.centre.extend(1.0),
            shine: Vec4::new(SPEC, SPEC_POW, RIM_POW, SUN),
            rim: Vec3::from(RIM).extend(0.0),
            glow: Vec4::new(r.glow[0], r.glow[1], r.glow[2], 0.0),
            misc: Vec4::new(r.depth, 0.0, if cut { 1.0 } else { 0.0 }, if r.triplanar { 1.0 } else { 0.0 }),
            ext: part.inv_extent.extend(r.gloss),
            // The moons' cracks lit from inside; the recipe's glow takes the light's hue.
            core: if cut { (Vec3::new(0.0, 0.5, 1.0) * CRACKS).extend(r.cut.max(1e-3)) } else { Vec4::ZERO },
            spill: Vec4::new(1.0, CRACK_CORE, if cut { CRACK_DEPTH } else { 0.0 }, CRACK_FALLOFF),
            ..default()
        };
        let m = materials.add(RockMaterial {
            params,
            albedo: Some(b.albedo.clone()),
            relief: Some(b.relief.clone()),
        });
        shared.0.insert(key, m.clone());
        commands.entity(e).insert((RockMat, MeshMaterial3d(m))).remove::<MeshMaterial3d<StandardMaterial>>();
    }
}

/// What the shared materials were last written with: the eye, the lights, how many materials.
type LastWrite = Option<(Vec3, Vec<[f32; 7]>, usize)>;

/// Each frame: the camera and the lights, into the shared materials, when they changed (a
/// write re-uploads a material: there are a few dozen, not one per rock).
fn update(
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    lights: Query<(&GlobalTransform, &DirectionalLight)>,
    shared: Res<RockMaterials>,
    mut materials: ResMut<Assets<RockMaterial>>,
    mut last: Local<LastWrite>,
    switches: Option<Res<crate::render::quality::PerfSwitches>>,
) {
    if switches.is_some_and(|s| s.freeze_rock_materials) {
        return;
    }
    let Ok(cam) = camera.single() else { return };
    let mut ls: Vec<(Vec3, Vec3, f32)> = lights
        .iter()
        .map(|(t, l)| {
            let c = l.color.to_linear();
            (t.forward().as_vec3(), Vec3::new(c.red, c.green, c.blue), l.illuminance)
        })
        .collect();
    ls.sort_by(|a, b| b.2.total_cmp(&a.2));
    let sig: Vec<[f32; 7]> = ls.iter().map(|l| [l.0.x, l.0.y, l.0.z, l.1.x, l.1.y, l.1.z, l.2]).collect();
    let eye = cam.translation();
    let moved = match &*last {
        Some((e, s, n)) => e.distance(eye) > 0.01 || *s != sig || *n != shared.0.len(),
        None => true,
    };
    if !moved {
        return;
    }
    *last = Some((eye, sig, shared.0.len()));
    let key = ls.first().copied().unwrap_or((Vec3::NEG_Y, Vec3::ONE, 1.0));
    let fill = ls.get(1).map(|f| (f.0, f.1 * (f.2 / key.2.max(1e-3)))).unwrap_or((Vec3::Y, Vec3::ZERO));
    for h in shared.0.values() {
        let Some(mut m) = materials.get_mut(h) else { continue };
        let p = &mut m.params;
        p.eye = eye.extend(1.0);
        p.ddir0 = key.0.extend(1.0);
        p.dcol0 = key.1.extend(0.0);
        p.ddir1 = fill.0.extend(0.0);
        p.dcol1 = fill.1.extend(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard (counts, not speed): the rocks of one mesh share one material, so
    /// an asteroid field is a few draws, not one per rock, and a tumbling rock rewrites
    /// nothing. Instances of a mesh differ in transform only, which is not in the key.
    #[test]
    fn rocks_of_one_mesh_share_a_material_and_other_meshes_do_not() {
        let part = |recipe: &str, c: f32| RockPart { recipe: recipe.into(), centre: Vec3::new(c, 0.0, 1.0), inv_extent: Vec3::splat(0.5) };
        let keys: std::collections::HashSet<_> = (0..600).map(|_| material_key(&part("masterrock", 3.0))).collect();
        assert_eq!(keys.len(), 1, "600 instances of one mesh: one material");
        assert_ne!(material_key(&part("masterrock", 3.0)), material_key(&part("masterrock", 4.0)));
        assert_ne!(material_key(&part("masterrock", 3.0)), material_key(&part("mdmoon", 3.0)));
    }

    #[test]
    fn rock_shader_validates() {
        // The mesh transform comes from Bevy's own shader library: a stub stands in for it.
        let src = include_str!("rock.wesl")
            .replace("constants::MATERIAL_BIND_GROUP", "3")
            .replace("import bevy_pbr::render::mesh_functions::get_world_from_local;", "fn get_world_from_local(i: u32) -> mat4x4<f32> { return mat4x4<f32>(); }");
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn shader_path_matches_embedded_registration() {
        let registered = bevy::asset::embedded_path!("src", "rock.wesl");
        assert_eq!(registered.to_string_lossy().replace('\\', "/"), "sta64/render/rock.wesl");
    }
}
