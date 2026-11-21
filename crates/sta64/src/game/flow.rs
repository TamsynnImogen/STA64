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
//! - **StartGame.** The whole Instant Action setup is retained while the
//!   loading screen comes up. The map view builds its sim from those players
//!   and options a few frames later.
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
use sta_sim::PlayerId;
use sta_sim::setup::{Control, GameSetup};

use super::{LocalRace, SkirmishCfg, verdict};
use crate::app::{AppState, GameSet};
use crate::frontend::loading::{LoadInfo, LoadProgress, LoadReport, LoadStage};
use crate::frontend::scene::{MenuMap, MenuSceneConfig};
use crate::frontend::screens::{ActiveMod, Browse, MapEntry, ModChosen, ModLaunchOptions, Screen, ScreenData, SelectedMod, SetupDraft, StartGame};
use crate::map::{Catalogue, MapBrowser, MapSeat};
use crate::sim_link::{CommandQueue, Selection, SimLink};
use crate::ui::notices::{Notice, Severity};
use crate::viewer::{Mounted, TeamColours};

/// Present when the app runs the front end flow (the map view then waits for the
/// loading screen instead of building at start).
#[derive(Resource)]
pub struct Flow;

/// The match being started or played, from the setup screen.
#[derive(Resource, Debug, Clone)]
pub struct ActiveMatch {
    pub setup: GameSetup,
    /// No local seat in an Observer match.
    pub local: Option<PlayerId>,
    pub speed: crate::clock::MatchSpeed,
    pub fog: crate::frontend::screens::SetupFog,
    /// The build was asked for (the builder's cue); false while the loading screen comes up.
    pub requested: bool,
    /// Frames in the current phase.
    pub frames: u32,
}

/// The mount's palette to restore after an Instant Action colour choice.
#[derive(Resource)]
struct MatchPaletteBackup(Option<TeamColours>);

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

/// Validate the offline roster and return the local participant, if playing.
fn local_seat(setup: &GameSetup) -> Result<Option<PlayerId>, &'static str> {
    if setup.players.len() < 2 {
        return Err("Instant Action needs at least two map seats");
    }
    let mut ids = std::collections::BTreeSet::new();
    if setup.players.iter().any(|p| !p.id.is_player() || !ids.insert(p.id) || setup.map.start_for(p.id.0).is_none()) {
        return Err("each participant needs a different playable map start");
    }
    if setup.players.iter().any(|p| !matches!(p.control, Control::Human | Control::Ai(_) | Control::Idle)) {
        return Err("Instant Action only supports local participants");
    }
    let mut humans = setup.players.iter().filter(|p| p.control == Control::Human);
    let local = humans.next().map(|p| p.id);
    if humans.next().is_some() {
        return Err("Instant Action only supports one local player");
    }
    Ok(local)
}

pub struct FlowPlugin;

