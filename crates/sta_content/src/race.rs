//! Races and what a seat starts with (INT-M3).
//!
//! `races.odf` lists the race ODFs (`race0 = "federation.odf"`, ...). Each race
//! ODF (read from the installs, Armada II and Fleet Operations alike) holds:
//!
//! - `name`: the race key unit definitions carry (`unitedfederation`);
//! - `instantactionslot`: its nonnegative place in the skirmish setup, including
//!   zero; absent or invalid slots do not make a modern race playable;
//! - `interfaceconfiguration`: its GUI cfg (`gui_federation.cfg`), and
//!   `singleplayermusic` / `multiplayermusic`;
//! - `standardunits1..`: what a normal start places (an outpost, the race's
//!   faction object and an effect), `minimalunits1..`: what a minimal start
//!   places (a construction ship), and the stock of a start at the "normal"
//!   and "lots" resource settings (`normaldilithium`, `normallatinum`, ...).
//!
//! [`Races::loadout`] uses only a known race's resolved `standardunits` in
//! authored numeric order, including duplicates, and its "normal" stock.
//! Unresolved entries are skipped; an empty authored start stays empty.
//! Unknown races use [`default_loadout`], the synthetic bare-content fallback.

use crate::odf::Loader;
use crate::{ContentDb, ContentKey, Fixed, Profile, Warning};

/// Resources per player: dilithium, latinum, metal, biomatter.
pub const RESOURCES: usize = 4;

/// The stock of a fallback start (Fleet Operations' "normal" federation).
pub const DEFAULT_STOCK: [i32; RESOURCES] = [4000, 2000, 0, 500];

/// The authored starting-resource choice for a skirmish seat.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StartingResources {
    #[default]
    Normal,
    Lots,
}

/// One race ODF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaceDef {
    /// The ODF's stem (`federation`).
    pub stem: String,
    /// The race key of its units, lower case (`unitedfederation`).
    pub name: String,
    /// Authored localization key or literal caption; never a simulation race ID.
    pub display_key: Option<String>,
    /// `instantactionslot`: `None` when the race is not playable.
    pub slot: Option<i64>,
    /// `interfaceconfiguration` (`gui_federation.cfg`).
    pub interface: Option<String>,
    /// Authored `interfaceSprites` GUI table, independent of the layout cfg.
    pub interface_sprites: Option<String>,
    pub single_player_music: Option<String>,
    pub multiplayer_music: Option<String>,
    /// `standardunits1..`, ODF stems.
    pub standard_units: Vec<String>,
    /// `minimalunits1..`, ODF stems.
    pub minimal_units: Vec<String>,
    /// The "normal" start stock per resource id.
    pub stock: [i64; RESOURCES],
    /// The "lots" start stock per resource id.
    pub lots_stock: [i64; RESOURCES],
    /// Authored crew stock, absent when the ODF does not configure it.
    pub normal_crew: Option<i64>,
    pub lots_crew: Option<i64>,
}

/// What a seat starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loadout {
    pub units: Vec<ContentKey>,
    pub stock: [Fixed; RESOURCES],
    pub crew: Option<Fixed>,
    /// Where it came from: `race <stem>` or `default`.
    pub source: String,
}

/// The crew keys of a race ODF: crew added to the team per second per
/// starbase, and the modifier near a planet (read; the sim does not apply it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaceCrew {
    pub accumulation_rate: Fixed,
    pub planet_modifier: Fixed,
}

/// What a team's race changes about the team itself (SIM-54), read from the
/// race ODF when it differs from the identity:
///
/// - `maxOfficerMultiplier` (float, default 1): the officer cap is
///   `floor(multiplier * limit)`. Read from the executable (static) and from
///   data: only `species8472` sets it (0.24) in Armada II stock and Classic.
/// - `latinumCostModifier` (float, a proposed STA64 key, 1 when neither it
///   nor the derivation applies): scales the latinum part of what the team
///   pays for a build. The original compares the team race's name with
///   `borg` in the engine and charges no latinum; to keep that without a name
///   in code the loader derives 0 for a race named `borg` when the key is
///   absent (every profile except Armada I, where it was not traced). An
///   authored key, including `1`, replaces the derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatRaceRule {
    pub officer_multiplier: Fixed,
    pub latinum_scale: Fixed,
}

