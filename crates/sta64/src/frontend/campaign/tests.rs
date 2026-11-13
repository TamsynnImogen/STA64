//! Windowless tests of the campaign screens (UI-25): the layout in every window shape and UI
//! scale, the card, list, briefing and debrief behaviour on a synthetic install, and progress
//! that survives a restart. The synthetic install ([`fixture`]) is shared with the mission
//! host's tests (`game::mission_tests`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::state::app::StatesPlugin;
use bevy::window::{PrimaryWindow, WindowResolution};
use sta_mission::campaign::{Campaign, CampaignBook, CampaignMission, MemSource, MissionHost, ScriptReader, ShellLayout, Transition, resolved_script};
use sta_sim::setup::GameSetup;

use super::layout::{self, CSlot, region};
use super::*;
use crate::app::{AppState, configure_sets};
use crate::frontend::screens::stock::layout::{fit_box, region as stock_region, Slot};
use crate::frontend::screens::{ScreenData, ScreensConfig, ScreensPlugin};
use crate::ui::theme::click;

pub(crate) const MSHELL: &str = "syn_tut1.bzn\r\n-\r\nsyn_fed01.bzn\r\nsyn_fed02.bzn\r\n-\r\nsyn_kl01.bzn\r\n-\r\n-\r\n";

const LABELS: &str = "single_player {\n\t+ federation_incomplete\t{Finish the first fleet}\n\t+ klingon_incomplete {Finish the first fleet}\n\t+ tutorials {Drills}\n\t+ federation_campaign {First Fleet}\n\t+ klingon_campaign {Second Fleet}\n}\nmission_select {\n\t+ syn_tut1.bzn\t{Warm Up}\n\t+ syn_fed01.bzn\t{First Light}\n\t+ syn_fed02.bzn\t{Second Wind}\n\t+ syn_kl01.bzn\t{Unfinished}\n}\n";

const OBJECTIVES: &str = "First Light\r\nThe synthetic fleet gathers.\r\n*  Survive one second\r\n*  Win\r\n";

/// A script that wins after a second of game time, with one objective it completes.
pub(crate) const WIN_AT_ONE_SECOND: &str = r#"
Objectives:load("SynF1_Objectives.txt")
Objectives.objectives:add('Hold the line')
MMM.register(function(t)
  if t >= 4 then
    Objectives.objectives[1].state = true
    Mission:finish(true, 0)
  end
end)
"#;

pub(crate) const LOSE_AT_HALF_SECOND: &str = "MMM.register(function(t) if t >= 0.5 then Mission:finish(false, 0) end end)\n";

/// Files of a synthetic install: tutorials (1 mission), federation (2), klingon (1 that has
/// no win event, so it is blocked) and an empty borg slot. `second_wins`: whether the second
/// federation mission wins.
pub(crate) fn source(second_wins: bool) -> MemSource {
    MemSource::new()
        .with("mshell.set", MSHELL)
        .with("label.map", LABELS)
        .with("ART_CFG.h", "int GIVE_ALL_MISSIONS = 0;\r\n")
        .with("bzn/syn_tut1.bzn", "not parsed")
        .with("bzn/syn_fed01.bzn", "not parsed")
        .with("bzn/syn_fed02.bzn", "not parsed")
        .with("bzn/syn_kl01.bzn", "not parsed")
        .with("missions/syn_tut1.lua", "MMM.register(function(t) if t >= 0.2 then Mission:finish(true, 0) end end)\n")
        .with("missions/syn_fed01.lua", WIN_AT_ONE_SECOND)
        .with("missions/syn_fed02.lua", if second_wins { "MMM.register(function(t) if t >= 0.5 then Mission:finish(true, 0) end end)\n" } else { LOSE_AT_HALF_SECOND })
        .with("missions/syn_kl01.lua", "-- nothing ends this mission\n")
        .with("objectives/SynF1_Objectives.txt", OBJECTIVES)
}

/// Scripts from a [`MemSource`]; the game is built by whoever hosts the test.
pub(crate) struct FakeHost(pub Arc<MemSource>);

