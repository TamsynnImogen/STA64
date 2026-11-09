//! `ContentDb::load`: unit and weapon definitions from ODFs (CON-03).
//!
//! Every stem of the [`OdfIndex`] is resolved (layered, through
//! `sta_odf::resolve_layered`) and sorted into one of:
//!
//! - a **weapon** if it sets `wpnName` (cannons, launchers, specials such as
//!   cloaking devices) or its class is one of [`WEAPON_CLASSES`]. Its ordnance
//!   ODF (`ordName`: `phaser`, `photontorpedo`, ...) gives damage and shot
//!   speed ([`crate::weapon`]);
//! - a **unit** if its class is in [`UNIT_CLASSES`] or it sets `unitName`
//!   ([`crate::unit`]);
//! - anything else (ordnance, physics, effects, races, tech trees) is not a
//!   definition and is skipped without a warning.
//!
//! Nothing here panics on data: an ODF that cannot make a definition, a key
//! that does not parse, a reference to an ODF that is not defined, all end up
//! in the returned warnings. Scenery that only the map view reads (stars,
//! nebulae, ...) stays with [`crate::legacy`] until the map loader (CON-07).
//!
//! Only what Milestone 3 needs is typed. The other keys of each class are in
//! `docs/data/odf_keys.csv` (its `sta64` column says which are read) and stay
//! reachable through [`crate::UnitDef::raw`] and [`crate::WeaponDef::raw`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sta_core::Fixed;
use sta_odf::Odf;

use crate::def::DefRef;
use crate::features::{Features, Profile};
use crate::profile::{resolve_features, Overrides};
use crate::index::OdfIndex;
use crate::{ContentDb, Result, Warning};

/// Class labels (lower case) of things that stand in the world and get a
/// [`crate::UnitDef`]: ships, stations, yards, resources.
pub const UNIT_CLASSES: &[&str] = &[
    "craft",
    "shipyard",
    "starbase",
    "constructionrig",
    "freighter",
    "sensor",
    "evolver",
    "repairship",
    "buildyard",
    "wingman",
    "mining",
    "p_mining",
    "cargo_ship",
    "fighter",
    "scavenger",
    "colony",
    "trading_station",
    "minearray",
    "research",
    "pod",
    "upgrade",
    "planet",
    "latinum",
    "scrap",
    "asteroid",
    "asteroidfield",
];

/// The subset of [`UNIT_CLASSES`] that are craft proper (mobile or yard):
/// each must yield a definition or a warning. Install tests hold to that.
pub const CRAFT_CLASSES: &[&str] = &[
    "craft",
    "shipyard",
    "starbase",
    "constructionrig",
    "freighter",
    "sensor",
    "evolver",
    "repairship",
    "buildyard",
    "wingman",
    "mining",
    "p_mining",
    "cargo_ship",
    "fighter",
    "scavenger",
];

/// Class labels of weapons that do not set `wpnName`.
pub const WEAPON_CLASSES: &[&str] = &[
    "cannon",
    "cannonimp",
    "launcher",
    "areacannon",
    "areamissile",
    "toggleweapon",
    "utilityweapon",
    "cloakingdevice",
    "cloakingdeviceimp",
    "ordnancedefenseweapon",
    "pointdefenselaser",
];

/// The name of an ODF as the lookups key it: lower case, no folder, no `.odf`.
pub(crate) fn stem(name: &str) -> String {
    let n = name.trim().trim_matches('"').rsplit(['/', '\\']).next().unwrap_or("").trim().to_ascii_lowercase();
    n.strip_suffix(".odf").map(str::to_owned).unwrap_or(n)
}

/// What an ODF makes, from its resolved keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Unit,
    Weapon,
    Other,
}

