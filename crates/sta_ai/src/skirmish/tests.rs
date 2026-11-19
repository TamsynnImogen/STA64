//! The skirmish AI against `IdleController` on the fake `two_bases` map
//! (AI-02's done-when), and the properties the plan asks of it.
//!
//! The harness plays the part of the game mode: it seats the AI as player 0
//! and an idle opponent as player 1, and gives both what a skirmish start
//! gives (a station, a yard and 1000 dilithium) with bench edits, since
//! `Sim::from_setup` spawns nothing at start positions yet.

use std::collections::BTreeSet;

use sta_content::{Aip, BuildItem, ContentDb, DefRef, TechTree};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Tick, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

use super::*;
use crate::Registry;

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const LIMIT_MINUTES: u64 = 30;

fn rebuilt(tree: Option<&str>) -> ContentDb {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    if let Some(t) = tree {
        let (t, w) = TechTree::parse("t.tt", t);
        assert!(w.is_empty(), "{w:?}");
        b = b.tech_tree(t);
    }
    b.build().unwrap()
}

/// `rebuilt`, for the special-forces tests.
pub(super) fn rebuilt_for_flags(tree: Option<&str>) -> ContentDb {
    rebuilt(tree)
}

fn setup(seed: u64, content: ContentDb) -> GameSetup {
    let seat = |id: u8, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
    GameSetup {
        seed,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(content),
        players: vec![seat(0, Control::Ai(SKIRMISH.into())), seat(1, Control::Ai(crate::IDLE.into()))],
        options: GameOptions::default(),
    }
}

/// A station, a yard and 1000 dilithium for each seat.
fn start(sim: &mut Sim, setup: &GameSetup) {
    let key = |odf: &str| setup.content.key(odf).unwrap();
    let mut seq = 0;
    let mut cmd = |e: Edit| {
        seq += 1;
        Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(e) }
    };
    let mut batch = Vec::new();
    for (i, s) in setup.map.starts.iter().enumerate() {
        let p = PlayerId(i as u8);
        for (odf, dz) in [("fx_station", 0), ("fx_yard", 300)] {
            let pos = Vec3 { x: s.pos.x, y: s.pos.y, z: s.pos.z + Fixed::from_int(dz) };
            batch.push(cmd(Edit::Spawn { odf: key(odf), player: p, pos, heading: s.heading }));
        }
        batch.push(cmd(Edit::Cargo(p, 0, Fixed::from_int(1000))));
    }
    sim.step(&batch);
}

struct Run {
    /// Tick the game was over at, if it was.
    won_at: Option<Tick>,
    winners: Option<Vec<PlayerId>>,
    digest: u64,
    rejected: usize,
    peak_miners: usize,
    sim: Sim,
}

/// Play until the game is over or `minutes` pass. `aip` tunes player 0.
/// Fails if the AI ever orders an attack or mining on something it cannot see.
fn play(setup: &GameSetup, aip: Option<&Aip>, minutes: u64) -> Run {
    let reg = Registry::builtin();
    let mut sim = Sim::from_setup(setup);
    start(&mut sim, setup);
    let mut ctl: Vec<_> = setup
        .players
        .iter()
        .map(|p| match (&p.control, aip, p.id) {
            (Control::Ai(n), Some(aip), P0) if n == SKIRMISH => {
                let cfg = SkirmishConfig::from_setup(setup, p.id).unwrap().with_aip(aip);
                Box::new(SkirmishController::new(cfg)) as Box<dyn sta_sim::setup::Controller>
            }
            (Control::Ai(n), ..) => reg.create_for_seat(n, setup, p.id).unwrap(),
            _ => unreachable!(),
        })
        .collect();
    let miner = setup.content.key("fx_miner").unwrap();
    let (mut events, mut rejected, mut peak_miners) = (Vec::<SimEvent>::new(), 0, 0);
    let mut won_at = None;
    // A node once seen is remembered, as a player remembers where the moon is.
    let mut ever_seen: BTreeSet<EntityId> = BTreeSet::new();
    for _ in 0..minutes * 60 * u64::from(TICK_HZ) {
        let view = sim.view();
        let mut out = Vec::new();
        for c in &mut ctl {
            c.think(&view, &events, &mut out);
        }
        ever_seen.extend(view.visible_to(P0).iter().map(|u| u.id));
        for c in &out {
            if let CommandKind::Order(Order::Attack { target, .. } | Order::Mine { node: target, .. }) = &c.kind {
                assert!(ever_seen.contains(target), "tick {}: ordered on {target:?}, which seat 0 has never seen", view.tick);
            }
        }
        let o = sim.step(&out);
        rejected += o.events.iter().filter(|e| matches!(e, SimEvent::Rejected { player, .. } if *player == P0)).count();
        events = o.events;
        peak_miners = peak_miners.max(o.view.units.iter().filter(|u| u.owner == P0 && u.def == miner).count());
        if sim.winners().is_some() {
            won_at = Some(o.view.tick);
            break;
        }
    }
    Run { won_at, winners: sim.winners(), digest: sim.digest(), rejected, peak_miners, sim }
}

