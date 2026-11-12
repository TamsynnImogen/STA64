//! Headless checks of the real map/skirmish and front-end conversion callers.

use super::{Catalogue, build_sim};
use crate::app::Opponent;
use crate::game::SkirmishCfg;
use sta_bzn::a1::{AiPath, Header, MapData};
use sta_content::{Fixed, OdfIndex, Races};
use sta_sim::WorldSource;
use std::path::PathBuf;
use std::sync::Arc;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sta64_selected_start_{}_{stamp}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn idle_skirmish() -> SkirmishCfg {
    SkirmishCfg {
        seat: 0,
        opponent: Opponent::Idle,
        race: None,
        opponent_race: None,
    }
}

#[test]
fn selected_a1_companion_reaches_shared_skirmish_setup() {
    let root = TempRoot::new();
    std::fs::create_dir(root.0.join("bzn")).unwrap();
    std::fs::write(
        root.0.join("bzn/renamed.mdf"),
        b"StartLocations = 2\nStart1 = 0 0\nStart2 = 1 1\n",
    )
    .unwrap();
    // An internal-name companion would prevent skirmish setup if selected.
    std::fs::write(root.0.join("bzn/internal.mdf"), b"StartLocations = 9\n").unwrap();
    let mut vfs = sta_vfs::Vfs::default();
    vfs.push_dir("root".into(), root.0.clone()).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let points = [[1000.25, 38.5, 1000.5], [5000.25, -7.5, 5000.5]];
    let map = sta_bzn::Map {
        file_name: "internal.bzn".into(),
        min_extent: [0.0, -100.0, 0.0],
        size: [6400.0, 200.0, 6400.0],
        a1: Some(MapData {
            header: Header {
                version: 2051,
                save_game_desc: [0; 128],
                binary_save: true,
                metadata_offset: 0,
            },
            terrain_name: String::new(),
            source_bounds: ([0.0, -100.0, 0.0], [6400.0, 100.0, 6400.0]),
            resized: false,
            declared_object_count: 0,
            object_refs: Vec::new(),
            initial_positions: Vec::new(),
            mission_title: [0; 64],
            mission_reference: 0,
            paths: points
                .iter()
                .enumerate()
                .map(|(i, point)| AiPath {
                    reference: 0,
                    label: format!("Camera_{}", i + 1),
                    points: vec![*point],
                    path_type: 0,
                    asteroid_belt_speed: 0.0,
                })
                .collect(),
            areas: Vec::new(),
        }),
        ..Default::default()
    };
    let content = Arc::new(sta_content::fake::db());
    let races = Races::default();
    let cfg = idle_skirmish();
    let side = build_sim(&map, "renamed", &content, &index, &vfs, Some(0), Some((&cfg, &races)));
    assert!(side.skirmish.as_ref().unwrap().is_ok());
    assert_eq!(side.setup.map.name, "renamed");
    assert_eq!(side.setup.map.starts.len(), 2);
    assert_eq!(side.setup.players.len(), 2);
    let view = side.sim.view();
    for (i, point) in points.iter().enumerate() {
        let start = side.setup.map.start_for(i as u8).unwrap();
        assert_eq!(start.pos.x, Fixed::from_f64(f64::from(point[0])));
        assert_eq!(start.pos.y, Fixed::from_f64(f64::from(point[1])));
        assert_eq!(start.pos.z, Fixed::from_f64(f64::from(point[2])));
        let units: Vec<_> = view.units.iter().filter(|u| usize::from(u.owner.0) == i).collect();
        assert!(!units.is_empty(), "seat {i} must receive actual shared loadout units");
        for unit in units {
            assert_eq!(unit.pos.y, start.pos.y);
            assert!((unit.pos.x - start.pos.x).abs() < Fixed::from_int(500));
            assert!((unit.pos.z - start.pos.z).abs() < Fixed::from_int(500));
        }
    }
    assert_eq!(map.file_name, "internal.bzn");
}

#[test]
#[ignore = "requires STA64_TEST_A1_INSTALL pointing at user-owned original A1 install"]
fn installed_a1_maps_reach_front_end_and_shared_skirmish_without_app_startup() {
    let root = PathBuf::from(std::env::var_os("STA64_TEST_A1_INSTALL").expect("set STA64_TEST_A1_INSTALL"));
    let vfs = sta_vfs::Vfs::mount_install(&root, None).unwrap();
    let mut catalogue = Catalogue::default();
    let data = crate::game::flow::front_end_data(&vfs, &mut catalogue, Some(&root)).unwrap();
    assert_eq!(data.screens.maps.len(), 28);
    assert_eq!(data.screens.maps.iter().map(|m| m.def.starts.len()).sum::<usize>(), 113);
    assert!(data.screens.maps.iter().any(|m| m.def.name == "2blue"));
    let source = sta_bzn::parse(&vfs.read("bzn/2blue.bzn").unwrap().unwrap()).unwrap();
    let content = catalogue.content(&vfs).unwrap();
    let cfg = idle_skirmish();
    let side = build_sim(
        &source,
        "2blue",
        &content.db,
        &content.index,
        &vfs,
        Some(0),
        Some((&cfg, &content.races)),
    );
    let seats = side.skirmish.as_ref().unwrap().as_ref().expect("A1 skirmish seats");
    assert_eq!(seats.players.len(), 2);
    assert_eq!(side.setup.map.starts.len(), 2);
    let view = side.sim.view();
    for player in &side.setup.players {
        assert!(
            view.units.iter().any(|u| u.owner == player.id),
            "seat {:?} has no initial units",
            player.id
        );
    }
    eprintln!(
        "A1 client headless: 28 menu maps, 113 starts; 2blue two-seat setup has {} units",
        view.units.len()
    );
}
