//! The save file container (SIM-81, ADR 0010 V1 to V3), end to end: a match
//! run to tick n is saved through the container to a file, loaded in a fresh
//! context (content rebuilt, nothing carried over in memory) and continued;
//! it must give the same digest and events on every tick, the same final
//! snapshot bytes, the same final journal and the same controller state as
//! the uninterrupted run. The harness runs in the client's order
//! (`SimLink::tick`): take what is pending, step, log, then every controller
//! thinks about that step's events and submits for the next tick; a save is
//! taken between ticks, so the commands in flight (the controllers' of that
//! think, the player's and the sandbox's, with the link's one sequence
//! counter) are part of it, and after a load they are submitted and the
//! controllers do not think before the next step. The scenarios are built only from the setup and
//! commands (spawns by `Edit::Spawn`), so the journal alone rebuilds the
//! match and the rebuilt replay can be played back.
//!
//! Also here: save twice, load then save, the header read without the
//! sections, one test per refusal, corruption of every byte, truncation at
//! every length, random bytes, the controller seam, atomic writes and the
//! listing. Refusals that need a file with valid checksums but a forged
//! header are unit tests inside the crate (`src/save/tests.rs`).
//!
//! Authored data only (`sta_content::fake`); the map hash is a constant the
//! test plays the caller with. No game files are read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use sta_content::{ContentDb, MapDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::replay::Replay;
use sta_sim::save::{
    self, ClientState, GameKind, InFlight, MapId, MapSource, SaveError, SaveInfo, load, read_header, store,
};
use sta_sim::setup::{Control, Controller, ControllerState, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::world::NEUTRAL;
use sta_sim::{Sim, SimEvent, TickOutput, WorldSource};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const MAP_HASH: u64 = 0x5eed_0001;
const EVERY: u64 = 30;

fn content() -> Arc<ContentDb> {
    Arc::new(sta_content::fake::db())
}

fn content_with_formations() -> Arc<ContentDb> {
    Arc::new(sta_content::fake::db_with_formations())
}

fn map() -> MapDef {
    sta_content::fake::two_bases()
}

fn map_source() -> Option<MapSource> {
    Some(MapSource { hash: MAP_HASH, map: Arc::new(map()) })
}

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

// ------------------------------------------------------ a stateful controller

/// A computer player with state of its own: a counter that decides where it
/// sends its units next, so a controller restored with the wrong state or
/// none sends other commands.
#[derive(Default)]
struct Counter {
    n: u32,
    /// Shots it has been told about: its decisions depend on the events it
    /// is given, and it answers shots at once with a command, so a controller
    /// that thinks twice about one step (or never about it) ends in another
    /// state and sends other commands afterwards.
    fired: u32,
}

impl Controller for Counter {
    fn think(&mut self, view: &WorldView, events: &[SimEvent], out: &mut Vec<Command>) {
        let shots = events.iter().filter(|e| matches!(e, SimEvent::Fired { .. })).count() as u32;
        self.fired += shots;
        if shots == 0 && view.tick % 25 != 3 {
            return;
        }
        let units: Vec<EntityId> = view.units.iter().filter(|u| u.owner == P1).map(|u| u.id).collect();
        if units.is_empty() {
            return;
        }
        self.n += 1;
        let k = self.n + self.fired;
        let to = v((k * 377 % 3000) as i32 - 1500, (k * 911 % 3000) as i32 - 1500);
        out.push(Command { player: P1, seq: self.n, kind: CommandKind::Order(Order::move_to(units, to, false)) });
    }
    fn save_state(&self) -> Option<ControllerState> {
        let mut data = self.n.to_le_bytes().to_vec();
        data.extend_from_slice(&self.fired.to_le_bytes());
        Some(ControllerState { version: 1, data })
    }
    fn restore_state(&mut self, state: &ControllerState) -> Result<(), String> {
        match (state.version, <[u8; 8]>::try_from(state.data.as_slice())) {
            (1, Ok(b)) => {
                self.n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                self.fired = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                Ok(())
            }
            (v, _) => Err(format!("record version {v} or length not readable")),
        }
    }
}

/// A controller that has not opted in to saving.
struct Silent;
impl Controller for Silent {
    fn think(&mut self, _: &WorldView, _: &[SimEvent], _: &mut Vec<Command>) {}
}

fn restore_counter(_seat: PlayerId, name: &str, state: &ControllerState) -> Result<Box<dyn Controller>, String> {
    if name != "counter" {
        return Err(format!("unknown controller {name}"));
    }
    let mut c = Counter::default();
    c.restore_state(state)?;
    Ok(Box::new(c))
}

// ---------------------------------------------------------------- scenarios

/// What the local player and the sandbox submit before a tick; each gets its
/// sequence number from the link's one counter when it is submitted.
#[derive(Default)]
struct Script {
    at: BTreeMap<u64, Vec<(PlayerId, CommandKind)>>,
}

impl Script {
    fn push(&mut self, tick: u64, player: PlayerId, kind: CommandKind) {
        self.at.entry(tick).or_default().push((player, kind));
    }
    fn order(&mut self, tick: u64, player: PlayerId, o: Order) {
        self.push(tick, player, CommandKind::Order(o));
    }
    fn edit(&mut self, tick: u64, e: Edit) {
        self.push(tick, PlayerId::BENCH, CommandKind::Edit(e));
    }
    fn at(&self, tick: u64) -> &[(PlayerId, CommandKind)] {
        self.at.get(&tick).map_or(&[][..], Vec::as_slice)
    }
}

struct Scenario {
    name: &'static str,
    content: fn() -> Arc<ContentDb>,
    enemy: Control,
    script: Script,
    ticks: u64,
}

fn setup_of(sc: &Scenario) -> GameSetup {
    GameSetup {
        seed: 11,
        map: Arc::new(map()),
        content: (sc.content)(),
        players: vec![
            PlayerSetup { id: P0, team: TeamId(0), race: "fx".into(), control: Control::Human },
            PlayerSetup { id: P1, team: TeamId(1), race: "fx".into(), control: sc.enemy.clone() },
        ],
        options: GameOptions::default(),
    }
}

/// A scenario whose units come from spawn commands at tick 0 and whose orders
/// `plan` writes against the ids the spawns get.
fn make(
    name: &'static str,
    content: fn() -> Arc<ContentDb>,
    enemy: Control,
    spawns: &[(&str, PlayerId, i32, i32)],
    ticks: u64,
    plan: impl FnOnce(&[EntityId], &mut Script),
) -> Scenario {
    let mut sc = Scenario { name, content, enemy, script: Script::default(), ticks };
    let db = content();
    for &(odf, owner, x, z) in spawns {
        let spawn = Edit::Spawn { odf: db.key(odf).expect("authored unit"), player: owner, pos: v(x, z), heading: (Fixed::ZERO, Fixed::ONE) };
        sc.script.push(0, PlayerId::BENCH, CommandKind::Edit(spawn));
    }
    // Which ids the spawns get: run tick 0 on a throwaway sim.
    let mut probe = Sim::from_setup(&setup_of(&sc));
    let before: Vec<EntityId> = probe.view().units.iter().map(|u| u.id).collect();
    let spawns: Vec<Command> =
        sc.script.at(0).iter().enumerate().map(|(i, (player, kind))| Command { player: *player, seq: i as u32 + 1, kind: kind.clone() }).collect();
    probe.step(&spawns);
    let ids: Vec<EntityId> = probe.view().units.iter().map(|u| u.id).filter(|id| !before.contains(id)).collect();
    assert_eq!(ids.len(), spawns.len(), "{name}: every spawn is accepted");
    plan(&ids, &mut sc.script);
    sc
}

fn fleet_combat() -> Scenario {
    make(
        "fleet_combat",
        content,
        Control::Idle,
        &[("fx_cruiser", P0, 2000, 2000), ("fx_cruiser", P0, 2100, 2000), ("fx_frigate", P1, 2000, 2400), ("fx_frigate", P1, 2150, 2450)],
        900,
        |ids, s| {
            s.order(1, P0, Order::Attack { units: vec![ids[0]], target: ids[2] });
            s.order(1, P0, Order::Attack { units: vec![ids[1]], target: ids[3] });
        },
    )
}

fn formation_round_a_moon() -> Scenario {
    make(
        "formation",
        content_with_formations,
        Control::Idle,
        &[
            ("fx_cruiser", P0, -1500, -300),
            ("fx_cruiser", P0, -1400, -300),
            ("fx_frigate", P0, -1450, -400),
            ("fx_frigate", P0, -1350, -400),
            ("fx_moon", NEUTRAL, 300, 600),
        ],
        1200,
        |ids, s| {
            let group = ids[..4].to_vec();
            s.order(1, P0, Order::Formation { units: group.clone(), kind: 3 });
            s.order(1, P0, Order::move_to(group.clone(), v(1800, 900), false));
            s.order(2, P0, Order::move_to(group.clone(), v(-1200, 1500), true));
            s.order(500, P0, Order::Formation { units: group.clone(), kind: 2 });
            s.order(500, P0, Order::move_to(group, v(0, -1500), false));
        },
    )
}

fn with_a_computer_player() -> Scenario {
    make(
        "computer_player",
        content,
        Control::Ai("counter".into()),
        &[("fx_cruiser", P0, -2500, 0), ("fx_frigate", P0, -2400, 100), ("fx_cruiser", P1, 2500, 0), ("fx_frigate", P1, 2400, 100), ("fx_yard", P1, 3000, 300)],
        1100,
        |ids, s| {
            s.order(1, P0, Order::move_to(vec![ids[0], ids[1]], v(2000, 0), false));
            s.order(600, P0, Order::Attack { units: vec![ids[0]], target: ids[2] });
            // Player and sandbox input submitted for the same tick (one sequence
            // counter between them), and more player input a few ticks later.
            s.order(700, P0, Order::move_to(vec![ids[1]], v(500, 500), false));
            s.edit(700, Edit::Cargo(P0, 0, Fixed::from_int(50)));
            s.order(705, P0, Order::move_to(vec![ids[1]], v(-500, -500), false));
            s.edit(710, Edit::Cargo(P0, 0, Fixed::from_int(60)));
        },
    )
}

// --------------------------------------------------------------------- runs

fn info() -> SaveInfo {
    SaveInfo {
        build_id: "test-build".into(),
        install_key: "authored".into(),
        mod_key: "none".into(),
        map: Some(MapId { path: "maps/two_bases".into(), hash: MAP_HASH }),
        local_seat: Some(P0),
        play_time_secs: 123,
        made_at: 1_700_000_000,
        description: "an authored test save".into(),
        ..SaveInfo::default()
    }
}

/// Where a file's header ends (after its checksum): the 18-byte prefix, the
/// header length at offset 14, the header, its 8-byte checksum.
fn header_end(file: &[u8]) -> usize {
    18 + u32::from_le_bytes([file[14], file[15], file[16], file[17]]) as usize + 8
}

fn client() -> ClientState {
    ClientState { version: 3, data: vec![9, 8, 7, 6, 5] }
}

struct Run {
    sim: Sim,
    replay: Replay,
    ctl: Vec<(PlayerId, Box<dyn Controller>)>,
    /// Submitted for the next tick and not yet stepped: the player's, the
    /// sandbox's and the controllers' commands (the link's pending list).
    pending: Vec<Command>,
    /// The link's one sequence counter, shared by the player's and the
    /// sandbox's commands.
    next_seq: u32,
}

impl Run {
    fn start(sc: &Scenario) -> Run {
        let setup = setup_of(sc);
        let sim = Sim::from_setup(&setup);
        let mut replay = Replay::new(&setup);
        replay.digests.push((0, sim.digest()));
        let ctl = setup
            .players
            .iter()
            .filter(|p| matches!(p.control, Control::Ai(_)))
            .map(|p| (p.id, Box::new(Counter::default()) as Box<dyn Controller>))
            .collect();
        Run { sim, replay, ctl, pending: Vec::new(), next_seq: 1 }
    }

    /// The player's and the sandbox's input for the tick the sim is at,
    /// submitted with the link's one counter (`SimLink::submit`).
    fn submit_script(&mut self, script: &Script) {
        for (player, kind) in script.at(self.sim.tick) {
            self.pending.push(Command { player: *player, seq: self.next_seq, kind: kind.clone() });
            self.next_seq += 1;
        }
    }

    /// One client tick, in the order of `SimLink::tick`: take what is
    /// pending, step, log the batch, then every controller thinks about THIS
    /// step's events and submits at once (for the next tick).
    fn tick(&mut self) -> TickOutput {
        let batch = std::mem::take(&mut self.pending);
        let t = self.sim.tick;
        let out = self.sim.step(&batch);
        self.replay.log_step(t, &batch, out.view.tick, out.view.digest, EVERY);
        let view = self.sim.view();
        for (_, c) in &mut self.ctl {
            c.think(&view, &out.events, &mut self.pending);
        }
        out
    }

    /// A frame: input, then a tick.
    fn step(&mut self, script: &Script) -> TickOutput {
        self.submit_script(script);
        self.tick()
    }

    fn in_flight(&self) -> InFlight {
        InFlight { next_seq: self.next_seq, commands: self.pending.clone() }
    }

    /// A save between ticks (after the input was submitted, before the tick).
    fn save(&self, info: &SaveInfo) -> Result<Vec<u8>, SaveError> {
        let refs: Vec<(PlayerId, &dyn Controller)> = self.ctl.iter().map(|(p, c)| (*p, c.as_ref())).collect();
        save::save(&self.sim, &self.replay, info, &refs, &self.in_flight(), &client())
    }

    fn controller_states(&self) -> Vec<(PlayerId, Option<ControllerState>)> {
        self.ctl.iter().map(|(p, c)| (*p, c.save_state())).collect()
    }
}

struct Reference {
    outs: Vec<TickOutput>,
    /// The save made at the tick (between ticks: after the controllers'
    /// think about the step before it and the input for the tick), and the
    /// commands in flight it holds.
    saves: BTreeMap<u64, Vec<u8>>,
    in_flight: BTreeMap<u64, InFlight>,
    last_snapshot: Vec<u8>,
    last_replay: Vec<u8>,
    last_controllers: Vec<(PlayerId, Option<ControllerState>)>,
}

fn run_reference(sc: &Scenario, saves: &[u64]) -> Reference {
    let mut run = Run::start(sc);
    let mut outs = Vec::new();
    let mut files = BTreeMap::new();
    let mut flights = BTreeMap::new();
    for t in 0..sc.ticks {
        run.submit_script(&sc.script);
        if saves.contains(&t) {
            files.insert(t, run.save(&info()).unwrap_or_else(|e| panic!("{}: save at {t}: {e}", sc.name)));
            flights.insert(t, run.in_flight());
        }
        outs.push(run.tick());
    }
    Reference {
        outs,
        saves: files,
        in_flight: flights,
        last_snapshot: run.sim.snapshot(),
        last_replay: run.replay.encode(),
        last_controllers: run.controller_states(),
    }
}

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("save-file-{}-{label}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).expect("scratch folder");
    dir
}

/// Through a file, as the client does it: written atomically under a
/// generated name, listed, read back.
fn through_file(label: &str, bytes: &[u8], db: &ContentDb) -> Vec<u8> {
    let dir = scratch(label);
    let path = store::write_new(&dir, bytes).expect("writes");
    let listed = store::list(&dir, Some(db)).expect("lists");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].outcome.is_ok(), "{label}: listed as refused: {:?}", listed[0].outcome);
    let back = store::read_file(&path).expect("reads");
    std::fs::remove_dir_all(&dir).expect("cleanup");
    back
}

