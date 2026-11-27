//! The client's save and load path (UI-55, ADR 0010 V3, the client side of V5, V7).
//!
//! - **Entry.** [`SaveMatch`] and [`LoadMatch`] are the whole interface: the menus (UI-56), tests and
//!   a console write them; [`SaveOutcome`] and [`LoadOutcome`] answer. [`SaveAvailability`] says,
//!   every frame, whether Save is allowed and, if not, why (V7).
//! - **Save.** The match's snapshot, journal, in-flight commands, computer players and the client's
//!   record go through [`container`] (the sim crate's save container, SIM-81) into `saves/` in the
//!   user data directory, atomically. A match whose map has no file is refused.
//! - **Load.** A save starts a match through the same `StartGame` -> loading screen -> match build as
//!   Instant Action (seats, options and seed from the header, the map from the mount by the header's
//!   path); once the build has put its link up, the saved sim, journal and controllers replace the
//!   link's ([`SimLink::restore`](crate::sim_link::SimLink)), and the client record is applied:
//!   camera, explored memory, game speed. Selection starts empty. Any refusal tears the fresh
//!   match down again and is reported.
//! - **Client record** ([`record`], version 1): camera pose, the local seat's explored memory, game
//!   speed, commands in flight. Control groups are not saved: they live in `input/`, under another
//!   package's claim (#960); they come back empty. An unreadable record never refuses a load.

pub mod container;
pub mod record;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use sta_core::PlayerId;
use sta_sim::replay::{ControlRec, SeatRec};
use sta_sim::setup::{Control, GameSetup, PlayerSetup};

use crate::app::{AppState, GameSet};
use crate::frontend::screens::{ScreenData, StartGame};
use crate::game::flow::{ActiveMatch, LeaveMatch};
use crate::map::MainCamera;
use crate::render::fog::FogMemory;
use crate::sim_link::{CommandQueue, ContentRes, Restored, Selection, SimLink};
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

/// Load a save: from the front end or from a running match (which is left first).
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
    /// The file cannot be loaded here; `reason` is the container's message.
    Refused { path: PathBuf, reason: String },
}

/// Whether Save is allowed now (V7), refreshed every frame in a match.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SaveAvailability(pub Option<SaveBlock>);

/// Where saves go; `None` is `saves/` in the user data directory.
#[derive(Resource, Debug, Clone, Default)]
pub struct SaveDir(pub Option<PathBuf>);

/// A lookup of a map file by name, for tests; in the game the mount is read. Returns the file's path
/// and bytes.
#[derive(Resource, Clone)]
pub struct MapFiles(pub Arc<MapLookup>);

/// A map name to the map file's path and bytes.
pub type MapLookup = dyn Fn(&str) -> Option<(String, Vec<u8>)> + Send + Sync;

/// Stand-in only (until SIM-81's `Controller::save_state` and `restore_state` are on main): how to read
/// a computer seat's record and how to build its controller from one. Tests register it; nothing in the
/// game does, so computer seats stay refused. Deleted by the wiring commit.
#[derive(Resource, Clone)]
pub struct StandInControllers {
    pub record: Arc<RecordFn>,
    pub restore: Arc<RestoreFn>,
}

/// A seat to its controller's record `(version, bytes)`.
pub type RecordFn = dyn Fn(PlayerId) -> Option<(u32, Vec<u8>)> + Send + Sync;
/// A seat, the registry name and a record to the controller they describe.
pub type RestoreFn = dyn Fn(PlayerId, &str, u32, &[u8]) -> Result<Box<dyn sta_sim::setup::Controller>, String> + Send + Sync;

/// 64-bit FNV-1a of the map file's bytes: the hash the save header names the map by.
pub fn file_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn mount_map_file(vfs: &sta_vfs::Vfs, name: &str) -> Option<(String, Vec<u8>)> {
    ["bzn", "sol"].into_iter().find_map(|ext| {
        let path = format!("bzn/{name}.{ext}");
        vfs.read(&path).and_then(Result::ok).map(|b| (path, b.to_vec()))
    })
}

