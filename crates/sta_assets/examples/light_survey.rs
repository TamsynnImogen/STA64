//! Map lighting data of every BZN in a mounted install: the map's ambient colour and its
//! `dlight` objects (direction from the transform's front axis, colour):
//! `light_survey <install> [mod]`. Counts and values only.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&a[0]), a.get(1).map(String::as_str)).unwrap();
    let mut names: Vec<String> = vfs.list("bzn").into_iter().filter(|p| p.to_ascii_lowercase().ends_with(".bzn")).collect();
    names.sort();
    let mut by_count = std::collections::BTreeMap::<usize, usize>::new();
    for p in names {
        let Some(Ok(data)) = vfs.read(&p) else { continue };
        let Ok(m) = sta_bzn::parse(&data) else { continue };
        let lights: Vec<String> = m
            .objects
            .iter()
            .filter(|o| o.odf.to_ascii_lowercase().contains("light"))
            .map(|o| {
                let f = [o.transform[6], o.transform[7], o.transform[8]];
                format!("dir({:.2},{:.2},{:.2}) col {:?}", f[0], f[1], f[2], o.light_colour.map(|c| [(c[0] * 100.0).round() / 100.0, (c[1] * 100.0).round() / 100.0, (c[2] * 100.0).round() / 100.0]))
            })
            .collect();
        *by_count.entry(lights.len()).or_default() += 1;
        println!("{p}: ambient {:.2?} dlights {} {}", m.ambient, lights.len(), lights.join("; "));
    }
    println!("maps by dlight count: {by_count:?}");
}