/// A deliberate fault for the controls: the harness must fail on each.
#[derive(Clone, Copy, PartialEq)]
enum Tamper {
    Nothing,
    /// The commands in flight are not submitted after the load.
    DropInFlight,
    /// The restored controllers think once about the saved tick's step
    /// before it goes on (a double think).
    DoubleThink,
    /// The sequence counter is derived from the local player's commands in
    /// flight instead of restored.
    DerivedSeq,
}

/// Loads the save made at tick `n` in a fresh context and continues.
fn resume(sc: &Scenario, r: &Reference, n: u64) {
    resume_tampered(sc, r, n, Tamper::Nothing);
}

fn resume_tampered(sc: &Scenario, r: &Reference, n: u64, tamper: Tamper) {
    let name = sc.name;
    let saved = &r.saves[&n];
    let bytes = through_file(&format!("{name}-{n}"), saved, &(sc.content)());
    assert!(&bytes == saved, "{name}: the file round trip changed the bytes (save at {n})");
    // Fresh context: the content is built anew, nothing is carried over.
    let loaded = load(&bytes, (sc.content)(), map_source()).unwrap_or_else(|e| panic!("{name}: save at {n} does not load: {e}"));
    assert_eq!(loaded.sim.tick, n);
    assert_eq!(loaded.header.tick, n);
    assert_eq!(loaded.client, client());
    assert_eq!(loaded.info, info());
    let want_digest = if n == 0 { Sim::from_setup(&setup_of(sc)).digest() } else { r.outs[n as usize - 1].view.digest };
    assert_eq!(loaded.sim.digest(), want_digest, "{name}: digest right after loading the save at {n}");

    // The replay rebuilt from the journal plays back to the same state.
    let played = loaded.replay.play((sc.content)()).unwrap_or_else(|e| panic!("{name}: the rebuilt replay at {n}: {e}"));
    assert_eq!(played.tick, n);
    assert_eq!(played.digest(), loaded.sim.digest(), "{name}: the rebuilt replay plays to another state (save at {n})");
    assert!(played.snapshot() == loaded.sim.snapshot(), "{name}: replayed snapshot differs (save at {n})");

    let ctl = loaded.restore_controllers(restore_counter).unwrap_or_else(|e| panic!("{name}: {e}"));
    // Load then save gives the same bytes.
    {
        let refs: Vec<(PlayerId, &dyn Controller)> = ctl.iter().map(|(p, c)| (*p, c.as_ref())).collect();
        let again = save::save(&loaded.sim, &loaded.replay, &loaded.info, &refs, &loaded.in_flight, &loaded.client).expect("saves again");
        assert!(again == bytes, "{name}: load then save at {n} gives other bytes");
    }

    // What was in flight comes back as it was, the counter included.
    assert_eq!(loaded.in_flight, r.in_flight[&n], "{name}: the commands in flight at {n}");
    // Continue as the client does: submit them for the next tick, do NOT
    // think (the controllers already thought about this step), step on.
    let mut in_flight = loaded.in_flight;
    match tamper {
        Tamper::DropInFlight => in_flight.commands.clear(),
        Tamper::DerivedSeq => {
            in_flight.next_seq = in_flight.commands.iter().filter(|c| c.player == P0).map(|c| c.seq + 1).max().unwrap_or(1);
        }
        Tamper::Nothing | Tamper::DoubleThink => {}
    }
    let mut run = Run { sim: loaded.sim, replay: loaded.replay, ctl, pending: in_flight.commands, next_seq: in_flight.next_seq };
    if tamper == Tamper::DoubleThink {
        let events = if n == 0 { Vec::new() } else { r.outs[n as usize - 1].events.clone() };
        let view = run.sim.view();
        for (_, c) in &mut run.ctl {
            c.think(&view, &events, &mut run.pending);
        }
    }
    for t in n..sc.ticks {
        // The input for the saved tick was submitted before the save.
        if t > n {
            run.submit_script(&sc.script);
        }
        let out = run.tick();
        let want = &r.outs[t as usize];
        assert_eq!(out.view.digest, want.view.digest, "{name}: save at {n}, digest of tick {t}");
        assert_eq!(out.events, want.events, "{name}: save at {n}, events of tick {t}");
        assert!(out == *want, "{name}: save at {n}, view of tick {t} differs although the digest is equal");
    }
    assert!(run.sim.snapshot() == r.last_snapshot, "{name}: save at {n}, snapshot bytes at the end differ");
    assert!(run.replay.encode() == r.last_replay, "{name}: save at {n}, the continued journal differs");
    assert_eq!(run.controller_states(), r.last_controllers, "{name}: save at {n}, controller state at the end differs");
}

