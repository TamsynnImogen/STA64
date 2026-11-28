//! The front end's side of the in-game mission conversion (LUA-15k): what turns the hook core
//! ([`super::mission_convert`], LUA-15j) into something the player uses.
//!
//! * **When it starts.** By itself, once a session, the first time Single Player's campaign cards are
//!   shown for the base game of an install whose missions wait for a conversion
//!   ([`Campaigns::convertible`] and [`Campaigns::awaits_conversion`]: no mod, D6); and on the mission
//!   list's buttons (Prepare missions, Convert again), which send [`RequestConversion`]. Both go
//!   through [`forward`], which adds the install and the active mod and sends the hook's
//!   [`StartConversion`]; the hook refuses a mod itself, so a request is never trusted for it.
//! * **What the player is told.** A notice when a run ends ([`finished`]); the mission list draws the
//!   progress line, Cancel and the per-mission failure text from [`ConversionState`]
//!   (`frontend::campaign`).
//! * **After a run.** When the output folder may have changed ([`Outcome::changed`](super::mission_convert::Outcome::changed)),
//!   the campaigns are loaded again so the converted scripts are mounted and the missions
//!   marked, keeping the player's place (campaign, mission, debrief). Not while a match is going: the
//!   reload waits for the front end.
//!
//! Nothing here runs the conversion; DLLs are only read as bytes by the converter (D2) and converted
//! Lua runs in the same restricted state as every script (D13).

use std::path::Path;
use std::sync::Arc;

use bevy::prelude::*;

use super::flow::{FlowConfig, apply_campaigns};
use super::mission_convert::{ConversionFinished, ConversionState, Outcome, StartConversion};
use crate::app::AppState;
use crate::frontend::campaign::{Campaigns, ConvertMode, RequestConversion};
use crate::frontend::screens::{ActiveMod, Screen};
use crate::media::campaign::MissionMovies;
use crate::ui::notices::{Notice, Severity};
use crate::viewer::Mounted;

/// Whether the automatic start has been made this session.
#[derive(Resource, Default, Debug)]
pub struct AutoConvert {
    pub tried: bool,
}

/// What loads the campaigns again after a run: the game's own [`super::mission_host::load_campaigns`]
/// unless a test puts another in its place (so it can keep the player's progress files out of it).
#[derive(Resource, Clone)]
pub struct CampaignLoader(pub Arc<LoadFn>);

pub type LoadFn = dyn Fn(&sta_vfs::Vfs, Option<&Path>, Option<&str>) -> Option<(Campaigns, MissionMovies)> + Send + Sync;

impl Default for CampaignLoader {
    fn default() -> Self {
        CampaignLoader(Arc::new(super::mission_host::load_campaigns))
    }
}

/// Starts the conversion once, when the campaign cards are first shown for an install that needs it.
fn auto_start(
    screen: Option<Res<State<Screen>>>,
    campaigns: Option<Res<Campaigns>>,
    state: Res<ConversionState>,
    mut auto: ResMut<AutoConvert>,
    mut requests: MessageWriter<RequestConversion>,
) {
    let (Some(screen), Some(campaigns)) = (screen, campaigns) else { return };
    if *screen.get() != Screen::SinglePlayer || auto.tried || state.running {
        return;
    }
    if campaigns.convertible && campaigns.awaits_conversion() {
        auto.tried = true;
        requests.write(RequestConversion { mode: ConvertMode::Prepare });
    }
}

/// A request from a button or the automatic start becomes the hook's start message for this install.
fn forward(
    mut requests: MessageReader<RequestConversion>,
    flow: Option<Res<FlowConfig>>,
    active: Option<Res<ActiveMod>>,
    mut start: MessageWriter<StartConversion>,
    mut notices: MessageWriter<Notice>,
) {
    for r in requests.read() {
        let Some(root) = flow.as_ref().and_then(|f| f.install_root.clone()) else {
            notices.write(Notice::new(Severity::Bad, "No game install is available to prepare the missions from"));
            continue;
        };
        let (forced, replace_edited) = match r.mode {
            ConvertMode::Prepare => (false, false),
            ConvertMode::Again => (true, false),
            ConvertMode::ReplaceEdited => (true, true),
        };
        start.write(StartConversion { install_root: root, active_mod: active.as_ref().and_then(|a| a.0.clone()), forced, replace_edited });
    }
}

