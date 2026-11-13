//! The front end around the match (INT-M4): launch with no mode flag, set up a
//! game, load, play, and come back.
//!
//! ```text
//!   Boot --(startup)--> FrontEnd --(StartGame)--> Loading --(match built)--> InGame
//!                          ^                         |                         |
//!                          +---(failed build)--------+-----(LeaveMatch)---------+
//! ```
//!
//! - **FrontEnd.** The screens ([`crate::frontend::screens`]) show over the menu
//!   scene; menu music plays (`audio::music` follows [`AppState`]).
//! - **StartGame.** The setup the screens produced is turned into a
//!   [`SkirmishCfg`] (what `--skirmish` reads), the loading screen comes up, and
//!   a few frames later the match is built by whatever builds matches (the map
//!   view's `rebuild` in the game, the same path `--skirmish` uses).
//! - **Loading.** Waits for the build to put a [`SimLink`] up and for the loading
//!   screen to finish, then `InGame`. A build that fails returns to the front end
//!   with a notice.
//! - **InGame.** F10 (or Enter/Esc once the match is decided) writes
//!   [`LeaveMatch`]; the state returns to `FrontEnd`, [`clear_match`] takes the
//!   match's resources down (the map's own entities go in `map::clear_match`)
//!   and the scene and the menu music come back with the state.
//!
//! This module holds the flow; the screens, the scene, the loading screen and the
//! map view are plugins it talks to through resources and messages, so the flow
//! runs in a bare `App` (`flow_tests`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use bevy::prelude::*;
use sta_content::MapDef;
use sta_sim::setup::{Control, GameSetup};

use super::{LocalRace, SkirmishCfg, verdict};
use crate::app::{AppState, GameSet, Opponent};
use crate::frontend::loading::{LoadInfo, LoadProgress, LoadReport, LoadStage};
use crate::frontend::scene::{MenuMap, MenuSceneConfig};
use crate::frontend::screens::{MapEntry, ModChosen, ScreenData, SetupDraft, StartGame};
use crate::map::{Catalogue, MapBrowser, MapSeat};
use crate::sim_link::{CommandQueue, Selection, SimLink};
use crate::ui::notices::{Notice, Severity};
use crate::viewer::Mounted;

/// Present when the app runs the front end flow (the map view then waits for the
/// loading screen instead of building at start).
#[derive(Resource)]
pub struct Flow;

/// The match being started or played, from the setup screen.
#[derive(Resource, Debug, Clone)]
pub struct ActiveMatch {
    pub setup: GameSetup,
    /// The build was asked for (the builder's cue); false while the loading screen comes up.
    pub requested: bool,
    /// Frames in the current phase.
    pub frames: u32,
}

/// Leave the match for the front end.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaveMatch;

/// The scene the front end opens each time it is entered.
#[derive(Resource, Clone)]
pub struct MenuSceneSource(pub MenuSceneConfig);

/// The menu playlist, for `audio::MusicConfig::menu`.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuMusic(pub Vec<String>);

/// Where the mount came from, for remounting with another mod.
#[derive(Resource, Debug, Clone)]
pub struct FlowConfig {
    pub install_root: Option<PathBuf>,
}

/// Frames the loading screen is up before the build starts. The build blocks a
/// frame, and the menu scene (closed by the second) must be gone from the link.
pub const LEAD_FRAMES: u32 = 3;
/// Frames a build may take with no builder to answer before it is given up.
const GIVE_UP_FRAMES: u32 = 3600;

/// Whether the map view's systems run: always outside the front end flow, and
/// from loading to the match's end inside it.
pub fn match_running(state: Option<Res<State<AppState>>>, flow: Option<Res<Flow>>) -> bool {
    match (flow, state) {
        (Some(_), Some(s)) => matches!(s.get(), AppState::Loading | AppState::InGame),
        _ => true,
    }
}

