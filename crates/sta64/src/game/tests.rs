use super::*;
use sta_content::fake;
use sta_sim::command::LocalRouter;
use sta_sim::{SimEvent, WorldSource};

fn cfg(opponent: Opponent) -> SkirmishCfg {
    SkirmishCfg { seat: 0, opponent, race: None, opponent_race: None }
}

fn setup(opponent: Opponent) -> (GameSetup, Races) {
    let (map, db) = (Arc::new(fake::two_bases()), Arc::new(fake::db()));
    let races = Races::default();
    let s = seats(&map, &cfg(opponent), &races, &db).expect("two starts");
    (GameSetup { seed: 5, map, content: db, players: s.players, options: Default::default() }, races)
}

#[test]
fn seats_are_the_local_player_and_one_opponent() {
    let (map, db) = (fake::two_bases(), fake::db());
    let s = seats(&map, &cfg(Opponent::Skirmish), &Races::default(), &db).unwrap();
    let controls: Vec<_> = s.players.iter().map(|p| (p.id.0, p.team.0, p.control.clone())).collect();
    assert_eq!(controls, [(0, 0, Control::Human), (1, 1, Control::Ai("skirmish".into()))]);
    assert_eq!(s.players[0].race, "fx");
    assert_eq!(s.local.name, "fx");
    assert!(s.summary.contains("skirmish AI"), "{}", s.summary);

    let idle = seats(&map, &SkirmishCfg { seat: 1, ..cfg(Opponent::Idle) }, &Races::default(), &db).unwrap();
    let controls: Vec<_> = idle.players.iter().map(|p| (p.id.0, p.control.clone())).collect();
    assert_eq!(controls, [(0, Control::Idle), (1, Control::Human)]);

    let missing = seats(&map, &SkirmishCfg { seat: 7, ..cfg(Opponent::Idle) }, &Races::default(), &db);
    assert!(missing.err().unwrap().contains("seat 7"));
    let mut one = map.clone();
    one.starts.truncate(1);
    assert!(seats(&one, &cfg(Opponent::Idle), &Races::default(), &db).is_err());
}

#[test]
fn seats_respect_catalogue_eligibility_including_explicit_requests() {
    let (map, db) = (fake::two_bases(), fake::db());
    assert!(seats(&map, &cfg(Opponent::Idle), &Races::from_defs(Vec::new()), &db)
        .err().unwrap().contains("no playable races"));
    let mut loader = |name: &str, version| {
        if version != 0 { return None; }
        match name.trim_end_matches(".odf") {
            "races" => Some("numberofraces=2\nrace0=allowed\nrace1=observer".into()),
            "allowed" => Some("name=fx\ninstantActionSlot=0".into()),
            "observer" => Some("name=observer".into()),
            _ => None,
        }
    };
    let (races, _) = Races::load_with(&mut loader);
    let wanted = SkirmishCfg { race: Some("observer".into()), ..cfg(Opponent::Idle) };
    let s = seats(&map, &wanted, &races, &db).unwrap();
    assert_eq!(s.local.name, "fx");
    assert!(s.players.iter().all(|p| p.race == "fx"));
}

#[test]
fn offered_player_keys_cannot_be_shadowed_by_an_npc_odf_stem() {
    let (map, db) = (fake::two_bases(), fake::db());
    let (races, _) = Races::load_with(&mut |name, version| {
        if version != 0 { return None; }
        match name.trim_end_matches(".odf") {
            "races" => Some("numberofraces=2\nrace0=allowed\nrace1=fx".into()),
            "allowed" => Some("name=fx\ninstantActionSlot=0\ninterfaceConfiguration=correct.cfg".into()),
            "fx" => Some("name=npc\ninterfaceConfiguration=wrong.cfg".into()),
            _ => None,
        }
    });
    let wanted = SkirmishCfg { race: Some("FX".into()), ..cfg(Opponent::Idle) };
    let s = seats(&map, &wanted, &races, &db).unwrap();
    assert_eq!(s.local.name, "fx");
    assert_eq!(s.local.interface.as_deref(), Some("correct.cfg"));
}

#[test]
fn only_the_ai_seat_gets_a_controller() {
    let (s, _) = setup(Opponent::Skirmish);
    assert_eq!(controllers(&s).len(), 1);
    let (s, _) = setup(Opponent::Idle);
    assert_eq!(controllers(&s).len(), 0);
}

