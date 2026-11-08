//! Bake a recipe and write an equirectangular preview of its colour cube (raw RGB,
//! width x height) plus timings: `bake_preview <recipe.ini> <out.rgb> [face]`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let r = sta_surface::Recipe::parse(&std::fs::read_to_string(&a[0]).unwrap());
    let face = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(if r.face > 0 { r.face } else { 512 });
    let t = std::time::Instant::now();
    let s = sta_surface::bake(&r, face, 512);
    eprintln!("baked {} at {face}: {:.2}s (clouds: {})", a[0], t.elapsed().as_secs_f32(), s.clouds.is_some());
    let (w, h) = (1024usize, 512usize);
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let lon = (x as f32 + 0.5) / w as f32 * std::f32::consts::TAU;
            let lat = std::f32::consts::FRAC_PI_2 - (y as f32 + 0.5) / h as f32 * std::f32::consts::PI;
            let d = [lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos()];
            let p = sta_surface::sample_nearest(&s.albedo, d);
            let c = s.clouds.as_ref().map(|c| sta_surface::sample_nearest(c, d)[0] as f32 / 255.0 * r.cloud_colour[3]).unwrap_or(0.0);
            for (pk, ck) in p.iter().zip(r.cloud_colour).take(3) {
                out.push((*pk as f32 * (1.0 - c) + 255.0 * ck * c) as u8);
            }
        }
    }
    std::fs::write(&a[1], out).unwrap();
}
