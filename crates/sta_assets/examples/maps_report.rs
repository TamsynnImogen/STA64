//! Every map of a mod through the map view's lookups (ODF -> model -> textures):
//! `maps_report <install> <mod>`
use std::collections::{BTreeMap, HashMap};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), Some(&a[1])).unwrap();
    let mut idx: HashMap<String, String> = HashMap::new();
    for dir in ["odf", "addon"] {
        for p in vfs.list(dir) {
            if let Some(stem) = p.strip_suffix(".odf") {
                idx.entry(stem.rsplit('/').next().unwrap().to_owned()).or_insert(p.clone());
            }
        }
    }
    let mut odfs: HashMap<String, Option<sta_odf::Odf>> = HashMap::new();
    let mut models: HashMap<String, Option<sta_assets::model::Model>> = HashMap::new();
    let mut no_odf: BTreeMap<String, usize> = BTreeMap::new();
    let mut no_model: BTreeMap<String, usize> = BTreeMap::new();
    let mut no_tex: BTreeMap<String, usize> = BTreeMap::new();
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    let (mut maps, mut objects) = (0, 0);
    let table = sta_assets::load_sprite_table(&vfs);
    let mut sky_bad: BTreeMap<String, usize> = BTreeMap::new();
    let mut stars_bad: BTreeMap<String, usize> = BTreeMap::new();
    let mut planet_tex_bad: BTreeMap<String, usize> = BTreeMap::new();
    for p in vfs.list("bzn").into_iter().filter(|p| p.ends_with(".bzn")) {
        let Some(Ok(d)) = vfs.read(&p) else { continue };
        let Ok(m) = sta_bzn::parse(&d) else { continue };
        maps += 1;
        // Sky: a cube SOD, or six faces <prefix>0..5 / <prefix>.1..6.
        let bg = m.background.trim();
        if !bg.is_empty() {
            let ok = if bg.to_ascii_lowercase().ends_with(".sod") {
                sta_assets::find_model(&vfs, bg).is_some()
            } else {
                (0..6).all(|i| sta_assets::find_texture(&vfs, &format!("{bg}{i}")).is_some())
                    || (1..=6).all(|i| sta_assets::find_texture(&vfs, &format!("{bg}.{i}")).is_some())
            };
            if !ok {
                *sky_bad.entry(bg.to_ascii_lowercase()).or_default() += 1;
            }
        }
        // Starfield ODF and its sprites.
        let sf = m.starfield_odf.trim().trim_end_matches(".odf").to_ascii_lowercase();
        if !sf.is_empty() {
            let mut load = |n: &str| {
                let s = n.to_ascii_lowercase();
                idx.get(s.trim_end_matches(".odf")).and_then(|p| vfs.read(p)).and_then(Result::ok).map(|b| String::from_utf8_lossy(&b).into_owned())
            };
            match sta_odf::resolve(&sf, &mut load) {
                Ok(o) => {
                    let bad: Vec<String> = o.keys().filter(|k| !k.to_ascii_lowercase().ends_with(".scale") && table.sprite(k).is_none()).map(str::to_owned).collect();
                    if !bad.is_empty() {
                        *stars_bad.entry(format!("{sf}: sprites {}", bad.join(" "))).or_default() += 1;
                    }
                }
                Err(_) => *stars_bad.entry(format!("{sf}: no ODF")).or_default() += 1,
            }
        }
        for o in &m.objects {
            objects += 1;
            let k = o.odf.to_ascii_lowercase();
            let odf = odfs.entry(k.clone()).or_insert_with(|| {
                let mut load = |n: &str| {
                    let s = n.to_ascii_lowercase();
                    idx.get(s.trim_end_matches(".odf")).and_then(|p| vfs.read(p)).and_then(Result::ok).map(|b| String::from_utf8_lossy(&b).into_owned())
                };
                sta_odf::resolve(&k, &mut load).ok()
            });
            let Some(odf) = odf else {
                *no_odf.entry(k).or_default() += 1;
                continue;
            };
            let class = odf.class_label().unwrap_or("?").to_ascii_lowercase();
            *classes.entry(class.clone()).or_default() += 1;
            if class == "planet" {
                let ground = odf.str("groundTextureName").unwrap_or("").trim().to_owned();
                if !ground.is_empty() {
                    for h in 1..=2 {
                        if sta_assets::find_texture(&vfs, &format!("{ground}{h}")).is_none() {
                            *planet_tex_bad.entry(format!("{ground}{h}")).or_default() += 1;
                        }
                    }
                }
                let atmo = odf.str("atmosphereTextureName").filter(|a| !a.trim().is_empty()).unwrap_or("PA_ECFR0").trim().to_owned();
                if sta_assets::find_texture(&vfs, &atmo).is_none() {
                    *planet_tex_bad.entry(atmo).or_default() += 1;
                }
            }
            if odf.i64("hidden").unwrap_or(0) != 0 || odf.i64("dont_load_SOD").unwrap_or(0) != 0 || class == "planet" || class == "asteroidfield" {
                continue;
            }
            let base = odf.str("basename").filter(|b| !b.trim().is_empty()).map(|b| b.trim().to_ascii_lowercase()).unwrap_or(k.clone());
            let model = models.entry(base.clone()).or_insert_with(|| {
                sta_assets::find_model(&vfs, &base).and_then(|(p, f)| sta_assets::load_model(&vfs, &p, f).ok())
            });
            let Some(model) = model else {
                *no_model.entry(format!("{k} ({class})")).or_default() += 1;
                continue;
            };
            for n in &model.nodes {
                if let Some(mesh) = n.mesh() {
                    for g in &mesh.groups {
                        if let Some(t) = &g.maps.diffuse
                            && sta_assets::find_texture(&vfs, t).is_none()
                        {
                            *no_tex.entry(t.to_ascii_lowercase()).or_default() += 1;
                        }
                    }
                }
            }
        }
    }
    println!("{maps} maps, {objects} objects, {} distinct ODFs", odfs.len());
    println!("classes: {classes:?}");
    let top = |m: &BTreeMap<String, usize>| {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(*e.1));
        v.into_iter().take(15).map(|(k, n)| format!("{k} x{n}")).collect::<Vec<_>>().join(", ")
    };
    println!("no ODF ({}): {}", no_odf.len(), top(&no_odf));
    println!("no model ({}): {}", no_model.len(), top(&no_model));
    println!("missing diffuse ({}): {}", no_tex.len(), top(&no_tex));
    println!("skies not found ({}): {}", sky_bad.len(), top(&sky_bad));
    println!("starfield problems ({}): {}", stars_bad.len(), top(&stars_bad));
    println!("planet textures missing ({}): {}", planet_tex_bad.len(), top(&planet_tex_bad));
}
