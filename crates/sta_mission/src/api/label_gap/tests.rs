use std::collections::BTreeMap;

use sta_core::{EntityId, Fixed, Vec3};
use sta_sim::{MoveStats, Sim, WorldSource};

use crate::api::MissionConfig;
use crate::map_labels::{LabelGap, LabelKind};
use crate::runtime::{MissionEndState, MissionRuntime};

fn gap(label: &str, odf: &str, kind: LabelKind) -> LabelGap {
    LabelGap { label: label.into(), odf: odf.into(), kind }
}

fn config() -> (MissionConfig, EntityId, Sim) {
    let mut sim = Sim::new(1);
    let hero = sim.spawn(0, Vec3::from_f32([0.0, 0.0, 0.0]), (Fixed::ZERO, Fixed::ONE), MoveStats::from_f32(10., 1., 1., 1., 0.2));
    let labels: BTreeMap<EntityId, String> = [(hero, "Hero".to_owned())].into();
    (MissionConfig { labels, view: Some((*sim.view()).clone()), ..MissionConfig::default() }, hero, sim)
}

/// One live unit labelled `Hero`; the map also has the gaps given.
fn scene(gaps: Vec<LabelGap>) -> (MissionRuntime, EntityId) {
    let (cfg, hero, _) = config();
    let rt = MissionRuntime::from_source_with_gaps("g.lua", "", 1, cfg, gaps);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    (rt, hero)
}

fn region_gaps() -> Vec<LabelGap> {
    vec![
        gap("Cloud1", "fx_cloud", LabelKind::Region),
        gap("Gate1", "fx_gate", LabelKind::Wormhole),
        gap("Lamp1", "fx_lamp_light", LabelKind::Light),
    ]
}

fn listed(rt: &MissionRuntime) -> Vec<String> {
    rt.unsupported().into_iter().map(|(k, _)| k).filter(|k| k.contains("GetHandle")).collect()
}

#[test]
fn a2_a_label_in_the_map_and_unresolved_is_a_handle_gap() {
    let (rt, _) = scene(region_gaps());
    assert_eq!(rt.eval::<i64>("return A2.GetHandle('Cloud1')").unwrap(), 0);
    let l = listed(&rt);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(l[0].starts_with("A2.GetHandle: label 'Cloud1': region `fx_cloud`"), "{l:?}");
}

#[test]
fn a2_matches_the_label_exactly() {
    let (rt, _) = scene(region_gaps());
    assert_eq!(rt.eval::<i64>("return A2.GetHandle('cloud1')").unwrap(), 0);
    assert!(listed(&rt).is_empty(), "the original compares case-sensitively, so 'cloud1' names nothing in the map");
}

#[test]
fn a1_a_label_in_the_map_and_unresolved_is_a_handle_gap_and_matches_without_case() {
    let (rt, _) = scene(region_gaps());
    assert!(rt.eval::<bool>("local h = A1.GetHandle('cloud1') return h == nil or h == 0").unwrap());
    let l = listed(&rt);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(l[0].starts_with("A1.GetHandle: label 'Cloud1'"), "{l:?}");
}

#[test]
fn a_wormhole_and_a_scenery_light_are_listed_with_their_own_text() {
    let (rt, _) = scene(region_gaps());
    rt.eval::<i64>("return A2.GetHandle('Gate1')").unwrap();
    rt.eval::<i64>("return A2.GetHandle('Lamp1')").unwrap();
    let l = listed(&rt);
    assert!(l[0].contains("wormhole object, not placed"), "{l:?}");
    assert!(l[1].contains("prop (light name) `fx_lamp_light`"), "{l:?}");
}

#[test]
fn a_label_in_neither_the_map_nor_the_script_is_no_gap() {
    let (rt, _) = scene(region_gaps());
    assert_eq!(rt.eval::<i64>("return A2.GetHandle('Sentinel')").unwrap(), 0);
    assert!(rt.eval::<bool>("local h = A1.GetHandle('Sentinel') return h == nil or h == 0").unwrap());
    assert!(listed(&rt).is_empty());
    // Nothing is tainted either: an end after a read of that handle stands.
    rt.eval::<()>("local h = A2.GetHandle('Sentinel') if not A2.IsAlive(h) then A2.SucceedMission(0, 0) end").unwrap();
    assert_eq!(rt.mission_end(5), MissionEndState::Ended { won: true });
}