fn check(sc: Scenario, saves: &[u64]) -> Reference {
    let r = run_reference(&sc, saves);
    for &n in saves {
        resume(&sc, &r, n);
    }
    r
}

#[test]
fn a_fight_saved_loaded_and_continued_equals_the_uninterrupted_run() {
    let r = check(fleet_combat(), &[0, 1, 2, 60, 250, 400, 700]);
    let fired = r.outs.iter().flat_map(|o| &o.events).filter(|e| matches!(e, SimEvent::Fired { .. })).count();
    assert!(fired > 0, "the scenario fights");
}

#[test]
fn a_formation_group_round_a_moon_saved_and_loaded_mid_route() {
    let r = check(formation_round_a_moon(), &[0, 1, 2, 90, 300, 500, 501, 800, 1100]);
    let flying = |n: usize| r.outs[n - 1].view.units.iter().any(|u| u.speed > Fixed::ZERO);
    assert!(flying(90) && flying(300) && flying(501), "the saves catch the group under way");
    // The saves at 1 and 500 have the player's orders in flight.
    assert!(r.in_flight[&1].commands.iter().any(|c| c.player == P0) && r.in_flight[&500].commands.iter().any(|c| c.player == P0));
}

#[test]
fn a_computer_player_with_its_own_state_continues_as_if_never_interrupted() {
    let saves = computer_player_save_points();
    let r = check(with_a_computer_player(), &saves);
    // The controller really acts and keeps state: its record differs from a fresh one.
    let (_, state) = &r.last_controllers[0];
    assert!(state.as_ref().unwrap().data != [0u8; 8], "the controller never acted");
    // The save points are the interesting ones: the controller's own command
    // is in flight right after its think, and at 700 the player's and the
    // sandbox's commands are in flight together with the controller's.
    assert!(saves.iter().any(|n| r.in_flight[n].commands.iter().any(|c| c.player == P1)), "no save has the controller's command in flight");
    let both = &r.in_flight[&700];
    assert!(both.commands.iter().any(|c| c.player == P0) && both.commands.iter().any(|c| c.player == PlayerId::BENCH));
}

