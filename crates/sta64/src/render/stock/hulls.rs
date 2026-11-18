//! The Stock profile's hulls (RND-24): ships and stations lit the way the original
//! engine lights them, from the SOD's own lighting material.
//!
//! Rule (read from data, checked against frames of the original, `bench/pairs/hull-*.ini`):
//! a face group's lit colour is the texture times the team colour (team groups) times
//! `material ambient x scene ambient + sum over the map's lights of material diffuse x
//! light colour x N.L`, plus the material's specular colour times `(N.H)^power` per light,
//! added after the texture; clipped at 1. An Armada I install (`Sta1Classic`) draws no specular
//! term: its SOD materials mostly carry one authoring default specular and the original
//! shows no sheen. Evidence grade: stated by a modder, not measured; the Armada II comparison
//! judged whole-frame luminance and did not isolate the specular term, so Armada II and Fleet
//! Operations keep the term unchanged. On an Armada I install the ambient part is the SUM
//! `material ambient + scene ambient` (the engine's vertex lighting starts each vertex from that
//! sum; read from the executable, static; the planet's disc then matches one frame of the
//! original on the same map within a few percent: seen in two frames of the original, one map);
//! which of the engine's two lighting paths runs is not established. The values are still read, for a later specular port. The Refit look (`hulls.rs`, `hulls.wesl`'s other
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
    /// A `team*` material's own diffuse colour is a placeholder, not a colour to draw:
    /// the SOD files carry red (A1 `team_fed`), purple (A2 Galaxy's `team`) and green
    /// values there, and the player's team colour takes the diffuse's place (the shader
    /// multiplies it in). Applying both turned A1 Federation rims and nacelle caps red
    /// and pink whatever the owner's colour. Read from data (the placeholder values in
    /// every SOD); measured on the A2 original (the Galaxy's `team` strut, diffuse purple
    /// in the SOD, is white under the white player colour); not measured on A1.
    ///
    /// The Borg glow family (`team_Bglow*`, case-insensitive) is left as it was: it keeps its own
    /// diffuse and ambient. Nothing is measured for those regions (not measured, left as on main).
    pub fn from_material(m: &sta_assets::model::LightingMaterial) -> StockLight {
        let placeholder = crate::render::hulls::is_team_material(&m.name) && !is_borg_glow(&m.name);
        let diffuse = if placeholder {
            [1.0; 3]
        } else {
            m.diffuse
        };
        // The ambient colour is the same kind of placeholder (red or purple on A1 `team_fed`,
        // purple on A2 `team`): a grey of equal luminance keeps the brightness and drops the hue.
        let ambient = if placeholder {
            let [r, g, b] = m.ambient;
            if r == g && g == b {
                m.ambient
            } else {
                [(0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 1.0); 3]
            }
        } else {
            m.ambient
        };
        StockLight { ambient, diffuse, specular: m.specular, power: m.power, model: m.model }
    }
}