#[test]
fn a_resolved_label_is_no_gap_even_when_the_map_list_names_it() {
    let (rt, hero) = scene(vec![gap("Hero", "fx_ship", LabelKind::Other)]);
    assert_eq!(rt.eval::<i64>("return A2.GetHandle('Hero')").unwrap(), i64::from(hero.0));
    assert!(listed(&rt).is_empty());
}

#[test]
fn a_label_carried_by_a_dead_unit_is_no_gap() {
    // The host lists only labels no sim object carries: `Hero` is carried at start, so it is not in the
    // list built from the map. Its unit then leaves the view: `GetHandle` answers 0 as the original does,
    // with no record, while a real gap still records.
    let map = sta_bzn::Map {
        objects: vec![
            sta_bzn::MapObject { odf: "fx_ship".into(), label: "Hero".into(), ..Default::default() },
            sta_bzn::MapObject { odf: "fx_cloud".into(), label: "Cloud1".into(), ..Default::default() },
        ],
        ..Default::default()
    };
    let carried = ["Hero".to_owned()].into();
    let gaps = crate::map_labels::find_label_gaps(&map, &carried, &mut |o| if o == "fx_cloud" { "nebula_obj".into() } else { "unit".into() });
    assert_eq!(gaps.len(), 1);
    let (cfg, hero, sim) = config();
    let mut rt = MissionRuntime::from_source_with_gaps("g.lua", "", 1, cfg, gaps);
    let mut v = (*sim.view()).clone();
    v.units.retain(|u| u.id != hero);
    sta_sim::setup::Controller::think(&mut rt, &v, &[], &mut Vec::new());
    assert_eq!(rt.eval::<i64>("return A2.GetHandle('Hero')").unwrap(), 0);
    assert!(listed(&rt).is_empty());
    rt.eval::<i64>("return A2.GetHandle('Cloud1')").unwrap();
    assert_eq!(listed(&rt).len(), 1);
}

#[test]
fn a_gap_blocks_an_end_that_reads_the_missing_handle() {
    let (rt, _) = scene(region_gaps());
    rt.eval::<()>("local h = A2.GetHandle('Cloud1') if not A2.IsAlive(h) then A2.SucceedMission(0, 0) end").unwrap();
    match rt.mission_end(5) {
        MissionEndState::Blocked(why) => assert!(why.contains("IsAlive") && why.contains("GetHandle"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_gap_does_not_block_an_end_that_reads_only_live_objects() {
    let (rt, hero) = scene(region_gaps());
    rt.eval::<()>(&format!("local _ = A2.GetHandle('Cloud1') if A2.IsAlive({}) then A2.SucceedMission(0, 0) end", hero.0)).unwrap();
    assert_eq!(rt.mission_end(5), MissionEndState::Ended { won: true });
}

/// Known over-block (the rule is global, `Taint::handle_gap`): once one label gap is recorded, a
/// read of any handle that never existed is tainted, also the 0 of a label in no map object, which
/// is not a gap by itself. Pinned so a change of the rule shows; the coordinator decides about it.
#[test]
fn pinned_over_block_a_gap_taints_a_later_read_of_a_sentinel_handle() {
    let (rt, _) = scene(region_gaps());
    rt.eval::<()>("local _ = A2.GetHandle('Cloud1') local h = A2.GetHandle('Sentinel') if not A2.IsAlive(h) then A2.SucceedMission(0, 0) end").unwrap();
    assert!(matches!(rt.mission_end(5), MissionEndState::Blocked(_)), "{:?}", rt.mission_end(5));
    // The form is global, not by value: a nonzero handle that never existed is tainted as well.
    let (rt, _) = scene(region_gaps());
    rt.eval::<()>("local _ = A2.GetHandle('Cloud1') if not A2.IsAlive(424242) then A2.SucceedMission(0, 0) end").unwrap();
    assert!(matches!(rt.mission_end(5), MissionEndState::Blocked(_)), "{:?}", rt.mission_end(5));
}

#[test]
fn a_get_handle_at_script_chunk_load_records_the_gap() {
    let (cfg, _, _) = config();
    let rt = MissionRuntime::from_source_with_gaps("g.lua", "H = A2.GetHandle('Cloud1')", 1, cfg, region_gaps());
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(listed(&rt).len(), 1, "{:?}", rt.unsupported());
}
