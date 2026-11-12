//! Install-gated (EDT-06): with STA64_TEST_INSTALL set (STA64_TEST_MOD
//! optionally), every ODF copy of every layer of the mounted install must
//! round-trip through the lossless syntax tree byte for byte, formatting must
//! be idempotent and keep what the reader sees, and every ODF must resolve
//! (`resolve_layered`) to the same values from formatted files as from the
//! originals.

use std::collections::BTreeMap;
use std::path::Path;

use sta_odf::{parse, resolve_layered, Cst};
use sta_vfs::{Source, Vfs};

fn read(vfs: &Vfs, layer: &str, src: &Source) -> Option<Vec<u8>> {
    vfs.read_from(vfs.layer_index(layer)?, src).ok()
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn every_installed_odf_round_trips_and_formats_to_the_same_values() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        return;
    };
    let m = std::env::var("STA64_TEST_MOD").ok();
    let vfs = Vfs::mount_install(Path::new(&root), m.as_deref().filter(|m| !m.is_empty())).unwrap();
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();
    let mut paths = Vec::new();
    for p in vfs.list("odf").into_iter().chain(vfs.list("addon")) {
        if p.ends_with(".odf") {
            let name = p.rsplit('/').next().unwrap().to_owned();
            by_name.entry(name).or_insert_with(|| p.clone());
            paths.push(p);
        }
    }
    assert!(!paths.is_empty(), "install contains no ODFs");

    let (mut copies, mut changed, mut crlf, mut non_utf8, mut failures) =
        (0usize, 0usize, 0usize, 0usize, Vec::new());
    for p in &paths {
        for (layer, src) in vfs.resolve_all(p) {
            let Some(bytes) = read(&vfs, layer, src) else {
                continue;
            };
            copies += 1;
            let cst = Cst::parse(&bytes);
            if cst.to_bytes() != bytes {
                failures.push(format!("{p} ({layer}): not byte-identical"));
                continue;
            }
            let rebuilt: Vec<u8> = cst
                .lines()
                .iter()
                .flat_map(|l| [cst.text(l), cst.eol(l)].concat())
                .collect();
            if rebuilt != bytes {
                failures.push(format!("{p} ({layer}): lines do not rebuild the file"));
            }
            crlf += bytes.windows(2).any(|w| w == b"\r\n") as usize;
            non_utf8 += std::str::from_utf8(&bytes).is_err() as usize;
            let out = cst.format();
            changed += (out != bytes) as usize;
            if Cst::parse(&out).format() != out {
                failures.push(format!("{p} ({layer}): format is not idempotent"));
            }
            let (a, b) = (parse(&lossy(&bytes)), parse(&lossy(&out)));
            let kv = |r: &sta_odf::RawOdf| {
                r.entries
                    .iter()
                    .map(|e| (e.key.clone(), e.value.clone()))
                    .collect::<Vec<_>>()
            };
            if a.includes != b.includes || kv(&a) != kv(&b) {
                failures.push(format!("{p} ({layer}): formatted file reads differently"));
            }
            if cst.includes() != a.includes {
                failures.push(format!("{p} ({layer}): includes differ from the reader's"));
            }
        }
    }

    // Resolution: the same names, once over originals and once over
    // formatted copies of the same layered files.
    let loader = |formatted: bool| {
        let vfs = &vfs;
        let by_name = &by_name;
        move |name: &str, version: usize| {
            let path = by_name.get(&name.to_ascii_lowercase())?;
            let (layer, src) = vfs.resolve_all(path).into_iter().nth(version)?;
            let bytes = read(vfs, layer, src)?;
            Some(lossy(&if formatted {
                Cst::parse(&bytes).format()
            } else {
                bytes
            }))
        }
    };
    let (mut resolved, mut unresolved) = (0usize, 0usize);
    for name in by_name.keys() {
        let (a, b) = (
            resolve_layered(name, &mut loader(false)),
            resolve_layered(name, &mut loader(true)),
        );
        match (a, b) {
            (Ok(a), Ok(b)) => {
                resolved += 1;
                let same = a.keys().eq(b.keys())
                    && a.keys().all(|k| a.raw(k) == b.raw(k))
                    && a.cut == b.cut;
                if !same {
                    failures.push(format!("{name}: resolves differently after formatting"));
                }
            }
            (a, b) => {
                unresolved += 1;
                if a.err() != b.err() {
                    failures.push(format!("{name}: resolution error differs after formatting"));
                }
            }
        }
    }
    assert!(
        resolved > 0,
        "nothing resolved: the comparison proves nothing"
    );
    eprintln!(
        "EDT-06: {} ODF paths, {copies} layer copies round-tripped ({crlf} with CRLF, {non_utf8} not UTF-8), \
         {changed} changed by the formatter; {resolved} names resolved identically ({unresolved} unresolvable in both)",
        paths.len()
    );
    assert!(
        failures.is_empty(),
        "{} failures, first 10: {:#?}",
        failures.len(),
        &failures[..failures.len().min(10)]
    );
}
