//! Count mesh-group blend modes across every model: `blend_survey <install> [mod]`
use std::collections::BTreeMap;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), a.get(1).map(String::as_str)).unwrap();
    let mut counts: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for name in sta_assets::list_models(&vfs) {
        let Some((path, fmt)) = sta_assets::find_model(&vfs, &name) else { continue };
        let Ok(m) = sta_assets::load_model(&vfs, &path, fmt) else { continue };
        for n in &m.nodes {
            let Some(mesh) = n.mesh() else { continue };
            for g in &mesh.groups {
                let e = counts.entry(g.blend.to_ascii_lowercase()).or_insert((0, name.clone()));
                e.0 += 1;
            }
        }
    }
    for (b, (n, ex)) in counts { println!("{n:7} {b:20} e.g. {ex}"); }
}
