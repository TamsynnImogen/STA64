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
        energy: None,
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
