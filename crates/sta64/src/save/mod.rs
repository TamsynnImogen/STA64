//! The client's save and load path (UI-55, ADR 0010 V3, the client side of V5, V7).
//!
//! - **Entry.** [`SaveMatch`] and [`LoadMatch`] are the whole interface: the menus (UI-56), tests and
//!   a console write them; [`SaveOutcome`] and [`LoadOutcome`] answer. [`SaveAvailability`] says,
//!   every frame, whether Save is allowed and, if not, why (V7); it reads cheap state only (see
//!   [`MatchFacts`]: the map file is hashed and the controllers asked once per match, not per frame).
//! - **Save.** The match's snapshot, journal, computer players' records, the commands in flight with
//!   the link's next command number, and the client's record go through [`container`] (the sim crate's
//!   save container, SIM-81) into `saves/` in the user data directory, atomically. A match whose map
//!   has no file is refused.
//! - **Load.** The whole file is checked and decoded **before** a running match is left: the
//!   container fully (damage, formats, content with the map's tech tree, map by the header's path and
//!   hash), the computer players restored, the client record read. Only a save that will load takes
//!   the match down. Then the existing `StartGame` -> loading screen -> match build runs (seats,
//!   options and seed from the header; the shroud and fog choice and the speed from the client
//!   record), and once the build has put its link up, the prepared game replaces the link's
//!   ([`SimLink::restore`](crate::sim_link::SimLink)). The pending load is tied to that one start: it
//!   is dropped, with a refusal, if the start fails or another match starts.
//! - **Client record** ([`record`], version 1): camera pose, the local seat's explored memory, game
//!   speed, the setup's shroud and fog choice. Commands in flight are in the container. Control groups
//!   are not saved: they live in `input/`, under another package's claim (#960); they come back
//!   empty. An unreadable record never refuses a load.

pub mod container;
pub mod record;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use sta_core::PlayerId;
use sta_sim::replay::{ControlRec, Replay, SeatRec};
use sta_sim::setup::{Control, Controller, GameSetup, PlayerSetup};

use crate::app::{AppState, GameSet};
use crate::frontend::screens::{ScreenData, StartGame};
use crate::game::flow::{ActiveMatch, LeaveMatch};
use crate::map::MainCamera;
use crate::render::fog::FogMemory;
use crate::sim_link::{CommandQueue, Restored, Selection, SimLink};
use crate::ui::notices::{Notice, Severity};
use crate::viewer::{Mounted, Orbit};
use container::{ClientState, GameKind, InFlight, MapId, MapSource, SaveError, SaveInfo};
use record::{CameraRec, ClientRecord};

/// Save the running match. `description` goes into the header; the file is a new one under a
/// generated name, or `overwrite` (a file name in `saves/`, which the caller has asked about).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct SaveMatch {
    pub description: String,
    pub overwrite: Option<String>,
}

/// Load a save: from the front end or from a running match (which is left only once the save is
/// known to load).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct LoadMatch {
    pub path: PathBuf,
}

/// Why Save is not allowed (V7). `Display` is the reason the menu shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveBlock {
    NotInMatch,
    Networked,
    Observer,
    /// A replay playback, a bench or a link that keeps no journal.
    NoJournal,
    Mission,
    Cutscene,
    Movie,
    /// A computer player that cannot be saved (yet); the seat is named.
    Controller(PlayerId),
    /// A generated map, or one started from the editor, has no file to name.
    NoMapFile,
    /// A seat is under the MCP bridge: its orders do not pass through the link's in-flight list.
    McpSeat,
    NoSaveFolder,
}

