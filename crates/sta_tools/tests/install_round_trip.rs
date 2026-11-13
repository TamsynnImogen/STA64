//! Install-gated (EDT-06): every ODF of every install named in
//! `STA64_TEST_INSTALLS` (install roots separated like `PATH`; falls back to
//! `STA64_TEST_INSTALL`) must survive `odf_fmt`: the tree rebuilds the file,
//! whitespace formatting and the grouped layout are idempotent, keep the
//! includes, keys and values, and `resolve_layered` returns the same values
//! from formatted copies as from the originals. Each install is mounted bare
//! and once per mod under `Data/Mods`; every layer copy is counted once.
//!
//! Failures print the path, the layer and the key, never file content.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sta_odf::{resolve_layered, Cst, GroupTable};
use sta_tools::odf_fmt::{check_bytes, Layout};
use sta_vfs::{Source, Vfs};

fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
    })
}

fn mods_of(root: &Path) -> Vec<String> {
    let Some(mods) = child_ci(root, "Data").and_then(|d| child_ci(&d, "Mods")) else {
        return Vec::new();
    };
    let mut v: Vec<String> = std::fs::read_dir(mods)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn read(vfs: &Vfs, layer: &str, src: &Source) -> Option<Vec<u8>> {
    vfs.read_from(vfs.layer_index(layer)?, src).ok()
}

#[derive(Default)]
struct Counts {
    files: usize,
    crlf: usize,
    lf_only: usize,
    mixed: usize,
    non_utf8: usize,
    plain_changed: usize,
    grouped_changed: usize,
    grouped_kept_order: usize,
    resolved: usize,
    unresolved: usize,
}

#[test]
fn every_installed_odf_round_trips_through_odf_fmt() {
    let roots: Vec<PathBuf> = match std::env::var_os("STA64_TEST_INSTALLS") {
        Some(v) => std::env::split_paths(&v).collect(),
        None => std::env::var_os("STA64_TEST_INSTALL").map(PathBuf::from).into_iter().collect(),
    };
    if roots.is_empty() {
        return;
    }
    let table = GroupTable::fo();
    let mut failures: Vec<String> = Vec::new();
    for root in &roots {
        let label = root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let mut counts = Counts::default();
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        let mut mounts: Vec<Option<String>> = vec![None];
        mounts.extend(mods_of(root).into_iter().map(Some));
        let mut mounted = 0;
        for m in &mounts {
            let Ok(vfs) = Vfs::mount_install(root, m.as_deref()) else {
                eprintln!("EDT-06: {label}: cannot mount {m:?}, skipped");
                continue;
            };
            mounted += 1;
            let mut by_name: BTreeMap<String, String> = BTreeMap::new();
            let mut paths = Vec::new();
            for dir in ["odf", "addon", ""] {
                for p in vfs.list(dir) {
                    // `list("")` lists everything; keep root-level ODFs only.
                    if p.ends_with(".odf") && (!dir.is_empty() || !p.contains('/')) {
                        by_name.entry(p.rsplit('/').next().unwrap().to_ascii_lowercase()).or_insert_with(|| p.clone());
                        paths.push(p);
                    }
                }
            }
            for p in &paths {
                for (layer, src) in vfs.resolve_all(p) {
                    if !seen.insert((layer.to_owned(), p.clone())) {
                        continue;
                    }
                    let Some(bytes) = read(&vfs, layer, src) else { continue };
                    counts.files += 1;
                    let (crlf, lf) = {
                        let mut c = (false, false);
                        for (i, &b) in bytes.iter().enumerate() {
                            if b == b'\n' {
                                if i > 0 && bytes[i - 1] == b'\r' { c.0 = true } else { c.1 = true }
                            }
                        }
                        c
                    };
                    counts.crlf += (crlf && !lf) as usize;
                    counts.lf_only += (lf && !crlf) as usize;
                    counts.mixed += (lf && crlf) as usize;
                    counts.non_utf8 += std::str::from_utf8(&bytes).is_err() as usize;
                    let cst = Cst::parse(&bytes);
                    let plain = cst.format();
                    let grouped = cst.format_grouped(table);
                    counts.plain_changed += (plain != bytes) as usize;
                    counts.grouped_changed += (grouped != bytes) as usize;
                    counts.grouped_kept_order += (grouped == plain) as usize;
                    for (mode, layout) in [("plain", Layout::Plain), ("grouped", Layout::Grouped(table))] {
                        for problem in check_bytes(&bytes, layout) {
                            failures.push(format!("{label}: {p} ({layer}): {mode}: {problem}"));
                        }
                    }
                }
            }
            // Resolution over originals and over grouped copies.
            let loader = |formatted: bool| {
                let (vfs, by_name) = (&vfs, &by_name);
                move |name: &str, version: usize| {
                    let path = by_name.get(&name.to_ascii_lowercase())?;
                    let (layer, src) = vfs.resolve_all(path).into_iter().nth(version)?;
                    let bytes = read(vfs, layer, src)?;
                    let bytes = if formatted { Cst::parse(&bytes).format_grouped(table) } else { bytes };
                    Some(String::from_utf8_lossy(&bytes).into_owned())
                }
            };
            for name in by_name.keys() {
                match (resolve_layered(name, &mut loader(false)), resolve_layered(name, &mut loader(true))) {
                    (Ok(a), Ok(b)) => {
                        counts.resolved += 1;
                        let (mut ka, mut kb): (Vec<_>, Vec<_>) = (a.keys().collect(), b.keys().collect());
                        ka.sort();
                        kb.sort();
                        if ka != kb {
                            failures.push(format!("{label}: {name}: resolved key set differs after grouping"));
                        } else if let Some(k) = ka.iter().find(|k| a.raw(k) != b.raw(k)) {
                            failures.push(format!("{label}: {name}: resolved value differs after grouping at key {k}"));
                        }
                    }
                    (a, b) => {
                        counts.unresolved += 1;
                        if a.err() != b.err() {
                            failures.push(format!("{label}: {name}: resolution error differs after grouping"));
                        }
                    }
                }
            }
        }
        eprintln!(
            "EDT-06 {label}: {mounted} mounts, {} ODF copies ({} CRLF, {} LF, {} mixed, {} not UTF-8); \
             whitespace format changes {}, grouped layout changes {} ({} unchanged by grouping); \
             {} names resolved identically ({} unresolvable in both)",
            counts.files, counts.crlf, counts.lf_only, counts.mixed, counts.non_utf8,
            counts.plain_changed, counts.grouped_changed, counts.grouped_kept_order,
            counts.resolved, counts.unresolved
        );
        if counts.files == 0 {
            failures.push(format!("{label}: no ODFs found"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures, first 20: {:#?}",
        failures.len(),
        &failures[..failures.len().min(20)]
    );
}
