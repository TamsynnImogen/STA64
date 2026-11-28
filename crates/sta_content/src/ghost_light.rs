//! The lighting of objects out of sight (#1146): the keys of the global light ODF.
//!
//! Rule (Armada II; read from the executable, static): a last-seen copy of an object is drawn with a lighting
//! material of its own in place of the scene's, built from the keys `ghostAmbient`, `ghostDiffuse` and
//! `ghostSpecular` of `light.odf`; the copy's team colour is scaled by `ghostTeamColorFactor`. Armada I: not
//! established beyond the same key names. Grade for the keys: read from data (a file's own values are the content's,
//! not a rule of this crate).
//!
//! This module only reads the keys. A key that is absent or malformed is `None`, with a warning when malformed;
//! no default is invented (the executable's own defaults were not read). A mod's `light.odf` wins over the base
//! game's through the ODF index like any other file. Nothing here says how a renderer combines the keys with
//! anything else.

use sta_odf::Odf;

use crate::{OdfIndex, Warning};

/// The keys of `light.odf` that light the copies.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GhostLighting {
    /// `ghostAmbient`: the lighting material's ambient colour.
    pub ambient: Option<[f32; 3]>,
    /// `ghostDiffuse`: its diffuse colour.
    pub diffuse: Option<[f32; 3]>,
    /// `ghostSpecular`: its specular colour.
    pub specular: Option<[f32; 3]>,
    /// `ghostTeamColorFactor`: the factor on a copy's team colour (the Armada II family only).
    pub team_factor: Option<f32>,
}

impl GhostLighting {
    /// Whether the three colours of the lighting material are all given: only then is there a lighting to put in
    /// place of the scene's.
    pub fn is_complete(&self) -> bool {
        self.ambient.is_some() && self.diffuse.is_some() && self.specular.is_some()
    }

    /// The keys of a resolved `light.odf`; `None` when it has none of them.
    pub fn from_odf(file: &str, odf: &Odf, warnings: &mut Vec<Warning>) -> Option<GhostLighting> {
        let colour = |key: &str, warnings: &mut Vec<Warning>| -> Option<[f32; 3]> {
            let raw = odf.raw(key)?;
            let parts: Vec<Option<f32>> = raw.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).map(|s| s.parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0)).collect();
            match parts.as_slice() {
                [Some(r), Some(g), Some(b)] => Some([*r, *g, *b]),
                _ => {
                    warnings.push(Warning::new(file, format!("{key} = {raw} is not three non-negative numbers; ignored")));
                    None
                }
            }
        };
        let team_factor = odf.raw("ghostTeamColorFactor").and_then(|raw| match odf.f32("ghostTeamColorFactor") {
            Some(v) if v.is_finite() && v >= 0.0 => Some(v),
            _ => {
                warnings.push(Warning::new(file, format!("ghostTeamColorFactor = {raw} is not a non-negative number; ignored")));
                None
            }
        });
        let keys = GhostLighting { ambient: colour("ghostAmbient", warnings), diffuse: colour("ghostDiffuse", warnings), specular: colour("ghostSpecular", warnings), team_factor };
        (keys != GhostLighting::default()).then_some(keys)
    }

    /// The keys of the `light.odf` the mounted layers resolve to, or `None` when there is none or it has none of
    /// them.
    pub fn load(vfs: &sta_vfs::Vfs, index: &OdfIndex) -> (Option<GhostLighting>, Vec<Warning>) {
        let mut warnings = Vec::new();
        let Some(text) = index.load(vfs, "light", 0) else { return (None, warnings) };
        let odf = match sta_odf::resolve("light", &mut |_| Some(text.clone())) {
            Ok(o) => o,
            Err(e) => {
                warnings.push(Warning::new("light.odf", e.to_string()));
                return (None, warnings);
            }
        };
        let keys = GhostLighting::from_odf("light.odf", &odf, &mut warnings);
        (keys, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> (Option<GhostLighting>, Vec<Warning>) {
        let odf = sta_odf::resolve("light", &mut |_| Some(text.to_owned())).unwrap();
        let mut w = Vec::new();
        (GhostLighting::from_odf("light.odf", &odf, &mut w), w)
    }

    #[test]
    fn the_keys_are_read_as_authored() {
        let (k, w) = read("ghostAmbient = 0.25 0.5 0.75\nghostDiffuse = 1 2 3\nghostSpecular = 0 0 0\nghostTeamColorFactor = 0.125\n");
        assert!(w.is_empty(), "{w:?}");
        let k = k.unwrap();
        assert_eq!(k.ambient, Some([0.25, 0.5, 0.75]));
        assert_eq!(k.diffuse, Some([1.0, 2.0, 3.0]));
        assert_eq!(k.specular, Some([0.0; 3]));
        assert_eq!(k.team_factor, Some(0.125));
        assert!(k.is_complete());
    }

    #[test]
    fn an_absent_key_is_none_and_no_default_is_made_up() {
        let (k, w) = read("ghostAmbient = 0.5 0.5 0.5\nghostDiffuse = 0.5 0.5 0.5\n");
        assert!(w.is_empty());
        let k = k.unwrap();
        assert_eq!((k.specular, k.team_factor), (None, None));
        assert!(!k.is_complete(), "no specular: no lighting to put in place of the scene's");
        assert_eq!(read("somethingElse = 1\n").0, None, "a file with none of the keys gives none");
    }

    #[test]
    fn a_malformed_key_is_ignored_with_a_warning() {
        let (k, w) = read("ghostAmbient = 0.5 0.5\nghostDiffuse = a b c\nghostSpecular = -1 0 0\nghostTeamColorFactor = x\nghostTeamColorFactor2 = 1\n");
        assert_eq!(k, None);
        assert_eq!(w.len(), 4, "{w:?}");
    }

    #[test]
    fn commas_separate_the_numbers_too() {
        let (k, _) = read("ghostAmbient = 0.1, 0.2, 0.3\n");
        assert_eq!(k.unwrap().ambient, Some([0.1, 0.2, 0.3]));
    }
}