impl std::fmt::Display for SaveBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveBlock::NotInMatch => write!(f, "There is no match to save"),
            SaveBlock::Networked => write!(f, "A networked game cannot be saved"),
            SaveBlock::Observer => write!(f, "A game watched as an observer cannot be saved"),
            SaveBlock::NoJournal => write!(f, "This game keeps no record of its orders (a replay or test session) and cannot be saved"),
            SaveBlock::Mission => write!(f, "Missions cannot be saved yet"),
            SaveBlock::Cutscene => write!(f, "A game cannot be saved while a cutscene plays"),
            SaveBlock::Movie => write!(f, "A game cannot be saved while a movie plays"),
            SaveBlock::Controller(seat) => write!(f, "The computer player of seat {} cannot be saved yet", seat.0),
            SaveBlock::NoMapFile => write!(f, "This match's map has no map file (a generated or unsaved map); it cannot be saved yet"),
            SaveBlock::McpSeat => write!(f, "A game with a seat under the MCP bridge cannot be saved"),
            SaveBlock::NoSaveFolder => write!(f, "No folder for saves is known on this machine"),
        }
    }
}

/// The answer to a [`SaveMatch`].
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved { path: PathBuf, tick: u64 },
    NotAllowed(SaveBlock),
    /// The container or the disk refused.
    Refused(SaveError),
}

/// The answer to a [`LoadMatch`].
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// The match is running from the save. `left_out` names what of the client's record could not be read.
    Loaded { path: PathBuf, tick: u64, left_out: Vec<String> },
    /// The file cannot be loaded here; `reason` is the container's message. A running match is still running.
    Refused { path: PathBuf, reason: String },
}

/// Whether Save is allowed now (V7), refreshed every frame from cheap state.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SaveAvailability(pub Option<SaveBlock>);

/// Where saves go; `None` is `saves/` in the user data directory.
#[derive(Resource, Debug, Clone, Default)]
pub struct SaveDir(pub Option<PathBuf>);

/// A lookup of a map file by its path, for tests; in the game the mount is read. Returns the bytes.
#[derive(Resource, Clone)]
pub struct MapFiles(pub Arc<MapLookup>);

/// A map file's path (as the mount names it, `bzn/<name>.bzn`) to its bytes.
pub type MapLookup = dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync;

/// Controllers a load can build besides `sta_ai`'s registry (tests register theirs): a registry name and
/// a seat to a new controller, which then takes the saved record back through `restore_state`. Nothing
/// in the game inserts it; when present it is consulted before the registry, so it can shadow a name.
#[derive(Resource, Clone)]
pub struct ExtraControllers(pub Arc<MakeController>);

/// A registry name and a seat to a new controller, or `None` for a name this does not know.
pub type MakeController = dyn Fn(&str, PlayerId) -> Option<Box<dyn Controller>> + Send + Sync;

/// 64-bit FNV-1a of the map file's bytes: the hash the save header names the map by.
pub fn file_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// What a match needs to know about itself for Save, found once when its link is up and kept until
/// the match ends: the map file's path and hash (the file is read once) and whether every computer
/// player can be saved. The per-frame availability check reads this, not the disk or the controllers.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct MatchFacts {
    map: Option<(String, u64)>,
    controller: Option<PlayerId>,
}

/// Pairs the computer seats of a game with the controllers its link holds, which `game::controllers`
/// builds in seat order and without a seat whose name the registry does not know. With as many
/// controllers as computer seats it is by order; with fewer, a seat whose name is not registered has
/// none. `None` for a seat means: no controller found.
fn pair_controllers(seats: &[SeatRec], controllers: usize) -> Vec<(PlayerId, Option<usize>)> {
    let ai: Vec<(PlayerId, &str)> = seats
        .iter()
        .filter_map(|s| match &s.control {
            ControlRec::Ai(name) => Some((s.id, name.as_str())),
            _ => None,
        })
        .collect();
    if ai.len() == controllers {
        return ai.iter().enumerate().map(|(i, (id, _))| (*id, Some(i))).collect();
    }
    let known = sta_ai::Registry::builtin().names();
    let mut next = 0;
    ai.iter()
        .map(|(id, name)| {
            if known.contains(name) && next < controllers {
                next += 1;
                (*id, Some(next - 1))
            } else {
                (*id, None)
            }
        })
        .collect()
}

