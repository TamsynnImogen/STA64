//! The random map table (CON-50) of a mounted install: the highest layer's
//! `random_map.json5` is the whole table, and an install without one, or
//! with a broken one, has the stock table.

use std::fs;
use std::path::PathBuf;

use sta_content::random_map::{self, StockSizes, Table};
use sta_vfs::Vfs;

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("sta64-random-map-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Tmp(path)
    }
    fn file(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/random_map").join(name)).unwrap()
}

#[test]
fn the_top_layer_s_file_is_the_table() {
    let t = Tmp::new("layers");
    t.file("Data/odf/other/x.odf", "classLabel = \"craft\"\n");
    t.file("Data/Mods/M/info.ini", "[Mod]\n");
    t.file("Data/Mods/M/odf/other/y.odf", "classLabel = \"craft\"\n");

    // No file anywhere: the stock table, silently.
    let (table, warnings) = Table::load(&Vfs::mount_install(&t.0, Some("M")).unwrap());
    assert_eq!(table, Table::stock());
    assert!(warnings.is_empty());

    // The install's own file.
    t.file("Data/random_map.json5", &fixture("stock.json5").replace("count: [10, 30]", "count: [3, 4]"));
    let (base, warnings) = Table::load(&Vfs::mount_install(&t.0, None).unwrap());
    assert_eq!(base.count, (3, 4));
    assert!(warnings.is_empty());

    // A mod's file replaces it whole.
    t.file("Data/Mods/M/random_map.json5", &fixture("two_sets.json5"));
    let (modded, warnings) = Table::load(&Vfs::mount_install(&t.0, Some("M")).unwrap());
    assert!(warnings.is_empty());
    assert_eq!(modded, Table::parse("two_sets.json5", &fixture("two_sets.json5")).unwrap());
    assert_eq!(Table::load(&Vfs::mount_install(&t.0, None).unwrap()).0, base);
    let map = random_map::generate_with(11, 2, &modded, Some("frozen"), &mut StockSizes);
    assert!(map.objects.iter().any(|o| o.odf == "fx_bio_ice"));
    assert!(!map.objects.iter().any(|o| o.odf == "mdmoon"));

    // A broken file: the stock table and a warning that says why.
    t.file("Data/Mods/M/random_map.json5", "{ sets: ['a'], resources: [{ name: 'r', sets: { b: ['x'] } }] }");
    let (table, warnings) = Table::load(&Vfs::mount_install(&t.0, Some("M")).unwrap());
    assert_eq!(table, Table::stock());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].file == random_map::table::FILE && warnings[0].message.contains("not in `sets`"), "{warnings:?}");
}
