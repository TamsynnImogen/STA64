//! JSON5 content (CON-04): the same definitions as the game's ODFs, written
//! as JSON5, loaded through the same code.
//!
//! A content file does not introduce a data model. Each entry of its `odf`
//! table is one ODF, keyed by ODF name, with the game's own keys and units
//! (`maxHealth`, `shieldRate` per second, `shotDelay` in seconds, `weapon1` /
//! `weaponHardPoints1`, `builditem1`, `physicsFile`, `ordName`, ...; see
//! `docs/data/odf_keys.csv`). [`load_str`] renders each entry as ODF text and
//! hands the lot to [`crate::odf::load_with`], so conversion, class handling
//! and profile rules are CON-03's and cannot drift from what installed ODFs
//! get. `#include` works between entries. The schema is in
//! `docs/CONTENT_JSON5.md`.
//!
//! Three things the ODF loader cannot express and the fakes need are read in
//! a small final pass ([`finish`]), each from a key the ODF format already
//! has or an `STA64_` extension in the style of `STA64_queueLimit`:
//!
//! - `miner = 0` on a `freighter` (the key exists; the ODF loader forces
//!   class `freighter` to mine),
//! - `eventAttack` (the key exists) fills the unit's attack acknowledgement,
//! - `STA64_fireArc<N>`: half-width in radians of the arc of weapon slot `N`
//!   on this unit alone (stock ODFs only limit a weapon everywhere it is
//!   mounted, through `restrictFireArc` / `fireArc`),
//! - `STA64_duration` on a weapon: seconds it stays active, for kinds whose
//!   ordnance gives none (point defence).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use sta_core::Fixed;

use crate::def::FireArc;
use crate::features::Profile;
use crate::odf::{numbered, stem};
use crate::profile::Overrides;
use crate::{ContentDb, ContentError, MapDef, Warning};

/// Why a content or map file could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Json5Error {
    #[error("{file}: {message}")]
    Parse { file: String, message: String },
    #[error("{0}")]
    Io(String),
    #[error(transparent)]
    Content(#[from] ContentError),
}

/// One ODF value. Booleans become `1`/`0`, a list becomes the space-separated
/// words ODFs use for several values (`"hp01" "hp02"`).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Val {
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Val>),
}

