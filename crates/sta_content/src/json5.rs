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
//! Keys beyond stock ODFs are read by the ODF loader itself, so installed ODFs
//! and JSON5 agree: `miner = 0` (the key decides, see `UnitDef::mines`),
//! `eventAttack`, and the `STA64_` extensions `STA64_fireArc<N>` (a slot's own
//! fire arc) and `STA64_duration` (a weapon's active seconds).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::features::Profile;
use crate::odf::stem;
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
    // An `"#include"` key (a name or a list) becomes the ODF's own include
    // lines, which stock ODFs put first; the keys after them override.
    let mut out = String::new();
    if let Some(inc) = keys.get("#include") {
        for line in inc.render().split("\" \"") {
            out.push_str(&format!("#include \"{}\"\n", line.trim_matches('"')));
        }
    }
    out.push_str("[GameObjectClass]\n");
    for (k, v) in keys.iter().filter(|(k, _)| *k != "#include") {
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
    let (db, warnings) =
        crate::odf::load_with(profile, &overrides, &stems, &mut |name, version| {
            if version == 0 {
                texts.get(&stem(name)).cloned()
            } else {
                None
            }
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use sta_core::Fixed;
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

    /// The keys the games' own ODFs carry, and the `STA64_` extensions, are read by the ODF loader.
    #[test]
    fn shared_keys_are_read_by_the_odf_loader() {
        let text = r##"{
            odf: {
                // The stock shape: the Data Miner includes freight.odf and says miner = 0.
                freight: { classLabel: "freighter", unitName: "F", miner: 1 },
                fdata:   { "#include": "freight.odf", unitName: "D", miner: 0 },
                bare:    { classLabel: "freighter", unitName: "B" },
                drone:   { classLabel: "craft", unitName: "C", miner: 1 },
                plain:   { classLabel: "craft", unitName: "P" },
                gun: { classLabel: "pointdefenselaser", wpnName: "G", ordName: "gun_ord", STA64_duration: 0.1 },
                beam: { classLabel: "cannon", wpnName: "B", ordName: "beam_ord", restrictFireArc: 1, fireArc: 1.0, STA64_duration: -1 },
                gun_ord: { classLabel: "pphaser", damageBase: 1 },
                beam_ord: { classLabel: "phaser", damageBase: 1, lifespan: 0.5 },
                ship: { classLabel: "craft", unitName: "S", eventAttack: "Atk",
                        weapon1: "gun", weaponHardPoints1: "hp01", STA64_fireArc1: 0.5,
                        weapon2: "gun", weaponHardPoints2: "hp02",
                        weapon3: "beam", weaponHardPoints3: "hp03", STA64_fireArc3: 0.25,
                        weapon4: "beam", weaponHardPoints4: "hp04",
                        weapon5: "gun", weaponHardPoints5: "hp05", STA64_fireArc5: "abc" },
            },
        }"##;
        let (db, warnings) = load_str("t", text).unwrap();
        let mines = |n: &str| db.unit(db.key(n).unwrap()).unwrap().mines();
        assert!(mines("freight") && mines("bare") && mines("drone"));
        assert!(
            !mines("fdata"),
            "miner = 0 on a freighter that includes a miner"
        );
        assert!(!mines("plain"));
        let ship = db.unit(db.key("ship").unwrap()).unwrap();
        assert_eq!(ship.sounds.ack_attack, ["Atk"]);
        let cos = |x: f64| Fixed::from_f64(x.cos());
        assert_eq!(
            ship.hardpoints[0].arc.expect("slot 1's own arc").min_dot,
            cos(0.5)
        );
        assert!(ship.hardpoints[1].arc.is_none());
        assert_eq!(
            ship.hardpoints[2]
                .arc
                .expect("slot 3 wins over the weapon's")
                .min_dot,
            cos(0.25)
        );
        assert_eq!(
            ship.hardpoints[3]
                .arc
                .expect("the weapon's own arc")
                .min_dot,
            cos(1.0)
        );
        assert!(ship.hardpoints[4].arc.is_none());
        assert!(
            warnings
                .iter()
                .any(|w| w.message.contains("STA64_fireArc5")),
            "{warnings:?}"
        );
        let gun = db.weapon(db.key("gun").unwrap()).unwrap();
        assert_eq!(gun.duration, sta_core::seconds_to_ticks(0.1));
        // A negative STA64_duration is ignored: the ordnance's lifespan stands.
        let beam = db.weapon(db.key("beam").unwrap()).unwrap();
        assert_eq!(beam.duration, sta_core::seconds_to_ticks(0.5));
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
    }
}