#[test]
fn beats_idle_on_two_bases_within_thirty_minutes() {
    let run = play(&setup(1, sta_content::fake::db()), None, LIMIT_MINUTES);
    let t = run.won_at.expect("the AI did not finish the idle player in 30 minutes");
    eprintln!("AI-02: skirmish beats idle on two_bases at {:.2} sim minutes (tick {t})", t as f64 / f64::from(TICK_HZ) / 60.0);
    assert_eq!(run.winners, Some(vec![P0]));
    assert!(t <= LIMIT_MINUTES * 60 * u64::from(TICK_HZ));
    assert_eq!(run.rejected, 0, "every order it sent was accepted");
    assert!(run.sim.view().player(P1).unwrap().eliminated);
}

#[test]
fn the_win_is_deterministic() {
    let a = play(&setup(7, sta_content::fake::db()), None, LIMIT_MINUTES);
    let b = play(&setup(7, sta_content::fake::db()), None, LIMIT_MINUTES);
    assert!(a.won_at.is_some());
    assert_eq!((a.won_at, a.digest), (b.won_at, b.digest));
}

#[test]
fn other_seeds_win_too() {
    for seed in [2, 3] {
        let run = play(&setup(seed, sta_content::fake::db()), None, LIMIT_MINUTES);
        assert_eq!(run.winners, Some(vec![P0]), "seed {seed}");
        eprintln!("AI-02: seed {seed} won at tick {:?}", run.won_at);
    }
}

#[test]
fn the_tech_tree_gates_what_it_builds() {
    // The tree does not list the cruiser, so the sim would refuse it.
    let db = rebuilt(Some("fx_frigate 0\nfx_miner 0\nfx_yard 0\nfx_station 0\n"));
    let cruiser = db.key("fx_cruiser").unwrap();
    let frigate = db.key("fx_frigate").unwrap();
    let run = play(&setup(1, db), None, 12);
    let view = run.sim.view();
    let mine = |k| view.units.iter().filter(|u| u.owner == P0 && u.def == k).count();
    assert_eq!(mine(cruiser), 0);
    assert!(mine(frigate) >= 4, "it still builds what it may: {}", mine(frigate));
    assert_eq!(run.rejected, 0);
}

#[test]
fn an_aip_sets_the_miner_count() {
    let (aip, w) = Aip::parse("t.aip", "int min_resource_force = 2;\nint min_attack_force = 3;\n");
    assert!(w.is_empty());
    let run = play(&setup(1, sta_content::fake::db()), Some(&aip), 6);
    assert_eq!(run.peak_miners, 2);
    let default = play(&setup(1, sta_content::fake::db()), None, 6);
    assert_eq!(default.peak_miners, params::DEFAULT_MINERS);
}

#[test]
fn registry_builds_it_for_a_seat_and_lists_it() {
    let reg = Registry::builtin();
    assert!(reg.names().contains(&SKIRMISH));
    let s = setup(1, sta_content::fake::db());
    assert!(reg.create_for_seat(SKIRMISH, &s, P0).is_ok());
    // A seat not in the setup gets a controller that does nothing.
    let mut stray = reg.create_for_seat(SKIRMISH, &s, PlayerId(5)).unwrap();
    let mut out = Vec::new();
    stray.think(&Sim::from_setup(&s).view(), &[], &mut out);
    assert!(out.is_empty());
    assert!(reg.create_for_seat("nope", &s, P0).is_err());
}

#[test]
fn config_reads_the_map() {
    let s = setup(1, sta_content::fake::db());
    let c = SkirmishConfig::from_setup(&s, P0).unwrap();
    assert_eq!(c.home.x, Fixed::from_int(-4000));
    assert_eq!(c.enemy_starts, [Vec3 { x: Fixed::from_int(4000), y: Fixed::ZERO, z: Fixed::ZERO }]);
    assert_eq!(c.nodes.len(), 1, "the moon");
    assert_eq!(c.race, "fx");
}

#[test]
fn welding_rigs_are_skipped_until_the_building_flag_clears() {
    let fake = sta_content::fake::db();
    let mut rig = sta_content::UnitDef::new("fx_rig", "craft");
    rig.race = "fx".into();
    rig.class.0 = "constructionrig".into();
    rig.build.list = vec![DefRef::new("fx_station")];
    let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    let db = Arc::new(b.unit(rig).build().unwrap());
    let rig_key = db.key("fx_rig").unwrap();
    let station_key = db.key("fx_station").unwrap();
    let mut cfg = SkirmishConfig {
        player: P0,
        race: "fx".into(),
        content: db,
        home: Vec3::ZERO,
        enemy_starts: Vec::new(),
        nodes: Vec::new(),
        seed: 1,
        params: Params::default(),
    };
    cfg.params.build_list = vec![BuildItem {
        unit: "fx_station".into(),
        count: Fixed::from_int(2),
        base: None,
        max: None,
        tech_level: None,
    }];
    let me = PlayerView { crew: None,
        id: P0,
        team: TeamId(0),
        resources: vec![Fixed::from_int(10_000); 4],
        officers: [Fixed::ZERO; 2],
        eliminated: false,
        relations: Vec::new(),
    };
    let rig_view = |id, building| sta_sim::view::UnitView {
        id: EntityId(id),
        def: rig_key,
        owner: P0,
        pos: Vec3::ZERO,
        heading: (Fixed::ONE, Fixed::ZERO),
        speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2],
        shields: [[Fixed::ZERO; 2]; 4],
        crew: None,
        energy: None, autonomy: None,
        flags: sta_sim::view::UnitFlags::SELECTABLE.with(sta_sim::view::UnitFlags::BUILDING, building),
        progress: None,
    };
    let run = |units| {
        let view = WorldView { units, ..WorldView::default() };
        let mut ai = SkirmishController::new(cfg.clone());
        let mut out = Vec::new();
        ai.plan(&view, &me, &mut out);
        out
    };
    let builds = |out: &[Command]| {
        out.iter().filter_map(|c| match &c.kind {
            CommandKind::Order(Order::Build { builder, what, .. }) => Some((*builder, *what)),
            _ => None,
        }).collect::<Vec<_>>()
    };

    // Its own units count even when the view shows them to no one (`hidden = 1`
    // helpers that are tech-tree prerequisites): it plans from `units`.
    let unseen = WorldView { units: vec![rig_view(1, false)], visibility: vec![(P0, Vec::new())], ..WorldView::default() };
    let mut ai = SkirmishController::new(cfg.clone());
    let mut out = Vec::new();
    ai.plan(&unseen, &me, &mut out);
    assert_eq!(builds(&out), [(EntityId(1), station_key)], "an own unit outside the visible set still counts");

    assert!(builds(&run(vec![rig_view(1, true)])).is_empty(), "a welding rig must not receive another build order");
    assert_eq!(builds(&run(vec![rig_view(1, true), rig_view(2, false)])), [(EntityId(2), station_key)]);
    assert_eq!(builds(&run(vec![rig_view(1, false)])), [(EntityId(1), station_key)], "the rig is available again once BUILDING clears");
}