/// What a setup asks of a skirmish: the first human seat plays; the first other
/// seat is the opponent (a match has two for now). Seat ids are the start slots.
pub fn skirmish_of(setup: &GameSetup) -> Result<SkirmishCfg, String> {
    let me = setup.players.iter().find(|p| p.control == Control::Human).ok_or("the setup has no human seat")?;
    let them = setup.players.iter().find(|p| p.id != me.id).ok_or("the setup has no opponent seat")?;
    if setup.players.len() > 2 {
        warn!("setup has {} seats; a match has two for now (seat {} against seat {})", setup.players.len(), me.id.0, them.id.0);
    }
    Ok(SkirmishCfg {
        seat: me.id.0,
        opponent: match them.control {
            Control::Ai(_) => Opponent::Skirmish,
            _ => Opponent::Idle,
        },
        race: Some(me.race.clone()),
        opponent_race: Some(them.race.clone()),
    })
}

pub struct FlowPlugin;

impl Plugin for FlowPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<State<AppState>>() {
            app.init_state::<AppState>();
        }
        app.add_plugins(super::mission_host::MissionHostPlugin).insert_resource(Flow)
            .add_message::<Notice>()
            .add_message::<StartGame>()
            .add_message::<ModChosen>()
            .add_message::<LeaveMatch>()
            .add_systems(Startup, |mut next: ResMut<NextState<AppState>>| next.set(AppState::FrontEnd))
            .add_systems(OnEnter(AppState::FrontEnd), (clear_match, open_scene).chain())
            .add_systems(OnEnter(AppState::Loading), close_scene)
            // A remount keeps no lookup of the previous mount: before anything reads the new one.
            .add_systems(Update, invalidate_mount_caches.before(GameSet::Input).run_if(resource_exists_and_changed::<Mounted>).run_if(not(resource_added::<Mounted>)))
            .add_systems(
                Update,
                (
                    on_start.run_if(in_state(AppState::FrontEnd)),
                    remount.run_if(in_state(AppState::FrontEnd)).run_if(resource_exists::<FlowConfig>),
                    drive_load.run_if(in_state(AppState::Loading)),
                    (leave_keys, on_leave).chain().run_if(in_state(AppState::InGame)),
                    show_match_ui,
                    apply_menu_music,
                )
                    .in_set(GameSet::Ui),
            );
    }
}

/// Everything cached by name from the mounted files (decoded textures with their misses, parts,
/// unit models, moon variants, silhouettes, surface recipes) and the sprite tables are rebuilt
/// from the new mount; systems that already follow `Mounted`'s change (fonts, fog art, i18n,
/// HUD sprites, cull and weapon tables, radar colours) need nothing here.
#[allow(clippy::too_many_arguments)]
fn invalidate_mount_caches(
    vfs: Res<Mounted>,
    textures: Option<ResMut<crate::render::textures::TextureCache>>,
    parts: Option<ResMut<crate::render::model_scene::PartCache>>,
    models: Option<ResMut<crate::render::units::UnitModels>>,
    moons: Option<ResMut<crate::render::moons::MoonVariants>>,
    silhouettes: Option<ResMut<crate::render::strategic::Silhouettes>>,
    surfaces: Option<ResMut<crate::render::surfaces::Surfaces>>,
    tables: Option<ResMut<crate::render::sprites::SpriteTables>>,
) {
    fn reset<T: Resource<Mutability = bevy::ecs::component::Mutable> + Default>(r: Option<ResMut<T>>) {
        if let Some(mut r) = r {
            *r = T::default();
        }
    }
    reset(textures);
    reset(parts);
    reset(models);
    reset(moons);
    reset(silhouettes);
    reset(surfaces);
    if let Some(mut t) = tables {
        t.0 = Arc::new(sta_assets::load_sprite_table(&vfs.0));
    }
}

fn open_scene(mut commands: Commands, source: Option<Res<MenuSceneSource>>) {
    if let Some(s) = source {
        commands.insert_resource(s.0.clone());
    }
}

fn close_scene(mut commands: Commands) {
    commands.remove_resource::<MenuSceneConfig>();
}

