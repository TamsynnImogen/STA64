//! Shared STA64 native client.
//!
//! Usage: `sta64 [--install-root <path>] [--mod <name>] [--view <model> | --map [<map>] | --skirmish [<map>] | --mission-report]`;
//! `sta64 --help` lists every flag, including the test bench's.
//!
//! The install root is a user-owned Armada II / Fleet Operations install. No
//! original game content ships with STA64; it is read from that install at run
//! time. With no mode flag the front end opens (menus, then Instant Action and the match); `--view`, `--map`, `--skirmish` and the bench flags go straight to their mode.

pub mod app;
pub mod bench;
pub mod clock;
pub mod game;
pub mod plugins;
pub mod sim_link;

mod platform;

mod audio;
mod capture;
mod frontend;
mod input;
mod map;
mod media;
mod render;
mod ui;
mod viewer;

use app::LaunchConfig;
use bevy::prelude::*;

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

/// Print which mission script every map would run (docs/MISSIONS.md).
fn mission_report(vfs: &sta_vfs::Vfs) {
    use sta_mission::Resolution;
    let labels = sta_mission::Labels::load(vfs);
    let mut maps: Vec<String> = vfs.list("bzn").into_iter().filter(|p| p.ends_with(".bzn") || p.ends_with(".sol")).collect();
    maps.sort();
    let (mut lua, mut legacy, mut none) = (0, 0, 0);
    for p in &maps {
        let Some(Ok(d)) = vfs.read(p) else { continue };
        let map = match sta_bzn::parse(&d) {
            Ok(m) => m,
            Err(e) => {
                println!("{p:40} unreadable: {e}");
                continue;
            }
        };
        let name = p.trim_start_matches("bzn/");
        match sta_mission::resolve(vfs, &labels, p, &map) {
            Resolution::Lua { path, matched_by } => {
                lua += 1;
                println!("{name:40} lua      {path}  (by {matched_by:?})");
            }
            Resolution::LegacyDll { files } => {
                legacy += 1;
                println!("{name:40} NEEDS LUA CONVERSION: {}", files.join(", "));
            }
            Resolution::None => {
                none += 1;
                println!("{name:40} -");
            }
        }
    }
    println!("\n{} maps: {lua} with a Lua mission, {legacy} with only legacy DLL scripts (need Lua conversion), {none} without a script", maps.len());
}

/// Launch the shared client with an explicit platform configuration.
pub fn run(cfg: LaunchConfig) {
    if cfg.help {
        print!("{}", app::USAGE);
        return;
    }
    for a in &cfg.unknown {
        eprintln!("ignoring unknown argument {a:?}");
    }
    for flag in cfg.unsupported() {
        eprintln!("unsupported: {flag}");
    }
    // Bench runs that need no install, window or renderer (docs/TESTBENCH.md).
    if cfg.sim_only {
        std::process::exit(bench::run_sim_only(&cfg));
    }
    // The front end's scene on its own (UI-06): `--headless` with `STA64_MENU_SCENE=1`.
    if frontend::scene::requested(&cfg) {
        std::process::exit(frontend::scene::run_headless(&cfg));
    }
    if cfg.headless && cfg.scene.is_some() {
        std::process::exit(bench::run_headless(&cfg));
    }
    let (vfs, summary) = mount(&cfg);
    println!("{summary}");
    if cfg.mission_report {
        mission_report(&vfs);
        return;
    }
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
        .insert_resource(render::sprites::SpriteTables(std::sync::Arc::new(table)))
        .insert_resource(teams);
    if cfg.mode() == app::Mode::FrontEnd {
        // INT-M4: no mode flag, so the front end; a match is built from its setup screen.
        let rts = sta_content::layered::cfg_h(&vfs, "RTS_CFG.h");
        let mut catalogue = map::Catalogue::default();
        let mut screens = frontend::screens::ScreensConfig::default();
        let maps = match game::flow::front_end_data(&vfs, &mut catalogue, cfg.install_root.as_deref()) {
            Ok(d) => {
                // The menu scene has the camera.
                screens.own_camera = false;
                let maps = game::flow::browser_for(&d.screens, browser.mount_summary.clone());
                app.insert_resource(d.screens)
                    .insert_resource(game::flow::MenuSceneSource(d.scene.clone()))
                    .insert_resource(d.scene)
                    .insert_resource(game::flow::MenuMusic(d.menu_music));
                maps
            }
            Err(e) => {
                eprintln!("front end: no content ({e}); the screens show nothing to play");
                let mut b = map::browser(&vfs, None, browser.mount_summary.clone());
                b.dirty = false;
                b
            }
        };
        app.insert_resource(screens)
            .insert_resource(catalogue)
            .insert_resource(game::flow::FlowConfig { install_root: cfg.install_root.clone() })
            .insert_resource(map::MapSeat(None))
            .insert_resource(map::CameraLimits::from_cfg(rts.as_ref()))
            .insert_resource(viewer::Mounted(vfs))
            .insert_resource(maps);
    } else if let Some(m) = &cfg.map {
        let maps = map::browser(&vfs, Some(m.as_str()).filter(|m| !m.is_empty()), browser.mount_summary.clone());
        println!("{} maps visible", maps.maps.len());
        let rts = sta_content::layered::cfg_h(&vfs, "RTS_CFG.h");
        // A skirmish plays seat 0 unless told otherwise.
        let seat = cfg.seat.or(cfg.skirmish.then_some(0));
        if cfg.skirmish {
            app.insert_resource(game::SkirmishCfg::from_launch(&cfg));
        }
        app.insert_resource(map::MapSeat(seat))
            .insert_resource(map::CameraLimits::from_cfg(rts.as_ref()))
            .insert_resource(viewer::Mounted(vfs))
            .insert_resource(maps);
    } else {
        app.insert_resource(viewer::Mounted(vfs)).insert_resource(browser);
    }
    // Sets, the sim link and the lanes' plugins for the mode.
    plugins::add_plugins(&mut app, &cfg);
    #[cfg(target_os = "android")]
    app.add_plugins(platform::lifecycle::AndroidLifecyclePlugin);
    app.run();
}

#[cfg(target_os = "android")]
#[bevy_main]
fn main() {
    run(platform::android_launch_config());
}
