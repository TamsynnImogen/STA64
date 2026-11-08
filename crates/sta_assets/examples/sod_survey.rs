//! Survey texture usage across every SOD in a mounted install.
//! `cargo run -p sta_assets --example sod_survey -- <install> [mod]`
use std::collections::BTreeMap;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), a.get(1).map(String::as_str)).unwrap();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: BTreeMap<String, String> = BTreeMap::new();
    let mut bump_named = 0;
    let mut bump_named_exists = 0;
    for name in sta_assets::list_models(&vfs) {
        let Some((path, sta_assets::ModelFormat::Sod)) = sta_assets::find_model(&vfs, &name) else { continue };
        let Ok(sod) = sta_assets::load_sod(&vfs, &path) else { continue };
        for n in &sod.nodes {
            let Some(m) = n.mesh() else { continue };
            let mut bump = |k: &str, ex: String| {
                *counts.entry(k.to_owned()).or_default() += 1;
                examples.entry(k.to_owned()).or_insert(ex);
            };
            bump(&format!("version {:.2}", sod.version), name.clone());
            bump(&format!("texture_material {}", m.texture_material.to_ascii_lowercase()), name.clone());
            bump(&format!("mesh flags {}", m.flags), name.clone());
            bump(&format!("texture count {}", m.textures.len()), name.clone());
            for t in &m.textures {
                bump(&format!("tex flags {:#x}", t.flags), format!("{name}: {}", t.name));
            }
            if !m.texture2.is_empty() {
                bump("texture2 set", format!("{name}: {}", m.texture2));
            }
            if let Some(d) = m.diffuse_texture() {
                bump_named += 1;
                if sta_assets::find_texture(&vfs, &format!("{d}_bump")).is_some() {
                    bump_named_exists += 1;
                }
                let lower = d.to_ascii_lowercase();
                if lower.ends_with("_b") {
                    bump("diffuse ends _b", format!("{name}: {d}"));
                }
            }
            for g in &m.groups {
                bump(&format!("lighting {}", g.material.to_ascii_lowercase().trim_end_matches(char::is_numeric)), name.clone());
            }
        }
        if !sod.anim_refs.is_empty() {
            *counts.entry("sods with anim refs".into()).or_default() += 1;
        }
    }
    for (k, v) in &counts {
        println!("{v:6}  {k}   e.g. {}", examples.get(k).map(String::as_str).unwrap_or(""));
    }
    println!("meshes with diffuse: {bump_named}; with <diffuse>_bump texture present: {bump_named_exists}");
}