/// What the player is told when a run ends, as notices. (The mission list keeps the same in its status
/// line; the files the Python tool left are moved aside quietly and only logged.)
pub fn notices_for(outcome: &Outcome, state: &ConversionState) -> Vec<Notice> {
    let mut v = Vec::new();
    match outcome {
        Outcome::Completed { converted, failed, .. } => {
            if *converted > 0 {
                v.push(Notice::new(Severity::Good, crate::frontend::campaign::missions_ready(*converted)));
            }
            if *failed == 1 {
                v.push(Notice::new(Severity::Warn, "1 mission could not be converted; the mission list says why"));
            } else if *failed > 1 {
                v.push(Notice::new(Severity::Warn, format!("{failed} missions could not be converted; the mission list says why")));
            }
            match state.kept_edited.len() {
                0 => {}
                1 => v.push(Notice::new(Severity::Info, "1 edited script was kept as you left it")),
                n => v.push(Notice::new(Severity::Info, format!("{n} edited scripts were kept as you left them"))),
            }
            // an explicit replacement: where the edits went
            if state.replacing && !state.moved_aside.is_empty() {
                v.push(Notice::new(Severity::Info, format!("{} replaced scripts were moved aside as .old files in the missions folder", state.moved_aside.len())));
            }
        }
        Outcome::Cancelled { .. } => v.push(Notice::new(Severity::Info, "Preparing the missions was cancelled")),
        Outcome::Aborted(why) => v.push(Notice::new(Severity::Bad, format!("Preparing the missions stopped: {why}"))),
        Outcome::Busy => v.push(Notice::new(Severity::Warn, "Another STA64 is preparing these missions")),
        Outcome::Refused(r) if r.is_fault() => v.push(Notice::new(Severity::Bad, format!("The missions cannot be prepared: {r}"))),
        Outcome::Refused(_) | Outcome::Skipped | Outcome::AlreadyRunning => {}
    }
    v
}

/// Tells the player how a run ended and loads the campaigns again when its files may have changed.
#[allow(clippy::too_many_arguments)]
fn finished(
    mut done: MessageReader<ConversionFinished>,
    mut notices: MessageWriter<Notice>,
    mut pending: Local<bool>,
    state: Res<ConversionState>,
    app_state: Option<Res<State<AppState>>>,
    flow: Option<Res<FlowConfig>>,
    active: Option<Res<ActiveMod>>,
    mounted: Option<Res<Mounted>>,
    old: Option<Res<Campaigns>>,
    loader: Res<CampaignLoader>,
    mut commands: Commands,
) {
    for f in done.read() {
        for n in notices_for(&f.outcome, &state) {
            notices.write(n);
        }
        if !state.moved_aside.is_empty() {
            info!("mission conversion: {} files of the Python tool were moved aside", state.moved_aside.len());
        }
        *pending |= f.outcome.changed();
    }
    // A match is going: wait for the front end (the campaigns resource is the match's host).
    if !*pending || app_state.is_some_and(|s| *s.get() != AppState::FrontEnd) {
        return;
    }
    *pending = false;
    let (Some(flow), Some(mounted)) = (flow, mounted) else { return };
    let mod_name = active.as_ref().and_then(|a| a.0.as_deref());
    let loaded = (loader.0)(&mounted.0, flow.install_root.as_deref(), mod_name).map(|(mut c, m)| {
        if let Some(o) = old.as_deref() {
            keep_place(&mut c, o);
        }
        (c, m)
    });
    apply_campaigns(&mut commands, loaded);
}

