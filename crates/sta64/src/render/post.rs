//! Opt-in post-processing (Refit's `postfx`).
//!
//! Refit's effects are not ported as shaders: this uses Bevy 0.20's built-in
//! ones, which cover the same ground: bloom, tonemapping choice, FXAA/SMAA,
//! auto-exposure, vignette and chromatic aberration. Film grain is not in
//! Bevy 0.20 and is left out.
//!
//! Everything is off by default, so the stock look is unchanged: with the
//! default [`PostFx`] no effect component is added to any camera. [`apply`]
//! adds and removes exactly the components of each switched effect on every
//! 3D camera except the map's background camera. Nothing here reads or writes
//! the `AnimClock` or the sim.
//!
//! Enable with `--postfx bloom,smaa` (names: `bloom`, `smaa`, `fxaa`,
//! `vignette`, `chromatic`, `exposure`, `aces`, `reinhard`, or `cinematic` for
//! a preset), or cycle the presets with F8.

use bevy::anti_alias::fxaa::Fxaa;
use bevy::anti_alias::smaa::Smaa;
use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::auto_exposure::{AutoExposure, AutoExposurePlugin};
use bevy::post_process::bloom::Bloom;
use bevy::post_process::effect_stack::{ChromaticAberration, Vignette};
use bevy::prelude::*;

use crate::map::BackgroundCamera;

/// Which anti-aliasing to run in place of MSAA.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Aa {
    #[default]
    Off,
    Fxaa,
    Smaa,
}

/// The post FX settings; every effect off by default.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct PostFx {
    pub bloom: bool,
    pub bloom_intensity: f32,
    /// `Some`: this tonemapper instead of Bevy's default.
    pub tonemapping: Option<Tonemapping>,
    pub aa: Aa,
    pub auto_exposure: bool,
    pub vignette: bool,
    pub vignette_intensity: f32,
    pub chromatic: bool,
    pub chromatic_intensity: f32,
}

impl Default for PostFx {
    fn default() -> Self {
        PostFx {
            bloom: false,
            bloom_intensity: 0.15,
            tonemapping: None,
            aa: Aa::Off,
            auto_exposure: false,
            vignette: false,
            vignette_intensity: 1.0,
            chromatic: false,
            chromatic_intensity: 0.02,
        }
    }
}

impl PostFx {
    /// Bloom, SMAA, ACES and a light vignette.
    pub fn cinematic() -> PostFx {
        PostFx { bloom: true, aa: Aa::Smaa, tonemapping: Some(Tonemapping::AcesFitted), vignette: true, vignette_intensity: 0.5, ..default() }
    }

    /// Parse `--postfx` names, comma separated; unknown names come back.
    pub fn parse(list: &str) -> (PostFx, Vec<String>) {
        let mut fx = PostFx::default();
        let mut unknown = Vec::new();
        for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            match name {
                "bloom" => fx.bloom = true,
                "smaa" => fx.aa = Aa::Smaa,
                "fxaa" => fx.aa = Aa::Fxaa,
                "vignette" => fx.vignette = true,
                "chromatic" => fx.chromatic = true,
                "exposure" => fx.auto_exposure = true,
                "aces" => fx.tonemapping = Some(Tonemapping::AcesFitted),
                "reinhard" => fx.tonemapping = Some(Tonemapping::ReinhardLuminance),
                "cinematic" => fx = PostFx::cinematic(),
                "off" | "none" => fx = PostFx::default(),
                other => unknown.push(other.to_owned()),
            }
        }
        (fx, unknown)
    }

    fn next_preset(&self) -> PostFx {
        let bloom_only = PostFx { bloom: true, ..default() };
        if *self == PostFx::default() {
            bloom_only
        } else if *self == bloom_only {
            PostFx::cinematic()
        } else {
            PostFx::default()
        }
    }
}

/// Marks a camera whose MSAA this module turned off for FXAA/SMAA.
#[derive(Component)]
pub struct PostFxMsaa;

pub struct PostFxPlugin {
    pub initial: PostFx,
}

impl PostFxPlugin {
    /// From the `--postfx` argument(s).
    pub fn from_cfg(cfg: &crate::app::LaunchConfig) -> PostFxPlugin {
        let (initial, unknown) = PostFx::parse(&cfg.postfx.join(","));
        for u in unknown {
            warn!("--postfx: unknown effect `{u}`");
        }
        PostFxPlugin { initial }
    }
}

