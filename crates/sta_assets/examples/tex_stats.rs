//! Texture alpha/colour stats: `tex_stats <install> <mod> <name>...`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), Some(&a[1])).unwrap();
    for n in &a[2..] {
        let Some((p, f)) = sta_assets::find_texture(&vfs, n) else { println!("{n}: not found"); continue };
        let t = sta_assets::decode_rgba(&vfs, &p, f).unwrap();
        let px = t.pixels.chunks(4);
        let (mut s, mut amin, mut amax, mut c) = ([0f64; 4], 255u8, 0u8, 0f64);
        for q in px { for i in 0..4 { s[i] += q[i] as f64; } amin = amin.min(q[3]); amax = amax.max(q[3]); c += 1.0; }
        println!("{n}: {p} {}x{} mean rgba {:?} alpha {amin}..{amax}", t.width, t.height, s.map(|v| (v / c).round()));
    }
}
