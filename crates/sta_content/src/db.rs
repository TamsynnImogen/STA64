//! The content database (S5): every definition under one name-sorted table.

use sta_core::{ContentKey, Fixed};

use crate::def::{DefRef, UnitDef, WeaponDef};
use crate::features::{Features, Profile};
use crate::index::OdfIndex;
use crate::{ContentError, Result, Warning};

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
    hash: u64,
}

impl ContentDb {
    /// Load every definition from a mounted install (CON-03; not yet written).
    pub fn load(vfs: &sta_vfs::Vfs, profile: Profile, index: &OdfIndex) -> Result<(ContentDb, Vec<Warning>)> {
        let _ = (vfs, profile, index);
        Err(ContentError::Unsupported("ContentDb::load from ODFs (CON-03)"))
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

    pub fn features(&self) -> &Features {
        &self.features
    }
}

/// Collects definitions, then sorts them, hands out keys and resolves
/// references in [`build`](ContentBuilder::build).
#[derive(Debug, Clone, Default)]
pub struct ContentBuilder {
    defs: Vec<Def>,
    profile: Profile,
    features: Features,
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
        let mut db = ContentDb { defs, profile: self.profile, features: self.features, hash: 0 };
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
                let m = &u.movement;
                for f in [u.scale, m.max_speed, m.forward_accel, m.turn_omega, m.turn_alpha, m.turn_at_rest] {
                    h.fixed(f);
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
                h.u64(u.build.ticks);
                h.fixed(u.build.officers);
                h.u64(u.build.list.len() as u64);
                for b in &u.build.list {
                    h.u64(b.key.0 as u64);
                }
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
            }
        }
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
    fn load_is_unsupported() {
        let r = ContentDb::load(&sta_vfs::Vfs::default(), Profile::Fo4, &OdfIndex::default());
        assert!(matches!(r, Err(ContentError::Unsupported(_))));
    }
}