impl Default for SeatRaceRule {
    fn default() -> Self {
        SeatRaceRule { officer_multiplier: Fixed::ONE, latinum_scale: Fixed::ONE }
    }
}

/// Every race of an install.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Races {
    races: Vec<RaceDef>,
    /// `crewAccumulationRate` / `planetCrewAccumulationModifier` per race
    /// key (SIM-25), filled by the loader only.
    crew: Vec<(String, RaceCrew)>,
    /// The races whose [`SeatRaceRule`] is not the identity, by key, in race order.
    seat: Vec<(String, SeatRaceRule)>,
    /// Legacy profiles and loaded catalogues are authoritative. Only a bare
    /// native/fake database may infer its choices from unit definitions.
    authoritative: bool,
}

fn stem(name: &str) -> String {
    crate::odf::stem(name)
}

fn numbered(odf: &sta_odf::Odf, prefix: &str) -> Vec<String> {
    crate::odf::numbered(odf, prefix)
        .into_iter()
        .filter(|(i, _)| *i > 0)
        .map(|(_, raw)| stem(raw))
        .filter(|s| !s.is_empty())
        .collect()
}

fn nonnegative_integer(odf: &sta_odf::Odf, key: &str) -> Option<i64> {
    let token = odf.str(key)?.split_whitespace().next()?
        .trim_end_matches(';').trim_end_matches(['f', 'F']).trim_end_matches(';');
    token.parse::<i64>().ok().or_else(|| {
        let value = token.parse::<f64>().ok()?;
        (value.is_finite() && value >= 0.0 && value < i64::MAX as f64 && value.fract() == 0.0)
            .then_some(value as i64)
    }).filter(|&slot| slot >= 0)
}

impl Races {
    /// The races of a mounted install.
    pub fn load(vfs: &sta_vfs::Vfs, index: &crate::OdfIndex) -> (Races, Vec<Warning>) {
        let (mut races, warnings) = Races::load_with_profile(Profile::detect(vfs), &mut |n, v| index.load(vfs, n, v));
        // An indexed but unreadable catalogue is not an intentionally bare DB.
        races.authoritative |= index.get("races").is_some();
        (races, warnings)
    }

    /// The races `races.odf` lists, through a loader (tests give an in-memory one).
    pub fn load_with(loader: &mut Loader<'_>) -> (Races, Vec<Warning>) {
        Self::load_with_profile(Profile::Native, loader)
    }

