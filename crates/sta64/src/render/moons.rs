//! Resource moons drawn with their game models (RND-16).
//!
//! A map places a moon as a `trigger_<resource>` object (`trigger_dilithium`,
//! `trigger_tritanium`, ...), a craft-class marker whose own SOD holds only the
//! editor's letter. In the game that marker carries one weapon of class
//! `replaceweapon` (`triggerW_dilithium`) that, when the map starts, swaps the
//! marker for another object: `replacement0class<N>` lists the candidate ODFs
//! (`map_dilithium_moon1` .. `4`, each a `scrap`-class node with a `resource`
//! and the moon's SOD as its model), one of which is picked; `replacement1class0`
//! is a second object put beside it (the `hook_*` mining hook, a craft with no
//! model of its own to draw).
//!
//! The renderer does the same swap: [`moon_for`] reads the replace weapon and
//! returns the moon definition to draw in the marker's place, picking among the
//! variants by the object's id. The sim makes the same pick when it places the
//! map (`sta_content::map::moon_variants`): the node it mines from is the moon
//! itself, a `scrap`-class resource, not the marker. Nothing here is named
//! after a resource, so mods with other triggers or more variants work
//! unchanged.

use std::collections::HashMap;

use bevy::prelude::*;
use sta_content::{ContentDb, ContentKey, OdfIndex, UnitDef};
use sta_vfs::Vfs;

/// The moon variants each marker definition can turn into, read once per
/// definition from the weapon ODFs.
#[derive(Resource, Default)]
pub struct MoonVariants {
    index: Option<OdfIndex>,
    cache: HashMap<ContentKey, Vec<ContentKey>>,
}

impl MoonVariants {
    /// The moon definition to draw for `def` (placed as entity `seed`), or
    /// `None` when `def` is not a moon marker.
    pub fn moon_for<'a>(
        &mut self,
        vfs: &Vfs,
        db: &'a ContentDb,
        def: &UnitDef,
        seed: u64,
    ) -> Option<&'a UnitDef> {
        let variants = self.cache.entry(def.key).or_insert_with(|| {
            let index = self.index.get_or_insert_with(|| OdfIndex::build(vfs).0);
            variants_of(db, def, &mut |n, v| index.load(vfs, n, v))
        });
        if variants.is_empty() {
            return None;
        }
        db.unit(variants[(seed % variants.len() as u64) as usize])
    }
}