/// StartGame: remember the setup, tell the match what it is, show the loading screen.
fn on_start(
    mut starts: MessageReader<StartGame>,
    mut commands: Commands,
    mut next: ResMut<NextState<AppState>>,
    mut notices: MessageWriter<Notice>,
    info: Option<ResMut<LoadInfo>>,
    progress: Option<ResMut<LoadProgress>>,
) {
    let Some(StartGame(setup)) = starts.read().last().cloned() else { return };
    let cfg = match skirmish_of(&setup) {
        Ok(c) => c,
        Err(e) => {
            warn!("start: {e}");
            notices.write(Notice::new(Severity::Bad, format!("Cannot start: {e}")));
            return;
        }
    };
    info!("start: {} on {}, seat {} against {:?}", setup.players.len(), setup.map.name, cfg.seat, cfg.opponent);
    if let Some(mut info) = info {
        *info = LoadInfo::from_map(&setup.map);
    }
    if let Some(mut p) = progress {
        p.begin();
    }
    commands.insert_resource(MapSeat(Some(cfg.seat)));
    commands.insert_resource(cfg);
    commands.insert_resource(ActiveMatch { setup, requested: false, frames: 0 });
    next.set(AppState::Loading);
}

/// Loading: the loading screen first, then the build, then in.
fn drive_load(
    active: Option<ResMut<ActiveMatch>>,
    link: Option<Res<SimLink>>,
    browser: Option<ResMut<MapBrowser>>,
    progress: Option<ResMut<LoadProgress>>,
    mut next: ResMut<NextState<AppState>>,
    mut notices: MessageWriter<Notice>,
) {
    let Some(mut m) = active else {
        next.set(AppState::FrontEnd);
        return;
    };
    m.frames += 1;
    if !m.requested {
        if m.frames >= LEAD_FRAMES {
            m.requested = true;
            m.frames = 0;
            if let Some(mut b) = browser {
                // The map view builds the map `index` names when `dirty`.
                if let Some(i) = b.maps.iter().position(|n| *n == m.setup.map.name) {
                    b.index = i;
                    b.dirty = true;
                } else {
                    let name = m.setup.map.name.clone();
                    warn!("start: {name} is not in the mount");
                    notices.write(Notice::new(Severity::Bad, format!("Cannot load {name}: not in the mount")));
                    next.set(AppState::FrontEnd);
                }
            }
        }
        return;
    }
    if link.is_some() {
        if let Some(mut p) = progress {
            if !p.is_complete() {
                for stage in LoadStage::ALL {
                    p.report(&LoadReport::finished(stage));
                }
            }
            if p.is_active() {
                return;
            }
        }
        info!("start: {} loaded", m.setup.map.name);
        next.set(AppState::InGame);
        return;
    }
    let failed = match &browser {
        Some(b) => m.frames >= 2 && !b.dirty,
        None => m.frames > GIVE_UP_FRAMES,
    };
    if failed {
        let why = browser.map(|b| b.info.replace('\n', ": ")).unwrap_or_default();
        warn!("start: {} did not load: {why}", m.setup.map.name);
        notices.write(Notice::new(Severity::Bad, format!("Cannot load {}: {why}", m.setup.map.name)));
        next.set(AppState::FrontEnd);
    }
}

/// F10 leaves the match at any time; Enter or Esc once it is decided.
fn leave_keys(keys: Option<Res<ButtonInput<KeyCode>>>, link: Option<Res<SimLink>>, mission: Option<Res<super::mission_host::MissionRun>>, mut leave: MessageWriter<LeaveMatch>) {
    let (Some(keys), Some(link)) = (keys, link) else { return };
    // A mission ends by its script, not by elimination.
    let decided = mission.is_none() && verdict(link.curr(), link.player()).is_some();
    if keys.just_pressed(KeyCode::F10) || (decided && keys.any_just_pressed([KeyCode::Enter, KeyCode::Escape])) {
        leave.write(LeaveMatch);
    }
}