/// Where the computer-player match is saved: before the first tick, just
/// after ticks whose shots the controller answered, and between two inputs
/// of the player and the sandbox (which share one sequence counter).
fn computer_player_save_points() -> Vec<u64> {
    let sc = with_a_computer_player();
    let probe = run_reference(&sc, &[]);
    let fired: Vec<u64> = probe
        .outs
        .iter()
        .enumerate()
        .filter(|(_, o)| o.events.iter().any(|e| matches!(e, SimEvent::Fired { .. })))
        .map(|(i, _)| i as u64 + 1)
        .filter(|&n| n < sc.ticks)
        .collect();
    assert!(fired.len() >= 4, "the scenario fires shots ({})", fired.len());
    let mut saves = vec![0, 1, 4, 29, 300, 601, 700, 706, 900];
    saves.extend([fired[0], fired[fired.len() / 3], fired[fired.len() / 2], fired[fired.len() - 1]]);
    saves.sort_unstable();
    saves.dedup();
    saves
}

/// A controller restored from a wrong record sends other commands: the
/// comparison above would fail. Control for the harness.
#[test]
#[should_panic(expected = "controller state at the end differs")]
fn control_a_controller_restored_without_its_state_is_noticed() {
    let sc = with_a_computer_player();
    let r = run_reference(&sc, &[300]);
    let loaded = load(&r.saves[&300], content(), map_source()).unwrap();
    let mut run = Run {
        sim: loaded.sim,
        replay: loaded.replay,
        ctl: vec![(P1, Box::new(Counter::default()))],
        pending: loaded.in_flight.commands,
        next_seq: loaded.in_flight.next_seq,
    };
    for _ in 300..sc.ticks {
        run.tick();
        run.submit_script(&sc.script);
    }
    assert_eq!(run.controller_states(), r.last_controllers, "controller state at the end differs");
}

