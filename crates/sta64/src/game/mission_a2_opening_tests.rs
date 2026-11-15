//! Install-gated (`STA64_TEST_A2_INSTALL`, needs `python3` and `objdump`): the opening of an
//! Armada II campaign mission (`STA64_TEST_A2_BZN`, default the first Klingon mission), converted on
//! this machine and driven headless from the screens with no input. It writes a per-frame trace of
//! what the host does (presentation requests, sounds started and stopped, fade, cineractive, camera)
//! to `STA64_TEST_TRACE_DIR` when set, and asserts the voice-line rules named below.
//!
//! The audio layer is the real one over a silent backend (`NullAudio`) with the install's sound
//! bank, so a clip's length is its decoded length. No window, no audio device.

use super::*;
use crate::audio::{AudioPlugin, BankResolver, Mixer, NullAudio, Resolver, SoundStarted};
use crate::input::camera_cinematic::{CinematicCamera, CinematicCameraPlugin};
use crate::sim_link::{PlaySound, StopSounds};
use crate::ui::presentation::{CineractiveState, FadeState, PresentationLog, ScriptSounds};
use bevy::time::TimeUpdateStrategy;
use std::fmt::Write as _;
use std::time::Duration;

const HZ: f64 = sta_core::TICK_HZ as f64;

#[derive(Resource, Default)]
struct Heard(Vec<(u64, String)>);

fn note(mut started: MessageReader<SoundStarted>, mut play: MessageReader<PlaySound>, mut stop: MessageReader<StopSounds>, mut out: ResMut<Heard>, link: Option<Res<SimLink>>) {
    let tick = link.map_or(0, |l| l.curr().tick);
    out.0.extend(play.read().map(|m| (tick, format!("play {} bus {:?}", m.name, m.bus))));
    out.0.extend(started.read().map(|m| (tick, format!("started {} bus {:?} length {:?}", m.name, m.bus, m.seconds))));
    out.0.extend(stop.read().map(|m| (tick, format!("stop-bus {:?}", m.bus))));
}

fn secs(t: u64) -> f64 {
    t as f64 / HZ
}

pub struct Frame {
    pub tick: u64,
    pub cine: bool,
    pub playing: Vec<u32>,
}

pub struct Trace {
    pub text: String,
    pub frames: Vec<Frame>,
    pub objectives: usize,
    pub heard: Vec<(u64, String)>,
}