impl MissionHost for FakeHost {
    fn game_setup(&self, _: &Campaign, _: &CampaignMission, _: u64) -> Result<GameSetup, String> {
        Err("the test's map builder makes the game".into())
    }

    fn script(&self, _: &Campaign, m: &CampaignMission) -> Result<(String, ScriptReader), String> {
        use sta_mission::campaign::CampaignSource;
        let src = self.0.clone();
        resolved_script(m, Arc::new(move |p| src.read(p)))
    }
}

/// The synthetic install's campaigns, with progress kept in `dir` (`None`: not saved).
pub(crate) fn fixture(second_wins: bool, dir: Option<&Path>) -> Campaigns {
    let src = Arc::new(source(second_wins));
    let book = CampaignBook::load(&*src, ShellLayout::Armada2);
    assert!(book.warnings.is_empty(), "{:?}", book.warnings);
    let store = dir.map_or_else(ProgressStore::none, |d| ProgressStore::in_dir(d, &book));
    Campaigns::new(book, Arc::new(FakeHost(src)), store)
}

/// A fresh directory under the system temp dir (removed by the caller).
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("sta64-campaign-{tag}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[test]
fn locked_unlocked_and_blocked_follow_the_book_and_the_progress() {
    let cs = fixture(false, None);
    // Federation: the first mission is open, the second not reached; Klingon is locked.
    assert_eq!(cs.state("federation", 0), MissionState::Ready);
    assert_eq!(cs.state("federation", 1), MissionState::Locked);
    assert_eq!(cs.state("klingon", 0), MissionState::Locked);
    assert!(cs.is_open("federation") && !cs.is_open("klingon") && !cs.is_open("borg"));
    // The Klingon mission has no win event: once reached it is blocked, with the reason.
    let mut cs = fixture(false, None);
    cs.progress.campaigns.get_mut("federation").unwrap().complete = true;
    match cs.state("klingon", 0) {
        MissionState::Blocked(why) => assert!(why.contains("no win event") || why.contains("Mission"), "{why}"),
        other => panic!("{other:?}"),
    }
    // Cards: the slot order of the shell, the locked text while locked.
    let fresh = fixture(false, None);
    assert_eq!(fresh.card_text(fresh.card(1).unwrap()), "First Fleet");
    assert_eq!(fresh.card_text(fresh.card(2).unwrap()), "Finish the first fleet");
}

#[test]
fn a_win_unlocks_the_next_mission_a_loss_does_not() {
    let mut cs = fixture(false, None);
    let d = cs.finish("federation", 0, RunResult::Won).unwrap().clone();
    assert_eq!(d.transition, Transition::Next { campaign: "federation".into(), mission: 1 });
    assert_eq!(cs.state("federation", 1), MissionState::Ready);
    assert_eq!(cs.mission, 1, "the list opens on the next mission");
    let before = cs.progress.clone();
    for r in [RunResult::Lost, RunResult::Abandoned] {
        let d = cs.finish("federation", 1, r).unwrap().clone();
        assert!(matches!(d.transition, Transition::Lost { .. }));
        assert_eq!(cs.progress, before, "a loss changes nothing");
    }
    assert!(!cs.is_open("klingon"));
    let d = cs.finish("federation", 1, RunResult::Won).unwrap().clone();
    assert_eq!(d.transition, Transition::CampaignComplete { campaign: "federation".into(), unlocked: vec!["klingon".into()] });
    assert!(cs.is_open("klingon"));
}

#[test]
fn progress_is_saved_after_a_result_and_read_back_by_a_new_state() {
    let dir = temp_dir("restart");
    let mut cs = fixture(true, Some(&dir));
    assert_eq!(cs.progress.state("federation").reached, 0);
    cs.finish("federation", 0, RunResult::Won);
    let path = cs.store.path().unwrap().to_owned();
    assert!(path.is_file(), "{path:?}");
    assert!(std::fs::read_to_string(&path).unwrap().starts_with("sta64-campaign-progress 1\n"));
    // A restart: the state is built again from the same store.
    let again = fixture(true, Some(&dir));
    assert_eq!(again.progress, cs.progress);
    assert_eq!(again.state("federation", 1), MissionState::Ready);
    assert!(again.load_warning.is_none());
    // Saved progress for another book is refused, not applied.
    std::fs::write(&path, "sta64-campaign-progress 1\nbook 0000000000000001\nend\n").unwrap();
    let refused = fixture(true, Some(&dir));
    assert!(refused.load_warning.is_some());
    assert_eq!(refused.progress.state("federation").reached, 0);
    // A file that cannot be written is reported on the debrief.
    let mut cs = fixture(true, Some(&dir.join("blocked/deeper")));
    std::fs::write(dir.join("blocked"), "a file where the directory should be").unwrap();
    let d = cs.finish("federation", 0, RunResult::Won).unwrap();
    assert!(d.save_error.is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------
// The screens
// ---------------------------------------------------------------------------

fn app_with(campaigns: Campaigns, window: Option<(u32, u32)>, scale: f32) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    configure_sets(&mut app);
    app.add_plugins(crate::ui::profile::UiProfilePlugin);
    app.insert_resource(ScreenData::fake())
        .insert_resource(UiScale(scale))
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: Screen::Main, ..default() })
        .insert_resource(campaigns)
        .init_resource::<crate::media::campaign::MissionMovieLog>()
        .add_plugins(ScreensPlugin);
    if let Some((w, h)) = window {
        app.world_mut().spawn((Window { resolution: WindowResolution::new(w, h), ..default() }, PrimaryWindow));
    }
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::FrontEnd);
    settle(&mut app);
    app
}