/// What the gate and the save need to know about the world.
#[derive(SystemParam)]
struct Env<'w> {
    active: Option<Res<'w, ActiveMatch>>,
    net: Option<Res<'w, crate::game::net_match::NetGame>>,
    mission: Option<Res<'w, crate::game::mission_host::MissionRun>>,
    editing: Option<Res<'w, crate::editor::Editing>>,
    authoring: Option<Res<'w, crate::editor::AuthoringRun>>,
    cine: Option<Res<'w, crate::ui::presentation::CineractiveState>>,
    movies: Option<Res<'w, crate::media::Movies>>,
    mounted: Option<Res<'w, Mounted>>,
    map_files: Option<Res<'w, MapFiles>>,
    meta: Option<Res<'w, crate::capture::MatchMeta>>,
    dir: Option<Res<'w, SaveDir>>,
    facts: Option<Res<'w, MatchFacts>>,
}

impl Env<'_> {
    fn dir(&self) -> Option<PathBuf> {
        match self.dir.as_deref().and_then(|d| d.0.clone()) {
            Some(d) => Some(d),
            None => crate::platform::data_dir().map(|d| d.join("saves")),
        }
    }

    /// A map file by the exact path the mount names it by.
    fn read_map(&self, path: &str) -> Option<Vec<u8>> {
        match &self.map_files {
            Some(m) => (m.0)(path),
            None => self.mounted.as_deref().and_then(|m| m.0.read(path)).and_then(Result::ok).map(|b| b.to_vec()),
        }
    }

    /// The path and hash of the map file a match on `name` was built from.
    fn find_map(&self, name: &str) -> Option<(String, u64)> {
        ["bzn", "sol"].into_iter().find_map(|ext| {
            let path = format!("bzn/{name}.{ext}");
            self.read_map(&path).map(|bytes| (path, file_hash(&bytes)))
        })
    }

    /// V7: why this match cannot be saved, if it cannot. Reads flags and the cached [`MatchFacts`] only.
    fn block(&self, link: Option<&SimLink>) -> Option<SaveBlock> {
        let (Some(link), Some(active)) = (link, self.active.as_deref()) else { return Some(SaveBlock::NotInMatch) };
        if self.net.is_some() {
            return Some(SaveBlock::Networked);
        }
        if self.mission.is_some() {
            return Some(SaveBlock::Mission);
        }
        if link.is_observer() || active.local.is_none() {
            return Some(SaveBlock::Observer);
        }
        if link.mcp.is_some() {
            return Some(SaveBlock::McpSeat);
        }
        if link.replay().is_none() {
            return Some(SaveBlock::NoJournal);
        }
        if self.cine.as_deref().is_some_and(|c| c.on) {
            return Some(SaveBlock::Cutscene);
        }
        if self.movies.as_deref().is_some_and(|m| m.fullscreen_busy()) {
            return Some(SaveBlock::Movie);
        }
        let Some(facts) = self.facts.as_deref() else { return Some(SaveBlock::NotInMatch) };
        if let Some(seat) = facts.controller {
            return Some(SaveBlock::Controller(seat));
        }
        if self.editing.is_some() || self.authoring.is_some() || facts.map.is_none() {
            return Some(SaveBlock::NoMapFile);
        }
        if self.dir().is_none() {
            return Some(SaveBlock::NoSaveFolder);
        }
        None
    }
}

/// Finds the match's facts once, when its link is up (reads the map file once, asks each computer
/// player once whether it can be saved). Removed when the front end is entered.
fn cache_facts(mut commands: Commands, env: Env<'_>, link: Option<ResMut<SimLink>>) {
    if env.facts.is_some() {
        return;
    }
    let (Some(mut link), Some(active)) = (link, env.active.as_deref()) else { return };
    let link = link.bypass_change_detection();
    if link.replay().is_none() {
        return;
    }
    let seats = link.replay().map(|r| r.players.clone()).unwrap_or_default();
    let refs = link.controller_refs();
    let controller = pair_controllers(&seats, refs.len())
        .into_iter()
        .find(|(_, i)| !i.and_then(|i| refs.get(i)).is_some_and(|c| c.save_state().is_some()))
        .map(|(seat, _)| seat);
    commands.insert_resource(MatchFacts { map: env.find_map(&active.setup.map.name), controller });
}

fn clear_match_state(mut commands: Commands, fog: Option<ResMut<FogMemory>>) {
    commands.remove_resource::<MatchFacts>();
    // A saved memory no update took in belongs to the match that is over.
    if let Some(mut f) = fog {
        f.import = None;
    }
}