/// The Borg glow material family (`team_Bglow`, `team_Bglow_1`), excluded from the placeholder rule.
fn is_borg_glow(name: &str) -> bool {
    name.get(..10).is_some_and(|p| p.eq_ignore_ascii_case("team_bglow"))
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
pub fn apply_material(p: &mut crate::render::hulls::HullParams, _m: &StandardMaterial, part: &crate::render::hulls::HullPart, no_specular: bool) {
    let l = part.stock.unwrap_or(StockLight { ambient: [0.0; 3], diffuse: [1.0; 3], specular: [0.0; 3], power: 1.0, model: 2 });
    // `mamb.w` 1: the scene's ambient light multiplies the material's ambient colour; 2 (an Armada I
    // install): the two are added, as the engine's vertex lighting starts each vertex from their sum
    // (read from the executable, static; see `hulls.wesl`'s `shade_stock`).
    p.mamb = Vec3::from(l.ambient).extend(if no_specular { 2.0 } else { 1.0 });
    p.mdif = Vec3::from(l.diffuse).extend(0.0);
    // Armada I: no highlight (the data is read; the specular term itself is untouched).
    let specular = if no_specular { [0.0; 3] } else { l.specular };
    p.mspc = Vec3::from(specular).extend(l.power.max(1.0));
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

    /// Armada I draws no specular term; Armada II and Fleet Operations keep theirs.
    #[test]
    fn armada_one_draws_no_specular() {
        let std = StandardMaterial::default();
        let part = HullPart { stock: Some(lit()), ..default() };
        let mut a1 = HullView::default();
        a1.set_stock(true);
        a1.set_no_specular(true);
        let mut other = HullView::default();
        other.set_stock(true);
        assert_eq!(hull_material(&std, &part, &a1).params.mspc, Vec4::new(0.0, 0.0, 0.0, 40.0));
        assert_eq!(hull_material(&std, &part, &other).params.mspc, Vec4::new(0.2, 0.2, 0.2, 40.0));
    }

    /// Armada I adds the scene ambient to the material's; Armada II and Fleet Operations multiply them.
    #[test]
    fn armada_one_adds_the_scene_ambient_to_the_materials() {
        let std = StandardMaterial::default();
        let part = HullPart { stock: Some(lit()), ..default() };
        let mut a1 = HullView::default();
        a1.set_stock(true);
        a1.set_no_specular(true);
        let mut other = HullView::default();
        other.set_stock(true);
        assert_eq!(hull_material(&std, &part, &a1).params.mamb.w, 2.0);
        assert_eq!(hull_material(&std, &part, &other).params.mamb.w, 1.0);
        let src = include_str!("../hulls.wesl");
        assert!(src.contains("k.mamb.rgb + k.amb.rgb, k.mamb.a > 1.5"));
    }

    fn material(name: &str, diffuse: [f32; 3]) -> sta_assets::model::LightingMaterial {
        sta_assets::model::LightingMaterial {
            name: name.into(),
            ambient: [0.5; 3],
            diffuse,
            specular: [1.0, 0.5, 1.0],
            power: 50.0,
            model: 2,
            self_illuminating: false,
        }
    }

    /// The A1 `team_fed` material carries a red diffuse, the A2 Galaxy's `team` a purple
    /// one: neither may reach the lit colour, only the team colour (the tint) does.
    #[test]
    fn a_team_material_does_not_apply_its_own_diffuse_colour() {
        for (name, diffuse) in [("team_fed", [1.0, 0.0, 0.0]), ("team", [0.55, 0.19, 0.52]), ("Team", [1.0, 0.0, 0.0]), ("team_basic_shaderbump", [0.2, 0.3, 0.4])] {
            let l = StockLight::from_material(&material(name, diffuse));
            assert_eq!(l.diffuse, [1.0; 3], "{name}");
            // The rest of the material is read as ever.
            assert_eq!((l.ambient, l.specular, l.power, l.model), ([0.5; 3], [1.0, 0.5, 1.0], 50.0, 2), "{name}");
        }
    }

    /// The Borg glow family keeps its own diffuse and ambient, as on main.
    #[test]
    fn the_borg_glow_family_keeps_its_own_colours() {
        for name in ["team_Bglow", "TEAM_BGLOW_1", "team_bglow"] {
            let mut m = material(name, [0.0, 1.0, 0.7]);
            m.ambient = [0.0, 0.333, 0.248];
            let l = StockLight::from_material(&m);
            assert_eq!((l.diffuse, l.ambient), ([0.0, 1.0, 0.7], [0.0, 0.333, 0.248]), "{name}");
        }
        let l = StockLight::from_material(&material("team_glow", [0.0, 1.0, 0.7]));
        assert_eq!(l.diffuse, [1.0; 3], "other glow materials follow the team rule");
    }

    /// A coloured team ambient (here a green-cyan one) becomes the grey of equal luminance;
    /// a black one stays black; other materials keep theirs.
    #[test]
    fn a_team_materials_ambient_loses_its_hue_not_its_brightness() {
        let mut m = material("team_fed", [0.0, 1.0, 0.7]);
        m.ambient = [0.0, 0.333, 0.248];
        let a = StockLight::from_material(&m).ambient;
        let lum = 0.587 * 0.333 + 0.114 * 0.248;
        assert!(a.iter().all(|c| (c - lum).abs() < 1e-6), "{a:?}");
        m.ambient = [0.0; 3];
        assert_eq!(StockLight::from_material(&m).ambient, [0.0; 3]);
        let mut other = material("base_fed", [1.0; 3]);
        other.ambient = [0.1, 0.2, 0.3];
        assert_eq!(StockLight::from_material(&other).ambient, [0.1, 0.2, 0.3]);
    }

    /// Every other group keeps the diffuse colour its SOD material states.
    #[test]
    fn other_materials_keep_their_diffuse_colour() {
        for (name, diffuse) in [("base_fed", [1.0, 1.0, 0.93]), ("impulse_fed", [1.0, 0.59, 0.0]), ("glow_white", [0.0, 0.7, 1.0]), ("steam", [0.1, 0.2, 0.3])] {
            assert_eq!(StockLight::from_material(&material(name, diffuse)).diffuse, diffuse, "{name}");
        }
    }

    /// Through the hull material: a team group under a team colour lights as white diffuse
    /// times the tint, so its colour follows the owner and not the SOD's placeholder.
    #[test]
    fn a_team_group_is_drawn_with_a_white_diffuse_under_the_team_colour() {
        let part = HullPart { stock: Some(StockLight::from_material(&material("team_fed", [1.0, 0.0, 0.0]))), ..default() };
        let std = StandardMaterial { base_color: Color::srgb(0.2, 0.4, 0.9), ..default() };
        let mut view = HullView::default();
        view.set_stock(true);
        let hull = hull_material(&std, &part, &view);
        assert_eq!(hull.params.mdif, Vec4::new(1.0, 1.0, 1.0, 0.0));
        let tint = Color::srgb(0.2, 0.4, 0.9).to_linear();
        assert_eq!(hull.params.tint, Vec4::new(tint.red, tint.green, tint.blue, 0.0));
    }

    #[test]
    fn a_part_without_a_material_gets_the_engine_default() {
        let mut p = crate::render::hulls::HullParams::default();
        apply_material(&mut p, &StandardMaterial::default(), &HullPart::default(), false);
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