#[test]
fn the_verdict_follows_eliminations() {
    let (s, races) = setup(Opponent::Idle);
    let mut sim = sta_sim::Sim::from_setup_with_starts(&s, &races).0;
    let view = sim.view();
    assert_eq!(verdict(&view, PlayerId(0)), None);
    let mut v = (*view).clone();
    v.players[1].eliminated = true;
    assert_eq!(verdict(&v, PlayerId(0)), Some(Verdict::Victory));
    assert_eq!(verdict(&v, PlayerId(1)), Some(Verdict::Defeat));
    v.players[0].eliminated = true;
    assert_eq!(verdict(&v, PlayerId(0)), Some(Verdict::Defeat));
    let _ = sim.step(&[]);
}

/// The real setup path on fake content: `seats` -> `GameSetup` -> `start` (starting
/// forces and stock, controllers) -> a `SimLink` with the controllers, stepped as the
/// game steps it. The AI seat (1) has a yard and a miner and nothing else; it mines,
/// builds warships, attacks the idle seat (0) and eliminates it.
#[test]
fn the_skirmish_ai_builds_and_attacks_in_the_game_setup_path() {
    let (s, races) = setup(Opponent::Skirmish);
    let (sim, _, ctrl) = start(&s, &races);
    let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)).with_controllers(ctrl);
    assert_eq!(link.controllers(), 1);

    let ai = PlayerId(1);
    let (mut built, mut hits, mut rejected, mut won_at) = (0, 0, 0, None);
    let mut shooters = std::collections::BTreeSet::new();
    for _ in 0..45_000 {
        let events = link.tick().expect("local router never stalls");
        let view = link.curr().clone();
        for e in &events {
            match e {
                SimEvent::Built { id, .. } if view.unit(*id).is_some_and(|u| u.owner == ai) => built += 1,
                SimEvent::Hit { by, .. } => {
                    if let Some(u) = view.unit(*by).filter(|u| u.owner == ai) {
                        hits += 1;
                        shooters.insert(u.def);
                    }
                }
                SimEvent::Rejected { player, .. } if *player == ai => rejected += 1,
                _ => {}
            }
        }
        if verdict(&view, PlayerId(0)).is_some() {
            won_at = Some(view.tick);
            break;
        }
    }
    println!("skirmish AI: {built} units built, {hits} hits from {} kinds of ship, {rejected} rejected, eliminated the idle seat at {won_at:?}", shooters.len());
    assert!(built >= 3, "the AI built warships ({built})");
    assert!(hits > 0, "the AI's ships hit the idle seat");
    assert_eq!(rejected, 0, "no order of the AI was rejected");
    let tick = won_at.expect("the AI wins");
    assert_eq!(verdict(link.curr(), PlayerId(0)), Some(Verdict::Defeat));
    assert_eq!(verdict(link.curr(), ai), Some(Verdict::Victory));
    assert!(tick < 45_000);
}

