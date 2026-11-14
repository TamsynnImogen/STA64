use std::collections::BTreeSet;

use super::*;

fn object(odf: &str, label: &str) -> sta_bzn::MapObject {
    sta_bzn::MapObject { odf: odf.into(), label: label.into(), ..Default::default() }
}

fn class(odf: &str) -> String {
    match odf {
        "mnebula1" => "nebula_obj",
        "mafield" => "asteroidfield",
        "msun1" | "dlight" => "background_obj",
        "mwrmhole" => "wormhole_obj",
        _ => "unit",
    }
    .to_owned()
}

#[test]
fn a_map_label_no_sim_object_carries_is_listed_with_its_kind() {
    let map = sta_bzn::Map {
        objects: vec![
            object("mnebula1", "Nebula1"),
            object("mafield", "asteroid_1"),
            object("msun1", "Sun"),
            object("mwrmhole", "Worm1"),
            object("mship", "Hero"),
            object("dlight", ""),
            object("mnebula1", "Nebula1"),
        ],
        ..Default::default()
    };
    let carried: BTreeSet<String> = ["Hero".to_owned()].into();
    let gaps = find_label_gaps(&map, &carried, &mut |o| class(o));
    let got: Vec<_> = gaps.iter().map(|g| (g.label.as_str(), g.kind)).collect();
    assert_eq!(
        got,
        [("Nebula1", LabelKind::Region), ("asteroid_1", LabelKind::Region), ("Sun", LabelKind::Prop), ("Worm1", LabelKind::Wormhole)],
        "carried and unlabelled objects are not gaps, a repeated label is listed once"
    );
}

#[test]
fn a_label_in_no_map_object_is_not_listed() {
    let map = sta_bzn::Map { objects: vec![object("mnebula1", "Nebula1")], ..Default::default() };
    let gaps = find_label_gaps(&map, &BTreeSet::new(), &mut |o| class(o));
    assert!(gaps.iter().all(|g| g.label != "Sentinel"));
}
