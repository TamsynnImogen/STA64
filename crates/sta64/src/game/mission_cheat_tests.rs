//! Campaign-host integration for the single-player mission-ending cheats.

use super::*;
use crate::cheats::input::CheatSubmitted;
use crate::frontend::screens::ScreenData;
use crate::sim_link::ContentRes;

fn cheat_app(campaigns: Campaigns) -> App {
    let mut app = super::app(campaigns);
    app.add_plugins(crate::cheats::CheatsPlugin);
    app
}

fn submit(app: &mut App, text: &str) {
    // Closing the menu scene drops its ContentRes. The real map builder installs the game's
    // content; build_fake supplies only the link and mission handle, so finish that fixture here.
    let content = app.world().resource::<ScreenData>().content.clone();
    app.insert_resource(ContentRes(content));
    app.world_mut().write_message(CheatSubmitted(text.into()));
    app.update();
}

fn diagnostics(app: &App) -> Vec<String> {
    let notices = app
        .world()
        .resource::<Messages<crate::ui::notices::Notice>>();
    let mut cursor = notices.get_cursor();
    let mut said: Vec<String> = cursor.read(notices).map(|n| n.text.clone()).collect();
    said.push(format!(
        "menu scene: {}, content: {}",
        app.world()
            .contains_resource::<crate::frontend::scene::MenuScene>(),
        app.world().contains_resource::<ContentRes>()
    ));
    said
}

#[test]
fn dispatched_win_cheat_reaches_campaign_debrief_and_advances_progress() {
    let mut app = cheat_app(with_first_script(
        "MMM.register(function(t) if t > 1e9 then Mission:finish(false, 0) end end)\n",
        false,
    ));
    app.insert_resource(crate::map::MapBrowser {
        maps: vec!["blueamber".into()],
        index: 0,
        dirty: false,
        mount_summary: String::new(),
        info: String::new(),
    });
    start_from_screens(&mut app, 1);
    submit(&mut app, "kobayashimaru");
    assert!(
        app.world()
            .resource::<MissionHandle>()
            .with(MissionRuntime::mission_end_forced),
        "{:?}",
        diagnostics(&app)
    );

    until(&mut app, 300, |a| state(a) == AppState::FrontEnd);
    settle(&mut app, 4);
    let debrief = campaigns(&app).debrief.as_ref().expect("campaign debrief");
    assert_eq!(
        (debrief.campaign.as_str(), debrief.mission, debrief.result),
        ("federation", 0, RunResult::Won)
    );
    assert_eq!(campaigns(&app).progress.state("federation").reached, 1);
}

#[test]
fn dispatched_loss_cheat_bypasses_blocking_for_a_script_with_gaps() {
    let script = "local gap = NoSuchBinding.x\nMMM.register(function(t) if t > 1e9 then Mission:finish(true, 0) end end)\n";
    let mut app = cheat_app(with_tutorial_script(script));
    start_direct(&mut app);
    let handle = app.world().resource::<MissionHandle>();
    assert!(
        !handle.with(|rt| rt.unsupported().is_empty()),
        "the script recorded its unsupported name"
    );

    submit(&mut app, "kobayashimaru_lost");
    assert!(
        app.world()
            .resource::<MissionHandle>()
            .with(MissionRuntime::mission_end_forced),
        "{:?}",
        diagnostics(&app)
    );
    until(&mut app, 30, |a| {
        a.world().resource::<MissionRun>().outcome.is_some()
    });
    assert!(
        app.world().resource::<MissionRun>().blocked.is_none(),
        "forced loss bypasses unsupported-gap blocking"
    );
    until(&mut app, 300, |a| state(a) == AppState::FrontEnd);
    settle(&mut app, 4);

    let debrief = campaigns(&app).debrief.as_ref().expect("campaign debrief");
    assert_eq!(
        (debrief.campaign.as_str(), debrief.mission, debrief.result),
        ("tutorials", 0, RunResult::Lost)
    );
    assert_eq!(campaigns(&app).progress.state("tutorials").reached, 0);
}
