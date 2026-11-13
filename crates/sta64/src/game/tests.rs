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
            "allowed" => Some("name=fx\ninstantActionSlot=0\ninterfaceConfiguration=correct.cfg\ninterfaceSprites=authored.spr".into()),
            "fx" => Some("name=npc\ninterfaceConfiguration=wrong.cfg\ninterfaceSprites=wrong.spr".into()),
            _ => None,
        }
    });
    let wanted = SkirmishCfg { race: Some("FX".into()), ..cfg(Opponent::Idle) };
    let s = seats(&map, &wanted, &races, &db).unwrap();
    assert_eq!(s.local.name, "fx");
    assert_eq!(s.local.interface.as_deref(), Some("correct.cfg"));
    assert_eq!(s.local.interface_sprites.as_deref(), Some("authored.spr"));
}

#[test]
fn local_race_skin_resets_between_matches_and_does_not_follow_selection() {
    use crate::ui::gui_sprites::GuiSkin;
    let mut app = App::new();
    app.init_resource::<GuiSkin>()
        .insert_resource(crate::ui::hud::HudConfig::default())
        .insert_resource(crate::sim_link::Selection::default())
        .add_systems(Update, apply_race);
    app.insert_resource(LocalRace {
        name: "local".into(), interface: None,
        interface_sprites: Some("authored.spr".into()), music: None,
    });
    app.update();
    assert_eq!(app.world().resource::<GuiSkin>().table.as_deref(), Some("authored.spr"));
    app.world_mut().resource_mut::<crate::sim_link::Selection>().0.push(sta_sim::EntityId(123));
    app.update();
    assert_eq!(app.world().resource::<GuiSkin>().table.as_deref(), Some("authored.spr"));
    app.insert_resource(LocalRace {
        name: "no_skin".into(), interface: None, interface_sprites: None, music: None,
    });
    app.update();
    assert_eq!(app.world().resource::<GuiSkin>().table, None);
    app.world_mut().resource_mut::<LocalRace>().interface_sprites = Some("second.spr".into());
    app.update();
    assert_eq!(app.world().resource::<GuiSkin>().table.as_deref(), Some("second.spr"));
    app.world_mut().remove_resource::<LocalRace>();
    app.update();
    assert_eq!(app.world().resource::<GuiSkin>().table, None);
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
/// player's freighter, right-clicked on each kind of resource moon the map places (dilithium,
/// tritanium, ...), is ordered to mine it, the sim accepts and the freighter fills its hold and
/// unloads at a dock.
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
    let centre = [bzn.min_extent[0] + bzn.size[0] * 0.5, bzn.min_extent[2] + bzn.size[2] * 0.5];
    let mut tried: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    // One trial per resource definition the map places (a moon of each kind).
    for nth in 0.. {
        let side = crate::map::build_sim(&bzn, &map_name, &db, &index, &vfs, Some(0), Some((&cfg, &races)));
        side.skirmish.expect("a skirmish").unwrap_or_else(|e| panic!("{e}"));
        let side_map = side.setup.map.clone();
        let mut sim = side.sim;
        let moons: Vec<_> = sta_sim::WorldSource::view(&sim)
            .units
            .iter()
            .filter_map(|u| db.unit(u.def).filter(|d| d.resource.is_some()).map(|d| (d.odf.clone(), u.id, u.pos)))
            .collect();
        let mut kinds: Vec<&String> = Vec::new();
        for (odf, ..) in &moons {
            if !kinds.contains(&odf) {
                kinds.push(odf);
            }
        }
        let Some(kind) = kinds.get(nth).map(|k| (*k).clone()) else { break };
        let (_, moon_id, moon_pos) = moons.iter().find(|m| m.0 == kind).cloned().unwrap();
        // The start gives an outpost; its freighter (the Federation's `fminer`) is built there.
        // Put one beside the moon, on the side of the map's centre, as if built.
        let key = db.key("fminer").expect("the Federation miner");
        assert!(db.unit(key).is_some_and(|d| d.mines()));
        let (dx, dz) = (centre[0] - moon_pos.x.to_f32(), centre[1] - moon_pos.z.to_f32());
        let len = (dx * dx + dz * dz).sqrt().max(1.0);
        let at = sta_sim::Vec3::from_f32([moon_pos.x.to_f32() + dx / len * 250.0, moon_pos.y.to_f32(), moon_pos.z.to_f32() + dz / len * 250.0]);
        sim.spawn_def(key, PlayerId(0), at, (sta_sim::Fixed::ZERO, sta_sim::Fixed::ONE)).expect("spawned");
        let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0));
        link.tick().unwrap();
        let view = link.curr().clone();
        let freighter = view.units.iter().find(|u| u.owner == PlayerId(0) && u.def == key && u.pos == at).expect("the freighter");
        let moon = view.unit(moon_id).expect("the moon");
        assert_eq!(moon.owner, sta_sim::world::NEUTRAL);
        // The right-click on the moon: a Mine order, which the sim accepts.
        let order = crate::input::context::context_order(Some(&*db), &view, PlayerId(0), &[freighter.id], moon.id)
            .unwrap_or_else(|| panic!("a freighter right-clicked on {kind} mines it"));
        assert!(matches!(&order, sta_sim::command::Order::Mine { node, .. } if *node == moon.id));
        link.submit(CommandKind::Order(order));
        let (mut rejected, mut delivered) = (false, false);
        for _ in 0..30_000 {
            for e in link.tick().unwrap() {
                rejected |= matches!(e, SimEvent::Rejected { .. });
                delivered |= matches!(e, SimEvent::Delivered { player, .. } if player == PlayerId(0));
            }
            if delivered {
                break;
            }
        }
        let f = link.curr().unit(freighter.id).map(|u| u.pos);
        let inside: Vec<String> = side_map.regions.iter().filter_map(|r| match r.shape {
            sta_content::RegionShape::Sphere { centre, radius } => {
                let d = |p: sta_sim::Vec3| { let (x, z) = ((p.x - centre.x).to_f32(), (p.z - centre.z).to_f32()); (x * x + z * z).sqrt() };
                (d(moon_pos) < radius.to_f32() || f.is_some_and(|f| d(f) < radius.to_f32())).then(|| format!("{:?} {} moon_in {} fr_in {}", r.kind, r.odf, d(moon_pos) < radius.to_f32(), f.is_some_and(|f| d(f) < radius.to_f32())))
            }
            _ => None,
        }).collect();
        println!("{kind}: moon {moon_id:?} at {:?} start {at:?} end {f:?} rejected {rejected} delivered {delivered} regions {inside:?}", moon_pos);
        tried.push(kind.clone());
        // A freighter next to a hostile base can be shot before it delivers: delivery is only
        // required of one that is still alive.
        if rejected || (f.is_some() && !delivered) {
            failed.push(kind);
        }
    }
    assert!(!tried.is_empty(), "the map places resource moons");
    assert!(failed.is_empty(), "no mining of {failed:?} (tried {tried:?})");
}

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, `STA64_TEST_MAP`): which unit the real picking
/// (`unit_under`) returns for rays aimed at the centre of each resource moon of a real map, with
/// the pickable set the map view gives (a click radius from the model's bounds for mobile,
/// player-owned and resource units; hidden `hidden = 1` units are not drawn). Strategic icons are
/// not modelled. Prints one line per moon.
#[test]
fn install_map_picks_resource_moons() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let map_name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| "Adapt".into());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (races, _) = Races::load(&vfs, &index);
    let db = Arc::new(db);
    let bzn = sta_bzn::parse(&vfs.read(&format!("bzn/{map_name}.bzn")).expect("map exists").unwrap()).unwrap();
    let cfg = SkirmishCfg { seat: 0, opponent: Opponent::Skirmish, race: Some("federation".into()), opponent_race: None };
    let side = crate::map::build_sim(&bzn, &map_name, &db, &index, &vfs, Some(0), Some((&cfg, &races)));
    side.skirmish.expect("a skirmish").unwrap_or_else(|e| panic!("{e}"));
    let view = sta_sim::WorldSource::view(&side.sim);
    let mut app = App::new();
    let mut names = std::collections::HashMap::new();
    let mut pickable = 0;
    for u in &view.units {
        let Some(def) = db.unit(u.def) else { continue };
        names.insert(u.id, format!("{}@{}", def.odf, u.owner.0));
        if def.raw.i64("hidden").unwrap_or(0) != 0 {
            continue;
        }
        let owned = u.owner != sta_sim::world::NEUTRAL;
        if !(def.movement.max_speed > sta_sim::Fixed::ZERO || owned || def.resource.is_some()) {
            continue;
        }
        let Some((path, fmt)) = sta_assets::find_model(&vfs, def.model.to_ascii_lowercase().trim_end_matches(".sod")) else { continue };
        let Ok(model) = sta_assets::load_model(&vfs, &path, fmt) else { continue };
        let scale = def.raw.f32("ScaleSOD").filter(|s| *s > 0.0).unwrap_or(1.0);
        let radius = model.bounds().map(|(lo, hi)| (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5).unwrap_or(20.0) * scale;
        let p = crate::render::units::to_bevy(u.pos.to_f32());
        app.world_mut().spawn((crate::sim_link::SimEntity(u.id), crate::render::units::SelectRadius(radius.max(5.0)), GlobalTransform::from(Transform::from_translation(p))));
        pickable += 1;
    }
    println!("pickable entities: {pickable}");
    let mut bad = 0;
    for u in &view.units {
        let Some(def) = db.unit(u.def).filter(|d| d.resource.is_some()) else { continue };
        let centre = crate::render::units::to_bevy(u.pos.to_f32());
        let mut hits: std::collections::BTreeMap<String, usize> = Default::default();
        for az in [0.0f32, 90.0, 180.0, 270.0] {
            for pitch in [35.0f32, 55.0, 75.0, 89.0] {
                let (a, p) = (az.to_radians(), pitch.to_radians());
                let dir = Vec3::new(a.cos() * p.cos(), -p.sin(), a.sin() * p.cos());
                let ray = Ray3d::new(centre - dir * 4000.0, Dir3::new(dir).unwrap());
                let hit = app.world_mut().run_system_once(move |units: crate::input::select::Targetable| crate::input::select::unit_under(Vec2::ZERO, ray, &units)).unwrap();
                let name = hit.map(|id| names.get(&id).cloned().unwrap_or_default()).unwrap_or_else(|| "nothing".into());
                *hits.entry(name).or_default() += 1;
            }
        }
        let own = hits.get(&format!("{}@{}", def.odf, u.owner.0)).copied().unwrap_or(0);
        bad += usize::from(own < 16);
        println!("{:?} {}: {hits:?}", u.id, def.odf);
    }
    println!("moons not always picked: {bad}");
    assert_eq!(bad, 0, "a ray at a moon's centre picks the moon");
}