/// The map name a save header's path names.
fn map_name(path: &str) -> &str {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

/// What the gate and the save need to know about the world.
#[derive(SystemParam)]
struct Env<'w> {
    active: Option<Res<'w, ActiveMatch>>,
    content: Option<Res<'w, ContentRes>>,
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
    stand_in: Option<Res<'w, StandInControllers>>,
}

impl Env<'_> {
    fn dir(&self) -> Option<PathBuf> {
        match self.dir.as_deref().and_then(|d| d.0.clone()) {
            Some(d) => Some(d),
            None => crate::platform::data_dir().map(|d| d.join("saves")),
        }
    }

    fn map_file(&self, name: &str) -> Option<(String, u64)> {
        let found = match &self.map_files {
            Some(m) => (m.0)(name),
            None => self.mounted.as_deref().and_then(|m| mount_map_file(&m.0, name)),
        };
        found.map(|(path, bytes)| (path, file_hash(&bytes)))
    }

    /// V7: why this match cannot be saved, if it cannot.
    fn block(&self, link: Option<&mut SimLink>) -> Option<SaveBlock> {
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
        let Some(seats) = link.replay().map(|r| r.players.clone()) else { return Some(SaveBlock::NoJournal) };
        if self.cine.as_deref().is_some_and(|c| c.on) {
            return Some(SaveBlock::Cutscene);
        }
        if self.movies.as_deref().is_some_and(|m| m.fullscreen_busy()) {
            return Some(SaveBlock::Movie);
        }
        let ai: Vec<PlayerId> = seats.iter().filter(|s| matches!(s.control, ControlRec::Ai(_))).map(|s| s.id).collect();
        let refs = link.controller_refs();
        for (i, seat) in ai.iter().enumerate() {
            let recorded = self.stand_in.as_deref().is_some_and(|s| (s.record)(*seat).is_some());
            if !refs.get(i).is_some_and(|c| recorded || container::controller_can_save(c.as_ref())) {
                return Some(SaveBlock::Controller(*seat));
            }
        }
        if self.editing.is_some() || self.authoring.is_some() || self.map_file(&active.setup.map.name).is_none() {
            return Some(SaveBlock::NoMapFile);
        }
        if self.dir().is_none() {
            return Some(SaveBlock::NoSaveFolder);
        }
        None
    }
}

