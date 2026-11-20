//! The footage log (docs/FOOTAGE_LOG.md): a timestamped JSONL file the game writes about itself,
//! so that a cutter holding a screen recording can tell, by wall clock, which game was running,
//! when a cutscene or a fight was on, who was in frame, and where the beats were.
//!
//! - Off unless `--footage-log [<path>]` or `STA64_FOOTAGE_LOG` asks for it; then the plugin is
//!   added and the file opened. Off, nothing of this module runs. A file that cannot be opened is
//!   a warning, never a crash.
//! - A read-only observer: its systems take `Res`/`Query`/message readers only, and write the
//!   file. They change nothing in the sim, the input, the picture or the determinism.
//! - Every line is flushed as it is written: a crash leaves a usable file.
//! - The pure parts (the in-frame test, the enter/leave debounce, the fight window, the `game`
//!   label, the line format) are in [`geom`] and [`log`] and are tested without Bevy.

mod geom;
mod log;

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use serde_json::{Value, json};
use sta_content::ContentDb;
use sta_core::PlayerId;
use sta_sim::event::SimEvent;
use sta_sim::present::{Presentation, Spot};
use sta_sim::view::UnitView;
use sta_sim::EntityId;

use crate::app::{AppState, GameSet, LaunchConfig};
use crate::capture::MatchMeta;
use crate::clock::AnimClock;
use crate::frontend::screens::{ActiveMod, ScreenData};
use crate::game::mission_host::MissionRun;
use crate::map::MainCamera;
use crate::media::campaign::{MissionBegan, MissionEnded};
use crate::media::{MovieFinished, MovieOutcome, MovieTarget, PlayMovie};
use crate::render::action_camera::{FIGHT_SECS, fighters};
use crate::render::units::{LocalPlayer, SelectRadius, cinematic_reveal, drawn_units};
use crate::sim_link::{ContentRes, SimEntities, SimLink, SimMsg};
use crate::ui::presentation::CineractiveState;
use crate::ui::profile::UiProfile;
use crate::viewer::Orbit;

use geom::{Edge, EdgeTracker, FightWatch, Placement, Projected};
use log::Stamps;

/// Frame evaluations per second of real time (fight in frame, cast, enter/leave).
const EVAL_SECS: f64 = 0.22;
/// A unit must be out of frame this long before a `leave`, and a fight out of frame before `fight_in_frame` off.
const LEAVE_AFTER: f64 = 0.5;
/// A `cast` after a shot or a cineractive start waits this long, for the scripted camera to arrive.
const SETTLE_SECS: f64 = 0.3;
/// Longest `cast` list; the rest is only counted in `total`.
const CAST_MAX: usize = 40;
/// The camera pose is looked at this often.
const CAMERA_SECS: f64 = 1.0;

/// `--footage-log [<path>]` (`Some("")` without a path), else `STA64_FOOTAGE_LOG`; `None`: off.
pub fn requested(cfg: &LaunchConfig) -> Option<String> {
    requested_from(cfg.footage_log.as_deref(), std::env::var("STA64_FOOTAGE_LOG").ok().as_deref())
}

fn requested_from(flag: Option<&str>, env: Option<&str>) -> Option<String> {
    if let Some(f) = flag {
        return Some(f.to_owned());
    }
    env.map(str::trim).filter(|v| !v.is_empty() && !matches!(v.to_ascii_lowercase().as_str(), "0" | "off" | "false" | "no")).map(str::to_owned)
}

/// The log file and the stamps of its records.
#[derive(Resource)]
pub struct FootageLog {
    sink: Box<dyn std::io::Write + Send + Sync>,
    stamps: Stamps,
    /// The sim tick of the match being played; `None` outside one.
    tick: Option<u64>,
    /// A match is on screen (a sim link, and `InGame` where the app has that state).
    in_match: bool,
    failed: bool,
}

impl FootageLog {
    fn new(sink: Box<dyn std::io::Write + Send + Sync>, stamps: Stamps) -> FootageLog {
        FootageLog { sink, stamps, tick: None, in_match: false, failed: false }
    }

    /// Seconds since the log started (monotonic).
    fn now(&self) -> f64 {
        self.stamps.elapsed().as_secs_f64()
    }