pub fn kind_of(odf: &Odf) -> Kind {
    let class = odf.class_label().unwrap_or("").trim().to_ascii_lowercase();
    if odf.raw("wpnName").is_some() || WEAPON_CLASSES.contains(&class.as_str()) {
        Kind::Weapon
    } else if UNIT_CLASSES.contains(&class.as_str()) || odf.raw("unitName").is_some() && !class.is_empty() {
        Kind::Unit
    } else {
        Kind::Other
    }
}

/// Reads typed numbers out of one ODF, listing the keys that do not parse.
pub(crate) struct Reader<'a> {
    pub odf: &'a Odf,
    pub file: &'a str,
    pub warnings: &'a mut Vec<Warning>,
}

/// Larger values are data errors (`1e30`); clamped so `Fixed` cannot overflow.
const LIMIT: f64 = 1.0e9;

impl Reader<'_> {
    /// The key's number, `None` if absent or not a finite number (the latter
    /// with a warning).
    pub fn num(&mut self, key: &str) -> Option<f64> {
        let raw = self.odf.raw(key)?;
        match self.odf.f32(key) {
            Some(v) if v.is_finite() => Some((v as f64).clamp(-LIMIT, LIMIT)),
            _ => {
                if !raw.trim().trim_matches('"').is_empty() {
                    self.warnings.push(Warning::new(self.file, format!("{key} = {raw} is not a number")));
                }
                None
            }
        }
    }

    /// A non-negative quantity, zero when absent or negative.
    pub fn fixed(&mut self, key: &str) -> Fixed {
        Fixed::from_f64(self.num(key).unwrap_or(0.0).max(0.0))
    }

    /// Seconds as ticks.
    pub fn ticks(&mut self, key: &str) -> u64 {
        sta_core::seconds_to_ticks(self.num(key).unwrap_or(0.0).max(0.0))
    }

    pub fn flag(&mut self, key: &str) -> Option<bool> {
        let raw = self.odf.raw(key)?;
        let b = self.odf.bool(key);
        if b.is_none() {
            self.warnings.push(Warning::new(self.file, format!("{key} = {raw} is not a flag")));
        }
        b
    }

    /// Text without quotes; `None` if absent or empty.
    pub fn text(&self, key: &str) -> Option<String> {
        self.odf.str(key).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
    }
}

/// Values of the numbered keys `{prefix}{n}` (case-insensitive), in number
/// order. Raw text; quotes retained.
pub(crate) fn numbered<'a>(odf: &'a Odf, prefix: &str) -> Vec<(u32, &'a str)> {
    let prefix = prefix.to_ascii_lowercase();
    let mut out: Vec<(u32, &str)> = odf
        .keys()
        .filter_map(|k| {
            let l = k.to_ascii_lowercase();
            let n = l.strip_prefix(&prefix)?.parse::<u32>().ok()?;
            Some((n, odf.raw(k)?))
        })
        .collect();
    out.sort_by_key(|(n, _)| *n);
    out
}

/// Words of a value like `"hp01" "hp02"`, quotes removed.
pub(crate) fn words(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(|w| w.trim_matches('"').to_owned()).filter(|w| !w.is_empty()).collect()
}

/// Where definitions' ODFs come from: `(name, version)` to text, as
/// `sta_odf::resolve_layered` asks.
pub type Loader<'a> = dyn FnMut(&str, usize) -> Option<String> + 'a;

