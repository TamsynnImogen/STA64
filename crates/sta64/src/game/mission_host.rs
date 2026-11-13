//! Campaign missions in the client (INT-M6): a game started from a campaign mission owns
//! a mission runtime, ticks it with the sim, shows what it asks for and returns to the
//! debrief with the result recorded.
//!
//! ```text
//!   StartMission --(on_start_mission)--> Loading --(map view builds the game)--> InGame
//!        (campaign screens)                          MissionLaunch + MissionHandle
//!                                                                         |
//!   Debrief <--(finish_mission, OnEnter FrontEnd)-- LeaveMatch <--(run_mission: script ended)
//! ```
//!
//! # Ticking, and what is replay-safe
//!
//! The map view builds the sim from the mission's BZN ([`crate::map`], `build_mission_sim`):
//! the seats are the campaign's (`sta_mission::campaign::map_setup`: eight seats, seat 0 the
//! player) and the mission's Lua runtime ([`sta_mission::campaign::launch_runtime`]) is attached
//! to the [`SimLink`] as a controller, like the skirmish AI. So after every tick the script
//! thinks over the new view and the tick's events, in the same place of the same order every
//! run, and its commands go to the router like a player's (D3: controllers run on the host and
//! replays record their commands, never the script). Presentation requests are `SimEvent::Present`
//! events: the sim link already sends them as [`SimMsg`] to UI-30's consumers (titles, fades,
//! camera, movies).
//!
//! **Not replay-safe: the answers to the script.** There is no command kind that carries "movie
//! done", "message dismissed" or "cineractive skipped" into the command stream (SIM-61 adds it).
//! Until then [`deliver_reply_host_side`] is the one function that calls the script for them,
//! outside the command stream: a replay of a game in which the player answered does not carry the
//! answer, and the script it drives does not run again at all (D3), so only a *live* game depends
//! on them. The same function acknowledges audio messages at once (no audio lane plays
//! `Presentation::Sound` yet), so a script that waits for its message does not stall.
//!
//! # AI seats and the plans scripts load (AI-06)
//!
//! The non-human seats of the mission get a [`sta_ai::MissionAiHub`] controller: passive (no
//! building, no waves) until the script loads a plan with `Player:loadAIP` / `AI_Team_Load_AIP`.
//! [`MissionHandle`]'s controller runs the script, hands the loads it took
//! (`MissionRuntime::take_aip_requests`) to the hub, then thinks the seats, in seat order, over
//! the same view and events; their commands join the script's in the same `out` vector and reach
//! the router like a player's, numbered from `0x4000_0000` so they never collide with the
//! script's. A load therefore applies on the tick the script asks for it. The files are the
//! install's `AI/AIPs/*.aip` ([`MissionHost::aip_library`]); a request that names none, or a seat
//! with no AI, changes nothing and is listed by [`MissionHandle::unresolved_aips`].
//!
//! **Not replay-safe: the plan.** It reaches the seat through the hub, outside the command
//! stream, and is not in a snapshot: a save taken after a load loses the plan (SIM-61's row), and
//! a replay never runs the AI at all (D3), only the commands it recorded. The seats' commands are
//! deterministic for a given game: same view, same events, same order each run.
//!
//! # The mission's end
//!
//! The script ends a mission with `Mission:finish(won, delay)` (or `SucceedMission` /
//! `FailMission`); [`scan_outcome`] reads it each frame. The mission goes on for `delay` seconds of
//! sim time and while a bridge scene plays ([`MissionMovieLog::busy`], with a limit), then writes
//! [`LeaveMatch`]. Back at the front end [`finish_mission`] records the result
//! ([`Campaigns::finish`](crate::frontend::campaign::Campaigns::finish): progress, saved) and opens
//! the debrief. F10 before the end abandons the mission: a loss, progress unchanged. A mission
//! whose script does not load never reaches `InGame` and goes back to the mission list.

use std::sync::{Arc, Mutex, PoisonError};

