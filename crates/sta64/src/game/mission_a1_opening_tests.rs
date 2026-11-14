//! Install-gated (`STA64_TEST_A1_INSTALL`, needs `python3` and `objdump`): the opening of
//! Armada I Federation 1, converted on this machine and driven headless from the screens with no
//! input, asserting the order and timing a player sees: sounds, cineractive, fades, camera, and
//! the hand-over to the RTS view with the HUD, after the voice line has ended.
//!
//! The audio layer is the real one over a silent backend (`NullAudio`) with the install's sound
//! bank, so a clip's length is its decoded length. No window, no audio device. The rule asserted:
//! the opening waits for the voice line to end and then for the pull-back shot; clip lengths and
//! camera poses are read from the install at run time, never written into the test. Times are
//! game seconds, tolerance 1.5 s.

use super::*;
use crate::audio::{AudioPlugin, BankResolver, Mixer, NullAudio, Resolver, SoundStarted};
use crate::input::camera_cinematic::{CinematicCamera, CinematicCameraPlugin};
use crate::ui::presentation::{CineractiveState, FadeState, PresentationLog, ScriptSounds};
use bevy::time::TimeUpdateStrategy;
use sta_sim::present::Presentation;
use std::time::Duration;

const HZ: f64 = sta_core::TICK_HZ as f64;

#[derive(Resource, Default)]
struct Started(Vec<(u64, String, Option<f32>)>);

fn note_started(mut msgs: MessageReader<SoundStarted>, mut out: ResMut<Started>, link: Option<Res<SimLink>>) {
    let tick = link.map_or(0, |l| l.curr().tick);
    out.0.extend(msgs.read().map(|m| (tick, m.name.clone(), m.seconds)));
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    tick: u64,
    cine: bool,
    locked: bool,
    hud_hidden: bool,
    fade: f32,
    playing: usize,
    eye: Vec3,
}

struct Run {
    frames: Vec<Frame>,
    log: Vec<(u64, Presentation)>,
    started: Vec<(u64, String, Option<f32>)>,
    app: App,
}

fn secs(t: u64) -> f64 {
    t as f64 / HZ
}

/// Plays the mission from the screens for `seconds` of game time with no input.
fn drive(seconds: f64) -> Option<Run> {
    drive_mission(seconds, "fed1.bzn")
}

/// The same, for the A1 campaign mission whose map is `bzn` (all missions opened).
fn drive_mission(seconds: f64, bzn: &str) -> Option<Run> {
    let Ok(root) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A1_INSTALL to an Armada I root");
        return None;
    };
    let root = Path::new(&root);
    let data = temp_dir("a1open-data");
    let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/convert_install_missions.py");
    let run = std::process::Command::new("python3").arg("-I").arg(tool).arg("--install").arg(root).env("STA64_DATA_DIR", &data).output().expect("run the converter");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let conv = sta_mission::converted::dir_in(&data, root, None);
    let dir = temp_dir("a1open-menu");
    let vfs = sta_vfs::Vfs::mount_install(root, None).expect("mounts");
    let (mut cs, _) = load_campaigns_with(&vfs, Some(root), None, Some(&conv), |b| crate::frontend::campaign::ProgressStore::in_dir(&dir, b)).expect("A1 campaigns");
    let (campaign, mission) = cs.book.campaigns.iter().find_map(|c| c.missions.iter().find(|m| m.bzn.eq_ignore_ascii_case(bzn)).map(|m| (c.id.clone(), m.index))).expect("the mission is in a campaign");
    let mut open = cs.book.clone();
    open.give_all_missions = true;
    cs.progress.apply_give_all(&open);
    let mut app = base(cs);
    let (null, _calls) = NullAudio::new();
    app.insert_resource(crate::viewer::Mounted(vfs))
        .init_resource::<crate::map::Catalogue>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Started>()
        .insert_resource(PresentationLog::default())
        .insert_non_send(Mixer::new(Box::new(null)))
        .add_plugins((AudioPlugin::default(), CinematicCameraPlugin))
        .insert_resource(Resolver(Box::new(BankResolver::mount(root, None).expect("sound bank"))))
        .add_systems(Update, note_started.after(crate::app::GameSet::Audio))
        .add_systems(Update, build_install.run_if(resource_exists::<ActiveMatch>).run_if(resource_exists::<MissionLaunch>));
    app.world_mut().spawn((crate::viewer::Orbit { focus: Vec3::ZERO, yaw: 0.0, pitch: 0.7, dist: 400.0, home: (Vec3::ZERO, 1.0) }, Transform::default()));
    // Stand-ins for the HUD's and the notices' roots (the real plugins need a window).
    let hud = app.world_mut().spawn((crate::ui::hud::HudRoot, Visibility::Inherited)).id();
    let notices = app.world_mut().spawn((crate::ui::notices::NoticeRoot, Visibility::Inherited)).id();
    // `STA64_TEST_A1_SOUNDS_AT_ONCE=1` reproduces the old host (every message acknowledged at once): the
    // opening test then fails at the hand-over, which is how the playtest bug was measured.
    if std::env::var_os("STA64_TEST_A1_SOUNDS_AT_ONCE").is_some() {
        app.world_mut().remove_resource::<crate::audio::ReportsSoundLengths>();
    }
    settle(&mut app, 4);
    app.world_mut().write_message(crate::frontend::campaign::StartMission { campaign, mission });
    until(&mut app, 600, |a| state(a) == AppState::InGame);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(0.1)));
    let mut frames = Vec::new();
    for _ in 0..20_000 {
        app.update();
        let eye = {
            let o = app.world_mut().query::<&crate::viewer::Orbit>().single(app.world()).unwrap();
            crate::input::camera_cinematic::camera_translation(&crate::input::camera_cinematic::Pose { focus: o.focus, yaw: o.yaw, pitch: o.pitch, dist: o.dist })
        };
        let w = app.world();
        let f = Frame {
            tick: w.resource::<SimLink>().curr().tick,
            cine: w.resource::<CineractiveState>().on,
            locked: w.resource::<CinematicCamera>().locked,
            hud_hidden: *w.get::<Visibility>(hud).unwrap() == Visibility::Hidden && *w.get::<Visibility>(notices).unwrap() == Visibility::Hidden,
            fade: w.resource::<FadeState>().alpha,
            playing: w.resource::<ScriptSounds>().playing().len(),
            eye,
        };
        frames.push(f);
        if secs(f.tick) >= seconds || state(&app) != AppState::InGame {
            break;
        }
    }
    let log = app.world().resource::<PresentationLog>().0.clone();
    let started = app.world().resource::<Started>().0.clone();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&data);
    Some(Run { frames, log, started, app })
}

