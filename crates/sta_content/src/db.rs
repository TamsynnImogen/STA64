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
    /// The mounted layers' names, top first: informational (D10), recorded
    /// in replays and snapshots, never compared.
    packs: Vec<String>,
    digest: ContentDigest,
}

impl ContentDb {
    pub fn rules(&self) -> &crate::Rules { &self.rules }

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
        let (races, _) = crate::Races::load(vfs, index);
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

    /// This database with every builder's [`LaunchPoint`](crate::LaunchPoint)
    /// read from its model (SIM-16), and the digest recomputed.
    pub fn with_launch_points(self, vfs: &sta_vfs::Vfs) -> ContentDb {
        self.with_launch_points_warned(vfs).0
    }

    /// [`with_launch_points`](ContentDb::with_launch_points) and the warnings
    /// for units whose collision radius could not be read from a SOD.
    pub fn with_launch_points_warned(mut self, vfs: &sta_vfs::Vfs) -> (ContentDb, Vec<Warning>) {
        crate::launch::attach(vfs, self.defs.iter_mut().filter_map(|(_, d)| if let Def::Unit(u) = d { Some(u) } else { None }));
        let warnings = crate::launch::attach_radii(vfs, self.defs.iter_mut().filter_map(|(_, d)| if let Def::Unit(u) = d { Some(u) } else { None }));
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
            map_confinement_warnings, rules: crate::Rules::default(), packs: Vec::new(),
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
    let UnitDef {
        key: _, odf: _, class: _, is_starbase: _, planet_role: _, planet_crew_gain: _, planet_crew_modifier: _,
        planet_colonizable: _, planet_civilian_population_modifier: _, race: _,
        model: _, scale: _, movement: _, hull: _, shields: _, initial_hull: _, initial_shields: _,
        crew: _, maximum_crew: _, sensor: _, hardpoints: _, build: _, launch: _, radius: _,
        sounds: _, fx: _, special_energy_max: _, special_energy_rate: _, miner: _, miner_off: _,
        max_dilithium: _, mining_rate: _, transfer_rate: _, resource: _, features: _,
        confinement: _, nebula_pathing: _, avoid_me: _, hidden: _, invalid_as_target: _, life: _,
        crew_gain: _, raw: _,
    } = u;
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
    // `warpSpeed` (SIM-36 seam), only when set: the sim reads it from the
    // definition, so it is a sim-visible rule; content without it keeps its text.
    if m.warp_speed != Fixed::ZERO {
        c.fixed("warp_speed", m.warp_speed);
    }
    c.finish()
}

/// A start helper's replacement rule (SIM-28).
fn replace_text(r: &crate::lifecycle::ReplaceDef) -> String {
    let crate::lifecycle::ReplaceDef { instant, delay, mode, required_officers, entries } = r;
    let entries = entries.iter().map(|e| {
        let classes = e.classes.iter().map(|c| {
            group(Canon::new().text("odf", &c.odf.to_ascii_lowercase()).int("weight", c.weight).list("offset", c.offset.map(fixed_text)))
        });
        group(Canon::new().raw("condition", &e.condition.as_deref().map_or_else(|| "-".into(), quoted)).list("classes", classes))
    });
    group(
        Canon::new()
            .list("instant", instant.map(|b| u8::from(b).to_string()))
            .int("delay", *delay)
            .int("mode", *mode)
            .raw("required_officers", &required_officers.map_or_else(|| "-".into(), |n| n.to_string()))
            .list("entries", entries),
    )
}

fn weapon_text(db: &ContentDb, w: &WeaponDef) -> String {
    // Every field, as in `unit_text`.
    let WeaponDef {
        key: _, odf: _, class: _, kind: _, range: _, damage: _, refire: _, shot_speed: _,
        duration: _, fire_sound: _, fx: _, effect_ticks: _, energy_drain: _, energy_cost: _,
        disable_weapons: _, decloak_on_hit: _, initial_shot_delay_modifier: _,
        ignore_shot_delay_effects: _, ordnance_defence: _, resource: _, replace: _, features: _, raw: _,
    } = w;
    Canon::new()
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
        .raw("features", &features_text(db.weapon_features(w)))
        .finish()
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
