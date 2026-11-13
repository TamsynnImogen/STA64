//! Playtest capture (TOOL-14): Ctrl+F12 in a match writes a folder that lets a
//! fix agent reproduce the moment: the replay so far, a sim snapshot, what was
//! selected, where the cursor and camera were, the mod mount and the build.
//! The folder layout and the code that reads it back are `sta_bench::capture`;
//! `sta-bench capture check <folder>` replays it headless and compares digests.
//!
//! - The match's [`SimLink`](crate::sim_link::SimLink) is armed with a replay
//!   log when the map view builds it ([`arm`]); the log only copies what each
//!   tick was given, so nothing here can change a digest.
//! - The key is [`Action::Capture`] (F12) with the group modifier (Ctrl) held,
//!   so it goes through the key bindings like every other action and collides
//!   with nothing: F12 alone is left to the platform's own tools.
//! - Captures go to `$STA64_CAPTURE_DIR`, else the per-user data directory
//!   (`<data>/sta64/captures/<UTC stamp>-t<tick>/`), never into an install.
//! - `STA64_CAPTURE_NOTE` in the environment fills `note.txt`; otherwise the
//!   file is empty for the player to type into.
//! - No frame is saved: the game has no in-app frame dump in a match (the
//!   headless bench's frame path draws stand-ins from the bench view), and a
//!   screen grab is not an option.

use std::path::PathBuf;

use bevy::prelude::*;
use sta_bench::capture::{self, BuildInfo, Capture, CameraPose, CaptureIndex, MountInfo, StartMode, UnitInfo};
use sta_content::ContentDb;
use sta_sim::replay::Replay;
use sta_sim::setup::GameSetup;
use sta_sim::view::UnitFlags;
use sta_sim::EntityId;

use crate::app::GameSet;
use crate::input::hover::HoveredUnit;
use crate::input::keys::{Action, KeyBindings};
use crate::input::orders::Aim;
use crate::map::MainCamera;
use crate::sim_link::{ContentRes, Selection, SimLink};
use crate::ui::notices::{Notice, Severity};
use crate::viewer::Orbit;

/// What a capture needs to know about the match that the link does not.
#[derive(Resource, Debug, Clone)]
pub struct MatchMeta {
    pub start: StartMode,
    pub mount: MountInfo,
}

/// Overrides where captures are written (tests); default [`capture::default_capture_dir`].
#[derive(Resource, Debug, Clone)]
pub struct CaptureDir(pub PathBuf);

/// The git commit the build was made from: whatever `STA64_GIT_COMMIT` was set to
/// when it was compiled (CI, release scripts), else unknown. Not read from git by a
/// build script: that would rebuild this crate on every commit.
const COMMIT: Option<&str> = option_env!("STA64_GIT_COMMIT");

fn build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        commit: COMMIT.filter(|c| !c.is_empty()).map(str::to_owned),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        debug: cfg!(debug_assertions),
    }
}

/// Starts logging `link`'s game and inserts it as the match's link, with the
/// facts a capture of it needs. `skirmish` is whether the sim was made with
/// each seat's starting forces (`Sim::from_setup_with_starts`).
pub fn arm(commands: &mut Commands, link: SimLink, setup: &GameSetup, skirmish: bool, vfs: &sta_vfs::Vfs) {
    let start = if skirmish { StartMode::Starts } else { StartMode::Plain };
    let mount = MountInfo::from_layers(vfs.layers().iter().map(|l| (l.name(), l.root.as_path())));
    commands.insert_resource(MatchMeta { start, mount });
    commands.insert_resource(link.with_replay(Replay::new(setup)));
}

/// Names of the set bits of a unit's flags, for people.
fn flag_names(flags: UnitFlags) -> Vec<String> {
    [
        (UnitFlags::CLOAKED, "cloaked"),
        (UnitFlags::DYING, "dying"),
        (UnitFlags::BUILDING, "building"),
        (UnitFlags::DOCKED, "docked"),
        (UnitFlags::SELECTABLE, "selectable"),
        (UnitFlags::IMMORTAL, "immortal"),
        (UnitFlags::WEAPONS_OFF, "weapons_off"),
        (UnitFlags::ENGINES_OFF, "engines_off"),
    ]
    .into_iter()
    .filter(|(f, _)| flags.contains(*f))
    .map(|(_, n)| n.to_owned())
    .collect()
}

fn unit_info(link: &SimLink, content: &ContentDb, replay: &Replay, id: EntityId) -> Option<UnitInfo> {
    let u = link.curr().unit(id)?;
    Some(UnitInfo {
        id: id.0,
        def: content.name(u.def).map(str::to_owned),
        owner: u.owner.0,
        race: replay.players.iter().find(|p| p.id == u.owner).map(|p| p.race.clone()),
        pos: u.pos.to_f32(),
        heading: [u.heading.0.to_f32(), u.heading.1.to_f32()],
        hull: [u.hull[0].to_f32(), u.hull[1].to_f32()],
        flags: u.flags.0,
        flag_names: flag_names(u.flags),
        progress: u.progress.map(|p| p.to_f32()),
    })
}

