//! Models with the most `team*` faces: `team_survey <install> <mod>`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), Some(&a[1])).unwrap();
    let mut out = Vec::new();
    for name in sta_assets::list_models(&vfs) {
        let Some((p, f)) = sta_assets::find_model(&vfs, &name) else { continue };
        let Ok(m) = sta_assets::load_model(&vfs, &p, f) else { continue };
        let (mut team, mut all) = (0, 0);
        for n in &m.nodes {
            if let Some(mesh) = n.mesh() {
                for g in &mesh.groups {
                    all += g.faces.len();
                    if g.material.to_ascii_lowercase().starts_with("team") {
                        team += g.faces.len();
                    }
                }
            }
        }
        if team > 0 {
            out.push((team * 100 / all.max(1), team, name));
        }
    }
    out.sort();
    println!("{} models use team materials", out.len());
    for (pct, team, name) in out.iter().rev().filter(|o| o.1 > 200).take(12) {
        println!("{pct:3}% {team:5}  {name}");
    }
}
