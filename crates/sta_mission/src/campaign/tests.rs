//! Asset-free campaign tests: a synthetic install in memory, `fake::db()`.

use super::*;

const MSHELL_A2: &str = "// drills\r\nsyn_tut1.bzn\r\n-\r\n// first fleet\r\nsyn_fed01.bzn // opener\r\nsyn_fed02.bzn\r\nsyn_fed03.bzn\r\n-\r\nsyn_kl01.bzn\r\n-\r\n-\r\n";

const LABELS: &str = "single_player {\n\t+ federation_incomplete\t{Finish the first fleet}\n\t+ klingon_incomplete {}\n\t+ tutorials {Drills}\n\t+ federation_campaign {First Fleet}\n\t+ klingon_campaign {Second Fleet}\n}\nmission_select {\n\t+ syn_fed01.bzn\t{First Light}\n\t+ syn_fed02.bzn\t{Second Wind}\n}\n";

const WIN_SOON: &str = r#"
Objectives:load("SynF1_Objectives.txt")
MMM.register(function(t)
  if t >= 1 then Mission:finish(true, 2) end
end)
"#;

const LOSE_SOON: &str = "MMM.register(function(t) if t >= 0.5 then Mission:finish(false, 0) end end)\n";

const OBJECTIVES: &str = "First Light\r\nThe synthetic fleet gathers.\r\n*  Survive one second\r\n*  Win\r\n";

fn source() -> MemSource {
    MemSource::new()
        .with("mshell.set", MSHELL_A2)
        .with("label.map", LABELS)
        .with("ART_CFG.h", "int GIVE_ALL_MISSIONS = 0; // off\r\n")
        .with("bzn/syn_tut1.bzn", "not parsed")
        .with("bzn/syn_fed01.bzn", "not parsed")
        .with("bzn/syn_fed02.bzn", "not parsed")
        .with("bzn/syn_fed03.bzn", "not parsed")
        .with("bzn/syn_kl01.bzn", "not parsed")
        .with("missions/syn_fed01.lua", WIN_SOON)
        .with("missions/syn_fed02.lua", LOSE_SOON)
        .with("missions/syn_fed03.lua", "MMM.register(function(t) if t >= 0.2 then Mission.finish(true) end end)\n")
        .with("missions/syn_kl01.lua", "-- nothing ends this mission\n")
        .with("objectives/SynF1_Objectives.txt", OBJECTIVES)
}

/// `fake::db()` on an empty map, two seats; scripts read from the source.
struct FakeHost(Arc<MemSource>);

impl MissionHost for FakeHost {
    fn game_setup(&self, _c: &Campaign, _m: &CampaignMission, seed: u64) -> Result<GameSetup, String> {
        Ok(GameSetup {
            seed,
            map: Arc::new(MapDef::empty()),
            content: Arc::new(sta_content::fake::db()),
            players: (0..2)
                .map(|id| PlayerSetup {
                    id: PlayerId(id),
                    team: TeamId(id),
                    race: "fx".into(),
                    control: if id == 0 { Control::Human } else { Control::Idle },
                })
                .collect(),
            options: GameOptions::default(),
        })
    }

    fn script(&self, _c: &Campaign, m: &CampaignMission) -> Result<(String, ScriptReader), String> {
        let src = self.0.clone();
        resolved_script(m, Arc::new(move |p| src.read(p)))
    }

    fn aip_library(&self) -> sta_ai::AipLibrary {
        let mut lib = sta_ai::AipLibrary::new();
        lib.insert("main.aip", sta_content::Aip::parse("main.aip", "int min_attack_force = 4;\n").0);
        lib
    }
}

#[test]
fn a_script_loading_aips_reaches_the_hosts_library() {
    let src = Arc::new(source().with(
        "missions/syn_fed01.lua",
        "Player(1):loadAIP('MAIN.AIP')\nPlayer(1):loadAIP('nothing.aip')\nMMM.register(function(t) if t >= 1 then Mission:finish(true, 0) end end)\n",
    ));
    let book = CampaignBook::load(&*src, ShellLayout::Armada2);
    let host = FakeHost(src);
    let mut r = CampaignRunner::new(book);
    let a = r.start(&host, "federation", 0, 1).unwrap();
    a.run(10 * TICK_HZ_U64).expect("ends");
    assert_eq!(a.unresolved_aips(), [(PlayerId(1), "nothing.aip".to_owned())]);
    assert!(a.runtime().unsupported().is_empty());
}