#[test]
fn empty_aip_builds_only_an_available_miner_station_prerequisite_once() {
    let fake = sta_content::fake::db();
    let mut base = fake.unit(fake.key("fx_station").unwrap()).unwrap().clone();
    base.build.list = vec![DefRef::new("fx_miner"), DefRef::new("fx_frigate")];
    base.build.ticks = 30;
    let mut rig = sta_content::UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = sta_content::MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    rig.build.list = vec![DefRef::new("fx_decoy"), DefRef::new("fx_refinery")];
    let mut refinery = sta_content::UnitDef::new("fx_refinery", "mining");
    refinery.race = "fx".into();
    refinery.hull = Fixed::from_int(100);
    refinery.build.cost = vec![Fixed::from_int(200)];
    refinery.build.ticks = 30;
    let mut decoy = sta_content::UnitDef::new("fx_decoy", "mining");
    decoy.race = "fx".into();
    decoy.build.cost = vec![Fixed::from_int(1)];
    let (tree, warnings) = TechTree::parse(
        "miner.tt",
        "fx_station 0\nfx_rig 0\nfx_miner 1 fx_refinery\nfx_frigate 0\nfx_refinery 0\nfx_decoy 0\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut builder = ContentDb::builder().profile(fake.profile()).features(fake.features().clone()).tech_tree(tree);
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    for unit in fake.units() {
        let mut unit = if unit.odf == "fx_station" { base.clone() } else { unit.clone() };
        if unit.odf == "fx_miner" {
            unit.build.ticks = 30;
            unit.build.cost = vec![Fixed::from_int(100)];
        }
        if unit.odf == "fx_frigate" {
            unit.build.cost = vec![Fixed::from_int(150)];
        }
        builder = builder.unit(unit);
    }
    let db = Arc::new(builder.unit(rig).unit(refinery).unit(decoy).build().unwrap());
    let key = |name| db.key(name).unwrap();
    let mut cfg = SkirmishConfig {
        player: P0,
        race: "fx".into(),
        content: Arc::clone(&db),
        home: Vec3::ZERO,
        enemy_starts: Vec::new(),
        nodes: Vec::new(),
        seed: 1,
        params: Params::default(),
    };
    let me = PlayerView {
        crew: None,
        id: P0,
        team: TeamId(0),
        resources: vec![Fixed::from_int(1000); 4],
        officers: [Fixed::ZERO; 2],
        eliminated: false,
        relations: Vec::new(),
    };
    let unit = |id, def| sta_sim::view::UnitView {
        id: EntityId(id),
        def,
        owner: P0,
        pos: Vec3::ZERO,
        heading: (Fixed::ONE, Fixed::ZERO),
        speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2],
        shields: [[Fixed::ZERO; 2]; 4],
        crew: None,
        energy: None,
        autonomy: None,
        flags: sta_sim::view::UnitFlags::SELECTABLE,
        progress: None,
    };
    let view = WorldView { units: vec![unit(1, key("fx_station")), unit(2, key("fx_rig")), unit(3, key("fx_rig"))], ..WorldView::default() };
    let builds = |out: &[Command]| out.iter().filter_map(|c| match c.kind {
        CommandKind::Order(Order::Build { builder, what, .. }) => Some((builder, what)),
        _ => None,
    }).collect::<Vec<_>>();

    let mut ai = SkirmishController::new(cfg.clone());
    let mut out = Vec::new();
    ai.plan(&view, &me, &mut out);
    assert_eq!(builds(&out), [(EntityId(2), key("fx_refinery"))], "the decoy is cheaper but does not unlock mining");
    let scarce = PlayerView { resources: vec![Fixed::from_int(200); 4], ..me.clone() };
    let mut scarce_ai = SkirmishController::new(cfg.clone());
    out.clear();
    scarce_ai.plan(&view, &scarce, &mut out);
    assert_eq!(builds(&out), [(EntityId(2), key("fx_refinery"))], "the yard must save station funds instead of buying a warship");
    out.clear();
    ai.plan(&view, &me, &mut out);
    assert!(builds(&out).is_empty(), "the in-flight rig job prevents duplicate construction and preserves miner funds");

    cfg.params.build_list = vec![BuildItem { unit: "fx_decoy".into(), count: Fixed::ONE, base: None, max: None, tech_level: None }];
    let mut ai = SkirmishController::new(cfg.clone());
    out.clear();
    ai.plan(&view, &me, &mut out);
    assert_eq!(builds(&out), [(EntityId(1), key("fx_frigate")), (EntityId(2), key("fx_decoy"))], "an explicit AIP row retains its policy");

    let (locked_tree, warnings) = TechTree::parse(
        "multi-level.tt",
        "fx_station 0\nfx_rig 0\nfx_miner 1 fx_refinery\nfx_frigate 0\nfx_refinery 1 fx_gate\nfx_decoy 0\nfx_gate 0\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut gate = sta_content::UnitDef::new("fx_gate", "mining");
    gate.race = "fx".into();
    let mut locked_builder = ContentDb::builder()
        .profile(cfg.content.profile())
        .features(cfg.content.features().clone())
        .tech_tree(locked_tree)
        .unit(gate);
    for weapon in cfg.content.weapons() {
        locked_builder = locked_builder.weapon(weapon.clone());
    }
    for unit in cfg.content.units() {
        locked_builder = locked_builder.unit(unit.clone());
    }
    let mut locked_cfg = cfg.clone();
    locked_cfg.params.build_list.clear();
    locked_cfg.content = Arc::new(locked_builder.build().unwrap());
    let locked_view = WorldView {
        units: vec![
            unit(1, locked_cfg.content.key("fx_station").unwrap()),
            unit(2, locked_cfg.content.key("fx_rig").unwrap()),
        ],
        ..WorldView::default()
    };
    let mut locked_ai = SkirmishController::new(locked_cfg);
    out.clear();
    locked_ai.plan(&locked_view, &me, &mut out);
    assert_eq!(builds(&out), [(EntityId(1), locked_ai.cfg.content.key("fx_frigate").unwrap())], "a locked prerequisite must not freeze normal yard production");

    let (alt_tree, warnings) = TechTree::parse(
        "alternate-miner.tt",
        "fx_station 0\nfx_rig 0\nfx_miner 1 fx_refinery\nfx_alt_miner 0\nfx_frigate 0\nfx_refinery 0\nfx_decoy 0\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut alt_base = base.clone();
    alt_base.build.list.insert(0, DefRef::new("fx_alt_miner"));
    let mut alt_miner = sta_content::UnitDef::new("fx_alt_miner", "freighter");
    alt_miner.race = "fx".into();
    alt_miner.build.cost = vec![Fixed::from_int(100)];
    let mut alt_builder = ContentDb::builder()
        .profile(cfg.content.profile())
        .features(cfg.content.features().clone())
        .tech_tree(alt_tree)
        .unit(alt_miner);
    for weapon in cfg.content.weapons() {
        alt_builder = alt_builder.weapon(weapon.clone());
    }
    for existing in cfg.content.units() {
        alt_builder = alt_builder.unit(if existing.odf == "fx_station" { alt_base.clone() } else { existing.clone() });
    }
    let mut alt_cfg = cfg.clone();
    alt_cfg.params.build_list.clear();
    alt_cfg.content = Arc::new(alt_builder.build().unwrap());
    let alt_view = WorldView {
        units: vec![
            unit(1, alt_cfg.content.key("fx_station").unwrap()),
            unit(2, alt_cfg.content.key("fx_rig").unwrap()),
        ],
        ..WorldView::default()
    };
    let alt_miner_key = alt_cfg.content.key("fx_alt_miner").unwrap();
    let mut alt_ai = SkirmishController::new(alt_cfg);
    out.clear();
    alt_ai.plan(&alt_view, &scarce, &mut out);
    assert_eq!(builds(&out), [(EntityId(1), alt_miner_key)], "an available miner takes precedence over a locked alternate's station");

    // Exercise admission and completion as well as planning. The starting
    // constructor and base mirror the Classic dependency chain, but use
    // authored fake definitions so this remains install-independent.
    let game = GameSetup { content: Arc::clone(&cfg.content), ..setup(1, sta_content::fake::db()) };
    let mut sim = Sim::from_setup(&game);
    let at = game.map.starts[0].pos;
    let spawn = |odf, player, pos| Command {
        player: PlayerId::BENCH,
        seq: 0,
        kind: CommandKind::Edit(Edit::Spawn { odf: key(odf), player, pos, heading: (Fixed::ONE, Fixed::ZERO) }),
    };
    sim.step(&[
        spawn("fx_station", P0, at),
        spawn("fx_rig", P0, Vec3 { x: at.x + Fixed::from_int(400), ..at }),
        spawn("fx_station", P1, game.map.starts[1].pos),
        // After paying 200 for the station, 150 is enough for a warship but
        // not for that warship and the 100-cost miner.
        Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(Edit::Cargo(P0, 0, Fixed::from_int(350))) },
    ]);
    let mut ai = SkirmishController::new(SkirmishConfig::from_setup(&game, P0).unwrap());
    let mut events = Vec::new();
    let (mut built_refinery, mut built_miner, mut mining, mut rejected, mut early_warship) = (false, false, false, 0, false);
    for _ in 0..3000 {
        let mut orders = Vec::new();
        ai.think(&sim.view(), &events, &mut orders);
        let output = sim.step(&orders);
        rejected += output.events.iter().filter(|e| matches!(e, SimEvent::Rejected { player, .. } if *player == P0)).count();
        for e in &output.events {
            if let SimEvent::Built { id, .. } = e {
                early_warship |= !built_miner && output.view.units.iter().any(|u| u.id == *id && u.def == key("fx_frigate"));
                built_refinery |= output.view.units.iter().any(|u| u.id == *id && u.def == key("fx_refinery"));
                built_miner |= output.view.units.iter().any(|u| u.id == *id && u.def == key("fx_miner"));
            }
            mining |= matches!(e, SimEvent::MinerPhase { phase: sta_sim::MinerPhase::Mining, .. });
        }
        events = output.events;
        if built_refinery && built_miner && mining { break; }
    }
    assert!(built_refinery && built_miner && mining, "refinery={built_refinery}, miner={built_miner}, mining={mining}");
    assert!(!early_warship, "a warship spent the reserved miner budget before mining started");
    assert_eq!(rejected, 0);
}

/// A rig can supply a shipyard once the station's miner economy is running.
#[test]
fn empty_aip_builds_one_armed_ship_producer_after_mining() {
    let fake = sta_content::fake::db();
    let mut rig = sta_content::UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.hull = Fixed::from_int(100);
    rig.movement = sta_content::MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.build.list = vec![DefRef::new("fx_decoy"), DefRef::new("fx_yard")];
    let mut decoy = sta_content::UnitDef::new("fx_decoy", "mining");
    decoy.race = "fx".into();
    decoy.build.cost = vec![Fixed::ONE];
    let (tree, warnings) = TechTree::parse("armed.tt", "fx_station 0\nfx_rig 0\nfx_miner 0\nfx_yard 0\nfx_frigate 1 fx_yard\nfx_cruiser 1 fx_yard\nfx_decoy 0\n");
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut builder = ContentDb::builder().profile(fake.profile()).features(fake.features().clone()).tech_tree(tree);
    for weapon in fake.weapons() { builder = builder.weapon(weapon.clone()); }
    for unit in fake.units() { builder = builder.unit(unit.clone()); }
    let db = Arc::new(builder.unit(rig).unit(decoy).build().unwrap());
    let key = |name| db.key(name).unwrap();
    let mut cfg = SkirmishConfig {
        player: P0, race: "fx".into(), content: Arc::clone(&db), home: Vec3::ZERO,
        enemy_starts: Vec::new(), nodes: Vec::new(), seed: 1, params: Params::default(),
    };
    cfg.params.miners = 1;
    let me = PlayerView {
        crew: None, id: P0, team: TeamId(0), resources: vec![Fixed::from_int(2000); 4],
        officers: [Fixed::ZERO; 2], eliminated: false, relations: Vec::new(),
    };
    let unit = |id, def, progress| sta_sim::view::UnitView {
        id: EntityId(id), def, owner: P0, pos: Vec3::ZERO,
        heading: (Fixed::ONE, Fixed::ZERO), speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4],
        crew: None, energy: None, autonomy: None,
        flags: sta_sim::view::UnitFlags::SELECTABLE, progress,
    };
    let base = vec![unit(1, key("fx_station"), None), unit(2, key("fx_rig"), None), unit(3, key("fx_rig"), None)];
    let orders = |view: &WorldView, cfg: SkirmishConfig| {
        let mut ai = SkirmishController::new(cfg);
        let mut out = Vec::new();
        ai.plan(view, &me, &mut out);
        (ai, out)
    };
    let builds = |out: &[Command]| out.iter().filter_map(|c| match c.kind {
        CommandKind::Order(Order::Build { builder, what, .. }) => Some((builder, what)),
        _ => None,
    }).collect::<Vec<_>>();
    let (_, out) = orders(&WorldView { units: base.clone(), ..WorldView::default() }, cfg.clone());
    assert_eq!(builds(&out), [(EntityId(1), key("fx_miner"))], "miner is first; no station yet");
    let mut mined = base.clone();
    mined.push(unit(4, key("fx_miner"), None));
    let (mut ai, out) = orders(&WorldView { units: mined.clone(), ..WorldView::default() }, cfg.clone());
    assert_eq!(builds(&out), [(EntityId(2), key("fx_yard"))], "one producer, not the cheaper decoy");
    let mut two_miners = cfg.clone();
    two_miners.params.miners = 2;
    let (_, out) = orders(&WorldView { units: mined.clone(), ..WorldView::default() }, two_miners);
    assert_eq!(builds(&out), [(EntityId(1), key("fx_miner")), (EntityId(2), key("fx_yard"))], "a queued miner meets the goal while an existing miner is already finished");
    let mut retry = Vec::new();
    ai.plan(&WorldView { units: mined.clone(), ..WorldView::default() }, &me, &mut retry);
    assert!(builds(&retry).is_empty(), "pending rig job prevents another producer");
    mined.push(unit(5, key("fx_yard"), Some(Fixed::from_f64(0.5))));
    let (_, out) = orders(&WorldView { units: mined.clone(), ..WorldView::default() }, cfg.clone());
    assert!(builds(&out).is_empty(), "unfinished yard counts as an incoming producer");
    mined.last_mut().unwrap().progress = None;
    let (_, out) = orders(&WorldView { units: mined.clone(), ..WorldView::default() }, cfg.clone());
    assert!(builds(&out).iter().any(|&(builder, what)| builder == EntityId(5) && armed_craft(&db, what)), "finished yard builds an armed craft");

    cfg.params.build_list = vec![BuildItem { unit: "fx_decoy".into(), count: Fixed::ONE, base: None, max: None, tech_level: None }];
    let (_, out) = orders(&WorldView { units: base, ..WorldView::default() }, cfg.clone());
    assert!(!builds(&out).iter().any(|&(_, what)| what == key("fx_yard")), "explicit AIP does not trigger fallback");

    // A direct candidate must itself be available, and adding it must be
    // sufficient to make a real armed craft available.
    for (case, text) in [
        ("locked yard", "fx_station 0\nfx_rig 0\nfx_miner 0\nfx_yard 1 fx_gate\nfx_frigate 1 fx_yard\nfx_cruiser 1 fx_yard\nfx_decoy 0\n"),
        ("locked armament", "fx_station 0\nfx_rig 0\nfx_miner 0\nfx_yard 0\nfx_frigate 1 fx_gate\nfx_cruiser 1 fx_gate\nfx_decoy 0\n"),
    ] {
        let (tree, warnings) = TechTree::parse("locked.tt", text);
        assert!(warnings.is_empty(), "{case}: {warnings:?}");
        let mut b = ContentDb::builder().profile(db.profile()).features(db.features().clone()).tech_tree(tree);
        for weapon in db.weapons() { b = b.weapon(weapon.clone()); }
        for unit in db.units() { b = b.unit(unit.clone()); }
        let mut locked_cfg = cfg.clone();
        locked_cfg.params.build_list.clear();
        locked_cfg.content = Arc::new(b.build().unwrap());
        let (_, out) = orders(&WorldView { units: mined[..4].to_vec(), ..WorldView::default() }, locked_cfg);
        assert!(builds(&out).is_empty(), "{case}: no station order or reserved budget without an available direct path");
    }
    let mut unarmed = ContentDb::builder().profile(db.profile()).features(db.features().clone());
    for weapon in db.weapons() { unarmed = unarmed.weapon(weapon.clone()); }
    let point_defence = db.unit(key("fx_station")).unwrap().hardpoints[1].clone();
    for mut unit in db.units().cloned() {
        if unit.odf == "fx_frigate" || unit.odf == "fx_cruiser" {
            unit.hardpoints = vec![point_defence.clone()];
        }
        unarmed = unarmed.unit(unit);
    }
    let (tree, warnings) = TechTree::parse("unarmed.tt", "fx_station 0\nfx_rig 0\nfx_miner 0\nfx_yard 0\nfx_frigate 1 fx_yard\nfx_cruiser 1 fx_yard\nfx_decoy 0\n");
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut unarmed_cfg = cfg.clone();
    unarmed_cfg.params.build_list.clear();
    unarmed_cfg.content = Arc::new(unarmed.tech_tree(tree).build().unwrap());
    let (_, out) = orders(&WorldView { units: mined[..4].to_vec(), ..WorldView::default() }, unarmed_cfg);
    assert!(builds(&out).is_empty(), "point defence alone cannot justify a yard");
    for case in ["minebm", "tractor_bm", "cloakingdevice", "resourceweapon"] {
        let mut b = ContentDb::builder().profile(db.profile()).features(db.features().clone());
        for mut weapon in db.weapons().cloned() {
            if weapon.odf == "fx_beam" {
                weapon.class.0 = case.into();
                if case == "resourceweapon" {
                    weapon.resource = Some(sta_content::ResourceWeaponDef {
                        add: [Fixed::ZERO; 4], crew: Fixed::ZERO, one_time: false,
                        always_player: false, always_ai: false,
                    });
                }
            }
            b = b.weapon(weapon);
        }
        let beam_mount = db.unit(key("fx_frigate")).unwrap().hardpoints[0].clone();
        for mut unit in db.units().cloned() {
            if unit.odf == "fx_frigate" || unit.odf == "fx_cruiser" {
                unit.hardpoints = vec![beam_mount.clone()];
            }
            b = b.unit(unit);
        }
        let (tree, warnings) = TechTree::parse("tool.tt", "fx_station 0\nfx_rig 0\nfx_miner 0\nfx_yard 0\nfx_frigate 1 fx_yard\nfx_cruiser 1 fx_yard\nfx_decoy 0\n");
        assert!(warnings.is_empty(), "{case}: {warnings:?}");
        let mut tool_cfg = cfg.clone();
        tool_cfg.params.build_list.clear();
        tool_cfg.content = Arc::new(b.tech_tree(tree).build().unwrap());
        let (_, out) = orders(&WorldView { units: mined[..4].to_vec(), ..WorldView::default() }, tool_cfg);
        assert!(builds(&out).is_empty(), "{case}: positive-damage, positive-range noncombat mount cannot justify a yard");
    }

    let game = GameSetup { content: Arc::clone(&db), ..setup(1, sta_content::fake::db()) };
    let run = || {
        let mut sim = Sim::from_setup(&game);
        let at = game.map.starts[0].pos;
        let spawn = |odf, player, pos| Command {
            player: PlayerId::BENCH, seq: 0,
            kind: CommandKind::Edit(Edit::Spawn { odf: key(odf), player, pos, heading: (Fixed::ONE, Fixed::ZERO) }),
        };
        sim.step(&[
            spawn("fx_station", P0, at),
            spawn("fx_rig", P0, Vec3 { x: at.x + Fixed::from_int(400), ..at }),
            spawn("fx_miner", P0, Vec3 { x: at.x + Fixed::from_int(300), ..at }),
            spawn("fx_station", P1, game.map.starts[1].pos),
            Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(Edit::Cargo(P0, 0, Fixed::from_int(5000))) },
        ]);
        let mut ai = SkirmishController::new(SkirmishConfig::from_setup(&game, P0).unwrap());
        ai.cfg.params.miners = 1;
        ai.cfg.params.min_attack = 1;
        ai.cfg.params.max_attack = 1;
        ai.cfg.params.defence = 0;
        let mut events = Vec::new();
        let mut commands = Vec::new();
        let (mut yard, mut armed, mut attack, mut rejected) = (false, false, false, 0);
        for _ in 0..8000 {
            let mut orders = Vec::new();
            ai.think(&sim.view(), &events, &mut orders);
            attack |= orders.iter().any(|c| matches!(c.kind, CommandKind::Order(Order::Attack { .. })));
            if !orders.is_empty() { commands.push((sim.view().tick, orders.clone())); }
            let output = sim.step(&orders);
            rejected += output.events.iter().filter(|e| matches!(e, SimEvent::Rejected { player, .. } if *player == P0)).count();
            yard |= output.view.units.iter().any(|u| u.owner == P0 && u.def == key("fx_yard") && finished(u));
            armed |= output.view.units.iter().any(|u| u.owner == P0 && armed_craft(&db, u.def) && finished(u));
            events = output.events;
            if yard && armed && attack { break; }
        }
        (yard, armed, attack, rejected, sim.digest(), commands)
    };
    let first = run();
    assert!(first.0 && first.1 && first.2, "yard={}, armed={}, attack={}", first.0, first.1, first.2);
    assert_eq!(first.3, 0, "all orders accepted");
    assert_eq!(first, run(), "the same seed yields the same result and digest");
}

