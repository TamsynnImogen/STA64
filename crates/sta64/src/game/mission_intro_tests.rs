//! Install-gated: the opening of Fleet Operations Roots' "Politics by Other Means", driven
//! headless from the start with no input (or one Space), asserting the whole ordered sequence
//! the script requests and what the host does with each step, up to the hand-over.
//!
//! Needs `STA64_TEST_ROOTS_INSTALL`. No window, no audio device: sounds are asserted as the
//! requests handed to the audio layer.

use super::*;
use crate::input::camera_cinematic::{CinematicCamera, CinematicCameraPlugin};
use crate::sim_link::PlaySound;
use crate::ui::presentation::{CineractiveState, FadeState, PresentationLog};
use sta_sim::present::Presentation;
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

#[derive(Resource, Default)]
struct Heard(Vec<(u64, String)>);

fn hear(mut msgs: MessageReader<PlaySound>, mut heard: ResMut<Heard>, link: Option<Res<SimLink>>) {
    let tick = link.map_or(0, |l| l.curr().tick);
    heard.0.extend(msgs.read().map(|m| (tick, m.name.clone())));
}

/// One frame's state.
#[derive(Debug, Clone, Copy)]
struct Frame {
    tick: u64,
    cine: bool,
    locked: bool,
    shot_placed: bool,
    hud_hidden: bool,
    fade: f32,
}

struct Run {
    frames: Vec<Frame>,
    log: Vec<(u64, Presentation)>,
    heard: Vec<(u64, String)>,
    /// The mission sim's tick on the first in-game frame.
    ticks_at_ingame: u64,
    vfs: sta_vfs::Vfs,
    app: App,
}

/// `load_secs`: the real time each frame of the loading screen takes (a long load is a big one).
fn drive(skip_at_tick: Option<u64>, load_secs: f32) -> Option<Run> {
    let Ok(root) = std::env::var("STA64_TEST_ROOTS_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_ROOTS_INSTALL to Fleet Operations Roots for the intro run");
        return None;
    };
    let dir = temp_dir("politics-intro");
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&root), None).expect("mounts");
    let (mut campaigns, _) = load_campaigns_in(&vfs, Some(Path::new(&root)), None, |b| crate::frontend::campaign::ProgressStore::in_dir(&dir, b)).expect("campaigns");
    let (campaign, mission) = campaigns.book.campaigns.iter().find_map(|c| c.missions.iter().find(|m| m.title.as_deref() == Some("Politics by Other Means")).map(|m| (c.id.clone(), m.index))).expect("the mission");
    let mut open = campaigns.book.clone();
    open.give_all_missions = true;
    campaigns.progress.apply_give_all(&open);
    let mut app = base(campaigns);
    app.insert_resource(crate::viewer::Mounted(sta_vfs::Vfs::mount_install(Path::new(&root), None).expect("mounts")))
        .init_resource::<crate::map::Catalogue>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Heard>()
        .insert_resource(PresentationLog::default())
        .add_plugins(CinematicCameraPlugin)
        .add_systems(Update, hear.after(crate::app::GameSet::Ui))
        .add_systems(Update, build_install.run_if(resource_exists::<ActiveMatch>).run_if(resource_exists::<MissionLaunch>));
    app.world_mut().spawn((crate::viewer::Orbit { focus: Vec3::ZERO, yaw: 0.0, pitch: 0.7, dist: 400.0, home: (Vec3::ZERO, 1.0) }, Transform::default()));
    // Stand-ins for the HUD's and the notices' roots (the real plugins need a window).
    let hud = app.world_mut().spawn((crate::ui::hud::HudRoot, Visibility::Inherited)).id();
    let notices = app.world_mut().spawn((crate::ui::notices::NoticeRoot, Visibility::Inherited)).id();
    settle(&mut app, 4);
    app.world_mut().write_message(crate::frontend::campaign::StartMission { campaign, mission });
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(load_secs)));
    settle(&mut app, 3);
    until(&mut app, 600, |a| state(a) == AppState::InGame);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(0.1)));
    let ticks_at_ingame = app.world().get_resource::<SimLink>().map_or(0, |l| l.curr().tick);
    let mut frames = Vec::new();
    let mut skipped = false;
    for _ in 0..3000 {
        if let Some(t) = skip_at_tick
            && !skipped
            && app.world().resource::<SimLink>().curr().tick >= t
        {
            app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);
            skipped = true;
        }
        app.update();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        let w = app.world();
        let cam = w.resource::<CinematicCamera>();
        let f = Frame {
            tick: w.resource::<SimLink>().curr().tick,
            cine: w.resource::<CineractiveState>().on,
            locked: cam.locked,
            shot_placed: cam.model.shot_placed,
            hud_hidden: *w.get::<Visibility>(hud).unwrap() == Visibility::Hidden && *w.get::<Visibility>(notices).unwrap() == Visibility::Hidden,
            fade: w.resource::<FadeState>().alpha,
        };
        frames.push(f);
        if f.tick >= 1500 || state(&app) != AppState::InGame {
            break;
        }
    }
    let log = app.world().resource::<PresentationLog>().0.clone();
    let heard = app.world().resource::<Heard>().0.clone();
    std::fs::remove_dir_all(dir).ok();
    Some(Run { frames, log, heard, ticks_at_ingame, vfs, app })
}