/// The player's view at the moment of the capture.
#[derive(Debug, Clone, Default)]
pub struct Looking {
    pub selection: Vec<EntityId>,
    pub hovered: Option<EntityId>,
    pub cursor_world: Option<[f32; 3]>,
    pub camera: Option<CameraPose>,
    /// What the click picking returns for the cursor ray.
    pub pick: Option<EntityId>,
    pub pointer_over_hud: Option<bool>,
    pub click_pending: Option<bool>,
    pub window: Option<[f32; 2]>,
}

/// The capture of the match as it stands: reads the link, never steps it.
/// `None` when the link keeps no replay (not a match) or has no snapshot.
pub fn take(link: &mut SimLink, meta: &MatchMeta, content: &ContentDb, looking: &Looking) -> Option<Capture> {
    let replay = link.replay()?.clone();
    let view = link.curr().clone();
    let mut index = CaptureIndex::new(&replay, content, view.tick, view.digest, meta.start, capture::now_unix(), build_info());
    index.mount = meta.mount.clone();
    index.units_total = view.units.len();
    index.selection = looking.selection.iter().filter_map(|&id| unit_info(link, content, &replay, id)).collect();
    index.hovered = looking.hovered.and_then(|id| unit_info(link, content, &replay, id));
    index.cursor_world = looking.cursor_world;
    index.pick = looking.pick.and_then(|id| unit_info(link, content, &replay, id));
    index.pointer_over_hud = looking.pointer_over_hud;
    index.click_pending = looking.click_pending;
    index.window = looking.window;
    index.camera = looking.camera.clone();
    let snapshot = link.snapshot();
    index.has_snapshot = snapshot.is_some();
    let note = std::env::var("STA64_CAPTURE_NOTE").unwrap_or_default();
    Some(Capture { index, replay, snapshot, note })
}

/// Where the ray meets the ground plane (Bevy `y = 0`), in the sim's map space
/// (Z mirrored).
fn ground_point(ray: Ray3d) -> Option<[f32; 3]> {
    let d = ray.direction.y;
    let t = -ray.origin.y / d;
    (d.abs() > 1e-6 && t > 0.0).then(|| {
        let p = ray.origin + *ray.direction * t;
        [p.x, 0.0, -p.z]
    })
}

#[allow(clippy::too_many_arguments)]
fn capture_hotkey(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Option<Res<KeyBindings>>,
    dir: Option<Res<CaptureDir>>,
    mut link: ResMut<SimLink>,
    meta: Res<MatchMeta>,
    content: Res<ContentRes>,
    selection: Res<Selection>,
    hovered: Option<Res<HoveredUnit>>,
    aim: Option<Res<Aim>>,
    cameras: Query<(&GlobalTransform, Option<&Orbit>), MainCamera>,
    units: crate::input::select::Targetable,
    pointer: Option<Res<crate::ui::hud::HudPointer>>,
    hud: Option<Res<crate::ui::hud::HudState>>,
    window: Option<Single<&Window, With<bevy::window::PrimaryWindow>>>,
    mut notices: MessageWriter<Notice>,
) {
    let defaults;
    let bindings = match &bindings {
        Some(b) => b,
        None => {
            defaults = KeyBindings::default();
            &defaults
        }
    };
    if !bindings.just_pressed(Action::Capture, &keys) || !bindings.pressed(Action::GroupModifier, &keys) {
        return;
    }
    let looking = Looking {
        selection: selection.0.clone(),
        hovered: hovered.and_then(|h| h.0),
        cursor_world: aim.as_deref().and_then(|a| a.0).and_then(|a| ground_point(a.ray)),
        pick: aim.as_deref().and_then(|a| a.0).and_then(|a| crate::input::select::unit_under(a.cursor, a.ray, &units)),
        pointer_over_hud: pointer.map(|p| p.0),
        click_pending: hud.map(|h| h.placing.is_some() || h.targeting.is_some()),
        window: window.map(|w| [w.width(), w.height()]),
        camera: cameras.single().ok().map(|(t, orbit)| {
            let (translation, rotation) = (t.translation(), t.to_scale_rotation_translation().1);
            CameraPose {
                translation: translation.to_array(),
                rotation: rotation.to_array(),
                focus: orbit.map(|o| o.focus.to_array()),
                yaw: orbit.map(|o| o.yaw),
                pitch: orbit.map(|o| o.pitch),
                distance: orbit.map(|o| o.dist),
            }
        }),
    };
    let Some(cap) = take(&mut link, &meta, &content.0, &looking) else {
        // A link with no log is the menu's scene or a bench link: nothing to capture.
        return;
    };
    let root = dir.map_or_else(capture::default_capture_dir, |d| d.0.clone());
    match cap.write(&root) {
        Ok(folder) => {
            info!("capture: {} (tick {}, digest {})", folder.display(), cap.index.tick, cap.index.digest);
            notices.write(Notice::new(Severity::Good, format!("Capture saved: {}", folder.display())));
        }
        Err(e) => {
            warn!("capture: {e}");
            notices.write(Notice::new(Severity::Bad, format!("Capture failed: {e}")));
        }
    }
}

