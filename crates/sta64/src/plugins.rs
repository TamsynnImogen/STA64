//! Which plugins run in which mode (docs/WORKPLAN.md S10).
//!
//! Append-only: each package adds one `app.add_plugins(...)` line at the end
//! of its lane's block. On a rebase conflict, keep both lines. Render, UI and
//! audio plugins only run when the mode has a window; `--sim-only` adds none
//! of them.
//!
//! The map view and the model viewer are added here by mode; `main.rs` inserts
//! the resources they read first.

use bevy::prelude::*;

use crate::app::{LaunchConfig, configure_sets};
use crate::clock::AnimClock;
use crate::input::InputPlugin;
use crate::map::MapPlugin;
use crate::render::units::UnitsRenderPlugin;
use crate::sim_link::SimLinkPlugin;
use crate::viewer::ViewerPlugin;

/// Add every lane's plugins for the launch mode, and order the sets.
pub fn add_plugins(app: &mut App, cfg: &LaunchConfig) {
    configure_sets(app);
    // Steps the sim once something inserts a `SimLink`; idle until then.
    app.insert_resource(AnimClock::new(cfg.clock_mode(), 0)).add_plugins(SimLinkPlugin);

    // The map view or the model viewer, chosen by the flags as before the bench
    // flags existed (they do nothing yet): `--map --headless` still opens the map.
    // The front end (INT-M4) runs the map view behind its loading screen: a match
    // is a skirmish the setup screen asked for.
    let front_end = cfg.mode() == crate::app::Mode::FrontEnd;
    if front_end {
        app.add_plugins(crate::game::flow::FlowPlugin);
    }
    let map_view = cfg.map.is_some() || front_end;
    if map_view {
        app.add_plugins(MapPlugin);
        // The map's ships; the map view is always windowed, so not gated below.
        app.add_plugins(UnitsRenderPlugin);
        app.add_plugins(crate::render::overlays::OverlaysPlugin);
        app.add_plugins(crate::render::fx::FxMaterialsPlugin);
        app.add_plugins(crate::render::fx::weapons::WeaponFxPlugin);
        app.add_plugins(crate::render::fx::specials::SpecialsFxPlugin);
        app.add_plugins(crate::render::fx::impacts::ImpactFxPlugin);
        // Inert until a `FogBounds` and a `LocalPlayer` exist.
        app.add_plugins(crate::render::fog::FogPlugin);
    } else if cfg.ui_gallery {
        app.add_plugins(crate::ui::theme::GalleryPlugin);
    } else {
        app.add_plugins(ViewerPlugin);
    }

    // input — one line per package, append inside your lane's block
    if map_view {
        app.add_plugins(InputPlugin);
    }

    // bench —
    // TOOL-14 playtest capture (Ctrl+F12 in a match)
    if map_view {
        app.add_plugins(crate::capture::CapturePlugin);
    }

    // game — the match flow (INT-M3)
    if cfg.skirmish || front_end {
        app.add_plugins(crate::game::GamePlugin);
    }

    if cfg.mode().has_window() {
        // render —
        app.add_plugins(crate::render::hulls::HullPlugin);
        app.add_plugins(crate::render::post::PostFxPlugin::from_cfg(cfg));
        // RND-07 cull distances and marks, RND-22 render profile, RND-28 developer shader shadowing
        app.add_plugins((crate::render::cull::CullPlugin, crate::render::profile::ProfilePlugin, crate::render::shader_shadow::ShaderShadowPlugin));

        // ui —
        app.add_plugins(crate::ui::theme::ThemePlugin);
        app.insert_resource(crate::ui::profile::UiPreference { lcars: cfg.lcars_ui })
            .add_plugins(crate::ui::profile::UiProfilePlugin);
        app.add_plugins(crate::ui::notices::NoticePlugin);
        if front_end {
            // INT-M4: the front end's screens over the menu scene, the loading screen.
            app.add_plugins(crate::frontend::scene::MenuScenePlugin);
            app.add_plugins(crate::frontend::screens::ScreensPlugin);
            app.world_mut().resource_mut::<crate::frontend::screens::Settings>().lcars_ui = cfg.lcars_ui;
            app.add_plugins(crate::frontend::loading::LoadingPlugin);
        }
        if cfg.skirmish || front_end {
            // INT-M3: the HUD (with the radar) for the local seat's race (`game::apply_race`
            // sets its layout), and the match's result.
            app.add_plugins(crate::ui::hud::HudPlugin);
        }

        // audio —
        app.add_plugins(crate::audio::AudioPlugin { mode: cfg.mode() });
        // CON-10: real sounds in place of the stub resolver.
        if let Some(r) = cfg.install_root.as_deref().and_then(|root| crate::audio::BankResolver::mount(root, cfg.active_mod.as_deref())) {
            app.insert_resource(crate::audio::Resolver(Box::new(r)));
        }
        // MED-03 movie playback; MED-05 the intro movies before the front end.
        app.add_plugins(crate::media::MediaPlugin);
        if front_end {
            app.insert_resource(crate::media::IntroSettings { enabled: !cfg.nointro }).add_plugins(crate::media::IntroPlugin);
        }
        if cfg.skirmish {
            // A match: calm and battle music of the local race (set by `game::apply_race`),
            // not the viewer's silence.
            app.insert_resource(crate::audio::MusicContext::Game);
        }
    }
}
