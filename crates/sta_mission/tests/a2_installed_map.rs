//! Read-only, opt-in A2 mission probe against an installed map and an
//! external clean-room port. No game or port bytes are bundled with tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex};
use sta_core::{EntityId, PlayerId, TeamId};
use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};
use sta_vfs::Vfs;

#[test]
#[ignore = "requires STA64_A2_INSTALL and STA64_A2_PORTS external directories"]
fn into_the_breach_installed_map_reaches_first_borg_wave() {
    let install =
        PathBuf::from(std::env::var("STA64_A2_INSTALL").expect("set STA64_A2_INSTALL to an installed game root"));
    let ports = PathBuf::from(std::env::var("STA64_A2_PORTS").expect("set STA64_A2_PORTS to external a2/missions"));
    let mod_name = std::env::var("STA64_A2_MOD").unwrap_or_else(|_| "STA2 Classic".into());
    let vfs = Arc::new(Vfs::mount_install(Path::new(&install), Some(&mod_name)).expect("mount installed game"));
    let (index, index_warnings) = OdfIndex::build(&vfs);
    let (db, content_warnings) = ContentDb::load(&vfs, &index).expect("load installed content");
    let bytes = vfs
        .read("bzn/a2_fed05.bzn")
        .expect("installed a2_fed05.bzn")
        .expect("read a2_fed05.bzn");
    let bzn = sta_bzn::parse(&bytes).expect("parse installed A2 map");
    let (map, map_warnings) = MapDef::from_bzn_in(&bzn, &db, &vfs, &index);
    let classes: Vec<_> = ["fente", "bportal", "fbase"]
        .into_iter()
        .map(|name| {
            let def = db
                .key(name)
                .and_then(|key| db.unit(key))
                .unwrap_or_else(|| panic!("installed unit ODF {name} missing"));
            (name, def.class.0.as_str())
        })
        .collect();
    eprintln!(
        "installed A2: mod={mod_name}, profile={:?}, content_hash={:#018x}, bzn_objects={}, map_objects={}, regions={}, warnings(index/content/map)={}/{}/{}",
        db.profile(),
        db.hash(),
        bzn.objects.len(),
        map.objects.len(),
        map.regions.len(),
        index_warnings.len(),
        content_warnings.len(),
        map_warnings.len()
    );
    eprintln!("installed A2 parsed classes: {classes:?}");
    let count = |name: &str| {
        map.objects
            .iter()
            .filter(|o| o.odf.trim_end_matches(".odf").eq_ignore_ascii_case(name))
            .count()
    };
    for name in ["fente", "bportal", "fbase"] {
        assert!(count(name) > 0, "installed map has no {name} object");
    }
    let db = Arc::new(db);
    let setup = GameSetup {
        seed: 1,
        map: Arc::new(map),
        content: db.clone(),
        players: (0..8)
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "test".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    };
    let (mut sim, spawn_warnings) = Sim::from_setup_with_warnings(&setup);
    eprintln!("installed A2 sim spawn warnings={}", spawn_warnings.len());
    for name in ["fente", "bportal", "fbase"] {
        let key = db.key(name).unwrap();
        let owners: Vec<_> = sim
            .view()
            .units
            .iter()
            .filter(|u| u.def == key)
            .map(|u| (u.id.0, u.owner.0))
            .collect();
        eprintln!("installed A2 {name} actual (id,core_slot)={owners:?}");
    }
    let cruiser1 = db.key("bcruise1").expect("installed bcruise1 ODF");
    let cruiser2 = db.key("bcruise2").expect("installed bcruise2 ODF");
    let cfg = MissionConfig {
        seat: Some(PlayerId(0)),
        player_numbering: PlayerNumbering::ArmadaTeams,
        content: Some(db),
        labels: sim.labels.iter().map(|(id, label)| (id, label.clone())).collect(),
        view: Some((*sim.view()).clone()),
        paths: Vec::new(),
    };
    let source = std::fs::read_to_string(ports.join("Into the Breach.lua")).expect("read external clean-room port");
    let observed = format!(
        "local register=MMM.register\nMMM.register=function(o) __port_instance=o; return register(o) end\n{source}"
    );
    let reader = vfs.clone();
    let mut rt = MissionRuntime::load_with("missions/Into the Breach.lua", 1, cfg, move |path| {
        if path == "missions/Into the Breach.lua" {
            Some(observed.as_bytes().to_vec())
        } else {
            reader.read(path).and_then(Result::ok)
        }
    });
    eprintln!(
        "installed A2 load errors={:?}; unsupported={:?}",
        rt.errors(),
        rt.unsupported()
    );
    assert!(rt.errors().is_empty(), "mission load errors: {:?}", rt.errors());

    let mut events = Vec::new();
    let mut rejected = Vec::new();
    let mut enterprise = None;
    let mut requests = BTreeMap::new();
    let mut receipts = BTreeMap::new();
    let mut attacks = Vec::new();
    for tick in 0..6150 {
        let mut commands = Vec::new();
        rt.think(&sim.view(), &events, &mut commands);
        if tick == 720 {
            assert!(rt.errors().is_empty(), "mission setup errors: {:?}", rt.errors());
            let handles: (Option<u32>, bool, Option<u32>, bool, Option<u32>, bool) = rt.eval(
                "return __port_instance.enterprise and __port_instance.enterprise.id, __port_instance.enterprise and __port_instance.enterprise.valid or false, __port_instance.portal and __port_instance.portal.id, __port_instance.portal and __port_instance.portal.valid or false, __port_instance.base and __port_instance.base.id, __port_instance.base and __port_instance.base.valid or false"
            ).expect("inspect port after opening");
            assert!(
                handles.0.is_some() && handles.1,
                "installed Enterprise not found/valid: {handles:?}"
            );
            assert!(
                handles.2.is_some() && handles.3,
                "installed portal not found/valid: {handles:?}"
            );
            assert!(
                handles.4.is_some() && handles.5,
                "installed base not found/valid: {handles:?}"
            );
            eprintln!(
                "installed A2 post-opening port handles: enterprise={:?}, portal={:?}, base={:?}",
                handles.0, handles.2, handles.4
            );
            enterprise = handles.0.map(EntityId);
        }
        for command in &commands {
            match &command.kind {
                CommandKind::Edit(Edit::SpawnRequest {
                    request, odf, player, ..
                }) => {
                    assert_eq!(command.player, PlayerId::SCRIPT);
                    assert_eq!(*player, PlayerId(3), "first Borg wave owner");
                    assert!(
                        *odf == cruiser1 || *odf == cruiser2,
                        "unexpected first-wave ODF {odf:?}"
                    );
                    assert!(
                        requests.insert(*request, (tick, *odf)).is_none(),
                        "duplicate request token {request}"
                    );
                }
                CommandKind::Order(Order::Attack { units, target }) if command.player == PlayerId(3) => {
                    attacks.push((tick, units.clone(), *target));
                }
                _ => {}
            }
        }
        let out = sim.step(&commands);
        for event in &out.events {
            match event {
                SimEvent::ScriptSpawned { request, id } => {
                    assert!(requests.contains_key(request), "receipt without wave request {request}");
                    assert!(
                        receipts.insert(*request, *id).is_none(),
                        "duplicate receipt for {request}"
                    );
                    let unit = out.view.unit(*id).expect("first-wave spawn alive in receipt view");
                    assert_eq!(unit.owner, PlayerId(3));
                    assert_eq!(unit.def, requests[request].1);
                }
                SimEvent::Rejected { player, seq } => rejected.push((tick, *player, *seq)),
                _ => {}
            }
        }
        events = out.events;
    }
    eprintln!(
        "installed A2 after 6150 ticks: runtime_errors={:?}; unsupported={:?}; sim_rejections={:?}; spawn_requests={:?}; receipts={:?}; attacks={:?}",
        rt.errors(),
        rt.unsupported(),
        rejected,
        requests,
        receipts,
        attacks
    );
    assert!(rt.errors().is_empty(), "mission Lua errors: {:?}", rt.errors());
    assert_eq!(rt.unsupported(), [("Player.loadAIP".to_owned(), 1)], "unexpected unsupported mission call");
    assert_eq!(rt.eval::<String>("return __port_instance.phase").unwrap(), "gameplay");
    assert!(rejected.is_empty(), "mission commands rejected by sim: {rejected:?}");
    assert_eq!(requests.len(), 4, "first Borg wave should request four craft");
    assert_eq!(receipts.len(), 4, "each first-wave request needs a real receipt");
    let mut by_def = BTreeMap::new();
    for &(tick, odf) in requests.values() {
        assert_eq!(tick, 6120, "first wave must be naturally scheduled after opening");
        *by_def.entry(odf).or_insert(0usize) += 1;
    }
    assert_eq!(by_def.get(&cruiser1), Some(&2));
    assert_eq!(by_def.get(&cruiser2), Some(&2));
    assert_eq!(attacks.len(), 4, "each first-wave craft should receive an Attack");
    let born: BTreeSet<_> = receipts.values().copied().collect();
    let mut attacked = BTreeSet::new();
    for (tick, units, target) in attacks {
        assert_eq!(tick, 6121, "Attack must follow receipt processing");
        assert_eq!(Some(target), enterprise, "Attack target must be the actual Enterprise");
        assert_eq!(units.len(), 1);
        assert!(born.contains(&units[0]), "Attack actor has no first-wave receipt");
        assert!(attacked.insert(units[0]), "duplicate first-wave Attack actor");
    }
    assert_eq!(attacked, born);
}
