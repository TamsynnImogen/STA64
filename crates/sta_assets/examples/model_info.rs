//! Print a model's groups, texture maps and how bump textures classify.
//! `cargo run -p sta_assets --example model_info -- <install> <mod> <model>`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), Some(&a[1])).unwrap();
    let (path, fmt) = sta_assets::find_model(&vfs, &a[2]).expect("model");
    let m = sta_assets::load_model(&vfs, &path, fmt).unwrap();
    println!("{path} {fmt:?} v{}", m.version);
    for n in &m.nodes {
        if let Some(mesh) = n.mesh() {
            for g in &mesh.groups {
                println!("  {} [{}] blend={} faces={} maps={:?}", n.name, g.material, g.blend, g.faces.len(), g.maps);
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for n in &m.nodes {
        if let Some(mesh) = n.mesh() {
            for g in &mesh.groups {
                for t in [&g.maps.bump, &g.maps.height].into_iter().flatten() {
                    if !seen.insert(t.to_ascii_lowercase()) { continue; }
                    match sta_assets::find_texture(&vfs, t) {
                        Some((p, f)) => match sta_assets::decode_rgba(&vfs, &p, f) {
                            Ok(img) => {
                                let mut sum = [0u64; 3];
                                for px in img.pixels.chunks_exact(4) { for c in 0..3 { sum[c] += px[c] as u64; } }
                                let n = (img.width * img.height) as u64;
                                println!("  bump {t} -> {p} {}x{} mean rgb {:?} normal_map={}", img.width, img.height,
                                    sum.map(|s| s / n), sta_assets::pixels::looks_like_normal_map(&img));
                            }
                            Err(e) => println!("  bump {t} -> {p}: {e}"),
                        },
                        None => println!("  bump {t}: NOT FOUND"),
                    }
                }
            }
        }
    }
    let damage = ["shields", "engines", "target", "sensors", "life", "borg", "crew", "damage"];
    for (i, n) in m.nodes.iter().enumerate() {
        if damage.iter().any(|d| n.name.eq_ignore_ascii_case(d)) {
            println!("  damage-ish node {} (#{i}) children={}", n.name, n.children.len());
        }
    }
}
