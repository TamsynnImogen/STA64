//! AI-06: a mission seat is passive until a script loads an AIP, and a loaded
//! file overlays the seat's plan.

use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

use super::*;

const P0: PlayerId = PlayerId(0);

fn setup() -> GameSetup {
    let seat = |id: u8| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    GameSetup {
        seed: 3,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(sta_content::fake::db()),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    }
}

fn library() -> AipLibrary {
    let files = [
        ("AI/AIPs/Build.aip", "int min_resource_force = 2;\nint min_attack_force = 3;\nint min_defense_force = 1;\n\
            Build_List_Element the_build_list[MAX_ELEMENTS] =\n{\n\"fx_miner\", 2,\n};\n"),
        ("AI/AIPs/aggressive.aip", "int min_attack_force = 7;\n"),
        ("AI/AIPs/other_list.aip", "Build_List_Element the_build_list[MAX_ELEMENTS] =\n{\n\"fx_frigate\", 1,\n};\n"),
    ];
    let names: Vec<String> = files.iter().map(|(n, _)| (*n).to_owned()).collect();
    AipLibrary::load(&names, &mut |n| files.iter().find(|(f, _)| *f == n).map(|(_, t)| (*t).to_owned()))
}

fn world(s: &GameSetup) -> Sim {
    let mut sim = Sim::from_setup(s);
    let key = |odf: &str| s.content.key(odf).unwrap();
    let mut batch = Vec::new();
    let mut seq = 0;
    for (i, st) in s.map.starts.iter().enumerate() {
        let p = PlayerId(i as u8);
        for (odf, dz) in [("fx_station", 0), ("fx_yard", 300)] {
            seq += 1;
            let pos = Vec3 { x: st.pos.x, y: st.pos.y, z: st.pos.z + Fixed::from_int(dz) };
            batch.push(Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(Edit::Spawn { odf: key(odf), player: p, pos, heading: st.heading }) });
        }
        seq += 1;
        batch.push(Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(Edit::Cargo(p, 0, Fixed::from_int(1000))) });
    }
    sim.step(&batch);
    sim
}

/// Commands the seat sent over `ticks`, loading `requests` (tick, file) as the script would.
fn run(requests: &[(u64, &str)], ticks: u64) -> (Vec<Command>, u64) {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let mut ai = hub.controller(&s, P0);
    let mut sim = world(&s);
    let (mut events, mut sent) = (Vec::<SimEvent>::new(), Vec::new());
    for t in 0..ticks {
        hub.submit(requests.iter().filter(|(rt, _)| *rt == t).map(|(_, n)| (P0, (*n).to_owned())));
        let view = sim.view();
        let mut out = Vec::new();
        ai.think(&view, &events, &mut out);
        sent.extend(out.iter().cloned());
        events = sim.step(&out).events;
    }
    (sent, sim.digest())
}

#[test]
fn a_seat_does_nothing_until_its_first_load() {
    let (none, _) = run(&[], 900);
    assert!(none.is_empty());
    let (late, _) = run(&[(600, "build.aip")], 900);
    assert!(!late.is_empty(), "it plays once a plan is loaded");
    assert!(late.iter().all(|c| matches!(c.kind, CommandKind::Order(_)) && c.player == P0));
}

#[test]
fn unknown_files_change_nothing_and_are_listed() {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let mut ai = hub.controller(&s, P0);
    hub.submit([(P0, "nope.aip".to_owned())]);
    let mut out = Vec::new();
    ai.think(&world(&s).view(), &[], &mut out);
    assert!(out.is_empty());
    assert_eq!(hub.unresolved(), [(P0, "nope.aip".to_owned())]);
}

#[test]
fn files_are_found_by_name_without_case_or_path() {
    let lib = library();
    assert!(lib.get("BUILD.AIP").is_some());
    assert!(lib.get("ai\\aips\\aggressive.aip").is_some());
    assert!(lib.get("missing.aip").is_none());
    assert_eq!(lib.len(), 3);
}

