//! SIM-70: station Recrew through real commands and simulation ticks.

use std::sync::Arc;

use sta_content::{ContentDb, CrewRules, MapDef, MapObject, Profile, Rules, UnitDef};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};

use crate::command::{Command, CommandKind, Edit, Order};
use crate::event::SimEvent;
use crate::recrew::{RecrewRec, Recrews, Row};
use crate::replay::{Recorder, Replay};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::UnitFlags;
use crate::{Sim, WorldSource};

const ME: PlayerId = PlayerId(0);
const OTHER: PlayerId = PlayerId(1);

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn station(
    name: &str,
    rate: Option<Fixed>,
    facility: Option<bool>,
    has_crew: Option<bool>,
) -> UnitDef {
    let mut def = UnitDef::new(name, "starbase");
    def.race = "fx".into();
    def.hull = n(1000);
    def.crew = n(100);
    def.maximum_crew = Some(n(100));
    def.casualty.recrew_rate = rate;
    def.facility = facility;
    def.has_crew = has_crew;
    def
}

fn content() -> Arc<ContentDb> {
    let mut craft = UnitDef::new("craft", "craft");
    craft.race = "fx".into();
    craft.hull = n(1000);
    craft.crew = n(100);
    craft.maximum_crew = Some(n(100));
    Arc::new(
        ContentDb::builder()
            .unit(station("station", None, None, None))
            .unit(station("slow", Some(Fixed::from_f64(2.5)), None, None))
            .unit(station("zero_rate", Some(Fixed::ZERO), None, None))
            .unit(station("negative_rate", Some(n(-2)), None, None))
            .unit(station("no_facility", None, Some(false), None))
            .unit(station("no_crew", None, None, Some(false)))
            .unit(craft)
            .build()
            .unwrap()
            .with_rules(Rules::default().with_crew(CrewRules {
                starting_capacity: Some(1000),
                max_crew: None,
                races: vec![("fx".into(), Fixed::ZERO)],
            })),
    )
}

fn setup(map: MapDef) -> GameSetup {
    GameSetup {
        seed: 70,
        map: Arc::new(map),
        content: content(),
        players: [ME, OTHER]
            .map(|id| PlayerSetup {
                id,
                team: TeamId(id.0),
                race: "fx".into(),
                control: Control::Idle,
            })
            .to_vec(),
        options: GameOptions::default(),
    }
}

fn game(pool: Option<Fixed>) -> Sim {
    let mut sim = Sim::from_setup(&setup(MapDef::empty()));
    if let Some(pool) = pool {
        sim.economy.set_personnel(ME, Some(pool), None);
    }
    sim
}

fn spawn(sim: &mut Sim, name: &str, owner: PlayerId, crew: Fixed) -> EntityId {
    let key = sim.content().unwrap().key(name).unwrap();
    let id = sim
        .spawn_def(key, owner, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE))
        .unwrap();
    sim.units.get_mut(id).unwrap().crew.as_mut().unwrap()[0] = crew;
    id
}

fn order(from: PlayerId, seq: u32, ids: &[EntityId], on: Option<bool>) -> Command {
    Command {
        player: from,
        seq,
        kind: CommandKind::Order(Order::Recrew {
            units: ids.to_vec(),
            on,
        }),
    }
}

fn edit(seq: u32, e: Edit) -> Command {
    Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(e),
    }
}

fn accepted(sim: &mut Sim, commands: &[Command]) {
    let events = sim.step(commands).events;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
}

fn rejected(sim: &mut Sim, command: Command) {
    assert!(
        sim.step(&[command])
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
}

fn crew(sim: &Sim, id: EntityId) -> Fixed {
    sim.units.get(id).unwrap().crew.unwrap()[0]
}

fn pool(sim: &Sim) -> Option<Fixed> {
    sim.economy.crew_of(ME)
}

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.step(&[]);
    }
}

#[test]
fn default_rate_transfers_ten_per_second_and_conserves_exact_crew() {
    let mut sim = game(Some(n(100)));
    let id = spawn(&mut sim, "station", ME, n(40));
    let sum = crew(&sim, id) + pool(&sim).unwrap();
    accepted(&mut sim, &[order(ME, 0, &[id], Some(true))]);
    run(&mut sim, TICK_HZ - 1);
    assert_eq!(crew(&sim, id), n(50));
    assert_eq!(crew(&sim, id) + pool(&sim).unwrap(), sum);
    assert!(sim.is_recrewing(id));
}