/// The player's place in the screens, carried over to the campaigns loaded again: the list they are on,
/// the picked mission, the debrief, and the progress as it is in memory (a save may have failed).
pub fn keep_place(new: &mut Campaigns, old: &Campaigns) {
    if new.book.digest() != old.book.digest() {
        return;
    }
    new.progress = old.progress.clone();
    new.load_warning = old.load_warning.clone();
    new.debrief = old.debrief.clone();
    if let Some(c) = old.campaign.as_deref().and_then(|id| new.book.campaign(id)) {
        let last = c.missions.len().saturating_sub(1);
        new.campaign = Some(c.id.clone());
        new.mission = old.mission.min(last);
    }
}

pub struct ConvertFrontendPlugin;

impl Plugin for ConvertFrontendPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RequestConversion>()
            .add_message::<Notice>()
            .init_resource::<AutoConvert>()
            .init_resource::<CampaignLoader>()
            .add_systems(Update, (auto_start, forward, finished).chain());
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use bevy::state::app::StatesPlugin;
    use sta_mission_convert::cache::Cache;
    use sta_mission_convert::install::{Env, Event, Install, MissionResult, Options, Summary};
    use sta_mission_convert::output::OutDir;

    use super::*;
    use crate::frontend::campaign::ProgressStore;
    use crate::frontend::campaign::tests::{MSHELL, WIN_AT_ONE_SECOND, temp_dir};
    use crate::game::mission_convert::{ConvertContext, ConverterFn, MissionConvertPlugin, Refusal};
    use crate::game::mission_host::load_campaigns_with;

    /// A closure as the converter (spelled out so that its argument types are inferred).
    fn fake<F>(f: F) -> Arc<ConverterFn>
    where
        F: Fn(&Install, &Options, &mut OutDir, Option<&Cache>, &dyn Fn() -> bool, &mut dyn FnMut(&Event)) -> Summary + Send + Sync + 'static,
    {
        Arc::new(f)
    }

    fn loader<F>(f: F) -> CampaignLoader
    where
        F: Fn(&sta_vfs::Vfs, Option<&Path>, Option<&str>) -> Option<(Campaigns, MissionMovies)> + Send + Sync + 'static,
    {
        CampaignLoader(Arc::new(f))
    }

    const SCRIPT: &str = "MMM.register(function(t) if t >= 0.5 then Mission:finish(true, 0) end end)\n";

    /// A synthetic Armada II install (its files below `Data`): the install ships a script for the
    /// first federation mission only; the second has a rule DLL of junk bytes and no script.
    struct Site {
        tmp: PathBuf,
        root: PathBuf,
        data: PathBuf,
        conv: PathBuf,
    }

    fn site(tag: &str) -> Site {
        let tmp = temp_dir(tag);
        let (root, data) = (tmp.join("install"), tmp.join("data"));
        let d = root.join("Data");
        for dir in ["bzn", "missions"] {
            std::fs::create_dir_all(d.join(dir)).unwrap();
        }
        std::fs::write(root.join("Armada2.exe"), b"").unwrap();
        std::fs::write(d.join("mshell.set"), MSHELL).unwrap();
        std::fs::write(d.join("label.map"), "single_player {\n\t+ federation_campaign {First Fleet}\n}\nmission_select {\n\t+ syn_fed01.bzn {First Light}\n\t+ syn_fed02.bzn {Second Wind}\n}\n").unwrap();
        std::fs::write(d.join("ART_CFG.h"), "int GIVE_ALL_MISSIONS = 0;\r\n").unwrap();
        for b in ["syn_tut1", "syn_fed01", "syn_fed02", "syn_kl01"] {
            std::fs::write(d.join(format!("bzn/{b}.bzn")), "not parsed").unwrap();
        }
        std::fs::write(d.join("missions/syn_fed01.lua"), WIN_AT_ONE_SECOND).unwrap();
        std::fs::write(d.join("missions/syn_fed02S.dsl"), b"junk").unwrap();
        let conv = sta_mission::converted::dir_in(&data, &root, None);
        Site { tmp, root, data, conv }
    }

    impl Site {
        fn load(&self, pretend: bool) -> CampaignLoader {
            let (conv, tmp) = (self.conv.clone(), self.tmp.clone());
            loader(move |vfs, root, m| {
                let (mut c, movies) = load_campaigns_with(vfs, root, m, Some(&conv), |b| ProgressStore::in_dir(&tmp, b))?;
                if pretend {
                    // a converter that cannot help: the missions go on waiting
                    c.pretend_legacy();
                    c.convertible = true;
                }
                Some((c, movies))
            })
        }

        fn app(&self, converter: Arc<ConverterFn>, pretend: bool) -> App {
            let vfs = sta_vfs::Vfs::mount_install(&self.root, None).expect("mounts");
            let loader = self.load(pretend);
            let (mut cs, movies) = (loader.0)(&vfs, Some(&self.root), None).expect("campaigns");
            // the first load: the base game, nothing converted, the missions waiting
            cs.pretend_legacy();
            cs.convertible = true;
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, StatesPlugin))
                .insert_state(AppState::FrontEnd)
                .init_state::<Screen>()
                .add_plugins((MissionConvertPlugin, ConvertFrontendPlugin))
                .insert_resource(ConvertContext { data_dir: Some(self.data.clone()), env: Env::default(), converter })
                .insert_resource(FlowConfig { install_root: Some(self.root.clone()) })
                .insert_resource(ActiveMod(None))
                .insert_resource(Mounted(vfs))
                .insert_resource(loader)
                .insert_resource(cs)
                .insert_resource(movies)
                .init_resource::<Told>()
                .add_systems(Last, collect);
            app.update();
            app
        }
    }

    #[derive(Resource, Default)]
    struct Told(Vec<Notice>);

    fn collect(mut told: ResMut<Told>, mut r: MessageReader<Notice>) {
        told.0.extend(r.read().cloned());
    }

    fn run_until_idle(app: &mut App) {
        for _ in 0..2_000_000 {
            app.update();
            let s = app.world().resource::<ConversionState>();
            if !s.running && s.runs > 0 {
                app.update();
                return;
            }
            std::thread::yield_now();
        }
        panic!("the run did not end");
    }

    fn go(app: &mut App, s: Screen) {
        app.world_mut().resource_mut::<NextState<Screen>>().set(s);
        app.update();
        app.update();
    }

    /// A converter that writes the one missing script, as a real run would, and counts its calls.
    fn writes_the_script(calls: Arc<AtomicUsize>) -> Arc<ConverterFn> {
        fake(move |_i, _o, out, _c, _cancel, sink| {
            calls.fetch_add(1, Ordering::SeqCst);
            sink(&Event::Planned { total: 1 });
            sink(&Event::Started { index: 0, total: 1, mission: "syn_fed02".into() });
            out.write("syn_fed02.lua", SCRIPT.as_bytes()).unwrap();
            out.finish().unwrap();
            sink(&Event::Done { index: 0, total: 1, mission: "syn_fed02".into() });
            let one = MissionResult { mission: "syn_fed02".into(), converted: true, written: vec!["syn_fed02.lua".into()], ..MissionResult::default() };
            Summary { total: 1, missions: vec![one], ..Summary::default() }
        })
    }

    /// A converter that does nothing useful and counts its calls.
    fn does_nothing(calls: Arc<AtomicUsize>) -> Arc<ConverterFn> {
        fake(move |_i, _o, out, _c, _cancel, _sink| {
            calls.fetch_add(1, Ordering::SeqCst);
            let _ = out.finish();
            Summary::default()
        })
    }

    fn blocked(app: &App) -> bool {
        app.world().resource::<Campaigns>().book.blocked().iter().any(|b| b.bzn == "syn_fed02.bzn")
    }

    #[test]
    fn the_run_starts_by_itself_once_when_the_cards_are_first_shown() {
        let site = site("auto");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = site.app(does_nothing(calls.clone()), true);
        // the main menu is not the cards: nothing starts
        for _ in 0..4 {
            app.update();
        }
        assert_eq!((calls.load(Ordering::SeqCst), app.world().resource::<ConversionState>().runs), (0, 0));
        // the cards are shown: one run
        go(&mut app, Screen::SinglePlayer);
        run_until_idle(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(app.world().resource::<AutoConvert>().tried);
        // the missions still wait (the converter did nothing), the cards are shown again: no second run
        assert!(app.world().resource::<Campaigns>().awaits_conversion());
        go(&mut app, Screen::Main);
        go(&mut app, Screen::SinglePlayer);
        for _ in 0..8 {
            app.update();
        }
        assert_eq!((calls.load(Ordering::SeqCst), app.world().resource::<ConversionState>().runs), (1, 1));
        let _ = std::fs::remove_dir_all(&site.tmp);
    }

    #[test]
    fn nothing_starts_by_itself_where_nothing_waits_or_a_mod_is_active() {
        let site = site("noauto");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = site.app(does_nothing(calls.clone()), true);
        // a mod: the campaigns are not convertible (D6)
        app.world_mut().resource_mut::<Campaigns>().convertible = false;
        go(&mut app, Screen::SinglePlayer);
        for _ in 0..8 {
            app.update();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!app.world().resource::<AutoConvert>().tried);
        // and a request that reaches the hook with a mod active is refused there, whoever sent it
        app.insert_resource(ActiveMod(Some("Some Mod".into())));
        app.world_mut().write_message(RequestConversion { mode: ConvertMode::Again });
        app.update();
        app.update();
        let s = app.world().resource::<ConversionState>();
        assert!(matches!(s.outcome, Some(Outcome::Refused(Refusal::ModActive(_)))), "{:?}", s.outcome);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(app.world().resource::<Told>().0.is_empty(), "a mod is not a fault to tell the player about");
        let _ = std::fs::remove_dir_all(&site.tmp);
    }

    #[test]
    fn a_finished_run_loads_the_campaigns_again_and_keeps_the_players_place() {
        let site = site("reload");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = site.app(writes_the_script(calls.clone()), false);
        assert!(blocked(&app), "before: the second mission has no script");
        {
            let mut cs = app.world_mut().resource_mut::<Campaigns>();
            cs.campaign = Some("federation".into());
            cs.mission = 1;
        }
        app.world_mut().write_message(RequestConversion { mode: ConvertMode::Prepare });
        run_until_idle(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let cs = app.world().resource::<Campaigns>();
        assert!(!blocked(&app), "after: the converted script is mounted and the mission can run");
        assert_eq!(cs.experimental.iter().collect::<Vec<_>>(), ["syn_fed02.bzn"]);
        assert_eq!((cs.campaign.as_deref(), cs.mission), (Some("federation"), 1), "the player's place");
        let told = &app.world().resource::<Told>().0;
        assert_eq!(told, &vec![Notice::new(Severity::Good, "1 mission ready (experimental)")]);
        let _ = std::fs::remove_dir_all(&site.tmp);
    }

    #[test]
    fn the_campaigns_are_not_replaced_under_a_running_match_but_when_the_front_end_is_back() {
        let site = site("deferred");
        let mut app = site.app(writes_the_script(Arc::new(AtomicUsize::new(0))), false);
        app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::InGame);
        app.update();
        app.world_mut().write_message(RequestConversion { mode: ConvertMode::Prepare });
        run_until_idle(&mut app);
        assert!(blocked(&app), "a match is going: the campaigns stay as they are");
        app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::FrontEnd);
        app.update();
        app.update();
        assert!(!blocked(&app), "back in the front end the campaigns are loaded again");
        let _ = std::fs::remove_dir_all(&site.tmp);
    }

    #[test]
    fn a_request_without_an_install_is_told_and_starts_nothing() {
        let site = site("noinstall");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = site.app(does_nothing(calls.clone()), true);
        app.insert_resource(FlowConfig { install_root: None });
        app.world_mut().write_message(RequestConversion { mode: ConvertMode::Prepare });
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(app.world().resource::<ConversionState>().runs, 0);
        assert!(app.world().resource::<Told>().0.iter().any(|n| n.severity == Severity::Bad && n.text.contains("No game install")));
        let _ = std::fs::remove_dir_all(&site.tmp);
    }

    #[derive(Resource, Default)]
    struct Starts(Vec<StartConversion>);

    fn collect_starts(mut starts: ResMut<Starts>, mut r: MessageReader<StartConversion>) {
        starts.0.extend(r.read().cloned());
    }

    #[test]
    fn only_the_explicit_request_asks_for_the_edits_to_be_replaced() {
        let site = site("modes");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = site.app(does_nothing(calls.clone()), true);
        app.init_resource::<Starts>().add_systems(Last, collect_starts);
        for (i, mode) in [ConvertMode::Prepare, ConvertMode::Again, ConvertMode::ReplaceEdited].into_iter().enumerate() {
            app.world_mut().write_message(RequestConversion { mode });
            for _ in 0..2_000_000 {
                app.update();
                let s = app.world().resource::<ConversionState>();
                if !s.running && s.runs as usize > i {
                    break;
                }
                std::thread::yield_now();
            }
        }
        let got: Vec<(bool, bool)> = app.world().resource::<Starts>().0.iter().map(|s| (s.forced, s.replace_edited)).collect();
        assert_eq!(got, vec![(false, false), (true, false), (true, true)]);
        let _ = std::fs::remove_dir_all(&site.tmp);
    }

    #[test]
    fn the_campaigns_offer_the_conversion_for_the_base_game_only() {
        // through the loader the game uses, not a flag set by hand
        let site = site("modrule");
        std::fs::create_dir_all(site.conv.join("missions")).unwrap();
        std::fs::write(site.conv.join("missions/syn_fed02.lua"), SCRIPT).unwrap();
        let mod_dir = site.root.join("Data/Mods/Some Mod");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("info.ini"), "[Mod]\nName=Some Mod\n").unwrap();
        let store = |b: &sta_mission::campaign::CampaignBook| ProgressStore::in_dir(&site.tmp, b);
        let base = sta_vfs::Vfs::mount_install(&site.root, None).expect("the base game mounts");
        let (cs, _) = load_campaigns_with(&base, Some(&site.root), None, Some(&site.conv), store).expect("campaigns");
        assert!(cs.convertible, "the base game with a conversion mounted");
        let modded = sta_vfs::Vfs::mount_install(&site.root, Some("Some Mod")).expect("the mod mounts");
        let (cs, _) = load_campaigns_with(&modded, Some(&site.root), Some("Some Mod"), Some(&site.conv), store).expect("campaigns");
        assert!(!cs.convertible, "a mod's chain is never converted (D6)");
        let _ = std::fs::remove_dir_all(&site.tmp);
    }

    #[test]
    fn what_the_player_is_told_for_each_ending() {
        let state = ConversionState { kept_edited: vec!["syn_fed01".into()], ..default() };
        let one = |o: Outcome| notices_for(&o, &state);
        assert_eq!(one(Outcome::Completed { total: 3, converted: 3, cached: 0, failed: 0 }), vec![
            Notice::new(Severity::Good, "3 missions ready (experimental)"),
            Notice::new(Severity::Info, "1 edited script was kept as you left it"),
        ]);
        let both = one(Outcome::Completed { total: 3, converted: 2, cached: 0, failed: 1 });
        assert!(both.iter().any(|n| n.severity == Severity::Warn && n.text.starts_with("1 mission could not be converted")), "{both:?}");
        assert_eq!(one(Outcome::Cancelled { converted: 1 }), vec![Notice::new(Severity::Info, "Preparing the missions was cancelled")]);
        assert_eq!(one(Outcome::Aborted("boom".into())), vec![Notice::new(Severity::Bad, "Preparing the missions stopped: boom")]);
        assert_eq!(one(Outcome::Busy).len(), 1);
        for quiet in [Outcome::Skipped, Outcome::AlreadyRunning, Outcome::Refused(Refusal::NoMissionDlls), Outcome::Refused(Refusal::ModActive("m".into()))] {
            assert!(one(quiet.clone()).is_empty(), "{quiet:?}");
        }
        assert_eq!(one(Outcome::Refused(Refusal::NoDataDir)).len(), 1, "a fault is told");
    }
}
