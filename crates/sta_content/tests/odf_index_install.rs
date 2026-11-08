//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD): build the ODF index
//! and resolve every ODF through it with `sta_odf::resolve_layered`, each
//! file of a name being one version. Requires 0 include cycles and that
//! every name's winner is in the highest root that has it.

use std::path::Path;

use sta_content::OdfIndex;
use sta_odf::OdfError;
use sta_vfs::Vfs;

#[test]
fn index_and_resolve_every_odf() {
    let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else {
        return;
    };
    let vfs = Vfs::mount_install(Path::new(&root), Some(&m)).unwrap();
    let (index, warnings) = OdfIndex::build(&vfs);
    let (mut ok, mut missing, mut cycles) = (0, 0, Vec::new());
    for (stem, winner) in index.iter() {
        let all = index.versions(stem);
        assert_eq!(all[0], winner);
        assert!(all.windows(2).all(|w| w[0].layer <= w[1].layer), "{stem}: {all:?}");
        match sta_odf::resolve_layered(stem, &mut |n, v| index.load(&vfs, n, v)) {
            Ok(_) => ok += 1,
            Err(OdfError::IncludeNotFound(_)) => missing += 1,
            Err(e @ OdfError::IncludeCycle(_)) => cycles.push(format!("{stem}: {e}")),
        }
    }
    eprintln!(
        "{} names: {ok} resolved, {missing} with a missing include, {} cycles; {} same-root duplicates",
        index.len(),
        cycles.len(),
        warnings.len()
    );
    assert!(!index.is_empty());
    assert!(cycles.is_empty(), "{cycles:#?}");
}