#[test]
fn fractional_rate_and_zero_rate_do_not_create_crew() {
    let mut sim = game(Some(n(100)));
    let slow = spawn(&mut sim, "slow", ME, n(40));
    let zero = spawn(&mut sim, "zero_rate", ME, n(40));
    let negative = spawn(&mut sim, "negative_rate", ME, n(40));
    let sum = crew(&sim, slow) + crew(&sim, zero) + crew(&sim, negative) + pool(&sim).unwrap();
    accepted(
        &mut sim,
        &[order(ME, 0, &[slow, zero, negative], Some(true))],
    );
    run(&mut sim, 2 * TICK_HZ - 1);
    assert_eq!(crew(&sim, slow), n(45));
    assert_eq!(crew(&sim, zero), n(40));
    assert_eq!(crew(&sim, negative), n(40));
    assert_eq!(
        crew(&sim, slow) + crew(&sim, zero) + crew(&sim, negative) + pool(&sim).unwrap(),
        sum
    );
}

#[test]
fn cap_auto_switches_off_and_pool_never_overdraws() {
    let mut sim = game(Some(n(100)));
    let id = spawn(&mut sim, "station", ME, n(99));
    accepted(&mut sim, &[order(ME, 0, &[id], Some(true))]);
    run(&mut sim, 2 * TICK_HZ);
    assert_eq!(crew(&sim, id), n(100));
    assert_eq!(pool(&sim), Some(n(99)));
    assert!(!sim.is_recrewing(id));
    rejected(&mut sim, order(ME, 0, &[id], Some(true)));

    let mut sim = game(Some(n(1)));
    let id = spawn(&mut sim, "station", ME, n(40));
    accepted(&mut sim, &[order(ME, 0, &[id], Some(true))]);
    run(&mut sim, 2 * TICK_HZ);
    assert!(crew(&sim, id) > n(40) && crew(&sim, id) < n(41));
    assert!(pool(&sim).unwrap() < Fixed::ONE);
    assert_eq!(crew(&sim, id) + pool(&sim).unwrap(), n(41));
    assert!(sim.is_recrewing(id), "the request waits for more team crew");
}

#[test]
fn empty_and_subunit_pools_wait_then_top_up_resumes_the_same_request() {
    for initial in [Fixed::ZERO, Fixed::from_f64(0.5)] {
        let mut sim = game(Some(initial));
        let id = spawn(&mut sim, "station", ME, n(40));
        accepted(&mut sim, &[order(ME, 0, &[id], Some(true))]);
        run(&mut sim, TICK_HZ);
        assert_eq!(crew(&sim, id), n(40));
        assert_eq!(pool(&sim), Some(initial));
        assert!(sim.is_recrewing(id));
        accepted(&mut sim, &[edit(0, Edit::TeamCrew(ME, n(10)))]);
        assert!(crew(&sim, id) > n(40));
        assert_eq!(crew(&sim, id) + pool(&sim).unwrap(), n(50));
    }
    let mut sim = game(None);
    let id = spawn(&mut sim, "station", ME, n(40));
    rejected(&mut sim, order(ME, 0, &[id], Some(true)));
    assert!(!sim.is_recrewing(id));
}

#[test]
fn explicit_on_is_idempotent_toggle_deduplicates_and_stop_cancels_only_listed_stations() {
    let mut sim = game(Some(n(100)));
    let a = spawn(&mut sim, "station", ME, n(40));
    let b = spawn(&mut sim, "station", ME, n(40));
    accepted(&mut sim, &[order(ME, 0, &[a, a, b], Some(true))]);
    assert!(sim.is_recrewing(a) && sim.is_recrewing(b));
    accepted(&mut sim, &[order(ME, 0, &[a], Some(true))]);
    assert!(sim.is_recrewing(a));
    accepted(&mut sim, &[order(ME, 0, &[a, a], None)]);
    assert!(!sim.is_recrewing(a) && sim.is_recrewing(b));
    let before_reenable = crew(&sim, a);
    accepted(&mut sim, &[order(ME, 0, &[a], None)]);
    assert!(sim.is_recrewing(a));
    assert!(
        crew(&sim, a) > before_reenable,
        "the re-enabled station transfers before Stop"
    );
    accepted(
        &mut sim,
        &[Command {
            player: ME,
            seq: 0,
            kind: CommandKind::Order(Order::Stop { units: vec![a] }),
        }],
    );
    assert!(!sim.is_recrewing(a) && sim.is_recrewing(b));
    let a_crew_after_stop = crew(&sim, a);
    let total_after_stop = crew(&sim, a) + crew(&sim, b) + pool(&sim).unwrap();
    run(&mut sim, TICK_HZ);
    assert_eq!(crew(&sim, a), a_crew_after_stop);
    assert!(crew(&sim, b) > n(40));
    assert_eq!(
        crew(&sim, a) + crew(&sim, b) + pool(&sim).unwrap(),
        total_after_stop
    );
    accepted(&mut sim, &[order(ME, 0, &[b], Some(false))]);
    assert!(!sim.is_recrewing(b));
}

