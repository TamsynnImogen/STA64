//! Bake strategic silhouettes to PGM: `silhouette <install> <mod> <out_dir> <model>...`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), Some(&a[1])).unwrap();
    for name in &a[3..] {
        let Some((path, fmt)) = sta_assets::find_model(&vfs, name) else { println!("{name}: no model"); continue };
        let m = sta_assets::load_model(&vfs, &path, fmt).unwrap();
        let Some(s) = sta_assets::silhouette::bake(&m, 64, 4) else { println!("{name}: nothing solid"); continue };
        let mut out = format!("P5\n{} {}\n255\n", s.size, s.size).into_bytes();
        out.extend_from_slice(&s.alpha);
        std::fs::write(format!("{}/{name}.pgm", a[2]), out).unwrap();
        println!("{name}: ok");
    }
}