use bevy::prelude::*;
use sta_content::{ContentDb, MapDef, Races};
use sta_core::{PlayerId, TICK_HZ};
use sta_mission::api::PlayerNumbering;
use sta_mission::campaign::{Campaign, CampaignBook, CampaignMission, MissionHost, MissionOutcome, ScriptReader, launch_runtime, map_setup, read_map_paths, resolved_script, scan_outcome};
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::Command;
use sta_sim::present::Presentation;
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{SimEvent, Sim, TeamId, WorldView};

use super::LocalRace;
use super::flow::{ActiveMatch, LeaveMatch};
use crate::app::{AppState, GameSet};
use crate::frontend::campaign::{Campaigns, MissionState, ProgressStore, RunResult, StartMission, mission_name};
use crate::frontend::loading::{LoadInfo, LoadProgress};
use crate::frontend::screens::{Screen, ScreenData};
use crate::map::MapSeat;
use crate::media::campaign::{MissionBegan, MissionEnded, MissionMovieLog, MissionMovies};
use crate::sim_link::{SimLink, SimMsg};
use crate::ui::notices::{Notice, Severity};
use crate::ui::presentation::{ObjectivesView, ScriptReply};

/// Seconds of sim time a finished mission waits for a bridge scene to end before it closes anyway.
const MOVIE_WAIT_SECS: u64 = 60;

/// A campaign mission about to be played: what the map view needs to build its game.
#[derive(Resource, Clone)]
pub struct MissionLaunch {
    pub campaign: String,
    pub mission: usize,
    pub seed: u64,
    /// The map as the book lists it (`m01.bzn`).
    pub bzn: String,
    /// The map's name in the map browser (`m01`, lower case).
    pub map: String,
    pub title: String,
    /// The script's VFS path.
    pub script: String,
    reader: ScriptReader,
    seat: (PlayerId, PlayerNumbering),
    /// The map's named paths for `A2.paths`.
    paths: Vec<(String, Vec<[f32; 3]>)>,
    /// The AIPs the script may load (AI-06).
    aips: sta_ai::AipLibrary,
}

impl std::fmt::Debug for MissionLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissionLaunch").field("campaign", &self.campaign).field("mission", &self.mission).field("map", &self.map).field("script", &self.script).finish_non_exhaustive()
    }
}

impl MissionLaunch {
    /// The launch of `mission` of `campaign`: it must be reachable and able to run
    /// ([`Campaigns::state`]), and its script resolves through the book's host.
    pub fn new(campaigns: &Campaigns, campaign: &str, mission: usize) -> Result<MissionLaunch, String> {
        let (c, m) = campaigns.mission(campaign, mission).ok_or_else(|| format!("{campaign} has no mission {mission}"))?;
        match campaigns.state(campaign, mission) {
            MissionState::Ready => {}
            MissionState::Locked => return Err(format!("{} is locked", mission_name(m))),
            MissionState::Blocked(why) => return Err(format!("{} cannot run yet: {why}", mission_name(m))),
        }
        let (script, reader) = campaigns.host.script(c, m)?;
        Ok(MissionLaunch {
            campaign: campaign.to_owned(),
            mission,
            seed: campaigns.seed(campaign, mission),
            bzn: m.bzn.clone(),
            map: m.stem().to_ascii_lowercase(),
            title: mission_name(m),
            script,
            reader,
            seat: campaigns.host.seat(),
            paths: campaigns.host.map_paths(c, m),
            aips: campaigns.host.aip_library(),
        })
    }

    /// The seat the player takes.
    pub fn player(&self) -> PlayerId {
        self.seat.0
    }

    /// The game over `map`: the campaign's seats ([`map_setup`]).
    pub fn setup(&self, map: MapDef, content: Arc<ContentDb>) -> GameSetup {
        map_setup(map, content, self.seed)
    }