#[test]
fn scarce_pool_goes_to_lower_id_regardless_of_order_list() {
    let mut sim = game(Some(n(1)));
    let low = spawn(&mut sim, "station", ME, n(40));
    let high = spawn(&mut sim, "station", ME, n(40));
    accepted(&mut sim, &[order(ME, 0, &[high, low], Some(true))]);
    run(&mut sim, TICK_HZ - 1);
    assert!(crew(&sim, low) > n(40));
    assert_eq!(crew(&sim, high), n(40));
    assert!(pool(&sim).unwrap() < Fixed::ONE);
    assert_eq!(crew(&sim, low) + pool(&sim).unwrap(), n(41));
    assert!(sim.is_recrewing(high));
}

#[test]
fn order_rejects_wrong_owner_kind_build_state_liveness_and_raw_exclusions() {
    let mut sim = game(Some(n(100)));
    let foreign = spawn(&mut sim, "station", OTHER, n(40));
    let craft = spawn(&mut sim, "craft", ME, n(40));
    let unfinished = spawn(&mut sim, "station", ME, n(40));
    sim.units.get_mut(unfinished).unwrap().progress = Some(Fixed::from_f64(0.5));
    let dying = spawn(&mut sim, "station", ME, n(40));
    let flags = sim.units.get(dying).unwrap().flags;
    sim.units.get_mut(dying).unwrap().flags = flags.with(UnitFlags::DYING, true);
    let no_facility = spawn(&mut sim, "no_facility", ME, n(40));
    let no_crew = spawn(&mut sim, "no_crew", ME, n(40));
    for id in [foreign, craft, unfinished, dying, no_facility, no_crew] {
        rejected(&mut sim, order(ME, 0, &[id], Some(true)));
        assert!(!sim.is_recrewing(id));
    }
}

#[test]
fn mission_disable_cancels_and_reenable_does_not_restart() {
    let mut sim = game(Some(n(100)));
    let id = spawn(&mut sim, "station", ME, n(40));
    accepted(&mut sim, &[order(ME, 0, &[id], Some(true))]);
    accepted(&mut sim, &[edit(0, Edit::CanRecrew(id, false))]);
    assert!(!sim.is_recrewing(id));
    rejected(&mut sim, order(ME, 0, &[id], Some(true)));
    accepted(&mut sim, &[edit(0, Edit::CanRecrew(id, true))]);
    assert!(!sim.is_recrewing(id));
    accepted(&mut sim, &[order(ME, 0, &[id], Some(true))]);
    assert!(sim.is_recrewing(id));
}

#[test]
fn positive_hit_and_script_damage_hold_for_ten_seconds_across_toggles() {
    let mut sim = game(Some(n(100)));
    let id = spawn(&mut sim, "station", ME, n(40));
    let db = sim.content().unwrap().clone();
    sim.recrew.note_hits(
        &sim.units,
        &db,
        &[SimEvent::Hit {
            target: id,
            by: EntityId(999),
            weapon: ContentKey(0),
            facing: 0,
            shield: n(1),
            hull: Fixed::ZERO,
            at: Vec3::ZERO,
        }],
        sim.tick,
    );
    accepted(&mut sim, &[order(ME, 0, &[id], Some(true))]);
    let before = crew(&sim, id);
    accepted(&mut sim, &[order(ME, 0, &[id], Some(false))]);
    accepted(&mut sim, &[order(ME, 0, &[id], Some(true))]);
    run(&mut sim, 10 * TICK_HZ - 4);
    assert_eq!(crew(&sim, id), before);
    run(&mut sim, 3);
    assert!(crew(&sim, id) > before);
    accepted(&mut sim, &[edit(0, Edit::Damage(id, n(1)))]);
    let paused = crew(&sim, id);
    run(&mut sim, TICK_HZ);
    assert_eq!(crew(&sim, id), paused);
}