fn tick_of(run: &Run, pick: impl Fn(&Presentation) -> bool) -> Option<u64> {
    run.log.iter().find(|(_, p)| pick(p)).map(|(t, _)| *t)
}

fn near(what: &str, got_s: f64, want_s: f64) {
    assert!((got_s - want_s).abs() <= 1.5, "{what}: at {got_s:.1} s, the script says {want_s:.1} s");
}

#[test]
fn install_gated_a1_fed1_opening_runs_in_the_original_order_and_hands_over_after_the_voice_line() {
    let Some(run) = drive(80.0) else { return };
    let sound = |n: &str| tick_of(&run, |p| matches!(p, Presentation::Sound { name, .. } if name.eq_ignore_ascii_case(n)));
    let length = |n: &str| run.started.iter().find(|s| s.1.eq_ignore_ascii_case(n)).and_then(|s| s.2).unwrap_or_else(|| panic!("{n} resolved to a clip"));
    // Audio, camera, fade at the start: control locked, a fade in, the first music.
    let on = tick_of(&run, |p| matches!(p, Presentation::Cineractive { on: true, .. })).expect("cineractive on");
    assert!(secs(on) < 0.5, "the cineractive starts with the mission: {}", secs(on));
    let fade_in = tick_of(&run, |p| matches!(p, Presentation::Fade { out: false, .. })).expect("fade in");
    assert!(secs(fade_in) < 0.5, "{}", secs(fade_in));
    near("music Mfed25", secs(sound("Mfed25.wav").expect("the music is requested")), 0.0);
    // The voice line starts after the first shot, and the audio layer found the clip lengths.
    let log_at = secs(sound("F1PIC1.wav").expect("Picard's log is requested"));
    assert!(log_at > 5.0, "the voice line comes after the first shot: {log_at}");
    let (music_len, log_len) = (f64::from(length("Mfed25.wav")), f64::from(length("F1PIC1.wav")));
    assert!(music_len > 1.0 && log_len > 1.0, "clip lengths {music_len} / {log_len}");
    // The fade out the voice line scheduled begins while it plays and before the hand-over.
    let fade_out = tick_of(&run, |p| matches!(p, Presentation::Fade { out: true, .. })).expect("fade out");
    assert!(secs(fade_out) > log_at && secs(fade_out) < log_at + log_len + 1.0, "fade out at {:.1} s, voice line {log_at:.1}..{:.1}", secs(fade_out), log_at + log_len);
    // Control stays with the script until the log is over and the zoom-out is done.
    let off = tick_of(&run, |p| matches!(p, Presentation::Cineractive { on: false, .. })).expect("the cineractive ends");
    let log_ends = log_at + log_len;
    assert!(secs(off) >= log_ends + 4.0, "control is released at {:.1} s, before the voice line ends ({log_ends:.1} s) and the zoom-out", secs(off));
    near("control released", secs(off), log_ends + 5.0);
    // The camera, HUD and fade for that whole span.
    let cine: Vec<&Frame> = run.frames.iter().filter(|f| f.cine).collect();
    assert!(!cine.is_empty() && cine.iter().all(|f| f.locked && f.hud_hidden), "the camera is the script's and the HUD is hidden for the whole opening");
    assert!(run.frames.iter().filter(|f| secs(f.tick) > log_at + 1.0 && secs(f.tick) < log_ends - 0.5).all(|f| f.cine && f.playing >= 1), "the voice line is playing and the opening runs until it ends");
    // The three shots, relative: the first holds one eye, the second has left it, the third's
    // pull-back moves the eye between its start and its end.
    let eye_at = |s: f64| run.frames.iter().min_by(|a, b| (secs(a.tick) - s).abs().total_cmp(&(secs(b.tick) - s).abs())).unwrap().eye;
    let flyby = eye_at(5.0);
    assert!(eye_at(20.0).distance(flyby) <= 8.0, "shot 1 holds its eye: {flyby:?} / {:?}", eye_at(20.0));
    assert!(eye_at(log_at + 4.0).distance(flyby) > 100.0, "shot 2 has left the first shot's pose: {:?}", eye_at(log_at + 4.0));
    assert!(eye_at(log_ends + 0.5).distance(eye_at(log_ends + 4.5)) > 20.0, "shot 3 pulls back: {:?} -> {:?}", eye_at(log_ends + 0.5), eye_at(log_ends + 4.5));
    // After the hand-over: RTS view, HUD, no fade, and it stays so.
    let after: Vec<&Frame> = run.frames.iter().filter(|f| f.tick > off + 3).collect();
    assert!(secs(run.frames.last().unwrap().tick) > log_ends + 15.0, "the run covers the time after the hand-over");
    assert!(after.iter().all(|f| !f.cine && !f.locked && !f.hud_hidden && f.fade == 0.0), "RTS view with the HUD and a clear screen from the hand-over on: {:?}", after.iter().find(|f| f.cine || f.locked || f.hud_hidden || f.fade != 0.0));
    let w = run.app.world();
    assert!(!w.resource::<crate::ui::presentation::ObjectivesView>().items.is_empty(), "the first objectives are issued");
    assert!(w.resource::<MissionRun>().outcome.is_none(), "the mission goes on");
    // The sounds reached the backend: the music, the log and the zoom-out music, in that order.
    let names: Vec<&str> = run.started.iter().map(|s| s.1.as_str()).collect();
    let at = |n: &str| names.iter().position(|x| x.eq_ignore_ascii_case(n)).unwrap_or_else(|| panic!("{n} not played: {names:?}"));
    assert!(at("Mfed25.wav") < at("F1PIC1.wav") && at("F1PIC1.wav") < at("mfed5.wav"), "{names:?}");
}