impl Plugin for FlowPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<State<AppState>>() {
            app.init_state::<AppState>();
        }
        app.add_plugins((super::mission_host::MissionHostPlugin, super::net_match::NetMatchPlugin, super::net_overlay::NetOverlayPlugin)).insert_resource(Flow)
            .init_resource::<ActiveMod>()
            .add_message::<Notice>()
            .add_message::<StartGame>()
            .add_message::<ModChosen>()
            .add_message::<LeaveMatch>()
            .add_systems(Startup, |mut next: ResMut<NextState<AppState>>| next.set(AppState::FrontEnd))
            .add_systems(OnEnter(AppState::FrontEnd), (clear_match, reset_input, open_scene).chain())
            .add_systems(OnEnter(AppState::Loading), close_scene)
            // A remount keeps no lookup of the previous mount: before anything reads the new one.
            .add_systems(Update, invalidate_mount_caches.in_set(GameSet::Ui).after(remount).run_if(resource_exists_and_changed::<Mounted>).run_if(not(resource_added::<Mounted>)))
            .add_systems(
                Update,
                (
                    on_start.run_if(in_state(AppState::FrontEnd)),
                    remount.after(crate::frontend::screens::on_buttons).run_if(in_state(AppState::FrontEnd)).run_if(resource_exists::<FlowConfig>),
                    drive_load.run_if(in_state(AppState::Loading)),
                    (leave_keys, on_leave).chain().run_if(in_state(AppState::InGame)),
                    show_match_ui.after(crate::ui::presentation::consume),
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
pub(crate) fn invalidate_mount_caches(
    mut commands: Commands,
    vfs: Res<Mounted>,
    textures: Option<ResMut<crate::render::textures::TextureCache>>,
    parts: Option<ResMut<crate::render::model_scene::PartCache>>,
    models: Option<ResMut<crate::render::units::UnitModels>>,
    moons: Option<ResMut<crate::render::moons::MoonVariants>>,
    silhouettes: Option<ResMut<crate::render::strategic::Silhouettes>>,
    surfaces: Option<ResMut<crate::render::surfaces::Surfaces>>,
    tables: Option<ResMut<crate::render::sprites::SpriteTables>>,
    bridge_art: Option<ResMut<crate::ui::bridge_scene::BridgeArt>>,
    objectives_art: Option<ResMut<crate::ui::objectives_panel::ObjectivesArt>>,
) {
    fn reset<T: Resource<Mutability = bevy::ecs::component::Mutable> + Default>(r: Option<ResMut<T>>) {
        if let Some(mut r) = r {
            *r = T::default();
        }
    }
    // A table from the old mount's map names entries of another palette.
    commands.remove_resource::<crate::viewer::MapTeamColours>();
    reset(textures);
    reset(parts);
    reset(models);
    reset(moons);
    reset(silhouettes);
    reset(surfaces);
    reset(bridge_art);
    reset(objectives_art);
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
    net: Option<Res<super::net_match::NetStart>>,
    mounted_palette: Option<Res<TeamColours>>,
    info: Option<ResMut<LoadInfo>>,
    progress: Option<ResMut<LoadProgress>>,
) {
    let Some(StartGame(setup, speed, palette, fog)) = starts.read().last().cloned() else { return };
    let local = match net.as_ref().and_then(|start| start.0.as_ref()) {
        Some(match_) => Ok(Some(match_.me)),
        None => local_seat(&setup),
    };
    let local = match local {
        Ok(player) => player,
        Err(e) => {
            warn!("start: {e}");
            notices.write(Notice::new(Severity::Bad, format!("Cannot start: {e}")));
            return;
        }
    };
    info!("start: {} map seats on {}, local seat {:?}", setup.players.len(), setup.map.name, local.map(|p| p.0));
    if let Some(mut info) = info {
        *info = LoadInfo::from_map(&setup.map);
    }
    if let Some(mut p) = progress {
        p.begin();
    }
    if let Some(palette) = palette {
        commands.insert_resource(MatchPaletteBackup(mounted_palette.as_deref().cloned()));
        commands.insert_resource(palette);
    }
    commands.insert_resource(MapSeat(local.map(|p| p.0)));
    commands.insert_resource(ActiveMatch { setup, local, speed, fog, requested: false, frames: 0 });
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
    entities: Option<ResMut<crate::sim_link::SimEntities>>,
    palette: Option<Res<MatchPaletteBackup>>,
) {
    // The match's ships are the link's: they go with it, so the menu scene (whose sim reuses the
    // ids 1.. ) does not inherit their models, symbols and hidden visibility.
    if let Some(mut e) = entities {
        for (_, ent) in std::mem::take(&mut e.0) {
            commands.entity(ent).try_despawn();
        }
    }
    commands.remove_resource::<ActiveMatch>();
    if let Some(palette) = palette {
        if let Some(original) = &palette.0 {
            commands.insert_resource(original.clone());
        } else {
            commands.remove_resource::<TeamColours>();
        }
        commands.remove_resource::<MatchPaletteBackup>();
    }
    // The map's own team colours end with its match: the menu scene's ships take the palette's.
    commands.remove_resource::<crate::viewer::MapTeamColours>();
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

/// Back at the front end (UI-34): what the player's input and the HUD held for the match goes:
/// control groups, move targets, the drag, the cursor aim, a placement or target waiting for a
/// click, and the animation clock's pause.
fn reset_input(
    groups: Option<ResMut<crate::input::select::ControlGroups>>,
    targets: Option<ResMut<crate::input::orders::OrderTargets>>,
    drag: Option<ResMut<crate::input::select::Drag>>,
    aim: Option<ResMut<crate::input::orders::Aim>>,
    hud: Option<ResMut<crate::ui::hud::HudState>>,
    clock: Option<ResMut<crate::clock::AnimClock>>,
) {
    if let Some(mut g) = groups {
        *g = default();
    }
    if let Some(mut t) = targets {
        t.0.clear();
    }
    if let Some(mut d) = drag {
        *d = default();
    }
    if let Some(mut a) = aim {
        *a = default();
    }
    if let Some(mut h) = hud {
        h.placing = None;
        h.targeting = None;
    }
    if let Some(mut c) = clock {
        c.set_paused(false);
    }
}

type MatchUi = Or<(With<crate::ui::hud::HudRoot>, With<crate::ui::radar::RadarRoot>)>;

/// The HUD and the radar belong to the match: hidden in the front end and the load.
fn show_match_ui(
    state: Option<Res<State<AppState>>>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    mut ui: Query<&mut Visibility, MatchUi>,
) {
    // Not over a script's cineractive either: the cinematic view has no HUD panel (Refit, static).
    let want = if state.is_some_and(|s| *s.get() == AppState::InGame) && !cine.is_some_and(|c| c.on) { Visibility::Inherited } else { Visibility::Hidden };
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
        let map = MapEntry::new(converted.map);
        if map.slots() >= 2 {
            maps.push(map);
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
    let colours = TeamColours::load(vfs);
    Ok(FrontEndData { screens: ScreenData { content: db, races, colours, maps, mods, base_name: install_root.and_then(|r| r.file_name()).map(|n| n.to_string_lossy().into_owned()) }, scene, menu_music })
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
    browse: Option<Res<Browse>>,
    mut next: Option<ResMut<NextState<Screen>>>,
    (intro, ids, movies): (
        Option<Res<crate::media::IntroSettings>>,
        Option<ResMut<crate::media::MovieIds>>,
        Option<ResMut<Messages<crate::media::PlayMovie>>>,
    ),
) {
    let Some(ModChosen(folder)) = chosen.read().last().cloned() else { return };
    let Some(root) = &config.install_root else {
        notices.write(Notice::new(Severity::Bad, "No game install is available"));
        return;
    };
    let vfs = match sta_vfs::Vfs::mount_install(root, folder.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            notices.write(Notice::new(Severity::Bad, format!("Cannot mount {}: {e}", folder.as_deref().unwrap_or("the base game"))));
            return;
        }
    };
    let mut next_catalogue = Catalogue::default();
    match front_end_data(&vfs, &mut next_catalogue, Some(root)) {
        Ok(d) => {
            info!("mounted {}", vfs.layers().iter().map(|l| l.name()).collect::<Vec<_>>().join(" -> "));
            browser.mount_summary = crate::mount_summary(root, &vfs);
            browser.maps = d.screens.maps.iter().map(|m| m.def.name.clone()).collect();
            commands.insert_resource(SetupDraft::new(&d.screens));
            commands.insert_resource(d.screens.colours.clone());
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
    *catalogue = next_catalogue;
    if let Some(browse) = browse {
        let options = ModLaunchOptions { no_intro: browse.mod_no_intro, no_ai: browse.mod_no_ai };
        commands.insert_resource(options);
        if !options.no_intro && intro.is_some_and(|i| i.enabled) && let (Some(mut ids), Some(mut movies)) = (ids, movies) {
            crate::media::intro::queue_sequence(&vfs, &mut ids, |movie| { movies.write(movie); });
        }
    }
    mounted.0 = vfs;
    commands.insert_resource(SelectedMod(folder.clone()));
    commands.insert_resource(ActiveMod(folder));
    if let Some(next) = &mut next {
        next.set(Screen::Main);
    }
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