fn kind(p: &Presentation) -> String {
    match p {
        Presentation::Cineractive { on, .. } => format!("cineractive {}", if *on { "on" } else { "off" }),
        Presentation::Fade { out, .. } => format!("fade {}", if *out { "out" } else { "in" }),
        Presentation::Sound { name, .. } => format!("sound {name}"),
        Presentation::StopSound { .. } => "stop sounds".into(),
        Presentation::Call { name, args } if name == "UI.addTitle" || name == "UI.addSubtitle" => format!("{name} {:?}", args.get(1)),
        Presentation::Call { name, args } if name == "Cineractive.lookFrom" => format!("lookFrom {:?}", args.first()),
        Presentation::Call { name, .. } => name.clone(),
        Presentation::CameraOffset { .. } => "camera offset".into(),
        other => format!("{other:?}").chars().take(30).collect(),
    }
}

/// The ordered opening, as the script requests it, with the sim ticks the host consumed each
/// step on (the script's timers: sound at 3 s, second shot at 26 s, fade out at 35 s, end at 37 s).
fn assert_opening_requests(run: &Run) {
    let first_cine_off = run.log.iter().position(|(_, p)| matches!(p, Presentation::Cineractive { on: false, .. })).expect("the cineractive ends");
    let steps: Vec<(u64, String)> = run.log[..=first_cine_off].iter().filter(|(_, p)| !matches!(p, Presentation::Call { name, .. } if name == "Cineractive.lookAt")).map(|(t, p)| (*t, kind(p))).collect();
    let names: Vec<&str> = steps.iter().map(|(_, k)| k.as_str()).collect();
    let want_in_order = [
        "cineractive on",
        "fade in",
        "UI.addTitle Some(Text(\"Politics by Other Means\"))",
        "UI.addSubtitle Some(Text(\"Press spacebar to skip\"))",
        "lookFrom Some(Text(\"path_1\"))",
        "camera offset",
        "sound PbOM_Intro.mp3",
        "lookFrom Some(Text(\"path_2\"))",
        "camera offset",
        "fade out",
        "stop sounds",
        "cineractive off",
    ];
    let mut at = 0;
    for w in want_in_order {
        at += names[at..].iter().position(|n| *n == w).unwrap_or_else(|| panic!("step {w:?} missing or out of order in {names:?}")) + 1;
    }
    let tick_of = |k: &str| steps.iter().find(|(_, n)| n == k).unwrap().0;
    let near = |what: &str, got: u64, want: u64| assert!(got.abs_diff(want) <= 3, "{what}: tick {got}, the script's timer says {want}");
    near("intro sound (3 s)", tick_of("sound PbOM_Intro.mp3"), 91);
    near("second shot (26 s)", tick_of("lookFrom Some(Text(\"path_2\"))"), 781);
    near("fade out (35 s)", tick_of("fade out"), 1051);
    near("end (37 s)", tick_of("cineractive off"), 1111);
}