#[test]
fn death_capture_and_changed_maximum_settle_state_without_extra_pool_debit() {
    let mut sim = game(Some(n(100)));
    let captured = spawn(&mut sim, "station", ME, n(40));
    let killed = spawn(&mut sim, "station", ME, n(40));
    let capped = spawn(&mut sim, "station", ME, n(40));
    accepted(
        &mut sim,
        &[order(ME, 0, &[captured, killed, capped], Some(true))],
    );
    let cap = crew(&sim, capped);
    accepted(
        &mut sim,
        &[
            edit(0, Edit::Team(captured, OTHER)),
            edit(1, Edit::Kill(killed)),
            edit(2, Edit::MaxCrew(capped, cap)),
        ],
    );
    assert!(!sim.is_recrewing(captured));
    assert!(!sim.is_recrewing(killed));
    assert!(!sim.is_recrewing(capped));
    assert_eq!(
        sim.units.get(capped).unwrap().crew.unwrap()[0],
        sim.units.get(capped).unwrap().crew.unwrap()[1]
    );
}

#[test]
fn active_paused_and_empty_pool_snapshots_continue_with_identical_digests() {
    for pool_at_start in [n(100), Fixed::ZERO] {
        let mut live = game(Some(pool_at_start));
        let id = spawn(&mut live, "station", ME, n(40));
        let db = live.content().unwrap().clone();
        live.recrew
            .note_damage(&live.units, &db, id, n(1), Fixed::ZERO, live.tick);
        accepted(&mut live, &[order(ME, 0, &[id], Some(true))]);
        let bytes = live.snapshot();
        let mut restored = Sim::from_snapshot(&bytes, live.content().cloned()).unwrap();
        assert_eq!(restored.snapshot(), bytes);
        assert_eq!(restored.digest(), live.digest());
        for _ in 0..(10 * TICK_HZ + 3) {
            let a = live.step(&[]);
            let b = restored.step(&[]);
            assert_eq!(a.events, b.events);
            assert_eq!(a.view.digest, b.view.digest);
        }
    }
}

#[test]
fn malformed_recrew_records_reject_without_pruning_valid_transient_rows() {
    let mut sim = game(Some(n(10)));
    let a = spawn(&mut sim, "station", ME, n(40));
    let b = spawn(&mut sim, "station", ME, n(40));
    let craft = spawn(&mut sim, "craft", ME, n(40));
    let valid = Row {
        owner: ME,
        on: true,
        resume_at: sim.tick + 10 * TICK_HZ as u64,
    };
    let record = |rows| RecrewRec { version: 1, rows };
    assert!(Recrews::from_rec(record(vec![(a, valid)]), &sim.units, sim.tick).is_some());
    sim.units.get_mut(a).unwrap().crew.as_mut().unwrap()[0] = n(100);
    sim.units.get_mut(a).unwrap().progress = Some(Fixed::from_f64(0.5));
    assert!(Recrews::from_rec(record(vec![(a, valid)]), &sim.units, sim.tick).is_some());
    for rows in [
        vec![(a, valid), (a, valid)],
        vec![(b, valid), (a, valid)],
        vec![(craft, valid)],
        vec![(EntityId(999), valid)],
        vec![(
            a,
            Row {
                owner: OTHER,
                ..valid
            },
        )],
        vec![(
            a,
            Row {
                resume_at: sim.tick + 10 * TICK_HZ as u64 + 1,
                ..valid
            },
        )],
    ] {
        assert!(Recrews::from_rec(record(rows), &sim.units, sim.tick).is_none());
    }
    assert!(
        Recrews::from_rec(
            RecrewRec {
                version: 2,
                rows: vec![]
            },
            &sim.units,
            sim.tick
        )
        .is_none()
    );
}

