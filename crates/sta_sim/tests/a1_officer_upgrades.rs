//! In-place officer-quarter progression and an optional STA1 Classic playset.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex, Profile, Races};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::production::view::OfficerUpgradeView;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn count(n: i32) -> Fixed { Fixed::from_int(n) }

fn seat(id: u8, race: &str) -> PlayerSetup {
    PlayerSetup { id: PlayerId(id), team: TeamId(id), race: race.into(), control: Control::Human }
}

fn edit_at(seq: u32, op: Edit) -> Command {
    Command { player: PlayerId::SCRIPT, seq, kind: CommandKind::Edit(op) }
}

fn build(sim: &mut Sim, builder: EntityId, what: ContentKey, seq: u32) -> bool {
    let command = Command {
        player: PlayerId(0), seq, kind: CommandKind::Order(Order::Build { builder, what, at: None }),
    };
    !sim.step(&[command]).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }))
}

fn quarters(sim: &Sim, base: EntityId) -> OfficerUpgradeView {
    sim.production_view().into_iter().find(|r| r.builder == base).unwrap().officer_upgrades.unwrap()
}

fn capacity(sim: &Sim, id: u8) -> Fixed {
    sim.view().player(PlayerId(id)).unwrap().officers[1]
}

fn test_content() -> Arc<ContentDb> {
    let files = [
        ("fed", "name = \"federation\"\nofficerUpgradeODF = \"quarters\""),
        ("base", "classLabel = \"starbase\"\nrace = \"federation\"\nmaxHealth = 100\nmaximumUpgrades = 2\nofficerGain = 20\nbuildItem1 = \"quarters\"\nbuildItem2 = \"enemy\"\nbuildItem3 = \"ship\""),
        ("quarters", "classLabel = \"upgrade\"\nofficerGain = 15\ncrewCost = 4\ndilithiumCost = 10\nbuildTime = 1.0"),
        ("enemy", "classLabel = \"upgrade\"\nofficerGain = 90\nbuildTime = 1.0"),
        ("ship", "classLabel = \"craft\"\nmaxHealth = 100\nofficerCost = 25\nbuildTime = 1.0"),
    ];
    let stems = files.iter().map(|(k, _)| (*k).to_owned()).collect::<Vec<_>>();
    let (db, _) = sta_content::odf::load_with(Profile::Sta1Classic, &Default::default(), &stems, &mut |name, version| {
        if version != 0 { return None; }
        let stem = name.trim_end_matches(".odf");
        files.iter().find(|(k, _)| *k == stem).map(|(_, text)| (*text).to_owned())
    }).unwrap();
    Arc::new(db)
}