/// The candidate moon definitions `def` is replaced by: for each of its
/// weapons that is a `replaceweapon`, the `replacement0class<N>` ODFs (N from
/// 0 up, until one is missing) that the database defines as resource nodes.
/// Empty for anything that is not a moon marker.
pub fn variants_of(
    db: &ContentDb,
    def: &UnitDef,
    load: &mut dyn FnMut(&str, usize) -> Option<String>,
) -> Vec<ContentKey> {
    // The sim places the same moon (`sta_content::map::moon_variants`), picked by the object's id.
    sta_content::map::moon_variants(db, def, &mut |n| {
        sta_odf::resolve_layered(n, &mut *load).ok().map(std::sync::Arc::new)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (ContentDb, ContentKey) {
        let mut marker = UnitDef::new("trigger_dilithium", "craft");
        marker.raw = std::sync::Arc::new(
            sta_odf::resolve("trigger_dilithium", &mut |_| {
                Some("weapon1 = \"triggerW_dilithium\"\nclassLabel = \"craft\"\n".to_owned())
            })
            .unwrap(),
        );
        let mut moon1 = UnitDef::new("map_dilithium_moon1", "scrap");
        moon1.resource = Some(0);
        let mut moon2 = UnitDef::new("map_dilithium_moon2", "scrap");
        moon2.resource = Some(0);
        let hook = UnitDef::new("hook_dilithiumsource", "craft");
        let db = ContentDb::builder()
            .unit(marker)
            .unit(moon1)
            .unit(moon2)
            .unit(hook)
            .build()
            .unwrap();
        let k = db.key("trigger_dilithium").unwrap();
        (db, k)
    }

    fn weapon(name: &str, _v: usize) -> Option<String> {
        name.eq_ignore_ascii_case("triggerW_dilithium.odf").then(|| {
            "replacement0class0 = \"map_dilithium_moon1\"\nreplacement0class1 = \"map_dilithium_moon2\"\nreplacement1class0 = \"hook_dilithiumsource\"\nclassLabel = \"replaceweapon\"\n".to_owned()
        })
    }

    #[test]
    fn a_marker_turns_into_its_moon_variants_not_the_hook() {
        let (db, k) = db();
        let def = db.unit(k).unwrap();
        let names: Vec<_> = variants_of(&db, def, &mut weapon)
            .into_iter()
            .map(|v| db.unit(v).unwrap().odf.clone())
            .collect();
        assert_eq!(names, ["map_dilithium_moon1", "map_dilithium_moon2"]);
    }

    #[test]
    fn other_units_are_not_markers() {
        let (db, _) = db();
        let moon = db.unit(db.key("map_dilithium_moon1").unwrap()).unwrap();
        assert!(variants_of(&db, moon, &mut weapon).is_empty());
    }

    /// Install-gated (skipped when an install is absent): on every map of FO4,
    /// FO, DW and A2, each `trigger_*` object that carries a weapon (a moon
    /// marker) has moon variants, each with a model file that is not an editor
    /// marker's, and every resource object placed as a moon itself (A2) has its model. Prints `marker -> moon ODF -> model` per install.
    #[test]
    fn every_moon_on_every_install_map_has_a_model() {
        use std::collections::BTreeMap;
        const HEROIC: &str = "/home/cedric/Games/Heroic";
        let installs = [
            (
                "FO4",
                "Star Trek Armada II Fleet Operations Roots",
                Some("Fleet Ops 4.0"),
            ),
            ("FO", "Star Trek Armada II Fleet Operations", None),
            ("DW", "Star Trek Armada II Dominion War", None),
            ("A2", "Star Trek Armada II", None),
        ];
        let mut checked = 0;
        let mut bad = Vec::new();
        for (label, dir, active) in installs {
            let root = std::path::Path::new(HEROIC).join(dir);
            // GOG's plain A2 folder has no `Data`: its loose files are the layer.
            let mounted = Vfs::mount_install(&root, active).ok().or_else(|| {
                let mut v = Vfs::default();
                (root.is_dir() && v.push_dir(label.into(), root.clone()).is_ok()).then_some(v)
            });
            let Some(vfs) = mounted else {
                eprintln!("{label}: skipped, not installed or does not mount");
                continue;
            };
            let (index, _) = OdfIndex::build(&vfs);
            let Ok((db, _)) = ContentDb::load(&vfs, &index) else {
                continue;
            };
            let mut seen: BTreeMap<String, usize> = BTreeMap::new();
            let mut direct: BTreeMap<String, usize> = BTreeMap::new();
            for path in vfs
                .list("bzn")
                .into_iter()
                .filter(|p| p.to_ascii_lowercase().ends_with(".bzn"))
            {
                let Some(Ok(data)) = vfs.read(&path) else {
                    continue;
                };
                let Ok(bzn) = sta_bzn::parse(&data) else {
                    continue;
                };
                for o in &bzn.objects {
                    let Some(def) = db.key(&o.odf).and_then(|k| db.unit(k)) else {
                        continue;
                    };
                    // A2 places its moons as the moon ODFs themselves: their own model is the moon.
                    if def.resource.is_some() && !o.odf.to_ascii_lowercase().starts_with("trigger_")
                    {
                        let found = sta_assets::find_model(&vfs, &def.model).map(|(p, _)| p);
                        if found.is_none() || def.model.to_ascii_lowercase().starts_with("trigger_")
                        {
                            bad.push(format!(
                                "{label} {path}: {} has no moon model ({})",
                                o.odf, def.model
                            ));
                        }
                        let n = direct
                            .entry(format!(
                                "{} -> {} ({})",
                                def.odf,
                                def.model,
                                found.unwrap_or_else(|| "MISSING".into())
                            ))
                            .or_default();
                        *n += 1;
                        checked += 1;
                        continue;
                    }
                    if !o.odf.to_ascii_lowercase().starts_with("trigger_")
                        || def.raw.raw("weapon1").is_none()
                    {
                        continue;
                    }
                    *seen.entry(o.odf.to_ascii_lowercase()).or_default() += 1;
                    checked += 1;
                    let moons = variants_of(&db, def, &mut |n, v| index.load(&vfs, n, v));
                    if moons.is_empty() {
                        bad.push(format!("{label} {path}: {} has no moon variants", o.odf));
                    }
                    for m in moons {
                        let m = db.unit(m).unwrap();
                        if sta_assets::find_model(&vfs, &m.model).is_none()
                            || m.model.to_ascii_lowercase().starts_with("trigger_")
                        {
                            bad.push(format!(
                                "{label} {path}: {} -> {} has no moon model ({})",
                                o.odf, m.odf, m.model
                            ));
                        }
                    }
                }
            }
            for (what, n) in &direct {
                eprintln!("{label}: {what} (x{n} on maps, placed as the moon itself)");
            }
            for (marker, n) in &seen {
                let def = db.unit(db.key(marker).unwrap()).unwrap();
                for m in variants_of(&db, def, &mut |n, v| index.load(&vfs, n, v)) {
                    let m = db.unit(m).unwrap();
                    let found = sta_assets::find_model(&vfs, &m.model)
                        .map(|(p, _)| p)
                        .unwrap_or_else(|| "MISSING".into());
                    eprintln!(
                        "{label}: {marker} (x{n} on maps) -> {} -> {} ({found})",
                        m.odf, m.model
                    );
                }
            }
        }
        if checked == 0 {
            eprintln!("no install with moon markers: nothing checked");
        }
        assert!(
            bad.is_empty(),
            "moons without a model:\n  {}",
            bad.join("\n  ")
        );
    }

    /// Install-gated: the FO4 `boobytrap` map's moon markers, resolved through
    /// [`MoonVariants::moon_for`] (the renderer's own path), each get a moon.
    #[test]
    fn boobytrap_markers_resolve_through_the_renderer_path() {
        let root = std::path::Path::new("/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots");
        let Ok(vfs) = Vfs::mount_install(root, Some("Fleet Ops 4.0")) else {
            eprintln!("skipped: FO4 not installed");
            return;
        };
        let (index, _) = OdfIndex::build(&vfs);
        let Ok((db, _)) = ContentDb::load(&vfs, &index) else { return };
        let data = vfs.read("bzn/boobytrap.bzn").expect("boobytrap").unwrap();
        let bzn = sta_bzn::parse(&data).unwrap();
        let mut moons = MoonVariants::default();
        let mut markers = std::collections::BTreeSet::new();
        for o in &bzn.objects {
            if !o.odf.to_ascii_lowercase().starts_with("trigger_") {
                continue;
            }
            let def = db.key(&o.odf).and_then(|k| db.unit(k)).unwrap_or_else(|| panic!("{} undefined", o.odf));
            for seed in 0..4 {
                let moon = moons.moon_for(&vfs, &db, def, seed);
                eprintln!("{} seed {seed} -> {:?}", o.odf, moon.map(|m| (&m.odf, &m.model, m.resource)));
                // Other triggers (`trigger_blocking`) swap for nothing: not moons.
                let Some(moon) = moon else { continue };
                markers.insert(o.odf.to_ascii_lowercase());
                // A resource: the renderers give such a root a click radius (not shown as craft).
                assert!(moon.resource.is_some(), "{} is not a resource node", moon.odf);
                assert!(sta_assets::find_model(&vfs, &moon.model).is_some(), "{}: model {}", moon.odf, moon.model);
            }
        }
        // Both of FO4's moon markers on this map (dilithium and tritanium) get their moons.
        assert!(markers.contains("trigger_dilithium") && markers.contains("trigger_tritanium"), "{markers:?}");
    }
}