fn refresh_availability(link: Option<Res<SimLink>>, env: Env<'_>, mut out: ResMut<SaveAvailability>) {
    let block = env.block(link.as_deref());
    out.set_if_neq(SaveAvailability(block));
}

fn install_id(env: &Env<'_>) -> (String, String) {
    let mount = env.meta.as_deref().map(|m| &m.mount);
    let root = mount.and_then(|m| m.install_root.as_deref()).and_then(|r| std::path::Path::new(r).file_name()).map(|n| n.to_string_lossy().into_owned());
    (root.unwrap_or_default(), mount.and_then(|m| m.active_mod.clone()).unwrap_or_default())
}

#[allow(clippy::too_many_arguments)]
fn save_match(
    mut requests: MessageReader<SaveMatch>,
    link: Option<ResMut<SimLink>>,
    queue: Option<ResMut<CommandQueue>>,
    env: Env<'_>,
    camera: Query<&Orbit, MainCamera>,
    fog: Option<Res<FogMemory>>,
    mut out: MessageWriter<SaveOutcome>,
) {
    let Some(req) = requests.read().last().cloned() else { return };
    let mut link = link;
    if let Some(why) = env.block(link.as_deref()) {
        out.write(SaveOutcome::NotAllowed(why));
        return;
    }
    let (Some(link), Some(active), Some(dir), Some(facts)) = (link.as_deref_mut(), env.active.as_deref(), env.dir(), env.facts.as_deref()) else { return };
    let Some((path, hash)) = facts.map.clone() else { return };
    // What input queued this frame goes to the router first, so it is in flight, not lost.
    if let Some(mut q) = queue {
        for kind in q.0.drain(..) {
            link.submit(kind);
        }
    }
    let Some(snapshot) = link.snapshot() else {
        out.write(SaveOutcome::Refused(SaveError::Invalid("this world cannot make a snapshot".into())));
        return;
    };
    let sim = match sta_sim::Sim::from_snapshot(&snapshot, Some(active.setup.content.clone())) {
        Ok(s) => s,
        Err(e) => {
            out.write(SaveOutcome::Refused(SaveError::Invalid(e.to_string())));
            return;
        }
    };
    let Some(replay) = link.replay().cloned() else { return };
    let (install_key, mod_key) = install_id(&env);
    let tick = sim.tick;
    let info = SaveInfo {
        build_id: crate::capture::build_info().version,
        install_key,
        mod_key,
        map: Some(MapId { path, hash }),
        kind: GameKind::Skirmish,
        local_seat: active.local,
        play_time_secs: tick / sta_sim::TICK_HZ as u64,
        made_at: sta_bench::capture::now_unix(),
        description: req.description.clone(),
        ..SaveInfo::default()
    };
    let record = ClientRecord {
        camera: camera.iter().next().map(|o| CameraRec { focus: o.focus.to_array(), yaw: o.yaw, pitch: o.pitch, dist: o.dist }),
        explored: fog.as_deref().and_then(FogMemory::explored),
        speed: active.speed,
        fog: active.fog,
    };
    let client = ClientState { version: record::VERSION, data: record.encode() };
    // Between ticks, after the controllers thought about the last step: what they and the player
    // submitted for the next one is part of the game and goes into the container, with the counter.
    let in_flight = InFlight { next_seq: link.next_seq(), commands: link.in_flight().to_vec() };
    let refs = link.controller_refs();
    let seated: Vec<(PlayerId, &dyn Controller)> =
        pair_controllers(&replay.players, refs.len()).into_iter().filter_map(|(seat, i)| Some((seat, refs.get(i?)?.as_ref()))).collect();
    let bytes = match container::save(&sim, &replay, &info, &seated, &in_flight, &client) {
        Ok(b) => b,
        Err(e) => {
            out.write(SaveOutcome::Refused(e));
            return;
        }
    };
    let written = match &req.overwrite {
        Some(name) => container::store::write_atomic(&dir, name, &bytes),
        None => container::store::write_new(&dir, &bytes),
    };
    out.write(match written {
        Ok(path) => SaveOutcome::Saved { path, tick },
        Err(e) => SaveOutcome::Refused(e),
    });
}