#[test]
fn fo4_aip_counts_promoted_craft_by_distance_below_grid() {
    let make_db = |profile, variant_tag| {
        let fake = sta_content::fake::db();
        let mut builder = ContentDb::builder().profile(profile).features(fake.features().clone());
        for weapon in fake.weapons() {
            builder = builder.weapon(weapon.clone());
        }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_cruiser" {
                unit.stats.distance_below_grid = Some(Fixed::from_int(50));
            }
            if unit.odf == "fx_yard" {
                // The promoted craft exists in the world but cannot be queued
                // directly; the yard can still build the base class.
                let mut veteran = fake.unit(fake.key("fx_cruiser").unwrap()).unwrap().clone();
                veteran.odf = "fx_cruiser_veteran".into();
                veteran.stats.distance_below_grid = Some(variant_tag);
                veteran.build.list.clear();
                builder = builder.unit(veteran);
            }
            builder = builder.unit(unit);
        }
        Arc::new(builder.build().unwrap())
    };
    let run = |profile, variant_tag, count| {
        let db = make_db(profile, variant_tag);
        let key = |name| db.key(name).unwrap();
        let mut cfg = SkirmishConfig {
            player: P0,
            race: "fx".into(),
            content: Arc::clone(&db),
            home: Vec3::ZERO,
            enemy_starts: Vec::new(),
            nodes: Vec::new(),
            seed: 1,
            params: Params::default(),
        };
        cfg.params.miners = 0;
        cfg.params.build_list = vec![BuildItem { unit: "fx_cruiser".into(), count, base: None, max: None, tech_level: None }];
        let me = PlayerView {
            crew: None,
            id: P0,
            team: TeamId(0),
            resources: vec![Fixed::from_int(10_000); 4],
            officers: [Fixed::ZERO; 2],
            eliminated: false,
            relations: Vec::new(),
        };
        let unit = |id, odf| sta_sim::view::UnitView {
            id: EntityId(id),
            def: key(odf),
            owner: P0,
            pos: Vec3::ZERO,
            heading: (Fixed::ONE, Fixed::ZERO),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4],
            crew: None,
            energy: None,
            autonomy: None,
            flags: sta_sim::view::UnitFlags::SELECTABLE,
            progress: None,
        };
        let view = WorldView {
            units: vec![unit(1, "fx_station"), unit(2, "fx_yard"), unit(3, "fx_cruiser_veteran")],
            ..WorldView::default()
        };
        let mut ai = SkirmishController::new(cfg);
        let mut out = Vec::new();
        ai.plan(&view, &me, &mut out);
        out.into_iter().filter_map(|c| match c.kind {
            CommandKind::Order(Order::Build { what, .. }) => Some(what),
            _ => None,
        }).collect::<Vec<_>>()
    };

    let fo4_base = make_db(sta_content::Profile::Fo4, Fixed::from_int(50));
    let fo4_cruiser = fo4_base.key("fx_cruiser").unwrap();
    assert!(
        !run(sta_content::Profile::Fo4, Fixed::from_int(50), Fixed::ONE).contains(&fo4_cruiser),
        "a matched promoted craft satisfies the explicit base-class row and blocks generic fallback"
    );
    assert!(
        run(sta_content::Profile::Fo4, Fixed::from_int(50), Fixed::from_int(2)).contains(&fo4_cruiser),
        "a higher requested count still wants another base-class craft"
    );
    assert!(
        run(sta_content::Profile::Fo4, Fixed::from_int(51), Fixed::ONE).contains(&fo4_cruiser),
        "a nonmatching promotion tag does not satisfy the base-class row"
    );
    assert!(
        run(sta_content::Profile::Native, Fixed::from_int(50), Fixed::ONE).contains(&fo4_cruiser),
        "non-Fo4 profiles match the explicit ODF key exactly"
    );
}

