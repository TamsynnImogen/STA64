//! Install-gated: with STA64_TEST_INSTALL set, resolve every
//! ODF of the mounted install through `resolve_layered` (each layer's copy of a
//! file is one version) and require that none ends in `IncludeCycle`.
//! STA64_TEST_MOD optionally selects a mod; absent/empty mounts the base or flat A1 root.

use std::collections::BTreeMap;
use std::path::Path;

use sta_odf::{OdfError, resolve_layered};
use sta_vfs::{Source, Vfs};

fn read(vfs: &Vfs, layer: &str, src: &Source) -> Option<Vec<u8>> {
    vfs.read_from(vfs.layer_index(layer)?, src).ok()
}

#[test]
fn no_include_cycles_in_install() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        return;
    };
    let m = std::env::var("STA64_TEST_MOD").ok();
    let vfs = Vfs::mount_install(Path::new(&root), m.as_deref().filter(|m| !m.is_empty())).unwrap();
    // File name -> virtual path; includes name files anywhere under odf/ (and A1 addon/).
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();
    for p in vfs.list("odf").into_iter().chain(vfs.list("addon")) {
        if p.ends_with(".odf") {
            let name = p.rsplit('/').next().unwrap().to_owned();
            by_name.entry(name).or_insert(p);
        }
    }
    let followed = std::cell::Cell::new(0usize);
    let mut load = |name: &str, version: usize| {
        let path = by_name.get(&name.to_ascii_lowercase())?;
        let (layer, src) = vfs.resolve_all(path).into_iter().nth(version)?;
        let text = read(&vfs, layer, src).map(|b| String::from_utf8_lossy(&b).into_owned());
        if version > 0 && text.is_some() {
            followed.set(followed.get() + 1);
        }
        text
    };
    let (mut ok, mut layered, mut cut, mut missing) = (0, 0, 0, 0);
    let mut cycles = Vec::new();
    let mut missing_names = Vec::new();
    let names: Vec<String> = by_name.keys().cloned().collect();
    for name in &names {
        let versions = vfs.resolve_all(&by_name[name]).len();
        match resolve_layered(name, &mut load) {
            Ok(odf) => {
                ok += 1;
                cut += odf.cut.len();
                if versions > 1 {
                    layered += 1;
                }
            }
            Err(OdfError::IncludeNotFound(inc)) => {
                missing += 1;
                missing_names.push(format!("{name} -> {inc}"));
            }
            Err(e @ OdfError::IncludeCycle(_)) => cycles.push(format!("{name}: {e}")),
        }
    }
    eprintln!(
        "{} odfs: {ok} resolved ({layered} with lower versions), {} lower-version reads for self-includes, {cut} self-includes cut, {missing} with a missing include, {} cycles",
        names.len(),
        followed.get(),
        cycles.len()
    );
    eprintln!(
        "missing includes (first 5): {:?}",
        &missing_names[..missing_names.len().min(5)]
    );
    assert!(!names.is_empty(), "install contains no ODFs");
    assert!(cycles.is_empty(), "{cycles:#?}");
}