fn on_leave(mut leave: MessageReader<LeaveMatch>, mut next: ResMut<NextState<AppState>>) {
    if leave.read().count() > 0 {
        next.set(AppState::FrontEnd);
    }
}

/// Back at the front end: the match's resources go. (The map's entities go in
/// `map::clear_match`; music, the scene and the screens follow the state.)
fn clear_match(
    mut commands: Commands,
    selection: Option<ResMut<Selection>>,
    queue: Option<ResMut<CommandQueue>>,
    seat: Option<ResMut<MapSeat>>,
    progress: Option<ResMut<LoadProgress>>,
    mut leave: ResMut<Messages<LeaveMatch>>,
) {
    commands.remove_resource::<ActiveMatch>();
    commands.remove_resource::<SimLink>();
    commands.remove_resource::<SkirmishCfg>();
    commands.remove_resource::<LocalRace>();
    if let Some(mut s) = selection {
        s.0.clear();
    }
    if let Some(mut q) = queue {
        q.0.clear();
    }
    if let Some(mut s) = seat {
        s.0 = None;
    }
    if let Some(mut p) = progress {
        p.dismiss();
    }
    leave.clear();
}

type MatchUi = Or<(With<crate::ui::hud::HudRoot>, With<crate::ui::radar::RadarRoot>)>;

