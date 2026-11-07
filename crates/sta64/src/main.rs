//! STA64 game executable.
//!
//! Usage: `sta64 [--install-root <path>] [--mod <name>]`
//!
//! The install root is a user-owned Armada II / Fleet Operations install. No
//! original game content ships with STA64; it is read from that install at run
//! time.

use bevy::prelude::*;
use std::path::PathBuf;

#[derive(Resource, Debug, Clone)]
struct LaunchConfig {
    install_root: Option<PathBuf>,
    active_mod: Option<String>,
}

#[derive(Resource)]
struct MountSummary(String);

fn parse_args() -> LaunchConfig {
    let mut cfg = LaunchConfig {
        install_root: None,
        active_mod: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--install-root" => cfg.install_root = args.next().map(PathBuf::from),
            "--mod" => cfg.active_mod = args.next(),
            _ => eprintln!("ignoring unknown argument {a:?}"),
        }
    }
    // Default: the directory the executable lives in, like the original game.
    if cfg.install_root.is_none() {
        cfg.install_root = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(PathBuf::from));
    }
    cfg
}

fn mount(cfg: &LaunchConfig) -> String {
    let Some(root) = &cfg.install_root else {
        return "No install root".into();
    };
    match sta_vfs::Vfs::mount_install(root, cfg.active_mod.as_deref()) {
        Ok(vfs) => {
            let chain: Vec<_> = vfs.layers().iter().map(|l| l.name()).collect();
            let odfs = vfs.list("odf").iter().filter(|p| p.ends_with(".odf")).count();
            format!(
                "Install: {}\nMounted: {}\n{} ODF files visible",
                root.display(),
                chain.join(" -> "),
                odfs
            )
        }
        Err(e) => format!("Install: {}\nNot mounted: {e}", root.display()),
    }
}

fn setup(mut commands: Commands, summary: Res<MountSummary>) {
    commands.spawn(Camera2d);
    commands.spawn((
        Text::new(format!("STA64 {}\n\n{}", env!("CARGO_PKG_VERSION"), summary.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(24.0),
            left: Val::Px(24.0),
            ..default()
        },
    ));
}

fn main() {
    let cfg = parse_args();
    let summary = mount(&cfg);
    println!("{summary}");

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "STA64".into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(cfg)
        .insert_resource(MountSummary(summary))
        .add_systems(Startup, setup)
        .run();
}