fn settle(app: &mut App) {
    for _ in 0..4 {
        app.update();
    }
}

fn go(app: &mut App, s: Screen) {
    app.world_mut().resource_mut::<NextState<Screen>>().set(s);
    settle(app);
}

fn screen(app: &App) -> Screen {
    *app.world().resource::<State<Screen>>().get()
}

fn act(app: &mut App, want: CampaignAct) -> Entity {
    app.world_mut().query::<(Entity, &CampaignAct)>().iter(app.world()).find(|(_, a)| **a == want).unwrap_or_else(|| panic!("no {want:?}")).0
}

fn press(app: &mut App, want: CampaignAct) {
    let e = act(app, want);
    click(app.world_mut(), e);
    settle(app);
}

fn has(app: &mut App, want: CampaignAct) -> bool {
    app.world_mut().query::<&CampaignAct>().iter(app.world()).any(|a| *a == want)
}

fn node_rect(app: &App, e: Entity) -> Rect {
    let n = app.world().get::<Node>(e).unwrap();
    let px = |v: Val| match v {
        Val::Px(p) => p,
        other => panic!("not px: {other:?}"),
    };
    Rect::new(px(n.left), px(n.top), px(n.left) + px(n.width), px(n.top) + px(n.height))
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x - 0.01 && b.min.x < a.max.x - 0.01 && a.min.y < b.max.y - 0.01 && b.min.y < a.max.y - 0.01
}

/// A debrief as a won, non-final mission leaves it (all six controls).
fn won_first(cs: &mut Campaigns) {
    cs.finish("federation", 0, RunResult::Won);
}

/// Windows: 21:9 (3440x1440), 16:9, 16:10 and 4:3, at the UI scales in use.
const WINDOWS: [(&str, u32, u32); 4] = [("21:9", 3440, 1440), ("16:9", 1920, 1080), ("16:10", 1920, 1200), ("4:3", 1600, 1200)];
const SCALES: [f32; 3] = [1.0, 1.25, 1.333];