    fn emit(&mut self, ty: &str, fields: Value) {
        if self.failed {
            return;
        }
        let (ms, t) = self.stamps.at(self.stamps.elapsed());
        let mut l = log::line(ms, t, self.tick, ty, fields);
        l.push('\n');
        // One write per line, so a line is never torn; the file is unbuffered, so it is flushed.
        if let Err(e) = self.sink.write_all(l.as_bytes()).and_then(|()| self.sink.flush()) {
            warn!("footage log: write failed ({e}); logging stops");
            self.failed = true;
        }
    }
}

/// What the systems remember between frames.
#[derive(Resource)]
struct FootageState {
    launch_mod: Option<String>,
    started: bool,
    app_state: Option<AppState>,
    /// The match's sim link has been logged (reset when a link is added or gone).
    matched: bool,
    races: BTreeMap<PlayerId, String>,
    cine: bool,
    paused: bool,
    verdict: Option<&'static str>,
    outcome: Option<(bool, u64)>,
    fight: FightWatch<EntityId>,
    in_frame_fight: EdgeTracker<()>,
    cast: EdgeTracker<EntityId>,
    classes: BTreeMap<EntityId, String>,
    /// A `cast` to write at this real time, and why.
    pending: Option<(&'static str, f64)>,
    shots: u32,
    next_eval: f64,
    next_camera: f64,
    camera: Option<[f32; 6]>,
}

impl FootageState {
    fn new(launch_mod: Option<String>) -> FootageState {
        FootageState {
            launch_mod,
            started: false,
            app_state: None,
            matched: false,
            races: BTreeMap::new(),
            cine: false,
            paused: false,
            verdict: None,
            outcome: None,
            fight: FightWatch::new(f64::from(FIGHT_SECS)),
            in_frame_fight: EdgeTracker::new(LEAVE_AFTER),
            cast: EdgeTracker::new(LEAVE_AFTER),
            classes: BTreeMap::new(),
            pending: None,
            shots: 0,
            next_eval: 0.0,
            next_camera: 0.0,
            camera: None,
        }
    }

    /// A new match: nothing of the last one carries over.
    fn new_match(&mut self) {
        self.cine = false;
        self.paused = false;
        self.verdict = None;
        self.outcome = None;
        self.fight.clear();
        self.in_frame_fight.reset([]);
        self.cast.reset([]);
        self.classes.clear();
        self.pending = None;
        self.shots = 0;
        self.camera = None;
    }
}

/// Registers the footage log, when it was asked for.
pub struct FootagePlugin {
    /// The `--footage-log` / `STA64_FOOTAGE_LOG` value: a file, a directory, or `1` / empty for the default place.
    pub spec: String,
    pub launch_mod: Option<String>,
}

impl FootagePlugin {
    pub fn from_cfg(spec: String, cfg: &LaunchConfig) -> FootagePlugin {
        FootagePlugin { spec, launch_mod: cfg.active_mod.clone() }
    }
}

impl Plugin for FootagePlugin {
    fn build(&self, app: &mut App) {
        let stamps = Stamps::now();
        let path = log::target_path(&self.spec, stamps.started_ms(), &log::default_dir());
        match log::open(&path) {
            Ok(file) => {
                info!("footage log: {}", path.display());
                install(app, FootageLog::new(Box::new(file), stamps), self.launch_mod.clone());
            }
            Err(e) => warn!("footage log: cannot open {}: {e}; carrying on without it", path.display()),
        }
    }
}

fn install(app: &mut App, log: FootageLog, launch_mod: Option<String>) {
    app.add_message::<SimMsg>()
        .add_message::<PlayMovie>()
        .add_message::<MovieFinished>()
        .add_message::<MissionBegan>()
        .add_message::<MissionEnded>()
        .add_message::<AppExit>()
        .init_resource::<SimEntities>()
        .insert_resource(log)
        .insert_resource(FootageState::new(launch_mod))
        .add_systems(
            Update,
            (
                log_start,
                context,
                log_app_state,
                log_match,
                log_movies,
                log_mission,
                log_cutscene,
                log_events,
                log_pause,
                log_verdict,
                watch_frame,
                log_camera,
                log_exit,
            )
                .chain()
                .after(GameSet::Audio),
        );
}

/// A number as the file writes it: three decimals.
fn n3(x: f32) -> f64 {
    log::round3(f64::from(x))
}

fn state_name(s: AppState) -> &'static str {
    match s {
        AppState::Boot => "boot",
        AppState::FrontEnd => "front_end",
        AppState::Loading => "loading",
        AppState::InGame => "in_game",
        AppState::Viewer => "viewer",
    }
}

