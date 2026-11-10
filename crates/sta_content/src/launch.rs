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

/// Fill `radius` (SIM-21) of every unit from its model's bounding circle
/// times `ScaleSOD`; units whose model cannot be read keep zero.
pub fn attach_radii<'a>(vfs: &sta_vfs::Vfs, units: impl Iterator<Item = &'a mut UnitDef>) {
    let mut circles: BTreeMap<String, Option<f32>> = BTreeMap::new();
    for u in units {
        let key = u.model.to_ascii_lowercase().trim_end_matches(".sod").to_owned();
        if key.is_empty() {
            continue;
        }
        let c = *circles.entry(key.clone()).or_insert_with(|| {
            let (path, _) = sta_assets::find_model(vfs, &key)?;
            sta_assets::load_sod(vfs, &path).ok().map(|s| bounding_circle(&s))
        });
        let scale = u.scale.to_f32();
        let scale = if scale > 0.0 { scale } else { 1.0 };
        if let Some(c) = c.filter(|c| *c > 0.0) {
            u.radius = fx(c * scale);
        }
    }
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
        let sod = sods.entry(key.clone()).or_insert_with(|| {
            let (path, _) = sta_assets::find_model(vfs, &key)?;
            sta_assets::load_sod(vfs, &path).ok()
        });
        u.launch = Some(launch_point(u, sod.as_ref()));
    }
}
