//! Read-only, opt-in check of the instance-name rule on a real install's maps: it asserts the
//! shape of what the reader returns and prints counts only (no names, no game bytes bundled).
//! `STA64_A2_INSTALL=<game root> cargo test -p sta_bzn --test installed_instance_names -- --ignored`

use std::path::{Path, PathBuf};

fn bzn_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            bzn_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bzn")) {
            out.push(p);
        }
    }
}

#[test]
#[ignore = "requires STA64_A2_INSTALL (an Armada II game root)"]
fn instance_names_of_installed_binary_maps_have_the_documented_shape() {
    let root = PathBuf::from(std::env::var("STA64_A2_INSTALL").expect("set STA64_A2_INSTALL to an installed game root"));
    let mut files = Vec::new();
    bzn_files(&root, &mut files);
    assert!(!files.is_empty(), "no .bzn under the install");
    let (mut maps, mut objects, mut named, mut blank) = (0usize, 0usize, 0usize, 0usize);
    for f in &files {
        let Ok(map) = sta_bzn::parse(&std::fs::read(f).unwrap()) else { continue };
        if !map.binary || map.a1.is_some() {
            continue;
        }
        maps += 1;
        for o in &map.objects {
            objects += 1;
            let n = &o.instance_name;
            if n.is_empty() {
                blank += 1;
                continue;
            }
            named += 1;
            assert_eq!(n.trim(), n, "{}: a name is trimmed", f.display());
            assert!(n.len() <= 3 * 127, "{}: at most the 127 visible bytes of the buffer", f.display());
            assert!(!n.contains('\0'), "{}: cut at the first NUL", f.display());
            assert!(!o.label.trim().is_empty(), "{}: a named object has a label to join by", f.display());
        }
    }
    println!("binary maps {maps}, objects {objects}, named {named}, without a name {blank}");
    assert!(maps > 0 && named > 0, "an Armada II install names some objects");
    assert!(blank > named, "most objects have no name buffer");
}
