//! Effects drawn from sim events, aligned with Armada II Refit: RND-09 is the
//! weapon set, RND-10 impacts, RND-11 specials. The shaders and recipes are ported
//! from Refit (`NOTICE.md`).

pub mod fire;
pub mod impacts;
pub mod quad;
pub mod recipe;
pub mod shield;
pub mod specials;
pub mod weapons;

use bevy::prelude::*;

/// The effects' shaders and materials. Left out of the windowless tests: the
/// effect modules attach visuals only when the material assets exist.
pub struct FxMaterialsPlugin;

impl Plugin for FxMaterialsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((quad::FxQuadPlugin, shield::ShieldPlugin, fire::FirePlugin)).add_systems(Startup, warm_up).add_systems(Update, end_warm_up);
    }
}

/// An effect pipeline's shader loads and compiles the first time something uses it;
/// an explosion that was the first would show a frame or two late, at a moment that
/// varies from run to run. These specks, drawn for a few frames at the start, take
/// that cost before any effect is due.
#[derive(Component)]
struct WarmUp(u8);

fn warm_up(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut quads: ResMut<Assets<quad::FxQuadMaterial>>,
    mut shields: ResMut<Assets<shield::ShieldMaterial>>,
    mut fires: ResMut<Assets<fire::FireMaterial>>,
) {
    let speck = Transform::from_scale(Vec3::splat(0.01));
    let rect = meshes.add(quad::rectangle());
    let ball = meshes.add(Sphere::new(1.0));
    commands.spawn((WarmUp(8), Mesh3d(rect), MeshMaterial3d(quads.add(quad::material(&quad::Look::new(quad::K_FLARE, Vec3::ZERO)))), speck));
    commands.spawn((WarmUp(8), Mesh3d(ball.clone()), MeshMaterial3d(shields.add(shield::ShieldMaterial { params: default() })), speck));
    commands.spawn((WarmUp(8), Mesh3d(ball.clone()), MeshMaterial3d(fires.add(fire::FireMaterial { params: default() })), speck));
    let ring = fire::FireParams { look: Vec4::new(0.0, 0.0, 1.0, 1.0), ..default() };
    commands.spawn((WarmUp(8), Mesh3d(ball), MeshMaterial3d(fires.add(fire::FireMaterial { params: ring })), speck));
}

fn end_warm_up(mut commands: Commands, mut q: Query<(Entity, &mut WarmUp)>) {
    for (e, mut w) in &mut q {
        w.0 -= 1;
        if w.0 == 0 {
            commands.entity(e).despawn();
        }
    }
}

/// Parse and validate a shader of this directory as Bevy would load it.
#[cfg(test)]
pub(crate) fn validate_wgsl(src: &str) {
    let src = src.replace("constants::MATERIAL_BIND_GROUP", "3");
    let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}"));
}

#[cfg(test)]
mod tests {
    /// Each shader is embedded at the path its material names.
    #[test]
    fn shader_paths_match_embedded_registration() {
        for name in ["quad.wesl", "shield.wesl", "fire.wesl"] {
            let registered = match name {
                "quad.wesl" => bevy::asset::embedded_path!("src", "quad.wesl"),
                "shield.wesl" => bevy::asset::embedded_path!("src", "shield.wesl"),
                _ => bevy::asset::embedded_path!("src", "fire.wesl"),
            };
            assert!(registered.to_string_lossy().replace('\\', "/").ends_with(&format!("render/fx/{name}")), "{registered:?}");
        }
    }
}
