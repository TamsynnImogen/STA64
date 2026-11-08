//! Install-gated: with STA64_TEST_AIP_DIRS (folders separated by the
//! platform's path separator, e.g. each game's install root), load every
//! `.aip` under them. Includes resolve by file name, preferring the including
//! file's own folder. Every file must load; warnings are tallied.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sta_content::Aip;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            walk(&e.path(), out);
        } else if ft.is_file() {
            out.push(e.path());
        }
    }
}

fn read(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[test]
fn every_installed_aip_loads() {
    let Some(dirs) = std::env::var_os("STA64_TEST_AIP_DIRS") else { return };
    for dir in std::env::split_paths(&dirs) {
        let mut files = Vec::new();
        walk(&dir, &mut files);
        files.sort();
        let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for p in &files {
            let n = p.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
            by_name.entry(n).or_default().push(p.clone());
        }
        let aips: Vec<&PathBuf> =
            files.iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("aip"))).collect();
        let (mut vars, mut lists, mut items) = (0, 0, 0);
        let mut kinds: BTreeMap<String, (usize, String)> = BTreeMap::new();
        let mut empty = Vec::new();
        for path in &aips {
            let here = path.parent().unwrap().to_path_buf();
            let root = path.to_string_lossy().into_owned();
            let mut load = |n: &str| -> Option<String> {
                if n == root {
                    return read(path);
                }
                let cands = by_name.get(&n.rsplit(['/', '\\']).next()?.to_ascii_lowercase())?;
                let pick = cands.iter().find(|c| c.parent() == Some(here.as_path())).or(cands.first())?;
                read(pick)
            };
            let (aip, warnings) = Aip::load(&root, &mut load);
            assert!(!warnings.iter().any(|w| w.file == root && w.message == "not found"), "{root}");
            vars += aip.vars().count();
            lists += aip.lists().count();
            items += aip.lists().filter_map(|l| aip.build_list(&l.name)).map(|b| b.len()).sum::<usize>();
            if aip.vars().count() == 0 && aip.lists().count() == 0 {
                empty.push(root.clone());
            }
            for w in warnings {
                let kind = match w.message.split_once(": ") {
                    Some((head, tail)) if head.starts_with("include not found") || head.starts_with("include loop") => {
                        format!("{head}: {tail}")
                    }
                    Some((_, tail)) => tail.to_owned(),
                    None => w.message.clone(),
                };
                let e = kinds.entry(kind).or_insert((0, w.file.clone()));
                e.0 += 1;
            }
        }
        eprintln!(
            "{}: {} aip files, {vars} params, {lists} lists, {items} build items; {} with nothing in them",
            dir.display(),
            aips.len(),
            empty.len()
        );
        for (k, (n, example)) in &kinds {
            let example = example.strip_prefix(&*dir.to_string_lossy()).unwrap_or(example);
            eprintln!("  {n:>5}  {k}  (e.g. {example})");
        }
        for e in empty.iter().take(5) {
            eprintln!("  empty: {e}");
        }
        assert!(!aips.is_empty(), "no .aip under {}", dir.display());
    }
}