/// A save point with the controller's command, the player's and the
/// sandbox's in flight, and right after shots.
fn control_points() -> (Scenario, Reference, u64, u64) {
    let sc = with_a_computer_player();
    let probe = run_reference(&sc, &[]);
    let shots = probe.outs.iter().position(|o| o.events.iter().any(|e| matches!(e, SimEvent::Fired { .. }))).expect("shots") as u64 + 1;
    let r = run_reference(&sc, &[shots, 700]);
    (sc, r, shots, 700)
}

/// The commands in flight matter: dropped after a load, the game goes
/// another way. Control for the harness (it must fail the comparison).
#[test]
#[should_panic(expected = "save at 700")]
fn control_commands_in_flight_dropped_are_noticed() {
    let (sc, r, _, both) = control_points();
    resume_tampered(&sc, &r, both, Tamper::DropInFlight);
}

/// A controller that thinks again about the saved tick's step (what
/// delivering the saved events after a load would do) is noticed.
#[test]
#[should_panic(expected = "save at")]
fn control_a_double_think_is_noticed() {
    let (sc, r, shots, _) = control_points();
    resume_tampered(&sc, &r, shots, Tamper::DoubleThink);
}

/// The sequence counter is shared between the player and the sandbox, so
/// deriving it from the player's own commands after a sandbox command gives
/// other numbers to what is submitted next. Control for the harness.
#[test]
#[should_panic(expected = "save at 700")]
fn control_a_derived_sequence_number_is_noticed() {
    let (sc, r, _, both) = control_points();
    resume_tampered(&sc, &r, both, Tamper::DerivedSeq);
}

// --------------------------------------------------------- bytes and header

#[test]
fn saving_twice_gives_identical_bytes_and_the_time_is_only_in_the_header() {
    let sc = fleet_combat();
    let mut run = Run::start(&sc);
    for _ in 0..200 {
        run.step(&sc.script);
    }
    let (a, b) = (run.save(&info()).unwrap(), run.save(&info()).unwrap());
    assert!(a == b, "two saves of one state differ");
    let mut later = info();
    later.made_at += 60;
    later.description = "another".into();
    let c = run.save(&later).unwrap();
    assert!(c != a);
    // Only the header differs: the sections after it are the same bytes.
    let end = header_end;
    assert!(a[end(&a)..a.len() - 8] == c[end(&c)..c.len() - 8], "the sections depend on the caller's time or description");
}

#[test]
fn the_header_reads_without_the_sections() {
    let sc = with_a_computer_player();
    let mut run = Run::start(&sc);
    for _ in 0..77 {
        run.step(&sc.script);
    }
    let bytes = run.save(&info()).unwrap();
    let header_end = header_end(&bytes);
    assert!(header_end < bytes.len() / 2, "the header is a small part of the file");
    let h = read_header(&bytes[..header_end]).expect("reads from the header's bytes alone");
    assert_eq!(h.tick, 77);
    assert_eq!(h.map, MapId { path: "maps/two_bases".into(), hash: MAP_HASH });
    assert_eq!(h.content_hash, content().hash());
    assert_eq!(h.lineage, content().lineage());
    assert_eq!(h.snapshot_format, sta_sim::snapshot::FORMAT_VERSION);
    assert_eq!(h.replay_format, sta_sim::replay::FORMAT_VERSION);
    assert_eq!(h.seam_rev, sta_sim::SEAM_REV);
    assert_eq!((h.seed, h.local_seat, h.play_time_secs, h.made_at), (11, Some(P0), 123, 1_700_000_000));
    assert_eq!(h.description, "an authored test save");
    assert_eq!(h.kind, GameKind::Skirmish);
    assert_eq!(h.mission, Default::default());
    assert_eq!(h.seats.len(), 2);
    assert!(h.refusal(Some(&content())).is_none());
    // One byte short of the header is not enough.
    assert!(matches!(read_header(&bytes[..header_end - 1]), Err(SaveError::Damaged(_))));
}