/// A save fully read, checked and restored, waiting for the match to be built.
struct Prepared {
    sim: sta_sim::Sim,
    replay: Replay,
    controllers: Vec<Box<dyn Controller>>,
    in_flight: InFlight,
    record: ClientRecord,
    left_out: Vec<String>,
    tick: u64,
}

/// A load waiting for its match to be built; tied to the one start it wrote.
#[derive(Resource)]
struct PendingLoad {
    path: PathBuf,
    /// The start this load wrote: the match that may receive the prepared game is this one only.
    start: StartGame,
    stage: Stage,
    /// Frames in the front end with the start written and no match built from it.
    waiting: u32,
    /// The match of this start has been seen.
    seen: bool,
    prepared: Mutex<Option<Prepared>>,
}

#[derive(PartialEq, Eq)]
enum Stage {
    /// A running match is being left; the start follows in the front end.
    Leaving,
    /// `StartGame` is written; waiting for the build.
    Started,
}

/// Frames the front end waits for a written start to become a match before the load is given up.
const START_PATIENCE: u32 = 30;

fn setup_of(header: &container::Header, map: &Arc<sta_content::MapDef>, content: Arc<sta_content::ContentDb>) -> GameSetup {
    let seat = |s: &SeatRec| PlayerSetup {
        id: s.id,
        team: s.team,
        race: s.race.clone(),
        control: match &s.control {
            ControlRec::Human => Control::Human,
            ControlRec::Remote => Control::Remote,
            ControlRec::Ai(n) => Control::Ai(n.clone()),
            ControlRec::RemoteAi(n) => Control::RemoteAi(n.clone()),
            ControlRec::Idle => Control::Idle,
        },
    };
    GameSetup { seed: header.seed, map: map.clone(), content, players: header.seats.iter().map(seat).collect(), options: header.options.clone() }
}

/// The content the match will run on: the mount's, with the tech tree the map names (as the build
/// does). Without a mount (tests) the front end's content stands.
fn match_content(env: &Env<'_>, screen: &ScreenData, map_bytes: &[u8], name: &str) -> Arc<sta_content::ContentDb> {
    match (env.map_files.is_none(), env.mounted.as_deref(), sta_bzn::parse(map_bytes)) {
        (true, Some(mount), Ok(map)) => crate::map::with_map_tech_tree(screen.content.clone(), &mount.0, &map.tech_tree_file, name, false),
        _ => screen.content.clone(),
    }
}

/// Reads and checks a save completely, without touching the running match. Every refusal that can be
/// known from the file, the content, the map and the registry is found here.
type Ready = (Prepared, GameSetup, crate::frontend::screens::SetupFog, crate::clock::MatchSpeed);

fn prepare(env: &Env<'_>, screen: Option<&ScreenData>, extra: Option<&ExtraControllers>, path: &std::path::Path) -> Result<Ready, String> {
    let bytes = container::store::read_file(path).map_err(|e| e.to_string())?;
    let header = container::read_header(&bytes).map_err(|e| e.to_string())?;
    if let Some(e) = header.refusal(None) {
        return Err(e.to_string());
    }
    let screen = screen.ok_or("no game content is loaded")?;
    let name = header.map.path.rsplit(['/', '\\']).next().unwrap_or(&header.map.path).rsplit_once('.').map_or("", |(stem, _)| stem);
    let missing = || SaveError::MapMissing { path: header.map.path.clone() }.to_string();
    let entry = screen.maps.iter().find(|m| m.def.name == name).ok_or_else(missing)?;
    let map_bytes = env.read_map(&header.map.path).ok_or_else(missing)?;
    let hash = file_hash(&map_bytes);
    if hash != header.map.hash {
        return Err(SaveError::MapChanged { path: header.map.path.clone(), saved: header.map.hash, given: hash }.to_string());
    }
    let content = match_content(env, screen, &map_bytes, name);
    let setup = setup_of(&header, &entry.def, content.clone());
    let loaded = container::load(&bytes, content, Some(MapSource { hash, map: entry.def.clone() })).map_err(|e| e.to_string())?;
    let (record, left_out) = match ClientRecord::decode(loaded.client.version, &loaded.client.data) {
        Ok(ok) => ok,
        Err(why) => (ClientRecord::default(), vec![format!("the whole client record ({why})")]),
    };
    // A computer player is built by name as at a match start, then takes its record back.
    let registry = sta_ai::Registry::builtin();
    let boxes = loaded
        .restore_controllers(|seat, name, state| {
            let mut ctl = match extra.and_then(|e| (e.0)(name, seat)) {
                Some(c) => c,
                None => registry.create_for_seat(name, &setup, seat).map_err(|e| e.to_string())?,
            };
            ctl.restore_state(state)?;
            Ok(ctl)
        })
        .map_err(|e| e.to_string())?;
    let (fog, speed) = (record.fog, record.speed);
    let prepared = Prepared {
        tick: loaded.sim.tick,
        sim: loaded.sim,
        replay: loaded.replay,
        controllers: boxes.into_iter().map(|(_, c)| c).collect(),
        in_flight: loaded.in_flight,
        record,
        left_out,
    };
    Ok((prepared, setup, fog, speed))
}

