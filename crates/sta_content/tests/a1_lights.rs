//! Install-gated (STA64_TEST_DIR, the stock Armada I folder; skipped when unset):
//! the per-object light colours of the binary A1 map bodies (CON-41) reach the
//! converted maps' lights. Every light the BZN carries (a `dlight` object with its
//! colour record shape) becomes a `SceneryLight` with that colour, not the white
//! default.

use sta_content::{ContentDb, MapDef, OdfIndex};
use sta_vfs::Vfs;

#[test]
fn a1_lights_carry_their_file_colours() {
    let Ok(dir) = std::env::var("STA64_TEST_DIR") else {
        eprintln!("SKIP: set STA64_TEST_DIR to the stock Armada I folder");
        return;
    };
    let mut vfs = Vfs::default();
    vfs.push_dir("root".into(), dir.clone().into()).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (mut file_lights, mut converted, mut coloured, mut problems) = (0, 0, 0, Vec::new());
    for path in vfs
        .list("bzn")
        .into_iter()
        .filter(|p| p.to_ascii_lowercase().ends_with(".bzn"))
    {
        let Some(Ok(data)) = vfs.read(&path) else {
            continue;
        };
        let Ok(bzn) = sta_bzn::parse(&data) else {
            continue;
        };
        let mut expected: Vec<[f32; 3]> =
            bzn.objects.iter().filter_map(|o| o.light_colour).collect();
        file_lights += expected.len();
        let map = MapDef::convert_bzn_in(&bzn, &db, &vfs, &index).map;
        converted += map.scenery.lights.len();
        for light in &map.scenery.lights {
            match expected.iter().position(|c| *c == light.colour) {
                Some(i) => {
                    expected.swap_remove(i);
                    coloured += (light.colour != [1.0; 3]) as usize;
                }
                None => problems.push(format!(
                    "{path}: light colour {:?} is not a file colour",
                    light.colour
                )),
            }
        }
    }
    eprintln!(
        "A1 lights: {file_lights} in the files, {converted} in converted maps, {coloured} not white"
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(converted, file_lights);
    assert!(
        coloured > 0,
        "no light has a colour but white: the colours were not read"
    );
}