/// Registers the capture hotkey.
pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Notice>().add_systems(
            Update,
            capture_hotkey
                .in_set(GameSet::Ui)
                .run_if(resource_exists::<SimLink>)
                .run_if(resource_exists::<MatchMeta>)
                .run_if(resource_exists::<ContentRes>)
                .run_if(not(resource_exists::<crate::frontend::scene::MenuScene>)),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::time::TimeUpdateStrategy;
    use sta_content::Races;
    use sta_core::{PlayerId, TeamId};
    use sta_sim::command::{CommandKind, LocalRouter, Order};
    use sta_sim::setup::{Control, GameOptions, PlayerSetup};
    use sta_sim::{Sim, Vec3 as SimVec3, WorldSource};

    use super::*;
    use crate::sim_link::{CommandQueue, SimLinkPlugin};

    fn setup() -> GameSetup {
        let seat = |i: u8, control| PlayerSetup { id: PlayerId(i), team: TeamId(i), race: "fx".into(), control };
        GameSetup {
            seed: 5,
            map: Arc::new(sta_content::fake::two_bases()),
            content: Arc::new(sta_content::fake::db()),
            players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
            options: GameOptions::default(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sta64-capture-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A headless skirmish: the sim link steps from the clock, an order is queued,
    /// Ctrl+F12 is pressed, and the folder it writes replays to the same digest.
    #[test]
    fn the_hotkey_writes_a_capture_that_replays_to_the_same_digest() {
        let s = setup();
        let races = Races::default();
        let (sim, _) = Sim::from_setup_with_starts(&s, &races);
        let mine: Vec<_> = sim.view().units.iter().filter(|u| u.owner == PlayerId(0)).map(|u| u.id).collect();
        assert!(!mine.is_empty());
        let out = scratch("hotkey");
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(1.0 / 60.0)))
            .configure_sets(Update, (GameSet::SimLink, GameSet::Ui).chain())
            .init_resource::<ButtonInput<KeyCode>>()
            .add_plugins((SimLinkPlugin, CapturePlugin))
            .insert_resource(ContentRes(s.content.clone()))
            .insert_resource(CaptureDir(out.clone()));
        // The link and meta as the map view arms them (without a VFS: a plain mount).
        let link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)).with_replay(Replay::new(&s));
        app.insert_resource(link).insert_resource(MatchMeta { start: StartMode::Starts, mount: MountInfo::default() });
        app.insert_resource(Selection(mine.clone()));
        app.world_mut().resource_mut::<CommandQueue>().0.push(CommandKind::Order(Order::Move {
            units: mine,
            to: SimVec3::from_f32([0.0, 0.0, 1200.0]),
            queue: false,
        }));
        for _ in 0..90 {
            app.update();
        }
        let tick_before = app.world().resource::<SimLink>().curr().tick;
        assert!(tick_before > 20, "the game ran: tick {tick_before}");
        // Not pressed: nothing written.
        assert!(!out.exists());
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::F12);
        }
        app.update();
        assert!(!out.exists(), "F12 alone is not the capture key");
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::F12);
            keys.clear();
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::F12);
        }
        app.update();
        let dirs = capture::list(&out);
        assert_eq!(dirs.len(), 1, "one capture folder in {}", out.display());

        let cap = Capture::load(&dirs[0]).unwrap();
        // The folder is a capture of the tick the sim stood at when the key was seen.
        assert!(cap.index.tick >= tick_before);
        assert!(!cap.index.selection.is_empty());
        assert!(cap.index.selection.iter().all(|u| u.owner == 0 && u.def.is_some() && u.race.as_deref() == Some("fx")));
        assert_eq!(cap.index.players.len(), 2);
        assert_eq!(cap.index.local_seat, Some(0));
        assert_eq!(cap.replay.commands.len(), 1, "the queued order is in the log");
        assert!(cap.snapshot.is_some());
        let verified = cap.verify(s.content.clone(), Some(&races)).unwrap();
        assert_eq!(verified.tick, cap.index.tick);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_link_without_a_log_is_not_captured() {
        let s = setup();
        let sim = Sim::from_setup(&s);
        let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0));
        let meta = MatchMeta { start: StartMode::Plain, mount: MountInfo::default() };
        assert!(take(&mut link, &meta, &s.content, &Looking::default()).is_none());
    }

    #[test]
    fn the_cursor_ray_meets_the_ground_in_map_space() {
        // Looking straight down from 100 above Bevy (3, 0, 4): sim z is mirrored.
        let ray = Ray3d::new(Vec3::new(3.0, 100.0, 4.0), Dir3::NEG_Y);
        assert_eq!(ground_point(ray), Some([3.0, 0.0, -4.0]));
        // Looking up or level: no ground.
        assert_eq!(ground_point(Ray3d::new(Vec3::new(0.0, 10.0, 0.0), Dir3::Y)), None);
        assert_eq!(ground_point(Ray3d::new(Vec3::new(0.0, 10.0, 0.0), Dir3::X)), None);
    }
}