impl Val {
    fn render(&self) -> String {
        match self {
            Val::Bool(b) => (*b as u8).to_string(),
            Val::Num(n) => n.to_string(),
            Val::Str(s) => format!("\"{}\"", s.replace('"', "")),
            Val::List(items) => items.iter().map(Val::render).collect::<Vec<_>>().join(" "),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Doc {
    /// `native` (default), `fo4`, `sta2classic`, `sta1classic`.
    #[serde(default)]
    profile: Option<String>,
    /// Feature switches over the profile's defaults, as `info.ini [Features]`.
    #[serde(default)]
    features: BTreeMap<String, bool>,
    /// ODFs by name.
    odf: BTreeMap<String, BTreeMap<String, Val>>,
}

/// An entry rendered as ODF text.
fn odf_text(keys: &BTreeMap<String, Val>) -> String {
    let mut out = String::from("[GameObjectClass]\n");
    for (k, v) in keys {
        out.push_str(&format!("{k} = {}\n", v.render()));
    }
    out
}

/// Build a database from the text of a content file. `file` names it in
/// errors and warnings.
pub fn load_str(file: &str, text: &str) -> Result<(ContentDb, Vec<Warning>), Json5Error> {
    let doc: Doc = ::json5::from_str(text).map_err(|e| Json5Error::Parse {
        file: file.to_owned(),
        message: e.to_string(),
    })?;
    let profile = match doc.profile.as_deref() {
        None => Profile::Native,
        Some(p) => Profile::from_name(p).ok_or_else(|| Json5Error::Parse {
            file: file.to_owned(),
            message: format!("unknown profile {p:?}"),
        })?,
    };
    let mut overrides = Overrides::default();
    for (name, on) in &doc.features {
        overrides.insert(name, *on);
    }
    let texts: BTreeMap<String, String> = doc
        .odf
        .iter()
        .map(|(n, k)| (stem(n), odf_text(k)))
        .collect();
    let stems: Vec<String> = texts.keys().cloned().collect();
    let (db, mut warnings) =
        crate::odf::load_with(profile, &overrides, &stems, &mut |name, version| {
            if version == 0 {
                texts.get(&stem(name)).cloned()
            } else {
                None
            }
        })?;
    let db = finish(&db, profile, &mut warnings)?;
    Ok((db, warnings))
}

/// Build a database from `<dir>/content.json5`.
pub fn load_dir(dir: &Path) -> Result<(ContentDb, Vec<Warning>), Json5Error> {
    let path = dir.join("content.json5");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Json5Error::Io(format!("{}: {e}", path.display())))?;
    load_str(&path.display().to_string(), &text)
}

/// A native map (`MapDef` as JSON5, plain numbers where the sim has `Fixed`).
pub fn load_map_str(file: &str, text: &str) -> Result<MapDef, Json5Error> {
    ::json5::from_str(text).map_err(|e| Json5Error::Parse {
        file: file.to_owned(),
        message: e.to_string(),
    })
}

/// The native map save format (CON-32): `MapDef` as JSON5, which
/// [`load_map_str`] reads back to an equal `MapDef`. Numbers are plain; enums
/// are externally tagged (see `docs/CONTENT_JSON5.md`).
pub fn save_map(map: &MapDef) -> Result<String, Json5Error> {
    ::json5::to_string(map).map_err(|e| Json5Error::Parse {
        file: map.name.clone(),
        message: e.to_string(),
    })
}

/// The final pass described in the module docs: rebuild the database with the
/// few fields the ODF loader leaves to their keys.
fn finish(
    db: &ContentDb,
    profile: Profile,
    warnings: &mut Vec<Warning>,
) -> Result<ContentDb, ContentError> {
    let mut b = ContentDb::builder()
        .profile(profile)
        .features(db.features().clone());
    let weapons: BTreeSet<String> = db.weapons().map(|w| stem(&w.odf)).collect();
    for w in db.weapons() {
        let mut w = w.clone();
        if let Some(s) = w
            .raw
            .f32("STA64_duration")
            .filter(|s| s.is_finite() && *s >= 0.0)
        {
            w.duration = sta_core::seconds_to_ticks(s as f64);
        }
        b = b.weapon(w);
    }
    for u in db.units() {
        let mut u = u.clone();
        let file = format!("{}.odf", u.odf);
        if u.raw.raw("miner").is_some() && u.raw.f32("miner") == Some(0.0) {
            u.miner = false;
        }
        if let Some(s) = u
            .raw
            .str("eventAttack")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            u.sounds.ack_attack = vec![s.to_owned()];
        }
        // Weapon slots that became hardpoints, in the loader's order.
        let slots: Vec<u32> = numbered(&u.raw, "weapon")
            .into_iter()
            .filter(|(_, w)| weapons.contains(&stem(w)))
            .map(|(n, _)| n)
            .collect();
        for key in u
            .raw
            .keys()
            .filter(|k| k.to_ascii_lowercase().starts_with("sta64_firearc"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
        {
            let n: Option<u32> = key["STA64_fireArc".len()..].parse().ok();
            // f64, not the loader's f32: the angle's cosine is hashed.
            let half = u
                .raw
                .raw(&key)
                .and_then(|t| t.trim().parse::<f64>().ok())
                .filter(|h| h.is_finite() && *h > 0.0);
            match (n.and_then(|n| slots.iter().position(|s| *s == n)), half) {
                (Some(i), Some(half)) if i < u.hardpoints.len() => {
                    u.hardpoints[i].arc = Some(FireArc {
                        dir: (Fixed::ZERO, Fixed::ONE),
                        min_dot: Fixed::from_f64(half.min(std::f64::consts::PI).cos()),
                    });
                }
                _ => warnings.push(Warning::new(
                    &file,
                    format!("{key} names no weapon slot or is not a positive angle; ignored"),
                )),
            }
        }
        b = b.unit(u);
    }
    b.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
    }

    #[test]
    fn fixture_content_hashes_like_the_fake() {
        let (db, warnings) = load_dir(&fixtures().join("content")).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let fake = crate::fake::db();
        assert_eq!(db.len(), fake.len());
        for (a, b) in db.units().zip(fake.units()) {
            let (mut a, mut b) = (a.clone(), b.clone());
            // Not hashed, and not expressible as ODF keys: explosion sound, damage effects.
            // Features equal to the database's are the same as none.
            for u in [&mut a, &mut b] {
                u.raw = Default::default();
                u.sounds.explode = None;
                u.fx.damage.clear();
                u.features = None;
            }
            assert_eq!(format!("{a:#?}"), format!("{b:#?}"), "{}", a.odf);
        }
        for (a, b) in db.weapons().zip(fake.weapons()) {
            let (mut a, mut b) = (a.clone(), b.clone());
            for w in [&mut a, &mut b] {
                w.raw = Default::default();
                w.features = None;
            }
            assert_eq!(format!("{a:#?}"), format!("{b:#?}"), "{}", a.odf);
        }
        assert_eq!(db.hash(), fake.hash());
        // Names, keys and cross-references agree, not only the hash.
        for (a, b) in db.units().zip(fake.units()) {
            assert_eq!(
                (a.key, &a.odf, &a.model, &a.race),
                (b.key, &b.odf, &b.model, &b.race)
            );
            assert_eq!(
                a.hardpoints
                    .iter()
                    .map(|h| (h.weapon.key, h.arc))
                    .collect::<Vec<_>>(),
                b.hardpoints
                    .iter()
                    .map(|h| (h.weapon.key, h.arc))
                    .collect::<Vec<_>>()
            );
            assert_eq!(a.sounds.ack_attack, b.sounds.ack_attack);
        }
    }

    #[test]
    fn fixture_map_equals_the_fake_map() {
        let text = std::fs::read_to_string(fixtures().join("content/two_bases.json5")).unwrap();
        assert_eq!(
            load_map_str("two_bases.json5", &text).unwrap(),
            crate::fake::two_bases()
        );
    }

    #[test]
    fn saved_maps_read_back_equal() {
        let map = crate::fake::two_bases();
        let text = save_map(&map).unwrap();
        assert_eq!(load_map_str("saved", &text).unwrap(), map);
        // Fixed-point values survive exactly, not only the round numbers.
        let mut odd = map.clone();
        odd.starts[0].pos.x = Fixed::from_f64(1234.567);
        odd.objects[0].heading = (Fixed::from_f64(0.6), Fixed::from_f64(0.8));
        assert_eq!(
            load_map_str("saved", &save_map(&odd).unwrap()).unwrap(),
            odd
        );
    }

    #[test]
    fn every_fixture_model_loads() {
        let mut vfs = sta_vfs::Vfs::default();
        vfs.push_dir("fixtures".into(), fixtures().join("models"))
            .unwrap();
        let (db, _) = load_dir(&fixtures().join("content")).unwrap();
        for u in db.units() {
            let m = sta_assets::gltf::load(&vfs, &format!("{}.glb", u.model))
                .unwrap_or_else(|e| panic!("{}: {e}", u.model));
            assert!(!m.nodes.is_empty(), "{} has no nodes", u.model);
        }
    }

    #[test]
    fn odf_keys_flow_through_the_odf_loader() {
        let text = r#"{
            profile: "fo4",
            odf: {
                gun:  { classLabel: "cannon", wpnName: "Gun", ordName: "gun_ord", range: 500, shotDelay: 2 },
                gun_ord: { classLabel: "photontorpedo", damageBase: 30, shotSpeed: 300 },
                ship: { classLabel: "craft", unitName: "S", maxHealth: 100, shieldRate: 3,
                        weapon1: "gun", weaponHardPoints1: ["hp01", "hp02"], eventAttack: "Atk" },
            },
        }"#;
        let (db, warnings) = load_str("t", text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(db.profile(), Profile::Fo4);
        let s = db.unit(db.key("ship").unwrap()).unwrap();
        assert_eq!(s.hardpoints[0].node, "hp01");
        assert_eq!(s.hull, Fixed::from_int(100));
        assert_eq!(s.sounds.ack_attack, ["Atk"]);
        let g = db.weapon(db.key("gun").unwrap()).unwrap();
        assert_eq!(g.refire, sta_core::seconds_to_ticks(2.0));
        assert_eq!(g.shot_speed, Fixed::from_int(300));
    }

    #[test]
    fn bad_input_is_an_error_or_a_warning() {
        assert!(matches!(
            load_str("t", "{ nope"),
            Err(Json5Error::Parse { .. })
        ));
        assert!(matches!(
            load_str("t", r#"{ odf: {}, extra: 1 }"#),
            Err(Json5Error::Parse { .. })
        ));
        assert!(matches!(
            load_str("t", r#"{ profile: "zzz", odf: {} }"#),
            Err(Json5Error::Parse { .. })
        ));
        let text = r#"{ odf: { a: { classLabel: "craft", unitName: "A", STA64_fireArc9: 1 } } }"#;
        let (_, w) = load_str("t", text).unwrap();
        assert!(
            w.iter().any(|w| w.message.contains("STA64_fireArc9")),
            "{w:?}"
        );
    }
}
