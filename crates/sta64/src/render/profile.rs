//! Render profiles (RND-22): which of two looks a scene is drawn in.
//!
//! - [`RenderProfile::Stock`], the default: the effects as the original games
//!   draw them (RND-23..26).
//! - [`RenderProfile::Refit`]: the Refit-derived effects the renderer has had so far
//!   (volumetric nebulae, the hull shader, ordnance, shield, construction and
//!   cloak effects, post FX). They are **kept**: nothing here deletes, renames or
//!   re-owns them, and their `.wesl` files are unchanged.
//!
//! The profile is a resource, so changing it switches a running scene with no restart:
//! [`RenderProfile::toggled`] (F7, developer key) or a setting writing it. The first
//! value comes from `STA64_RENDER_PROFILE=refit|stock`, else Stock. A game's own
//! default (INT-M7 sets it per game) is whatever the front end writes at start.
//!
//! # For RND-23..26
//!
//! An effect has a Refit implementation (every [`Effect`] does, today) and may gain a
//! stock one. A stock package calls [`ProfileEffects::register_stock`] for its effect
//! and gates its systems with [`stock_active`]; the Refit systems are gated with
//! [`refit_active`]. [`ProfileEffects::implementation`] says which one draws:
//! the selected profile's if the effect has one, else Refit's, so choosing Stock
//! never takes away an effect that has no stock version yet. A change sends
//! [`ProfileChanged`] for systems that must rebuild what they spawned.

#![allow(dead_code)] // the stock packages (RND-23..26) are the registry's users; until they land only the plugin runs

use bevy::prelude::*;

/// Which look is drawn.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderProfile {
    #[default]
    Stock,
    Refit,
}

impl RenderProfile {
    pub fn toggled(self) -> RenderProfile {
        match self {
            RenderProfile::Stock => RenderProfile::Refit,
            RenderProfile::Refit => RenderProfile::Stock,
        }
    }

    pub fn parse(name: &str) -> Option<RenderProfile> {
        match name.trim().to_ascii_lowercase().as_str() {
            "stock" => Some(RenderProfile::Stock),
            "refit" => Some(RenderProfile::Refit),
            _ => None,
        }
    }

    /// `STA64_RENDER_PROFILE`, else the default.
    pub fn from_env() -> RenderProfile {
        std::env::var("STA64_RENDER_PROFILE").ok().and_then(|v| RenderProfile::parse(&v)).unwrap_or_default()
    }
}

/// The effects that have a Refit-derived implementation, by the package that made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Effect {
    /// RND-04: custom SOD materials.
    Materials,
    /// RND-05: nebula, rock and moon scenery.
    Scenery,
    /// RND-06: hull lighting.
    Hulls,
    /// RND-09: beams, ordnance, muzzle flashes.
    Weapons,
    /// RND-10: impacts, shield hits, explosions, damage smoke.
    Impacts,
    /// RND-11: construction, repair, warp, cloak.
    Specials,
    /// RND-14: post FX.
    Post,
    /// RND-15: nebula and hull shadows.
    Shadows,
}

/// What stays in the repository under Refit: each effect, its package, and the
/// shader files it is drawn with (relative to `render/`; none for an effect built
/// on Bevy's own stages).
pub const REFIT_EFFECTS: &[(Effect, &str, &[&str])] = &[
    (Effect::Materials, "RND-04", &[]),
    (Effect::Scenery, "RND-05", &["nebula.wesl", "rock.wesl", "planet.wesl"]),
    (Effect::Hulls, "RND-06", &["hulls.wesl"]),
    (Effect::Weapons, "RND-09", &["fx/quad.wesl"]),
    (Effect::Impacts, "RND-10", &["fx/shield.wesl", "fx/fire.wesl"]),
    (Effect::Specials, "RND-11", &[]),
    (Effect::Post, "RND-14", &[]),
    (Effect::Shadows, "RND-15", &["shadows.wesl"]),
];

/// Original, profile-independent presentation shaders. These are available
/// under both looks and are not part of the preserved Refit-derived effects.
pub const SHARED_SHADERS: &[&str] = &["fx/persistent_shields.wesl"];

/// Which effects have a stock implementation registered.
#[derive(Resource, Default, Debug)]
pub struct ProfileEffects {
    stock: std::collections::BTreeSet<Effect>,
}

impl ProfileEffects {
    pub fn register_stock(&mut self, effect: Effect) {
        self.stock.insert(effect);
    }

    pub fn has_stock(&self, effect: Effect) -> bool {
        self.stock.contains(&effect)
    }

    /// The implementation that draws `effect` under `profile`.
    pub fn implementation(&self, profile: RenderProfile, effect: Effect) -> RenderProfile {
        match profile {
            RenderProfile::Stock if self.has_stock(effect) => RenderProfile::Stock,
            _ => RenderProfile::Refit,
        }
    }
}