#[test]
fn a_loaded_file_overlays_and_a_list_replaces_only_when_present() {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let mut seat = ScriptedAi {
        seat: P0,
        hub: hub.clone(),
        params: SkirmishConfig::from_setup(&s, P0).unwrap().params,
        ai: SkirmishController::new(SkirmishConfig::from_setup(&s, P0).unwrap()),
        loaded: false,
    };
    let view = world(&s).view();
    lock(&hub.shared).seats.entry(P0).or_default().flags = Some(Flags { ai: true, enabled: false });
    hub.submit([(P0, "build.aip".into()), (P0, "aggressive.aip".into())]);
    seat.think(&view, &[], &mut Vec::new());
    let p = seat.params().clone();
    assert_eq!((p.miners, p.min_attack, p.defence), (2, 7, 1), "only the key the second file sets changed");
    assert_eq!(p.build_list.len(), 1);
    assert_eq!(p.build_list[0].unit, "fx_miner");
    hub.submit([(P0, "other_list.aip".into())]);
    seat.think(&view, &[], &mut Vec::new());
    assert_eq!(seat.params().build_list[0].unit, "fx_frigate");
    assert_eq!(seat.params().min_attack, 7, "the keys stay");
    assert!(seat.has_plan());
}

#[test]
fn requests_for_seats_without_ai_are_reported_not_applied() {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let mut ai = hub.controller(&s, P0);
    hub.submit([(PlayerId(1), "build.aip".to_owned())]);
    let mut out = Vec::new();
    ai.think(&world(&s).view(), &[], &mut out);
    assert!(out.is_empty());
    assert_eq!(hub.unresolved(), [(PlayerId(1), "build.aip".to_owned())]);
}

#[test]
fn same_requests_give_the_same_game() {
    let a = run(&[(30, "build.aip"), (300, "aggressive.aip")], 1500);
    let b = run(&[(30, "build.aip"), (300, "aggressive.aip")], 1500);
    assert!(!a.0.is_empty());
    assert_eq!(a.1, b.1);
    assert_eq!(format!("{:?}", a.0), format!("{:?}", b.0));
}

#[test]
fn seats_without_ai_are_reported_and_never_queue() {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let _ai = hub.controller(&s, P0);
    hub.submit([(PlayerId(1), "build.aip".to_owned()), (PlayerId(9), "build.aip".to_owned())]);
    assert_eq!(hub.unresolved().len(), 2, "no AI controller was made for seat 1 or 9");
}

#[test]
fn includes_resolve_by_name_in_the_including_folder_first() {
    let files = [
        ("AI/AIPs/base.aip", "int min_attack_force = 5;\n"),
        ("AI/AIPs/sub/base.aip", "int min_attack_force = 8;\n"),
        ("AI/AIPs/child.aip", "#include \"base.aip\"\nint min_resource_force = 3;\n"),
        ("AI/AIPs/sub/child2.aip", "#include \"base.aip\"\n"),
    ];
    let names: Vec<String> = files.iter().map(|(n, _)| (*n).to_owned()).collect();
    let lib = AipLibrary::load(&names, &mut |n| files.iter().find(|(f, _)| *f == n).map(|(_, t)| (*t).to_owned()));
    assert_eq!(lib.get("child.aip").unwrap().int("min_attack_force"), Some(5));
    assert_eq!(lib.get("child2.aip").unwrap().int("min_attack_force"), Some(8));
}