#[allow(clippy::too_many_arguments)]
fn request_load(
    mut requests: MessageReader<LoadMatch>,
    mut commands: Commands,
    state: Option<Res<State<AppState>>>,
    screen: Option<Res<ScreenData>>,
    env: Env<'_>,
    extra: Option<Res<ExtraControllers>>,
    mut start: MessageWriter<StartGame>,
    mut leave: MessageWriter<LeaveMatch>,
    mut out: MessageWriter<LoadOutcome>,
    mut notices: MessageWriter<Notice>,
) {
    let Some(req) = requests.read().last().cloned() else { return };
    let mut refuse = |why: String| {
        notices.write(Notice::new(Severity::Bad, format!("Cannot load: {why}")));
        out.write(LoadOutcome::Refused { path: req.path.clone(), reason: why });
    };
    let in_match = match state.as_deref().map(|s| *s.get()) {
        Some(AppState::InGame) => true,
        Some(AppState::FrontEnd) => false,
        _ => return refuse("a game can be loaded from the front end or from a running game".into()),
    };
    let prepared = prepare(&env, screen.as_deref(), extra.as_deref(), &req.path);
    let (prepared, setup, fog, speed) = match prepared {
        Ok(p) => p,
        Err(why) => return refuse(why),
    };
    // Only now, with a save that will load, is a running match left.
    let start_game = StartGame(setup, speed, None, fog, None);
    if in_match {
        leave.write(LeaveMatch);
    } else {
        start.write(start_game.clone());
    }
    commands.insert_resource(PendingLoad {
        path: req.path,
        start: start_game,
        stage: if in_match { Stage::Leaving } else { Stage::Started },
        waiting: 0,
        seen: false,
        prepared: Mutex::new(Some(prepared)),
    });
}

/// Back at the front end after leaving a running match: the start is written now.
fn begin_pending_load(pending: Option<ResMut<PendingLoad>>, mut start: MessageWriter<StartGame>) {
    if let Some(mut p) = pending
        && p.stage == Stage::Leaving
    {
        start.write(p.start.clone());
        p.stage = Stage::Started;
    }
}

/// Whether `active` is the match the pending load's start asks for.
fn is_this_start(start: &StartGame, active: &ActiveMatch) -> bool {
    let (a, b) = (&start.0, &active.setup);
    a.seed == b.seed && a.map.name == b.map.name && a.players == b.players
}