fn log_start(mut log: ResMut<FootageLog>, mut st: ResMut<FootageState>, window: Query<&Window, With<PrimaryWindow>>) {
    if std::mem::replace(&mut st.started, true) {
        return;
    }
    let size = window.single().ok().map(|w| json!({"width": w.width(), "height": w.height(), "physical_width": w.physical_width(), "physical_height": w.physical_height()}));
    let ms = log.stamps.started_ms();
    log.emit(
        "start",
        json!({
            "format": log::FORMAT,
            "build": serde_json::to_value(crate::capture::build_info()).unwrap_or(Value::Null),
            "pid": std::process::id(),
            "started_at": log::rfc3339(ms),
            "window": size,
        }),
    );
}

/// Whether a match is on screen, and its tick, for every record after this.
fn context(mut log: ResMut<FootageLog>, mut st: ResMut<FootageState>, link: Option<Res<SimLink>>, state: Option<Res<State<AppState>>>, clock: Option<Res<AnimClock>>) {
    let on = link.is_some() && state.is_none_or(|s| *s.get() == AppState::InGame);
    if link.as_ref().is_none_or(|l| l.is_added()) {
        st.matched = false;
    }
    if !on {
        st.matched = false;
    }
    log.in_match = on;
    log.tick = on.then(|| clock.map(|c| c.tick())).flatten();
}

fn log_app_state(mut log: ResMut<FootageLog>, mut st: ResMut<FootageState>, state: Option<Res<State<AppState>>>) {
    let Some(now) = state.map(|s| *s.get()) else { return };
    if st.app_state != Some(now) {
        let from = st.app_state.replace(now);
        log.emit("app_state", json!({"state": state_name(now), "from": from.map(state_name)}));
    }
}

fn control_name(c: &sta_sim::replay::ControlRec) -> String {
    use sta_sim::replay::ControlRec as C;
    match c {
        C::Human => "human".into(),
        C::Remote => "remote".into(),
        C::Ai(n) => format!("ai:{n}"),
        C::Idle => "idle".into(),
        C::RemoteAi(n) => format!("remote_ai:{n}"),
    }
}

