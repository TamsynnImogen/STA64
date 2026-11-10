//! The content database (S5): every definition under one name-sorted table.

use std::sync::Arc;

use sta_core::{ContentKey, Fixed};

use crate::def::{DefRef, UnitDef, WeaponDef};
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
    /// What the game calls its stock slots (display only: not in the hash).
    resources: ResourceModel,
    /// What may be built given what is owned (CON-06); `None` allows
    /// everything, as the fakes do.
    tech: Option<Arc<TechTree>>,
    hash: u64,
}

impl ContentDb {
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
        ContentDb::load_as(vfs, Profile::detect(vfs), index)
    }

    /// [`load`](ContentDb::load) with the profile given instead of detected
    /// (a lobby choice, a test).
    pub fn load_as(vfs: &sta_vfs::Vfs, profile: Profile, index: &OdfIndex) -> Result<(ContentDb, Vec<Warning>)> {
        let (db, mut warnings) = crate::odf::load(vfs, profile, index)?;
        let db = db.with_launch_points(vfs).with_resources(ResourceModel::from_strings(profile, &crate::StringTable::load(vfs)));
        if db.is_empty() {
            return Ok((db, warnings));
        }
        // The skirmish tree: `tech1`, from the top-most layer that has one.
        // A map may name another (CON-07/UI); `with_tech_tree` swaps it.
        match TechTree::from_vfs(vfs, DEFAULT_TECH_TREE) {
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
    /// read from its model (SIM-16), and the hash recomputed.
    pub fn with_launch_points(mut self, vfs: &sta_vfs::Vfs) -> ContentDb {
        crate::launch::attach(vfs, self.defs.iter_mut().filter_map(|(_, d)| if let Def::Unit(u) = d { Some(u) } else { None }));
        crate::launch::attach_radii(vfs, self.defs.iter_mut().filter_map(|(_, d)| if let Def::Unit(u) = d { Some(u) } else { None }));
        self.hash = hash(&self);
        self
    }

    /// This database gated by `tree` (replacing any), with the hash
    /// recomputed.
    pub fn with_tech_tree(mut self, tree: TechTree) -> ContentDb {
        self.tech = Some(Arc::new(tree));
        self.hash = hash(&self);
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

    /// FNV-1a over the sorted names, the profile, the features and every
    /// number the sim reads. Peers compare it before a game; it is folded
    /// into tick 0's digest (SIM-01).
    pub fn hash(&self) -> u64 {
        self.hash
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
        let mut db = ContentDb { defs, profile: self.profile, features: self.features, resources, tech: self.tech.map(Arc::new), hash: 0 };
        db.hash = hash(&db);
        Ok(db)
    }
}

/// FNV-1a, the same function as `Sim::digest`.
struct Fnv(u64);

impl Fnv {
    fn byte(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(0x100_0000_01b3);
    }
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }
    fn fixed(&mut self, v: Fixed) {
        self.u64(v.to_bits() as u64);
    }
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        for b in s.bytes() {
            self.byte(b);
        }
    }
}

/// A def's own switches, hashed only when they differ from the database-wide
/// set, so fakes and defs without overrides hash as they did before.
fn feature_bits(h: &mut Fnv, own: Option<&Features>, db: &Features) {
    if let Some(f) = own.filter(|f| *f != db) {
        h.byte(0xe3);
        for b in f.bits() {
            h.byte(b as u8);
        }
    }
}

