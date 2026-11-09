//! Install-gated: load every `.tt` and `.xml` tech tree through the VFS.
//! Includes resolve beside the including file, then under `techtree/`, then
//! by bare name. Every file must load without a malformed-file, not-found or
//! include-loop warning, and have entries; other warnings are tallied.
//!
//! - STA64_TEST_INSTALL (+ STA64_TEST_MOD): a Fleet Operations / Roots install.
//! - STA64_TEST_TT_DIRS: plain game folders (A1 / A2 retail), each mounted as
//!   one layer; separated by the platform's path separator. A1's trees live
//!   in `assets.zfs`, which the VFS mounts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sta_content::TechTree;
use sta_vfs::Vfs;

fn check(label: &str, vfs: &Vfs) -> usize {
    let files: Vec<String> = vfs
        .list("")
        .into_iter()
        .filter(|p| {
            let in_tree = p.starts_with("techtree/") || !p.contains('/');
            in_tree && (p.ends_with(".tt") || p.ends_with(".xml"))
        })
        .filter(|p| p.starts_with("techtree/") || p.ends_with(".tt"))
        .collect();
    let (mut tt, mut xml, mut entries, mut caps, mut max_prereqs) = (0, 0, 0, 0, 0);
    let mut kinds: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for path in &files {
        let (dir, name) = match path.rsplit_once('/') {
            Some((d, n)) => (format!("{d}/"), n.to_owned()),
            None => (String::new(), path.clone()),
        };
        let mut load = |n: &str| {
            let base = n.rsplit(['/', '\\']).next()?;
            let bytes = [format!("{dir}{base}"), format!("techtree/{base}"), base.to_owned()]
                .iter()
                .find_map(|p| vfs.read(p))?
                .ok()?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        };
        let (tree, warnings) = TechTree::load(&name, &mut load);
        for w in &warnings {
            assert!(
                !(w.message.starts_with("malformed xml") || w.message == "not found" || w.message.starts_with("include loop")),
                "{path}: {w}"
            );
            // Group by the message's shape: the text after "<odf>: ".
            let kind = match w.message.split_once(": ") {
                Some((head, tail)) if head.starts_with("include") || head.starts_with("capacity") => format!("{head}: {tail}"),
                Some((_, tail)) => tail.split(|c: char| c.is_ascii_digit()).next().unwrap_or("").trim().to_owned(),
                None => w.message.clone(),
            };
            kinds.entry(kind).or_insert((0, format!("{path}: {w}"))).0 += 1;
        }
        assert!(!tree.is_empty(), "{path} has no entries");
        if name.ends_with(".xml") {
            xml += 1;
        } else {
            tt += 1;
        }
        entries += tree.len();
        caps += tree.capacity_lists().count();
        max_prereqs = max_prereqs.max(tree.entries().map(|e| e.prerequisites().len()).max().unwrap_or(0));
    }
    eprintln!(
        "{label}: {tt} .tt and {xml} .xml files, {entries} entries in all, {caps} capacity lists, up to {max_prereqs} prerequisites on one entry"
    );
    for (k, (n, example)) in &kinds {
        eprintln!("  {n:>5}  {k}  (e.g. {example})");
    }
    files.len()
}

#[test]
fn every_installed_tech_tree_loads() {
    if let Ok(root) = std::env::var("STA64_TEST_INSTALL") {
        let m = std::env::var("STA64_TEST_MOD").ok();
        let vfs = Vfs::mount_install(Path::new(&root), m.as_deref()).unwrap();
        assert!(check(m.as_deref().unwrap_or("Data"), &vfs) > 0, "no tech trees found");
    }
    if let Some(dirs) = std::env::var_os("STA64_TEST_TT_DIRS") {
        for dir in std::env::split_paths(&dirs) {
            let mut vfs = Vfs::default();
            vfs.push_dir("root".into(), PathBuf::from(&dir)).unwrap();
            assert!(check(&dir.display().to_string(), &vfs) > 0, "no tech trees under {}", dir.display());
        }
    }
}