/// The `match` record: once when a match becomes `InGame`.
#[allow(clippy::too_many_arguments)]
fn log_match(
    mut log: ResMut<FootageLog>,
    mut st: ResMut<FootageState>,
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    local: Option<Res<LocalPlayer>>,
    race: Option<Res<crate::game::LocalRace>>,
    ui: Option<Res<UiProfile>>,
    meta: Option<Res<MatchMeta>>,
    mission: Option<Res<MissionRun>>,
    active: Option<Res<ActiveMod>>,
    screens: Option<Res<ScreenData>>,
) {
    let Some(link) = link else { return };
    if !log.in_match || st.matched {
        return;
    }
    st.matched = true;
    st.new_match();
    let replay = link.replay();
    let profile = content.as_ref().map(|c| c.0.profile());
    // The mod: what was mounted, else what the front end or the command line chose.
    let folder = meta.as_ref().and_then(|m| m.mount.active_mod.clone()).or_else(|| active.as_ref().and_then(|a| a.0.clone())).or_else(|| st.launch_mod.clone());
    let display = folder.as_ref().and_then(|f| screens.as_ref()?.mods.iter().find(|m| &m.folder == f).map(|m| m.name.clone()));
    let layers: Vec<String> = meta.as_ref().map(|m| m.mount.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
    let mod_names: Vec<&str> = folder.iter().chain(display.iter()).chain(layers.iter().filter(|n| !matches!(n.as_str(), "Data" | "root"))).map(String::as_str).collect();
    let game = profile.map(|p| geom::game_label(p, &mod_names));
    let players: Vec<Value> = replay
        .map(|r| r.players.iter().map(|p| json!({"id": p.id.0, "team": p.team.0, "race": p.race, "control": control_name(&p.control)})).collect())
        .unwrap_or_default();
    st.races = replay.map(|r| r.players.iter().map(|p| (p.id, p.race.clone())).collect()).unwrap_or_default();
    let mode = if mission.is_some() {
        "mission"
    } else {
        match meta.as_ref().map(|m| m.start) {
            Some(sta_bench::capture::StartMode::Starts) => "skirmish",
            Some(sta_bench::capture::StartMode::Plain) => "map",
            None => "net",
        }
    };
    log.emit(
        "match",
        json!({
            "profile": profile.map(|p| format!("{p:?}")),
            "mod": folder,
            "mod_name": display,
            "mounted": layers,
            "game": game,
            "ui_profile": ui.map(|u| format!("{:?}", *u)),
            "map": replay.map(|r| r.map.name.clone()),
            "map_label": replay.and_then(|r| r.map.label.clone()),
            "mode": mode,
            "campaign": mission.as_ref().map(|m| m.campaign.clone()),
            "mission": mission.as_ref().map(|m| m.mission),
            "local_race": race.map(|r| r.name.clone()),
            "local_seat": local.and_then(|l| l.0).unwrap_or(link.player()).0,
            "players": players,
        }),
    );
}

fn log_movies(mut log: ResMut<FootageLog>, mut start: MessageReader<PlayMovie>, mut end: MessageReader<MovieFinished>) {
    for m in start.read() {
        let target = match m.target {
            MovieTarget::Fullscreen => "fullscreen",
            MovieTarget::Panel(_) => "panel",
        };
        log.emit("movie_start", json!({"id": m.id.0, "name": m.name, "target": target, "skippable": m.skippable}));
    }
    for m in end.read() {
        let (outcome, reason) = match &m.outcome {
            MovieOutcome::Completed => ("completed", None),
            MovieOutcome::Skipped => ("skipped", None),
            MovieOutcome::Stopped => ("stopped", None),
            MovieOutcome::Failed(why) => ("failed", Some(why.clone())),
        };
        log.emit("movie_end", json!({"id": m.id.0, "name": m.name, "outcome": outcome, "reason": reason}));
    }
}

fn log_mission(mut log: ResMut<FootageLog>, mut st: ResMut<FootageState>, mut began: MessageReader<MissionBegan>, mut ended: MessageReader<MissionEnded>, run: Option<Res<MissionRun>>) {
    if let Some(o) = run.as_ref().and_then(|r| r.outcome) {
        st.outcome = Some((o.won, o.tick));
    }
    for b in began.read() {
        log.emit("mission_begin", json!({"map": b.map, "campaign": run.as_ref().map(|r| r.campaign.clone()), "mission": run.as_ref().map(|r| r.mission)}));
    }
    for _ in ended.read() {
        let outcome = st.outcome.map(|(won, _)| if won { "won" } else { "lost" });
        log.emit("mission_end", json!({"outcome": outcome, "outcome_tick": st.outcome.map(|(_, t)| t), "campaign": run.as_ref().map(|r| r.campaign.clone()), "mission": run.as_ref().map(|r| r.mission)}));
        st.outcome = None;
    }
}

/// The edges of a script's cineractive; a skip by the player ends one with no sim event, so the state is watched.
fn log_cutscene(mut log: ResMut<FootageLog>, mut st: ResMut<FootageState>, cine: Option<Res<CineractiveState>>) {
    let now = cine.as_deref().copied().unwrap_or_default();
    if now.on != st.cine {
        st.cine = now.on;
        log.emit("cutscene", json!({"on": now.on, "skippable": now.skippable}));
        // The cast is taken once the scripted camera has had a moment to get there.
        st.cast.reset([]);
        let due = log.now() + SETTLE_SECS;
        st.pending = now.on.then_some(("cutscene", due));
    }
}

fn spot_json(s: &Spot) -> Value {
    match s {
        Spot::Entity(id) => json!({"entity": id.0}),
        Spot::PathPoint { path, index } => json!({"path": path, "index": index}),
        Spot::Path(path) => json!({"path": path}),
        Spot::Position(p) => json!({"pos": p.to_f32().map(n3)}),
        _ => json!({"other": true}),
    }
}

/// Class (ODF name), display name and owner of a unit, as the notices resolve them: a unit that
/// just died is only in the view before, one just built only in this one.
fn describe(u: &UnitView, db: Option<&ContentDb>) -> (Option<String>, Option<String>) {
    let class = db.and_then(|d| d.name(u.def)).map(str::to_owned);
    let name = db.and_then(|d| d.unit(u.def)).and_then(|d| crate::ui::hud::model::craft_names::unit_name(d)).map(str::to_owned);
    (class, name)
}

fn log_events(
    mut log: ResMut<FootageLog>,
    mut st: ResMut<FootageState>,
    mut events: MessageReader<SimMsg>,
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    clock: Option<Res<AnimClock>>,
) {
    let Some(link) = link.filter(|_| log.in_match) else {
        events.clear();
        return;
    };
    let db = content.as_ref().map(|c| &*c.0);
    let sim_now = clock.map_or(0.0, |c| c.now());
    let lookup = |id: EntityId| link.prev().unit(id).or_else(|| link.curr().unit(id));
    let who = |id: EntityId| -> (Option<String>, Option<u8>) {
        let u = lookup(id);
        (u.and_then(|u| describe(u, db).0), u.map(|u| u.owner.0))
    };
    for SimMsg(ev) in events.read() {
        // Shots fired and taken only feed the fight watch; they are not records.
        for id in fighters(ev) {
            st.fight.note(id, sim_now);
        }
        match ev {
            SimEvent::Destroyed { id, by } => {
                let (class, owner) = who(*id);
                log.emit("destroyed", json!({"id": id.0, "by": by.map(|b| b.0), "class": class, "owner": owner}));
            }
            SimEvent::ShieldsDown { id, facing } => {
                let (class, owner) = who(*id);
                log.emit("shields_down", json!({"id": id.0, "facing": facing, "class": class, "owner": owner}));
            }
            SimEvent::Built { builder, id } => {
                let (class, owner) = who(*id);
                log.emit("built", json!({"id": id.0, "builder": builder.0, "class": class, "owner": owner}));
            }
            SimEvent::CloakChanged { id, on } => {
                let (class, owner) = who(*id);
                log.emit("cloak", json!({"id": id.0, "on": on, "class": class, "owner": owner}));
            }
            SimEvent::PlayerEliminated { player } => log.emit("eliminated", json!({"player": player.0})),
            SimEvent::Present(Presentation::Shot { from, at, seconds }) => {
                st.shots += 1;
                let (shot, due) = (st.shots, log.now() + SETTLE_SECS);
                log.emit("shot", json!({"shot": shot, "from": spot_json(from), "at": spot_json(at), "seconds": n3(seconds.to_f32())}));
                // The picture after the cut: a full cast, once the camera is there.
                st.cast.reset([]);
                st.pending = Some(("shot", due));
            }
            SimEvent::Present(Presentation::Title { text, subtitle, .. }) => log.emit("title", json!({"text": text, "subtitle": subtitle})),
            _ => {}
        }
    }
}

fn log_pause(mut log: ResMut<FootageLog>, mut st: ResMut<FootageState>, clock: Option<Res<AnimClock>>) {
    let now = clock.is_some_and(|c| c.paused()) && log.in_match;
    if now != st.paused {
        st.paused = now;
        log.emit("pause", json!({"on": now}));
    }
}

fn log_verdict(mut log: ResMut<FootageLog>, mut st: ResMut<FootageState>, link: Option<Res<SimLink>>, local: Option<Res<LocalPlayer>>) {
    let Some(link) = link.filter(|_| log.in_match) else { return };
    let me = local.and_then(|l| l.0).unwrap_or(link.player());
    let v = crate::game::verdict(link.curr(), me).map(|v| match v {
        crate::game::Verdict::Victory => "victory",
        crate::game::Verdict::Defeat => "defeat",
    });
    if v != st.verdict {
        st.verdict = v;
        if let Some(result) = v {
            log.emit("verdict", json!({"result": result}));
        }
    }
}

/// One unit as `cast` lists it.
#[derive(Debug, Clone, PartialEq)]
struct CastUnit {
    id: u32,
    class: Option<String>,
    name: Option<String>,
    owner: u8,
    race: Option<String>,
    at: Placement,
}

fn unit_json(u: &CastUnit) -> Value {
    json!({
        "id": u.id, "class": u.class, "name": u.name, "owner": u.owner, "race": u.race,
        "x": n3(u.at.x), "y": n3(u.at.y), "size": n3(u.at.size),
    })
}

/// The `cast` fields: the [`CAST_MAX`] largest in frame, biggest first, and the full count.
fn cast_fields(why: &str, shot: Option<u32>, mut units: Vec<CastUnit>) -> Value {
    let total = units.len();
    units.sort_by(|a, b| b.at.size.total_cmp(&a.at.size).then(a.id.cmp(&b.id)));
    units.truncate(CAST_MAX);
    json!({"why": why, "shot": shot, "total": total, "units": units.iter().map(unit_json).collect::<Vec<_>>()})
}

/// The main camera's projection numbers.
struct Eye<'a> {
    cam: &'a Camera,
    tf: &'a GlobalTransform,
    w: f32,
    h: f32,
    /// Pixels per world unit at distance 1.
    focal: f32,
}