#[test]
fn same_build_replay_recreates_station_recrew_digest() {
    let mut map = MapDef::empty();
    map.objects.push(MapObject {
        odf: "station".into(),
        owner: Some(ME.0),
        pos: Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        label: None,
    });
    let setup = setup(map);
    let db = setup.content.clone();
    let mut recorder = Recorder::with_interval(&setup, 1);
    let id = recorder.sim().units.iter().next().unwrap().id;
    recorder.step(&[
        edit(0, Edit::TeamCrew(ME, n(100))),
        edit(1, Edit::UnitCrew(id, n(40))),
    ]);
    recorder.step(&[order(ME, 0, &[id], Some(true))]);
    for _ in 0..TICK_HZ {
        recorder.step(&[]);
    }
    let expected = recorder.sim().digest();
    let replay = Replay::decode(&recorder.finish().encode()).unwrap();
    let played = replay.play(db).unwrap();
    assert_eq!(played.digest(), expected);
    assert!(played.is_recrewing(id));
}

#[test]
fn station_recrew_clears_the_existing_derelict_latch_when_crew_reaches_one() {
    let mut features = Profile::Sta2Classic.features();
    features.crew_loss_on_hit = true;
    let db = Arc::new(
        ContentDb::builder()
            .profile(Profile::Sta2Classic)
            .features(features)
            .unit(station("derelict_station", None, None, None))
            .build()
            .unwrap()
            .with_rules(Rules::default().with_crew(CrewRules {
                starting_capacity: Some(1000),
                max_crew: None,
                races: vec![("fx".into(), Fixed::ZERO)],
            })),
    );
    let setup = GameSetup {
        seed: 70,
        map: Arc::new(MapDef::empty()),
        content: db,
        players: vec![PlayerSetup {
            id: ME,
            team: TeamId(ME.0),
            race: "fx".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    sim.economy.set_personnel(ME, Some(n(100)), None);
    let id = spawn(&mut sim, "derelict_station", ME, n(100));
    accepted(&mut sim, &[edit(0, Edit::UnitCrew(id, Fixed::ZERO))]);
    assert_eq!(crew(&sim, id), Fixed::ZERO);
    assert_eq!(sim.is_derelict(id), Some(true));
    let conserved = crew(&sim, id) + pool(&sim).unwrap();
    let mut cleared = false;
    for tick in 0..TICK_HZ {
        if tick == 0 {
            accepted(&mut sim, &[order(ME, 0, &[id], Some(true))]);
        } else {
            sim.step(&[]);
        }
        assert_eq!(crew(&sim, id) + pool(&sim).unwrap(), conserved);
        if crew(&sim, id) < Fixed::ONE {
            assert_eq!(sim.is_derelict(id), Some(true));
        } else {
            assert_eq!(crew(&sim, id), Fixed::ONE);
            assert_eq!(
                sim.is_derelict(id),
                Some(false),
                "casualty upkeep clears the latch on this tick"
            );
            cleared = true;
            break;
        }
    }
    assert!(
        cleared,
        "Recrew must reach one crew within a second at the default rate"
    );
}

/// Mount only when explicitly requested. The selected definition and its
/// rules come from the install, while the two simulations share one setup.
#[test]
fn installed_station_recrew_records_a_binary_replay() {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    let Some(root) = std::env::var_os("STA64_TEST_RECREW_INSTALL") else {
        return;
    };
    let active = std::env::var("STA64_TEST_RECREW_MOD")
        .ok()
        .filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&root), active.as_deref()).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let def = db
        .units()
        .find(|d| {
            crate::world::EntityKind::of_def(d) == crate::world::EntityKind::Station
                && d.hull > Fixed::ZERO
                && d.facility != Some(false)
                && d.has_crew != Some(false)
                && d.maximum_crew.unwrap_or(d.crew) >= n(2)
                && d.casualty.recrew_rate.unwrap_or(n(10)) > Fixed::ZERO
        })
        .expect("installed content has a crew-capable station with positive Recrew rate");
    let station_name = def.odf.clone();
    let station_race = def.race.clone();
    let half = Fixed::from_bits(def.maximum_crew.unwrap_or(def.crew).to_bits() / 2);
    let db = Arc::new(db);
    let mut map = MapDef::empty();
    map.objects.push(MapObject {
        odf: station_name.clone(),
        owner: Some(ME.0),
        pos: Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        label: None,
    });
    let setup = GameSetup {
        seed: 70,
        map: Arc::new(map),
        content: db.clone(),
        players: vec![PlayerSetup {
            id: ME,
            team: TeamId(ME.0),
            race: station_race,
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    };
    let mut recorded = Recorder::with_interval(&setup, 1);
    let mut control = Sim::from_setup(&setup);
    let id = recorded
        .sim()
        .units
        .iter()
        .next()
        .expect("map station spawned")
        .id;
    assert_eq!(
        control.units.get(id).unwrap().def,
        recorded.sim().units.get(id).unwrap().def
    );
    let initialise = [
        edit(0, Edit::UnitCrew(id, half)),
        edit(1, Edit::TeamCrew(ME, n(1_000_000))),
    ];
    assert!(
        !recorded
            .step(&initialise)
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    accepted(&mut control, &initialise);
    let before_unit = crew(recorded.sim(), id);
    let before_pool = pool(recorded.sim()).expect("script initialized crew stock");
    assert_eq!(before_unit, crew(&control, id));
    assert_eq!(Some(before_pool), pool(&control));
    assert!(
        !recorded
            .step(&[order(ME, 0, &[id], Some(true))])
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    control.step(&[]);
    for _ in 1..TICK_HZ {
        recorded.step(&[]);
        control.step(&[]);
    }
    let gained = crew(recorded.sim(), id) - crew(&control, id);
    let spent = pool(&control).unwrap() - pool(recorded.sim()).unwrap();
    assert!(gained > Fixed::ZERO, "installed station must receive crew");
    assert_eq!(
        gained, spent,
        "transfer must conserve crew after control regeneration"
    );
    assert!(crew(recorded.sim(), id) <= recorded.sim().units.get(id).unwrap().crew.unwrap()[1]);
    let tick = recorded.sim().tick;
    let digest = recorded.sim().digest();
    let after_unit = crew(recorded.sim(), id);
    let after_pool = pool(recorded.sim()).unwrap();
    let replay = recorded.finish();
    assert_eq!(
        Replay::decode(&replay.encode())
            .unwrap()
            .play(db.clone())
            .unwrap()
            .digest(),
        digest
    );

    if let Some(path) = std::env::var_os("STA64_RECREW_REPLAY_OUT") {
        let path = PathBuf::from(path);
        assert!(
            path.is_absolute(),
            "STA64_RECREW_REPLAY_OUT must be absolute"
        );
        let parent = path
            .parent()
            .unwrap()
            .canonicalize()
            .expect("replay parent exists");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        assert!(
            !parent.starts_with(&repo),
            "replay and expectation must stay outside the repository"
        );
        let expect_name = format!(
            "{}.expect.json",
            path.file_name().unwrap().to_string_lossy()
        );
        let expect_path = parent.join(expect_name);
        assert!(
            !path.exists() && !expect_path.exists(),
            "refusing to overwrite a replay or expectation"
        );
        let json_string = |s: &str| {
            let mut out = String::from("\"");
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        };
        let expectation = format!(
            "{{\n  \"install\": {},\n  \"mod\": {},\n  \"station_odf\": {},\n  \"content_hash\": \"{:#018x}\",\n  \"lineage\": {},\n  \"tick\": {},\n  \"digest\": \"{:#018x}\",\n  \"entity_id\": {},\n  \"before_unit_crew_bits\": {},\n  \"before_pool_bits\": {},\n  \"after_unit_crew_bits\": {},\n  \"after_pool_bits\": {},\n  \"control_after_unit_crew_bits\": {},\n  \"control_after_pool_bits\": {}\n}}\n",
            json_string(&Path::new(&root).to_string_lossy()),
            active
                .as_deref()
                .map(&json_string)
                .unwrap_or_else(|| "null".into()),
            json_string(&station_name),
            db.hash(),
            json_string(db.lineage()),
            tick,
            digest,
            id.0,
            before_unit.to_bits(),
            before_pool.to_bits(),
            after_unit.to_bits(),
            after_pool.to_bits(),
            crew(&control, id).to_bits(),
            pool(&control).unwrap().to_bits(),
        );
        let mut replay_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("new replay output");
        let mut expect_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&expect_path)
            .expect("new expectation output");
        replay_file
            .write_all(&replay.encode())
            .expect("write replay");
        expect_file
            .write_all(expectation.as_bytes())
            .expect("write expectation");
        eprintln!(
            "recrew replay: {}\nexpectation: {}",
            path.display(),
            expect_path.display()
        );
    }
}