/// LUA-20: a switch the script throws on a tick applies on that same `ActiveMission::tick`, in call
/// order with the plan loads of the tick, before the seat thinks; and a mission with only switches
/// lists nothing as unsupported once the runner has taken them.
#[test]
fn ai_switches_apply_on_the_tick_the_script_throws_them() {
    // Tick 1+: load (enables), disable all, then on a later tick enable team 1 and re-disable.
    let script = "FIRED = 0\nMMM.register(function(t)\n\
        if t >= 1 and FIRED == 0 then FIRED = 1 Player(1):loadAIP('MAIN.AIP') A2.DisableAllAITeams() end\n\
        if t >= 2 and FIRED == 1 then FIRED = 2 A2.EnableTeam(1) end\n\
        if t >= 3 and FIRED == 2 then FIRED = 3 A2.DisableTeam(1) A2.EnableAllAITeams() A2.DisableTeam(1) A2.ResetStrategyCycle() A2.EnableTeam(0) end\n\
        if t >= 4 then Mission:finish(true, 0) end\nend)\n";
    let src = Arc::new(source().with("missions/syn_fed01.lua", script));
    let book = CampaignBook::load(&*src, ShellLayout::Armada2);
    let host = FakeHost(src);
    let mut r = CampaignRunner::new(book);
    let a = r.start(&host, "federation", 0, 1).unwrap();
    let p1 = PlayerId(1);
    assert!(a.ai_hub().is_ai(p1) && !a.ai_hub().is_enabled(p1), "a computer seat starts ai, disabled");
    let fired = |a: &ActiveMission| a.runtime().eval::<i64>("return FIRED").unwrap();
    let mut seen = Vec::new();
    for _ in 0..(10 * TICK_HZ_U64) {
        let before = fired(a);
        a.tick();
        let after = fired(a);
        if after != before {
            // The effect of this tick's calls is already in the hub when tick() returns.
            seen.push((after, a.ai_hub().is_enabled(p1)));
        }
        if a.outcome().is_some() {
            break;
        }
    }
    assert_eq!(seen, [(1, false), (2, true), (3, false)], "load+disable, then enable, then the last of its calls wins");
    assert!(a.ai_hub().unresolved().is_empty(), "the human seat's EnableTeam(0) is the original's no-op");
    assert!(a.runtime().unsupported().is_empty(), "{:?}", a.runtime().unsupported());
}

#[test]
fn mshell_groups_per_layout() {
    assert_eq!(
        parse_mshell(MSHELL_A2, ShellLayout::Armada2).0,
        vec![vec!["syn_tut1.bzn"], vec!["syn_fed01.bzn", "syn_fed02.bzn", "syn_fed03.bzn"], vec!["syn_kl01.bzn"], vec![]]
    );
    let a1 = "f1.bzn\r\nf2.bzn\r\nnone\r\nnone\r\nk1.bzn\r\nnone \r\nnone\r\nb1.bzn\r\nnone";
    let (groups, warn) = parse_mshell(a1, ShellLayout::Armada1);
    assert_eq!(groups, vec![vec!["f1.bzn", "f2.bzn"], vec!["k1.bzn"], vec!["b1.bzn"]]);
    assert!(warn.is_some(), "9 lines do not make 5 equal slots");
    // Five slots of three lines; the empty second slot keeps its place.
    let a1 = "f1.bzn\r\nf2.bzn\r\nnone\r\nnone\r\nnone\r\nnone\r\nr1.bzn\r\nnone\r\nnone\r\nb1.bzn\r\nb2.bzn\r\nb3.bzn\r\nx1.bzn\r\nnone\r\nnone";
    let (groups, warn) = parse_mshell(a1, ShellLayout::Armada1);
    assert_eq!(warn, None);
    assert_eq!(groups, vec![vec!["f1.bzn", "f2.bzn"], vec![], vec!["r1.bzn"], vec!["b1.bzn", "b2.bzn", "b3.bzn"], vec!["x1.bzn"]]);
    assert_eq!(parse_mshell("-\r\n-\r\n-\r\n-\r\n", ShellLayout::Armada2).0, vec![Vec::<String>::new(); 4]);
}

