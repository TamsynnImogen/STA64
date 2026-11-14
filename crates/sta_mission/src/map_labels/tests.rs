use std::collections::BTreeSet;

use super::*;

fn object(odf: &str, label: &str) -> sta_bzn::MapObject {
    sta_bzn::MapObject { odf: odf.into(), label: label.into(), ..Default::default() }
}

fn class(odf: &str) -> String {
    match odf {
        "fx_cloud" => "nebula_obj",
        "fx_rocks" => "asteroidfield",
        "fx_star" | "fx_lamp_light" => "background_obj",
        "fx_gate" => "wormhole_obj",
        _ => "unit",
    }
    .to_owned()
}

fn gaps(objects: Vec<sta_bzn::MapObject>, carried: &[&str]) -> Vec<LabelGap> {
    let map = sta_bzn::Map { objects, ..Default::default() };
    let carried: BTreeSet<String> = carried.iter().map(|s| (*s).to_owned()).collect();
    find_label_gaps(&map, &carried, &mut |o| class(o))
}

#[test]
fn a_map_label_no_sim_object_carries_is_listed_with_its_kind() {
    let got = gaps(
        vec![
            object("fx_cloud", "Cloud1"),
            object("fx_rocks", "rocks_a"),
            object("fx_star", "Star"),
            object("fx_lamp_light", "Lamp1"),
            object("fx_gate", "Gate1"),
            object("fx_ship", "Hero"),
            object("fx_lamp_light", ""),
            object("fx_cloud", "Cloud1"),
        ],
        &["Hero"],
    );
    let got: Vec<_> = got.iter().map(|g| (g.label.as_str(), g.kind)).collect();
    assert_eq!(
        got,
        [("Cloud1", LabelKind::Region), ("rocks_a", LabelKind::Region), ("Star", LabelKind::Prop), ("Lamp1", LabelKind::Light), ("Gate1", LabelKind::Wormhole)],
        "carried and unlabelled objects are not gaps, a repeated label is listed once, a scenery light has its own kind"
    );
}

#[test]
fn a_label_in_no_map_object_is_not_listed_and_a_carried_one_is_not_either() {
    let got = gaps(vec![object("fx_cloud", "Cloud1"), object("fx_ship", "Hero")], &["Hero"]);
    assert!(got.iter().all(|g| g.label != "Sentinel" && g.label != "Hero"), "{got:?}");
    assert_eq!(got.len(), 1);
}

#[test]
fn a_label_with_surrounding_space_that_the_sim_carries_trimmed_is_no_gap() {
    // `MapDef` trims a label's whitespace; the comparison trims the same way.
    assert!(gaps(vec![object("fx_ship", "  Hero \0")], &["Hero"]).is_empty());
    let got = gaps(vec![object("fx_cloud", " Cloud1 ")], &[]);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].label, "Cloud1");
}

#[test]
fn an_unreadable_map_gives_no_gaps() {
    let dir = std::env::temp_dir().join(format!("sta64-lua25-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Data").join("bzn")).unwrap();
    let vfs = Vfs::mount_install(&dir, None).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    assert!(read_map_label_gaps(&vfs, &index, "missing.bzn", &BTreeSet::new()).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}
