//! STA64 game executable.
//!
//! Usage: `sta64 [--install-root <path>] [--mod <name>] [--view <model> | --map [<map>]]`
//!
//! The install root is a user-owned Armada II / Fleet Operations install. No
//! original game content ships with STA64; it is read from that install at run
//! time. Until gameplay exists, the executable opens the model viewer.

mod particles;
mod map;
mod model_scene;
mod space;
mod sprites;
mod textures;
mod viewer;

use bevy::prelude::*;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
struct LaunchConfig {
    install_root: Option<PathBuf>,
    active_mod: Option<String>,
    view: Option<String>,
    map: Option<String>,
}

fn parse_args() -> LaunchConfig {
    let mut cfg = LaunchConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--install-root" => cfg.install_root = args.next().map(PathBuf::from),
            "--mod" => cfg.active_mod = args.next(),
            "--view" => cfg.view = args.next(),
            "--map" => cfg.map = Some(args.next().unwrap_or_default()),
            _ => eprintln!("ignoring unknown argument {a:?}"),
        }
    }
    // Default: the directory the executable lives in, like the original game.
    if cfg.install_root.is_none() {
        cfg.install_root = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
    }
    cfg
}

fn mount(cfg: &LaunchConfig) -> (sta_vfs::Vfs, String) {
    let Some(root) = &cfg.install_root else {
        return (sta_vfs::Vfs::default(), "No install root".into());
    };
    match sta_vfs::Vfs::mount_install(root, cfg.active_mod.as_deref()) {
        Ok(vfs) => {
            let chain: Vec<_> = vfs.layers().iter().map(|l| l.name()).collect();
            let archives = vfs.layers().iter().map(|l| l.archives.len()).sum::<usize>();
            let summary = format!(
                "Install: {}\nMounted: {}  ({} FPQ archives)",
                root.display(),
                chain.join(" -> "),
                archives
            );
            (vfs, summary)
        }
        Err(e) => (
            sta_vfs::Vfs::default(),
            format!("Install: {}\nNot mounted: {e}", root.display()),
        ),
    }
}

fn main() {
    let cfg = parse_args();
    let (vfs, summary) = mount(&cfg);
    println!("{summary}");
    let mut browser = viewer::browser(&vfs, cfg.view.as_deref(), summary);
    let teams = viewer::TeamColours::load(&vfs);
    browser.team = teams.default_index();
    println!("{} team colours", teams.0.len());
    println!("{} models visible", browser.models.len());
    let table = sta_assets::load_sprite_table(&vfs);
    println!(
        "sprite tables: {} sprites, {} sprite nodes, {} animations",
        table.sprites.len(),
        table.sprite_nodes.len(),
        table.animations.len()
    );

    let mut app = App::new();
    app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "STA64".into(),
                        // Wayland app_id / X11 WM_CLASS: lets the desktop match
                        // sta64.desktop and show the STA64 icon.
                        name: Some("sta64".into()),
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_linear()),
        )
        .insert_resource(sprites::SpriteTables(std::sync::Arc::new(table)))
        .insert_resource(teams);
    if let Some(m) = &cfg.map {
        let maps = map::browser(&vfs, Some(m.as_str()).filter(|m| !m.is_empty()), browser.mount_summary.clone());
        println!("{} maps visible", maps.maps.len());
        app.insert_resource(viewer::Mounted(vfs)).insert_resource(maps).add_plugins(map::MapPlugin);
    } else {
        app.insert_resource(viewer::Mounted(vfs)).insert_resource(browser).add_plugins(viewer::ViewerPlugin);
    }
    app.run();
}