#[test]
fn a_max_below_the_min_does_not_panic_and_seq_stays_clear() {
    let s = setup();
    let mut c = SkirmishController::new(SkirmishConfig::from_setup(&s, P0).unwrap());
    c.set_params(Params { min_attack: 5, max_attack: 2, ..Params::default() });
    assert_eq!(c.params().max_attack, 5);
    let hub = MissionAiHub::new(library());
    let mut ai = hub.controller(&s, P0);
    hub.submit([(P0, "build.aip".to_owned())]);
    let (mut sim, mut out) = (world(&s), Vec::new());
    for _ in 0..60 {
        out.clear();
        ai.think(&sim.view(), &[], &mut out);
        sim.step(&out);
        if !out.is_empty() {
            break;
        }
    }
    assert!(!out.is_empty() && out.iter().all(|c| c.seq >= AI_SEQ_BASE));
}

// ---- LUA-20: the AI switches ----

const P1: PlayerId = PlayerId(1);

/// Ticks and commands of seat 0 over `ticks`, applying `requests` (tick, request) before the
/// seat thinks on that tick, as the hosts do (script first, then the seats).
fn run_requests(requests: &[(u64, AiRequest)], ticks: u64) -> (Vec<(u64, Command)>, u64) {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let mut ai = hub.controller(&s, P0);
    let mut sim = world(&s);
    let (mut events, mut sent) = (Vec::<SimEvent>::new(), Vec::new());
    for t in 0..ticks {
        hub.submit_requests(requests.iter().filter(|(rt, _)| *rt == t).map(|(_, r)| r.clone()));
        let view = sim.view();
        let mut out = Vec::new();
        ai.think(&view, &events, &mut out);
        sent.extend(out.iter().map(|c| (t, c.clone())));
        events = sim.step(&out).events;
    }
    (sent, sim.digest())
}

fn load(file: &str) -> AiRequest {
    AiRequest::Load(P0, file.to_owned())
}

#[test]
fn flags_start_ai_and_disabled_and_a_load_enables() {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let _a = hub.controller(&s, P0);
    let _b = hub.controller(&s, P1);
    assert!([P0, P1].iter().all(|&p| hub.is_ai(p) && !hub.is_enabled(p)));
    hub.submit_requests([load("build.aip")]);
    assert!(hub.is_enabled(P0) && !hub.is_enabled(P1), "the load enabled its own seat only");
    hub.submit_requests([AiRequest::Disable(P0), AiRequest::Enable(P1)]);
    assert!(!hub.is_enabled(P0) && hub.is_enabled(P1), "per seat; Enable needs no plan");
    hub.submit_requests([AiRequest::EnableAll]);
    assert!(hub.is_enabled(P0) && hub.is_enabled(P1));
    hub.submit_requests([AiRequest::DisableAll]);
    assert!(!hub.is_enabled(P0) && !hub.is_enabled(P1));
    assert!(hub.unresolved().is_empty());
}

#[test]
fn the_human_seat_is_never_touched_by_a_switch() {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let _ai = hub.controller(&s, P0); // seat 1 is the human's: no controller
    hub.submit_requests([AiRequest::Enable(P1), AiRequest::EnableAll, AiRequest::SetAi(P1, true), AiRequest::Replan]);
    assert!(!hub.is_ai(P1) && !hub.is_enabled(P1), "no AI flag appears on the human seat");
    assert!(hub.is_enabled(P0), "EnableAll reached the computer seat");
    hub.submit_requests([AiRequest::Disable(P1), AiRequest::DisableAll, AiRequest::SetAi(P1, false)]);
    assert!(!hub.is_ai(P1) && !hub.is_enabled(P1));
    assert!(hub.unresolved().is_empty(), "switches on it are the original's no-ops, not unresolved loads");
    // A plan load for it is still reported, as before.
    hub.submit_requests([AiRequest::Load(P1, "build.aip".into())]);
    assert_eq!(hub.unresolved().len(), 1);
}

