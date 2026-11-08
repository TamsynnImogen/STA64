//! Summarise a BZN: `map_info <install> <mod> <map>`
use std::collections::BTreeMap;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), Some(&a[1])).unwrap();
    let data = vfs.read(&format!("bzn/{}.bzn", a[2])).expect("map").unwrap();
    let m = sta_bzn::parse(&data).unwrap();
    println!("v{} binary={} bg={} stars={} race={} tt={} scripts={:?} size={:?} min={:?} ambient={:?}",
        m.version, m.binary, m.background, m.starfield_odf, m.race, m.tech_tree_file, m.scripts, m.size, m.min_extent, m.ambient);
    let mut odf_index: BTreeMap<String, String> = BTreeMap::new();
    for dir in ["odf", "addon"] {
        for p in vfs.list(dir) {
            if p.ends_with(".odf") {
                let stem = p.rsplit('/').next().unwrap().trim_end_matches(".odf").to_owned();
                odf_index.entry(stem).or_insert(p);
            }
        }
    }
    let mut by: BTreeMap<String, (usize, String, String)> = BTreeMap::new();
    for o in &m.objects {
        let k = o.odf.to_ascii_lowercase();
        let e = by.entry(k.clone()).or_insert((0, String::new(), String::new()));
        e.0 += 1;
        if e.0 == 1 {
            let mut load = |n: &str| odf_index.get(n.to_ascii_lowercase().trim_end_matches(".odf")).and_then(|p| vfs.read(p)).and_then(Result::ok).map(|b| String::from_utf8_lossy(&b).into_owned());
            match sta_odf::resolve(&k, &mut load) {
                Ok(odf) => {
                    e.1 = odf.class_label().unwrap_or("?").to_owned();
                    let keys: Vec<String> = odf.keys().filter(|k| { let l = k.to_ascii_lowercase(); l.contains("sod") || l.contains("model") || l.contains("scale") || l.contains("texture") || l.contains("radius") }).map(|k| format!("{k}={}", odf.raw(k).unwrap_or(""))).collect();
                    e.2 = keys.join(" ");
                }
                Err(err) => e.1 = format!("ERR {err}"),
            }
        }
    }
    let mut v: Vec<_> = by.into_iter().collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1 .0));
    for (k, (n, class, keys)) in v.iter() {
        let sod = sta_assets::find_model(&vfs, k).map(|x| x.0).unwrap_or_default();
        println!("{n:5} {k:28} {class:18} sod={sod:28} {}", &keys[..keys.len().min(110)]);
    }
}
