//! Developer-only shader shadowing (RND-28, MODDING_DECISIONS D12).
//!
//! **Unstable. Not a mod contract.** A shader author can replace one of the
//! renderer's embedded `.wesl` files by dropping a file of the same path into a
//! mounted layer (a mod folder), so a shader can be tried without a rebuild. The
//! shaders' bindings, uniform layouts and names change with the renderer and are
//! not versioned; a shadow written for one build may not compile on the next. A real
//! shader slot is built when a second author asks for one (D12), and none of this
//! is a shim or a compatibility promise.
//!
//! Off unless `STA64_DEV_SHADER_SHADOW=1` is set, so a shipped game never reads
//! these files and stock output is unchanged. With it on, each time the mounted
//! files change the layers are searched for `shaders/<embedded path>` (for
//! example `shaders/render/fx/quad.wesl`); a hit is written over the embedded
//! source and the shader, if it was already loaded, is reloaded. The shader log
//! names every file shadowed, at `warn`.
//!
//! The paths are those the materials ask for, `embedded://sta64/<path>`: the list
//! below must name every `embedded_asset!` of a `.wesl`.

use std::path::{Path, PathBuf};

use bevy::asset::io::embedded::EmbeddedAssetRegistry;
use bevy::prelude::*;

use crate::viewer::Mounted;

/// The switch: set to `1` to let mounted files shadow embedded shaders.
pub const ENV: &str = "STA64_DEV_SHADER_SHADOW";

/// Where in the mounted files a shadow lives, before the embedded path.
pub const VFS_DIR: &str = "shaders";

/// Every embedded `.wesl`, by the path under `embedded://sta64/`.
pub const EMBEDDED: &[&str] = &[
    "render/fog.wesl",
    "render/hulls.wesl",
    "render/nebula.wesl",
    "render/planet.wesl",
    "render/rock.wesl",
    "render/fx/fire.wesl",
    "render/fx/quad.wesl",
    "render/fx/shield.wesl",
];

/// Whether the environment switches shadowing on.
pub fn enabled() -> bool {
    std::env::var(ENV).as_deref() == Ok("1")
}

/// The shadows the mounted files hold: `(embedded path, source)`, in [`EMBEDDED`] order.
pub fn shadows(vfs: &sta_vfs::Vfs) -> Vec<(&'static str, Vec<u8>)> {
    EMBEDDED
        .iter()
        .filter_map(|path| Some((*path, vfs.read(&format!("{VFS_DIR}/{path}"))?.ok()?)))
        .collect()
}

pub struct ShaderShadowPlugin;

impl Plugin for ShaderShadowPlugin {
    fn build(&self, app: &mut App) {
        if enabled() {
            app.add_systems(Update, apply.run_if(resource_exists_and_changed::<Mounted>));
        }
    }
}

fn apply(vfs: Res<Mounted>, registry: Option<Res<EmbeddedAssetRegistry>>, server: Option<Res<AssetServer>>) {
    let Some(registry) = registry else { return };
    for (path, source) in shadows(&vfs.0) {
        warn!("shader shadow (developer option, unstable): {VFS_DIR}/{path} replaces embedded://sta64/{path}");
        let asset = format!("sta64/{path}");
        registry.insert_asset(PathBuf::new(), Path::new(&asset), source);
        if let Some(server) = &server {
            // Already loaded under the embedded source: read it again. A shader not yet
            // loaded simply finds the shadow when it is first asked for.
            let _ = server.reload(format!("embedded://{asset}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(files: &[(&str, &str)]) -> sta_vfs::Vfs {
        let dir = std::env::temp_dir().join(format!("sta64-shadow-{}-{}", std::process::id(), files.len()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut vfs = sta_vfs::Vfs::default();
        vfs.push_dir("mod".into(), dir).unwrap();
        vfs
    }

    #[test]
    fn only_listed_embedded_paths_are_shadowed() {
        let vfs = mount(&[("shaders/render/fx/quad.wesl", "// mine"), ("shaders/render/other.wesl", "// not embedded")]);
        let found = shadows(&vfs);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "render/fx/quad.wesl");
        assert_eq!(found[0].1, b"// mine");
        assert!(shadows(&sta_vfs::Vfs::default()).is_empty());
    }

    #[test]
    fn every_embedded_wesl_is_listed() {
        // The list is the set of `embedded_asset!` calls of a `.wesl`.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/render");
        let mut found = Vec::new();
        let mut dirs = vec![src.clone()];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p.extension().is_some_and(|x| x == "wesl") {
                    found.push(p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        found.sort();
        let mut listed: Vec<String> = EMBEDDED.iter().map(|p| p.trim_start_matches("render/").to_owned()).collect();
        listed.sort();
        assert_eq!(found, listed);
    }
}