/// A load is tied to its one start: given up (with a refusal) when that start fails or another match
/// starts, so it can never be delivered to a later, ordinary match.
fn expire_pending(
    mut commands: Commands,
    pending: Option<ResMut<PendingLoad>>,
    state: Option<Res<State<AppState>>>,
    active: Option<Res<ActiveMatch>>,
    mut out: MessageWriter<LoadOutcome>,
    mut notices: MessageWriter<Notice>,
) {
    let (Some(mut p), Some(state)) = (pending, state) else { return };
    if p.stage != Stage::Started {
        return;
    }
    let why = match active.as_deref() {
        Some(a) if is_this_start(&p.start, a) => {
            p.seen = true;
            return;
        }
        Some(_) => "another game was started first",
        None if *state.get() == AppState::FrontEnd && (p.seen || p.waiting >= START_PATIENCE) => "the game could not be started",
        None => {
            if *state.get() == AppState::FrontEnd {
                p.waiting += 1;
            }
            return;
        }
    };
    notices.write(Notice::new(Severity::Bad, format!("Cannot load: {why}")));
    out.write(LoadOutcome::Refused { path: p.path.clone(), reason: why.to_owned() });
    commands.remove_resource::<PendingLoad>();
}

#[derive(SystemParam)]
struct Resume<'w, 's> {
    selection: Option<ResMut<'w, Selection>>,
    queue: Option<ResMut<'w, CommandQueue>>,
    camera: Query<'w, 's, &'static mut Orbit, MainCamera>,
    fog: Option<ResMut<'w, FogMemory>>,
    clock: Option<ResMut<'w, crate::clock::AnimClock>>,
    active: Option<Res<'w, ActiveMatch>>,
    scene: Option<Res<'w, crate::frontend::scene::MenuScene>>,
}

/// The build has put the match's link up: the prepared game replaces what it was built with.
#[allow(clippy::too_many_arguments)]
fn finish_pending_load(
    mut commands: Commands,
    pending: Option<Res<PendingLoad>>,
    link: Option<ResMut<SimLink>>,
    state: Option<Res<State<AppState>>>,
    mut r: Resume<'_, '_>,
    mut out: MessageWriter<LoadOutcome>,
    mut notices: MessageWriter<Notice>,
) {
    let (Some(pending), Some(mut link), Some(state)) = (pending, link, state) else { return };
    let ready = pending.stage == Stage::Started
        && matches!(state.get(), AppState::Loading | AppState::InGame)
        && r.scene.is_none()
        && r.active.as_deref().is_some_and(|a| a.requested && is_this_start(&pending.start, a))
        && link.replay().is_some();
    if !ready {
        return;
    }
    commands.remove_resource::<PendingLoad>();
    let Some(p) = pending.prepared.lock().ok().and_then(|mut g| g.take()) else { return };
    link.restore(Restored { sim: p.sim, replay: p.replay, controllers: p.controllers, in_flight: p.in_flight.commands, next_seq: p.in_flight.next_seq });
    // What the client derives from the sim starts as on a fresh start.
    if let Some(mut s) = r.selection {
        s.0.clear();
    }
    if let Some(mut q) = r.queue {
        q.0.clear();
    }
    if let Some(c) = p.record.camera {
        for mut o in &mut r.camera {
            (o.focus, o.yaw, o.pitch, o.dist) = (Vec3::from(c.focus), c.yaw, c.pitch, c.dist);
        }
    }
    if let Some(mut f) = r.fog {
        f.import = p.record.explored;
    }
    if let Some(mut clock) = r.clock {
        clock.set_match_speed(p.record.speed);
    }
    if !p.left_out.is_empty() {
        notices.write(Notice::new(Severity::Warn, format!("Loaded with defaults for: {}", p.left_out.join(", "))));
    }
    out.write(LoadOutcome::Loaded { path: pending.path.clone(), tick: p.tick, left_out: p.left_out });
}

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SaveMatch>()
            .add_message::<LoadMatch>()
            .add_message::<SaveOutcome>()
            .add_message::<LoadOutcome>()
            .add_message::<Notice>()
            .add_message::<LeaveMatch>()
            .add_message::<StartGame>()
            .init_resource::<SaveAvailability>()
            .add_systems(OnEnter(AppState::FrontEnd), clear_match_state)
            .add_systems(
                Update,
                (
                    cache_facts,
                    // After the fog's update (Render): its memory is of the tick being saved.
                    refresh_availability,
                    save_match,
                    request_load,
                    begin_pending_load.run_if(in_state(AppState::FrontEnd)),
                    expire_pending,
                    finish_pending_load,
                )
                    .chain()
                    .in_set(GameSet::Ui),
            );
    }
}