#[test]
fn install_gated_a1_fed1_opening_skipped_with_space_ends_without_a_late_black_screen() {
    let Some(mut run) = drive(0.0) else { return };
    // Space at 30 s, in the middle of the voice line .
    while secs(run.frames.last().unwrap().tick) < 30.0 {
        run.app.update();
        let tick = run.app.world().resource::<SimLink>().curr().tick;
        run.frames.push(Frame { tick, cine: false, locked: false, hud_hidden: false, fade: 0.0, playing: 0, eye: Vec3::ZERO });
    }
    run.app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);
    let mut worst = 0.0f32;
    for _ in 0..800 {
        run.app.update();
        run.app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        let w = run.app.world();
        let tick = w.resource::<SimLink>().curr().tick;
        if secs(tick) > 40.0 {
            worst = worst.max(w.resource::<FadeState>().alpha);
        }
        if secs(tick) >= 75.0 {
            break;
        }
    }
    let w = run.app.world();
    assert!(!w.resource::<CineractiveState>().on && !w.resource::<CinematicCamera>().locked, "the skip hands the game over");
    assert_eq!(worst, 0.0, "the fade out the opening scheduled does not darken the RTS view after the skip");
}

/// The next two Federation openings (Armada I missions 2 and 3): each takes the camera and the
/// HUD for its opening, and hands the RTS view back with a clear screen. Prints what each did;
/// it asserts the hand-over only, not their timing (no reference read for them).
#[test]
fn install_gated_a1_fed2_and_fed3_openings_hand_the_game_over() {
    for bzn in ["fed2.bzn", "fed3.bzn"] {
        let Some(run) = drive_mission(150.0, bzn) else { return };
        let on = run.frames.iter().position(|f| f.cine);
        let off = run.frames.iter().rposition(|f| f.cine);
        let sounds: Vec<(f64, &str)> = run.started.iter().map(|s| (secs(s.0), s.1.as_str())).collect();
        eprintln!("{bzn}: cineractive frames {on:?}..{off:?} (ticks {:?}..{:?}), last tick {}, sounds {sounds:?}", on.map(|i| run.frames[i].tick), off.map(|i| run.frames[i].tick), run.frames.last().unwrap().tick);
        let last = run.frames.last().unwrap();
        assert!(on.is_some(), "{bzn}: the opening takes the camera");
        assert!(!last.cine && !last.locked && !last.hud_hidden && last.fade == 0.0, "{bzn}: the RTS view with the HUD and a clear screen after the opening: {last:?}");
    }
}
