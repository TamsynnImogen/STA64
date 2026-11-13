//! The few launch, storage and lifecycle boundaries that differ by platform.

use bevy::prelude::*;
use std::path::PathBuf;

#[cfg(not(target_os = "android"))]
pub fn default_install_root() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
}

#[cfg(target_os = "android")]
fn private_data_dir() -> Option<PathBuf> {
    bevy::android::ANDROID_APP
        .get()
        .and_then(|app| app.internal_data_path())
}

#[cfg(target_os = "android")]
pub fn default_install_root() -> Option<PathBuf> {
    private_data_dir().map(|dir| dir.join("content"))
}

#[cfg(target_os = "android")]
pub fn android_launch_config() -> crate::app::LaunchConfig {
    crate::app::LaunchConfig {
        install_root: default_install_root(),
        ..default()
    }
}

pub fn surface_cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    let base = private_data_dir().map(|dir| dir.join("cache"))?;
    #[cfg(not(target_os = "android"))]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    Some(base.join("sta64").join("surfaces"))
}

/// Which platform's conventions [`config_dir_in`] follows (a parameter so every
/// platform's path is tested on any host).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOs {
    Linux,
    Windows,
    MacOs,
}

impl ConfigOs {
    pub const HOST: ConfigOs = if cfg!(target_os = "windows") {
        ConfigOs::Windows
    } else if cfg!(target_os = "macos") {
        ConfigOs::MacOs
    } else {
        ConfigOs::Linux
    };
}

/// The folder of STA64's own settings file, from the environment `var` reads:
/// `$STA64_CONFIG_DIR` (the folder itself), else `$XDG_CONFIG_HOME/sta64`, then
/// `%APPDATA%\sta64` on Windows, `~/Library/Application Support/sta64` on macOS and
/// `~/.config/sta64` elsewhere (the capture folder's convention in `sta_bench`,
/// with the config rather than the data base). `None` when no base is known: the
/// caller then does not persist, rather than writing into the working directory.
pub fn config_dir_in(os: ConfigOs, var: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let var = |k: &str| var(k).filter(|v| !v.as_os_str().is_empty());
    if let Some(dir) = var("STA64_CONFIG_DIR") {
        return Some(dir);
    }
    let home = || var("HOME").or_else(|| var("USERPROFILE"));
    let base = var("XDG_CONFIG_HOME")
        .or_else(|| if os == ConfigOs::Windows { var("APPDATA") } else { None })
        .or_else(|| {
            home().map(|h| match os {
                ConfigOs::MacOs => h.join("Library/Application Support"),
                ConfigOs::Windows => h.join("AppData/Roaming"),
                ConfigOs::Linux => h.join(".config"),
            })
        })?;
    Some(base.join("sta64"))
}

/// [`config_dir_in`] for this host and environment (the app's private files on Android).
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    return private_data_dir().map(|dir| dir.join("config"));
    #[cfg(not(target_os = "android"))]
    config_dir_in(ConfigOs::HOST, |k| std::env::var_os(k).map(PathBuf::from))
}

/// The folder of STA64's own user data (derived content such as converted missions):
/// `$STA64_DATA_DIR` (the folder itself), else `$XDG_DATA_HOME/sta64`, `%LOCALAPPDATA%\sta64`,
/// `~/Library/Application Support/sta64` or `~/.local/share/sta64` (the capture folder's
/// convention in `sta_bench`). `None` when no base is known. Never inside an install.
pub fn data_dir_in(os: ConfigOs, var: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let var = |k: &str| var(k).filter(|v| !v.as_os_str().is_empty());
    if let Some(dir) = var("STA64_DATA_DIR") {
        return Some(dir);
    }
    let base = var("XDG_DATA_HOME")
        .or_else(|| if os == ConfigOs::Windows { var("LOCALAPPDATA") } else { None })
        .or_else(|| {
            var("HOME").or_else(|| var("USERPROFILE")).map(|h| match os {
                ConfigOs::MacOs => h.join("Library/Application Support"),
                ConfigOs::Windows => h.join("AppData/Local"),
                ConfigOs::Linux => h.join(".local/share"),
            })
        })?;
    Some(base.join("sta64"))
}

/// [`data_dir_in`] for this host and environment (none on Android: no converter runs there).
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    return None;
    #[cfg(not(target_os = "android"))]
    data_dir_in(ConfigOs::HOST, |k| std::env::var_os(k).map(PathBuf::from))
}

/// The folder the user's converted missions for this install and mod live in
/// (`sta_mission::converted`), whether or not it exists yet.
pub fn converted_missions_dir(install_root: &std::path::Path, active_mod: Option<&str>) -> Option<PathBuf> {
    data_dir().map(|d| sta_mission::converted::dir_in(&d, install_root, active_mod))
}

/// Absent on desktop, so ordinary desktop scheduling is unchanged.
#[derive(Resource)]
pub struct LifecycleGate {
    active: bool,
    skip_resume_frame: bool,
}

impl LifecycleGate {
    pub fn is_foreground(&self) -> bool {
        self.active && !self.skip_resume_frame
    }
}

pub fn foreground(gate: Option<Res<LifecycleGate>>) -> bool {
    gate.is_none_or(|gate| gate.is_foreground())
}

