//! Asset-free campaign tests: a synthetic install in memory, `fake::db()`.

use super::*;

const MSHELL_A2: &str = "// Misc. missions here: (and tutorial missions)\r\nsyn_tut1.bzn\r\n-\r\n// Fed missions here:\r\nsyn_fed01.bzn\r\nsyn_fed02.bzn\r\nsyn_fed03.bzn\r\n-\r\nsyn_kl01.bzn\r\n-\r\n-\r\n";

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
}

#[test]
fn mshell_groups_per_layout() {
    assert_eq!(
        parse_mshell(MSHELL_A2, ShellLayout::Armada2),
        vec![vec!["syn_tut1.bzn"], vec!["syn_fed01.bzn", "syn_fed02.bzn", "syn_fed03.bzn"], vec!["syn_kl01.bzn"], vec![]]
    );
    let a1 = "f1.bzn\r\nf2.bzn\r\nnone\r\nnone\r\nk1.bzn\r\nnone \r\nnone\r\nb1.bzn\r\nnone";
    assert_eq!(parse_mshell(a1, ShellLayout::Armada1), vec![vec!["f1.bzn", "f2.bzn"], vec!["k1.bzn"], vec!["b1.bzn"]]);
    assert_eq!(parse_mshell("-\r\n-\r\n-\r\n-\r\n", ShellLayout::Armada2), vec![Vec::<String>::new(); 4]);
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
        .with("missions/x1S.dsl", b"MZ\0\0x1_A.txt\0x1_intro.bik\0?SucceedMission@@YAXMPAD@Z\0?FailMission@@YAXMPAD@Z\0".to_vec());
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
    assert_eq!(includes(br#"include("A_Monitor.lua") include 'B.lua' x.include = 1"#), ["A_Monitor.lua", "B.lua"]);
    assert_eq!(legacy_files(&["a2_Tutorial1r.drl".into()]), ["a2_Tutorial1r.drl", "a2_Tutorial1S.dsl"]);
    let src = MemSource::new()
        .with("missions/M.lua", "include(\"M_Monitor.lua\")\n")
        .with("missions/M_Monitor.lua", "include(\"M.lua\") Objectives:load(\"M_Objectives1.txt\") Mission:finish(true, 0)\n");
    let texts = lua_texts(&src, "missions/M.lua");
    assert_eq!(texts.len(), 2, "the cycle back to M.lua is not followed");
}

#[test]
fn lua_string_literals_name_files() {
    let lua = br#"Objectives:load( "Co_Objectives1.txt" ) UI:play('intro movie.bik') x = "a.lua""#;
    assert_eq!(file_names(lua), ["Co_Objectives1.txt", "movie.bik"]);
}

#[test]
fn unlock_chain_and_stock_counter_rule() {
    let book = CampaignBook::load(&source(), ShellLayout::Armada2);
    let mut r = CampaignRunner::new(book);
    assert_eq!(r.open_campaigns(), ["tutorials", "federation"], "borg is empty, klingon locked");
    assert_eq!(r.progress.available(&r.book, "federation"), 0..1);
    assert_eq!(r.record("federation", 0, true), Transition::Next { campaign: "federation".into(), mission: 1 });
    assert_eq!(r.progress.state("federation"), CampaignState { reached: 1, complete: false });
    // Replaying an earlier mission moves nothing; a loss moves nothing.
    assert_eq!(r.record("federation", 0, true), Transition::Next { campaign: "federation".into(), mission: 1 });
    assert_eq!(r.record("federation", 1, false), Transition::Lost { campaign: "federation".into(), mission: 1 });
    assert_eq!(r.progress.state("federation").reached, 1);
    r.record("federation", 1, true);
    assert_eq!(
        r.record("federation", 2, true),
        Transition::CampaignComplete { campaign: "federation".into(), unlocked: vec!["klingon".into()] }
    );
    assert_eq!(r.progress.state("federation"), CampaignState { reached: 2, complete: true });
    assert!(r.progress.is_open(&r.book, "klingon"));
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
    r.record("federation", 0, true);
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
    assert!(matches!(
        CampaignProgress::restore(&text.replace("campaign federation 1 0", "campaign federation 1 2")),
        Err(ProgressError::Format(..))
    ));
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
