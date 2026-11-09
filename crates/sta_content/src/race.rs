//! Races and what a seat starts with (INT-M3).
//!
//! `races.odf` lists the race ODFs (`race0 = "federation.odf"`, ...). Each race
//! ODF (read from the installs, Armada II and Fleet Operations alike) holds:
//!
//! - `name`: the race key unit definitions carry (`unitedfederation`);
//! - `instantactionslot`: its place in the skirmish setup, zero or absent when
//!   it is not a playable race;
//! - `interfaceconfiguration`: its GUI cfg (`gui_federation.cfg`), and
//!   `singleplayermusic` / `multiplayermusic`;
//! - `standardunits1..`: what a normal start places (an outpost, the race's
//!   faction object and an effect), `minimalunits1..`: what a minimal start
//!   places (a construction ship), and the stock of a start at the "normal"
//!   and "lots" resource settings (`normaldilithium`, `normallatinum`, ...).
//!
//! [`Races::loadout`] is the one rule for a seat's start: the race's standard
//! units that are craft or stations, plus the first minimal unit when it is
//! the race's own (a builder the player can use at once), and the "normal"
//! stock. Fleet Operations 4.0 starts a race with a stub outpost that builds
//! nothing and names the working one (`originalclass`): that one is placed
//! instead. When no unit can build, the race's [`basic_builder`] comes along. A race the data does
//! not describe, or one whose units are not in the database, falls back to
//! [`default_loadout`]: the race's first shipyard (else starbase) that builds
//! something, and its first miner (else builder), with [`DEFAULT_STOCK`].

use crate::odf::{CRAFT_CLASSES, Loader};
use crate::{ContentDb, ContentKey, Fixed, Warning};

/// Resources per player: dilithium, latinum, metal, biomatter.
pub const RESOURCES: usize = 4;

/// The stock of a fallback start (Fleet Operations' "normal" federation).
pub const DEFAULT_STOCK: [i32; RESOURCES] = [4000, 2000, 0, 500];

/// One race ODF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaceDef {
    /// The ODF's stem (`federation`).
    pub stem: String,
    /// The race key of its units, lower case (`unitedfederation`).
    pub name: String,
    /// `instantactionslot`: `None` when the race is not playable.
    pub slot: Option<i64>,
    /// `interfaceconfiguration` (`gui_federation.cfg`).
    pub interface: Option<String>,
    pub single_player_music: Option<String>,
    pub multiplayer_music: Option<String>,
    /// `standardunits1..`, ODF stems.
    pub standard_units: Vec<String>,
    /// `minimalunits1..`, ODF stems.
    pub minimal_units: Vec<String>,
    /// The "normal" start stock per resource id.
    pub stock: [i64; RESOURCES],
}

/// What a seat starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loadout {
    pub units: Vec<ContentKey>,
    pub stock: [Fixed; RESOURCES],
    /// Where it came from: `race <stem>` or `default`.
    pub source: String,
}

/// Every race of an install.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Races {
    races: Vec<RaceDef>,
}

fn stem(name: &str) -> String {
    crate::odf::stem(name)
}

fn numbered(odf: &sta_odf::Odf, prefix: &str) -> Vec<String> {
    (1..=32).map_while(|i| odf.str(&format!("{prefix}{i}")).map(stem)).filter(|s| !s.is_empty()).collect()
}

impl Races {
    /// The races of a mounted install.
    pub fn load(vfs: &sta_vfs::Vfs, index: &crate::OdfIndex) -> (Races, Vec<Warning>) {
        Races::load_with(&mut |n, v| index.load(vfs, n, v))
    }

