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

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, `STA64_TEST_MAP`): what the real picking returns
/// across the visible disc of each resource moon when the owned units next to it are drawn as
/// strategic icons (a unit whose model is under `SYMBOL_ENTER_PX` pixels tall), at 3440x1440 and
/// 1920x1080 with the camera 60 degrees down, aimed at the moon, at a few distances. The units are
/// those seat 0 sees at the start (`visible_to`); an icon is a silhouette (`silhouette_size`) at
/// the model's centre. Prints one line per moon, resolution and distance: the share of the disc
/// that picks something else and what. A research probe: it only asserts that the moon is picked
/// at its centre when no icon is within reach of it.
#[test]
fn install_map_icons_over_resource_moons() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use crate::render::strategic::{Strategic, centre_for_bounds, shape_for, silhouette_size, SYMBOL_ENTER_PX};
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
    let seen: std::collections::BTreeSet<_> = view.visible_to(PlayerId(0)).iter().map(|u| u.id).collect();
    struct P { id: sta_sim::EntityId, name: String, pos: Vec3, radius: f32, centre: Vec3, owned: bool, class: String, mobile: bool }
    let mut all: Vec<P> = Vec::new();
    for u in &view.units {
        let Some(def) = db.unit(u.def) else { continue };
        if def.raw.i64("hidden").unwrap_or(0) != 0 {
            continue;
        }
        let owned = u.owner != sta_sim::world::NEUTRAL;
        let mobile = def.movement.max_speed > sta_sim::Fixed::ZERO;
        if !(mobile || owned || def.resource.is_some()) {
            continue;
        }
        let Some((path, fmt)) = sta_assets::find_model(&vfs, def.model.to_ascii_lowercase().trim_end_matches(".sod")) else { continue };
        let Ok(model) = sta_assets::load_model(&vfs, &path, fmt) else { continue };
        let scale = def.raw.f32("ScaleSOD").filter(|s| *s > 0.0).unwrap_or(1.0);
        let b = model.bounds();
        let radius = (b.map(|(lo, hi)| (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5).unwrap_or(20.0) * scale).max(5.0);
        let centre = b.map(|(lo, hi)| centre_for_bounds(lo, hi, scale)).unwrap_or_default();
        all.push(P { id: u.id, name: format!("{}@{}", def.odf, u.owner.0), pos: crate::render::units::to_bevy(u.pos.to_f32()), radius, centre, owned, class: def.class.0.clone(), mobile });
    }
    let moons: Vec<&P> = all.iter().filter(|p| !p.owned && db.unit(view.unit(p.id).unwrap().def).is_some_and(|d| d.resource.is_some())).collect();
    for moon in &moons {
        println!("MOON {:?} {} radius {:.0} visible_to_seat0 {}", moon.id, moon.name, moon.radius, seen.contains(&moon.id));
    }
    let fov = std::f32::consts::FRAC_PI_4;
    for (w, h) in [(3440.0f32, 1440.0f32), (1920.0, 1080.0)] {
        let focal = h * 0.5 / (fov * 0.5).tan();
        for dist in [2500.0f32, 5000.0, 9000.0] {
            for moon in &moons {
                let target = moon.pos;
                let pitch = 60f32.to_radians();
                let eye = target + Vec3::new(0.0, pitch.sin(), pitch.cos()) * dist;
                let cam = Transform::from_translation(eye).looking_at(target, Vec3::Y);
                let (rot, inv) = (cam.rotation, cam.rotation.inverse());
                let project = |p: Vec3| -> Option<(Vec2, f32)> {
                    let c = inv * (p - eye);
                    let depth = -c.z;
                    (depth > 1.0).then(|| (Vec2::new(w * 0.5 + c.x / depth * focal, h * 0.5 - c.y / depth * focal), depth))
                };
                let mut app = App::new();
                for p in all.iter().filter(|p| p.owned || seen.contains(&p.id)) {
                    let gt = GlobalTransform::from(Transform::from_translation(p.pos));
                    let mut e = app.world_mut().spawn((crate::sim_link::SimEntity(p.id), crate::render::units::SelectRadius(p.radius), gt));
                    if p.owned && !matches!(p.class.as_str(), "background_obj" | "latinum" | "asteroidfield" | "nebula_obj" | "planet" | "wormhole_obj") {
                        let mut s = Strategic::new(shape_for(&p.class, p.mobile), Color::WHITE, p.radius).with_centre(p.centre);
                        s.size_px = silhouette_size(p.radius);
                        let centre = p.pos + p.centre;
                        if let Some((px, depth)) = project(centre) {
                            let projected = p.radius * focal / depth;
                            if projected <= SYMBOL_ENTER_PX && px.x >= 0.0 && px.x <= w && px.y >= 0.0 && px.y <= h {
                                s.symbol = true;
                                s.shown = 1.0;
                                s.screen = px;
                            }
                        }
                        e.insert(s);
                    }
                }
                let Some((c_px, depth)) = project(moon.pos) else { continue };
                let r_px = moon.radius * focal / depth;
                let mut pts = vec![c_px];
                for f in [0.25f32, 0.5, 0.75, 0.95] {
                    for k in 0..8 {
                        let a = k as f32 * std::f32::consts::FRAC_PI_4;
                        pts.push(c_px + Vec2::new(a.cos(), a.sin()) * r_px * f);
                    }
                }
                let mut other: std::collections::BTreeMap<String, usize> = Default::default();
                let mut centre_hit = String::new();
                for (i, px) in pts.iter().enumerate() {
                    let v = Vec3::new((px.x - w * 0.5) / focal, -(px.y - h * 0.5) / focal, -1.0);
                    let ray = Ray3d::new(eye, Dir3::new(rot * v).unwrap());
                    let cursor = *px;
                    let hit = app.world_mut().run_system_once(move |units: crate::input::select::Targetable| crate::input::select::unit_under(cursor, ray, &units)).unwrap();
                    let name = hit.and_then(|id| all.iter().find(|p| p.id == id)).map_or("nothing".to_owned(), |p| p.name.clone());
                    if i == 0 {
                        centre_hit = name.clone();
                    }
                    if hit != Some(moon.id) {
                        *other.entry(name).or_default() += 1;
                    }
                }
                let stolen: usize = other.values().sum();
                println!("PICK {w}x{h} dist {dist} {:?} {} disc {:.0}px: {:.0}% not the moon (centre {centre_hit}) {other:?}", moon.id, moon.name, r_px, 100.0 * stolen as f32 / pts.len() as f32);
            }
        }
    }
}

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, `STA64_TEST_MAP`): the miners a skirmish seat
/// owns at the start (a map's preplaced ones) are right-clicked onto each resource moon the seat
/// sees at the start. Prints the order `context_order` gives, whether the sim accepted it and
/// whether the miner delivered; also what a group of a miner and a non-miner gets.
#[test]
fn install_preplaced_miners_mine_the_moons_the_seat_sees() {
    use sta_sim::command::CommandKind;
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
    let build = || {
        let side = crate::map::build_sim(&bzn, &map_name, &db, &index, &vfs, Some(0), Some((&cfg, &races)));
        side.skirmish.expect("a skirmish").unwrap_or_else(|e| panic!("{e}"));
        side.sim
    };
    let first = build();
    let view = sta_sim::WorldSource::view(&first);
    let moons: Vec<_> = view.visible_to(PlayerId(0)).iter().filter(|u| db.unit(u.def).is_some_and(|d| d.resource.is_some())).map(|u| (u.id, db.unit(u.def).unwrap().odf.clone())).collect();
    let miners: Vec<_> = view.units.iter().filter(|u| u.owner == PlayerId(0) && db.unit(u.def).is_some_and(|d| d.mines())).map(|u| (u.id, db.unit(u.def).unwrap().odf.clone(), u.pos)).collect();
    let other = view.units.iter().find(|u| u.owner == PlayerId(0) && db.unit(u.def).is_some_and(|d| !d.mines() && d.movement.max_speed > sta_sim::Fixed::ZERO)).map(|u| u.id);
    println!("seat 0 sees {} moons, owns {} miners; non-miner mobile unit {other:?}", moons.len(), miners.len());
    drop(first);
    for (moon, moon_odf) in &moons {
        for (miner, miner_odf, _) in miners.iter().take(3) {
            let mut link = SimLink::new(Box::new(build()), Box::new(LocalRouter::new()), PlayerId(0));
            link.tick().unwrap();
            let view = link.curr().clone();
            let order = crate::input::context::context_order(Some(&*db), &view, PlayerId(0), &[*miner], *moon);
            let mixed = other.and_then(|o| crate::input::context::context_order(Some(&*db), &view, PlayerId(0), &[*miner, o], *moon));
            let (mut rejected, mut delivered) = (false, false);
            if let Some(order) = order.clone() {
                link.submit(CommandKind::Order(order));
                for _ in 0..30_000 {
                    for e in link.tick().unwrap() {
                        rejected |= matches!(e, SimEvent::Rejected { .. });
                        delivered |= matches!(e, SimEvent::Delivered { player, .. } if player == PlayerId(0));
                    }
                    if delivered {
                        break;
                    }
                }
            }
            let alive = link.curr().unit(*miner).is_some();
            println!("MINE {miner_odf} {miner:?} -> {moon_odf} {moon:?}: order {} mixed {} rejected {rejected} delivered {delivered} alive {alive}", order.is_some(), mixed.map_or("move".to_owned(), |o| format!("{o:?}").chars().take(40).collect()));
        }
    }
}