    /// A stand-in setup for the loading phase, before the map view has converted the map: it
    /// names the map the browser must build and holds the player's seat.
    pub fn placeholder(&self, content: Arc<ContentDb>) -> GameSetup {
        let mut map = MapDef::empty();
        map.name.clone_from(&self.map);
        let player = |id: u8, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: String::new(), control };
        GameSetup { seed: self.seed, map: Arc::new(map), content, players: vec![player(self.seat.0.0, Control::Human)], options: GameOptions::default() }
    }

    /// Load the script over the built game and share it between the link and the host.
    pub fn attach(&self, setup: &GameSetup, sim: &Sim) -> Result<MissionHandle, String> {
        let runtime = launch_runtime(&self.script, self.reader.clone(), self.seed, self.seat, setup, sim, self.paths.clone()).map_err(|e| e.to_string())?;
        let hub = sta_ai::MissionAiHub::new(self.aips.clone());
        let ai = setup.players.iter().filter(|p| !matches!(p.control, Control::Human)).map(|p| hub.controller(setup, p.id)).collect();
        Ok(MissionHandle { runtime: Arc::new(Mutex::new(runtime)), seats: Arc::new(Mutex::new(Seats { hub, ai })) })
    }

    /// The seat's race for the HUD, music and cfg (`None`: the install does not list it).
    pub fn local_race(&self, setup: &GameSetup, races: &Races) -> Option<LocalRace> {
        let race = &setup.players.iter().find(|p| p.id == self.seat.0)?.race;
        let def = races.get_playable(race).or_else(|| races.get(race))?;
        Some(LocalRace {
            name: def.name.clone(),
            interface: def.interface.clone(),
            interface_sprites: def.interface_sprites.clone(),
            music: def.single_player_music.clone().or_else(|| def.multiplayer_music.clone()),
        })
    }
}

/// The mission's script runtime, shared by the [`SimLink`] (which thinks it after every tick as
/// a controller) and this module (objectives, outcome, answers). Never locked across a frame.
#[derive(Resource, Clone)]
pub struct MissionHandle {
    runtime: Arc<Mutex<MissionRuntime>>,
    seats: Arc<Mutex<Seats>>,
}

/// The mission's AI seats and the queue of plan loads between the script and them.
struct Seats {
    hub: sta_ai::MissionAiHub,
    ai: Vec<Box<dyn Controller>>,
}

impl MissionHandle {
    /// The controller for [`SimLink::with_controllers`]: the script, then the AI seats.
    pub fn controller(&self) -> Box<dyn Controller> {
        Box::new(Shared { rt: self.runtime.clone(), seats: self.seats.clone() })
    }

