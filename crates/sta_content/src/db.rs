//! The content database (S5): every definition under one name-sorted table.

use std::sync::Arc;

use sta_core::{ContentKey, Fixed};

use crate::def::{DefRef, UnitDef, WeaponDef};
use crate::digest::{self, Canon, ContentDigest, DigestEntry, fixed_text, group, quoted};
use crate::features::{Features, Profile};
use crate::resources::ResourceModel;
use crate::index::OdfIndex;
use crate::techtree::TechTree;
use crate::{ContentError, Result, Warning};

/// The tech tree `load` mounts: what a skirmish game uses.
pub const DEFAULT_TECH_TREE: &str = "tech1";

// Units are much larger than weapons; a table built once can afford that.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum Def {
    Unit(UnitDef),
    Weapon(WeaponDef),
}

impl Def {
    fn name(&self) -> &str {
        match self {
            Def::Unit(u) => &u.odf,
            Def::Weapon(w) => &w.odf,
        }
    }
}

/// Every unit and weapon definition of the mounted content, immutable once
/// built and shared as `Arc`. A [`ContentKey`] is the index of a name in the
/// table sorted bytewise (names lower-cased, as ODF lookups ignore case).
#[derive(Debug, Clone)]
pub struct ContentDb {
    /// Sorted by lower-cased name; the index is the key.
    defs: Vec<(String, Def)>,
    profile: Profile,
    features: Features,
    /// What the game calls its stock slots. The names are display text and
    /// not in the digest; the slots and their aliases are.
    resources: ResourceModel,
    /// What may be built given what is owned (CON-06); `None` allows
    /// everything, as the fakes do.
    tech: Option<Arc<TechTree>>,
    /// Definition-key-aligned immutable map rules; weapons have no policy.
    map_confinement: Vec<Option<crate::ConfinementRules>>,
    construction_confinement: Vec<Option<crate::ConfinementRules>>,
    has_map_confinement: bool,
    map_confinement_warnings: Vec<Warning>,
    rules: crate::Rules,
    /// Armada I's play-physics block (CON-52): only under `Sta1Classic`, and
    /// only when the file is there and usable.
    a1_play: Option<crate::A1Dynamics>,
    a1_officer_policy: bool,
    /// The mounted layers' names, top first: informational (D10), recorded
    /// in replays and snapshots, never compared.
    packs: Vec<String>,
    digest: ContentDigest,
}

impl ContentDb {
    pub fn rules(&self) -> &crate::Rules { &self.rules }

    /// Whether a race can trade. Unknown races and races without an authored
    /// `canTrade = 0` default to true.
    pub fn can_trade(&self, race: &str) -> bool {
        self.rules.can_trade(race)
    }

    /// Whether the resolved content uses Armada I officer rules.
    pub fn a1_officers(&self) -> bool {
        self.profile == Profile::Sta1Classic || self.a1_officer_policy
    }

    /// The explicit Armada I `cfgMaxOfficers`, when valid and representable.
    pub fn a1_starting_officers(&self) -> Option<i32> {
        self.a1_officers().then(|| self.rules.a1_max_officers()).flatten()
    }

    /// Resolve each A1 starbase's race upgrade after all definition keys exist.
    pub(crate) fn with_a1_officer_upgrades(mut self, enabled: bool, races: &std::collections::BTreeMap<String, String>, warnings: &mut Vec<Warning>) -> Self {
        self.a1_officer_policy = enabled;
        if !self.a1_officers() { return self; }
        let upgrades: std::collections::BTreeMap<String, ContentKey> = self.units()
            .filter(|u| u.a1_officer_upgrade_gain.is_some())
            .map(|u| (u.odf.clone(), u.key)).collect();
        let upgrade_keys: std::collections::BTreeSet<ContentKey> = upgrades.values().copied().collect();
        let native = self.profile == Profile::Sta1Classic;
        for (_, def) in &mut self.defs {
            let Def::Unit(base) = def else { continue };
            let Some(mut officers) = base.a1_base_officers else { continue };
            let file = format!("{}.odf", base.odf);
            let named = races.get(&base.race.to_ascii_lowercase()).and_then(|s| upgrades.get(s)).copied();
            let fallback = if !native && !races.contains_key(&base.race.to_ascii_lowercase()) {
                let candidates: std::collections::BTreeSet<_> = base.build.list.iter()
                    .filter(|item| upgrade_keys.contains(&item.key))
                    .map(|item| item.key).collect();
                if candidates.len() == 1 { candidates.iter().next().copied() } else { None }
            } else { None };
            officers.upgrade = named.or(fallback);
            if named.is_none() && fallback.is_some() {
                warnings.push(Warning::new(&file, "officer upgrade inferred from the only eligible build item"));
            }
            if races.contains_key(&base.race.to_ascii_lowercase()) && named.is_none() {
                warnings.push(Warning::new(&file, "race officerUpgradeODF does not name a defined upgrade"));
            }
            base.build.list.retain(|item| !upgrade_keys.contains(&item.key) || Some(item.key) == officers.upgrade);
            if let Some(key) = officers.upgrade && !base.build.list.iter().any(|item| item.key == key) {
                let name = upgrades.iter().find_map(|(name, value)| (*value == key).then_some(name.clone())).expect("upgrade key came from map");
                base.build.list.push(DefRef { odf: name, key });
            }
            base.a1_base_officers = Some(officers);
        }
        self.digest = digest_of(&self);
        self
    }

    /// Armada I's play-physics block (CON-52): `Some` only under
    /// `Sta1Classic` with a usable file. The movement model asks for it.
    pub fn a1_play_physics(&self) -> Option<&crate::A1Dynamics> {
        self.a1_play.as_ref().filter(|_| self.profile == Profile::Sta1Classic)
    }

    pub fn with_rules(mut self, rules: crate::Rules) -> Self {
        self.rules = rules;
        self.digest = digest_of(&self);
        self
    }

    /// The names of the packs (mounted layers) this content came from, top
    /// first; empty for content built in code. Informational only (D10).
    pub fn packs(&self) -> &[String] {
        &self.packs
    }

    /// This database with these pack names (the digest does not change).
    pub fn with_packs(mut self, packs: Vec<String>) -> Self {
        self.packs = packs;
        self
    }

