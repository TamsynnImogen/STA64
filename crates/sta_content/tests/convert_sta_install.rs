//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD): convert the active
//! mod's folder to one `.sta` (`sta-archive convert`'s code), mount it in the
//! folder's place above the same parents and Data, and require that the ODF
//! index picks the same ODFs with the same bytes as with the folder.

use std::path::Path;

use sta_content::OdfIndex;
use sta_vfs::{Vfs, mod_chain};

#[test]
fn converted_mod_serves_the_same_odfs() {
    let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else {
        return;
    };
    let data = Path::new(&root).join("Data");
    let original = Vfs::mount_install(Path::new(&root), Some(&m)).unwrap();

    let chain = mod_chain(&data, &m).unwrap();
    // Streamed to disk, as `sta-archive convert` does, so large mods fit in memory.
    let folder = sta_vfs::convert::ModFolder::open(&chain[0].1).unwrap();
    let tmp = std::env::temp_dir().join(format!("sta_convert_install_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let mut f = std::io::BufWriter::new(std::fs::File::create(tmp.join("mod.sta")).unwrap());
    sta_vfs::sta_write::pack_to(&mut f, folder.items(), sta_vfs::sta_write::Compression::Auto).unwrap();
    drop(f);
    let size = std::fs::metadata(tmp.join("mod.sta")).unwrap().len();
    let report = folder.report.clone();

    let mut converted = Vfs::default();
    converted.push_dir(chain[0].0.clone(), tmp.clone()).unwrap();
    for (name, dir) in &chain[1..] {
        converted.push_dir(name.clone(), dir.clone()).unwrap();
    }
    converted.push_dir("Data".into(), data).unwrap();

    let (a, _) = OdfIndex::build(&original);
    let (b, _) = OdfIndex::build(&converted);
    let stems_a: Vec<&str> = a.iter().map(|(s, _)| s).collect();
    let stems_b: Vec<&str> = b.iter().map(|(s, _)| s).collect();
    assert_eq!(stems_a, stems_b, "same ODF names");
    let mut differ = Vec::new();
    for (stem, ea) in a.iter() {
        let eb = b.get(stem).unwrap();
        if OdfIndex::read(&original, ea) != OdfIndex::read(&converted, eb) {
            differ.push(format!("{stem}: {} vs {}", ea.path, eb.path));
        }
    }
    eprintln!(
        "{m}: {} ODF names, {} different winners; archive {} bytes; {report:?}",
        stems_a.len(),
        differ.len(),
        size
    );
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(differ.is_empty(), "{:#?}", &differ[..differ.len().min(10)]);
}