impl Plugin for PostFxPlugin {
    fn build(&self, app: &mut App) {
        // Only when there is a renderer (the windowless tests have none).
        if app.get_sub_app(bevy::render::RenderApp).is_some() && !app.is_plugin_added::<AutoExposurePlugin>() {
            app.add_plugins(AutoExposurePlugin);
        }
        app.insert_resource(self.initial.clone()).add_systems(Update, (cycle_preset, apply).chain());
    }
}

/// F8: off, bloom, cinematic, off...
fn cycle_preset(keys: Option<Res<ButtonInput<KeyCode>>>, mut fx: ResMut<PostFx>) {
    if keys.is_some_and(|k| k.just_pressed(KeyCode::F8)) {
        *fx = fx.next_preset();
        info!("post fx: {:?}", *fx);
    }
}

type NotBackground = (With<Camera3d>, Without<BackgroundCamera>);

/// Put the settings on the cameras: when they change, and on new cameras.
///
/// The commands are the `try_` kind: a camera can be despawned by another
/// system's commands in the same frame (the menu camera when a match
/// launches), and a plain `insert` on it would panic.
pub fn apply(
    mut commands: Commands,
    fx: Res<PostFx>,
    cams: Query<Entity, NotBackground>,
    added: Query<Entity, (Added<Camera3d>, Without<BackgroundCamera>)>,
    marked: Query<(), With<PostFxMsaa>>,
    profile: Option<Res<crate::render::profile::RenderProfile>>,
    effects: Option<Res<crate::render::profile::ProfileEffects>>,
) {
    // Stock (RND-24): the original draws with no tonemapper unless one is asked for.
    let stock = matches!((profile.as_deref(), effects.as_deref()), (Some(p), Some(e)) if e.implementation(*p, crate::render::profile::Effect::Post) == crate::render::profile::RenderProfile::Stock);
    let stock_changed = profile.as_ref().is_some_and(|p| p.is_changed()) || effects.as_ref().is_some_and(|e| e.is_changed());
    if !fx.is_changed() && added.is_empty() && !stock_changed {
        return;
    }
    for e in &cams {
        let mut cam = commands.entity(e);
        if fx.bloom {
            cam.try_insert(Bloom { intensity: fx.bloom_intensity, ..Bloom::NATURAL });
        } else {
            cam.try_remove::<Bloom>();
        }
        if fx.auto_exposure {
            cam.try_insert(AutoExposure::default());
        } else {
            cam.try_remove::<AutoExposure>();
        }
        // Bloom and auto-exposure bring `Hdr` along; take it back with them.
        if !fx.bloom && !fx.auto_exposure {
            cam.try_remove::<Hdr>();
        }
        cam.try_insert(fx.tonemapping.unwrap_or(if stock { Tonemapping::None } else { Tonemapping::default() }));
        if fx.vignette {
            cam.try_insert(Vignette { intensity: fx.vignette_intensity, ..default() });
        } else {
            cam.try_remove::<Vignette>();
        }
        if fx.chromatic {
            cam.try_insert(ChromaticAberration { intensity: fx.chromatic_intensity, ..default() });
        } else {
            cam.try_remove::<ChromaticAberration>();
        }
        match fx.aa {
            Aa::Off => {
                cam.try_remove::<(Fxaa, Smaa)>();
            }
            Aa::Fxaa => {
                cam.try_remove::<Smaa>().try_insert(Fxaa::default());
            }
            Aa::Smaa => {
                cam.try_remove::<Fxaa>().try_insert(Smaa::default());
            }
        }
        // FXAA/SMAA replace MSAA; give it back afterwards.
        if fx.aa != Aa::Off {
            if !marked.contains(e) {
                cam.try_insert((PostFxMsaa, Msaa::Off));
            }
        } else if marked.contains(e) {
            cam.try_remove::<PostFxMsaa>().try_insert(Msaa::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(fx: PostFx) -> (App, Entity, Entity) {
        let mut app = App::new();
        // Like the game: a failed command panics instead of logging.
        app.set_error_handler(bevy::ecs::error::panic).add_plugins(PostFxPlugin { initial: fx });
        let main = app.world_mut().spawn(Camera3d::default()).id();
        let bg = app.world_mut().spawn((Camera3d::default(), BackgroundCamera)).id();
        app.update();
        (app, main, bg)
    }

    fn has<C: Component>(app: &App, e: Entity) -> bool {
        app.world().entity(e).contains::<C>()
    }

    #[test]
    fn default_adds_nothing() {
        let (app, main, _) = app(PostFx::default());
        assert!(!has::<Bloom>(&app, main));
        assert!(!has::<Hdr>(&app, main));
        assert!(!has::<Smaa>(&app, main) && !has::<Fxaa>(&app, main));
        assert!(!has::<AutoExposure>(&app, main) && !has::<Vignette>(&app, main) && !has::<ChromaticAberration>(&app, main));
        assert!(!has::<PostFxMsaa>(&app, main));
        assert_eq!(app.world().get::<Tonemapping>(main), Some(&Tonemapping::default()));
    }

    #[test]
    fn toggling_adds_and_removes_exact_components() {
        let (mut app, main, bg) = app(PostFx::default());
        *app.world_mut().resource_mut::<PostFx>() = PostFx { bloom: true, aa: Aa::Smaa, vignette: true, tonemapping: Some(Tonemapping::AcesFitted), ..default() };
        app.update();
        assert!(has::<Bloom>(&app, main) && has::<Hdr>(&app, main));
        assert!(has::<Smaa>(&app, main) && !has::<Fxaa>(&app, main));
        assert!(has::<Vignette>(&app, main) && !has::<ChromaticAberration>(&app, main));
        assert_eq!(app.world().get::<Msaa>(main), Some(&Msaa::Off));
        assert_eq!(app.world().get::<Tonemapping>(main), Some(&Tonemapping::AcesFitted));
        assert!(!has::<Bloom>(&app, bg) && !has::<Smaa>(&app, bg), "the background camera is left alone");

        app.world_mut().resource_mut::<PostFx>().aa = Aa::Fxaa;
        app.update();
        assert!(has::<Fxaa>(&app, main) && !has::<Smaa>(&app, main));

        *app.world_mut().resource_mut::<PostFx>() = PostFx::default();
        app.update();
        assert!(!has::<Bloom>(&app, main) && !has::<Hdr>(&app, main) && !has::<Fxaa>(&app, main) && !has::<Vignette>(&app, main));
        assert!(!has::<PostFxMsaa>(&app, main));
        assert_eq!(app.world().get::<Msaa>(main), Some(&Msaa::default()));
        assert_eq!(app.world().get::<Tonemapping>(main), Some(&Tonemapping::default()));
    }

    #[test]
    fn cameras_spawned_later_get_the_settings() {
        let (mut app, _, _) = app(PostFx { bloom: true, ..default() });
        let late = app.world_mut().spawn(Camera3d::default()).id();
        app.update();
        assert!(has::<Bloom>(&app, late));
    }

    #[test]
    fn camera_despawned_in_the_same_frame_does_not_panic() {
        fn despawn_cams(mut commands: Commands, cams: Query<Entity, NotBackground>) {
            for e in &cams {
                commands.entity(e).despawn();
            }
        }
        let (mut app, main, _) = app(PostFx::cinematic());
        // No sync point between them: both command queues apply at the end
        // of the frame, the despawn first, as when a match launches.
        app.add_systems(Update, despawn_cams.before_ignore_deferred(apply));
        // A changed resource makes `apply` queue commands for every camera.
        app.world_mut().resource_mut::<PostFx>().aa = Aa::Fxaa;
        app.update();
        assert!(app.world().get_entity(main).is_err());
    }

    #[test]
    fn parse_and_presets() {
        let (fx, unknown) = PostFx::parse("bloom, smaa,wat");
        assert!(fx.bloom && fx.aa == Aa::Smaa);
        assert_eq!(unknown, ["wat"]);
        assert_eq!(PostFx::parse("").0, PostFx::default());
        let mut fx = PostFx::default();
        let mut seen = Vec::new();
        for _ in 0..3 {
            fx = fx.next_preset();
            seen.push(fx.clone());
        }
        assert_eq!(seen, [PostFx { bloom: true, ..default() }, PostFx::cinematic(), PostFx::default()]);
    }
}
