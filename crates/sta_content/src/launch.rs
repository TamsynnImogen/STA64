//! Launch points (SIM-16): where a finished unit leaves its builder.
//!
//! Convention, read from A1, A2, Dominion War, Fleet Operations and the
//! STA mods: a builder's ODF names a model node in `buildHardpoint` (`build`,
//! `hp01`, `hp30`, `hp31`, the placeholder `hp99`, or `root`). That node's
//! position (model space, times `ScaleSOD`) is where the unit appears and its
//! +z axis the way it faces. Builders that deploy instead (construction rigs,
//! evolvers) have no key, and a key whose node is missing from the model is
//! as good as none; both take the stated fallback: the builder's centre,
//! facing ahead, clear once outside the model's bounding circle (or
//! [`DEFAULT_CLEAR`] when no model is found). `root` is the model origin.

use std::collections::BTreeMap;

use a2fmt::sod::Sod;
use sta_core::Fixed;

use crate::Warning;
use crate::def::{LaunchPoint, UnitDef};

/// Clear radius, units, when the builder's model cannot be read.
pub const DEFAULT_CLEAR: i32 = 100;

fn fx(v: f32) -> Fixed {
    Fixed::from_f64(if v.is_finite() { v as f64 } else { 0.0 })
}

fn unit2(x: f32, z: f32) -> Option<(f32, f32)> {
    let l = (x * x + z * z).sqrt();
    (l.is_finite() && l > 1e-3).then(|| (x / l, z / l))
}

/// Loosest bounding circle round the model origin (XZ), model units: every
/// mesh's sphere.
fn bounding_circle(sod: &Sod) -> f32 {
    let mut clear = 0.0f32;
    for (i, n) in sod.nodes.iter().enumerate() {
        if let Some(m) = n.mesh() {
            let (c, r) = m.bounds();
            let p = sod.model_transform(i).transform_point(c);
            clear = clear.max((p[0] * p[0] + p[2] * p[2]).sqrt() + r);
        }
    }
    clear
}

/// Collision radius, units, of a unit whose model exists only as a render
/// replacement (glTF/GLB/M3D) with no readable SOD to measure. Equal to
/// [`DEFAULT_CLEAR`], the stated size for an unreadable model; a placeholder
/// until the engine's own size rule is ported. Not scaled by `ScaleSOD`.
/// Units with no model at all
/// (or only an unreadable SOD) keep radius zero, as before.
pub const FALLBACK_RADIUS: i32 = DEFAULT_CLEAR;

/// What the VFS holds for a model name.
pub(crate) enum ModelSod {
    /// A readable SOD (the simulation's geometry).
    Sod(Box<Sod>),
    /// A render replacement exists but no readable SOD does.
    RenderOnly,
    /// No model, or only an unreadable SOD.
    Missing,
}

/// The unit's SOD, read even when a higher layer (or the same one) supplies a
/// glTF/GLB/M3D that the renderer prefers: the renderer's pick is tried first
/// when it is a SOD (or an M3D that really holds one), then the best `.sod`.
pub(crate) fn read_sod(vfs: &sta_vfs::Vfs, key: &str) -> ModelSod {
    use sta_assets::ModelFormat;
    let Some((path, format)) = sta_assets::find_model(vfs, key) else {
        return ModelSod::Missing;
    };
    if matches!(format, ModelFormat::Sod | ModelFormat::M3d)
        && let Ok(s) = sta_assets::load_sod(vfs, &path)
    {
        return ModelSod::Sod(Box::new(s));
    }
    if let Some(s) = sta_assets::find_sod(vfs, key).and_then(|p| sta_assets::load_sod(vfs, &p).ok()) {
        return ModelSod::Sod(Box::new(s));
    }
    if format == ModelFormat::Sod {
        ModelSod::Missing
    } else {
        ModelSod::RenderOnly
    }
}