    /// Apply profile-specific race-menu defaults after ordinary layered ODF resolution.
    pub fn load_with_profile(profile: Profile, loader: &mut Loader<'_>) -> (Races, Vec<Warning>) {
        let mut warnings = Vec::new();
        let root = loader("races.odf", 0);
        let authoritative = root.is_some() || profile != Profile::Native;
        let list = match sta_odf::resolve_layered("races", &mut |name, version| {
            if version == 0 && stem(name) == "races" {
                root.clone()
            } else {
                loader(name, version)
            }
        }) {
            Ok(o) => o,
            Err(e) => {
                warnings.push(Warning::new("races.odf", e.to_string()));
                return (Races { races: Vec::new(), crew: Vec::new(), seat: Vec::new(), authoritative }, warnings);
            }
        };
        let declared = nonnegative_integer(&list, "numberofraces");
        if list.raw("numberofraces").is_some() && declared.is_none() {
            warnings.push(Warning::new("races.odf", "numberOfRaces must be a nonnegative integer"));
            return (Races { races: Vec::new(), crew: Vec::new(), seat: Vec::new(), authoritative }, warnings);
        }
        // Walk authored keys rather than an arbitrary fixed cap or a potentially
        // enormous declared count. Inherited trailing keys are inactive when a
        // child lowers numberOfRaces.
        let mut entries: Vec<u64> = list.keys().filter_map(|key| {
            let key = key.to_ascii_lowercase();
            let index = key.strip_prefix("race")?.parse::<u64>().ok()?;
            (key == format!("race{index}") && declared.is_none_or(|count| index < count as u64))
                .then_some(index)
        }).collect();
        entries.sort_unstable();
        let mut races = Vec::new();
        let mut interfaces = Vec::new();
        let mut crew_rates: Vec<(String, RaceCrew)> = Vec::new();
        let mut seat_rules: Vec<(String, SeatRaceRule)> = Vec::new();
        for i in entries {
            let Some(file) = list.str(&format!("race{i}")) else {
                continue;
            };
            let s = stem(file);
            match sta_odf::resolve_layered(&s, loader) {
                Ok(o) => {
                    let Some(name) = o
                        .str("name")
                        .map(|n| n.trim().to_ascii_lowercase())
                        .filter(|n| !n.is_empty())
                    else {
                        warnings.push(Warning::new(format!("{s}.odf"), "race has no name"));
                        continue;
                    };
                    let text = |k: &str| {
                        o.str(k)
                            .map(str::trim)
                            .filter(|v| !v.is_empty())
                            .map(str::to_owned)
                    };
                    let stock = |mode: &str, k: &str| o.i64(&format!("{mode}{k}")).unwrap_or(0).max(0);
                    let mut crew = |mode: &str| {
                        let key = format!("{mode}crew");
                        let value = nonnegative_integer(&o, &key);
                        if o.raw(&key).is_some() && value.is_none() {
                            warnings.push(Warning::new(format!("{s}.odf"), format!("{key} must be a nonnegative integer")));
                        }
                        value
                    };
                    let normal_crew = crew("normal");
                    let lots_crew = crew("lots");
                    let slot = nonnegative_integer(&o, "instantactionslot");
                    if o.raw("instantactionslot").is_some() && slot.is_none() {
                        warnings.push(Warning::new(format!("{s}.odf"), "instantActionSlot must be a nonnegative integer"));
                    }
                    interfaces.push(o.raw("interfaceconfiguration").is_some());
                    if o.raw("crewaccumulationrate").is_some() {
                        let num = |k: &str, default: f32| o.f32(k).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(default);
                        crew_rates.push((
                            name.clone(),
                            RaceCrew {
                                accumulation_rate: Fixed::from_f64(f64::from(num("crewaccumulationrate", 0.0)).min(1.0e6)),
                                planet_modifier: Fixed::from_f64(f64::from(num("planetcrewaccumulationmodifier", 1.0)).min(1.0e3)),
                            },
                        ));
                    }
                    let factor = |k: &str| o.f32(k).filter(|v| v.is_finite() && *v >= 0.0).map(|v| Fixed::from_f64(f64::from(v).min(1.0e3)));
                    let rule = SeatRaceRule {
                        officer_multiplier: factor("maxofficermultiplier").unwrap_or(Fixed::ONE),
                        latinum_scale: factor("latinumcostmodifier").unwrap_or(if name == "borg" && profile != Profile::Sta1Classic {
                            Fixed::ZERO
                        } else {
                            Fixed::ONE
                        }),
                    };
                    if rule != SeatRaceRule::default() {
                        seat_rules.push((name.clone(), rule));
                    }
                    races.push(RaceDef {
                        stem: s,
                        name,
                        display_key: text("displaykey"),
                        slot,
                        interface: text("interfaceconfiguration"),
                        interface_sprites: text("interfacesprites"),
                        single_player_music: text("singleplayermusic"),
                        multiplayer_music: text("multiplayermusic"),
                        standard_units: numbered(&o, "standardunits"),
                        minimal_units: numbered(&o, "minimalunits"),
                        stock: [
                            stock("normal", "dilithium"),
                            stock("normal", "latinum"),
                            stock("normal", "metal"),
                            stock("normal", "biomatter"),
                        ],
                        lots_stock: [
                            stock("lots", "dilithium"),
                            stock("lots", "latinum"),
                            stock("lots", "metal"),
                            stock("lots", "biomatter"),
                        ],
                        normal_crew,
                        lots_crew,
                    });
                }
                Err(e) => warnings.push(Warning::new(format!("{s}.odf"), e.to_string())),
            }
        }
        if profile == Profile::Sta1Classic {
            // A2FOExtensions config.rs::a2fo_rs_config_race: preserve a fully
            // valid explicit table; otherwise derive A1's legacy menu from
            // interfaceConfiguration presence, in race declaration order.
            let count = declared.unwrap_or(races.len() as i64);
            let mut occupied = std::collections::BTreeSet::new();
            let valid = races.iter().all(|r| r.slot.is_some_and(|s| s < count && occupied.insert(s)));
            if !valid {
                let mut next = 0;
                for (race, has_interface) in races.iter_mut().zip(interfaces) {
                    race.slot = has_interface.then(|| {
                        let slot = next;
                        next += 1;
                        slot
                    });
                }
            }
        }
        (Races { races, crew: crew_rates, seat: seat_rules, authoritative }, warnings)
    }

