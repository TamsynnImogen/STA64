//! Rocks with recipe surfaces: mesh parts whose texture names a recipe (the
//! asteroids' `Master*`, the dilithium moons' `Mdmoon`) switch to Armada II Refit's
//! rock look (`rock.wgsl`) once their surface is baked.
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
        ShaderRef::Path(AssetPath::parse("embedded://sta64/rock.wgsl").into_owned())
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

/// The part's own rock material (per instance: it carries the instance's axes).
#[derive(Component)]
struct RockMat(Handle<RockMaterial>);

pub struct RockPlugin;

impl Plugin for RockPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "rock.wgsl");
        app.add_plugins(MaterialPlugin::<RockMaterial>::default())
            .add_systems(Update, apply.after(crate::surfaces::poll))
            .add_systems(PostUpdate, update.after(TransformSystems::Propagate));
    }
}

/// Swap waiting parts to their rock material when the surface is ready.
fn apply(
    mut commands: Commands,
    vfs: Res<crate::viewer::Mounted>,
    mut surfaces: ResMut<crate::surfaces::Surfaces>,
    parts: Query<(Entity, &RockPart), Without<RockMat>>,
    mut materials: ResMut<Assets<RockMaterial>>,
) {
    for (e, part) in &parts {
        let Some(b) = surfaces.request(&vfs.0, &part.recipe, crate::surfaces::ROCK_FACE) else { continue };
        let r = &b.recipe;
        let cut = r.cut > 0.0;
        let params = RockParams {
            amb: Vec3::from(AMBIENT).extend(1.0),
            shine: Vec4::new(SPEC, SPEC_POW, RIM_POW, SUN),
            rim: Vec3::from(RIM).extend(0.0),
            glow: Vec4::new(r.glow[0], r.glow[1], r.glow[2], 0.0),
            misc: Vec4::new(r.depth, 0.0, if cut { 1.0 } else { 0.0 }, if r.triplanar { 1.0 } else { 0.0 }),
            ext: part.inv_extent.extend(r.gloss),
            // The moons' cracks lit from inside; the recipe's glow takes the light's hue.
            core: if cut { (Vec3::new(0.0, 0.5, 1.0) * CRACKS).extend(r.cut.max(1e-3)) } else { Vec4::ZERO },
            spill: Vec4::new(1.0, CRACK_CORE, 0.0, 0.0),
            ..default()
        };
        let m = materials.add(RockMaterial {
            params,
            albedo: Some(b.albedo.clone()),
            relief: Some(b.relief.clone()),
        });
        commands.entity(e).insert((RockMat(m.clone()), MeshMaterial3d(m))).remove::<MeshMaterial3d<StandardMaterial>>();
    }
}

/// Each frame: the instance's axes and centre, the camera and the lights.
fn update(
    camera: Query<&GlobalTransform, With<Camera3d>>,
    lights: Query<(&GlobalTransform, &DirectionalLight)>,
    parts: Query<(Ref<GlobalTransform>, &RockPart, Ref<RockMat>)>,
    mut materials: ResMut<Assets<RockMaterial>>,
    mut last_eye: Local<Vec3>,
) {
    let Ok(cam) = camera.single() else { return };
    // Write a material only when its rock or the camera moved: a write re-uploads it.
    let eye_moved = cam.translation().distance(*last_eye) > 0.01;
    *last_eye = cam.translation();
    let mut ls: Vec<(Vec3, Vec3, f32)> = lights
        .iter()
        .map(|(t, l)| {
            let c = l.color.to_linear();
            (t.forward().as_vec3(), Vec3::new(c.red, c.green, c.blue), l.illuminance)
        })
        .collect();
    ls.sort_by(|a, b| b.2.total_cmp(&a.2));
    let key = ls.first().copied().unwrap_or((Vec3::NEG_Y, Vec3::ONE, 1.0));
    let fill = ls.get(1).map(|f| (f.0, f.1 * (f.2 / key.2.max(1e-3)))).unwrap_or((Vec3::Y, Vec3::ZERO));
    for (gt, part, mat) in &parts {
        if !(eye_moved || gt.is_changed() || mat.is_added()) {
            continue;
        }
        let Some(mut m) = materials.get_mut(&mat.0) else { continue };
        let a = gt.affine();
        let p = &mut m.params;
        p.row0 = Vec3::from(a.matrix3.x_axis).extend(0.0);
        p.row1 = Vec3::from(a.matrix3.y_axis).extend(0.0);
        p.row2 = Vec3::from(a.matrix3.z_axis).extend(0.0);
        p.ctr = gt.transform_point(part.centre).extend(1.0);
        p.eye = cam.translation().extend(1.0);
        p.ddir0 = key.0.extend(1.0);
        p.dcol0 = key.1.extend(0.0);
        p.ddir1 = fill.0.extend(0.0);
        p.dcol1 = fill.1.extend(0.0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rock_shader_validates() {
        let src = include_str!("rock.wgsl").replace("#{MATERIAL_BIND_GROUP}", "3");
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn shader_path_matches_embedded_registration() {
        let registered = bevy::asset::embedded_path!("src", "rock.wgsl");
        assert_eq!(registered.to_string_lossy().replace('\\', "/"), "sta64/rock.wgsl");
    }
}