fn assert_handed_over(run: &mut Run) {
    let last = *run.frames.last().unwrap();
    assert!(!last.cine && !last.locked && !last.hud_hidden, "control is the player's again, with the HUD: {last:?}");
    assert_eq!(last.fade, 0.0, "the screen is not black");
    let w = run.app.world();
    assert!(!w.resource::<CinematicCamera>().locked, "the camera is the player's");
    assert!(!w.resource::<crate::ui::presentation::ObjectivesView>().items.is_empty(), "the first objectives are issued");
    assert!(w.resource::<MissionRun>().outcome.is_none(), "the mission goes on");
    assert_eq!(state(&run.app), AppState::InGame);
}

#[test]
fn install_gated_politics_opening_runs_in_order_and_hands_the_game_over() {
    let Some(mut run) = drive(None, 0.1) else { return };
    assert_opening_requests(&run);
    // Camera: the first shot is taken in the first frames, held through both shots, never the player's meanwhile.
    let begin = run.frames.iter().position(|f| f.cine).expect("cineractive on");
    let end = run.frames.iter().rposition(|f| f.cine).unwrap();
    let placed = run.frames.iter().position(|f| f.shot_placed).expect("a shot is placed");
    assert!(run.frames[placed].tick <= 100, "the first shot (path_1) is taken at the start, not at the second one: tick {}", run.frames[placed].tick);
    assert!(run.frames[begin..=end].iter().all(|f| f.locked), "the camera is the script's for the whole intro");
    // HUD hidden for the whole intro, back after.
    assert!(run.frames[begin..=end].iter().all(|f| f.hud_hidden), "no HUD over the intro");
    assert!(run.frames[begin].fade > 0.5, "the opening starts black, not on the RTS view: {:?}", run.frames[begin]);
    // Fade: in at the start (clear by 2 s), out near the end (black), then clear at the hand-over.
    let max_fade = run.frames[begin..=end].iter().map(|f| f.fade).fold(0.0, f32::max);
    assert!(max_fade > 0.9, "the fade out reached black before the end: {max_fade}");
    // Voice lines: each requested sound exists in the install and reached the audio layer.
    let asked: Vec<&str> = run.log.iter().filter_map(|(_, p)| if let Presentation::Sound { name, .. } = p { Some(name.as_str()) } else { None }).collect();
    assert_eq!(asked, ["PbOM_Intro.mp3"]);
    for n in &asked {
        assert!(sta_assets::audio::find_sound(&run.vfs, n).is_some(), "{n} is in the install");
        assert!(run.heard.iter().any(|(_, h)| h == n), "{n} was handed to the audio layer: {:?}", run.heard);
    }
    assert_handed_over(&mut run);
}

#[test]
fn install_gated_politics_opening_skipped_with_space_ends_in_the_same_playable_state() {
    let Some(mut run) = drive(Some(300), 0.1) else { return };
    let end = run.frames.iter().rposition(|f| f.cine).expect("the intro ran");
    assert!(run.frames[end].tick < 600, "the skip ended the intro early: tick {}", run.frames[end].tick);
    assert_handed_over(&mut run);
}

#[test]
fn install_gated_politics_sim_starts_with_the_game_whatever_the_load_took() {
    let (Some(short), Some(long)) = (drive(None, 0.1), drive(None, 3.0)) else { return };
    // The mission's tick 0 is the first in-game frame: nothing ran behind the loading screen.
    assert_eq!(short.ticks_at_ingame, 1, "no mission tick before InGame; the first frame shown runs the first (short load)");
    assert_eq!(long.ticks_at_ingame, 1, "no mission tick before InGame; the first frame shown runs the first (a long load, 3 s a frame)");
    // Each request is carried out on the tick the script issued it, load or no load.
    let ticks = |r: &Run| r.log.iter().map(|(t, p)| (*t, kind(p))).collect::<Vec<_>>();
    assert_eq!(ticks(&short), ticks(&long), "the opening's timeline does not depend on the load");
    assert_opening_requests(&short);
    assert_opening_requests(&long);
    // The camera's paths were there for the first tick: the first shot is taken on the first frames.
    let first = |r: &Run| r.frames.iter().find(|f| f.shot_placed).map(|f| f.tick);
    assert!(first(&long).is_some_and(|t| t <= 6), "{:?}", first(&long));
}