// ----------------------------------------------------------------- refusals

fn a_save() -> (Scenario, Vec<u8>) {
    let sc = with_a_computer_player();
    let mut run = Run::start(&sc);
    for _ in 0..40 {
        run.step(&sc.script);
    }
    let bytes = run.save(&info()).unwrap();
    (sc, bytes)
}

#[test]
fn not_a_save_file() {
    for junk in [&b""[..], b"ST", b"STAS....", b"hello, this is not a save file at all"] {
        let e = load(junk, content(), map_source()).err().expect("refused");
        assert_eq!(e, SaveError::NotASave);
        assert_eq!(e.to_string(), "not a STA64 save file");
    }
    // A snapshot or a replay is not a save either.
    let sc = fleet_combat();
    let run = Run::start(&sc);
    assert_eq!(load(&run.sim.snapshot(), content(), map_source()).err(), Some(SaveError::NotASave));
    assert_eq!(load(&run.replay.encode(), content(), map_source()).err(), Some(SaveError::NotASave));
}

#[test]
fn another_container_version_is_refused_not_migrated() {
    let (_, mut bytes) = a_save();
    for found in [0u16, 2, 77] {
        bytes[4..6].copy_from_slice(&found.to_le_bytes());
        let e = load(&bytes, content(), map_source()).err().expect("refused");
        assert_eq!(e, SaveError::ContainerVersion { found, expected: save::CONTAINER_VERSION });
        assert!(e.to_string().contains("save container version"), "{e}");
        assert!(matches!(read_header(&bytes), Err(SaveError::ContainerVersion { .. })));
    }
}

#[test]
fn other_content_is_refused_naming_where_it_came_from() {
    let (_, bytes) = a_save();
    let other = content_with_formations();
    assert_ne!(other.hash(), content().hash(), "the two authored databases differ");
    let e = load(&bytes, other, map_source()).err().expect("refused");
    match &e {
        SaveError::ContentMismatch { saved, given, lineage, .. } => {
            assert_eq!(*saved, content().hash());
            assert_ne!(saved, given);
            assert_eq!(lineage, "native");
        }
        e => panic!("{e:?}"),
    }
    assert!(e.to_string().contains("other content"), "{e}");
    // The same refusal, from the header alone, is what the load list shows.
    assert_eq!(read_header(&bytes).unwrap().refusal(Some(&content_with_formations())), Some(e));
}

#[test]
fn the_map_missing_or_changed_is_refused() {
    let (_, bytes) = a_save();
    let e = load(&bytes, content(), None).err().expect("refused");
    assert_eq!(e, SaveError::MapMissing { path: "maps/two_bases".into() });
    assert!(e.to_string().contains("maps/two_bases"));
    let changed = Some(MapSource { hash: MAP_HASH + 1, map: Arc::new(map()) });
    let e = load(&bytes, content(), changed).err().expect("refused");
    assert_eq!(e, SaveError::MapChanged { path: "maps/two_bases".into(), saved: MAP_HASH, given: MAP_HASH + 1 });
    assert!(e.to_string().contains("changed"));
}

#[test]
fn a_controller_that_cannot_be_restored_refuses_the_load_naming_the_seat() {
    let (_, bytes) = a_save();
    let loaded = load(&bytes, content(), map_source()).unwrap();
    assert_eq!(loaded.controllers.len(), 1);
    assert_eq!((loaded.controllers[0].seat, loaded.controllers[0].name.as_str()), (P1, "counter"));
    let e = loaded.restore_controllers(|_, _, _| Err("record version 9 is not readable".to_string())).err().expect("refused");
    assert_eq!(e, SaveError::Controller { seat: P1, reason: "record version 9 is not readable".into() });
    assert!(e.to_string().contains("seat 1"), "{e}");
    // A record of a wrong shape is refused by the controller itself.
    let mut c = Counter::default();
    assert!(c.restore_state(&ControllerState { version: 1, data: vec![1] }).is_err());
}

#[test]
fn a_controller_that_has_not_opted_in_refuses_the_save_naming_the_seat() {
    let sc = with_a_computer_player();
    let run = Run::start(&sc);
    let e = save::save(&run.sim, &run.replay, &info(), &[(P1, &Silent)], &InFlight::default(), &client()).expect_err("refused");
    assert_eq!(e, SaveError::ControllerCannotSave { seat: P1 });
    assert!(e.to_string().contains("seat 1"));
    // No controller at all for a computer seat, and one for a seat that has none.
    assert!(matches!(save::save(&run.sim, &run.replay, &info(), &[], &InFlight::default(), &client()), Err(SaveError::Invalid(_))));
    let idle = fleet_combat();
    let run = Run::start(&idle);
    assert!(matches!(save::save(&run.sim, &run.replay, &info(), &[(P1, &Counter::default())], &InFlight::default(), &client()), Err(SaveError::Invalid(_))));
}

#[test]
fn a_match_whose_map_has_no_file_is_refused_at_save() {
    let sc = fleet_combat();
    let run = Run::start(&sc);
    let mut i = info();
    i.map = None;
    let e = run.save(&i).expect_err("refused");
    assert_eq!(e, SaveError::NoMapFile);
    assert!(e.to_string().contains("no map file"));
}

#[test]
fn a_networked_match_is_refused_at_save() {
    let mut sc = fleet_combat();
    sc.enemy = Control::Remote;
    let run = Run::start(&sc);
    assert_eq!(run.save(&info()).err(), Some(SaveError::Networked { seat: P1 }));
    sc.enemy = Control::RemoteAi("counter".into());
    let run = Run::start(&sc);
    assert_eq!(run.save(&info()).err(), Some(SaveError::Networked { seat: P1 }));
}