#[test]
fn queues_pay_crew_and_complete_on_the_base_then_reset_on_capture() {
    let db = test_content();
    let setup = GameSetup {
        seed: 2, map: Arc::new(MapDef::empty()), content: db.clone(),
        players: vec![seat(0, "federation"), seat(1, "klingon")], options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let p = PlayerId(0);
    let heading = (Fixed::ZERO, Fixed::ONE);
    let base = sim.spawn_def(db.key("base").unwrap(), p, Vec3::ZERO, heading).unwrap();
    let upgrade = db.key("quarters").unwrap();
    assert_eq!(capacity(&sim, 0), count(20));
    assert!(sim.spawn_def(upgrade, p, Vec3::ZERO, heading).is_none());
    assert!(!build(&mut sim, base, db.key("enemy").unwrap(), 1));
    sim.step(&[edit_at(0, Edit::Cargo(p, 0, count(100))), edit_at(1, Edit::TeamCrew(p, count(3)))]);
    assert!(build(&mut sim, base, upgrade, 2));
    assert!(!sim.production_view().iter().find(|r| r.builder == base).unwrap().jobs[0].paid);
    assert_eq!(sim.view().player(p).unwrap().resources[0], count(100));
    sim.step(&[edit_at(2, Edit::TeamCrew(p, count(20)))]);
    assert_eq!(sim.view().player(p).unwrap().crew, Some(count(16)));
    assert_eq!(sim.view().player(p).unwrap().resources[0], count(90));
    let row = sim.production_view().into_iter().find(|r| r.builder == base).unwrap();
    assert!(row.jobs[0].started && row.site.is_none());
    assert!(build(&mut sim, base, upgrade, 3));
    assert_eq!(quarters(&sim, base).remaining, 0);
    assert!(!build(&mut sim, base, upgrade, 4));
    sim = Sim::from_snapshot(&sim.snapshot(), Some(db.clone())).unwrap();
    assert_eq!(quarters(&sim, base).remaining, 0);
    let ticks = db.unit(upgrade).unwrap().build.ticks;
    for _ in 0..2 * ticks + 4 {
        sim.step(&[]);
    }
    assert_eq!(quarters(&sim, base).completed, 2);
    assert_eq!(sim.view().player(p).unwrap().crew, Some(count(12)));
    assert_eq!(sim.view().player(p).unwrap().resources[0], count(80));
    assert!(!sim.view().units.iter().any(|u| u.def == upgrade));
    assert!(!build(&mut sim, base, upgrade, 5));
    assert_eq!(capacity(&sim, 0), count(50));
    let saved_digest = sim.digest();
    sim = Sim::from_snapshot(&sim.snapshot(), Some(db.clone())).unwrap();
    assert_eq!(sim.digest(), saved_digest);
    assert_eq!(capacity(&sim, 0), count(50));
    let ship = db.key("ship").unwrap();
    assert!(build(&mut sim, base, ship, 6));
    for _ in 0..db.unit(ship).unwrap().build.ticks + 2 { sim.step(&[]); }
    assert!(sim.view().units.iter().any(|u| u.def == ship));
    assert_eq!(sim.view().player(p).unwrap().officers, [count(25), count(50)]);
    sim.step(&[edit_at(3, Edit::Team(base, PlayerId(1)))]);
    assert_eq!((capacity(&sim, 0), capacity(&sim, 1)), (Fixed::ZERO, count(20)));
    assert_eq!(quarters(&sim, base).completed, 0);
    assert!(sim.despawn(base, None));
    assert_eq!(capacity(&sim, 1), Fixed::ZERO);
}

#[test]
fn the_installed_a1compat_starbase_offers_and_builds_each_races_upgrade() {
    let Ok(root) = std::env::var("STA64_TEST_A1_COMPAT_INSTALL") else { return; };
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some("STA1 Classic")).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let db = Arc::new(ContentDb::load(&vfs, &index).unwrap().0);
    assert!(db.a1_officers(), "the required A1Compat module enables this policy");
    let (races, _) = Races::load(&vfs, &index);
    for (race, base, upgrade) in [
        ("federation", "fbase", "fedoff"), ("klingon", "kbase", "klingoff"),
        ("romulan", "rbase", "romoff"), ("borg", "bbase", "borgoff"),
    ] {
        let setup = GameSetup {
            seed: 3, map: Arc::new(sta_content::fake::two_bases()), content: db.clone(),
            players: vec![seat(0, race), seat(1, race)], options: GameOptions::default(),
        };
        let (mut sim, _) = Sim::from_setup_with_starts(&setup, &races);
        let base_key = db.key(base).unwrap();
        let upgrade = db.key(upgrade).unwrap();
        let base_def = db.unit(base_key).unwrap();
        let base = sim.view().units.iter().find(|u| u.def == base_key && u.owner == PlayerId(0)).unwrap().id;
        assert_eq!(base_def.a1_base_officers.unwrap().upgrade, Some(upgrade), "{race}");
        assert_eq!(capacity(&sim, 0), count(20), "{race}");
        assert!(build(&mut sim, base, upgrade, 1), "{race}");
        for _ in 0..db.unit(upgrade).unwrap().build.ticks + 2 { sim.step(&[]); }
        assert_eq!(quarters(&sim, base), OfficerUpgradeView { completed: 1, remaining: 5 }, "{race}");
        assert_eq!(capacity(&sim, 0), count(40), "{race}");
        assert!(!sim.view().units.iter().any(|u| u.def == upgrade));
    }
}