/// Run condition: the Refit implementation of `effect` is the one drawing.
pub fn refit_active(effect: Effect) -> impl FnMut(Res<RenderProfile>, Res<ProfileEffects>) -> bool + Clone {
    move |profile, effects| effects.implementation(*profile, effect) == RenderProfile::Refit
}

/// Run condition: the stock implementation of `effect` is the one drawing.
pub fn stock_active(effect: Effect) -> impl FnMut(Res<RenderProfile>, Res<ProfileEffects>) -> bool + Clone {
    move |profile, effects| effects.implementation(*profile, effect) == RenderProfile::Stock
}

/// Sent when the profile changes, after the first frame.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileChanged {
    pub from: RenderProfile,
    pub to: RenderProfile,
}

pub struct ProfilePlugin;

impl Plugin for ProfilePlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<RenderProfile>() {
            app.insert_resource(RenderProfile::from_env());
        }
        app.init_resource::<ProfileEffects>().add_message::<ProfileChanged>().add_systems(Update, (toggle_key, announce).chain());
    }
}

/// F7 flips the profile (a developer key, like post FX's F8).
fn toggle_key(keys: Option<Res<ButtonInput<KeyCode>>>, mut profile: ResMut<RenderProfile>) {
    if keys.is_some_and(|k| k.just_pressed(KeyCode::F7)) {
        *profile = profile.toggled();
    }
}

fn announce(profile: Res<RenderProfile>, mut last: Local<Option<RenderProfile>>, mut out: MessageWriter<ProfileChanged>) {
    if let Some(from) = *last
        && from != *profile
    {
        out.write(ProfileChanged { from, to: *profile });
        info!("render profile: {from:?} -> {:?}", *profile);
    }
    *last = Some(*profile);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Drawn {
        refit: u32,
        stock: u32,
        changes: u32,
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).insert_resource(RenderProfile::Stock).add_plugins(ProfilePlugin).init_resource::<ButtonInput<KeyCode>>().init_resource::<Drawn>();
        app.add_systems(Update, (|mut d: ResMut<Drawn>| d.refit += 1).run_if(refit_active(Effect::Weapons)).after(announce));
        app.add_systems(Update, (|mut d: ResMut<Drawn>| d.stock += 1).run_if(stock_active(Effect::Weapons)).after(announce));
        app.add_systems(Update, (|mut d: ResMut<Drawn>, mut m: MessageReader<ProfileChanged>| d.changes += m.read().count() as u32).after(announce));
        app
    }

    #[test]
    fn default_is_stock_and_names_parse() {
        assert_eq!(RenderProfile::default(), RenderProfile::Stock);
        assert_eq!(RenderProfile::parse(" Refit "), Some(RenderProfile::Refit));
        assert_eq!(RenderProfile::parse("x"), None);
    }

    #[test]
    fn stock_falls_back_to_refit_until_a_stock_effect_exists() {
        let mut app = app();
        app.update();
        app.update();
        let d = app.world().resource::<Drawn>();
        assert_eq!((d.refit, d.stock), (2, 0), "Stock without a stock effect still draws Refit's");
    }

    #[test]
    fn the_toggle_switches_a_running_scene_with_no_restart() {
        let mut app = app();
        app.world_mut().resource_mut::<ProfileEffects>().register_stock(Effect::Weapons);
        app.update();
        {
            let d = app.world().resource::<Drawn>();
            assert_eq!((d.refit, d.stock), (0, 1));
        }
        // F7: the same app, the next frame draws the other implementation.
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::F7);
        app.update();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::F7);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        app.update();
        assert_eq!(*app.world().resource::<RenderProfile>(), RenderProfile::Refit);
        let d = app.world().resource::<Drawn>();
        assert_eq!(d.stock, 1, "stock stopped");
        assert!(d.refit >= 1, "Refit's started");
        assert_eq!(d.changes, 1);
        // And back.
        *app.world_mut().resource_mut::<RenderProfile>() = RenderProfile::Stock;
        let before = app.world().resource::<Drawn>().stock;
        app.update();
        assert_eq!(app.world().resource::<Drawn>().stock, before + 1);
    }

    #[test]
    fn every_refit_shader_is_still_in_the_repository() {
        let render = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/render");
        for (effect, package, shaders) in REFIT_EFFECTS {
            for s in *shaders {
                assert!(render.join(s).is_file(), "{effect:?} ({package}): render/{s} is gone");
            }
        }
        // And each file of the embedded list is some effect's (nothing orphaned).
        for shader in SHARED_SHADERS {
            assert!(render.join(shader).is_file(), "shared shader render/{shader} is gone");
        }
        let named: Vec<&str> = REFIT_EFFECTS.iter().flat_map(|(_, _, s)| s.iter().copied()).chain(SHARED_SHADERS.iter().copied()).collect();
        for e in crate::render::shader_shadow::EMBEDDED {
            let rel = e.trim_start_matches("render/");
            if rel != "fog.wesl" {
                assert!(named.contains(&rel), "{rel} is registered under no effect");
            }
        }
    }
}