    pub fn from_defs(races: Vec<RaceDef>) -> Races {
        Races { races, crew: Vec::new(), seat: Vec::new(), authoritative: true }
    }

    /// The crew keys of a race by its key (`unitedfederation`), any case.
    pub fn crew(&self, name: &str) -> Option<RaceCrew> {
        let n = name.trim().to_ascii_lowercase();
        self.crew.iter().find(|(k, _)| *k == n).map(|(_, c)| *c)
    }

    /// The [`SeatRaceRule`]s that are not the identity, in race order.
    pub fn seat_rules(&self) -> impl Iterator<Item = (&str, SeatRaceRule)> {
        self.seat.iter().map(|(k, r)| (k.as_str(), *r))
    }

    /// Every race's crew keys, in race order.
    pub fn crew_rates(&self) -> impl Iterator<Item = (&str, RaceCrew)> {
        self.crew.iter().map(|(k, c)| (k.as_str(), *c))
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
            .or_else(|| self.races.iter().find(|r| r.name == n && r.slot.is_some_and(|slot| slot >= 0)))
            .or_else(|| self.races.iter().find(|r| r.name == n))
    }

    /// The playable races in setup order (`instantactionslot`).
    pub fn playable(&self) -> Vec<&RaceDef> {
        let mut v: Vec<&RaceDef> = self.races.iter().filter(|r| r.slot.is_some_and(|slot| slot >= 0)).collect();
        v.sort_by_key(|r| (r.slot, r.stem.clone()));
        v
    }

    /// A playable unit race key, in setup order. Unlike `get`, an unrelated
    /// ODF stem cannot shadow a key emitted by the player race picker.
    pub fn get_playable(&self, key: &str) -> Option<&RaceDef> {
        let key = key.trim();
        self.playable().into_iter().find(|race| race.name.eq_ignore_ascii_case(key))
    }

    /// What a seat of `race` (a unit race key) starts with.
    pub fn loadout(&self, db: &ContentDb, race: &str) -> Loadout {
        self.loadout_with_mode(db, race, StartingResources::Normal)
    }

    /// What a seat starts with under the selected resource mode.
    pub fn loadout_with_mode(&self, db: &ContentDb, race: &str, mode: StartingResources) -> Loadout {
        if let Some(r) = self.get_playable(race).or_else(|| self.get(race)) {
            let units = r.standard_units.iter().filter_map(|s| {
                db.key(s).filter(|&k| db.unit(k).is_some())
            }).collect();
            let (stock, crew) = match mode {
                StartingResources::Normal => (r.stock, r.normal_crew),
                StartingResources::Lots => (r.lots_stock, r.lots_crew),
            };
            return Loadout {
                units,
                stock: stock.map(|n| Fixed::from_int(n.clamp(0, 1_000_000_000) as i32)),
                crew: crew.map(|n| Fixed::from_int(n.clamp(0, 1_000_000_000) as i32)),
                source: format!("race {}", r.stem),
            };
        }
        default_loadout(db, race)
    }
}

