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
    lock(&hub.shared).seats.push(P0);
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