#[test]
fn set_ai_on_a_computer_seat_is_forced_back() {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let _ai = hub.controller(&s, P0);
    hub.submit_requests([load("build.aip"), AiRequest::SetAi(P0, false)]);
    assert!(hub.is_ai(P0) && hub.is_enabled(P0));
    let plain = run_requests(&[(0, load("build.aip"))], 600);
    let forced = run_requests(&[(0, load("build.aip")), (0, AiRequest::SetAi(P0, false)), (300, AiRequest::SetAi(P0, true))], 600);
    assert!(!plain.0.is_empty());
    assert_eq!(format!("{:?}", plain.0), format!("{:?}", forced.0));
    assert_eq!(plain.1, forced.1);
}

#[test]
fn load_then_disable_idles_the_seat_and_disable_then_load_does_not() {
    let (a, _) = run_requests(&[(0, load("build.aip")), (0, AiRequest::Disable(P0))], 900);
    assert!(a.is_empty(), "loaded but disabled: no building, no waves: {a:?}");
    let (b, _) = run_requests(&[(0, AiRequest::Disable(P0)), (0, load("build.aip"))], 900);
    assert!(!b.is_empty(), "a load re-enables, so the later load wins");
    // The same through DisableAll, and a later Enable brings the plan back.
    let (c, _) = run_requests(&[(0, AiRequest::DisableAll), (0, load("build.aip")), (0, AiRequest::DisableAll), (450, AiRequest::EnableAll)], 900);
    assert!(!c.is_empty() && c.iter().all(|(t, _)| *t >= 450), "idle until the Enable at 450: {:?}", c.first());
}

#[test]
fn a_load_before_any_switch_keeps_its_old_behaviour() {
    let a = run(&[(0, "build.aip")], 600);
    let (b, digest) = run_requests(&[(0, load("build.aip"))], 600);
    assert_eq!(format!("{:?}", a.0), format!("{:?}", b.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>()));
    assert_eq!(a.1, digest);
}

#[test]
fn replan_only_moves_the_next_plan_earlier() {
    // Loop tick 20 is not a period tick (the world's own first step puts the view one ahead, so
    // the 15th-tick plans are on loop ticks 14, 29, ...): the load at 20 plans at 29 without a
    // reset and at 20 with one.
    let (slow, _) = run_requests(&[(20, load("build.aip"))], 400);
    let (fast, _) = run_requests(&[(20, load("build.aip")), (20, AiRequest::Replan)], 400);
    assert_eq!(slow.first().map(|(t, _)| *t), Some(29));
    assert_eq!(fast.first().map(|(t, _)| *t), Some(20));
    // A reset with nothing to plan changes nothing, and it is used up by one think.
    let (none, _) = run_requests(&[(0, AiRequest::Replan)], 100);
    assert!(none.is_empty());
    let (once, _) = run_requests(&[(20, load("build.aip")), (20, AiRequest::Replan)], 21);
    let (again, _) = run_requests(&[(20, load("build.aip")), (20, AiRequest::Replan)], 29);
    assert_eq!(once.len(), again.len(), "no further off-period plan after the reset's own");
}

#[test]
fn the_same_requests_give_the_same_commands_and_game() {
    let reqs = [
        (0, AiRequest::DisableAll),
        (10, load("build.aip")),
        (40, AiRequest::DisableAll),
        (200, AiRequest::EnableAll),
        (201, AiRequest::Replan),
        (300, load("aggressive.aip")),
        (500, AiRequest::Disable(P0)),
        (700, AiRequest::Enable(P0)),
    ];
    let a = run_requests(&reqs, 1500);
    let b = run_requests(&reqs, 1500);
    assert!(!a.0.is_empty());
    assert_eq!(format!("{:?}", a.0), format!("{:?}", b.0));
    assert_eq!(a.1, b.1);
}

