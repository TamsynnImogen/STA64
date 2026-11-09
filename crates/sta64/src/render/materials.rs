//! Custom SOD materials: a mesh group's `blend` name picks how its Bevy
//! material is set up (docs/WORKPLAN.md RND-04). Table driven; names not in the
//! table fall back to opaque and are logged once.
//!
//! Depth writing follows from the alpha mode in Bevy (`Opaque`/`Mask` write,
//! `Blend`/`Add` do not), so it is not a separate column.

use std::collections::HashSet;
use std::sync::Mutex;

use bevy::prelude::AlphaMode;

/// How one blend name is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialSpec {
    pub alpha_mode: AlphaMode,
    /// Ignores scene lighting (glows, beams).
    pub unlit: bool,
    /// Drawn without back-face culling whatever the mesh says.
    pub double_sided: bool,
    /// Specular strength (Bevy `reflectance`); 0 for purely diffuse.
    pub reflectance: f32,
}

const fn spec(alpha_mode: AlphaMode, unlit: bool, double_sided: bool, reflectance: f32) -> MaterialSpec {
    MaterialSpec { alpha_mode, unlit, double_sided, reflectance }
}

const OPAQUE: MaterialSpec = spec(AlphaMode::Opaque, false, false, 0.3);
const BLEND: MaterialSpec = spec(AlphaMode::Blend, false, false, 0.3);
/// Additive glow, e.g. engine and shield effects.
const GLOW: MaterialSpec = spec(AlphaMode::Add, true, false, 0.0);
/// Additive weapon or effect part seen from every side (beams, coils, cores).
const EFFECT: MaterialSpec = spec(AlphaMode::Add, true, true, 0.0);

/// Lower-case blend name to setup. Weapon/effect names (`w*`, `x*`) are the
/// engine's custom materials: additive and unlit, our reading of how they look
/// (no engine source for them); `zferengi` is a see-through lit shell.
const TABLE: &[(&str, MaterialSpec)] = &[
    ("", OPAQUE),
    ("default", OPAQUE),
    ("opaque", OPAQUE),
    ("lambert", spec(AlphaMode::Opaque, false, false, 0.0)),
    ("phong", spec(AlphaMode::Opaque, false, false, 0.5)),
    ("phon", spec(AlphaMode::Opaque, false, false, 0.5)),
    ("alpha", BLEND),
    ("translucent", BLEND),
    ("transparent", BLEND),
    // FO's planet SODs: a cloud shell over the ground sphere, blended by the
    // texture's alpha (mostly clear) and lit.
    ("atmosphere", BLEND),
    ("alphathreshold", spec(AlphaMode::Mask(0.5), false, false, 0.3)),
    ("additive", GLOW),
    ("wormhole", GLOW),
    ("whobber", EFFECT),
    ("wtranscoil", EFFECT),
    ("wwarpcoreol", EFFECT),
    ("wtractor", EFFECT),
    ("xcritical05", EFFECT),
    ("xore", EFFECT),
    ("zferengi", BLEND),
];

/// The table entry for a blend name, if it has one.
pub fn known(blend: &str) -> Option<MaterialSpec> {
    TABLE.iter().find(|(n, _)| n.eq_ignore_ascii_case(blend)).map(|(_, s)| *s)
}

/// The setup for a blend name; unknown names draw opaque and are logged once.
pub fn lookup(blend: &str) -> MaterialSpec {
    known(blend).unwrap_or_else(|| {
        static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
        let key = blend.to_ascii_lowercase();
        if let Ok(mut seen) = SEEN.lock()
            && seen.get_or_insert_with(HashSet::new).insert(key.clone())
        {
            bevy::log::warn!("unknown SOD blend `{key}`: drawing opaque");
        }
        OPAQUE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_surveyed_name_is_mapped() {
        let names: Vec<&str> =
            include_str!("blend_names.txt").lines().map(str::trim).filter(|l| !l.starts_with('#') && !l.is_empty()).collect();
        assert!(names.len() >= 15);
        for n in names.into_iter().chain([""]) {
            assert!(known(n).is_some(), "blend `{n}` falls back");
        }
    }

    #[test]
    fn names_are_case_insensitive_and_unique() {
        assert_eq!(known("Additive"), known("additive"));
        let set: HashSet<_> = TABLE.iter().map(|(n, _)| *n).collect();
        assert_eq!(set.len(), TABLE.len());
    }

    #[test]
    fn mappings_have_expected_properties() {
        let s = lookup("wtractor");
        assert_eq!(s.alpha_mode, AlphaMode::Add);
        assert!(s.unlit && s.double_sided);
        assert_eq!(lookup("alphathreshold").alpha_mode, AlphaMode::Mask(0.5));
        assert_eq!(lookup("lambert").reflectance, 0.0);
        assert!(lookup("phong").reflectance > lookup("lambert").reflectance);
        assert_eq!(lookup("atmosphere").alpha_mode, AlphaMode::Blend);
    }

    #[test]
    fn unknown_falls_back_opaque() {
        assert!(known("nonesuch").is_none());
        assert_eq!(lookup("nonesuch"), OPAQUE);
    }
}