fn check_layout(screen: Screen) {
    for (name, w, h) in WINDOWS {
        for scale in SCALES {
            let mut cs = fixture(true, None);
            won_first(&mut cs);
            cs.campaign = Some("federation".into());
            cs.mission = 1;
            let mut app = app_with(cs, Some((w, h)), scale);
            go(&mut app, screen);
            let ctx = format!("{screen:?} {name} x{scale}");
            let avail = Vec2::new(w as f32 / scale, h as f32 / scale);
            let b = fit_box(avail);
            assert_eq!(app.world().resource::<crate::frontend::screens::stock::StockBox>().0, b, "{ctx}");
            let all: Vec<(CSlot, Entity)> = app.world_mut().query::<(Entity, &CPlaced)>().iter(app.world()).map(|(e, p)| (p.0, e)).collect();
            for slot in layout::slots_of(screen) {
                assert!(all.iter().any(|(s, _)| s == slot), "{ctx}: no {slot:?}");
            }
            assert_eq!(all.len(), layout::slots_of(screen).len(), "{ctx}: controls outside the layout");
            let mut rects = Vec::new();
            for (slot, e) in &all {
                let r = node_rect(&app, *e);
                let want = region(*slot).px(b);
                for (got, exp) in [(r.min.x, want.min.x), (r.min.y, want.min.y), (r.max.x, want.max.x), (r.max.y, want.max.y)] {
                    assert!((got - exp).abs() < 0.01, "{ctx}: {slot:?} {r:?} is not its region {want:?}");
                }
                assert!(r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= b.x + 0.01 && r.max.y <= b.y + 0.01, "{ctx}: {slot:?} leaves the box");
                if let Some(kid) = app.world().get::<Children>(*e).and_then(|k| k.first().copied())
                    && let Some(n) = app.world().get::<Node>(kid)
                {
                    if let Val::Px(kh) = n.height {
                        assert!(kh <= r.height() + 0.5, "{ctx}: {slot:?} widget {kh}px high in a {}px region", r.height());
                    }
                    if let Val::Px(kw) = n.width {
                        assert!(kw <= r.width() + 0.5, "{ctx}: {slot:?} widget {kw}px wide in a {}px region", r.width());
                    }
                }
                rects.push((*slot, r));
            }
            for (i, (sa, ra)) in rects.iter().enumerate() {
                for (sb, rb) in &rects[i + 1..] {
                    assert!(!overlap(*ra, *rb), "{ctx}: {sa:?} overlaps {sb:?}");
                }
            }
        }
    }
}

#[test]
fn mission_list_layout_in_every_window_and_scale() {
    check_layout(Screen::Missions);
}

#[test]
fn briefing_layout_in_every_window_and_scale() {
    check_layout(Screen::Briefing);
}

#[test]
fn debrief_layout_in_every_window_and_scale() {
    check_layout(Screen::Debrief);
}

#[test]
fn single_player_cards_keep_their_regions_with_campaigns_installed() {
    for (name, w, h) in WINDOWS {
        for scale in SCALES {
            let mut app = app_with(fixture(false, None), Some((w, h)), scale);
            go(&mut app, Screen::SinglePlayer);
            let b = fit_box(Vec2::new(w as f32 / scale, h as f32 / scale));
            for i in 0..4 {
                let e = app.world_mut().query::<(Entity, &crate::frontend::screens::stock::Placed)>().iter(app.world()).find(|(_, p)| p.0 == Slot::Card(i)).map(|(e, _)| e).unwrap_or_else(|| panic!("{name} x{scale}: no card {i}"));
                let want = stock_region(Slot::Card(i)).px(b);
                let got = node_rect(&app, e);
                assert!((got.min - want.min).length() < 0.01 && (got.max - want.max).length() < 0.01, "{name} x{scale}: card {i}");
            }
        }
    }
}

#[test]
fn a_five_campaign_shell_stacks_its_cards_inside_the_box() {
    for i in 0..5 {
        let r = layout::card_region(i, 5).rect();
        assert!(r.min.y >= 0.0 && r.max.y <= 560.0 && r.min.x >= 0.0 && r.max.x <= 800.0);
        if i > 0 {
            assert!(!overlap(r, layout::card_region(i - 1, 5).rect()));
        }
    }
    // Armada II's four are the single player screen's own.
    for i in 0..4 {
        assert_eq!(layout::card_region(i, 4), stock_region(Slot::Card(i)));
    }
}

#[test]
fn campaign_regions_are_graded_and_only_the_cards_are_measured() {
    use crate::frontend::screens::stock::layout::Grade;
    for slot in [CSlot::ListRows, CSlot::ListDetails, CSlot::BriefText, CSlot::DebText, CSlot::ListPlay, CSlot::DebRetry] {
        assert_eq!(region(slot).grade, Grade::Arranged, "{slot:?}");
    }
    assert_eq!(layout::card_region(0, 4).grade, Grade::Measured);
}

