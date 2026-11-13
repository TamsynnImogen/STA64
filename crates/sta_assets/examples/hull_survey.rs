//! Which SOD material features occur, over every SOD of a mounted install:
//! `hull_survey <install> [mod]`. Lighting-material values (ambient / diffuse /
//! specular / power / model byte / self-illumination), texture-material blends and mode
//! bytes, bump flags, culling, plus the distinct values each field takes. A counting
//! survey: it prints no game data beyond key names and numbers.
use std::collections::BTreeMap;

fn bump(map: &mut BTreeMap<String, (usize, String)>, key: String, ex: &str) {
    let e = map.entry(key).or_insert((0, ex.to_owned()));
    e.0 += 1;
}

fn q(v: f32) -> String {
    format!("{:.2}", v)
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), a.get(1).map(String::as_str)).unwrap();
    let mut c: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut models = 0;
    for name in sta_assets::list_models(&vfs) {
        let Some((path, sta_assets::ModelFormat::Sod)) = sta_assets::find_model(&vfs, &name) else { continue };
        let Ok(sod) = sta_assets::load_sod(&vfs, &path) else { continue };
        models += 1;
        bump(&mut c, format!("version {:.2}", sod.version), &name);
        for m in &sod.lighting_materials {
            let team = m.name.to_ascii_lowercase().starts_with("team");
            bump(&mut c, format!("lm model={} self_illum={} team={team}", m.model, m.self_illuminating), &name);
            bump(&mut c, format!("lm ambient {} {} {}", q(m.ambient[0]), q(m.ambient[1]), q(m.ambient[2])), &name);
            bump(&mut c, format!("lm diffuse {} {} {}", q(m.diffuse[0]), q(m.diffuse[1]), q(m.diffuse[2])), &name);
            bump(&mut c, format!("lm specular {} {} {}", q(m.specular[0]), q(m.specular[1]), q(m.specular[2])), &name);
            bump(&mut c, format!("lm power {}", q(m.power)), &name);
            let mx = |v: [f32; 3]| v.iter().cloned().fold(0.0, f32::max);
            bump(&mut c, format!("lm model={} ambient>0 {} specular>0.1 {} power>=1 {}", m.model, mx(m.ambient) > 0.0, mx(m.specular) > 0.1, m.power >= 1.0), &name);
            bump(&mut c, format!("lm name {}", m.name.to_ascii_lowercase()), &name);
        }
        for t in &sod.texture_materials {
            bump(&mut c, format!("tm modes {:?} param {}", t.modes, t.param), &name);
        }
        for n in &sod.nodes {
            let Some(m) = n.mesh() else { continue };
            let tm = if sod.texture_materials.iter().any(|t| t.ident.eq_ignore_ascii_case(&m.texture_material)) {
                "<ident>".to_owned()
            } else {
                m.texture_material.to_ascii_lowercase()
            };
            bump(&mut c, format!("mesh blend {tm}"), &name);
            bump(&mut c, format!("mesh flags {} cull {} textures {} tex2 {}", m.flags, m.cull, m.textures.len(), !m.texture2.is_empty()), &name);
            for g in &m.groups {
                let lm = sod.lighting_materials.iter().find(|l| l.name.eq_ignore_ascii_case(&g.material));
                bump(&mut c, format!("group material {} (defined {})", g.material.to_ascii_lowercase(), lm.is_some()), &name);
                bump(&mut c, format!("group faces/blend {tm} {}", if lm.is_some_and(|l| l.self_illuminating) { "self_illum" } else { "plain" }), &name);
            }
        }
    }
    println!("{models} SOD models");
    for (k, (n, ex)) in &c {
        println!("{n:7} {k}   e.g. {ex}");
    }
}
