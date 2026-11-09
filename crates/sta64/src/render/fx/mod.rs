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
        app.add_plugins((quad::FxQuadPlugin, shield::ShieldPlugin, fire::FirePlugin));
    }
}

/// Parse and validate a shader of this directory as Bevy would load it.
#[cfg(test)]
pub(crate) fn validate_wgsl(src: &str) {
    let src = src.replace("#{MATERIAL_BIND_GROUP}", "3");
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
        for name in ["quad.wgsl", "shield.wgsl", "fire.wgsl"] {
            let registered = match name {
                "quad.wgsl" => bevy::asset::embedded_path!("src", "quad.wgsl"),
                "shield.wgsl" => bevy::asset::embedded_path!("src", "shield.wgsl"),
                _ => bevy::asset::embedded_path!("src", "fire.wgsl"),
            };
            assert!(registered.to_string_lossy().replace('\\', "/").ends_with(&format!("render/fx/{name}")), "{registered:?}");
        }
    }
}