/// Build the database from ODFs found by `stems`. [`load`] is this over an
/// install; tests give an in-memory loader.
pub fn load_with(profile: Profile, mod_overrides: &Overrides, stems: &[String], loader: &mut Loader<'_>) -> Result<(ContentDb, Vec<Warning>)> {
    let mut warnings = Vec::new();
    for name in &mod_overrides.unknown {
        warnings.push(Warning::new("info.ini", format!("[Features] {name} is not a feature switch")));
    }
    // Defaults < mod overrides < the ODF's own `Feature_<name>` keys.
    let features_of = |file: &str, odf: &Odf, warnings: &mut Vec<Warning>| -> Features {
        let own = Overrides::from_odf_keys(odf.keys().filter_map(|k| Some((k, odf.raw(k)?))));
        for name in &own.unknown {
            warnings.push(Warning::new(file, format!("Feature_{name} is not a feature switch")));
        }
        resolve_features(profile, &[mod_overrides, &own])
    };
    let mut resolved: BTreeMap<String, Option<Arc<Odf>>> = BTreeMap::new();
    let mut get = |name: &str, warn: &mut Vec<Warning>, loader: &mut Loader<'_>| -> Option<Arc<Odf>> {
        let key = stem(name);
        if let Some(o) = resolved.get(&key) {
            return o.clone();
        }
        let odf = match sta_odf::resolve_layered(&key, loader) {
            Ok(o) => Some(Arc::new(o)),
            Err(e) => {
                warn.push(Warning::new(format!("{key}.odf"), e.to_string()));
                None
            }
        };
        resolved.insert(key, odf.clone());
        odf
    };

    // First pass: classify.
    let mut units: Vec<(String, Arc<Odf>)> = Vec::new();
    let mut weapons: Vec<(String, Arc<Odf>)> = Vec::new();
    for s in stems {
        let Some(odf) = get(s, &mut warnings, loader) else { continue };
        match kind_of(&odf) {
            Kind::Unit => units.push((s.clone(), odf)),
            Kind::Weapon => weapons.push((s.clone(), odf)),
            Kind::Other => {}
        }
    }

    let mut builder = ContentDb::builder().profile(profile).features(resolve_features(profile, &[mod_overrides]));
    let mut weapon_arcs = BTreeMap::new();
    let mut weapon_names = BTreeSet::new();
    for (s, odf) in &weapons {
        let ord = odf.str("ordName").map(str::trim).filter(|o| !o.is_empty()).and_then(|o| {
            let found = get(o, &mut warnings, loader);
            if found.is_none() {
                warnings.push(Warning::new(format!("{s}.odf"), format!("ordName {o} has no ODF")));
            }
            found
        });
        let (mut def, arc) = crate::weapon::weapon_def(s, odf, ord.as_deref(), &mut warnings);
        def.features = Some(features_of(&format!("{s}.odf"), odf, &mut warnings));
        weapon_arcs.insert(stem(s), arc);
        weapon_names.insert(stem(s));
        builder = builder.weapon(def);
    }

    let unit_names: BTreeSet<String> = units.iter().map(|(s, _)| stem(s)).collect();
    for (s, odf) in &units {
        let phys = match odf.str("physicsFile").map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => {
                let found = get(p, &mut warnings, loader);
                if found.is_none() {
                    warnings.push(Warning::new(format!("{s}.odf"), format!("physicsFile {p} has no ODF; the unit does not move")));
                }
                found
            }
            None => None,
        };
        let mut def = crate::unit::unit_def(s, odf, phys.as_deref(), &mut warnings);
        let file = format!("{s}.odf");
        def.features = Some(features_of(&file, odf, &mut warnings));
        // References to ODFs that are not definitions are dropped and listed,
        // so the database always builds.
        def.hardpoints.retain_mut(|h| {
            let w = stem(&h.weapon.odf);
            if !weapon_names.contains(&w) {
                warnings.push(Warning::new(&file, format!("weapon {} is not a defined weapon; hardpoint {} left empty", h.weapon.odf, h.node)));
                return false;
            }
            h.weapon = DefRef::new(w.clone());
            h.arc = weapon_arcs.get(&w).copied().flatten();
            true
        });
        def.build.list.retain_mut(|b| {
            let n = stem(&b.odf);
            if !unit_names.contains(&n) {
                warnings.push(Warning::new(&file, format!("build item {} is not a defined unit; dropped", b.odf)));
                return false;
            }
            *b = DefRef::new(n);
            true
        });
        builder = builder.unit(def);
    }

    // Names are unique by stem; `build` can only fail on a bug here.
    let db = builder.build()?;
    Ok((db, warnings))
}

