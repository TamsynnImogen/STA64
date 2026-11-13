//! The Stock profile's hulls (RND-24): ships and stations lit the way the original
//! engine lights them, from the SOD's own lighting material.
//!
//! Rule (read from data, checked against frames of the original, `bench/pairs/hull-*.ini`):
//! a face group's lit colour is the texture times the team colour (team groups) times
//! `material ambient x scene ambient + sum over the map's lights of material diffuse x
//! light colour x N.L`, plus the material's specular colour times `(N.H)^power` per light,
//! added after the texture; clipped at 1. The Refit look (`hulls.rs`, `hulls.wesl`'s other
//! branch) keeps its own constants and is drawn under [`RenderProfile::Refit`].
//!
//! Data keys read: per SOD lighting material `ambient`, `diffuse`, `specular`, `power`,
//! `self_illuminating`; the mesh's bump flag (read, and not drawn as relief); the map's ambient colour (`GlobalAmbientLight`) and its `dlight`
//! colours (the scene's directional lights).

use bevy::prelude::*;

use crate::render::profile::{Effect, ProfileEffects};

/// A SOD lighting material as the Stock hull look takes it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StockLight {
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    pub power: f32,
    pub model: u8,
}

impl StockLight {
    pub fn from_material(m: &sta_assets::model::LightingMaterial) -> StockLight {
        StockLight { ambient: m.ambient, diffuse: m.diffuse, specular: m.specular, power: m.power, model: m.model }
    }
}

/// Registers the stock hull look: `Effect::Hulls` has a stock implementation, which
/// `HullView` reads to write each hull material in the stock way.
pub struct StockHullsPlugin;

impl Plugin for StockHullsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProfileEffects>();
        let mut effects = app.world_mut().resource_mut::<ProfileEffects>();
        effects.register_stock(Effect::Hulls);
        // The original draws with no tonemapper: Stock's `post::apply` leaves it off.
        effects.register_stock(Effect::Post);
    }
}

/// Writes the stock look into a hull material's parameters (a part with no lighting
/// material, e.g. a glTF, takes the engine's default white one; an M3D's material carries
/// no ambient colour, so only the lights reach it).
pub fn apply_material(p: &mut crate::render::hulls::HullParams, _m: &StandardMaterial, part: &crate::render::hulls::HullPart) {
    let l = part.stock.unwrap_or(StockLight { ambient: [0.0; 3], diffuse: [1.0; 3], specular: [0.0; 3], power: 1.0, model: 2 });
    p.mamb = Vec3::from(l.ambient).extend(1.0);
    p.mdif = Vec3::from(l.diffuse).extend(0.0);
    p.mspc = Vec3::from(l.specular).extend(l.power.max(1.0));
    p.misc = Vec4::new(1.0, 1.0, l.power.max(1.0), 1.0);
    p.rim = Vec4::ZERO;
    p.cut.z = 1.0;
    // A bump-mapped mesh is the engine's GPU vertex path, not a look: the originals' Borg
    // faces show no relief (measured, `hull-borg` pairs), so the normal map is not applied.
    p.cut.y = 0.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::hulls::{hull_material, HullPart, HullView};

    fn lit() -> StockLight {
        StockLight { ambient: [0.5; 3], diffuse: [0.9, 0.8, 0.7], specular: [0.2; 3], power: 40.0, model: 1 }
    }

    #[test]
    fn stock_writes_the_sod_material_and_refit_does_not() {
        let part = HullPart { stock: Some(lit()), ..default() };
        let std = StandardMaterial::default();
        let refit = hull_material(&std, &part, &HullView::default());
        assert_eq!(refit.params.mamb.w, 0.0, "Refit's look is untouched");
        let mut view = HullView::default();
        view.set_stock(true);
        let stock = hull_material(&std, &part, &view);
        assert_eq!(stock.params.mamb, Vec4::new(0.5, 0.5, 0.5, 1.0));
        assert_eq!(stock.params.mdif, Vec4::new(0.9, 0.8, 0.7, 0.0));
        assert_eq!(stock.params.mspc, Vec4::new(0.2, 0.2, 0.2, 40.0));
        assert_eq!(stock.params.rim, Vec4::ZERO, "no rim light in the original");
    }

    #[test]
    fn a_part_without_a_material_gets_the_engine_default() {
        let mut p = crate::render::hulls::HullParams::default();
        apply_material(&mut p, &StandardMaterial::default(), &HullPart::default());
        assert_eq!(p.mdif, Vec4::new(1.0, 1.0, 1.0, 0.0));
        assert_eq!(p.mamb.w, 1.0);
    }

    #[test]
    fn the_plugin_registers_hulls_and_post_as_stock() {
        let mut app = App::new();
        app.add_plugins(StockHullsPlugin);
        let e = app.world().resource::<ProfileEffects>();
        assert!(e.has_stock(Effect::Hulls) && e.has_stock(Effect::Post));
    }
}