    /// The content lineage (the profile, D4) by name: `native`, `fo4`,
    /// `sta2classic`, `sta1classic`.
    pub fn lineage(&self) -> &'static str {
        match self.profile {
            Profile::Native => "native",
            Profile::Fo4 => "fo4",
            Profile::Fo3 => "fo3",
            Profile::Sta2Classic => "sta2classic",
            Profile::Sta1Classic => "sta1classic",
        }
    }

    /// Load every unit and weapon definition from the ODFs of a mounted
    /// install (see [`crate::odf`]). ODFs that cannot make a definition are
    /// listed in the warnings, not errors.
    ///
    /// The profile is detected from the mod chain ([`Profile::detect`]); the
    /// database-wide features are the profile's defaults with the chain's
    /// `info.ini` `[Features]` applied, and each unit and weapon carries its
    /// own set with its ODF's `Feature_<name>` keys on top
    /// ([`unit_features`](ContentDb::unit_features)).
    ///
    /// The tech tree [`DEFAULT_TECH_TREE`] is loaded too when one is mounted
    /// (`.xml` or `.tt`, see [`TechTree::find`]) and gates building (SIM-09);
    /// without one `tech_tree()` is `None` and a warning says so.
    pub fn load(vfs: &sta_vfs::Vfs, index: &OdfIndex) -> Result<(ContentDb, Vec<Warning>)> {
        let detection = Profile::detect_with_source(vfs);
        let (db, mut warnings) = ContentDb::load_as(vfs, detection.profile, index)?;
        warnings.splice(0..0, detection.warnings);
        Ok((db, warnings))
    }

    /// [`load`](ContentDb::load) with the profile given instead of detected
    /// (a lobby choice, a test).
    pub fn load_as(vfs: &sta_vfs::Vfs, profile: Profile, index: &OdfIndex) -> Result<(ContentDb, Vec<Warning>)> {
        let (db, mut warnings) = crate::odf::load(vfs, profile, index)?;
        let (races, race_warnings) = crate::Races::load(vfs, index);
        // A bare content database has no race catalogue; keep its historical
        // quiet fallback while reporting malformed authored race metadata.
        if index.get("races").is_some() {
            warnings.extend(race_warnings);
        }
        let (rules, rule_warnings) = crate::Rules::load(vfs, &races)?;
        warnings.extend(rule_warnings);
        let (db, radius_warnings) = db.with_rules(rules).with_launch_points_warned(vfs);
        warnings.extend(radius_warnings);
        let db = db
            .with_resources(ResourceModel::from_strings(profile, &crate::StringTable::load(vfs)))
            .with_packs(vfs.layers().iter().map(|l| l.name().to_owned()).collect());
        if db.is_empty() {
            return Ok((db, warnings));
        }
        // The skirmish tree: `tech1`, from the top-most layer that has one.
        // A map may name another (CON-07/UI); `with_tech_tree` swaps it.
        match TechTree::from_vfs(vfs, DEFAULT_TECH_TREE)? {
            // Load what parses; the warnings (mostly duplicate entries in
            // Fleet Operations' files) are the caller's to show.
            Some((tree, w)) if !tree.is_empty() => {
                warnings.extend(w);
                Ok((db.with_tech_tree(tree), warnings))
            }
            // Unreadable or empty: no gating rather than gating everything.
            Some((_, w)) => {
                warnings.extend(w);
                warnings.push(Warning::new(DEFAULT_TECH_TREE, "tech tree has no entries; building is not gated"));
                Ok((db, warnings))
            }
            None => {
                warnings.push(Warning::new(DEFAULT_TECH_TREE, "no tech tree found; building is not gated"));
                Ok((db, warnings))
            }
        }
    }

    /// This database with builder launch points, collision radii and authored
    /// repair attachment geometry read from simulation SODs, then re-digested.
    pub fn with_launch_points(self, vfs: &sta_vfs::Vfs) -> ContentDb {
        self.with_launch_points_warned(vfs).0
    }

    /// [`with_launch_points`](ContentDb::with_launch_points) and warnings for
    /// unreadable collision geometry or authored repair nodes that cannot resolve.
    pub fn with_launch_points_warned(mut self, vfs: &sta_vfs::Vfs) -> (ContentDb, Vec<Warning>) {
        let mut warnings = crate::launch::attach(vfs, self.defs.iter_mut().filter_map(|(_, d)| if let Def::Unit(u) = d { Some(u) } else { None }));
        warnings.extend(crate::launch::attach_radii(vfs, self.defs.iter_mut().filter_map(|(_, d)| if let Def::Unit(u) = d { Some(u) } else { None })));
        warnings.extend(crate::repair_geometry::attach(vfs, self.defs.iter_mut().filter_map(|(_, d)| if let Def::Unit(u) = d { Some(u) } else { None })));
        self.digest = digest_of(&self);
        (self, warnings)
    }

    /// Resolve indexed native and extended research lists once every pod is known.
    pub(crate) fn with_research_builds_warned(mut self) -> (ContentDb, Vec<Warning>) {
        use std::collections::{BTreeMap, BTreeSet};
        let mut warnings = Vec::new();
        let pods: BTreeMap<String, (ContentKey, Option<crate::SystemUpgrade>)> = self
            .units()
            .map(|u| (u.odf.to_ascii_lowercase(), (u.key, u.system_upgrade)))
            .collect();
        for (_, def) in &mut self.defs {
            let Def::Unit(station) = def else { continue };
            if station.class.0 != "research" {
                continue;
            }
            let odf = &station.raw;
            let primary: BTreeMap<usize, &str> = crate::odf::numbered(odf, "builditem")
                .into_iter()
                .map(|(i, raw)| (i as usize, raw))
                .collect();
            let secondary: BTreeMap<usize, &str> = crate::odf::numbered(odf, "secondarybuilditem")
                .into_iter()
                .map(|(i, raw)| (i as usize, raw))
                .collect();
            let mut extended = BTreeMap::<(u32, usize), &str>::new();
            for key in odf.keys() {
                let lower = key.to_ascii_lowercase();
                let Some(rest) = lower.strip_prefix("tier") else {
                    continue;
                };
                let Some((tier, index)) = rest.split_once("builditem") else {
                    continue;
                };
                if let (Ok(tier), Ok(index), Some(raw)) =
                    (tier.parse::<u32>(), index.parse::<usize>(), odf.raw(key))
                {
                    let slots = if tier == 0 { 57 } else { 58 };
                    if tier <= 14 && index < slots {
                        extended.insert((tier, index), raw);
                    } else {
                        warnings.push(Warning::new(
                            format!("{}.odf", station.odf),
                            format!("{key} exceeds the upgrade station tier or slot limit; ignored"),
                        ));
                    }
                }
            }
            if secondary.is_empty() && extended.is_empty() {
                continue;
            }
            let mut indices: BTreeSet<usize> = primary
                .keys()
                .copied()
                .chain(secondary.keys().copied())
                .collect();
            indices.extend(extended.keys().map(|(_, index)| *index));
            let file = format!("{}.odf", station.odf);
            let value = |raw: &str| {
                let stem = crate::odf::stem(raw);
                if stem.is_empty() || stem == "null" {
                    None
                } else {
                    pods.get(&stem).copied()
                }
            };
            let valid_override = |raw: &str,
                                  tier: u32,
                                  previous: Option<crate::SystemUpgrade>,
                                  warnings: &mut Vec<Warning>| {
                let stem = crate::odf::stem(raw);
                if stem.is_empty() || stem == "null" {
                    return Some(None);
                }
                match value(raw) {
                    Some((key, Some(upgrade)))
                        if upgrade.tier == tier + 2
                            && previous.is_none_or(|p| p.system == upgrade.system) =>
                    {
                        Some(Some((key, Some(upgrade))))
                    }
                    _ => {
                        warnings.push(Warning::new(&file, format!("tier{tier}BuildItem references invalid or mismatched pod {stem}; ignored")));
                        None
                    }
                }
            };
            for index in indices {
                let base = primary.get(&index).and_then(|raw| value(raw));
                let mut current = base;
                if let Some(raw) = extended.get(&(0, index))
                    && let Some(replacement) = valid_override(raw, 0, None, &mut warnings)
                {
                    current = replacement;
                }
                let primary_key = current.map(|(key, _)| key);
                let mut successors = Vec::new();
                let mut previous = current.and_then(|(_, upgrade)| upgrade);
                let mut next = secondary.get(&index).and_then(|raw| value(raw));
                if let Some(raw) = secondary.get(&index) {
                    let name = crate::odf::stem(raw);
                    if !name.is_empty() && name != "null" && next.is_none() {
                        warnings.push(Warning::new(&file, format!("secondaryBuildItem{index} references undefined unit {name}; ignored")));
                    }
                }
                if let Some((_, upgrade)) = next {
                    let wrong_tier = upgrade.is_some_and(|u| u.tier != 3);
                    let wrong_chain = previous.is_some_and(|p| upgrade.is_none_or(|u| u.system != p.system));
                    if wrong_tier || wrong_chain {
                        warnings.push(Warning::new(&file, format!("secondaryBuildItem{index} is not a matching tier-3 pod; ignored")));
                        next = None;
                    }
                }
                if let Some(raw) = extended.get(&(1, index))
                    && let Some(replacement) = valid_override(raw, 1, previous, &mut warnings)
                {
                    next = replacement;
                }
                if let Some((key, upgrade)) = next {
                    successors.push(key);
                    previous = upgrade;
                } else {
                    previous = None;
                }
                for tier in 2..=14 {
                    let Some(raw) = extended.get(&(tier, index)) else {
                        continue;
                    };
                    if successors.len() != (tier - 1) as usize || previous.is_none() {
                        warnings.push(Warning::new(
                            &file,
                            format!(
                                "tier{tier}BuildItem{index} has no preceding pod tier; ignored"
                            ),
                        ));
                        continue;
                    }
                    match valid_override(raw, tier, previous, &mut warnings) {
                        Some(Some((key, upgrade))) => {
                            successors.push(key);
                            previous = upgrade;
                        }
                        Some(None) => {
                            previous = None;
                        }
                        None => {}
                    }
                }
                station.research_build.push(crate::ResearchBuildSlot {
                    index,
                    primary: primary_key,
                    successors,
                });
            }
        }
        self.digest = digest_of(&self);
        (self, warnings)
    }

    /// This database gated by `tree` (replacing any), with the digest
    /// recomputed.
    pub fn with_tech_tree(mut self, tree: TechTree) -> ContentDb {
        self.tech = Some(Arc::new(tree));
        self.digest = digest_of(&self);
        self
    }

    /// The database gated by the tech tree a map names with its header key
    /// `techTreeFilename` (a file name, `.tt` or `.xml`, looked up by stem as
    /// [`TechTree::find`] does): `Some` when that swaps the tree. An empty name,
    /// `none`, or the tree already in use gives `None` (keep this database). A
    /// tree the mount lacks, that fails to load or that has no entries gives
    /// `None` and a warning that the current tree stays (the caller logs it).
    /// Read from the executable, static (Armada I and II): the
    /// game plays with the tree its map names.
    pub fn with_map_tech_tree(&self, vfs: &sta_vfs::Vfs, name: &str) -> (Option<ContentDb>, Vec<Warning>) {
        let name = name.trim();
        let file_stem = |n: &str| {
            let n = n.trim().rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
            n.strip_suffix(".tt").or_else(|| n.strip_suffix(".xml")).map(str::to_owned).unwrap_or(n)
        };
        let stem = file_stem(name);
        if stem.is_empty() || stem == "none" || self.tech.as_ref().is_some_and(|t| file_stem(&t.name) == stem) {
            return (None, Vec::new());
        }
        match TechTree::from_vfs(vfs, &stem) {
            Ok(Some((tree, w))) if !tree.is_empty() => (Some(self.clone().with_tech_tree(tree)), w),
            Ok(Some((_, mut w))) => {
                w.push(Warning::new(name, "map tech tree has no entries; the current tree stays"));
                (None, w)
            }
            Ok(None) => (None, vec![Warning::new(name, "map tech tree not found in the mount; the current tree stays")]),
            Err(e) => (None, vec![Warning::new(name, format!("map tech tree unreadable ({e}); the current tree stays"))]),
        }
    }

    /// [`Self::with_map_tech_tree`] for the map named `map` (a file stem under
    /// `bzn/`, `.bzn` then `.sol`): the tree its header names. It is for tools
    /// that rebuild the content of a game they did not start (the bench's
    /// capture check; not `sta-bench desync`, whose lobby matches keep the
    /// default tree); the game itself has the parsed map and calls
    /// [`Self::with_map_tech_tree`] with its `techTreeFilename`. A map that
    /// cannot be read leaves the current tree, with a warning. The capture check
    /// always applies the map's tree, which is right only while lobby matches
    /// are never captured.
    pub fn with_named_map_tech_tree(&self, vfs: &sta_vfs::Vfs, map: &str) -> (Option<ContentDb>, Vec<Warning>) {
        let Some(read) = vfs.read(&format!("bzn/{map}.bzn")).or_else(|| vfs.read(&format!("bzn/{map}.sol"))) else {
            return (None, vec![Warning::new(map, "map not found in the mount; its tech tree is not applied")]);
        };
        match read.map_err(|e| e.to_string()).and_then(|d| sta_bzn::parse(&d).map_err(|e| e.to_string())) {
            Ok(m) => self.with_map_tech_tree(vfs, &m.tech_tree_file),
            Err(e) => (None, vec![Warning::new(map, format!("map unreadable ({e}); its tech tree is not applied"))]),
        }
    }

    /// Build a database from definitions made in code (fakes) or JSON5.
    pub fn builder() -> ContentBuilder {
        ContentBuilder::default()
    }

    pub fn key(&self, odf: &str) -> Option<ContentKey> {
        let name = odf.to_ascii_lowercase();
        self.defs.binary_search_by(|(n, _)| n.as_str().cmp(&name)).ok().map(|i| ContentKey(i as u32))
    }

    /// The (lower-cased) name of a key.
    pub fn name(&self, k: ContentKey) -> Option<&str> {
        self.defs.get(k.0 as usize).map(|(n, _)| n.as_str())
    }

    pub fn unit(&self, k: ContentKey) -> Option<&UnitDef> {
        match self.defs.get(k.0 as usize) {
            Some((_, Def::Unit(u))) => Some(u),
            _ => None,
        }
    }

    /// Whether an effective research chain's held pod satisfies an earlier
    /// prerequisite at the same indexed station slot.
    pub fn research_satisfies(&self, held: ContentKey, required: ContentKey) -> bool {
        if held == required { return true; }
        let (Some(held_upgrade), Some(required_upgrade)) = (
            self.unit(held).and_then(|unit| unit.system_upgrade),
            self.unit(required).and_then(|unit| unit.system_upgrade),
        ) else { return false; };
        if held_upgrade.system != required_upgrade.system || held_upgrade.tier <= required_upgrade.tier { return false; }
        self.units().filter(|unit| unit.class.0 == "research").any(|station| {
            station.research_build.iter().any(|slot| {
                let mut chain = slot.primary.into_iter().chain(slot.successors.iter().copied());
                let required_index = chain.clone().position(|key| key == required);
                let held_index = chain.position(|key| key == held);
                matches!((required_index, held_index), (Some(required), Some(held)) if held > required)
            })
        })
    }

    /// Tier-2 pod whose independent stat survives replacement by this pod.
    /// A shared successor with conflicting primaries has no inferred value.
    pub fn research_primary(&self, successor: ContentKey) -> Option<crate::SystemUpgrade> {
        let system = self.unit(successor)?.system_upgrade?.system;
        let mut primaries = self.units().flat_map(|u| &u.research_build)
            .filter(|slot| slot.successors.contains(&successor))
            .filter_map(|slot| self.unit(slot.primary?)?.system_upgrade)
            .filter(|pod| pod.system == system && pod.tier == 2);
        let first = primaries.next()?;
        primaries.all(|pod| pod == first).then_some(first)
    }

    /// The aperture ODF named by a yard, resolved through the mounted data.
    pub fn launch_wormhole(&self, builder: &UnitDef) -> Option<&UnitDef> {
        let name = crate::odf::stem(builder.raw.str("wormhole")?);
        self.key(&name).and_then(|key| self.unit(key)).filter(|unit| unit.class.0 == "wormhole_obj")
    }

    pub fn weapon(&self, k: ContentKey) -> Option<&WeaponDef> {
        match self.defs.get(k.0 as usize) {
            Some((_, Def::Weapon(w))) => Some(w),
            _ => None,
        }
    }

    /// Unit definitions in key order.
    pub fn units(&self) -> impl Iterator<Item = &UnitDef> {
        self.defs.iter().filter_map(|(_, d)| match d {
            Def::Unit(u) => Some(u),
            _ => None,
        })
    }

    /// Weapon definitions in key order.
    pub fn weapons(&self) -> impl Iterator<Item = &WeaponDef> {
        self.defs.iter().filter_map(|(_, d)| match d {
            Def::Weapon(w) => Some(w),
            _ => None,
        })
    }

    /// Number of definitions (keys run from 0 to this).
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// The first eight bytes of the content [`digest`](ContentDb::digest):
    /// replays and snapshots carry it, and every sim digest folds it in
    /// (SIM-01), so tick 0 already tells different content apart.
    pub fn hash(&self) -> u64 {
        self.digest.hash64()
    }

    /// The keyed digest of every resolved sim-visible rule (D10): what
    /// peers compare, and what names the first differing key when they
    /// disagree. See [`crate::digest`].
    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// What the game calls its stock slots (the install's string table over
    /// the profile's words).
    pub fn resources(&self) -> &ResourceModel {
        &self.resources
    }

    /// This database with these slot names.
    pub fn with_resources(mut self, resources: ResourceModel) -> ContentDb {
        self.resources = resources;
        self.digest = digest_of(&self);
        self
    }

    /// The database-wide switches: the profile's defaults and the mod
    /// chain's overrides.
    pub fn features(&self) -> &Features {
        &self.features
    }

    /// The tech tree the sim gates building with (SIM-09). `None` means no
    /// gating: every def a builder lists may be built.
    pub fn tech_tree(&self) -> Option<&TechTree> {
        self.tech.as_deref()
    }

    /// The switches that apply to one unit: its own resolved set, or the
    /// database-wide one for defs built in code.
    pub fn unit_features<'a>(&'a self, u: &'a UnitDef) -> &'a Features {
        u.features.as_ref().unwrap_or(&self.features)
    }

    /// Total station jobs for this producer, including its active head.
    /// An explicit zero disables new queue admissions without cancelling
    /// existing jobs. See docs/QUEUE_LIMITS.md for profile defaults.
    pub fn build_queue_limit(&self, u: &UnitDef) -> u32 {
        u.build.queue_limit.unwrap_or_else(|| self.profile.build_queue_limit())
    }

    /// Effective per-definition rules, resolved once at build time.
    pub fn map_confinement(&self, key: ContentKey) -> Option<&crate::ConfinementRules> {
        self.map_confinement.get(key.0 as usize).and_then(Option::as_ref)
    }

    /// Plain-GameObject policy while the final definition is under construction.
    pub fn construction_confinement(&self, key: ContentKey) -> Option<&crate::ConfinementRules> {
        self.construction_confinement.get(key.0 as usize).and_then(Option::as_ref)
    }

    /// Includes supported definitions that may be spawned later, not just actors.
    pub fn has_map_confinement(&self) -> bool {
        self.has_map_confinement
    }

    /// Diagnostics for unsupported policies or typed overrides built in code.
    /// Sim setup adds volume diagnostics and reports these once per setup.
    pub fn map_confinement_warnings(&self) -> &[Warning] {
        &self.map_confinement_warnings
    }

    /// The switches that apply to one weapon, as [`unit_features`](ContentDb::unit_features).
    pub fn weapon_features<'a>(&'a self, w: &'a WeaponDef) -> &'a Features {
        w.features.as_ref().unwrap_or(&self.features)
    }
}