    /// Run `f` on the runtime.
    pub fn with<R>(&self, f: impl FnOnce(&MissionRuntime) -> R) -> R {
        f(&self.runtime.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Plan loads that changed nothing: a seat with no AI or a file the install does not have.
    pub fn unresolved_aips(&self) -> Vec<(PlayerId, String)> {
        self.seats.lock().unwrap_or_else(PoisonError::into_inner).hub.unresolved()
    }
}

struct Shared {
    rt: Arc<Mutex<MissionRuntime>>,
    seats: Arc<Mutex<Seats>>,
}

impl Controller for Shared {
    fn think(&mut self, view: &WorldView, events: &[SimEvent], out: &mut Vec<Command>) {
        let mut rt = self.rt.lock().unwrap_or_else(PoisonError::into_inner);
        // No audio lane plays `Presentation::Sound` yet: acknowledge each message on the tick it
        // was raised (tick-exact, not frame-rate dependent), through the host-side reply function.
        for e in events {
            if let SimEvent::Present(Presentation::Sound { id, .. }) = e {
                acknowledge_audio(&rt, *id);
            }
        }
        rt.think(view, events, out);
        // The script ran first, so a plan it loads this tick reaches its seat this tick.
        let loads = rt.take_aip_requests();
        drop(rt);
        let mut seats = self.seats.lock().unwrap_or_else(PoisonError::into_inner);
        seats.hub.submit(loads);
        for ai in &mut seats.ai {
            ai.think(view, events, out);
        }
    }
}

/// The running mission, from the start request to the debrief.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct MissionRun {
    pub campaign: String,
    pub mission: usize,
    /// `InGame` was reached (a failed load never gets here, and records nothing).
    pub began: bool,
    /// What the script ended the mission with.
    pub outcome: Option<MissionOutcome>,
    /// The sim tick the mission closes at (the script's delay after its end).
    pub close_at: Option<u64>,
    /// `LeaveMatch` was written.
    pub leaving: bool,
}

/// The answers to the script, from the client, and the audio acknowledgement.
///
/// **The one place a script is answered outside the command stream** (see the module docs and
/// SIM-61): a replay does not carry these calls. `A1.` / `A2.MovieDone()` and
/// `AudioMessageDone(id)` are the report calls of the Armada bindings; a cineractive skip sets
/// the A2 `Cineractive.active` flag off.
pub fn deliver_reply_host_side(rt: &MissionRuntime, reply: &ScriptReply) {
    let result = match reply {
        ScriptReply::MovieDone { .. } => rt.eval::<()>(REPORT_MOVIE),
        ScriptReply::MessageDismissed { id } => rt.eval_with::<(), _>(REPORT_AUDIO, *id),
        ScriptReply::CineractiveSkipped => rt.eval::<()>(REPORT_CINERACTIVE),
        // The enum is non-exhaustive outside this crate; a kind added later gets its arm here.
    };
    if let Err(e) = result {
        warn!("mission: reply {reply:?} not delivered: {e}");
    }
}

/// Acknowledge an audio message as played (no audio consumer reports it yet; same caveat).
fn acknowledge_audio(rt: &MissionRuntime, id: u32) {
    deliver_reply_host_side(rt, &ScriptReply::MessageDismissed { id });
}

const REPORT_MOVIE: &str = "for _, n in ipairs({'A1', 'A2'}) do local t = rawget(_G, n) if type(t) == 'table' then local f = rawget(t, 'MovieDone') if f then pcall(f) end end end";
const REPORT_AUDIO: &str = "local id = ... for _, n in ipairs({'A1', 'A2'}) do local t = rawget(_G, n) if type(t) == 'table' then local f = rawget(t, 'AudioMessageDone') if f then pcall(f, id) end end end";
const REPORT_CINERACTIVE: &str = "local c = rawget(_G, 'Cineractive') if c ~= nil then pcall(function() c.active = false end) end";

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// A campaign screen asked for a mission: check it, tell the loading screen what it is and let
/// the map view build it.
#[allow(clippy::too_many_arguments)]
fn on_start_mission(
    mut starts: MessageReader<StartMission>,
    campaigns: Option<Res<Campaigns>>,
    data: Res<ScreenData>,
    mut commands: Commands,
    mut next: ResMut<NextState<AppState>>,
    mut notices: MessageWriter<Notice>,
    info: Option<ResMut<LoadInfo>>,
    progress: Option<ResMut<LoadProgress>>,
    browser: Option<ResMut<crate::map::MapBrowser>>,
) {
    let Some(StartMission { campaign, mission }) = starts.read().last().cloned() else { return };
    let Some(campaigns) = campaigns else { return };
    let launch = match MissionLaunch::new(&campaigns, &campaign, mission) {
        Ok(l) => l,
        Err(e) => {
            warn!("mission: {e}");
            notices.write(Notice::new(Severity::Bad, format!("Cannot start: {e}")));
            return;
        }
    };
    info!("mission: {} ({}, {}) from {}", launch.title, campaign, launch.map, launch.script);
    if let Some(mut info) = info {
        let m = campaigns.mission(&campaign, mission).map(|(_, m)| m);
        let b = m.and_then(|m| m.briefing.as_ref());
        *info = LoadInfo {
            kicker: format!("{} {}", campaigns.book.campaign(&campaign).map_or_else(String::new, crate::frontend::campaign::campaign_name), mission + 1),
            title: launch.title.clone(),
            briefing: b.map(|b| b.text.trim().split("\n\n").map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
            objectives: b.map(|b| b.goals.clone()).unwrap_or_default(),
            background: String::new(),
        };
    }
    // The front end lists only multiplayer maps; a mission's map is built by name all the same.
    if let Some(mut b) = browser
        && !b.maps.iter().any(|m| m.eq_ignore_ascii_case(&launch.map))
    {
        b.maps.push(launch.map.clone());
    }
    if let Some(mut p) = progress {
        p.begin();
    }
    commands.insert_resource(MapSeat(Some(launch.player().0)));
    commands.insert_resource(ActiveMatch { setup: launch.placeholder(data.content.clone()), requested: false, frames: 0 });
    commands.insert_resource(MissionRun { campaign, mission, began: false, outcome: None, close_at: None, leaving: false });
    commands.insert_resource(launch);
    next.set(AppState::Loading);
}

/// The match is up: the mission begins (its movies are tracked from here, the objectives panel opens).
fn begin_mission(run: Option<ResMut<MissionRun>>, launch: Option<Res<MissionLaunch>>, mut began: MessageWriter<MissionBegan>, view: Option<ResMut<ObjectivesView>>) {
    let (Some(mut run), Some(launch)) = (run, launch) else { return };
    run.began = true;
    began.write(MissionBegan { map: launch.bzn.clone() });
    if let Some(mut v) = view {
        // The stock mission opens on its objectives; Alt+O toggles them.
        v.visible = true;
    }
}

/// Each frame of the mission: the objectives panel, the answers to the script, the script's end.
#[allow(clippy::too_many_arguments)]
fn run_mission(
    handle: Res<MissionHandle>,
    link: Res<SimLink>,
    mut run: ResMut<MissionRun>,
    mut replies: MessageReader<ScriptReply>,
    mut view: Option<ResMut<ObjectivesView>>,
    log: Option<Res<MissionMovieLog>>,
    mut leave: MessageWriter<LeaveMatch>,
) {
    let tick = link.curr().tick;
    for r in replies.read() {
        handle.with(|rt| deliver_reply_host_side(rt, r));
    }
    if let Some(v) = view.as_mut() {
        let mut want = (**v).clone();
        handle.with(|rt| want.fill(&rt.objectives(), rt.objective_briefing().as_ref()));
        if want != **v {
            **v = want;
        }
    }
    if run.outcome.is_none()
        && let Some(o) = handle.with(|rt| scan_outcome(rt, tick))
    {
        info!("mission: {} {} at tick {tick} (closes after {}s)", run.campaign, if o.won { "won" } else { "lost" }, o.delay);
        run.close_at = Some(tick.saturating_add((o.delay.clamp(0.0, 600.0) * f64::from(TICK_HZ)).ceil() as u64));
        run.outcome = Some(o);
    }
    if let (Some(close), false) = (run.close_at, run.leaving) {
        let waited_long = tick >= close.saturating_add(MOVIE_WAIT_SECS * u64::from(TICK_HZ));
        if tick >= close && (waited_long || !log.is_some_and(|l| l.busy())) {
            run.leaving = true;
            leave.write(LeaveMatch);
        }
    }
}

/// Back at the front end: record what happened and open the debrief (or the list, when the
/// mission never started). Takes the mission's resources and what it left on the screen down.
#[allow(clippy::too_many_arguments)]
fn finish_mission(
    run: Option<Res<MissionRun>>,
    mut commands: Commands,
    campaigns: Option<ResMut<Campaigns>>,
    mut next: ResMut<NextState<Screen>>,
    mut ended: MessageWriter<MissionEnded>,
    view: Option<ResMut<ObjectivesView>>,
    titles: Option<ResMut<crate::ui::presentation::Titles>>,
    fade: Option<ResMut<crate::ui::presentation::FadeState>>,
    cineractive: Option<ResMut<crate::ui::presentation::CineractiveState>>,
    camera: Option<ResMut<crate::input::camera_cinematic::CinematicCamera>>,
) {
    let Some(run) = run else { return };
    commands.remove_resource::<MissionRun>();
    commands.remove_resource::<MissionLaunch>();
    commands.remove_resource::<MissionHandle>();
    ended.write(MissionEnded);
    if let Some(mut v) = view {
        *v = ObjectivesView::default();
    }
    if let Some(mut t) = titles {
        *t = default();
    }
    if let Some(mut f) = fade {
        *f = default();
    }
    if let Some(mut c) = cineractive {
        *c = default();
    }
    if let Some(mut c) = camera {
        *c = default();
    }
    let Some(mut campaigns) = campaigns else { return };
    if !run.began {
        // The load failed (the notice is up): back to the list.
        campaigns.campaign = Some(run.campaign.clone());
        campaigns.mission = run.mission;
        next.set(Screen::Missions);
        return;
    }
    let result = match run.outcome {
        Some(o) if o.won => RunResult::Won,
        Some(_) => RunResult::Lost,
        None => RunResult::Abandoned,
    };
    if campaigns.finish(&run.campaign, run.mission, result).is_some() {
        next.set(Screen::Debrief);
    } else {
        next.set(Screen::Missions);
    }
}

/// The install's side of [`MissionHost`] for the client: scripts are read through a mount of
/// their own (the runtime reads `include()`s while it runs, long after the front end's mount is
/// borrowed by anything else), and the game itself is built by the map view, not here.
pub struct ClientHost {
    vfs: Arc<sta_vfs::Vfs>,
}

impl ClientHost {
    pub fn new(vfs: Arc<sta_vfs::Vfs>) -> ClientHost {
        ClientHost { vfs }
    }
}

impl MissionHost for ClientHost {
    fn game_setup(&self, _: &Campaign, mission: &CampaignMission, _: u64) -> Result<GameSetup, String> {
        Err(format!("{} is built by the map view (game::mission_host), not by the host", mission.bzn))
    }

    fn script(&self, _: &Campaign, mission: &CampaignMission) -> Result<(String, ScriptReader), String> {
        let vfs = self.vfs.clone();
        resolved_script(mission, Arc::new(move |p| vfs.read(p).and_then(Result::ok)))
    }

    fn map_paths(&self, _: &Campaign, mission: &CampaignMission) -> Vec<(String, Vec<[f32; 3]>)> {
        read_map_paths(&self.vfs, &mission.bzn)
    }

    fn seat(&self) -> (PlayerId, PlayerNumbering) {
        (PlayerId(0), PlayerNumbering::ArmadaTeams)
    }

    fn aip_library(&self) -> sta_ai::AipLibrary {
        let names = self.vfs.list("AI/AIPs");
        sta_ai::AipLibrary::load(&names, &mut |n| {
            let bytes = self.vfs.read(n)?.ok()?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        })
    }
}

/// The installed campaigns for the front end, and the movies their scripts name: `None` when the
/// install has no campaign mission (Fleet Ops 4.0 and Dominion War list none) or cannot be mounted
/// again for the scripts. Progress is read from the platform config directory.
pub fn load_campaigns(vfs: &sta_vfs::Vfs, install_root: Option<&std::path::Path>, active_mod: Option<&str>) -> Option<(Campaigns, MissionMovies)> {
    let converted = install_root.and_then(|r| crate::platform::converted_missions_dir(r, active_mod));
    load_campaigns_with(vfs, install_root, active_mod, converted.as_deref(), ProgressStore::platform)
}

/// [`load_campaigns`] without the user's converted missions, and with the progress store
/// given (tests keep the player's files out of it).
pub fn load_campaigns_in(vfs: &sta_vfs::Vfs, install_root: Option<&std::path::Path>, active_mod: Option<&str>, store: impl FnOnce(&CampaignBook) -> ProgressStore) -> Option<(Campaigns, MissionMovies)> {
    load_campaigns_with(vfs, install_root, active_mod, None, store)
}

/// The command that converts an install's missions (shown by the missions screen).
pub fn convert_hint(install_root: &std::path::Path, active_mod: Option<&str>) -> String {
    let m = active_mod.filter(|m| !m.is_empty()).map(|m| format!(" --mod \"{m}\"")).unwrap_or_default();
    format!("Run once, then restart: python3 tools/convert_install_missions.py --install \"{}\"{m}", install_root.display())
}

/// [`load_campaigns_in`] with the converted-missions folder `converted` (when it holds
/// scripts) mounted below the install: maps with no Lua of their own run the converted one,
/// marked experimental (D6: the layer follows the install and mod it was made for; the
/// install's own scripts win). Without it, missions that wait for a conversion carry the
/// command to run.
pub fn load_campaigns_with(
    vfs: &sta_vfs::Vfs,
    install_root: Option<&std::path::Path>,
    active_mod: Option<&str>,
    converted: Option<&std::path::Path>,
    store: impl FnOnce(&CampaignBook) -> ProgressStore,
) -> Option<(Campaigns, MissionMovies)> {
    let t0 = std::time::Instant::now();
    let mut host_vfs = match sta_vfs::Vfs::mount_install(install_root?, active_mod) {
        Ok(v) => v,
        Err(e) => {
            warn!("campaigns: cannot mount the install for the mission scripts: {e}");
            return None;
        }
    };
    let own: std::collections::BTreeSet<String> = host_vfs.list("missions").into_iter().map(|p| p.to_ascii_lowercase()).collect();
    let mounted = converted.filter(|d| sta_mission::converted::mount(&mut host_vfs, d));
    // The book reads the same layers the scripts will be read from.
    let book = CampaignBook::load_vfs(&host_vfs);
    for w in &book.warnings {
        warn!("campaigns: {w}");
    }
    let missions: usize = book.campaigns.iter().map(|c| c.missions.len()).sum();
    if missions == 0 {
        info!("campaigns: none in this install");
        return None;
    }
    let movies = MissionMovies::from_book(&book, vfs);
    let store = store(&book);
    let mut campaigns = Campaigns::new(book, Arc::new(ClientHost::new(Arc::new(host_vfs))), store);
    if let Some(dir) = mounted {
        campaigns.mark_converted(dir, &own);
        info!("campaigns: {} missions run converted scripts from {} (experimental)", campaigns.experimental.len(), dir.display());
    } else if campaigns.awaits_conversion()
        && let Some(root) = install_root
    {
        campaigns.convert_hint = Some(convert_hint(root, active_mod));
        info!("campaigns: missions wait for a Lua conversion; {}", campaigns.convert_hint.as_deref().unwrap_or_default());
    }
    if let Some(w) = &campaigns.load_warning {
        warn!("campaigns: saved progress not used: {w}");
    }
    info!("campaigns: {} campaigns, {missions} missions, {} cannot run yet, in {:.1?}", campaigns.book.campaigns.len(), campaigns.book.blocked().len(), t0.elapsed());
    Some((campaigns, movies))
}

pub struct MissionHostPlugin;

impl Plugin for MissionHostPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<StartMission>()
            .add_message::<MissionBegan>()
            .add_message::<MissionEnded>()
            .add_message::<ScriptReply>()
            .add_message::<SimMsg>()
            .add_message::<Notice>()
            .add_systems(OnEnter(AppState::InGame), begin_mission)
            .add_systems(OnEnter(AppState::FrontEnd), finish_mission.after(crate::frontend::screens::enter_front_end))
            .add_systems(
                Update,
                (
                    on_start_mission.run_if(in_state(AppState::FrontEnd)).run_if(resource_exists::<ScreenData>),
                    run_mission.run_if(in_state(AppState::InGame)).run_if(resource_exists::<MissionHandle>).run_if(resource_exists::<MissionRun>).run_if(resource_exists::<SimLink>),
                )
                    .in_set(GameSet::Ui),
            );
    }
}