fn hash(db: &ContentDb) -> u64 {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    h.u64(db.profile as u64);
    for b in db.features.bits() {
        h.byte(b as u8);
    }
    h.u64(db.defs.len() as u64);
    for (name, def) in &db.defs {
        h.str(name);
        match def {
            Def::Unit(u) => {
                h.byte(0);
                h.str(&u.class.0);
                // Native default false keeps the old hash path. A true
                // legacy flag must participate before mission queries use it.
                if u.is_starbase {
                    h.byte(0xe6);
                    h.byte(1);
                }
                let m = &u.movement;
                for f in [u.scale, m.max_speed, m.forward_accel, m.turn_omega, m.turn_alpha, m.turn_at_rest] {
                    h.fixed(f);
                }
                // Smooth-physics keys (SIM-15) only when set, so content
                // without them hashes as before.
                if m.has_smooth_keys() {
                    for f in m.smooth_fields() {
                        h.fixed(f);
                    }
                }
                for f in [u.hull, u.shields.max, u.shields.recharge, u.crew, u.sensor] {
                    h.fixed(f);
                }
                h.u64(u.hardpoints.len() as u64);
                for hp in &u.hardpoints {
                    h.u64(hp.weapon.key.0 as u64);
                    match hp.arc {
                        Some(a) => {
                            h.byte(1);
                            for f in [a.dir.0, a.dir.1, a.min_dot] {
                                h.fixed(f);
                            }
                        }
                        None => h.byte(0),
                    }
                }
                h.u64(u.build.cost.len() as u64);
                for &c in &u.build.cost {
                    h.fixed(c);
                }
                // Only when set, so content without launch points keeps its hash.
                if let Some(l) = &u.launch {
                    h.byte(2);
                    for f in [l.offset.0, l.offset.1, l.dir.0, l.dir.1, l.clear] {
                        h.fixed(f);
                    }
                }
                // Only when read, so content without models keeps its hash.
                if u.radius > Fixed::ZERO {
                    h.byte(3);
                    h.fixed(u.radius);
                }
                h.u64(u.build.ticks);
                h.fixed(u.build.officers);
                // The old shared rule was five for every profile. Preserve
                // its hashes only when the effective rule is still five.
                let queue_limit = db.build_queue_limit(u);
                if queue_limit != 5 {
                    h.byte(0xe5);
                    h.u64(u64::from(queue_limit));
                }
                h.u64(u.build.list.len() as u64);
                for b in &u.build.list {
                    h.u64(b.key.0 as u64);
                }
                // Later additions: hashed only when set, so databases made
                // before them (the fakes) keep their hash.
                let extra = [u.special_energy_max, u.special_energy_rate, u.max_dilithium, u.mining_rate, u.transfer_rate];
                if u.miner || u.resource.is_some() || extra.iter().any(|f| *f != Fixed::ZERO) {
                    h.byte(0xe1);
                    h.byte(u.miner as u8);
                    h.u64(u.resource.map_or(0, |r| r as u64 + 1));
                    for f in extra {
                        h.fixed(f);
                    }
                }
                feature_bits(&mut h, u.features.as_ref(), &db.features);
            }
            Def::Weapon(w) => {
                h.byte(1);
                h.str(&w.class.0);
                h.byte(w.kind as u8);
                for f in [w.range, w.damage, w.shot_speed] {
                    h.fixed(f);
                }
                h.u64(w.refire);
                h.u64(w.duration);
                if w.effect_ticks != 0 || w.energy_drain != Fixed::ZERO || w.energy_cost != Fixed::ZERO || !w.disable_weapons || w.decloak_on_hit {
                    h.byte(0xe2);
                    h.u64(w.effect_ticks);
                    h.fixed(w.energy_drain);
                    h.fixed(w.energy_cost);
                    h.byte(w.disable_weapons as u8);
                    h.byte(w.decloak_on_hit as u8);
                }
                feature_bits(&mut h, w.features.as_ref(), &db.features);
            }
        }
    }
    // Hashed only when set, so databases without a tree keep their hash. The
    // entries come out in sorted order, so the Debug text is stable.
    if let Some(t) = &db.tech {
        h.byte(0xe3);
        let entries: Vec<_> = t.entries().collect();
        let caps: Vec<_> = t.capacity_lists().collect();
        h.str(&format!("{entries:?}{caps:?}"));
    }
    h.0
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
}