/// Build the database from every ODF of a mounted install.
pub fn load(vfs: &sta_vfs::Vfs, profile: Profile, index: &OdfIndex) -> Result<(ContentDb, Vec<Warning>)> {
    let stems: Vec<String> = index.iter().map(|(s, _)| s.to_owned()).collect();
    load_with(profile, &Overrides::from_vfs(vfs), &stems, &mut |n, v| index.load(vfs, n, v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::def::WeaponKind;
    use std::collections::HashMap;

    const FIXTURES: &[(&str, &str)] = &[
        (
            "fcruiser",
            "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"Cruiser Class\"\nrace = \"unitedfederation\"\nbasename = \"fed_cruiser\"\nScaleSOD = 1.5\n\
             physicsFile = \"smooth_100.odf\"\nmaxHealth = 4000\nmaxShields = 2000\nshieldRate = 3.0\ncrewCost = 250\nrangeScan = 900\n\
             weapon1 = \"fphaser\"\nweaponHardPoints1 = \"hp01\" \"hp02\"\nweapon2 = \"fcloak\"\nweaponHardPoints2 = \"root\"\nweapon3 = \"ghost\"\nweaponHardPoints3 = \"hp03\"\n\
             dilithiumCost = 600\nlatinumCost = 100\nbiomatterCost = 5\nbuildTime = 30\nofficerCost = 4\n\
             maxSpecialEnergy = 500\nspecialEnergyRate = 10\neventSelect = \"CruiserSelect\"\neventAcknowledge = \"CruiserAck\"\nfireball = \"x_large_explosion\"\n",
        ),
        ("smooth_100", "impulseSpeed = 140.0\nforwardAccel = 2;\nturnOmega = 1.2f\nturnAlpha = 0.8\nturnOmegaFractionAtRest = 0.3\n"),
        (
            "fyard",
            "[GameObjectClass]\nclassLabel = \"shipyard\"\nunitName = \"Yard\"\nmaxHealth = 9000\nbuilditem1 = \"fcruiser\"\nbuilditem2 = \"fcruiser.odf\"\nbuilditem3 = \"fnothing\"\nbuilditem4 = \"NULL\"\n",
        ),
        (
            "fminer",
            "[GameObjectClass]\nclassLabel = \"freighter\"\nunitName = \"Miner\"\nmaxdilithium = 300\nminingRate = 20\ndilithiumTransferRate = 30\nmaxHealth = oops\n",
        ),
        ("moon", "[GameObjectClass]\nclassLabel = \"latinum\"\nunitName = \"Latinum\"\nresource = \"latinum\"\n"),
        ("cloakless", "[GameObjectClass]\nclassLabel = \"craft\"\nphysicsFile = \"missing_phys.odf\"\n#include \"nowhere.odf\"\n"),
        (
            "fphaser",
            "[WeaponClass]\nclassLabel = \"cannon\"\nwpnName = \"Phaser\"\nordName = \"fphaser_ord\"\nrange = 600\nshotDelay = 1.5\nrestrictFireArc = 1\nfireArc = 1.5707963\nfireSound = \"phaser.wav\"\n",
        ),
        ("fphaser_ord", "[OrdnanceClass]\nclassLabel = \"phaser\"\ndamageBase = 12.5\nshotSpeed = 1e6\nlifespan = 0.5\nshotGeometry = \"phaser_beam\"\n"),
        (
            "fcloak",
            "[WeaponClass]\nclassLabel = \"cloakingdeviceimp\"\nwpnName = \"Cloak\"\neffectTimer = 2.0f\nenergyDrainRate = 15\nspecialEnergyCost = 50\ndisableWeapons = 0\ndecloakOnHit = 1\n",
        ),
        (
            "ftorp",
            "[WeaponClass]\nclassLabel = \"launcher\"\nwpnName = \"Torpedo\"\nordName = \"ftorp_ord\"\nrange = 900\nshotDelay = 3\nfireSound = \"torp.wav\"\n",
        ),
        ("ftorp_ord", "classLabel = \"photontorpedo\"\ndamageBase = 60\nshotSpeed = 400\n"),
        ("fpd", "classLabel = \"pointdefenselaser\"\nwpnName = \"PD\"\nrange = 250\nshotDelay = 0.25\n"),
        ("race_fed", "classLabel = \"race\"\n"),
    ];

    fn stems() -> Vec<String> {
        FIXTURES.iter().map(|(n, _)| (*n).to_owned()).collect()
    }

    fn fixture_db() -> (ContentDb, Vec<Warning>) {
        let map: HashMap<String, &str> = FIXTURES.iter().map(|(n, t)| (format!("{n}.odf"), *t)).collect();
        load_with(Profile::Fo4, &Overrides::default(), &stems(), &mut |n, v| {
            let n = if n.ends_with(".odf") { n.to_owned() } else { format!("{n}.odf") };
            if v == 0 { map.get(&n.to_ascii_lowercase()).map(|t| (*t).to_owned()) } else { None }
        })
        .unwrap()
    }

    fn near(a: Fixed, b: f64) -> bool {
        (a.to_f32() as f64 - b).abs() < 1e-4
    }

    #[test]
    fn kinds() {
        let o = |t: &str| sta_odf::resolve("x", &mut |_| Some(t.to_owned())).unwrap();
        assert_eq!(kind_of(&o("classLabel = \"craft\"")), Kind::Unit);
        assert_eq!(kind_of(&o("classLabel = \"Weird\"\nunitName = \"x\"")), Kind::Unit);
        assert_eq!(kind_of(&o("classLabel = \"weird\"\nwpnName = \"x\"")), Kind::Weapon);
        assert_eq!(kind_of(&o("classLabel = \"phaser\"")), Kind::Other);
        assert_eq!(kind_of(&o("impulseSpeed = 1")), Kind::Other);
    }

    #[test]
    fn units_come_out_typed() {
        let (db, _) = fixture_db();
        let c = db.unit(db.key("FCruiser").unwrap()).unwrap();
        assert_eq!((c.class.0.as_str(), c.race.as_str(), c.model.as_str()), ("craft", "unitedfederation", "fed_cruiser"));
        assert!(near(c.scale, 1.5) && near(c.hull, 4000.0) && near(c.shields.max, 2000.0) && near(c.crew, 250.0) && near(c.sensor, 900.0));
        assert!(near(c.shields.recharge, 3.0 / sta_core::TICK_HZ as f64));
        assert_eq!(c.movement, crate::MoveStats::from_f32(140.0, 2.0, 1.2, 0.8, 0.3));
        assert_eq!(c.build.ticks, sta_core::seconds_to_ticks(30.0));
        assert!(near(c.build.officers, 4.0));
        assert_eq!(c.build.cost.iter().map(|f| f.to_f32()).collect::<Vec<_>>(), vec![600.0, 100.0, 0.0, 5.0]);
        assert!(near(c.special_energy_max, 500.0) && near(c.special_energy_rate, 10.0));
        assert_eq!(c.sounds.select, ["CruiserSelect"]);
        assert_eq!(c.sounds.ack_move, ["CruiserAck"]);
        assert_eq!(c.fx.explosion.as_deref(), Some("x_large_explosion"));
        assert_eq!(&*c.raw.name, "fcruiser");
    }

    #[test]
    fn hardpoints_resolve_and_unknown_weapons_are_dropped() {
        let (db, warnings) = fixture_db();
        let c = db.unit(db.key("fcruiser").unwrap()).unwrap();
        assert_eq!(c.hardpoints.len(), 2);
        assert_eq!(c.hardpoints[0].node, "hp01");
        assert_eq!(c.hardpoints[0].weapon.key, db.key("fphaser").unwrap());
        assert_eq!(c.hardpoints[1].node, "root");
        assert!(c.hardpoints[1].arc.is_none());
        let arc = c.hardpoints[0].arc.unwrap();
        assert!(near(arc.min_dot, 0.0) && arc.dir == (Fixed::ZERO, Fixed::ONE));
        assert!(warnings.iter().any(|w| w.file == "fcruiser.odf" && w.message.contains("ghost")), "{warnings:?}");
    }

    #[test]
    fn build_lists_keep_order_and_drop_unknowns() {
        let (db, warnings) = fixture_db();
        let y = db.unit(db.key("fyard").unwrap()).unwrap();
        let c = db.key("fcruiser").unwrap();
        assert_eq!(y.build.list.iter().map(|r| r.key).collect::<Vec<_>>(), vec![c, c]);
        assert!(warnings.iter().any(|w| w.file == "fyard.odf" && w.message.contains("fnothing")));
        assert!(!warnings.iter().any(|w| w.message.contains("NULL")), "NULL is a gap, not a reference... {warnings:?}");
    }

    #[test]
    fn weapons_come_out_typed() {
        let (db, _) = fixture_db();
        let p = db.weapon(db.key("fphaser").unwrap()).unwrap();
        assert_eq!((p.kind, p.class.0.as_str()), (WeaponKind::Beam, "phaser"));
        assert!(near(p.range, 600.0) && near(p.damage, 12.5));
        assert_eq!(p.refire, sta_core::seconds_to_ticks(1.5));
        assert_eq!(p.shot_speed, Fixed::ZERO);
        assert_eq!(p.duration, sta_core::seconds_to_ticks(0.5));
        assert_eq!(p.fire_sound.as_deref(), Some("phaser.wav"));
        assert_eq!(p.fx.as_deref(), Some("phaser_beam"));
        let t = db.weapon(db.key("ftorp").unwrap()).unwrap();
        assert_eq!((t.kind, t.class.0.as_str()), (WeaponKind::Projectile, "photontorpedo"));
        assert!(near(t.shot_speed, 400.0) && near(t.damage, 60.0));
        assert_eq!(db.weapon(db.key("fpd").unwrap()).unwrap().kind, WeaponKind::PointDefence);
    }

    #[test]
    fn specials_and_economy_keys_are_typed() {
        let (db, _) = fixture_db();
        let c = db.weapon(db.key("fcloak").unwrap()).unwrap();
        assert_eq!(c.class.0, "cloakingdeviceimp");
        assert_eq!(c.effect_ticks, sta_core::seconds_to_ticks(2.0));
        assert!(near(c.energy_drain, 15.0) && near(c.energy_cost, 50.0));
        assert!(!c.disable_weapons && c.decloak_on_hit);
        let m = db.unit(db.key("fminer").unwrap()).unwrap();
        assert!(m.miner && near(m.max_dilithium, 300.0) && near(m.mining_rate, 20.0) && near(m.transfer_rate, 30.0));
        assert_eq!(db.unit(db.key("moon").unwrap()).unwrap().resource, Some(1));
        assert_eq!(db.unit(db.key("fcruiser").unwrap()).unwrap().resource, None);
    }

    #[test]
    fn problems_are_warnings_not_panics() {
        let (db, warnings) = fixture_db();
        let text: Vec<String> = warnings.iter().map(|w| w.to_string()).collect();
        assert!(text.iter().any(|w| w.contains("fminer.odf") && w.contains("maxHealth")), "{text:?}");
        assert!(text.iter().any(|w| w.contains("cloakless.odf") && w.contains("nowhere")), "{text:?}");
        // The ODF with the missing include yields no definition, but is listed.
        assert!(db.key("cloakless").is_none());
        // Not definitions, and not warnings.
        assert!(db.key("race_fed").is_none() && db.key("fphaser_ord").is_none());
        assert!(!text.iter().any(|w| w.contains("race_fed") || w.contains("fphaser_ord")));
        assert_eq!(db.units().count(), 4);
        assert_eq!(db.weapons().count(), 4);
    }

    #[test]
    fn the_same_odfs_give_the_same_hash_in_any_order() {
        let (a, _) = fixture_db();
        let mut rev = stems();
        rev.reverse();
        let map: HashMap<String, &str> = FIXTURES.iter().map(|(n, t)| (format!("{n}.odf"), *t)).collect();
        let (b, _) = load_with(Profile::Fo4, &Overrides::default(), &rev, &mut |n, v| {
            let n = if n.ends_with(".odf") { n.to_owned() } else { format!("{n}.odf") };
            if v == 0 { map.get(&n.to_ascii_lowercase()).map(|t| (*t).to_owned()) } else { None }
        })
        .unwrap();
        assert_eq!(a.hash(), b.hash());
    }

    /// Defaults < `info.ini` `[Features]` < the ODF's `Feature_<name>` keys,
    /// per unit and per weapon; the database-wide set ignores the ODF keys.
    #[test]
    fn odf_keys_override_one_defs_feature() {
        let fixtures: &[(&str, &str)] = &[
            ("plain", "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"A\"\n"),
            ("noarcs", "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"B\"\nFeature_fire_arcs = 0\nFeature_bogus = 1\n"),
            ("pd", "[WeaponClass]\nclassLabel = \"cannon\"\nwpnName = \"PD\"\nFeature_a1_compat = 1\nFeature_turrets = 0\n"),
            ("gun", "[WeaponClass]\nclassLabel = \"cannon\"\nwpnName = \"Gun\"\n"),
        ];
        let map: HashMap<String, &str> = fixtures.iter().map(|(n, t)| (format!("{n}.odf"), *t)).collect();
        let stems: Vec<String> = fixtures.iter().map(|(n, _)| (*n).to_owned()).collect();
        let run = |profile: Profile, mod_ov: &Overrides| {
            load_with(profile, mod_ov, &stems, &mut |n, v| {
                let n = if n.ends_with(".odf") { n.to_owned() } else { format!("{n}.odf") };
                if v == 0 { map.get(&n.to_ascii_lowercase()).map(|t| (*t).to_owned()) } else { None }
            })
            .unwrap()
        };
        let (db, warnings) = run(Profile::Native, &Overrides::default());
        assert!(Profile::Native.features().fire_arcs && Profile::Native.features().turrets);
        let plain = db.unit(db.key("plain").unwrap()).unwrap();
        let noarcs = db.unit(db.key("noarcs").unwrap()).unwrap();
        assert!(db.unit_features(plain).fire_arcs);
        assert!(!db.unit_features(noarcs).fire_arcs, "the ODF switches it off for this unit");
        assert!(db.features().fire_arcs, "the database-wide set is untouched");
        let pd = db.weapon(db.key("pd").unwrap()).unwrap();
        let gun = db.weapon(db.key("gun").unwrap()).unwrap();
        assert!(db.weapon_features(pd).a1_compat && !db.features().a1_compat);
        assert!(!db.weapon_features(pd).turrets);
        assert_eq!(db.weapon_features(gun), db.features());
        assert!(warnings.iter().any(|w| w.file == "noarcs.odf" && w.message.contains("bogus")), "{warnings:?}");
        // A mod-level switch sits under the ODF key.
        let mod_ov = Overrides::from_info_ini("[Features]\nfire_arcs = 0\nturrets = 0\n");
        let (db2, _) = run(Profile::Native, &mod_ov);
        assert!(!db2.features().fire_arcs);
        assert!(!db2.unit_features(db2.unit(db2.key("plain").unwrap()).unwrap()).fire_arcs);
        // Overrides change the hash; unset ones do not.
        let (same, _) = run(Profile::Native, &Overrides::default());
        assert_eq!(same.hash(), db.hash());
        assert_ne!(db2.hash(), db.hash());
    }
}