#[test]
fn requests_apply_when_submitted_and_the_seat_sees_them_on_its_next_think() {
    // The hosts submit after the script thought and before the seats think. A request made
    // after the seat's think of a tick is seen from its next think. The twin run without the
    // DisableAll shows the seat would have planned at tick 30: the silence is the switch's.
    let s = setup();
    let planned_at_30 = |disable: bool| {
        let hub = MissionAiHub::new(library());
        let mut ai = hub.controller(&s, P0);
        let sim = world(&s);
        let mut view = sim.view();
        std::sync::Arc::make_mut(&mut view).tick = 15;
        hub.submit_requests([load("build.aip")]);
        let mut out = Vec::new();
        ai.think(&view, &[], &mut out);
        assert!(!out.is_empty(), "submitted before the think: acts in that think");
        if disable {
            hub.submit_requests([AiRequest::DisableAll]);
            assert!(!hub.is_enabled(P0), "applied at the submit");
        }
        std::sync::Arc::make_mut(&mut view).tick = 30;
        let mut out = Vec::new();
        ai.think(&view, &[], &mut out);
        !out.is_empty()
    };
    assert!(planned_at_30(false), "without the switch the seat plans again at tick 30");
    assert!(!planned_at_30(true), "submitted after the think: seen by the next one");
}

// ---- LUA-20 follow-up ----

/// Every command of the seats in `seats` over `ticks`, thinking them in the given order each tick
/// and stepping the sim with the commands sorted by (player, seq), as the hosts' routers do.
fn run_seats(requests: &[(u64, AiRequest)], ticks: u64, seats: [PlayerId; 2]) -> (Vec<(u64, Command)>, u64) {
    let s = setup();
    let hub = MissionAiHub::new(library());
    let mut ctl: Vec<_> = seats.iter().map(|&p| hub.controller(&s, p)).collect();
    let mut sim = world(&s);
    let (mut events, mut sent) = (Vec::<SimEvent>::new(), Vec::new());
    for t in 0..ticks {
        hub.submit_requests(requests.iter().filter(|(rt, _)| *rt == t).map(|(_, r)| r.clone()));
        let view = sim.view();
        let mut out = Vec::new();
        for c in &mut ctl {
            c.think(&view, &events, &mut out);
        }
        out.sort_by_key(|c| (c.player, c.seq));
        sent.extend(out.iter().map(|c| (t, c.clone())));
        events = sim.step(&out).events;
    }
    (sent, sim.digest())
}

#[test]
fn two_seats_give_the_same_commands_and_game_in_either_seat_order() {
    let reqs = [
        (0, AiRequest::Load(P0, "build.aip".into())),
        (0, AiRequest::Load(P1, "build.aip".into())),
        (200, AiRequest::Disable(P0)),
        (300, AiRequest::DisableAll),
        (400, AiRequest::Enable(P0)),
        (450, AiRequest::Replan),
        (600, AiRequest::EnableAll),
        (700, AiRequest::Load(P1, "aggressive.aip".into())),
    ];
    let a = run_seats(&reqs, 1200, [P0, P1]);
    let b = run_seats(&reqs, 1200, [P1, P0]);
    assert!(a.0.iter().any(|(_, c)| c.player == P0) && a.0.iter().any(|(_, c)| c.player == P1), "both seats played");
    assert_eq!(format!("{:?}", a.0), format!("{:?}", b.0), "the seat order changes no command");
    assert_eq!(a.1, b.1);
}

#[test]
fn a_reset_reaches_every_seat_in_the_same_think() {
    // The original staggers the replans (team t on the t-th tick after the call); here both seats
    // plan off-period in the think that follows the reset.
    let reqs = [(20, AiRequest::Load(P0, "build.aip".into())), (20, AiRequest::Load(P1, "build.aip".into())), (20, AiRequest::Replan)];
    let (sent, _) = run_seats(&reqs, 28, [P0, P1]);
    for p in [P0, P1] {
        let first = sent.iter().find(|(_, c)| c.player == p).map(|(t, _)| *t);
        assert_eq!(first, Some(20), "seat {p:?}");
    }
}