/// Both seats plan on the same ticks. A stagger by seat number once gave seat
/// 1 every plan one tick before seat 0, which decided mirror games for it.
#[test]
fn every_seat_plans_on_the_same_ticks() {
    let s = GameSetup {
        players: (0..2)
            .map(|i| PlayerSetup { id: PlayerId(i), team: TeamId(i), race: "fx".into(), control: Control::Ai(SKIRMISH.into()) })
            .collect(),
        ..setup(1, sta_content::fake::db())
    };
    let reg = Registry::builtin();
    let mut sim = Sim::from_setup(&s);
    start(&mut sim, &s);
    let mut ctl: Vec<_> = (0..2).map(|i| reg.create_for_seat(SKIRMISH, &s, PlayerId(i)).unwrap()).collect();
    let mut planned = [0usize; 2];
    for _ in 0..90 {
        let view = sim.view();
        let mut all = Vec::new();
        for (i, c) in ctl.iter_mut().enumerate() {
            let mut out = Vec::new();
            c.think(&view, &[], &mut out);
            assert!(view.tick.is_multiple_of(PERIOD) || out.is_empty(), "seat {i} planned off the period, at tick {}", view.tick);
            planned[i] += usize::from(!out.is_empty());
            all.extend(out);
        }
        sim.step(&all);
    }
    assert!(planned[0] > 0 && planned[0] == planned[1], "{planned:?}");
}

