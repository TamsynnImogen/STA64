//! Resolved ODF keys matching a filter: `odf_dump <install> <mod> <odf> [filter]`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), Some(&a[1])).unwrap();
    let mut idx = std::collections::HashMap::new();
    for dir in ["odf", "addon"] {
        for p in vfs.list(dir) {
            if let Some(stem) = p.strip_suffix(".odf") {
                idx.entry(stem.rsplit('/').next().unwrap().to_owned()).or_insert(p.clone());
            }
        }
    }
    let mut load = |n: &str| {
        let s = n.to_ascii_lowercase();
        idx.get(s.trim_end_matches(".odf")).and_then(|p| vfs.read(p)).and_then(Result::ok).map(|b| String::from_utf8_lossy(&b).into_owned())
    };
    let odf = sta_odf::resolve(&a[2], &mut load).unwrap();
    let f = a.get(3).map(|s| s.to_ascii_lowercase());
    for k in odf.keys() {
        if f.as_ref().is_none_or(|f| f.split('|').any(|p| k.to_ascii_lowercase().contains(p))) {
            println!("{k} = {}", odf.raw(k).unwrap_or(""));
        }
    }
}