/// Whether a seat of `race` owning `key` can build something at once: it has a
/// build list and, with a tech tree, at least one entry of it is available.
pub fn can_build(db: &ContentDb, key: ContentKey, race: &str) -> bool {
    let Some(u) = db.unit(key).filter(|u| !u.build.list.is_empty()) else {
        return false;
    };
    match db.tech_tree() {
        Some(tree) => {
            let own = crate::Owned::new(race).with(&u.odf, 1);
            tree.build_options(db, &u.odf, &own, false)
                .iter()
                .any(|o| o.availability.is_available())
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
        .filter(|u| {
            u.race.eq_ignore_ascii_case(&race)
                && u.class.0 == "constructionrig"
                && can_build(db, u.key, &race)
        })
        .min_by(|a, b| (a.build.list.len(), &a.odf).cmp(&(b.build.list.len(), &b.odf)))
        .map(|u| u.key)
}

/// The fallback start of `race`: its first shipyard (else starbase) that builds
/// something, and its first miner (else mobile builder), by ODF name, with
/// [`DEFAULT_STOCK`]. Empty when the race has neither.
pub fn default_loadout(db: &ContentDb, race: &str) -> Loadout {
    let race = race.trim().to_ascii_lowercase();
    let mut of_race: Vec<&crate::UnitDef> = db
        .units()
        .filter(|u| u.race.to_ascii_lowercase() == race)
        .collect();
    of_race.sort_by(|a, b| a.odf.cmp(&b.odf));
    let mut units = Vec::new();
    let builds = |u: &&&crate::UnitDef| !u.build.list.is_empty();
    let mobile = |u: &&&crate::UnitDef| u.movement.max_speed > Fixed::ZERO;
    for class in ["shipyard", "starbase"] {
        if let Some(u) = of_race
            .iter()
            .filter(builds)
            .find(|u| u.class.0 == class && !mobile(u))
        {
            units.push(u.key);
            break;
        }
    }
    let miner = of_race
        .iter()
        .find(|u| u.mines() && mobile(u))
        .or_else(|| of_race.iter().filter(builds).find(|u| mobile(u)));
    if let Some(u) = miner {
        units.push(u.key);
    }
    Loadout {
        units,
        stock: DEFAULT_STOCK.map(Fixed::from_int),
        crew: None,
        source: "default".into(),
    }
}

/// The races a skirmish can be played with, in setup order: the playable races
/// (`instantactionslot`) of the install, independent of start-unit availability.
/// Only a bare database without a race catalogue may infer races from its units.
pub fn skirmish_races(races: &Races, db: &ContentDb) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in races.playable() {
        if !out.contains(&r.name) {
            out.push(r.name.clone());
        }
    }
    if !races.authoritative {
        let mut all: Vec<String> = db
            .units()
            .map(|u| u.race.to_ascii_lowercase())
            .filter(|r| !r.is_empty())
            .collect();
        all.sort();
        all.dedup();
        out = all
            .into_iter()
            .filter(|r| !races.loadout(db, r).units.is_empty())
            .collect();
    }
    out
}

/// The race a player's forces belong to: the `race` key that most of `unit_races` share
/// (empty values are ignored; a tie goes to the one seen first).
///
/// A start loadout is not all of one race: Roots' `standardUnits` begin with the shared
/// helper units `all_ai_pod` (race `borg`) and `all_start` (race `species8472`) for every
/// race, so "the race of the first unit" names Borg for a Federation player. What the player
/// chose is what its units mostly are, and that is read from their ODFs.
pub fn dominant_race<'a>(unit_races: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for r in unit_races.into_iter().filter(|r| !r.trim().is_empty()) {
        match counts.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(r)) {
            Some((_, n)) => *n += 1,
            None => counts.push((r, 1)),
        }
    }
    let mut best: Option<(&str, usize)> = None;
    for (r, n) in counts {
        if best.is_none_or(|(_, b)| n > b) {
            best = Some((r, n));
        }
    }
    best.map(|(r, _)| r)
}