/// The HUD and the radar belong to the match: hidden in the front end and the load.
fn show_match_ui(
    state: Option<Res<State<AppState>>>,
    mut ui: Query<&mut Visibility, MatchUi>,
) {
    let want = if state.is_some_and(|s| *s.get() == AppState::InGame) { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut ui {
        v.set_if_neq(want);
    }
}

fn apply_menu_music(menu: Option<Res<MenuMusic>>, music: Option<ResMut<crate::audio::MusicConfig>>) {
    let (Some(menu), Some(mut music)) = (menu, music) else { return };
    if (menu.is_changed() || music.is_added()) && music.menu != menu.0 {
        music.menu.clone_from(&menu.0);
    }
}

// ---------------------------------------------------------------------------
// The install's lists, for the screens and the scene
// ---------------------------------------------------------------------------

/// What the front end shows from a mount.
pub(crate) struct FrontEndData {
    pub screens: ScreenData,
    pub scene: MenuSceneConfig,
    pub menu_music: Vec<String>,
}

/// The content, the multiplayer maps (two start slots or more, each as a
/// converted `MapDef` named by its file), the mods and the menu scene of a mount.
pub(crate) fn front_end_data(vfs: &sta_vfs::Vfs, catalogue: &mut Catalogue, install_root: Option<&Path>) -> Result<FrontEndData, String> {
    let t0 = std::time::Instant::now();
    let content = catalogue.content(vfs)?;
    let (db, index, races) = (content.db.clone(), content.index.clone(), content.races.clone());
    // Star Trek: Legacy `.sol` maps (CON-20) sit beside the `.bzn` files and parse to the same `Map`.
    let mut names: Vec<String> = vfs.list("bzn").into_iter().filter(|p| p.ends_with(".bzn") || p.ends_with(".sol")).map(|p| p.trim_start_matches("bzn/").trim_end_matches(".bzn").trim_end_matches(".sol").to_owned()).collect();
    names.sort();
    names.dedup();
    let mut maps = Vec::new();
    for name in &names {
        let Some(Ok(bytes)) = vfs.read(&format!("bzn/{name}.bzn")).or_else(|| vfs.read(&format!("bzn/{name}.sol"))) else { continue };
        let Ok(map) = sta_bzn::parse(&bytes) else { continue };
        let converted = MapDef::convert_bzn_named_in(&map, name, &db, vfs, &index);
        for warning in converted.warnings.iter().filter(|w| w.file.ends_with(".mdf")) {
            debug!("{warning}");
        }
        let def = converted.map;
        let mut slots: Vec<u8> = def.starts.iter().map(|s| s.slot).collect();
        slots.sort_unstable();
        slots.dedup();
        if slots.len() >= 2 {
            maps.push(MapEntry::new(def));
        }
    }
    let data_dir = install_root.map(|r| if r.join("Data").is_dir() { r.join("Data") } else { r.to_path_buf() });
    let (mods, notes) = data_dir.as_deref().map(ScreenData::mods_in).unwrap_or_default();
    for n in notes {
        debug!("mods: {n}");
    }
    info!("front end: {} multiplayer maps of {}, {} mods, in {:.1?}", maps.len(), names.len(), mods.len(), t0.elapsed());
    let menu_music = races.playable().into_iter().find_map(|r| r.single_player_music.clone()).into_iter().collect();
    let scene = MenuSceneConfig::new(db.clone(), MenuMap::from_vfs(vfs), MapDef::empty());
    Ok(FrontEndData { screens: ScreenData { content: db, races, maps, mods, base_name: install_root.and_then(|r| r.file_name()).map(|n| n.to_string_lossy().into_owned()) }, scene, menu_music })
}

/// Use mod: remount the install with it and refill the lists. The scene shows
/// the new content from the next time the front end opens it.
#[allow(clippy::too_many_arguments)]
fn remount(
    mut chosen: MessageReader<ModChosen>,
    config: Res<FlowConfig>,
    mut commands: Commands,
    mut mounted: ResMut<Mounted>,
    mut catalogue: ResMut<Catalogue>,
    mut browser: ResMut<MapBrowser>,
    mut notices: MessageWriter<Notice>,
) {
    let Some(ModChosen(folder)) = chosen.read().last().cloned() else { return };
    let Some(root) = &config.install_root else { return };
    let vfs = match sta_vfs::Vfs::mount_install(root, folder.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            notices.write(Notice::new(Severity::Bad, format!("Cannot mount {}: {e}", folder.as_deref().unwrap_or("the base game"))));
            return;
        }
    };
    *catalogue = Catalogue::default();
    match front_end_data(&vfs, &mut catalogue, Some(root)) {
        Ok(d) => {
            info!("mounted {}", vfs.layers().iter().map(|l| l.name()).collect::<Vec<_>>().join(" -> "));
            browser.mount_summary = crate::mount_summary(root, &vfs);
            browser.maps = d.screens.maps.iter().map(|m| m.def.name.clone()).collect();
            commands.insert_resource(SetupDraft::new(&d.screens));
            commands.insert_resource(d.screens);
            commands.insert_resource(MenuSceneSource(d.scene));
            commands.insert_resource(MenuMusic(d.menu_music));
            // The campaigns follow the mount (UI-25 / INT-M6).
            apply_campaigns(&mut commands, super::mission_host::load_campaigns(&vfs, Some(root), folder.as_deref()));
        }
        Err(e) => {
            notices.write(Notice::new(Severity::Bad, format!("Cannot load content: {e}")));
            return;
        }
    }
    mounted.0 = vfs;
}

/// Make the campaign resources match the mount: the book when the install has campaigns, none
/// when it has not. The movie catalogue always exists (empty without campaigns): the media
/// tracker reads it every frame, so removing it on a remount to a campaign-less mod panicked.
pub(crate) fn apply_campaigns(commands: &mut Commands, loaded: Option<(crate::frontend::campaign::Campaigns, crate::media::campaign::MissionMovies)>) {
    match loaded {
        Some((campaigns, movies)) => {
            commands.insert_resource(campaigns);
            commands.insert_resource(movies);
        }
        None => {
            commands.remove_resource::<crate::frontend::campaign::Campaigns>();
            commands.insert_resource(crate::media::campaign::MissionMovies::default());
        }
    }
}

/// The maps of `data` as the map view's browser lists them.
pub(crate) fn browser_for(data: &ScreenData, summary: String) -> MapBrowser {
    MapBrowser { maps: data.maps.iter().map(|m| m.def.name.clone()).collect(), index: 0, dirty: false, mount_summary: summary, info: String::new() }
}