#[test]
fn a_journal_of_another_run_is_refused_at_save() {
    let sc = fleet_combat();
    let mut run = Run::start(&sc);
    run.step(&sc.script);
    run.step(&sc.script);
    run.replay.end_tick = 1;
    assert!(matches!(run.save(&info()), Err(SaveError::Invalid(why)) if why.contains("tick")));
    run.replay.end_tick = 2;
    run.replay.content_hash ^= 1;
    assert!(matches!(run.save(&info()), Err(SaveError::Invalid(why)) if why.contains("content")));
}

// ---------------------------------------------- damage, cuts and random bytes

#[test]
fn every_flipped_byte_is_refused_never_accepted() {
    let (sc, bytes) = a_save();
    for i in 0..bytes.len() {
        for mask in [0x01u8, 0x80] {
            let mut b = bytes.clone();
            b[i] ^= mask;
            assert!(load(&b, (sc.content)(), map_source()).is_err(), "byte {i} flipped with {mask:#x} was accepted");
        }
    }
    // The header's fields each flip to a refusal too (a list sees it from the header alone).
    let header_end = header_end(&bytes);
    for i in 0..header_end {
        let mut b = bytes[..header_end].to_vec();
        b[i] ^= 0x10;
        assert!(read_header(&b).is_err(), "header byte {i} flipped was accepted");
    }
}

#[test]
fn truncation_at_every_length_is_refused_as_damage() {
    let (sc, bytes) = a_save();
    for n in 0..bytes.len() {
        let e = load(&bytes[..n], (sc.content)(), map_source()).err().unwrap_or_else(|| panic!("a file cut to {n} bytes loaded"));
        if n >= 10 {
            assert!(matches!(e, SaveError::Damaged(_) | SaveError::NotASave), "cut at {n}: {e:?}");
        }
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(load(&longer, content(), map_source()).is_err(), "a byte appended is accepted");
}

/// A small xorshift: random but the same on every run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn random_bytes_are_refused_without_a_panic() {
    let (_, good) = a_save();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let db = content();
    for round in 0..3000 {
        let len = (rng.next() % 300) as usize;
        let mut b: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        // Half of them start like a save, some with the real prefix.
        match round % 3 {
            0 => {}
            1 if b.len() >= 10 => b[..4].copy_from_slice(b"STAG"),
            2 if b.len() >= 10 => {
                let k = ((rng.next() % 40) as usize).min(good.len()).min(b.len());
                b[..k].copy_from_slice(&good[..k]);
            }
            _ => {}
        }
        assert!(load(&b, Arc::clone(&db), map_source()).is_err(), "random bytes loaded (round {round})");
        let _ = read_header(&b);
    }
}

// ------------------------------------------------------------ the file store

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    v.sort();
    v
}

#[test]
fn an_interrupted_write_leaves_the_earlier_save_intact_and_is_not_listed() {
    let dir = scratch("atomic");
    let (_, first) = a_save();
    let path = store::write_atomic(&dir, "game.stasave", &first).expect("writes");
    assert_eq!(names(&dir), ["game.stasave"]);

    // The last step (the rename) fails: the target is untouched, no temporary file stays.
    let sc = fleet_combat();
    let mut run = Run::start(&sc);
    run.step(&sc.script);
    let second = run.save(&info()).unwrap();
    assert!(second != first);
    let failing = |_: &Path, _: &Path| Err(std::io::Error::other("the disk went away"));
    let e = store::write_atomic_with(&dir, "game.stasave", &second, &failing).expect_err("fails");
    assert!(matches!(e, SaveError::Io(_)));
    assert!(std::fs::read(&path).unwrap() == first, "the earlier save was changed");
    assert_eq!(names(&dir), ["game.stasave"]);

    // A crash that left the temporary file behind (half written).
    std::fs::write(dir.join("game.stasave.tmp"), &second[..second.len() / 2]).unwrap();
    std::fs::write(dir.join("notes.txt"), b"not a save").unwrap();
    let listed = store::list(&dir, Some(&content())).unwrap();
    assert_eq!(listed.len(), 1, "temporary files and other kinds are not listed");
    assert_eq!(listed[0].file_name, "game.stasave");
    assert!(listed[0].outcome.is_ok());
    assert!(load(&store::read_file(&path).unwrap(), content(), map_source()).is_ok(), "the earlier save no longer loads");

    // A completed write replaces it whole.
    store::write_atomic(&dir, "game.stasave", &second).unwrap();
    assert!(std::fs::read(&path).unwrap() == second);
    std::fs::remove_dir_all(&dir).unwrap();
}

type Writer = fn(&Path, &[u8]) -> Result<PathBuf, SaveError>;

/// `write_new` as it runs where hard links work.
fn linking(dir: &Path, bytes: &[u8]) -> Result<PathBuf, SaveError> {
    store::write_new(dir, bytes)
}

/// `write_new` with the link step failing as on a volume without hard links:
/// the fallback path.
fn fallback(dir: &Path, bytes: &[u8]) -> Result<PathBuf, SaveError> {
    store::write_new_with(dir, bytes, &|_, _| Err(std::io::Error::from(std::io::ErrorKind::Unsupported)))
}

#[test]
fn file_names_are_checked_and_generated_names_never_overwrite() {
    names_are_checked(linking, "names");
}

#[test]
fn the_same_with_the_fallback_forced() {
    names_are_checked(fallback, "names-fb");
}