#[test]
fn book_reads_names_titles_scripts_and_briefings() {
    let book = CampaignBook::load(&source(), ShellLayout::Armada2);
    assert!(book.warnings.is_empty(), "{:?}", book.warnings);
    assert!(!book.give_all_missions);
    let ids: Vec<_> = book.campaigns.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["tutorials", "federation", "klingon", "borg"]);
    let fed = book.campaign("federation").unwrap();
    assert_eq!(fed.name.as_deref(), Some("First Fleet"));
    assert_eq!(fed.requires, None);
    let kl = book.campaign("klingon").unwrap();
    assert_eq!(kl.requires.as_deref(), Some("federation"));
    assert_eq!(kl.locked_text.as_deref(), Some("Finish the first fleet"));
    assert_eq!(book.campaign("borg").unwrap().locked_text, None, "an empty label is no text");
    let m1 = &fed.missions[0];
    assert_eq!((m1.index, m1.stem(), m1.title.as_deref()), (0, "syn_fed01", Some("First Light")));
    assert_eq!(m1.script, MissionScript::Lua { path: "missions/syn_fed01.lua".into() });
    assert_eq!(m1.objective_files, ["SynF1_Objectives.txt"]);
    let b = m1.briefing.as_ref().expect("briefing");
    assert_eq!((b.title.as_str(), b.text.as_str()), ("First Light", "The synthetic fleet gathers."));
    assert_eq!(b.goals, ["Survive one second", "Win"]);
    assert_eq!(m1.end_calls, ["Mission:finish"]);
    assert_eq!(fed.missions[2].end_calls, ["Mission.finish"]);

    let blocked = book.blocked();
    let report: Vec<_> = blocked.iter().map(|b| (b.campaign.as_str(), b.bzn.as_str())).collect();
    assert_eq!(report, [("tutorials", "syn_tut1.bzn"), ("klingon", "syn_kl01.bzn")]);
    assert!(blocked[0].reason.contains("no mission script"), "{}", blocked[0].reason);
    assert!(blocked[1].reason.contains("no win event"), "{}", blocked[1].reason);
}

#[test]
fn legacy_dll_missions_are_reported_with_their_end_calls() {
    let src = MemSource::new()
        .with("mshell.set", "x1.bzn\r\n-\r\n")
        .with("bzn/x1.bzn", "not parsed")
        .with("missions/x1S.dsl", b"MZ\0\0x1_A.txt\0x1_intro.bik\0SucceedMission\0FailMission\0".to_vec());
    // The resolver only sees DLLs the map names; give the source a parsed map
    // instead by asking for the DLL through the Map's scripts.
    let map = sta_bzn::Map { scripts: vec!["x1S.dsl".into()], ..Default::default() };
    let r = crate::resolve_with(&|p| src.exists(p), &[], "x1.bzn", &map);
    assert_eq!(r, Resolution::LegacyDll { files: vec!["x1S.dsl".into()] });
    assert_eq!(file_names(&src.read("missions/x1S.dsl").unwrap()), ["x1_A.txt", "x1_intro.bik"]);
    let m = CampaignMission {
        index: 0,
        bzn: "x1.bzn".into(),
        title: None,
        map_present: true,
        script: MissionScript::LegacyDll { files: vec!["x1S.dsl".into()] },
        objective_files: vec![],
        briefing: None,
        movies: vec![],
        end_calls: vec!["SucceedMission".into(), "FailMission".into()],
    };
    let why = blocked_reason(ShellLayout::Armada2, &m).unwrap();
    assert!(why.contains("LUA-05") && why.contains("SucceedMission"), "{why}");
    assert!(blocked_reason(ShellLayout::Armada1, &m).unwrap().contains("LUA-06"));
}