fn refresh_availability(link: Option<ResMut<SimLink>>, env: Env<'_>, mut out: ResMut<SaveAvailability>) {
    let mut link = link;
    let block = env.block(link.as_mut().map(|l| l.bypass_change_detection()));
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
    if let Some(why) = env.block(link.as_deref_mut()) {
        out.write(SaveOutcome::NotAllowed(why));
        return;
    }
    let (Some(link), Some(active), Some(dir)) = (link.as_deref_mut(), env.active.as_deref(), env.dir()) else { return };
    // What input queued this frame goes to the router first, so it is in flight, not lost.
    if let Some(mut q) = queue {
        for kind in q.0.drain(..) {
            link.submit(kind);
        }
    }
    let content = env.content.as_deref().map_or_else(|| active.setup.content.clone(), |c| c.0.clone());
    let Some(snapshot) = link.snapshot() else {
        out.write(SaveOutcome::Refused(SaveError::Invalid("this world cannot make a snapshot".into())));
        return;
    };
    let sim = match sta_sim::Sim::from_snapshot(&snapshot, Some(content)) {
        Ok(s) => s,
        Err(e) => {
            out.write(SaveOutcome::Refused(SaveError::Invalid(e.to_string())));
            return;
        }
    };
    let Some(replay) = link.replay().cloned() else { return };
    let Some((path, hash)) = env.map_file(&active.setup.map.name) else { return };
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
    };
    let record = ClientRecord {
        camera: camera.iter().next().map(|o| CameraRec { focus: o.focus.to_array(), yaw: o.yaw, pitch: o.pitch, dist: o.dist }),
        explored: fog.as_deref().and_then(FogMemory::explored),
        speed: active.speed,
    };
    let client = ClientState { version: record::VERSION, data: record.encode() };
    // Between ticks, after the controllers thought about the last step: what they and the player
    // submitted for the next one is part of the game and goes into the container, with the counter.
    let in_flight = InFlight { next_seq: link.next_seq(), commands: link.in_flight().to_vec() };
    // Stand-in only: the registered records. The wiring commit passes the controllers to `container::save`.
    let records: Vec<container::SeatController> = replay
        .players
        .iter()
        .filter_map(|s| match (&s.control, env.stand_in.as_deref()) {
            (ControlRec::Ai(name), Some(si)) => (si.record)(s.id).map(|(version, data)| container::SeatController { seat: s.id, name: name.clone(), version, data }),
            _ => None,
        })
        .collect();
    let bytes = match container::save_with_records(&sim, &replay, &info, &records, &in_flight, &client) {
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

/// A load waiting for the match to be built.
#[derive(Resource)]
struct PendingLoad {
    path: PathBuf,
    bytes: Vec<u8>,
    map_hash: u64,
    start: StartGame,
    stage: Stage,
}

#[derive(PartialEq, Eq)]
enum Stage {
    /// A running match is being left; the start follows in the front end.
    Leaving,
    /// `StartGame` is written; waiting for the build.
    Started,
}

fn setup_of(header: &container::Header, map: &sta_content::MapDef, content: Arc<sta_content::ContentDb>) -> GameSetup {
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
    GameSetup { seed: header.seed, map: Arc::new(map.clone()), content, players: header.seats.iter().map(seat).collect(), options: header.options.clone() }
}

#[allow(clippy::too_many_arguments)]
fn request_load(
    mut requests: MessageReader<LoadMatch>,
    mut commands: Commands,
    state: Option<Res<State<AppState>>>,
    screen: Option<Res<ScreenData>>,
    env: Env<'_>,
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
    let prepared = (|| -> Result<(Vec<u8>, u64, StartGame), String> {
        let bytes = container::store::read_file(&req.path).map_err(|e| e.to_string())?;
        let header = container::read_header(&bytes).map_err(|e| e.to_string())?;
        if let Some(e) = header.refusal(None) {
            return Err(e.to_string());
        }
        let screen = screen.as_deref().ok_or("no game content is loaded")?;
        let name = map_name(&header.map.path);
        let missing = || SaveError::MapMissing { path: header.map.path.clone() }.to_string();
        let entry = screen.maps.iter().find(|m| m.def.name == name).ok_or_else(missing)?;
        let (_, hash) = env.map_file(name).ok_or_else(missing)?;
        if hash != header.map.hash {
            return Err(SaveError::MapChanged { path: header.map.path.clone(), saved: header.map.hash, given: hash }.to_string());
        }
        let setup = setup_of(&header, &entry.def, screen.content.clone());
        Ok((bytes, hash, StartGame(setup, crate::clock::MatchSpeed::Normal, None, Default::default(), None)))
    })();
    let (bytes, map_hash, start_game) = match prepared {
        Ok(p) => p,
        Err(why) => return refuse(why),
    };
    let in_match = state.is_some_and(|s| *s.get() == AppState::InGame);
    if in_match {
        leave.write(LeaveMatch);
    } else {
        start.write(start_game.clone());
    }
    commands.insert_resource(PendingLoad { path: req.path, bytes, map_hash, start: start_game, stage: if in_match { Stage::Leaving } else { Stage::Started } });
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

#[derive(SystemParam)]
struct Resume<'w, 's> {
    selection: Option<ResMut<'w, Selection>>,
    queue: Option<ResMut<'w, CommandQueue>>,
    camera: Query<'w, 's, &'static mut Orbit, MainCamera>,
    fog: Option<ResMut<'w, FogMemory>>,
    clock: Option<ResMut<'w, crate::clock::AnimClock>>,
    active: Option<ResMut<'w, ActiveMatch>>,
    content: Option<Res<'w, ContentRes>>,
    stand_in: Option<Res<'w, StandInControllers>>,
    scene: Option<Res<'w, crate::frontend::scene::MenuScene>>,
}

/// A load failed after the match was built: back to the front end, which takes the match down.
#[derive(Resource)]
struct Aborting;

fn abort_load(mut commands: Commands, state: Res<State<AppState>>, mut next: ResMut<NextState<AppState>>) {
    if *state.get() == AppState::FrontEnd {
        commands.remove_resource::<Aborting>();
    } else {
        next.set(AppState::FrontEnd);
    }
}

/// The build has put the match's link up: the saved game replaces what it was built with.
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
        && r.active.as_deref().is_some_and(|a| a.requested)
        && link.replay().is_some();
    if !ready {
        return;
    }
    commands.remove_resource::<PendingLoad>();
    let mut fail = |why: String| {
        notices.write(Notice::new(Severity::Bad, format!("Cannot load: {why}")));
        out.write(LoadOutcome::Refused { path: pending.path.clone(), reason: why });
        // The match the build made is taken down again, whichever state it is in.
        commands.insert_resource(Aborting);
    };
    let Some(active) = r.active.as_deref_mut() else { return };
    let content = r.content.as_deref().map_or_else(|| active.setup.content.clone(), |c| c.0.clone());
    let Some(map) = link.replay().map(|rep| rep.map.clone()) else { return };
    let loaded = match container::load(&pending.bytes, content, Some(MapSource { hash: pending.map_hash, map: Arc::new(map) })) {
        Ok(l) => l,
        Err(e) => return fail(e.to_string()),
    };
    let stand_in = r.stand_in.as_deref();
    let boxes = match loaded.restore_controllers(|seat, name, version, data| match stand_in {
        Some(s) => (s.restore)(seat, name, version, data),
        None => Err(format!("seat {} cannot be restored by this build", seat.0)),
    }) {
        Ok(b) => b,
        Err(e) => return fail(e.to_string()),
    };
    let (rec, left_out) = match ClientRecord::decode(loaded.client.version, &loaded.client.data) {
        Ok(ok) => ok,
        Err(why) => (ClientRecord::default(), vec![format!("the whole client record ({why})")]),
    };
    let tick = loaded.sim.tick;
    link.restore(Restored {
        sim: loaded.sim,
        replay: loaded.replay,
        controllers: boxes.into_iter().map(|(_, c)| c).collect(),
        in_flight: loaded.in_flight.commands,
        next_seq: loaded.in_flight.next_seq,
    });
    // What the client derives from the sim starts as on a fresh start.
    if let Some(mut s) = r.selection {
        s.0.clear();
    }
    if let Some(mut q) = r.queue {
        q.0.clear();
    }
    if let Some(c) = rec.camera {
        for mut o in &mut r.camera {
            (o.focus, o.yaw, o.pitch, o.dist) = (Vec3::from(c.focus), c.yaw, c.pitch, c.dist);
        }
    }
    if let Some(mut f) = r.fog {
        f.import = rec.explored;
    }
    active.speed = rec.speed;
    if let Some(mut clock) = r.clock {
        clock.set_match_speed(rec.speed);
    }
    if !left_out.is_empty() {
        notices.write(Notice::new(Severity::Warn, format!("Loaded with defaults for: {}", left_out.join(", "))));
    }
    out.write(LoadOutcome::Loaded { path: pending.path.clone(), tick, left_out });
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
            .add_systems(
                Update,
                (
                    // After the fog's update (Render): its memory is of the tick being saved.
                    refresh_availability,
                    save_match,
                    request_load,
                    begin_pending_load.run_if(in_state(AppState::FrontEnd)),
                    finish_pending_load,
                    abort_load.run_if(resource_exists::<Aborting>),
                )
                    .chain()
                    .in_set(GameSet::Ui),
            );
    }
}
