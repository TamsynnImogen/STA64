//! Install-gated: load every `misc/gui_*.cfg` through the VFS, includes
//! resolved as the game does (by name under `misc/`, mod layers first).
//!
//! - STA64_TEST_INSTALL (+ STA64_TEST_MOD): a Fleet Operations / Roots install.
//! - STA64_TEST_GUI_DIRS: plain game folders (A1 / A2 retail), each mounted as
//!   one layer; separated by the platform's path separator.

use std::path::{Path, PathBuf};

use sta_content::GuiCfg;
use sta_vfs::Vfs;

fn check(label: &str, vfs: &Vfs, default_canvas: (i32, i32)) -> usize {
    let files: Vec<String> = vfs
        .list("misc")
        .into_iter()
        .filter(|p| {
            let n = p.rsplit('/').next().unwrap();
            n.starts_with("gui_") && n.ends_with(".cfg")
        })
        .collect();
    let (mut panels, mut warns) = (0, Vec::new());
    for path in &files {
        let name = path.rsplit('/').next().unwrap().to_owned();
        let mut load = |n: &str| {
            let p = format!("misc/{}", n.rsplit(['/', '\\']).next()?);
            vfs.read(&p)?.ok().map(|b| String::from_utf8_lossy(&b).into_owned())
        };
        let (cfg, w) = GuiCfg::load(&name, &mut load);
        assert!(!cfg.files().is_empty(), "{path} did not load");
        let layout = cfg.layout(default_canvas);
        assert!(layout.canvas.0 > 0 && layout.canvas.1 > 0, "{path}");
        panels += layout.panels.len();
        warns.extend(w.into_iter().map(|w| w.to_string()));
    }
    eprintln!("{label}: {} gui cfgs, {panels} panels in all, {} warnings", files.len(), warns.len());
    for w in warns.iter().take(8) {
        eprintln!("  {w}");
    }
    files.len()
}

#[test]
fn every_installed_gui_cfg_loads() {
    if let Ok(root) = std::env::var("STA64_TEST_INSTALL") {
        let m = std::env::var("STA64_TEST_MOD").ok();
        let vfs = Vfs::mount_install(Path::new(&root), m.as_deref()).unwrap();
        let n = check(m.as_deref().unwrap_or("Data"), &vfs, (1600, 1200));
        assert!(n > 0, "no gui cfgs found");
    }
    if let Some(dirs) = std::env::var_os("STA64_TEST_GUI_DIRS") {
        for dir in std::env::split_paths(&dirs) {
            let mut vfs = Vfs::default();
            vfs.push_dir("root".into(), PathBuf::from(&dir)).unwrap();
            // Armada I lays out a 640 x 480 canvas; Armada II states its own.
            let n = check(&dir.display().to_string(), &vfs, (640, 480));
            assert!(n > 0, "no gui cfgs under {}", dir.display());
        }
    }
}