/// Collects definitions, then sorts them, hands out keys and resolves
/// references in [`build`](ContentBuilder::build).
#[derive(Debug, Clone, Default)]
pub struct ContentBuilder {
    defs: Vec<Def>,
    profile: Profile,
    features: Features,
    resources: Option<ResourceModel>,
    tech: Option<TechTree>,
    a1_play: Option<crate::A1Dynamics>,
}

impl ContentBuilder {
    pub fn unit(mut self, def: UnitDef) -> Self {
        self.defs.push(Def::Unit(def));
        self
    }

    pub fn weapon(mut self, def: WeaponDef) -> Self {
        self.defs.push(Def::Weapon(def));
        self
    }

    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self
    }

    pub fn features(mut self, features: Features) -> Self {
        self.features = features;
        self
    }

    /// The slot names; the profile's when not given.
    pub fn resources(mut self, resources: ResourceModel) -> Self {
        self.resources = Some(resources);
        self
    }

    /// Armada I's play-physics block (CON-52). Ignored (and not hashed) unless
    /// the profile is `Sta1Classic`.
    pub fn a1_play_physics(mut self, play: Option<crate::A1Dynamics>) -> Self {
        self.a1_play = play;
        self
    }

    /// Gate building with this tech tree (it is part of the content hash).
    pub fn tech_tree(mut self, tree: TechTree) -> Self {
        self.tech = Some(tree);
        self
    }

    /// Fails on a name defined twice or a reference to a missing name. The
    /// order definitions were added in does not matter.
    pub fn build(self) -> Result<ContentDb> {
        let mut defs: Vec<(String, Def)> = self.defs.into_iter().map(|d| (d.name().to_ascii_lowercase(), d)).collect();
        defs.sort_by(|a, b| a.0.cmp(&b.0));
        if let Some(w) = defs.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(ContentError::Duplicate(w[0].0.clone()));
        }
        let names: Vec<String> = defs.iter().map(|(n, _)| n.clone()).collect();
        let resolve = |from: &str, r: &mut DefRef| -> Result<()> {
            let name = r.odf.to_ascii_lowercase();
            match names.binary_search(&name) {
                Ok(i) => {
                    r.key = ContentKey(i as u32);
                    Ok(())
                }
                Err(_) => Err(ContentError::Dangling { from: from.to_owned(), to: r.odf.clone() }),
            }
        };
        for (i, (name, def)) in defs.iter_mut().enumerate() {
            let key = ContentKey(i as u32);
            match def {
                Def::Unit(u) => {
                    u.key = key;
                    for h in &mut u.hardpoints {
                        resolve(name, &mut h.weapon)?;
                    }
                    for b in &mut u.build.list {
                        resolve(name, b)?;
                    }
                }
                Def::Weapon(w) => w.key = key,
            }
        }
        let resources = self.resources.unwrap_or_else(|| ResourceModel::for_profile(self.profile));
        let mut map_confinement_warnings = Vec::new();
        let map_confinement: Vec<_> = defs.iter().map(|(_, def)| match def {
            Def::Unit(u) => Some(crate::confinement::resolve(
                self.profile, &u.class.0, u.confinement, &format!("{}.odf", u.odf),
                &mut map_confinement_warnings,
            )),
            Def::Weapon(_) => None,
        }).collect();
        let has_map_confinement = map_confinement.iter().flatten().any(|r| r.is_enabled());
        let construction_confinement = defs.iter().zip(&map_confinement).map(|((_, def), rules)| {
            match (def, rules) {
                (Def::Unit(u), Some(r)) => Some(crate::confinement::construction(self.profile, *r, u.confinement)),
                _ => None,
            }
        }).collect();
        let mut db = ContentDb {
            defs, profile: self.profile, features: self.features, resources,
            tech: self.tech.map(Arc::new), map_confinement, construction_confinement, has_map_confinement,
            map_confinement_warnings, rules: crate::Rules::default(),
            a1_play: self.a1_play.filter(|_| self.profile == Profile::Sta1Classic), a1_officer_policy: false, packs: Vec::new(),
            digest: ContentDigest::default(),
        };
        db.digest = digest_of(&db);
        Ok(db)
    }
}

