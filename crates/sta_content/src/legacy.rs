//! TEMPORARY: what `sta64`'s map view did with ODFs before the real loaders
//! exist. CON-03 replaces this module with `ContentDb::load`; delete it then.
//!
//! The map view asks for the ODFs its objects name, one at a time, found by
//! file name across all ODF folders. This does the same and wraps each in a
//! [`UnitDef`]: class, drawn model (`basename`, else the ODF's own name),
//! `ScaleSOD`, and movement from the physics ODF (`physicsFile`). Nothing is
//! checked against a profile and no hardpoints, builds or weapons are read.
//! The resolved ODF stays in [`UnitDef::raw`] for the scenery keys the map
//! reads straight from it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use sta_core::Fixed;

use crate::{ContentDb, MoveStats, UnitDef};

/// ODF lookups, kept across maps like the cache this replaces.
#[derive(Default)]
pub struct Legacy {
    /// Lower-cased file stem to virtual path.
    paths: HashMap<String, String>,
    odfs: HashMap<String, Option<Arc<sta_odf::Odf>>>,
}

/// An ODF name as the lookups key it: lower case, without `.odf`.
fn stem(name: &str) -> String {
    let n = name.trim().to_ascii_lowercase();
    n.strip_suffix(".odf").map(str::to_owned).unwrap_or(n)
}

impl Legacy {
    fn index(&mut self, vfs: &sta_vfs::Vfs) {
        if !self.paths.is_empty() {
            return;
        }
        // ODFs are found by file name across all ODF folders (and A1's addon/).
        for dir in ["odf", "addon"] {
            for p in vfs.list(dir) {
                if let Some(s) = p.strip_suffix(".odf").map(|s| s.rsplit('/').next().unwrap_or(s).to_owned()) {
                    self.paths.entry(s).or_insert(p);
                }
            }
        }
    }

    /// The resolved ODF (includes followed), or `None` if there is none.
    pub fn odf(&mut self, vfs: &sta_vfs::Vfs, name: &str) -> Option<Arc<sta_odf::Odf>> {
        self.index(vfs);
        let key = stem(name);
        if let Some(o) = self.odfs.get(&key) {
            return o.clone();
        }
        let paths = &self.paths;
        let mut load = |n: &str| {
            let n = stem(n);
            paths.get(&n).and_then(|p| vfs.read(p)).and_then(Result::ok).map(|b| String::from_utf8_lossy(&b).into_owned())
        };
        let odf = sta_odf::resolve(&key, &mut load).ok().map(Arc::new);
        self.odfs.insert(key, odf.clone());
        odf
    }

    /// A database of the named ODFs that exist; the rest are simply absent
    /// (`ContentDb::key` says `None`). Names compare ignoring case.
    pub fn db<'a>(&mut self, vfs: &sta_vfs::Vfs, names: impl IntoIterator<Item = &'a str>) -> ContentDb {
        // Sorted and de-duplicated, so the same names give the same database.
        let wanted: BTreeMap<String, &str> = names.into_iter().map(|n| (stem(n), n)).collect();
        let mut builder = ContentDb::builder();
        for (key, given) in wanted {
            let Some(odf) = self.odf(vfs, &key) else { continue };
            let model = odf.str("basename").map(str::trim).filter(|b| !b.is_empty()).unwrap_or(given.trim()).to_owned();
            let mut def = UnitDef::new(given.trim().trim_end_matches(".odf"), odf.class_label().unwrap_or(""));
            def.model = model;
            def.scale = Fixed::from_f64(odf.f32("ScaleSOD").filter(|s| *s > 0.0).unwrap_or(1.0) as f64);
            def.movement = self.movement(vfs, &odf);
            // The typed fields the sim reads instead of the raw ODF (D9).
            def.avoid_me = odf.bool("avoidMe").unwrap_or(false);
            def.hidden = crate::lifecycle::hidden(&odf);
            let energy = |key: &str| Fixed::from_f64(odf.f32(key).filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0) as f64);
            def.special_energy_max = energy("maxSpecialEnergy");
            def.special_energy_rate = energy("specialEnergyRate");
            def.raw = odf;
            builder = builder.unit(def);
        }
        builder.build().expect("legacy definitions have unique names and no references")
    }

    /// Limits from the physics ODF; `STATIC` for craft that do not move.
    fn movement(&mut self, vfs: &sta_vfs::Vfs, odf: &sta_odf::Odf) -> MoveStats {
        let Some(phys) = odf.str("physicsFile").filter(|p| !p.is_empty()) else { return MoveStats::STATIC };
        let Some(ph) = self.odf(vfs, phys) else { return MoveStats::STATIC };
        if !ph.f32("impulseSpeed").is_some_and(|s| s > 0.0) {
            return MoveStats::STATIC;
        }
        let m = MoveStats::from_f32(
            ph.f32("impulseSpeed").unwrap_or(100.0),
            ph.f32("forwardAccel").unwrap_or(1.0),
            ph.f32("turnOmega").unwrap_or(1.0),
            ph.f32("turnAlpha").unwrap_or(1.0),
            ph.f32("turnOmegaFractionAtRest").unwrap_or(0.2),
        );
        crate::unit::with_smooth_keys(m, |k| ph.f32(k).filter(|v| v.is_finite()).map(f64::from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install() -> sta_vfs::Vfs {
        let root = std::env::temp_dir().join(format!("sta_content_legacy_{}", std::process::id()));
        let odf = root.join("odf");
        std::fs::create_dir_all(&odf).unwrap();
        std::fs::write(odf.join("ship.odf"), "[GameObjectClass]\nclassLabel = \"craft\"\nbasename = \"hull_a\"\nphysicsFile = \"ship_phys.odf\"\nScaleSOD = 2.0\n").unwrap();
        std::fs::write(odf.join("ship_phys.odf"), "[Physics]\nimpulseSpeed = 140.0\nturnOmega = 2.0\n").unwrap();
        std::fs::write(odf.join("base.odf"), "[GameObjectClass]\nclassLabel = \"station\"\n").unwrap();
        let mut vfs = sta_vfs::Vfs::default();
        vfs.push_dir("test".into(), root).unwrap();
        vfs
    }

    #[test]
    fn wraps_the_odfs_the_map_names() {
        let vfs = install();
        let mut legacy = Legacy::default();
        let db = legacy.db(&vfs, ["Ship", "base", "missing", "SHIP.odf"]);
        assert_eq!(db.len(), 2);
        let ship = db.unit(db.key("ship").unwrap()).unwrap();
        assert_eq!(ship.model, "hull_a");
        assert_eq!(ship.class.0, "craft");
        assert_eq!(ship.scale, Fixed::from_int(2));
        assert_eq!(ship.movement, MoveStats::from_f32(140.0, 1.0, 2.0, 1.0, 0.2));
        let base = db.unit(db.key("BASE").unwrap()).unwrap();
        assert_eq!(base.model, "base", "no basename: the ODF's own name");
        assert_eq!(base.movement, MoveStats::STATIC);
        assert!(db.key("missing").is_none());
        assert_eq!(legacy.db(&vfs, ["base", "ship"]).hash(), db.hash());
    }
}