/// The next wave's size is drawn from the seed between the AIP's minimum and
/// the growing ceiling, so seeded games differ; and a seed gives the same
/// draws every time.
#[test]
fn wave_sizes_are_drawn_from_the_seed_within_the_aip_range() {
    let sizes = |seed: u64| {
        let s = setup(seed, sta_content::fake::db());
        let mut c = SkirmishController::new(SkirmishConfig::from_setup(&s, P0).unwrap());
        let (lo, hi) = (c.cfg.params.min_attack, c.cfg.params.max_attack);
        (0..12)
            .map(|_| {
                c.raise_ceiling_and_draw();
                assert!((lo..=hi).contains(&c.next_wave), "{} outside {lo}..={hi}", c.next_wave);
                assert!(c.next_wave <= c.wave_size);
                c.next_wave
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(sizes(5), sizes(5));
    let all: BTreeSet<Vec<usize>> = (1..=8).map(sizes).collect();
    assert!(all.len() > 4, "eight seeds, {} different wave plans", all.len());
}

#[test]
fn an_order_the_sim_could_not_serve_is_sent_again() {
    let s = setup(1, sta_content::fake::db());
    let mut ai = SkirmishController::new(SkirmishConfig::from_setup(&s, P0).unwrap());
    let unit = EntityId(7);
    let to = Vec3 { x: Fixed::from_int(10), y: Fixed::ZERO, z: Fixed::from_int(20) };
    let mut out = Vec::new();
    ai.go(0, &mut out, unit, to);
    ai.go(1, &mut out, unit, to);
    assert_eq!(out.len(), 1, "the same order is not repeated while it is live");
    ai.absorb(2, &[SimEvent::OrderBlocked { id: unit }]);
    ai.go(3, &mut out, unit, to);
    assert_eq!(out.len(), 1, "a hold keeps the order: not sent again");
    ai.absorb(4, &[SimEvent::OrderAbandoned { id: unit }]);
    ai.go(5, &mut out, unit, to);
    assert_eq!(out.len(), 2, "after OrderAbandoned the AI sends it again");
}

/// AI-05: a warship whose ODF says `collisionRadius = 2` plays no fleet role; any other value,
/// or none, keeps it a warship.
#[test]
fn a_collision_radius_of_two_keeps_a_warship_out_of_fleets() {
    let fake = sta_content::fake::db();
    let with = |r: Option<i32>| {
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            let mut u = u.clone();
            if u.odf == "fx_cruiser" {
                u.stats.collision_radius = r.map(sta_sim::Fixed::from_int);
            }
            b = b.unit(u);
        }
        b.build().unwrap()
    };
    for (r, want) in [(None, Role::Warship), (Some(3), Role::Warship), (Some(2), Role::Other)] {
        let db = with(r);
        assert_eq!(super::fleet_role(&db, db.key("fx_cruiser").unwrap()), want, "collisionRadius {r:?}");
    }
    // The miner keeps its role whatever the key says.
    let db = with(Some(2));
    assert_eq!(super::fleet_role(&db, db.key("fx_miner").unwrap()), Role::Miner);
}