    /// The races `races.odf` lists, through a loader (tests give an in-memory one).
    pub fn load_with(loader: &mut Loader<'_>) -> (Races, Vec<Warning>) {
        let mut warnings = Vec::new();
        let list = match sta_odf::resolve_layered("races", loader) {
            Ok(o) => o,
            Err(e) => {
                warnings.push(Warning::new("races.odf", e.to_string()));
                return (Races::default(), warnings);
            }
        };
        let mut races = Vec::new();
        for i in 0..64 {
            let Some(file) = list.str(&format!("race{i}")) else {
                if i >= list.i64("numberofraces").unwrap_or(0) {
                    break;
                }
                continue;
            };
            let s = stem(file);
            match sta_odf::resolve_layered(&s, loader) {
                Ok(o) => {
                    let Some(name) = o.str("name").map(|n| n.trim().to_ascii_lowercase()).filter(|n| !n.is_empty()) else { continue };
                    let text = |k: &str| o.str(k).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
                    let stock = |k: &str| o.i64(&format!("normal{k}")).unwrap_or(0).max(0);
                    races.push(RaceDef {
                        stem: s,
                        name,
                        slot: o.i64("instantactionslot").filter(|&n| n > 0),
                        interface: text("interfaceconfiguration"),
                        single_player_music: text("singleplayermusic"),
                        multiplayer_music: text("multiplayermusic"),
                        standard_units: numbered(&o, "standardunits"),
                        minimal_units: numbered(&o, "minimalunits"),
                        stock: [stock("dilithium"), stock("latinum"), stock("metal"), stock("biomatter")],
                    });
                }
                Err(e) => warnings.push(Warning::new(format!("{s}.odf"), e.to_string())),
            }
        }
        (Races { races }, warnings)
    }

    pub fn from_defs(races: Vec<RaceDef>) -> Races {
        Races { races }
    }