impl Eye<'_> {
    /// Where a sphere of world `radius` at `pos` shows, or `None` when it is not in frame.
    fn project(&self, pos: Vec3, radius: f32) -> Option<Projected> {
        let depth = (pos - self.tf.translation()).dot(self.tf.forward().as_vec3());
        let in_front = depth > 0.0;
        let p = self.cam.world_to_viewport(self.tf, pos).ok()?;
        let proj = Projected { x: p.x, y: p.y, radius: geom::projected_radius(radius, depth, self.focal), in_front };
        geom::in_frame(proj, self.w, self.h).then_some(proj)
    }
}

type Objects<'w, 's> = Query<'w, 's, (&'static GlobalTransform, Option<&'static SelectRadius>)>;

/// The fight, the fight in frame, the cast and the enter/leave edges, a few times a second of real time.
#[allow(clippy::too_many_arguments)]
fn watch_frame(
    mut log: ResMut<FootageLog>,
    mut st: ResMut<FootageState>,
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    local: Option<Res<LocalPlayer>>,
    cine: Option<Res<CineractiveState>>,
    clock: Option<Res<AnimClock>>,
    entities: Res<SimEntities>,
    objects: Objects,
    camera: Query<(&Camera, &GlobalTransform, &Projection), MainCamera>,
) {
    let Some(link) = link.filter(|_| log.in_match) else { return };
    let now = log.now();
    if now < st.next_eval {
        return;
    }
    st.next_eval = now + EVAL_SECS;
    let db = content.as_ref().map(|c| &*c.0);
    let cutscene = cine.as_deref().is_some_and(|c| c.on);
    let view = link.curr();
    // What the client draws: the same set the renderer keeps entities for.
    let drawn: BTreeMap<EntityId, &UnitView> = drawn_units(view, local.and_then(|l| l.0), cinematic_reveal(cine.as_deref())).into_iter().map(|u| (u.id, u)).collect();

    let sim_now = clock.map_or(0.0, |c| c.now());
    let fighting = st.fight.fighting(sim_now, |id| drawn.contains_key(id));
    if let Some(on) = st.fight.edge(fighting.len()) {
        log.emit("fight", json!({"on": on, "units": fighting.len()}));
    }

    let Ok((cam, tf, proj)) = camera.single() else { return };
    let (Some(size), Projection::Perspective(p)) = (cam.logical_viewport_size(), proj) else { return };
    let eye = Eye { cam, tf, w: size.x, h: size.y, focal: size.y * 0.5 / (p.fov * 0.5).tan() };
    let races = std::mem::take(&mut st.races);
    let in_frame = |id: EntityId| -> Option<CastUnit> {
        let u = drawn.get(&id)?;
        let (gt, radius) = objects.get(*entities.0.get(&id)?).ok()?;
        let scale = gt.to_scale_rotation_translation().0.abs().max_element();
        let at = eye.project(gt.translation(), radius.map_or(1.0, |r| r.0) * scale)?;
        let (class, name) = describe(u, db);
        Some(CastUnit { id: id.0, class, name, owner: u.owner.0, race: races.get(&u.owner).cloned(), at: geom::placement(at, eye.w, eye.h) })
    };
    let everyone = || -> Vec<CastUnit> { drawn.keys().filter_map(|&id| in_frame(id)).collect() };

    if let Some((why, due)) = st.pending {
        if now >= due {
            st.pending = None;
            let cast = everyone();
            st.cast.reset(cast.iter().map(|u| EntityId(u.id)));
            remember(&mut st.classes, &cast);
            let shot = (why == "shot").then_some(st.shots);
            log.emit("cast", cast_fields(why, shot, cast));
        }
    } else if cutscene {
        let cast = everyone();
        let sample: BTreeSet<EntityId> = cast.iter().map(|u| EntityId(u.id)).collect();
        let edges = st.cast.update(now, &sample);
        for e in edges {
            match e {
                Edge::Enter(id) => {
                    if let Some(u) = cast.iter().find(|u| u.id == id.0) {
                        log.emit("enter", unit_json(u));
                    }
                }
                Edge::Leave(id) => {
                    let class = st.classes.get(&id).cloned();
                    log.emit("leave", json!({"id": id.0, "class": class}));
                }
            }
        }
        remember(&mut st.classes, &cast);
    } else if !fighting.is_empty() || !st.in_frame_fight.is_empty() {
        let seen: Vec<CastUnit> = fighting.iter().filter_map(|&id| in_frame(id)).collect();
        let sample: BTreeSet<()> = if seen.is_empty() { BTreeSet::new() } else { BTreeSet::from([()]) };
        for e in st.in_frame_fight.update(now, &sample) {
            match e {
                Edge::Enter(()) => {
                    let classes: Vec<&Option<String>> = seen.iter().map(|u| &u.class).collect();
                    log.emit("fight_in_frame", json!({"on": true, "units": seen.len(), "classes": classes}));
                    let cast = everyone();
                    remember(&mut st.classes, &cast);
                    log.emit("cast", cast_fields("fight", None, cast));
                }
                Edge::Leave(()) => log.emit("fight_in_frame", json!({"on": false, "units": 0})),
            }
        }
    }
    st.races = races;
}

