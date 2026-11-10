//! The HUD in the bench's headless frames (`--headless --frames`), for the
//! `ui-hud-layout` bench: an environment switch, not a flag (`app.rs` is the
//! INT lane's; INT-M3 wires the real mode).
//!
//! - `STA64_UI_HUD=1`: draw the HUD (and the radar and notices it hosts) over
//!   the frame, fed from the bench's view as player 1 on the bench's fake
//!   content.
//! - `STA64_UI_HUD_SELECT=a,b`: scene object names to select.
//! - `STA64_UI_HUD_PAGE=build`: open the grid's build page.
//! - `STA64_UI_HUD_CFG=path`: a GUI cfg (CON-12) to lay the HUD out from;
//!   `#include`s are read from its directory.
//!
//! The layout is printed once per change as a `hud layout:` line.

use std::path::Path;

use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use sta_content::{GuiCfg, GuiLayout};

use super::panels::HudRoot;
use super::{ExternalFeed, HudConfig, HudPlugin};
use crate::app::GameSet;
use crate::ui::notices::NoticeRoot;
use crate::ui::radar::{FrameUiCamera, add_frame_overlay};

pub fn requested() -> bool {
    std::env::var("STA64_UI_HUD").as_deref() == Ok("1")
}

/// Scene object names to select.
pub fn select_names() -> Vec<String> {
    std::env::var("STA64_UI_HUD_SELECT").map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()).unwrap_or_default()
}

/// Whether the build page is open from the start.
pub fn open_build() -> bool {
    std::env::var("STA64_UI_HUD_PAGE").as_deref() == Ok("build")
}

/// The cfg `STA64_UI_HUD_CFG` names, resolved.
pub fn cfg_from_env() -> Option<GuiLayout> {
    let path = std::env::var_os("STA64_UI_HUD_CFG")?;
    let path = Path::new(&path);
    let dir = path.parent()?.to_owned();
    let name = path.file_name()?.to_string_lossy().into_owned();
    let (cfg, warnings) = GuiCfg::load(&name, &mut |n| std::fs::read_to_string(dir.join(n)).ok());
    for w in warnings {
        eprintln!("hud cfg: {w}");
    }
    Some(cfg.layout_auto())
}

/// Add the HUD to a frame run: the radar's camera and scale (UI-05), the HUD's
/// plugin, and every HUD root aimed at that camera. The caller feeds
/// [`HudFeed`](super::HudFeed) and `RadarFeed`.
pub fn add_overlay(app: &mut App, target: Handle<Image>, size: (u32, u32)) {
    add_frame_overlay(app, target, size.1);
    app.insert_resource(HudConfig { gui: cfg_from_env(), aspect: size.0 as f32 / size.1 as f32, fit_scale: false, log: true })
        .insert_resource(ExternalFeed)
        .add_plugins(HudPlugin)
        .add_systems(Update, aim_at_camera.before(GameSet::Ui));
}

#[allow(clippy::type_complexity)]
fn aim_at_camera(
    mut commands: Commands,
    cam: Option<Res<FrameUiCamera>>,
    roots: Query<Entity, (Or<(With<HudRoot>, With<NoticeRoot>)>, Without<UiTargetCamera>)>,
) {
    let Some(cam) = cam else { return };
    for e in &roots {
        commands.entity(e).insert(UiTargetCamera(cam.0));
    }
}