/// What the screens sent.
#[derive(Resource, Default)]
struct Seen(Vec<StartMission>);

fn record(mut seen: ResMut<Seen>, mut a: MessageReader<StartMission>) {
    seen.0.extend(a.read().cloned());
}

fn pick_row(app: &mut App, i: usize) {
    let row = app.world_mut().query::<(Entity, &crate::ui::theme::ListItem)>().iter(app.world()).find(|(_, r)| r.0 == i).map(|(e, _)| e).unwrap_or_else(|| panic!("no row {i}"));
    click(app.world_mut(), row);
    settle(app);
}

#[test]
fn the_flow_from_the_card_to_the_start_request() {
    let mut app = app_with(fixture(false, None), None, 1.0);
    app.init_resource::<Seen>().add_systems(Last, record);
    go(&mut app, Screen::SinglePlayer);
    // Tutorials and Federation are open; Klingon and Borg are not (their cards are off).
    assert!(has(&mut app, CampaignAct::Card(0)) && has(&mut app, CampaignAct::Card(1)));
    assert!(!has(&mut app, CampaignAct::Card(2)) && !has(&mut app, CampaignAct::Card(3)));
    press(&mut app, CampaignAct::Card(1));
    assert_eq!(screen(&app), Screen::Missions);
    assert_eq!(app.world().resource::<Campaigns>().campaign.as_deref(), Some("federation"));
    // The second mission is not reached: it can be looked at, not opened.
    pick_row(&mut app, 1);
    assert_eq!(app.world().resource::<Campaigns>().mission, 1);
    assert!(!has(&mut app, CampaignAct::ToBriefing));
    pick_row(&mut app, 0);
    press(&mut app, CampaignAct::ToBriefing);
    assert_eq!(screen(&app), Screen::Briefing);
    // A bridge scene is playing: Start waits for it.
    let busy = crate::media::campaign::MissionMovieLog {
        mission: Some("syn_fed01".into()),
        movies: vec![crate::media::campaign::LoggedMovie { id: crate::media::MovieId(1), name: "intro.bik".into(), listed: false, outcome: None }],
    };
    *app.world_mut().resource_mut::<crate::media::campaign::MissionMovieLog>() = busy;
    settle(&mut app);
    assert!(!has(&mut app, CampaignAct::Start), "Start waits for the bridge scene");
    *app.world_mut().resource_mut::<crate::media::campaign::MissionMovieLog>() = default();
    settle(&mut app);
    press(&mut app, CampaignAct::Start);
    assert_eq!(app.world().resource::<Seen>().0, vec![StartMission { campaign: "federation".into(), mission: 0 }]);
    // Back to the list, back to Single Player.
    press(&mut app, CampaignAct::ToMissions);
    assert_eq!(screen(&app), Screen::Missions);
    press(&mut app, CampaignAct::ToSinglePlayer);
    assert_eq!(screen(&app), Screen::SinglePlayer);
}

#[test]
fn a_blocked_mission_shows_its_reason_and_cannot_be_started() {
    let mut cs = fixture(false, None);
    cs.progress.campaigns.get_mut("federation").unwrap().complete = true;
    let mut app = app_with(cs, None, 1.0);
    app.init_resource::<Seen>().add_systems(Last, record);
    go(&mut app, Screen::SinglePlayer);
    press(&mut app, CampaignAct::Card(2));
    assert_eq!(screen(&app), Screen::Missions);
    let reason = match app.world().resource::<Campaigns>().state("klingon", 0) {
        MissionState::Blocked(r) => r,
        other => panic!("{other:?}"),
    };
    // The reason is what the details block shows.
    let texts: Vec<String> = app.world_mut().query::<&crate::ui::sprite_font::SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect();
    assert!(texts.iter().any(|t| *t == reason), "{reason:?} in {texts:?}");
    press(&mut app, CampaignAct::ToBriefing);
    assert_eq!(screen(&app), Screen::Briefing);
    assert!(!has(&mut app, CampaignAct::Start), "no Start for a blocked mission");
    // Even a request that gets past the screen is refused by the host's check.
    assert!(app.world().resource::<Seen>().0.is_empty());
}