#[test]
fn an_idle_opponent_never_acts_and_survives() {
    let (s, races) = setup(Opponent::Idle);
    let (sim, _, ctrl) = start(&s, &races);
    assert!(ctrl.is_empty());
    let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)).with_controllers(ctrl);
    let before = link.curr().units.len();
    for _ in 0..600 {
        link.tick().unwrap();
    }
    assert_eq!(link.curr().units.len(), before);
    assert_eq!(verdict(link.curr(), PlayerId(0)), None);
}

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, empty for none): a skirmish on a real
/// multiplayer map (`STA64_TEST_MAP`, default `blueamber`) built as the game builds it.
/// Each seat gets its race's start units and stock, and the AI seat issues orders that
/// the sim accepts. Prints what the AI did in the first minutes.
#[test]
fn install_skirmish_starts_and_the_ai_acts() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let map_name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| "blueamber".into());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (races, _) = Races::load(&vfs, &index);
    let db = Arc::new(db);
    let bzn = sta_bzn::parse(&vfs.read(&format!("bzn/{map_name}.bzn")).expect("map exists").unwrap()).unwrap();
    let cfg = SkirmishCfg { seat: 0, opponent: Opponent::Skirmish, race: std::env::var("STA64_TEST_RACE").ok(), opponent_race: std::env::var("STA64_TEST_OPPONENT_RACE").ok() };
    let side = crate::map::build_sim(&bzn, &map_name, &db, &index, &vfs, Some(0), Some((&cfg, &races)));
    let plan = side.skirmish.expect("a skirmish").unwrap_or_else(|e| panic!("{e}"));
    println!("{}", plan.summary);
    let side_starts: Vec<_> = bzn.start_locs.iter().map(|s| (s.team, s.pos[0] as i64, s.pos[2] as i64)).collect();
    println!("team objects {:?}", bzn.objects.iter().filter(|o| o.team > 0).map(|o| (o.odf.clone(), o.team, o.transform[9] as i64, o.transform[11] as i64)).collect::<Vec<_>>());
    let ai = PlayerId(plan.players.iter().find(|p| matches!(p.control, Control::Ai(_))).unwrap().id.0);
    let mut link = SimLink::new(Box::new(side.sim), Box::new(LocalRouter::new()), PlayerId(0)).with_controllers(side.controllers);
    assert_eq!(link.controllers(), 1);
    let name = |k| db.name(k).unwrap_or("?").to_owned();
    let (mut built, mut rejected, mut mined) = (Vec::new(), 0, 0);
    let view = link.curr().clone();
    for p in &view.players {
        let mine: Vec<_> = view.units.iter().filter(|u| u.owner == p.id).map(|u| name(u.def)).collect();
        let at: Vec<_> = view.units.iter().filter(|u| u.owner == p.id).map(|u| (name(u.def), u.pos.x.to_f32() as i64, u.pos.z.to_f32() as i64)).collect();
        println!("seat {} at {at:?}; starts {:?}", p.id.0, side_starts);
        println!("seat {}: {mine:?} stock {:?}", p.id.0, p.resources.iter().map(|r| r.to_f32() as i64).collect::<Vec<_>>());
        if p.id == ai {
            assert!(!mine.is_empty(), "the AI seat has start units");
        }
    }
    for _ in 0..(30 * 60 * 5) {
        for e in link.tick().unwrap() {
            match e {
                SimEvent::Built { id, .. } => {
                    let v = link.curr();
                    if let Some(u) = v.unit(id).filter(|u| u.owner == ai) {
                        built.push((v.tick, name(u.def)));
                    }
                }
                SimEvent::Rejected { player, .. } if player == ai => rejected += 1,
                SimEvent::Delivered { .. } => mined += 1,
                _ => {}
            }
        }
    }
    let v = link.curr();
    println!("after 5 minutes: AI built {built:?}, {rejected} rejected, {mined} deliveries; AI units now {:?}", v.units.iter().filter(|u| u.owner == ai).map(|u| name(u.def)).collect::<Vec<_>>());
    assert!(!built.is_empty(), "the AI built something in five minutes");
    assert_eq!(rejected, 0, "the sim accepted every order of the AI");
}

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, `STA64_TEST_MAP`): on a real skirmish
/// map (default `boobytrap`) the player's construction ship, ordered to build a structure
/// at sites 100 to 600 off, starts building when the player can pay (the HUD refuses the click when not; the
/// sim, pay on deploy, accepts it and the deploy waits on funds).
#[test]
fn install_skirmish_rig_starts_building() {
    use sta_sim::command::{Command, CommandKind, Edit, Order};
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let map_name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| "boobytrap".into());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (races, _) = Races::load(&vfs, &index);
    let db = Arc::new(db);
    let bzn = sta_bzn::parse(&vfs.read(&format!("bzn/{map_name}.bzn")).expect("map exists").unwrap()).unwrap();
    let cfg = SkirmishCfg { seat: 0, opponent: Opponent::Skirmish, race: Some("federation".into()), opponent_race: None };
    let mut tried = 0;
    for funded in [false, true] {
        for (d, a) in [(150.0f32, 0.0f32), (300.0, 90.0), (300.0, 200.0), (600.0, 300.0), (100.0, 45.0)] {
            let side = crate::map::build_sim(&bzn, &map_name, &db, &index, &vfs, Some(0), Some((&cfg, &races)));
            side.skirmish.expect("a skirmish").unwrap_or_else(|e| panic!("{e}"));
            let bounds = side.setup.map.bounds;
            let mut sim = side.sim;
            if funded {
                let rich = |r: u8| Command { player: PlayerId::BENCH, seq: 1 + r as u32, kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), r, sta_sim::Fixed::from_int(100_000))) };
                sim.step(&[rich(0), rich(1), rich(2), rich(3)]);
            }
            let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0));
            let view = link.curr().clone();
            let rig = view.units.iter().find(|u| u.owner == PlayerId(0) && db.unit(u.def).is_some_and(|d| d.class.0 == "constructionrig")).expect("a rig");
            let bd = db.unit(rig.def).unwrap();
            let what = bd.build.list.iter().find(|r| db.unit(r.key).is_some_and(|d| d.movement.max_speed == sta_sim::Fixed::ZERO && d.build.ticks > 0)).expect("buildable").clone();
            let r = a.to_radians();
            let at = sta_sim::Vec3::from_f32([rig.pos.x.to_f32() + d * r.sin(), rig.pos.y.to_f32(), rig.pos.z.to_f32() + d * r.cos()]);
            // Only sites the placement ghost would show clear: the same rule as the sim's.
            let clear = sta_sim::production::site::check_site(&db, Some(&bounds), what.key, at, sta_sim::production::site::view_obstacles(&db, &view, Some(rig.id)));
            if clear.is_err() {
                println!("{} d{d} a{a}: site {clear:?}, skipped", what.odf);
                continue;
            }
            tried += 1;
            link.submit(CommandKind::Order(Order::Build { builder: rig.id, what: what.key, at: Some(at) }));
            let (mut started, mut rejected) = (false, false);
            for _ in 0..3000 {
                for e in link.tick().unwrap() {
                    started |= matches!(e, SimEvent::BuildStarted { builder, .. } if builder == rig.id);
                    rejected |= matches!(e, SimEvent::Rejected { .. });
                }
                if started {
                    break;
                }
            }
            println!("{} d{d} a{a} funded {funded}: started {started} rejected {rejected}", what.odf);
            assert_eq!((started, rejected), (funded, false), "{} {d} {a}", what.odf);
        }
    }
    assert!(tried >= 4, "only {tried} clear sites tried");
}

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, `STA64_TEST_MAP`): on a real skirmish map the
/// player's freighter, right-clicked on a resource moon, is ordered to mine it, the sim accepts
/// and the freighter starts filling its hold.
#[test]
fn install_skirmish_freighter_mines_a_moon() {
    use sta_sim::command::CommandKind;
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let map_name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| "boobytrap".into());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (races, _) = Races::load(&vfs, &index);
    let db = Arc::new(db);
    let bzn = sta_bzn::parse(&vfs.read(&format!("bzn/{map_name}.bzn")).expect("map exists").unwrap()).unwrap();
    let cfg = SkirmishCfg { seat: 0, opponent: Opponent::Skirmish, race: Some("federation".into()), opponent_race: None };
    let side = crate::map::build_sim(&bzn, &map_name, &db, &index, &vfs, Some(0), Some((&cfg, &races)));
    side.skirmish.expect("a skirmish").unwrap_or_else(|e| panic!("{e}"));
    let mut sim = side.sim;
    // The start gives an outpost; its freighter (the Federation's `fminer`) is built there.
    // Put one beside it, as if built.
    let outpost = sta_sim::WorldSource::view(&sim).units.iter().find(|u| u.owner == PlayerId(0)).expect("a start unit").pos;
    let key = db.key("fminer").expect("the Federation miner");
    assert!(db.unit(key).is_some_and(|d| d.class.0 == "freighter" || d.miner));
    let at = sta_sim::Vec3::from_f32([outpost.x.to_f32() + 150.0, outpost.y.to_f32(), outpost.z.to_f32()]);
    sim.spawn_def(key, PlayerId(0), at, (sta_sim::Fixed::ZERO, sta_sim::Fixed::ONE)).expect("spawned");
    let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0));
    link.tick().unwrap();
    let view = link.curr().clone();
    let freighter = view
        .units
        .iter()
        .find(|u| u.owner == PlayerId(0) && u.def == key)
        .expect("the freighter");
    let dist = |a: sta_sim::Vec3| {
        let (dx, dz) = ((a.x - freighter.pos.x).to_f32(), (a.z - freighter.pos.z).to_f32());
        dx * dx + dz * dz
    };
    let moon = view
        .units
        .iter()
        .filter(|u| db.unit(u.def).is_some_and(|d| d.resource.is_some()))
        .min_by(|a, b| dist(a.pos).total_cmp(&dist(b.pos)))
        .expect("the map places resource moons as resources, not as craft markers");
    assert_eq!(moon.owner, sta_sim::world::NEUTRAL);
    // The right-click on the moon: a Mine order, which the sim accepts.
    let order = crate::input::context::context_order(Some(&*db), &view, PlayerId(0), &[freighter.id], moon.id)
        .expect("a freighter right-clicked on a moon mines it");
    assert!(matches!(&order, sta_sim::command::Order::Mine { node, .. } if *node == moon.id));
    link.submit(CommandKind::Order(order));
    let (mut rejected, mut delivered) = (false, false);
    for _ in 0..20_000 {
        for e in link.tick().unwrap() {
            rejected |= matches!(e, SimEvent::Rejected { .. });
            delivered |= matches!(e, SimEvent::Delivered { player, .. } if player == PlayerId(0));
        }
        if delivered {
            break;
        }
    }
    assert!(!rejected, "the sim accepted the Mine order");
    assert!(delivered, "the freighter mined the moon and unloaded at a dock");
}