#[cfg(test)]
#[path = "race/eligibility_tests.rs"]
mod eligibility_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn loader(
        files: &'static [(&'static str, &'static str)],
    ) -> impl FnMut(&str, usize) -> Option<String> {
        let map: HashMap<String, String> = files
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect();
        move |n, v| {
            if v == 0 {
                map.get(n.to_ascii_lowercase().trim_end_matches(".odf"))
                    .cloned()
            } else {
                None
            }
        }
    }

    const FILES: &[(&str, &str)] = &[
        (
            "races",
            "numberofraces = 3\nrace0 = \"norace.odf\"\nrace1 = \"federation.odf\"\nrace2 = \"klingon.odf\"\n",
        ),
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
    fn interface_sprite_table_is_authored_and_inherited_independently() {
        let (races, warnings) = Races::load_with(&mut loader(&[
            ("races", "race0=child\nrace1=blank\nrace2=absent"),
            ("base", "name=fx\ninterfaceConfiguration=layout.cfg\ninterfaceSprites=authored.spr"),
            ("child", "#include \"base.odf\"\ninstantActionSlot=0"),
            ("blank", "name=blank\ninterfaceSprites=\" \""),
            ("absent", "name=absent\ninterfaceConfiguration=gui_wrong.cfg"),
        ]));
        assert!(warnings.is_empty(), "{warnings:?}");
        let child = races.get("child").unwrap();
        assert_eq!(child.interface.as_deref(), Some("layout.cfg"));
        assert_eq!(child.interface_sprites.as_deref(), Some("authored.spr"));
        assert_eq!(races.get("blank").unwrap().interface_sprites, None);
        assert_eq!(races.get("absent").unwrap().interface_sprites, None);
    }

    #[test]
    fn display_key_preserves_authored_case_literal_and_inheritance() {
        let (races, warnings) = Races::load_with(&mut loader(&[
            ("races", "race0=child\nrace1=literal\nrace2=blank\nrace3=absent"),
            ("parent", "name=internal_fed\ndisplayKey=GUI_RD_FEDERATION"),
            ("child", "#include \"parent.odf\"\ninstantActionSlot=0"),
            ("literal", "name=internal_borg\ndisplayKey=\"Borg Collective\"\ninstantActionSlot=1"),
            ("blank", "name=blank\ndisplayKey=\" \""),
            ("absent", "name=absent"),
        ]));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(races.get("child").unwrap().display_key.as_deref(), Some("GUI_RD_FEDERATION"));
        assert_eq!(races.get("literal").unwrap().display_key.as_deref(), Some("Borg Collective"));
        assert_eq!(races.get("blank").unwrap().display_key, None);
        assert_eq!(races.get("absent").unwrap().display_key, None);
        assert_eq!(races.playable().iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["internal_fed", "internal_borg"]);
    }

    #[test]
    fn reads_races_and_start_keys() {
        let r = races();
        assert_eq!(r.iter().count(), 3);
        let f = r.get("UnitedFederation").unwrap();
        assert_eq!((f.stem.as_str(), f.slot), ("federation", Some(2)));
        assert_eq!(r.get("federation"), Some(f));
        assert_eq!(f.interface.as_deref(), Some("gui_federation.cfg"));
        assert_eq!(f.interface_sprites, None);
        assert_eq!(f.multiplayer_music.as_deref(), Some("Federation.mp3"));
        assert_eq!(f.standard_units, ["fx_yard", "fx_effects", "fx_station"]);
        assert_eq!(f.minimal_units, ["fx_miner"]);
        assert_eq!(f.stock, [4000, 2000, 0, 500]);
        assert_eq!(r.get("norace").unwrap().slot, None);
        let slots: Vec<_> = r.playable().iter().map(|r| r.stem.as_str()).collect();
        assert_eq!(slots, ["federation", "klingon"]);
    }

    #[test]
    fn loadout_uses_only_resolved_authored_standard_units() {
        // This checks authored list handling, not missing FO lifecycle parity.
        let db = crate::fake::db();
        let mut def = races().get("federation").cloned().unwrap();
        def.name = "fx".into();
        def.standard_units = vec![
            "fx_station".into(), "fx_effects".into(), "fx_yard".into(),
            "fx_station".into(), "fx_beam".into(),
        ];
        let r = Races::from_defs(vec![def]);
        let l = r.loadout(&db, "fx");
        let names: Vec<_> = l.units.iter().map(|&k| db.name(k).unwrap()).collect();
        assert_eq!(names, ["fx_station", "fx_yard", "fx_station"]);
        assert_eq!(l.source, "race federation");
        assert_eq!(l.stock[0], Fixed::from_int(4000));
    }

    #[test]
    fn known_empty_and_stub_starts_do_not_infer_units() {
        // Authored correctness does not supply missing FO lifecycle parity.
        let base = crate::fake::db();
        let mut stub = crate::UnitDef::new("fx_outposts", "shipyard");
        stub.race = "fx".into();
        stub.raw = std::sync::Arc::new(
            sta_odf::resolve("fx_outposts", &mut |_| {
                Some("originalclass = \"fx_yard\"\n".into())
            })
            .unwrap(),
        );
        let mut b = crate::ContentDb::builder()
            .profile(base.profile())
            .features(base.features().clone());
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
        let l = Races::from_defs(vec![def.clone()]).loadout(&db, "fx");
        let names: Vec<_> = l.units.iter().map(|&k| db.name(k).unwrap()).collect();
        assert_eq!(names, ["fx_outposts"]);
        def.standard_units.clear();
        def.minimal_units = vec!["fx_yard".into()];
        let empty = Races::from_defs(vec![def.clone()]).loadout(&db, "fx");
        assert!(empty.units.is_empty());
        assert_eq!(empty.source, "race federation");
        def.standard_units = vec!["missing".into()];
        let missing = Races::from_defs(vec![def]).loadout(&db, "fx");
        assert!(missing.units.is_empty());
        assert_eq!(missing.source, "race federation");
    }

    #[test]
    fn numbered_start_keys_keep_gaps_high_indexes_and_repeated_values() {
        let (races, warnings) = Races::load_with(&mut loader(&[
            ("races", "race0=fx"),
            ("fx", "name=fx\ninstantActionSlot=0\nstandardUnits0=fx_miner\nstandardUnits1=fx_station\nstandardUnits3=fx_yard\nstandardUnits40=fx_station"),
        ]));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(races.get("fx").unwrap().standard_units, ["fx_station", "fx_yard", "fx_station"]);
        let db = crate::fake::db();
        let names: Vec<_> = races.loadout(&db, "fx").units.iter().map(|&k| db.name(k).unwrap()).collect();
        assert_eq!(names, ["fx_station", "fx_yard", "fx_station"]);
    }

    #[test]
    fn a_race_without_data_gets_the_documented_default() {
        let db = crate::fake::db();
        let l = Races::default().loadout(&db, "fx");
        let names: Vec<_> = l.units.iter().map(|&k| db.name(k).unwrap()).collect();
        assert_eq!(names, ["fx_yard", "fx_miner"]);
        assert_eq!(l.source, "default");
        assert_eq!(l.stock, DEFAULT_STOCK.map(Fixed::from_int));
        assert_eq!(l.crew, None);
        let lots = Races::default().loadout_with_mode(&db, "fx", StartingResources::Lots);
        assert_eq!(lots.stock, l.stock);
        assert_eq!(lots.crew, None);
        assert!(Races::default().loadout(&db, "nobody").units.is_empty());
    }

    #[test]
    fn resolved_modes_keep_authored_stock_crew_and_exact_units() {
        let (races, warnings) = Races::load_with(&mut loader(&[
            ("races", "race0=child"),
            ("base", "name=fx\ninstantActionSlot=0\nstandardUnits1=fx_station\nstandardUnits2=fx_yard\nstandardUnits3=fx_station\nnormalDilithium=11\nnormalLatinum=12\nnormalMetal=13\nnormalBiomatter=14\nlotsDilithium=21\nlotsLatinum=22\nlotsMetal=23\nlotsBiomatter=24\nnormalCrew=0\nlotsCrew=31"),
            ("child", "#include \"base.odf\"\nnormalLatinum=17\nlotsMetal=27\nlotsCrew=37"),
        ]));
        assert!(warnings.is_empty(), "{warnings:?}");
        let race = races.get("child").unwrap();
        assert_eq!(race.stock, [11, 17, 13, 14]);
        assert_eq!(race.lots_stock, [21, 22, 27, 24]);
        assert_eq!(race.normal_crew, Some(0));
        assert_eq!(race.lots_crew, Some(37));
        let db = crate::fake::db();
        let normal = races.loadout(&db, "fx");
        let lots = races.loadout_with_mode(&db, "fx", StartingResources::Lots);
        let names = |l: &Loadout| l.units.iter().map(|&k| db.name(k).unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(names(&normal), ["fx_station", "fx_yard", "fx_station"]);
        assert_eq!(names(&lots), names(&normal));
        assert_eq!(normal.stock, [11, 17, 13, 14].map(Fixed::from_int));
        assert_eq!(lots.stock, [21, 22, 27, 24].map(Fixed::from_int));
        assert_eq!(normal.crew, Some(Fixed::ZERO));
        assert_eq!(lots.crew, Some(Fixed::from_int(37)));
    }

    #[test]
    fn missing_and_invalid_crew_remain_absent() {
        let (races, warnings) = Races::load_with(&mut loader(&[
            ("races", "race0=missing\nrace1=invalid"),
            ("missing", "name=missing\ninstantActionSlot=0"),
            ("invalid", "name=invalid\ninstantActionSlot=1\nnormalCrew=-1\nlotsCrew=word"),
        ]));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.message.contains("normalcrew")));
        assert!(warnings.iter().any(|w| w.message.contains("lotscrew")));
        for race in races.iter() {
            assert_eq!(race.normal_crew, None);
            assert_eq!(race.lots_crew, None);
        }
        let db = crate::fake::db();
        assert_eq!(races.loadout(&db, "missing").crew, None);
        assert_eq!(races.loadout_with_mode(&db, "invalid", StartingResources::Lots).crew, None);
    }

    #[test]
    fn seat_rules_are_read_and_the_borg_latinum_rule_is_derived_unless_authored() {
        let files: &[(&str, &str)] = &[
            ("races", "race0=fed\nrace1=spec\nrace2=borg\nrace3=freeborg\nrace4=paid"),
            ("fed", "name=federation"),
            ("spec", "name=species8472\nmaxOfficerMultiplier=0.24f"),
            ("borg", "name=borg"),
            ("freeborg", "name=freeborg"),
            ("paid", "name=borg\nlatinumCostModifier=1"),
        ];
        let (races, warnings) = Races::load_with_profile(Profile::Sta2Classic, &mut loader(files));
        assert!(warnings.is_empty(), "{warnings:?}");
        let rules: Vec<_> = races.seat_rules().collect();
        // Only non-identity rules are kept: the default race, the freeborg (a different name) and
        // the authored `latinumCostModifier = 1` carry none.
        assert_eq!(
            rules,
            vec![
                ("species8472", SeatRaceRule { officer_multiplier: Fixed::from_f64(0.24f32 as f64), latinum_scale: Fixed::ONE }),
                ("borg", SeatRaceRule { officer_multiplier: Fixed::ONE, latinum_scale: Fixed::ZERO }),
            ]
        );
        // Armada I was not traced for the name compare: no derivation there.
        let (a1, _) = Races::load_with_profile(Profile::Sta1Classic, &mut loader(files));
        assert_eq!(a1.seat_rules().count(), 1);
    }
}