#[test]
fn a_mission_waiting_for_a_conversion_says_what_to_run_and_a_converted_one_is_marked() {
    let mut cs = fixture(false, None);
    cs.progress.campaigns.get_mut("federation").unwrap().complete = true;
    for b in cs.blocked.iter_mut() {
        b.reason = "legacy DLL only (syn_kl01R.drl): needs a Lua conversion on LUA-05".into();
    }
    assert!(cs.awaits_conversion());
    cs.convert_hint = Some(crate::game::mission_host::convert_hint(Path::new("/games/a2")));
    let hint = cs.convert_hint.clone().unwrap();
    assert!(hint.contains("tools/convert_install_missions.py --install \"/games/a2\""), "{hint}");
    let mut app = app_with(cs, None, 1.0);
    go(&mut app, Screen::SinglePlayer);
    press(&mut app, CampaignAct::Card(2));
    let texts: Vec<String> = app.world_mut().query::<&crate::ui::sprite_font::SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect();
    assert!(texts.contains(&hint), "{hint:?} in {texts:?}");

    // A mounted conversion marks the missions whose script it holds, in the list and the details.
    let dir = temp_dir("converted");
    std::fs::create_dir_all(dir.join("missions")).unwrap();
    std::fs::write(dir.join("missions/syn_fed01.lua"), "-- converted\n").unwrap();
    let mut cs = fixture(true, None);
    cs.mark_converted(&dir, &Default::default());
    assert_eq!(cs.experimental.iter().collect::<Vec<_>>(), ["syn_fed01.bzn"]);
    let mut app = app_with(cs, None, 1.0);
    go(&mut app, Screen::SinglePlayer);
    press(&mut app, CampaignAct::Card(1));
    let texts: Vec<String> = app.world_mut().query::<&crate::ui::sprite_font::SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("(experimental)")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("Experimental: converted")), "{texts:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_debrief_offers_the_next_mission_or_a_retry() {
    let mut cs = fixture(true, None);
    cs.finish("federation", 0, RunResult::Won);
    let mut app = app_with(cs, None, 1.0);
    go(&mut app, Screen::Debrief);
    assert!(has(&mut app, CampaignAct::Pick(1)), "next mission");
    assert!(has(&mut app, CampaignAct::Pick(0)), "play again");
    press(&mut app, CampaignAct::Pick(1));
    assert_eq!(screen(&app), Screen::Briefing);
    assert_eq!(app.world().resource::<Campaigns>().mission, 1);

    let mut cs = fixture(true, None);
    cs.finish("federation", 0, RunResult::Won);
    cs.finish("federation", 1, RunResult::Lost);
    let mut app = app_with(cs, None, 1.0);
    go(&mut app, Screen::Debrief);
    assert!(has(&mut app, CampaignAct::Pick(1)), "retry the mission that was lost");
    assert!(!has(&mut app, CampaignAct::Pick(0)));

    let mut cs = fixture(true, None);
    cs.finish("federation", 0, RunResult::Won);
    cs.finish("federation", 1, RunResult::Won);
    let mut app = app_with(cs, None, 1.0);
    go(&mut app, Screen::Debrief);
    press(&mut app, CampaignAct::ToSinglePlayer);
    assert_eq!(screen(&app), Screen::SinglePlayer);
    assert!(has(&mut app, CampaignAct::Card(2)), "the Klingon card is open now");
}

#[test]
fn the_briefing_shows_the_book_text() {
    let mut cs = fixture(false, None);
    cs.campaign = Some("federation".into());
    cs.mission = 0;
    let mut app = app_with(cs, None, 1.0);
    go(&mut app, Screen::Briefing);
    let texts: Vec<String> = app.world_mut().query::<&crate::ui::sprite_font::SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect();
    for want in ["First Light", "The synthetic fleet gathers.", "* Survive one second", "* Win"] {
        assert!(texts.iter().any(|t| t.to_lowercase().contains(&want.to_lowercase())), "{want:?} in {texts:?}");
    }
}