fn remember(classes: &mut BTreeMap<EntityId, String>, cast: &[CastUnit]) {
    for u in cast {
        if let Some(c) = &u.class {
            classes.insert(EntityId(u.id), c.clone());
        }
    }
}

/// The orbit pose, about once a second, when it moved.
fn log_camera(mut log: ResMut<FootageLog>, mut st: ResMut<FootageState>, orbit: Query<&Orbit, MainCamera>) {
    if !log.in_match {
        return;
    }
    let now = log.now();
    if now < st.next_camera {
        return;
    }
    let Ok(o) = orbit.single() else { return };
    st.next_camera = now + CAMERA_SECS;
    let pose = [o.focus.x, o.focus.y, o.focus.z, o.yaw, o.pitch, o.dist];
    if st.camera.is_none_or(|last| pose_moved(&last, &pose)) {
        st.camera = Some(pose);
        log.emit("camera", json!({"focus": [n3(pose[0]), n3(pose[1]), n3(pose[2])], "yaw": n3(pose[3]), "pitch": n3(pose[4]), "dist": n3(pose[5])}));
    }
}

/// Whether the orbit pose differs from the last logged one by more than a small epsilon:
/// a world unit of focus, a hundredth of a radian, one percent of the distance.
fn pose_moved(a: &[f32; 6], b: &[f32; 6]) -> bool {
    let focus = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    focus > 1.0 || (a[3] - b[3]).abs() > 0.01 || (a[4] - b[4]).abs() > 0.01 || (a[5] - b[5]).abs() > a[5].abs() * 0.01 + 0.5
}

fn log_exit(mut log: ResMut<FootageLog>, mut exit: MessageReader<AppExit>) {
    for e in exit.read() {
        let code = match e {
            AppExit::Success => 0,
            AppExit::Error(c) => i32::from(c.get()),
        };
        log.emit("stop", json!({"code": code}));
    }
}

#[cfg(test)]
mod tests;