#[test]
fn includes_and_dll_pairs_are_scanned() {
    assert_eq!(includes(br#"include("A_helper.lua") include 'B.lua' x.include = 1"#), ["A_helper.lua", "B.lua"]);
    assert_eq!(legacy_files(&["m7r.drl".into()]), ["m7r.drl", "m7S.dsl"]);
    let src = MemSource::new()
        .with("missions/M.lua", "include(\"M_helper.lua\")\n")
        .with("missions/M_helper.lua", "include(\"M.lua\") Objectives:load(\"M_Objectives1.txt\") Mission:finish(true, 0)\n");
    let texts = lua_texts(&src, "missions/M.lua");
    assert_eq!(texts.len(), 2, "the cycle back to M.lua is not followed");
}

#[test]
fn lua_string_literals_name_files() {
    let lua = br#"Objectives:load( "Co_Objectives1.txt" ) UI:play('intro movie.bik') x = "a.lua""#;
    assert_eq!(file_names(lua), ["Co_Objectives1.txt", "intro movie.bik"]);
}

#[test]
fn unlock_chain_and_stock_counter_rule() {
    let book = CampaignBook::load(&source(), ShellLayout::Armada2);
    let mut r = CampaignRunner::new(book);
    assert_eq!(r.open_campaigns(), ["tutorials", "federation"], "borg is empty, klingon locked");
    assert_eq!(r.progress.available(&r.book, "federation"), 0..1);
    assert_eq!(r.record("federation", 0, true).unwrap(), Transition::Next { campaign: "federation".into(), mission: 1 });
    assert_eq!(r.progress.state("federation"), CampaignState { reached: 1, complete: false });
    // Replaying an earlier mission moves nothing; a loss moves nothing.
    assert_eq!(r.record("federation", 0, true).unwrap(), Transition::Next { campaign: "federation".into(), mission: 1 });
    assert_eq!(r.record("federation", 1, false).unwrap(), Transition::Lost { campaign: "federation".into(), mission: 1 });
    assert_eq!(r.progress.state("federation").reached, 1);
    r.record("federation", 1, true).unwrap();
    assert_eq!(
        r.record("federation", 2, true).unwrap(),
        Transition::CampaignComplete { campaign: "federation".into(), unlocked: vec!["klingon".into()] }
    );
    assert_eq!(r.progress.state("federation"), CampaignState { reached: 2, complete: true });
    assert!(r.progress.is_open(&r.book, "klingon"));
}

#[test]
fn record_rejects_unknown_campaigns_and_missions() {
    let mut r = CampaignRunner::new(CampaignBook::load(&source(), ShellLayout::Armada2));
    assert_eq!(r.record("nope", 0, true), Err(StartError::UnknownCampaign("nope".into())));
    assert_eq!(r.record("federation", 3, true), Err(StartError::NoSuchMission { campaign: "federation".into(), mission: 3 }));
    assert_eq!(r.progress.state("federation"), CampaignState::default());
}

#[test]
fn oversized_counters_are_clamped_not_overflowed() {
    let book = CampaignBook::load(&source(), ShellLayout::Armada2);
    let mut p = CampaignProgress::new(&book);
    p.campaigns.insert("federation".into(), CampaignState { reached: usize::MAX, complete: false });
    assert_eq!(p.available(&book, "federation"), 0..3);
    let r = CampaignRunner::with_progress(book, p);
    assert_eq!(r.progress.state("federation").reached, 2);
}

#[test]
fn stem_survives_multibyte_names() {
    let m = |bzn: &str| CampaignMission {
        index: 0,
        bzn: bzn.into(),
        title: None,
        map_present: false,
        script: MissionScript::None,
        objective_files: vec![],
        briefing: None,
        movies: vec![],
        end_calls: vec![],
    };
    assert_eq!(m("dir/M01.BZN").stem(), "M01");
    assert_eq!(m("\u{e9}\u{e9}").stem(), "\u{e9}\u{e9}");
    assert_eq!(m("ab\u{e9}").stem(), "ab\u{e9}");
}

#[test]
fn give_all_missions_opens_everything() {
    let src = source().with("ART_CFG.h", "int GIVE_ALL_MISSIONS = 1;\r\n");
    let book = CampaignBook::load(&src, ShellLayout::Armada2);
    assert!(book.give_all_missions);
    let p = CampaignProgress::new(&book);
    assert_eq!(p.state("federation"), CampaignState { reached: 2, complete: true });
    assert_eq!(p.state("borg"), CampaignState { reached: 0, complete: false }, "an empty campaign stays empty");
    assert_eq!(p.available(&book, "klingon"), 0..1);
}

#[test]
fn progress_round_trips_byte_for_byte() {
    let book = CampaignBook::load(&source(), ShellLayout::Armada2);
    let mut r = CampaignRunner::new(book);
    r.record("federation", 0, true).unwrap();
    r.progress.carried.insert("fleet name".into(), "Task Force 7 / 100%".into());
    r.progress.carried.insert("empty".into(), String::new());
    let text = r.progress.save();
    assert!(text.starts_with("sta64-campaign-progress 1\n"), "{text}");
    let back = CampaignProgress::restore_for(&r.book, &text).unwrap();
    assert_eq!(back, r.progress);
    assert_eq!(back.save(), text);

    assert_eq!(CampaignProgress::restore("sta64-campaign-progress 2\nend\n"), Err(ProgressError::Version(2)));
    assert!(matches!(CampaignProgress::restore("hello"), Err(ProgressError::Format(1, _))));
    assert!(matches!(CampaignProgress::restore(&text.replace("end\n", "")), Err(ProgressError::Format(..))));
    for (from, to) in [
        ("campaign federation 1 0", "campaign federation 1 2"),
        ("campaign federation 1 0", "campaign federation +1 0"),
        ("campaign federation 1 0", "campaign federation 01 0"),
        ("campaign federation 1 0", "campaign Federation%41 1 0"),
        ("fleet%20name", "fleet%20name%2"),
    ] {
        assert!(matches!(CampaignProgress::restore(&text.replace(from, to)), Err(ProgressError::Format(..))), "{to}");
    }
    let upper = text.replacen(&format!("{:016x}", r.progress.book), &format!("{:016X}", r.progress.book), 1);
    if upper != text {
        assert!(matches!(CampaignProgress::restore(&upper), Err(ProgressError::Format(..))));
    }
    let other = CampaignBook::load(&source().with("mshell.set", "a.bzn\r\n-\r\n"), ShellLayout::Armada2);
    assert!(matches!(CampaignProgress::restore_for(&other, &text), Err(ProgressError::Book { .. })));
}

#[test]
fn runner_plays_mission_one_to_its_win_and_selects_mission_two() {
    let src = Arc::new(source());
    let book = CampaignBook::load(&*src, ShellLayout::Armada2);
    let host = FakeHost(src);
    let mut r = CampaignRunner::new(book);
    assert!(matches!(r.start(&host, "klingon", 0, 1), Err(StartError::Locked { .. })));
    assert!(matches!(r.start(&host, "federation", 1, 1), Err(StartError::Locked { .. })));
    assert!(matches!(r.start(&host, "tutorials", 0, 1), Err(StartError::Blocked(_))));
    r.start(&host, "federation", 0, 7).unwrap();
    assert!(matches!(r.start(&host, "federation", 0, 7), Err(StartError::Busy)));
    assert_eq!(r.abandon(), Some(Transition::Lost { campaign: "federation".into(), mission: 0 }));

    let active = r.start(&host, "federation", 0, 7).unwrap();
    let briefing = active.runtime().objective_briefing().expect("script loaded its objectives");
    assert_eq!(briefing.title, "First Light");
    let outcome = active.run(10 * TICK_HZ_U64).expect("mission 1 ends");
    assert!(outcome.won);
    assert_eq!(outcome.delay, 2.0);
    assert_eq!(outcome.tick, TICK_HZ_U64, "Mission:finish at t >= 1 s");
    assert!(active.runtime().errors().is_empty(), "{:?}", active.runtime().errors());
    let next = r.finish(outcome).unwrap();
    assert_eq!(next, Transition::Next { campaign: "federation".into(), mission: 1 });
    assert!(r.active().is_none());

    // Mission 2 loses: back to the list, progress kept, and it saves.
    let saved = r.progress.save();
    let a = r.start(&host, "federation", 1, 7).unwrap();
    let lost = a.run(10 * TICK_HZ_U64).unwrap();
    assert!(!lost.won);
    assert_eq!(r.finish(lost), Some(Transition::Lost { campaign: "federation".into(), mission: 1 }));
    assert_eq!(r.progress.save(), saved);

    // Restored progress starts where it left off.
    let restored = CampaignProgress::restore_for(&r.book, &saved).unwrap();
    let mut r2 = CampaignRunner::with_progress(r.book.clone(), restored);
    assert!(r2.start(&host, "federation", 1, 7).is_ok());
}

#[test]
fn same_seed_same_outcome_tick() {
    let src = Arc::new(source());
    let run = || {
        let mut r = CampaignRunner::new(CampaignBook::load(&*src, ShellLayout::Armada2));
        let a = r.start(&FakeHost(src.clone()), "federation", 0, 3).unwrap();
        let o = a.run(100).unwrap();
        (o.tick, a.sim().digest())
    };
    assert_eq!(run(), run());
}

const TICK_HZ_U64: u64 = sta_sim::TICK_HZ as u64;

#[test]
fn outcome_follows_lua_truthiness_and_unhooked_finish_is_no_win() {
    let play = |script: &str| {
        let src = Arc::new(source().with("missions/syn_fed01.lua", script.to_owned()));
        let mut r = CampaignRunner::new(CampaignBook::load(&*src, ShellLayout::Armada2));
        let a = r.start(&FakeHost(src.clone()), "federation", 0, 1).unwrap();
        a.run(10).expect("mission ends")
    };
    assert!(play("MMM.register(function() Mission:finish(1) end)\n").won);
    assert!(!play("MMM.register(function() Mission:finish(nil) end)\n").won);
    // A reference kept before the hook: the runtime's finished flag still ends it, as no win.
    assert!(!play("local finish = Mission.finish\nMMM.register(function() finish(Mission, true) end)\n").won);
}

fn run_end(script: &str, ticks: u64) -> (Option<MissionOutcome>, MissionEndState) {
    let src = Arc::new(source().with("missions/syn_fed01.lua", script));
    let book = CampaignBook::load(&*src, ShellLayout::Armada2);
    let host = FakeHost(src);
    let mut r = CampaignRunner::new(book);
    let a = r.start(&host, "federation", 0, 1).unwrap();
    let out = a.run(ticks);
    let state = a.runtime().mission_end(a.sim().view().tick);
    (out, state)
}

#[test]
fn a2_end_calls_end_the_mission_once_time_is_past_their_time() {
    let (won, _) = run_end("MMM.register(function(t) if t >= 1 then A2.SucceedMission(2.0, 0) end end)\n", 300);
    let won = won.expect("ends");
    assert!(won.won);
    assert_eq!(won.delay, 0.0);
    assert_eq!(won.tick, 61, "the first tick with mission time past 2.0 s");
    let (lost, _) = run_end("MMM.register(function(t) if t >= 1 then A2.FailMission(1.0, nil) end end)\n", 300);
    let lost = lost.expect("ends");
    assert!(!lost.won);
    assert_eq!(lost.tick, 31);
}

#[test]
fn the_last_end_call_wins_until_the_mission_has_ended() {
    let s = "A2.SucceedMission(1.0, 0)\nMMM.register(function(t) if t >= 0.5 then A2.FailMission(1.0, 'x') end end)\n";
    let (o, _) = run_end(s, 300);
    assert!(!o.unwrap().won);
}

#[test]
fn an_end_resting_on_a_gap_is_blocked_not_a_loss() {
    // A handle-producing call that was a gap, then a loss decided on the
    // handle it should have given: undecidable.
    let s = "A2.NoteGap('BuildObject', 'x')\nlocal done\nMMM.register(function(t) if t >= 1 and not done and not A2.IsAlive(0) then done = true; A2.FailMission(A2.GetTime(), 0) end end)\n";
    let (o, state) = run_end(s, 300);
    assert!(o.is_none(), "no outcome");
    match state {
        MissionEndState::Blocked(why) => assert!(why.contains("IsAlive") && why.contains("never existed"), "{why}"),
        other => panic!("{other:?}"),
    }
    // The same query with no gap stands as the mission's own.
    let s = "local done\nMMM.register(function(t) if t >= 1 and not done and not A2.IsAlive(0) then done = true; A2.FailMission(A2.GetTime(), 0) end end)\n";
    assert!(!run_end(s, 300).0.unwrap().won);
    // And an end with no query at all.
    let s = "local done\nMMM.register(function(t) if t >= 1 and not done then done = true; A2.FailMission(A2.GetTime(), 0) end end)\n";
    assert!(!run_end(s, 300).0.unwrap().won);
}

#[test]
fn a_script_with_a2_end_calls_has_a_win_event() {
    let src = Arc::new(source().with("missions/syn_fed01.lua", "A2.SucceedMission(1, 0)\nA2.FailMission(1, 0)\n"));
    let book = CampaignBook::load(&*src, ShellLayout::Armada2);
    assert!(book.blocked().iter().all(|b| b.campaign != "federation" || b.index != 0), "{:?}", book.blocked());
}