/// The switches that are on, by name: `[fire_arcs,turrets]`.
fn features_text(f: &Features) -> String {
    let on: Vec<&str> = f.bits().iter().zip(crate::FEATURE_NAMES).filter(|(b, _)| **b).map(|(_, n)| n).collect();
    format!("[{}]", on.join(","))
}

fn confinement_text(r: Option<&crate::ConfinementRules>) -> String {
    match r {
        None => "-".into(),
        Some(r) => group(
            Canon::new()
                .flag("capability", r.capability)
                .int("spatial", r.spatial.hash_tag())
                .int("initial", r.initial.hash_tag())
                .flag("rearm", r.rearm)
                .fixed("inset", r.inset)
                .fixed("upper_epsilon", r.upper_epsilon),
        ),
    }
}

/// A unit's sim-visible values. Display strings (sounds, effects, model and
/// node names) and the model scale (it reaches the sim only through the
/// geometry entry) are left out.
fn unit_text(db: &ContentDb, u: &UnitDef) -> String {
    // Every field, so a new one fails to compile here until someone decides
    // whether the sim reads it (then it is digested below) or not.
    // `transport` (SIM-46 P0): the sim reads `transporter`, `boardingPartyStrength`,
    // `disableShieldsOnTransport` and `shieldDelay` (SIM-69), digested below when
    // set; `has_crew`, `assault`, `weldingAnimSpeed`, `eventTransport` are not read yet.
    let UnitDef {
        key: _, odf: _, class: _, is_starbase: _, planet_role: _, planet_crew_gain: _, planet_crew_modifier: _,
        a1_base_officers: _, a1_officer_upgrade_gain: _, trade_station: _, trade_cargo_capacity: _,
        planet_colonizable: _, planet_civilian_population_modifier: _, planet_max_population: _, can_have_orbital_station: _, fleet_cap: _, orbit_time: _, a1_borg: _, station_mining: _, node_income: _, race: _,
        model: _, scale: _, movement: _, hull: _, self_destruct_blast: _, shields: _, initial_hull: _, initial_shields: _,
        crew: _, maximum_crew: _, facility: _, has_crew: _, sensor: _, hardpoints: _, build: _, launch: _, radius: _,
        free_unit: _, dock: _, sounds: _, fx: _, special_energy_max: _, special_energy_rate: _, miner: _, miner_off: _,
        max_dilithium: _, mining_rate: _, transfer_rate: _, freight_transfer: _, resource: _, features: _,
        confinement: _, nebula_pathing: _, nebula_effects: _, avoid_me: _, hidden: _, invalid_as_target: _, attack_power: _, life: _,
        crew_gain: _, transport: _, sane_crew_loss: _, system_hit_points: _, casualty: _, repair: _, repair_attachment: _, research_slots: _, research_build: _, research_station_upgrade: _, system_upgrade: _, stats: _, raw: _,
    } = u;
    // `stats` (CON-55): only the keys a sim rule reads are digested (the planet keys
    // under SIM-73's target-mask switch, below); the rest is typed only.
    let crate::BuildDef { cost: _, ticks: _, officers: _, list: _, queue_limit: _, flagged_slots: _ } = &u.build;
    let name = |r: &crate::DefRef| quoted(&r.odf.to_ascii_lowercase());
    let m = &u.movement;
    let mut c = Canon::new();
    c.text("class", &u.class.0)
        .text("race", &u.race)
        .flag("starbase", u.is_starbase)
        .raw("planet_role", match u.planet_role {
            None => "-",
            Some(crate::PlanetRole::Background) => "background",
            Some(crate::PlanetRole::Producer) => "producer",
        })
        // Read only for a producer planet.
        .int("planet_crew_gain", if u.planet_role == Some(crate::PlanetRole::Producer) { u.planet_crew_gain } else { 0 })
        .opt_fixed("planet_crew_modifier", u.planet_crew_modifier)
        // CON-37: effective for a producer planet, false/zero otherwise.
        .flag("planet_colonizable", u.planet_colonizable)
        .fixed("planet_civilian_population_modifier", u.planet_civilian_population_modifier)
        .list("movement", [m.max_speed, m.forward_accel, m.turn_omega, m.turn_alpha, m.turn_at_rest].into_iter().chain(m.smooth_fields()).map(fixed_text))
        .fixed("hull", u.hull)
        .fixed("shields", u.shields.max)
        .fixed("shield_rate", u.shields.recharge)
        // Effective values: an absent key and its default are the same rule.
        .fixed("initial_hull", u.initial_hull.unwrap_or(u.hull))
        .fixed("initial_shields", u.initial_shields.unwrap_or(u.shields.max))
        .fixed("crew", u.crew)
        .fixed("crew_capacity", u.maximum_crew.unwrap_or(u.crew).max(Fixed::ZERO))
        .fixed("sensor", u.sensor)
        .list(
            "hardpoints",
            u.hardpoints.iter().map(|h| {
                let arc = h.arc.map_or_else(|| "-".into(), |a| format!("[{},{},{}]", fixed_text(a.dir.0), fixed_text(a.dir.1), fixed_text(a.min_dot)));
                group(Canon::new().raw("weapon", &name(&h.weapon)).raw("arc", &arc))
            }),
        )
        .list("cost", u.build.cost.iter().map(|c| fixed_text(*c)))
        .int("build_ticks", u.build.ticks)
        .fixed("officers", u.build.officers)
        .int("queue_limit", db.build_queue_limit(u))
        .list("builds", u.build.list.iter().map(name))
        .raw("flagged_slots", &u.build.flagged_slots.as_ref().map_or_else(|| "-".into(), |f| format!("[{}]", f.iter().map(|s| quoted(s)).collect::<Vec<_>>().join(","))))
        .raw("launch", &u.launch.as_ref().map_or_else(|| "-".into(), |l| {
            format!("[{}]", [l.offset.0, l.offset.1, l.dir.0, l.dir.1, l.clear].map(fixed_text).join(","))
        }))
        .fixed("special_energy", u.special_energy_max)
        .fixed("special_energy_rate", u.special_energy_rate)
        // Effective: the `miner` key, else class `freighter`.
        .flag("miner", u.mines())
        .fixed("max_dilithium", u.max_dilithium)
        .fixed("mining_rate", u.mining_rate)
        .fixed("transfer_rate", u.transfer_rate)
        .raw("resource", &u.resource.map_or_else(|| "-".into(), |r| r.to_string()))
        .raw("nebula_pathing", &u.nebula_pathing.map_or_else(|| "-".into(), |n| {
            group(Canon::new().raw("cost", &n.cost.map_or_else(|| "-".into(), |c| c.to_string())).flag("planned", n.planned))
        }))
        .flag("avoid_me", u.avoid_me)
        .flag("hidden", u.hidden)
        .flag("invalid_as_target", u.invalid_as_target)
        .raw("life", &u.life.map_or_else(|| "-".into(), |t| t.to_string()))
        .fixed("crew_gain", u.crew_gain)
        .raw("features", &features_text(db.unit_features(u)))
        // A disabled or unsupported map rule confines nothing, whatever its
        // other fields say; the construction rule applies only under it.
        .raw("confinement", &match db.map_confinement(u.key).filter(|r| r.is_enabled()) {
            None => "-".to_owned(),
            Some(r) => format!("{{map={};construction={}}}", confinement_text(Some(r)), confinement_text(db.construction_confinement(u.key))),
        });
    // Peers using the old constructor behavior would diverge on Evolution.
    if u.is_evolver() {
        c.flag("evolves_in_place", true);
    }
    if let Some(blast) = u.self_destruct_blast {
        c.raw("self_destruct_blast", &group(Canon::new()
            .fixed("damage", blast.damage)
            .fixed("radius_factor", blast.radius_factor)
            .fixed("rate", blast.rate)));
    }
    if let Some(rates) = u.freight_transfer {
        c.list("freight_transfer", rates.into_iter().map(fixed_text));
        if let Some(berth) = u.dock.as_ref().filter(|p| p.node.is_some()) {
            c.list("freight_berth", [berth.offset.0, berth.height, berth.offset.1, berth.dir.0, berth.dir.1]
                .into_iter().map(fixed_text));
        }
    }
    if u.research_station_upgrade {
        c.flag("research_station_upgrade", true);
    }
    if let Some(upgrade) = u.system_upgrade {
        c.raw(
            "system_upgrade",
            &format!(
                "{:?}:{}:{}",
                upgrade.system,
                upgrade.tier,
                fixed_text(upgrade.multiplier)
            ),
        );
    }
    if !u.research_build.is_empty() {
        c.list(
            "research_build",
            u.research_build.iter().map(|slot| {
                format!(
                    "{}:{}:[{}]",
                    slot.index,
                    slot.primary
                        .map_or_else(|| "-".to_owned(), |key| key.0.to_string()),
                    slot.successors
                        .iter()
                        .map(|key| key.0.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }),
        );
    }
    if !u.research_slots.is_empty() {
        let scale = if u.scale > Fixed::ZERO { u.scale } else { Fixed::ONE };
        c.fixed("research_scale", scale);
        c.list("research_slots", u.research_slots.iter().map(|slot| {
            let reservation = slot.reserved_for.as_deref().unwrap_or("");
            slot.pose.map_or_else(|| format!("{reservation}:-"), |pose| {
                let values = [pose.origin, pose.axes[0], pose.axes[1], pose.axes[2]]
                    .into_iter().flat_map(|v| [v.x, v.y, v.z]).map(fixed_text).collect::<Vec<_>>();
                format!("{reservation}:{}", values.join(","))
            })
        }));
    }
    if u.allows_station_motion() {
        c.flag("ship_motion", true);
    }
    if let Some(gate) = db.launch_wormhole(u) {
        c.text("launch_wormhole", &gate.odf)
            .int("wormhole_open_ticks", gate.wormhole_open_ticks())
            .fixed("launch_height", u.launch.as_ref().map_or(Fixed::ZERO, |point| point.height));
    }
    // SIM-70: absent authored flags keep the prior digest; explicit false is
    // distinct from absence because it excludes a station from recrewing.
    if let Some(facility) = u.facility {
        c.flag("facility", facility);
    }
    if let Some(has_crew) = u.has_crew {
        c.flag("has_crew", has_crew);
    }
    // SIM-39, SIM-55: only when set, so content without the keys keeps its digest.
    if u.sane_crew_loss {
        c.flag("sane_crew_loss", true);
    }
    // `freighterName` (mining station): only when set, so content without the key keeps its digest.
    if let Some(name) = &u.free_unit {
        c.text("free_unit", name);
        if let Some(d) = &u.dock {
            c.raw("dock", &format!("[{}]", [d.offset.0, d.offset.1, d.dir.0, d.dir.1].map(fixed_text).join(",")));
        }
    }
    // SIM-75: only for nebulae, so other units keep their digest.
    if let Some(n) = &u.nebula_effects {
        let flags = [n.disable_weapons, n.disable_shields, n.disable_crew, n.disable_sensors, n.disable_cloak, n.hide_contents];
        c.raw("nebula_effects", &group(
            Canon::new()
                .raw("disable", &flags.map(|f| if f { "1" } else { "0" }).concat())
                .fixed("damage", n.damage)
                .list("crew_loss", [n.crew_loss.threshold, n.crew_loss.shield, n.crew_loss.hull].map(fixed_text))
                .fixed("regen", n.regen)
                .fixed("repair", n.repair)
                .fixed("speed_coefficient", n.speed_coefficient)
                .opt_fixed("speed_limit", n.speed_limit)
                .opt_fixed("max_speed_fraction", n.max_speed_fraction),
        ));
    }
    if u.system_hit_points.iter().any(|f| *f != Fixed::ZERO) {
        c.list("system_hit_points", u.system_hit_points.map(fixed_text));
    }
    if u.casualty != crate::def::CasualtyDef::default() {
        let k = &u.casualty;
        let opt = |v: Option<Fixed>| v.map_or_else(|| "-".to_owned(), fixed_text);
        c.raw("casualty", &group(
            Canon::new()
                .raw("shield_protection", &opt(k.shield_protection))
                .list("system_hit_percent", k.system_hit_percent.map(fixed_text))
                .fixed("crew_hit_percent", k.crew_hit_percent)
                .raw("hull_hit_percent", &opt(k.hull_hit_percent))
                .list("system_crew_loss", k.system_crew_loss.map(fixed_text))
                .fixed("life_support_loss", k.life_support_loss)
                .raw("recrew_rate", &opt(k.recrew_rate)),
        ));
    }
    let repair = &u.repair;
    if repair.system_times.iter().any(Option::is_some)
        || repair.repair_facility.is_some()
        || repair.repair_hardpoint.is_some()
        || repair.repair_side.is_some()
        || repair.can_shipyard_repair.is_some()
        || repair.recycles_in_place.is_some()
        || repair.decommission_facility.is_some()
        || repair.dilithium_recycling.is_some()
        || repair.metal_recycling.is_some()
        || repair.repair_rate.is_some()
        || repair.repairs_shields.is_some()
        || repair.recycle_amounts.is_some()
        || repair.recycle_time.is_some()
    {
        let mut g = Canon::new();
        g
            .list("system_times", repair.system_times.map(|t| t.map_or_else(|| "-".to_owned(), fixed_text)))
            .raw("facility", &repair.repair_facility.map_or_else(|| "-".to_owned(), |b| u8::from(b).to_string()))
            .raw("hardpoint", &repair.repair_hardpoint.as_ref().map_or_else(|| "-".to_owned(), |s| quoted(&s.to_ascii_lowercase())))
            .raw("side", &repair.repair_side.map_or_else(|| "-".to_owned(), |side| side.to_string()))
            .raw("can_shipyard_repair", &repair.can_shipyard_repair.map_or_else(|| "-".to_owned(), |b| u8::from(b).to_string()));
        // SIM-45: each only when set (D10).
        let flag = |b: bool| u8::from(b).to_string();
        if let Some(b) = repair.recycles_in_place {
            g.raw("recycles_in_place", &flag(b));
        }
        if let Some(b) = repair.decommission_facility {
            g.raw("decommission_facility", &flag(b));
        }
        if let Some(v) = repair.dilithium_recycling {
            g.fixed("dilithium_recycling", v);
        }
        if let Some(v) = repair.metal_recycling {
            g.fixed("metal_recycling", v);
        }
        // SIM-41: only when set (D10).
        if let Some(rate) = repair.repair_rate {
            g.fixed("rate", rate);
        }
        if let Some(on) = repair.repairs_shields {
            g.raw("repairs_shields", &u8::from(on).to_string());
        }
        if let Some(a) = repair.recycle_amounts {
            g.list("recycle_amounts", a.map(crate::digest::fixed_text));
        }
        if let Some(t) = repair.recycle_time {
            g.fixed("recycle_time", t);
        }
        c.raw("repair", &group(&mut g));
    }
    // SIM-69: the transport keys the sim reads, only when set.
    {
        let t = &u.transport;
        let d = crate::TransportDef::default();
        if (t.transporter, t.boarding_party_strength, t.disable_shields_on_transport, t.shield_delay)
            != (d.transporter, d.boarding_party_strength, d.disable_shields_on_transport, d.shield_delay)
        {
            let opt = |v: Option<Fixed>| v.map_or_else(|| "-".to_owned(), fixed_text);
            c.raw("transport", &group(
                Canon::new()
                    .raw("transporter", &t.transporter.map_or_else(|| "-".to_owned(), |b| u8::from(b).to_string()))
                    .fixed("party_strength", t.boarding_party_strength)
                    .flag("disable_shields", t.disable_shields_on_transport)
                    .raw("shield_delay", &opt(t.shield_delay)),
            ));
        }
    }
    // `warpSpeed` (SIM-36 seam), only when set: the sim reads it from the
    // definition, so it is a sim-visible rule; content without it keeps its text.
    if m.warp_speed != Fixed::ZERO {
        c.fixed("warp_speed", m.warp_speed);
    }
    // SIM-64: Armada I's warp-on-move keys, only when set (Armada II and Fleet Operations have none).
    if m.too_close_for_warp != Fixed::ZERO {
        c.fixed("too_close_for_warp", m.too_close_for_warp);
        c.fixed("heading_close_for_warp", m.heading_close_for_warp);
    }
    // `attackPower` (SIM-36), only when set, for the same reason.
    if let Some(p) = u.attack_power {
        c.fixed("attack_power", p);
    }
    // SIM-30: the population level cap of a producer planet, only when set (Native fake hash unchanged).
    if u.planet_max_population != 0 {
        c.int("planet_max_population", i32::from(u.planet_max_population));
    }
    // SIM-62: orbital placement reads both, so they are sim-visible rules; only when set.
    if u.can_have_orbital_station {
        c.flag("can_have_orbital_station", true);
    }
    // SIM-44: the cap file the build gate reads, only when set.
    if let Some(cap) = &u.fleet_cap {
        let members: Vec<String> = cap.members.iter().map(|(o, v)| format!("{o}:{v}")).collect();
        c.raw("fleet_cap", &format!("{} {} [{}]", cap.name, cap.max, members.join(" ")));
    }
    // CON-52: the Borg spin keys, only when set (other content keeps its text).
    if let Some(b) = &u.a1_borg {
        c.raw("a1_borg", &crate::a1_physics::borg_text(b));
    }
    // The class is already hashed; the period only when the ODF set one.
    if let Some(t) = u.orbit_time.filter(|t| *t != Fixed::ZERO) {
        c.fixed("orbit_time", t);
    }
    // CON-51: orbital income keys, only when the ODF carries one, so content without them keeps its digest.
    if let Some(m) = u.station_mining {
        c.raw("station_mining", &group(
            Canon::new()
                .opt_fixed("multiplier", m.multiplier)
                .raw("can_handle", &m.can_handle.map_or_else(|| "-".to_owned(), |h| format!("[{}]", h.map(|b| u8::from(b).to_string()).join(","))))));
    }
    if let Some(t) = &u.trade_station {
        c.raw("trade_station", &group(
            Canon::new()
                .fixed("transfer_rate", t.transfer_rate)
                .fixed("profit_share", t.profit_share)
                .list("goods", t.goods.iter().map(|g| group(
                    Canon::new().text("key", &g.key).fixed("value", g.value),
                ))),
        ));
    }
    if let Some(capacity) = u.trade_cargo_capacity {
        c.fixed("trade_cargo_capacity", capacity);
    }
    if let Some(n) = u.node_income {
        c.raw("node_income", &group(
            Canon::new()
                .opt_fixed("mining_rate", n.mining_rate)
                .opt_fixed("trickle_amount", n.trickle_amount)
                .opt_fixed("trickle_fraction", n.trickle_fraction)
                .raw("infinite", &n.infinite.map_or_else(|| "-".to_owned(), |b| u8::from(b).to_string()))
                .list("amounts", n.amounts.map(|a| a.map_or_else(|| "-".to_owned(), fixed_text)))));
    }
    // SIM-65: `addToPlanner = 0` leaves the unit out of path planning (read by `sta_sim::nav`); only when set.
    if let Some(on) = u.stats.add_to_planner {
        c.raw("add_to_planner", if on { "1" } else { "0" });
    }
    let push_items: Vec<_> = u.stats.push_build_items.iter().map(|name| quoted(name)).collect();
    if !push_items.is_empty() {
        c.list("push_build_items", push_items);
    }
    // AI-05: the Roots planner treats definitions with the same positive tag as one build item.
    if db.profile() == Profile::Fo4
        && let Some(tag) = u.stats.distance_below_grid.filter(|tag| *tag > Fixed::ZERO)
    {
        c.fixed("distance_below_grid", tag);
    }
    // `canAttack = 0` keeps a class from taking up a fight by itself (read by `sta_sim::movement::approach`); only when set to 0.
    if u.stats.can_attack == Some(false) {
        c.raw("can_attack", "0");
    }
    if db.a1_officers() {
        if let Some(base) = u.a1_base_officers {
            let upgrade = base.upgrade.and_then(|key| db.name(key)).map_or_else(|| "-".to_owned(), quoted);
            let mut rule = Canon::new();
            rule.int("gain", base.gain).int("maximum_upgrades", base.maximum_upgrades).raw("upgrade", &upgrade);
            c.raw("a1_base_officers", &group(&mut rule));
        }
        if let Some(gain) = u.a1_officer_upgrade_gain {
            c.int("a1_officer_upgrade_gain", gain);
        }
    }
    // SIM-73: the shooter's planet keys, only under the target-mask switch and only when set.
    if db.unit_features(u).weapon_target_masks && (u.stats.can_attack_planets.is_some() || u.stats.can_only_attack_planets.is_some()) {
        let flag = |b: Option<bool>| b.map_or_else(|| "-".to_owned(), |b| u8::from(b).to_string());
        c.raw("attack_planets", &group(Canon::new().raw("can", &flag(u.stats.can_attack_planets)).raw("only", &flag(u.stats.can_only_attack_planets))));
    }
    c.finish()
}

/// A start helper's replacement rule (SIM-28).
fn replace_text(r: &crate::lifecycle::ReplaceDef) -> String {
    let crate::lifecycle::ReplaceDef { instant, delay, mode, required_officers, entries, options } = r;
    let entries = entries.iter().map(|e| {
        let classes = e.classes.iter().map(|c| {
            group(Canon::new().text("odf", &c.odf.to_ascii_lowercase()).int("weight", c.weight).list("offset", c.offset.map(fixed_text)))
        });
        group(Canon::new().raw("condition", &e.condition.as_deref().map_or_else(|| "-".into(), quoted)).list("classes", classes))
    });
    let mut c = Canon::new();
    c
            .list("instant", instant.map(|b| u8::from(b).to_string()))
            .int("delay", *delay)
            .int("mode", *mode)
            .raw("required_officers", &required_officers.map_or_else(|| "-".into(), |n| n.to_string()))
            .list("entries", entries);
    if options != &crate::lifecycle::ReplaceOptions::default() {
        if options.ignore_fleet_cap {
            c.flag("ignore_fleet_cap", true);
        }
        c.flag("keep_name", options.keep_name)
            .flag("keep_owner", options.keep_owner)
            .flag("reset_crew", options.reset_crew)
            .flag("reset_energy", options.reset_energy)
            .flag("maintain_orders", options.maintain_orders)
            .flag("ignore_same_class", options.ignore_same_class)
            .flag("ignore_tech_tree", options.ignore_tech_tree)
            .flag("find_open_spot", options.find_open_spot)
            .flag("warp_in", options.warp_in)
            .flag("transfer_stats", options.transfer_stats)
            .int("select_mode", options.select_mode)
            .int("concurrent_children", options.concurrent_children as i128)
            .int("cycle", options.cycle)
            .list("cost", options.cost.map(fixed_text))
            .fixed("crew_cost", options.crew_cost)
            .fixed("warp_speed", options.warp_speed);
    }
    group(&mut c)
}

fn weapon_text(db: &ContentDb, w: &WeaponDef) -> String {
    // Every field, as in `unit_text`.
    let WeaponDef {
        key: _, odf: _, class: _, kind: _, range: _, damage: _, refire: _, shot_speed: _,
        duration: _, fire_sound: _, fx: _, shot_scale: _, look: _, effect_ticks: _, energy_drain: _, energy_cost: _, a1_special: _, a2_special: _,
        disable_weapons: _, decloak_on_hit: _, initial_shot_delay_modifier: _,
        ignore_shot_delay_effects: _, ordnance_defence: _, activate_on_team_change: _, crew_loss: _, damage_variance: _, resource: _, replace: _, colonize: _, base: _, features: _, raw: _,
    } = w;
    // `base` (CON-54): only the keys a sim rule reads are digested, and only for a
    // weapon whose switch for that rule is on (SIM-73, below); the rest is typed only.
    let mut c = Canon::new();
    c
        .text("class", &w.class.0)
        .raw("kind", match w.kind {
            crate::WeaponKind::Beam => "beam",
            crate::WeaponKind::Projectile => "projectile",
            crate::WeaponKind::PointDefence => "point_defence",
        })
        .fixed("range", w.range)
        .fixed("damage", w.damage)
        .int("refire", w.refire)
        .fixed("shot_speed", w.shot_speed)
        .int("duration", w.duration)
        .int("effect_ticks", w.effect_ticks)
        .fixed("energy_drain", w.energy_drain)
        .fixed("energy_cost", w.energy_cost)
        .flag("disable_weapons", w.disable_weapons)
        .flag("decloak_on_hit", w.decloak_on_hit)
        // Signed; absent is not zero (SIM-35).
        .opt_fixed("initial_shot_delay_modifier", w.initial_shot_delay_modifier)
        .flag("ignore_shot_delay_effects", w.ignore_shot_delay_effects)
        .flag("ordnance_defence", w.ordnance_defence)
        .raw("resource", &w.resource.map_or_else(|| "-".into(), |r| group(
            Canon::new()
                .list("add", r.add.map(fixed_text))
                .fixed("crew", r.crew)
                .flag("one_time", r.one_time)
                .flag("always_player", r.always_player)
                .flag("always_ai", r.always_ai),
        )))
        .raw("replace", &w.replace.as_ref().map_or_else(|| "-".into(), replace_text))
        .raw("features", &features_text(db.weapon_features(w)));
    // SIM-29, SIM-39: only when set, so content without the keys keeps its digest.
    if w.activate_on_team_change {
        c.flag("activate_on_team_change", true);
    }
    if let Some(l) = w.crew_loss {
        c.raw("crew_loss", &group(Canon::new().fixed("threshold", l.threshold).fixed("shield", l.shield).fixed("hull", l.hull)));
    }
    if w.damage_variance != Fixed::ZERO {
        c.fixed("damage_variance", w.damage_variance);
    }
    // SIM-30: a colonizer pod's payload and range, only when the ordnance is one.
    if let Some(p) = w.colonize {
        c.raw("colonize", &group(Canon::new().fixed("party_size", p.party_size).fixed("orbit_range", p.orbit_range)));
    }
    if let Some(s) = w.a1_special {
        c.raw("a1_special", &group(Canon::new()
            .raw("kind", match s.kind {
                crate::A1SpecialKind::Mines => "mines",
                crate::A1SpecialKind::ChainPulsar => "chain_pulsar",
                crate::A1SpecialKind::EngineOverload => "engine_overload",
                crate::A1SpecialKind::Reflector => "reflector",
                crate::A1SpecialKind::DetectCloak => "detect_cloak",
                crate::A1SpecialKind::Gemini => "gemini",
                crate::A1SpecialKind::ShieldDisruptor => "shield_disruptor",
                crate::A1SpecialKind::EngineeringTeam => "engineering_team",
                crate::A1SpecialKind::PointDefense => "point_defense",
                crate::A1SpecialKind::Stasis => "stasis",
            })
            .int("duration_ticks", s.duration_ticks)
            .int("flight_ticks", s.flight_ticks)
            .fixed("radius", s.radius)
            .int("limit", s.limit)
            .fixed("bonus_damage", s.bonus_damage)
            .flag("reflect_missile", s.reflect_missile)
            .flag("reflect_beam", s.reflect_beam)
            .int("hit_condition", s.hit_condition)
            .fixed("shields_returned", s.shields_returned)
            .int("detect_class", s.detect_class)));
        if let Some((reflect, return_to_source)) = s.reflect_chance {
            c.raw("reflection", &group(Canon::new().fixed("reflect", reflect).fixed("return", return_to_source)));
        }
    }
    if let Some(s) = w.a2_special {
        let mut special = Canon::new();
        special.raw("target", match s.target {
            crate::A2Target::Immediate => "immediate",
            crate::A2Target::Entity => "entity",
            crate::A2Target::Point => "point",
        });
        special.raw("effect", match s.effect {
            crate::A2Effect::Myotron => "myotron",
            crate::A2Effect::SensorJammer => "sensor_jammer",
            crate::A2Effect::Polaron => "polaron",
            crate::A2Effect::RestoreShields(_) => "restore_shields",
            crate::A2Effect::RestoreEnergy(_) => "restore_energy",
            crate::A2Effect::Remodulate => "remodulate",
            crate::A2Effect::PsychonicBlast(_) => "psychonic_blast",
            crate::A2Effect::MicroDrain { .. } => "micro_drain",
            crate::A2Effect::SelfDestruct(_) => "self_destruct",
            crate::A2Effect::Commando(_) => "commando",
            crate::A2Effect::Regeneration(_) => "regeneration",
            crate::A2Effect::Plasma => "plasma",
            crate::A2Effect::HoldingBeam { .. } => "holding_beam",
            crate::A2Effect::AutoAssimilator(_) => "auto_assimilator",
            crate::A2Effect::ShieldProjector(_) => "shield_projector",
            crate::A2Effect::ShieldInversion(_) => "shield_inversion",
            crate::A2Effect::DeathChant { .. } => "death_chant",
        });
        match s.effect {
            crate::A2Effect::RestoreShields(value)
            | crate::A2Effect::RestoreEnergy(value)
            | crate::A2Effect::PsychonicBlast(value)
            | crate::A2Effect::SelfDestruct(value)
            | crate::A2Effect::Commando(value)
            | crate::A2Effect::Regeneration(value)
            | crate::A2Effect::AutoAssimilator(value)
            | crate::A2Effect::ShieldProjector(value)
            | crate::A2Effect::ShieldInversion(value) => { special.fixed("value", value); }
            crate::A2Effect::HoldingBeam { delay, interval, party } => {
                special.int("delay", delay).int("interval", interval).fixed("party", party);
            }
            crate::A2Effect::DeathChant { fire, repair, turn, damage, speed, hit } => {
                special.fixed("fire", fire).fixed("repair", repair).fixed("turn", turn)
                    .fixed("damage", damage).fixed("speed", speed).fixed("hit", hit);
            }
            crate::A2Effect::MicroDrain { system, per_second } => {
                special.int("system", system).fixed("per_second", per_second);
            }
            _ => {}
        }
        special.int("duration", s.duration)
            .int("flight", s.flight)
            .int("max_targets", s.max_targets)
            .int("hit_condition", s.hit_condition)
            .flag("projectile", s.projectile);
        c.raw("a2_special", &group(&mut special));
    }
    // SIM-73: the keys of the hit roll and of per-target damage, only under their
    // switches and only when set, so every other weapon keeps its text.
    let table = |l: &crate::Lookup| group(Canon::new().fixed("base", l.base).list("rows", l.iter().map(|(name, v)| format!("{}:{}", quoted(name), fixed_text(v)))));
    let features = db.weapon_features(w);
    if features.weapon_hit_roll {
        let t = &w.base.to_hit;
        if let Some(l) = &w.base.hit_chance {
            c.raw("hit_chance", &table(l));
        }
        if [t.stop_to_hit, t.to_hit_stop, t.impulse_to_hit, t.to_hit_impulse].iter().any(Option::is_some) {
            c.raw("to_hit", &group(
                Canon::new()
                    .opt_fixed("stop_to_hit", t.stop_to_hit)
                    .opt_fixed("to_hit_stop", t.to_hit_stop)
                    .opt_fixed("impulse_to_hit", t.impulse_to_hit)
                    .opt_fixed("to_hit_impulse", t.to_hit_impulse)));
        }
    }
    if features.weapon_target_damage
        && let Some(l) = &w.base.damage_base
    {
        c.raw("damage_rows", &table(l));
    }
    if features.weapon_target_damage
        && let Some(l) = &w.base.damage_variance
    {
        c.raw("variance_rows", &table(l));
    }
    // The weapon's `special` key (read by `sta_sim::movement::approach`); only when set to 1.
    if w.base.special == Some(true) {
        c.raw("special", "1");
    }
    if features.weapon_cloak_fire && w.base.cloak_enable == Some(true) {
        c.raw("cloak_enable", "1");
    }
    if features.weapon_target_masks && w.base.targets != crate::TargetMaskDef::default() {
        let m = &w.base.targets;
        let flag = |b: Option<bool>| b.map_or_else(|| "-".to_owned(), |b| u8::from(b).to_string());
        c.raw("targets", &group(
            Canon::new()
                .raw("enemy_craft", &flag(m.enemy_craft))
                .raw("enemy_buildings", &flag(m.enemy_buildings))
                .raw("allied_craft", &flag(m.allied_craft))
                .raw("allied_buildings", &flag(m.allied_buildings))
                .raw("enemy_planets_only", &flag(m.enemy_planets_only))
                .raw("location", &flag(m.location))));
    }
    c.finish()
}

/// A unit's collision geometry as the sim uses it: today the bounding
/// radius (SIM-21; zero when no model was read). SIM-32 widens the value to
/// the ellipsoid axes, radius and centre offset; the key stays.
fn geometry_text(u: &UnitDef) -> String {
    Canon::new().fixed("radius", u.radius).finish()
}

fn condition_text(c: &crate::Condition) -> String {
    use crate::Condition as C;
    match c {
        C::Have { odf, must_not: false } => format!("have:{}", quoted(odf)),
        C::Have { odf, must_not: true } => format!("lack:{}", quoted(odf)),
        C::Ally(r) => format!("ally:{}", quoted(r)),
        C::Race(r) => format!("race:{}", quoted(r)),
        // The tooltips are display text.
        C::Unique { max, .. } => format!("unique:{max}"),
        C::CapList(n) => format!("cap:{}", quoted(n)),
    }
}

/// A tech tree entry's gate: `{withheld=0;dependency_build_possible=1;requirements=[...]}`.
fn tech_text(e: &crate::TechEntry) -> String {
    let reqs = e.requirements.iter().map(|r| {
        group(
            Canon::new()
                .raw("group", &r.group.map_or_else(|| "-".into(), |g| g.to_string()))
                .flag("hide", r.hide_unavailable)
                .list("when", r.conditions.iter().map(condition_text)),
        )
    });
    group(
        Canon::new()
            .flag("withheld", e.withheld)
            .flag("dependency_build_possible", e.dependency_build_possible)
            .list("requirements", reqs),
    )
}

/// Every resolved sim-visible rule of `db` (see [`crate::digest`]).
fn digest_entries(db: &ContentDb) -> Vec<DigestEntry> {
    let mut out = db.rules.digest_entries();
    out.push(DigestEntry::new("content:profile", db.lineage()));
    out.push(DigestEntry::new("content:features", features_text(&db.features)));
    if db.a1_officers() {
        out.push(DigestEntry::new("content:a1-officer-progression", "enabled"));
    }
    if let Some(maximum) = db.a1_starting_officers() {
        out.push(DigestEntry::new("content:a1-max-officers", maximum.to_string()));
    }
    // CON-52: only when the model has its block, so every other content keeps its digest.
    if let Some(p) = db.a1_play_physics() {
        out.push(DigestEntry::new("content:a1-play-physics", crate::a1_physics::dynamics_text(p)));
    }
    out.push(DigestEntry::new("content:tech-tree", if db.tech.is_some() { "gated" } else { "none" }));
    out.push(DigestEntry::new("resources:slots", format!("[{}]", crate::resources::KEYS.join(","))));
    out.push(DigestEntry::new("resources:aliases", db.resources.aliases_text()));
    // A definition's tech tree entry is part of its own entry (the `tech`
    // field), not an entry of its own: Fleet Operations 4.0 has ~5,800 of
    // them for its definitions, and with separate keys its rule list would
    // come within 15% of the wire's bound (NET-01: 16,384). Entries for
    // ODFs that are no definition here gate nothing the sim can meet:
    // nothing can build or own such an ODF, so a condition on it holds or
    // fails alike whatever the entry says (FO 4.0's tree has ~3,400).
    let tech = |name: &str| db.tech.as_ref().and_then(|t| t.entry(name)).map_or_else(|| "-".to_owned(), tech_text);
    for (name, def) in &db.defs {
        match def {
            Def::Unit(u) => {
                let value = format!("{};tech={}", unit_text(db, u), tech(name));
                out.push(DigestEntry::new(digest::key("unit:", name), value));
                out.push(DigestEntry::new(digest::key("geometry:", name), geometry_text(u)));
                if let Some(attachment) = u.repair_attachment {
                    let vector = |v: sta_core::Vec3| format!("[{},{},{}]", fixed_text(v.x), fixed_text(v.y), fixed_text(v.z));
                    let value = Canon::new()
                        .raw("origin", &vector(attachment.origin))
                        .list("axes", attachment.axes.map(vector))
                        .finish();
                    out.push(DigestEntry::new(digest::key("repair-geometry:", name), value));
                }
            }
            Def::Weapon(w) => {
                let value = format!("{};tech={}", weapon_text(db, w), tech(name));
                out.push(DigestEntry::new(digest::key("weapon:", name), value));
            }
        }
    }
    if let Some(t) = &db.tech {
        for c in t.capacity_lists() {
            let value = Canon::new()
                .int("max", c.max)
                .list("members", c.members.iter().map(|(odf, n)| format!("{}:{n}", quoted(odf))))
                .finish();
            // Keyed by lower-cased name in the tree, so unique.
            out.push(DigestEntry::new(digest::key("techcap:", &c.name.to_ascii_lowercase()), value));
        }
    }
    out
}

fn digest_of(db: &ContentDb) -> ContentDigest {
    ContentDigest::from_entries(digest_entries(db))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::def::WeaponKind;

    /// Written out from the text the previous code (a read of `raw` in `unit_text`) produced.
    const EXPECTED_YARD_TEXT: &str = "class=\"shipyard\";race=\"\";starbase=0;planet_role=-;planet_crew_gain=0;planet_crew_modifier=-;planet_colonizable=0;planet_civilian_population_modifier=0;movement=[0,1,0,1,0,0,0,50,150,20,0,70];hull=100;shields=0;shield_rate=0;initial_hull=100;initial_shields=0;crew=0;crew_capacity=0;sensor=0;hardpoints=[];cost=[];build_ticks=0;officers=0;queue_limit=5;builds=[];flagged_slots=-;launch=-;special_energy=0;special_energy_rate=0;miner=0;max_dilithium=0;mining_rate=0;transfer_rate=0;resource=-;nebula_pathing=-;avoid_me=0;hidden=0;invalid_as_target=0;life=-;crew_gain=0;features=[];confinement=-;push_build_items=[\"alpha\",\"beta\",\"alpha\"]";

    #[test]
    fn keys_follow_sorted_names_whatever_the_insertion_order() {
        let a = ContentDb::builder()
            .unit(UnitDef::new("Zeta", "craft"))
            .unit(UnitDef::new("alpha", "craft"))
            .weapon(WeaponDef::new("mid", "phaser", WeaponKind::Beam))
            .build()
            .unwrap();
        let b = ContentDb::builder()
            .weapon(WeaponDef::new("mid", "phaser", WeaponKind::Beam))
            .unit(UnitDef::new("alpha", "craft"))
            .unit(UnitDef::new("Zeta", "craft"))
            .build()
            .unwrap();
        assert_eq!(a.key("alpha"), Some(ContentKey(0)));
        assert_eq!(a.key("MID"), Some(ContentKey(1)));
        assert_eq!(a.key("zeta"), Some(ContentKey(2)));
        assert_eq!(a.unit(ContentKey(2)).unwrap().key, ContentKey(2));
        assert!(a.unit(ContentKey(1)).is_none());
        assert!(a.weapon(ContentKey(1)).is_some());
        assert_eq!(a.hash(), b.hash());
    }

    /// The push-build items come from the loaded definition's typed field; the
    /// unit text (and so the content digest) is the text the raw-ODF read gave.
    #[test]
    fn push_build_items_digest_text_is_the_authored_text() {
        let odf = sta_odf::resolve("yard", &mut |_| {
            Some("classLabel = \"shipyard\"\nmaxHealth = 100\npushBuildItem0 = Alpha.odf\npushBuildItem2 = NULL\npushBuildItem3 = beta\npushBuildItem4 = alpha\n".into())
        })
        .unwrap();
        let mut warnings = Vec::new();
        let def = crate::unit::unit_def("yard", &odf, None, &mut warnings);
        let db = ContentDb::builder().unit(def.clone()).build().unwrap();
        assert_eq!(unit_text(&db, &def), EXPECTED_YARD_TEXT);
    }

    #[test]
    fn duplicates_and_dangling_references_fail() {
        let dup = ContentDb::builder().unit(UnitDef::new("a", "craft")).unit(UnitDef::new("A", "craft")).build();
        assert_eq!(dup.unwrap_err(), ContentError::Duplicate("a".into()));
        let mut u = UnitDef::new("yard", "shipyard");
        u.build.list.push(DefRef::new("ghost"));
        let dangling = ContentDb::builder().unit(u).build();
        assert!(matches!(dangling, Err(ContentError::Dangling { .. })));
    }

    #[test]
    fn load_of_an_empty_install_is_empty() {
        let (db, warnings) = ContentDb::load_as(&sta_vfs::Vfs::default(), Profile::Fo4, &OdfIndex::default()).unwrap();
        assert!(db.is_empty() && warnings.is_empty());
        assert_eq!(db.profile(), Profile::Fo4);
    }

    /// `fake::db()` with `patch` applied to every definition.
    fn patched(unit: impl Fn(&mut UnitDef), weapon: impl Fn(&mut WeaponDef)) -> ContentDb {
        let fake = crate::fake::db();
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for u in fake.units() {
            let mut u = u.clone();
            unit(&mut u);
            b = b.unit(u);
        }
        for w in fake.weapons() {
            let mut w = w.clone();
            weapon(&mut w);
            b = b.weapon(w);
        }
        b.build().unwrap()
    }

    /// The first differing key between the fake and a patched copy.
    fn first_key(unit: impl Fn(&mut UnitDef), weapon: impl Fn(&mut WeaponDef)) -> Option<String> {
        crate::fake::db().digest().first_difference(patched(unit, weapon).digest()).map(|d| d.key)
    }

    #[test]
    fn display_strings_and_render_data_are_not_in_the_digest() {
        let fake = crate::fake::db();
        let art = patched(
            |u| {
                u.model = "hd_model".into();
                // Only the field: a real scale change moves the radius, which
                // `geometry:` compares (sta_net's rules_handshake tests that
                // render-only models leave the geometry alone).
                u.scale = Fixed::from_int(3);
                u.sounds.select = vec!["new_voice".into()];
                u.fx.explosion = Some("big_boom".into());
                for h in &mut u.hardpoints {
                    h.node = "renamed_node".into();
                }
                if let Some(l) = &mut u.launch {
                    l.node = Some("other".into());
                }
            },
            |w| {
                w.fire_sound = Some("pew".into());
                w.fx = Some("beam_hd".into());
            },
        );
        assert_eq!(art.digest(), fake.digest());
        // Slot names are display text; pack names are informational.
        let table = crate::StringTable::parse("\"GUI_RD_LATINUM_VTOOLTIP\", \"Gold-pressed: currency\",\n");
        let names = ResourceModel::from_strings(fake.profile(), &table);
        assert_eq!(names.name(crate::resources::LATINUM), "Gold-pressed");
        let renamed = crate::fake::db().with_resources(names).with_packs(vec!["Some Pack".into()]);
        assert_eq!(renamed.digest(), fake.digest());
    }

    #[test]
    fn every_sim_visible_value_names_its_definition() {
        let cruiser = |f: fn(&mut UnitDef)| move |u: &mut UnitDef| if u.odf == "fx_cruiser" { f(u) };
        let none = |_: &mut WeaponDef| {};
        assert_eq!(first_key(cruiser(|u| u.hull = u.hull + Fixed::ONE), none).as_deref(), Some("unit:fx_cruiser"));
        assert_eq!(first_key(cruiser(|u| u.race = "other".into()), none).as_deref(), Some("unit:fx_cruiser"));
        assert_eq!(first_key(cruiser(|u| u.avoid_me = true), none).as_deref(), Some("unit:fx_cruiser"));
        assert_eq!(first_key(cruiser(|u| u.build.flagged_slots = Some(Vec::new())), none).as_deref(), Some("unit:fx_cruiser"));
        assert_eq!(first_key(cruiser(|u| u.special_energy_max = Fixed::ONE), none).as_deref(), Some("unit:fx_cruiser"));
        assert_eq!(first_key(cruiser(|u| u.radius = u.radius + Fixed::ONE), none).as_deref(), Some("geometry:fx_cruiser"));
        let beam = |w: &mut WeaponDef| if w.odf == "fx_beam" { w.energy_cost = Fixed::ONE };
        assert_eq!(first_key(|_| {}, beam).as_deref(), Some("weapon:fx_beam"));
        // An absent key and its default are the same rule.
        assert_eq!(first_key(cruiser(|u| u.initial_hull = Some(u.hull)), none), None);
        assert_eq!(first_key(cruiser(|u| u.maximum_crew = Some(u.crew)), none), None);
        // The field is named for a finer message.
        let d = crate::fake::db().digest().first_difference(patched(cruiser(|u| u.sensor = Fixed::from_int(7)), none).digest()).unwrap();
        assert_eq!(d.field().map(|f| (f.0, f.2)), Some(("sensor".to_owned(), Some("7".to_owned()))));
    }

    #[test]
    fn the_tech_tree_is_in_each_definition_and_its_tooltips_are_not() {
        let tree = |text: &str| TechTree::parse("t.xml", text).0;
        let xml = |tip: &str| format!(
            "<techtree><object ODF=\"fx_cruiser.odf\"><requirement><cap maxCapValue=\"2\" tooltip=\"{tip}\"/></requirement></object>\
             <object ODF=\"not_a_def.odf\" buildPossible=\"false\"/></techtree>"
        );
        let a = crate::fake::db().with_tech_tree(tree(&xml("Only two")));
        let b = crate::fake::db().with_tech_tree(tree(&xml("Nur zwei")));
        assert_eq!(a.digest(), b.digest());
        assert!(a.digest().get("unit:fx_cruiser").unwrap().contains("tech={withheld=0"), "{:?}", a.digest().get("unit:fx_cruiser"));
        assert_ne!(a.hash(), crate::fake::db().hash());
        // An entry for an ODF that is no definition gates nothing.
        let c = crate::fake::db().with_tech_tree(tree(
            "<techtree><object ODF=\"fx_cruiser.odf\"><requirement><cap maxCapValue=\"2\" tooltip=\"x\"/></requirement></object></techtree>",
        ));
        assert_eq!(a.digest(), c.digest());
        assert_eq!(a.digest().first_difference(crate::fake::db().digest()).unwrap().key, "content:tech-tree");
    }

    #[test]
    fn every_key_is_unique_bounded_and_the_digest_is_stable() {
        let db = crate::fake::db();
        let d = db.digest();
        assert!(d.entries().windows(2).all(|w| w[0].key.as_bytes() < w[1].key.as_bytes()));
        assert!(d.entries().iter().all(|e| e.key.len() <= digest::MAX_KEY_BYTES));
        for k in ["content:profile", "content:features", "content:tech-tree", "resources:slots", "resources:aliases", "RTS_CFG.h:MP_OFFICER_LIMIT", "geometry:fx_cruiser", "unit:fx_cruiser", "weapon:fx_beam"] {
            assert!(d.get(k).is_some(), "{k}");
        }
        assert_eq!(d.get("content:profile"), Some(db.lineage()));
        // The value itself is pinned in fake.rs (and checked on four OSes).
        assert_eq!(db.hash(), d.hash64());
    }
}