fn names_are_checked(write_new: Writer, label: &str) {
    let dir = scratch(label);
    let (_, bytes) = a_save();
    for bad in ["", "x", "a/b.stasave", "..\\x.stasave", ".hidden.stasave", "c:x.stasave", "x.txt", "a b.stasave"] {
        assert!(matches!(store::write_atomic(&dir, bad, &bytes), Err(SaveError::Invalid(_))), "{bad:?}");
    }
    let one = write_new(&dir, &bytes).unwrap();
    let two = write_new(&dir, &bytes).unwrap();
    assert_ne!(one, two, "a second save of the same moment kept its own file");
    assert_eq!(names(&dir).len(), 2);
    assert!(one.file_name().unwrap().to_str().unwrap().starts_with("save-"));
    assert!(write_new(&dir, b"junk").is_err());
    // A missing folder lists as empty; a file in its place is an error.
    assert!(store::list(&dir.join("nope"), None).unwrap().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A save already under the generated name, and others at its `-2`, `-3`
/// names, are never overwritten, and the returned path is the file written,
/// on both paths.
fn never_overwrites(write_new: Writer, label: &str) {
    let dir = scratch(label);
    let (_, first) = a_save();
    let name = store::generated_name(&read_header(&first).unwrap());
    let stem = name.strip_suffix(".stasave").unwrap().to_owned();
    // Same moment, other content (another description): the same generated name.
    let sc = with_a_computer_player();
    let mut run = Run::start(&sc);
    for _ in 0..40 {
        run.step(&sc.script);
    }
    let mut other_info = info();
    other_info.description = "another save of the same moment".into();
    let second = run.save(&other_info).unwrap();
    assert!(second != first);
    assert_eq!(store::generated_name(&read_header(&second).unwrap()), name);
    // Taken: the plain name and the first suffix, with content of their own.
    std::fs::write(dir.join(&name), b"precious one").unwrap();
    std::fs::write(dir.join(format!("{stem}-2.stasave")), b"precious two").unwrap();
    let mut written = Vec::new();
    for bytes in [&first, &second, &first] {
        let path = write_new(&dir, bytes).unwrap();
        assert!(std::fs::read(&path).unwrap() == *bytes, "the returned path {path:?} is not the file that was written");
        written.push(path);
    }
    let mut unique = written.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3, "repeated calls gave the same file: {written:?}");
    assert_eq!(std::fs::read(dir.join(&name)).unwrap(), b"precious one", "an existing save was overwritten");
    assert_eq!(std::fs::read(dir.join(format!("{stem}-2.stasave"))).unwrap(), b"precious two", "an existing save was overwritten");
    assert!(written.iter().all(|p| p != &dir.join(&name) && p != &dir.join(format!("{stem}-2.stasave"))));
    // Two precious files, three new ones, no temporary file left.
    assert_eq!(names(&dir).len(), 5, "{:?}", names(&dir));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_write_never_overwrites_an_existing_save() {
    never_overwrites(linking, "keep");
}

#[test]
fn a_write_never_overwrites_with_the_fallback_forced() {
    never_overwrites(fallback, "keep-fb");
}

/// Writers of the same moment never overwrite each other, and none leaves a
/// temporary file behind.
#[test]
fn concurrent_writers_each_get_their_own_file() {
    concurrent(linking, "concurrent");
}

#[test]
fn concurrent_writers_with_the_fallback_forced() {
    concurrent(fallback, "concurrent-fb");
}

fn concurrent(write_new: Writer, label: &str) {
    let dir = scratch(label);
    let (_, bytes) = a_save();
    let paths: Vec<PathBuf> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8).map(|_| s.spawn(|| write_new(&dir, &bytes).expect("writes"))).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut unique = paths.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 8, "two writers got the same file");
    assert_eq!(names(&dir).len(), 8, "{:?}", names(&dir));
    assert!(names(&dir).iter().all(|n| n.ends_with(".stasave")));
    for p in &paths {
        assert!(std::fs::read(p).unwrap() == bytes);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_listing_shows_refused_files_with_their_reason() {
    let dir = scratch("list");
    let (_, good) = a_save();
    store::write_atomic(&dir, "a-good.stasave", &good).unwrap();
    store::write_atomic(&dir, "b-junk.stasave", b"this is not a save file").unwrap();
    let mut old = good.clone();
    old[4..6].copy_from_slice(&0u16.to_le_bytes());
    store::write_atomic(&dir, "c-old.stasave", &old).unwrap();
    let mut cut = good.clone();
    cut.truncate(14);
    store::write_atomic(&dir, "d-cut.stasave", &cut).unwrap();
    store::write_atomic(&dir, "e-other-content.stasave", &good).unwrap();

    let listed = store::list(&dir, Some(&content())).unwrap();
    let by: BTreeMap<&str, &Result<_, SaveError>> = listed.iter().map(|l| (l.file_name.as_str(), &l.outcome)).collect();
    assert_eq!(by.len(), 5, "refused files are listed, never hidden");
    assert!(by["a-good.stasave"].is_ok());
    assert!(matches!(by["b-junk.stasave"], Err(SaveError::NotASave)));
    assert!(matches!(by["c-old.stasave"], Err(SaveError::ContainerVersion { found: 0, .. })));
    assert!(matches!(by["d-cut.stasave"], Err(SaveError::Damaged(_))));
    // Without content the list cannot judge it; with other content it refuses.
    let other = content_with_formations();
    let listed = store::list(&dir, Some(&other)).unwrap();
    let e = listed.iter().find(|l| l.file_name == "a-good.stasave").unwrap();
    assert!(matches!(e.outcome, Err(SaveError::ContentMismatch { .. })));
    let e = store::list(&dir, None).unwrap();
    assert!(e.iter().find(|l| l.file_name == "a-good.stasave").unwrap().outcome.is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}