#[cfg(any(target_os = "android", test))]
pub mod lifecycle {
    use super::*;
    use crate::app::GameSet;
    use bevy::input::{
        InputSystems,
        mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseMotion, MouseWheel},
    };
    use bevy::ui::UiSystems;
    use bevy::window::AppLifecycle;

    pub struct AndroidLifecyclePlugin;

    impl Plugin for AndroidLifecyclePlugin {
        fn build(&self, app: &mut App) {
            // WindowPlugin normally registers this; also supports headless tests.
            app.add_message::<AppLifecycle>()
                .insert_resource(LifecycleGate {
                    active: true,
                    skip_resume_frame: false,
                })
                .configure_sets(
                    Update,
                    (
                        GameSet::Input,
                        GameSet::SimLink,
                        GameSet::Interpolate,
                        GameSet::Render,
                        GameSet::Ui,
                        GameSet::Audio,
                    )
                        .run_if(foreground),
                )
                .add_systems(First, lifecycle)
                .add_systems(
                    PreUpdate,
                    clear_pending_input
                        .after(InputSystems)
                        .after(UiSystems::Focus)
                        .run_if(not(foreground)),
                )
                .add_systems(
                    Last,
                    (clear_pending_widgets.run_if(not(foreground)), finish_frame).chain(),
                );
        }
    }

    fn lifecycle(mut messages: MessageReader<AppLifecycle>, mut gate: ResMut<LifecycleGate>) {
        for message in messages.read() {
            match message {
                AppLifecycle::Idle | AppLifecycle::WillSuspend | AppLifecycle::Suspended => {
                    gate.active = false
                }
                AppLifecycle::WillResume => {
                    gate.active = true;
                    gate.skip_resume_frame = true;
                }
                AppLifecycle::Running => {
                    if !gate.active {
                        gate.skip_resume_frame = true;
                    }
                    gate.active = true;
                }
            }
        }
    }

    fn finish_frame(mut gate: ResMut<LifecycleGate>) {
        gate.skip_resume_frame = false;
    }

    fn clear_pending_widgets(world: &mut World) {
        use crate::ui::theme::{
            ButtonClicked, ListSelected, SliderChanged, ToggleChanged, WidgetState,
        };
        if let Some(mut pointers) = world.get_resource_mut::<bevy::picking::events::PointerState>()
        {
            pointers.pointer_buttons.clear();
        }
        let pressed: Vec<_> = world
            .query_filtered::<Entity, With<bevy::ui::Pressed>>()
            .iter(world)
            .collect();
        for entity in pressed {
            world.entity_mut(entity).remove::<bevy::ui::Pressed>();
        }
        // Pointer observers run outside GameSet::Ui. Discard pending widget
        // events after picking and deferred observers have finished this frame.
        if let Some(mut messages) = world.get_resource_mut::<Messages<ButtonClicked>>() {
            messages.clear();
        }
        if let Some(mut messages) = world.get_resource_mut::<Messages<ToggleChanged>>() {
            messages.clear();
        }
        if let Some(mut messages) = world.get_resource_mut::<Messages<SliderChanged>>() {
            messages.clear();
        }
        if let Some(mut messages) = world.get_resource_mut::<Messages<ListSelected>>() {
            messages.clear();
        }
        for mut state in world.query::<&mut WidgetState>().iter_mut(world) {
            if *state != WidgetState::Disabled {
                *state = WidgetState::Normal;
            }
        }
    }

    // This touches Bevy data only, never OS input devices. Commands that have
    // already entered CommandQueue and the player's game-pause state survive.
    #[allow(clippy::too_many_arguments)]
    fn clear_pending_input(
        keys: Option<ResMut<ButtonInput<KeyCode>>>,
        mouse: Option<ResMut<ButtonInput<MouseButton>>>,
        touches: Option<ResMut<Touches>>,
        motion: Option<ResMut<AccumulatedMouseMotion>>,
        scroll: Option<ResMut<AccumulatedMouseScroll>>,
        motion_messages: Option<ResMut<Messages<MouseMotion>>>,
        wheel_messages: Option<ResMut<Messages<MouseWheel>>>,
        drag: Option<ResMut<crate::input::select::Drag>>,
        aim: Option<ResMut<crate::input::orders::Aim>>,
        hud: Option<ResMut<crate::ui::hud::HudState>>,
        mut boxes: Query<&mut Visibility, With<crate::input::select::SelectBox>>,
    ) {
        if let Some(mut keys) = keys {
            keys.reset_all();
        }
        if let Some(mut mouse) = mouse {
            mouse.reset_all();
        }
        if let Some(mut touches) = touches {
            touches.reset_all();
        }
        if let Some(mut motion) = motion {
            motion.delta = Vec2::ZERO;
        }
        if let Some(mut scroll) = scroll {
            scroll.delta = Vec2::ZERO;
        }
        if let Some(mut messages) = motion_messages {
            messages.clear();
        }
        if let Some(mut messages) = wheel_messages {
            messages.clear();
        }
        if let Some(mut drag) = drag {
            *drag = default();
        }
        if let Some(mut aim) = aim {
            aim.0 = None;
        }
        if let Some(mut hud) = hud {
            hud.placing = None;
            hud.targeting = None;
        }
        for mut visibility in &mut boxes {
            *visibility = Visibility::Hidden;
        }
    }
}

#[cfg(test)]
mod tests;