    pub fn is_empty(&self) -> bool {
        self.races.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RaceDef> {
        self.races.iter()
    }

    /// A race by its ODF stem (`federation`) or key (`unitedfederation`), any case.
    pub fn get(&self, name: &str) -> Option<&RaceDef> {
        let n = name.trim().to_ascii_lowercase();
        // The ODF's stem first (`observer` and `federation` share the key `federation`),
        // then the playable race of that key, then any.
        self.races
            .iter()
            .find(|r| r.stem == n)
            .or_else(|| self.races.iter().find(|r| r.name == n && r.slot.is_some()))
            .or_else(|| self.races.iter().find(|r| r.name == n))
    }

    /// The playable races in setup order (`instantactionslot`).
    pub fn playable(&self) -> Vec<&RaceDef> {
        let mut v: Vec<&RaceDef> = self.races.iter().filter(|r| r.slot.is_some()).collect();
        v.sort_by_key(|r| (r.slot, r.stem.clone()));
        v
    }

    /// What a seat of `race` (a unit race key) starts with.
    pub fn loadout(&self, db: &ContentDb, race: &str) -> Loadout {
        if let Some(r) = self.get(race) {
            let is_craft = |k: ContentKey| db.unit(k).is_some_and(|u| CRAFT_CLASSES.contains(&u.class.0.as_str()));
            let mut units: Vec<ContentKey> = Vec::new();
            // The standard units, then the first minimal unit when it is the race's own
            // (Fleet Operations 4.0 names `fconst` for every race: not a builder of theirs).
            let own = |k: ContentKey| db.unit(k).is_some_and(|u| u.race.eq_ignore_ascii_case(&r.name));
            let minimal = r.minimal_units.iter().take(1).filter(|s| db.key(s).is_some_and(|k| own(k) && can_build(db, k, &r.name)));
            for s in r.standard_units.iter().chain(minimal) {
                let Some(mut k) = db.key(s).filter(|&k| is_craft(k)) else { continue };
                // Fleet Operations 4.0's start outpost is a stub (`fed_outpostS`) that names
                // the working one it stands for (`originalclass = "fed_outpostY"`).
                if !can_build(db, k, &r.name) {
                    let original = db.unit(k).and_then(|u| u.raw.str("originalclass")).map(crate::odf::stem);
                    if let Some(o) = original.and_then(|o| db.key(&o)).filter(|&o| can_build(db, o, &r.name)) {
                        k = o;
                    }
                }
                if !units.contains(&k) {
                    units.push(k);
                }
            }
            // A seat that cannot build at all (the stub names no working station) gets
            // its race's basic construction ship.
            let builds = |k: &ContentKey| can_build(db, *k, &r.name);
            if !units.is_empty() && !units.iter().any(builds) {
                units.extend(basic_builder(db, &r.name));
            }
            if !units.is_empty() {
                return Loadout { units, stock: r.stock.map(|n| Fixed::from_int(n.clamp(0, 1_000_000_000) as i32)), source: format!("race {}", r.stem) };
            }
        }
        default_loadout(db, race)
    }
}

/// Whether a seat of `race` owning `key` can build something at once: it has a
/// build list and, with a tech tree, at least one entry of it is available.
pub fn can_build(db: &ContentDb, key: ContentKey, race: &str) -> bool {
    let Some(u) = db.unit(key).filter(|u| !u.build.list.is_empty()) else { return false };
    match db.tech_tree() {
        Some(tree) => {
            let own = crate::Owned::new(race).with(&u.odf, 1);
            tree.build_options(db, &u.odf, &own).iter().any(|o| o.availability.is_available())
        }
        None => true,
    }
}

/// The race's basic construction ship: of its `constructionrig`s, those the
/// tech tree (when the content has one) lets build something from the start,
/// the one with the shortest build list, by ODF name. Fleet Operations 4.0's
/// `fconst` is not in its tree; its Mediterranean class is the one that builds
/// the outposts.
pub fn basic_builder(db: &ContentDb, race: &str) -> Option<ContentKey> {
    let race = race.trim().to_ascii_lowercase();
    db.units()
        .filter(|u| u.race.eq_ignore_ascii_case(&race) && u.class.0 == "constructionrig" && can_build(db, u.key, &race))
        .min_by(|a, b| (a.build.list.len(), &a.odf).cmp(&(b.build.list.len(), &b.odf)))
        .map(|u| u.key)
}

/// The fallback start of `race`: its first shipyard (else starbase) that builds
/// something, and its first miner (else mobile builder), by ODF name, with
/// [`DEFAULT_STOCK`]. Empty when the race has neither.
pub fn default_loadout(db: &ContentDb, race: &str) -> Loadout {
    let race = race.trim().to_ascii_lowercase();
    let mut of_race: Vec<&crate::UnitDef> = db.units().filter(|u| u.race.to_ascii_lowercase() == race).collect();
    of_race.sort_by(|a, b| a.odf.cmp(&b.odf));
    let mut units = Vec::new();
    let builds = |u: &&&crate::UnitDef| !u.build.list.is_empty();
    let mobile = |u: &&&crate::UnitDef| u.movement.max_speed > Fixed::ZERO;
    for class in ["shipyard", "starbase"] {
        if let Some(u) = of_race.iter().filter(builds).find(|u| u.class.0 == class && !mobile(u)) {
            units.push(u.key);
            break;
        }
    }
    let miner = of_race.iter().find(|u| (u.miner || u.class.0 == "freighter") && mobile(u)).or_else(|| of_race.iter().filter(builds).find(|u| mobile(u)));
    if let Some(u) = miner {
        units.push(u.key);
    }
    Loadout { units, stock: DEFAULT_STOCK.map(Fixed::from_int), source: "default".into() }
}

/// The races a skirmish can be played with, in setup order: the playable races
/// (`instantactionslot`) of the install that start with something; when the data
/// names none (a bare database), every race of the units that does.
pub fn skirmish_races(races: &Races, db: &ContentDb) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in races.playable() {
        if !out.contains(&r.name) && !races.loadout(db, &r.name).units.is_empty() {
            out.push(r.name.clone());
        }
    }
    if out.is_empty() {
        let mut all: Vec<String> = db.units().map(|u| u.race.to_ascii_lowercase()).filter(|r| !r.is_empty()).collect();
        all.sort();
        all.dedup();
        out = all.into_iter().filter(|r| !races.loadout(db, r).units.is_empty()).collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn loader(files: &'static [(&'static str, &'static str)]) -> impl FnMut(&str, usize) -> Option<String> {
        let map: HashMap<String, String> = files.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect();
        move |n, v| if v == 0 { map.get(n.to_ascii_lowercase().trim_end_matches(".odf")).cloned() } else { None }
    }

    const FILES: &[(&str, &str)] = &[
        ("races", "numberofraces = 3\nrace0 = \"norace.odf\"\nrace1 = \"federation.odf\"\nrace2 = \"klingon.odf\"\n"),
        ("norace", "name = \"norace\"\n"),
        (
            "federation",
            "name = \"unitedfederation\"\ninstantactionslot = 2\ninterfaceconfiguration = \"gui_federation.cfg\"\nmultiplayermusic = \"Federation.mp3\"\n\
             minimalunits1 = \"fx_miner.odf\"\nstandardunits1 = \"fx_yard.odf\"\nstandardunits2 = \"fx_effects.odf\"\nstandardunits3 = \"fx_station.odf\"\n\
             normaldilithium = 4000\nnormallatinum = 2000\nnormalmetal = 0\nnormalbiomatter = 500\n",
        ),
        ("klingon", "name = \"klingon\"\ninstantactionslot = 3\n"),
    ];

    fn races() -> Races {
        let (r, w) = Races::load_with(&mut loader(FILES));
        assert!(w.is_empty(), "{w:?}");
        r
    }

    #[test]
    fn reads_races_and_start_keys() {
        let r = races();
        assert_eq!(r.iter().count(), 3);
        let f = r.get("UnitedFederation").unwrap();
        assert_eq!((f.stem.as_str(), f.slot), ("federation", Some(2)));
        assert_eq!(r.get("federation"), Some(f));
        assert_eq!(f.interface.as_deref(), Some("gui_federation.cfg"));
        assert_eq!(f.multiplayer_music.as_deref(), Some("Federation.mp3"));
        assert_eq!(f.standard_units, ["fx_yard", "fx_effects", "fx_station"]);
        assert_eq!(f.minimal_units, ["fx_miner"]);
        assert_eq!(f.stock, [4000, 2000, 0, 500]);
        assert_eq!(r.get("norace").unwrap().slot, None);
        let slots: Vec<_> = r.playable().iter().map(|r| r.stem.as_str()).collect();
        assert_eq!(slots, ["federation", "klingon"]);
    }

    #[test]
    fn loadout_takes_craft_and_stations_of_the_data() {
        // The fake content stands in: `fx_effects` is not defined, so it is skipped.
        let db = crate::fake::db();
        let mut def = races().get("federation").cloned().unwrap();
        def.name = "fx".into();
        // `fx_miner` is `fx`'s own, so it comes with the standard units.
        let r = Races::from_defs(vec![def]);
        let l = r.loadout(&db, "fx");
        let names: Vec<_> = l.units.iter().map(|&k| db.name(k).unwrap()).collect();
        // `fx_miner` builds nothing, so the minimal unit is not taken.
        assert_eq!(names, ["fx_yard", "fx_station"]);
        assert_eq!(l.source, "race federation");
        assert_eq!(l.stock[0], Fixed::from_int(4000));
    }

    #[test]
    fn a_stub_outpost_is_replaced_by_the_station_it_names() {
        // Fleet Operations 4.0: `fed_outpostS` builds nothing and says `originalclass = "fed_outpostY"`.
        let base = crate::fake::db();
        let mut stub = crate::UnitDef::new("fx_outposts", "shipyard");
        stub.race = "fx".into();
        stub.raw = std::sync::Arc::new(sta_odf::resolve("fx_outposts", &mut |_| Some("originalclass = \"fx_yard\"\n".into())).unwrap());
        let mut b = crate::ContentDb::builder().profile(base.profile()).features(base.features().clone());
        for w in base.weapons() {
            b = b.weapon(w.clone());
        }
        for u in base.units() {
            b = b.unit(u.clone());
        }
        let db = b.unit(stub).build().unwrap();
        let mut def = races().get("federation").cloned().unwrap();
        def.name = "fx".into();
        def.standard_units = vec!["fx_outposts".into()];
        def.minimal_units.clear();
        let l = Races::from_defs(vec![def]).loadout(&db, "fx");
        let names: Vec<_> = l.units.iter().map(|&k| db.name(k).unwrap()).collect();
        assert_eq!(names, ["fx_yard"]);
    }

    #[test]
    fn a_race_without_data_gets_the_documented_default() {
        let db = crate::fake::db();
        let l = Races::default().loadout(&db, "fx");
        let names: Vec<_> = l.units.iter().map(|&k| db.name(k).unwrap()).collect();
        assert_eq!(names, ["fx_yard", "fx_miner"]);
        assert_eq!(l.source, "default");
        assert_eq!(l.stock, DEFAULT_STOCK.map(Fixed::from_int));
        assert!(Races::default().loadout(&db, "nobody").units.is_empty());
    }
}
