use std::collections::BTreeMap;

use sta_core::{EntityId, Fixed, Vec3};
use sta_sim::{MoveStats, Sim, WorldSource};

use crate::api::MissionConfig;
use crate::map_labels::{LabelGap, LabelKind};
use crate::runtime::{MissionEndState, MissionRuntime};

fn gap(label: &str, odf: &str, kind: LabelKind) -> LabelGap {
    LabelGap { label: label.into(), odf: odf.into(), kind }
}

/// One live unit labelled `Hero`; the map also has the gaps given.
fn scene(gaps: Vec<LabelGap>) -> (MissionRuntime, EntityId) {
    let mut sim = Sim::new(1);
    let hero = sim.spawn(0, Vec3::from_f32([0.0, 0.0, 0.0]), (Fixed::ZERO, Fixed::ONE), MoveStats::from_f32(10., 1., 1., 1., 0.2));
    let labels: BTreeMap<EntityId, String> = [(hero, "Hero".to_owned())].into();
    let cfg = MissionConfig { labels, view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with("g.lua", "", 1, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.set_label_gaps(gaps);
    (rt, hero)
}

fn region_gaps() -> Vec<LabelGap> {
    vec![gap("Nebula1", "mnebula1", LabelKind::Region), gap("Worm1", "mwrmhole", LabelKind::Wormhole)]
}

fn listed(rt: &MissionRuntime) -> Vec<String> {
    rt.unsupported().into_iter().map(|(k, _)| k).filter(|k| k.contains("GetHandle")).collect()
}

#[test]
fn a2_a_label_in_the_map_and_unresolved_is_a_handle_gap() {
    let (rt, _) = scene(region_gaps());
    assert_eq!(rt.eval::<i64>("return A2.GetHandle('Nebula1')").unwrap(), 0);
    let l = listed(&rt);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(l[0].starts_with("A2.GetHandle: label 'Nebula1': region `mnebula1`"), "{l:?}");
}

#[test]
fn a1_a_label_in_the_map_and_unresolved_is_a_handle_gap_and_matches_without_case() {
    let (rt, _) = scene(region_gaps());
    assert!(rt.eval::<bool>("return A1.GetHandle('nebula1') == nil or A1.GetHandle('nebula1') == 0").unwrap());
    let l = listed(&rt);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(l[0].starts_with("A1.GetHandle: label 'Nebula1'"), "{l:?}");
}

#[test]
fn a_wormhole_label_is_listed_apart_with_its_own_text() {
    let (rt, _) = scene(region_gaps());
    rt.eval::<i64>("return A2.GetHandle('Worm1')").unwrap();
    let l = listed(&rt);
    assert!(l[0].contains("wormhole object, not an entity yet (LUA-24)"), "{l:?}");
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
    let (rt, hero) = scene(vec![gap("Hero", "mship", LabelKind::Other)]);
    assert_eq!(rt.eval::<i64>("return A2.GetHandle('Hero')").unwrap(), i64::from(hero.0));
    assert!(listed(&rt).is_empty());
}

#[test]
fn a_gap_blocks_an_end_that_reads_the_missing_handle() {
    let (rt, _) = scene(region_gaps());
    rt.eval::<()>("local h = A2.GetHandle('Nebula1') if not A2.IsAlive(h) then A2.SucceedMission(0, 0) end").unwrap();
    match rt.mission_end(5) {
        MissionEndState::Blocked(why) => assert!(why.contains("IsAlive") && why.contains("GetHandle"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_gap_does_not_block_an_end_that_reads_only_live_objects() {
    let (rt, hero) = scene(region_gaps());
    rt.eval::<()>(&format!("local _ = A2.GetHandle('Nebula1') if A2.IsAlive({}) then A2.SucceedMission(0, 0) end", hero.0)).unwrap();
    assert_eq!(rt.mission_end(5), MissionEndState::Ended { won: true });
}