#[test]
fn a_reset_is_used_up_by_a_seat_that_is_disabled_or_has_no_plan() {
    // Without a reset the plan after a load at 20 waits for loop tick 29 (see the replan test).
    // A disabled seat consumes the reset, so the Enable one tick later brings no off-period plan.
    let (a, _) = run_requests(&[(5, load("build.aip")), (5, AiRequest::Disable(P0)), (20, AiRequest::Replan), (21, AiRequest::Enable(P0))], 60);
    assert_eq!(a.first().map(|(t, _)| *t), Some(29), "{:?}", a.first());
    // A seat with no plan consumes it as well: the later load plans on the period tick.
    let (b, _) = run_requests(&[(20, AiRequest::Replan), (21, load("build.aip"))], 60);
    assert_eq!(b.first().map(|(t, _)| *t), Some(29), "{:?}", b.first());
    // The same two with the reset after the enable / load do plan at once.
    let (c, _) = run_requests(&[(5, load("build.aip")), (5, AiRequest::Disable(P0)), (21, AiRequest::Enable(P0)), (21, AiRequest::Replan)], 60);
    assert_eq!(c.first().map(|(t, _)| *t), Some(21));
}

/// The specials fixture with a PD weapon on `fx_cloaker` (as the AI-04 tests have it).
fn pd_content() -> sta_content::ContentDb {
    use sta_content::{ContentDb, DefRef, Hardpoint};
    let base = sta_sim::specials::fixture::db();
    let mut features = base.features().clone();
    features.pd_energy = true;
    let mut b = ContentDb::builder().profile(base.profile()).features(features);
    for mut w in base.weapons().cloned() {
        if w.odf == "fx_pd" {
            w.energy_cost = Fixed::from_int(20);
        }
        b = b.weapon(w);
    }
    for mut u in base.units().cloned() {
        if u.odf == "fx_cloaker" {
            u.hardpoints[0] = Hardpoint { weapon: DefRef::new("fx_pd"), node: "hp01".into(), arc: None };
        }
        b = b.unit(u);
    }
    b.build().unwrap()
}

#[test]
fn a_loaded_but_disabled_seat_still_switches_point_defence_on_when_attacked() {
    use sta_sim::command::Order;
    let seat = |id: u8| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let s = GameSetup {
        seed: 9,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(pd_content()),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    };
    let db = Arc::clone(&s.content);
    let hub = MissionAiHub::new(library());
    let mut ai = hub.controller(&s, P0);
    let mut sim = Sim::from_setup(&s);
    let v = |x: i32, z: i32| Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) };
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let ship = sim.spawn_def(db.key("fx_cloaker").unwrap(), P0, v(2000, 2800), ahead).unwrap();
    let boat = sim.spawn_def(db.key("fx_cruiser").unwrap(), P1, v(2000, 2000), ahead).unwrap();
    hub.submit_requests([load("build.aip"), AiRequest::Disable(P0)]);
    assert!(hub.is_ai(P0) && !hub.is_enabled(P0));
    let (mut events, mut mine) = (Vec::<SimEvent>::new(), Vec::new());
    for t in 0..120u64 {
        let view = sim.view();
        let mut out = Vec::new();
        ai.think(&view, &events, &mut out);
        mine.extend(out.iter().filter(|c| c.player == P0).cloned());
        if t == 0 {
            out.push(Command { player: PlayerId::BENCH, seq: 9, kind: CommandKind::Edit(Edit::Engines(ship, false)) });
            out.push(Command { player: P1, seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![boat], target: ship }) });
        }
        events = sim.step(&out).events;
    }
    let specials = mine.iter().filter(|c| matches!(&c.kind, CommandKind::Order(Order::Special { units, .. }) if units == &[ship])).count();
    assert!(specials >= 1, "the unit-level AI of a disabled seat still defends itself: {mine:?}");
    assert!(
        mine.iter().all(|c| matches!(&c.kind, CommandKind::Order(Order::Special { .. }))),
        "and nothing else (no build, no wave, no mining): {mine:?}"
    );
}