/// Plays the mission from the screens for `seconds` of game time with no input.
pub fn drive(seconds: f64) -> Option<Trace> {
    let Ok(root) = std::env::var("STA64_TEST_A2_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A2_INSTALL to an Armada II root");
        return None;
    };
    let bzn = std::env::var("STA64_TEST_A2_BZN").unwrap_or_else(|_| "a2_kling01".into());
    let root = Path::new(&root);
    let data = temp_dir("a2open-data");
    let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/convert_install_missions.py");
    let run = std::process::Command::new("python3").arg("-I").arg(tool).arg("--install").arg(root).env("STA64_DATA_DIR", &data).output().expect("run the converter");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let conv = sta_mission::converted::dir_in(&data, root, None);
    let dir = temp_dir("a2open-menu");
    let vfs = sta_vfs::Vfs::mount_install(root, None).expect("mounts");
    let (mut cs, _) = load_campaigns_with(&vfs, Some(root), None, Some(&conv), |b| crate::frontend::campaign::ProgressStore::in_dir(&dir, b)).expect("campaigns");
    let (campaign, mission) = cs
        .book
        .campaigns
        .iter()
        .find_map(|c| c.missions.iter().find(|m| m.stem().eq_ignore_ascii_case(&bzn)).map(|m| (c.id.clone(), m.index)))
        .expect("the mission is in a campaign");
    let mut open = cs.book.clone();
    open.give_all_missions = true;
    cs.progress.apply_give_all(&open);
    let mut app = base(cs);
    let (null, _calls) = NullAudio::new();
    app.insert_resource(crate::viewer::Mounted(vfs))
        .init_resource::<crate::map::Catalogue>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Heard>()
        .insert_resource(PresentationLog::default())
        .insert_non_send(Mixer::new(Box::new(null)))
        .add_plugins((AudioPlugin::default(), CinematicCameraPlugin))
        .insert_resource(Resolver(Box::new(BankResolver::mount(root, None).expect("sound bank"))))
        .add_systems(Update, note.after(crate::app::GameSet::Audio))
        .add_systems(Update, build_install.run_if(resource_exists::<ActiveMatch>).run_if(resource_exists::<MissionLaunch>));
    app.world_mut().spawn((crate::viewer::Orbit { focus: Vec3::ZERO, yaw: 0.0, pitch: 0.7, dist: 400.0, home: (Vec3::ZERO, 1.0) }, Transform::default()));
    let hud = app.world_mut().spawn((crate::ui::hud::HudRoot, Visibility::Inherited)).id();
    let notices = app.world_mut().spawn((crate::ui::notices::NoticeRoot, Visibility::Inherited)).id();
    settle(&mut app, 4);
    app.world_mut().write_message(crate::frontend::campaign::StartMission { campaign, mission });
    until(&mut app, 600, |a| state(a) == AppState::InGame);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(0.1)));
    let mut text = String::new();
    let mut frames = Vec::new();
    let mut logged = 0;
    let mut heard_at = 0;
    for _ in 0..20_000 {
        app.update();
        let w = app.world();
        let tick = w.resource::<SimLink>().curr().tick;
        let (eye, focus) = {
            let o = app.world().iter_entities().find_map(|e| e.get::<crate::viewer::Orbit>().map(|o| (o.focus, o.yaw, o.pitch, o.dist))).unwrap();
            (crate::input::camera_cinematic::camera_translation(&crate::input::camera_cinematic::Pose { focus: o.0, yaw: o.1, pitch: o.2, dist: o.3 }), o.0)
        };
        let w = app.world();
        let hidden = *w.get::<Visibility>(hud).unwrap() == Visibility::Hidden && *w.get::<Visibility>(notices).unwrap() == Visibility::Hidden;
        frames.push(Frame { tick, cine: w.resource::<CineractiveState>().on, playing: w.resource::<ScriptSounds>().playing() });
        let _ = writeln!(
            text,
            "F t={:.2} tick={tick} cine={} locked={} hud_hidden={hidden} fade={:.2} playing={:?} eye=({:.0},{:.0},{:.0}) focus=({:.0},{:.0},{:.0})",
            secs(tick),
            w.resource::<CineractiveState>().on,
            w.resource::<CinematicCamera>().locked,
            w.resource::<FadeState>().alpha,
            w.resource::<ScriptSounds>().playing(),
            eye.x, eye.y, eye.z, focus.x, focus.y, focus.z
        );
        for (t, p) in &w.resource::<PresentationLog>().0[logged..] {
            let _ = writeln!(text, "P t={:.2} tick={t} {p:?}", secs(*t));
        }
        logged = w.resource::<PresentationLog>().0.len();
        for (t, h) in &w.resource::<Heard>().0[heard_at..] {
            let _ = writeln!(text, "A t={:.2} tick={t} {h}", secs(*t));
        }
        heard_at = w.resource::<Heard>().0.len();
        if secs(tick) >= seconds || state(&app) != AppState::InGame {
            break;
        }
    }
    let objectives = app.world().resource::<crate::ui::presentation::ObjectivesView>().items.len();
    let heard = app.world().resource::<Heard>().0.clone();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&data);
    if let Ok(out) = std::env::var("STA64_TEST_TRACE_DIR") {
        let _ = std::fs::create_dir_all(&out);
        let _ = std::fs::write(Path::new(&out).join(format!("{bzn}-host.txt")), &text);
    }
    Some(Trace { text, frames, objectives, heard })
}

/// The rules this opening depends on (the evidence for each is in the PR body):
/// - a stop of the music track is not a stop of the voice line playing beside it (the first voice
///   line, 50 s long, plays to its end: no bus cut at all, since nobody skips);
/// - the script's wait on its last music track ends with the clip, so the opening hands over
///   (cineractive off, objectives issued) after its last voice line instead of never.
#[test]
fn install_gated_a2_kling01_opening_keeps_its_voice_line_and_hands_over() {
    let Some(t) = drive(float_env("STA64_TEST_A2_SECONDS", 150.0)) else { return };
    assert!(!t.heard.iter().any(|(_, h)| h.starts_with("stop-bus")), "nothing stopped a bus without a skip: {:?}", t.heard.iter().filter(|(_, h)| h.starts_with("stop-bus")).collect::<Vec<_>>());
    // Rant_1 is message 2 (the track before it is message 1) and plays for its whole clip.
    let started = t.heard.iter().find(|(_, h)| h.starts_with("started a2_kling01_Martoks_Rant_1.wav")).expect("the first voice line starts");
    let length: f64 = started.1.split("Some(").nth(1).and_then(|r| r.trim_end_matches(')').parse().ok()).expect("its clip length is read from the file");
    assert!(length > 30.0, "{length}");
    let end = secs(started.0) + length;
    let playing_at = |s: f64| t.frames.iter().min_by(|a, b| (secs(a.tick) - s).abs().total_cmp(&(secs(b.tick) - s).abs())).unwrap().playing.contains(&2);
    assert!(playing_at(end - 2.0), "the first voice line is still playing {:.0} s in (clip {length:.0} s)", end - 2.0 - secs(started.0));
    // Control comes back after the last shot (Martok's speech is the longest wait), not later.
    let off = t.frames.iter().find(|f| secs(f.tick) > 100.0 && !f.cine).map(|f| secs(f.tick));
    assert!(off.is_some_and(|s| s < 125.0), "the opening hands over: {off:?}");
    assert!(t.objectives > 0, "the mission's objectives are issued");
}

#[test]
fn install_gated_a2_opening_trace() {
    let Some(t) = drive(float_env("STA64_TEST_A2_SECONDS", 200.0)) else { return };
    assert!(!t.text.is_empty());
}

fn float_env(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