/// Fill `radius` (SIM-21) of every unit from its SOD's bounding circle times
/// `ScaleSOD`, whatever render model replaces it. A unit with a render
/// replacement but no readable SOD gets [`FALLBACK_RADIUS`] and a warning;
/// one with no model at all keeps zero.
pub fn attach_radii<'a>(vfs: &sta_vfs::Vfs, units: impl Iterator<Item = &'a mut UnitDef>) -> Vec<Warning> {
    let mut circles: BTreeMap<String, Option<f32>> = BTreeMap::new();
    let mut warnings = Vec::new();
    for u in units {
        let key = u.model.to_ascii_lowercase().trim_end_matches(".sod").to_owned();
        if key.is_empty() {
            continue;
        }
        let c = *circles.entry(key.clone()).or_insert_with(|| match read_sod(vfs, &key) {
            ModelSod::Sod(s) => Some(bounding_circle(&s)),
            ModelSod::RenderOnly => Some(-1.0),
            ModelSod::Missing => None,
        });
        let scale = u.scale.to_f32();
        let scale = if scale > 0.0 { scale } else { 1.0 };
        match c {
            Some(c) if c < 0.0 => {
                u.radius = Fixed::from_int(FALLBACK_RADIUS);
                warnings.push(Warning::new(
                    format!("sod/{key}.sod"),
                    format!("{}: no readable SOD for the collision radius (a render model replaces it); using {FALLBACK_RADIUS}", u.odf),
                ));
            }
            Some(c) if c > 0.0 => u.radius = fx(c * scale),
            _ => {}
        }
    }
    warnings
}

/// The launch point of `def` from its model (`None`: model not readable).
pub fn launch_point(def: &UnitDef, sod: Option<&Sod>) -> LaunchPoint {
    let scale = def.scale.to_f32();
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let Some(sod) = sod else {
        return LaunchPoint {
            offset: (Fixed::ZERO, Fixed::ZERO),
            dir: (Fixed::ZERO, Fixed::ONE),
            clear: Fixed::from_int(DEFAULT_CLEAR),
            node: None,
        };
    };
    let clear = bounding_circle(sod);
    let clear = if clear > 0.0 {
        clear * scale
    } else {
        DEFAULT_CLEAR as f32
    };
    let name = def
        .raw
        .str("buildHardpoint")
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let found = name.and_then(|n| {
        if n.eq_ignore_ascii_case("root") {
            return Some((n, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]));
        }
        let i = sod.find(n)?;
        let m = sod.model_transform(i);
        Some((n, m.translation(), m.axis(2)))
    });
    match found {
        Some((n, t, z)) => {
            let (ox, oz) = (t[0] * scale, t[2] * scale);
            let (dx, dz) = unit2(z[0], z[2])
                .or_else(|| unit2(ox, oz))
                .unwrap_or((0.0, 1.0));
            // Never nearer than the hardpoint itself.
            let clear = clear.max((ox * ox + oz * oz).sqrt());
            LaunchPoint {
                offset: (fx(ox), fx(oz)),
                dir: (fx(dx), fx(dz)),
                clear: fx(clear),
                node: Some(n.to_owned()),
            }
        }
        None => LaunchPoint {
            offset: (Fixed::ZERO, Fixed::ZERO),
            dir: (Fixed::ZERO, Fixed::ONE),
            clear: fx(clear),
            node: None,
        },
    }
}

/// Fill `launch` of every unit that builds something.
pub fn attach<'a>(vfs: &sta_vfs::Vfs, units: impl Iterator<Item = &'a mut UnitDef>) {
    let mut sods: BTreeMap<String, Option<Sod>> = BTreeMap::new();
    for u in units {
        if u.build.list.is_empty() {
            continue;
        }
        let key = u
            .model
            .to_ascii_lowercase()
            .trim_end_matches(".sod")
            .to_owned();
        let sod = sods.entry(key.clone()).or_insert_with(|| match read_sod(vfs, &key) {
            ModelSod::Sod(s) => Some(*s),
            _ => None,
        });
        u.launch = Some(launch_point(u, sod.as_ref()));
    }
}
