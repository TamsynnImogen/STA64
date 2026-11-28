//! Install-gated (`STA64_TEST_A2_INSTALL`, needs `python3` and `objdump`): the opening of an
//! Armada II campaign mission (`STA64_TEST_A2_BZN`, default the first Klingon mission), converted on
//! this machine and driven headless from the screens with no input. It writes a per-frame trace of
//! what the host does (presentation requests, sounds started and stopped, fade, cineractive, camera)
//! to `STA64_TEST_TRACE_DIR` when set (never inside the repository: it is the script's sequence), and
//! asserts rules over the trace that hold for any mission, none of them taken from the script.
//!
//! The audio layer is the real one over a silent backend (`NullAudio`) with the install's sound
//! bank, so a clip's length is its decoded length. No window, no audio device.

use super::*;
use crate::audio::{AudioPlugin, BankResolver, Mixer, NullAudio, Resolver, SoundStarted};
use crate::input::camera_cinematic::{CinematicCamera, CinematicCameraPlugin};
use crate::sim_link::{PlayScriptSound, PlaySound, StopScriptSound, StopSounds};
use crate::ui::presentation::{CineractiveState, FadeState, PresentationLog, ScriptSounds};
use bevy::time::TimeUpdateStrategy;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;
use sta_sim::present::{Arg, Presentation};

const HZ: f64 = sta_core::TICK_HZ as f64;

#[derive(Resource, Default)]
struct Heard(Vec<(u64, String)>);

#[allow(clippy::too_many_arguments)]
fn note(mut started: MessageReader<SoundStarted>, mut play: MessageReader<PlaySound>, mut stop: MessageReader<StopSounds>, mut play_script: MessageReader<PlayScriptSound>, mut stop_script: MessageReader<StopScriptSound>, mut out: ResMut<Heard>, link: Option<Res<SimLink>>) {
    let tick = link.map_or(0, |l| l.curr().tick);
    out.0.extend(play.read().map(|m| (tick, format!("play {} bus {:?}", m.name, m.bus))));
    out.0.extend(started.read().map(|m| (tick, format!("started {} bus {:?} length {:?}", m.name, m.bus, m.seconds))));
    out.0.extend(play_script.read().map(|m| (tick, format!("play {} bus Ui", m.name))));
    out.0.extend(stop_script.read().map(|m| (tick, format!("stop-script {}", m.id))));
    out.0.extend(stop.read().map(|m| (tick, format!("stop-bus {:?}", m.bus))));
}

fn secs(t: u64) -> f64 {
    t as f64 / HZ
}

/// Removes the temp dirs when the test ends, panicking or not.
struct Cleanup(Vec<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for d in &self.0 {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

pub struct Frame {
    pub tick: u64,
    pub cine: bool,
    pub playing: Vec<u32>,
}

/// One script message: when it started, how long the audio layer says it is, when the script stopped it.
pub struct Msg {
    pub id: u32,
    pub start: u64,
    pub length: Option<f64>,
    pub stopped: Option<u64>,
}

pub struct Trace {
    pub frames: Vec<Frame>,
    pub msgs: Vec<Msg>,
    /// Every stop of one script stream: when, and which message id.
    pub ui_cuts: Vec<(u64, u32)>,
    pub objectives: usize,
}

/// Messages of the presentation log with the lengths the audio layer reported for them.
fn messages(log: &[(u64, Presentation)], heard: &[(u64, String)]) -> Vec<Msg> {
    let mut msgs: Vec<(Msg, String)> = Vec::new();
    for (tick, p) in log {
        match p {
            Presentation::Sound { id, name, .. } => msgs.push((Msg { id: *id, start: *tick, length: None, stopped: None }, name.clone())),
            Presentation::Call { name, args } if name == "A2.PlayMusic" => {
                if let [Arg::Int(id), Arg::Text(file)] = args.as_slice() {
                    msgs.push((Msg { id: *id as u32, start: *tick, length: None, stopped: None }, file.clone()));
                }
            }
            Presentation::StopSound { id } => {
                if let Some((m, _)) = msgs.iter_mut().rev().find(|(m, _)| m.id == *id && m.stopped.is_none()) {
                    m.stopped = Some(*tick);
                }
            }
            _ => {}
        }
    }
    for (tick, h) in heard {
        let Some(rest) = h.strip_prefix("started ") else { continue };
        let Some((name, tail)) = rest.split_once(" bus Ui length Some(") else { continue };
        let length: Option<f64> = tail.trim_end_matches(')').parse().ok();
        if let Some((m, _)) = msgs.iter_mut().find(|(m, n)| m.length.is_none() && m.start == *tick && n == name) {
            m.length = length;
        }
    }
    msgs.into_iter().map(|(m, _)| m).collect()
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
    let dir = temp_dir("a2open-menu");
    let _cleanup = Cleanup(vec![data.clone(), dir.clone()]);
    let conv = crate::game::convert_e2e_tests::hook_convert(root, &data).0;
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
    let (null, calls) = NullAudio::new();
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
        // What reached the mixer backend (every bus), as it would have sounded.
        for c in calls.drain() {
            if !matches!(c, crate::audio::Call::Listener { .. } | crate::audio::Call::BusVolume { .. }) {
                let _ = writeln!(text, "C t={:.2} tick={tick} {c:?}", secs(tick));
            }
        }
        if secs(tick) >= seconds || state(&app) != AppState::InGame {
            break;
        }
    }
    if let Some(h) = app.world().get_resource::<MissionHandle>() {
        for e in h.with(|rt| rt.errors().to_vec()) {
            let _ = writeln!(text, "E script error: {e}");
        }
    }
    let objectives = app.world().resource::<crate::ui::presentation::ObjectivesView>().items.len();
    let heard = app.world().resource::<Heard>().0.clone();
    let log = app.world().resource::<PresentationLog>().0.clone();
    if let Ok(out) = std::env::var("STA64_TEST_TRACE_DIR") {
        let out = PathBuf::from(out);
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let inside = out.canonicalize().or_else(|_| std::fs::create_dir_all(&out).and_then(|()| out.canonicalize())).is_ok_and(|o| repo.canonicalize().is_ok_and(|r| o.starts_with(r)));
        if !inside {
            let _ = std::fs::write(out.join(format!("{bzn}-host.txt")), &text);
        }
    }
    let ui_cuts = heard.iter().filter_map(|(t, h)| h.strip_prefix("stop-script ").and_then(|id| id.parse().ok()).map(|id| (*t, id))).collect();
    Some(Trace { frames, msgs: messages(&log, &heard), ui_cuts, objectives })
}

/// Rules over the trace of an opening that nobody skips (none comes from the script):
/// - a stop names one script stream and only one that is still playing (no bus is cut);
/// - every message plays for the length the audio layer reported, unless the script stopped it;
/// - the opening hands over once its last message has ended, well before the hold cap, and the
///   mission's objectives are issued.
#[test]
fn install_gated_a2_opening_obeys_the_message_and_hand_over_rules() {
    let Some(t) = drive(float_env("STA64_TEST_A2_SECONDS", 150.0)) else { return };
    let slack = 0.6;
    assert!(!t.msgs.is_empty() && t.msgs.iter().all(|m| m.length.is_some()), "every message resolved to a clip with a length");
    for &(cut, id) in &t.ui_cuts {
        // A stop names one stream, and only a stream that is still playing (a stale id is ignored).
        let m = t.msgs.iter().filter(|m| m.id == id && m.start <= cut).last().expect("a stop names a message the script started");
        assert!(secs(cut) < secs(m.start) + m.length.unwrap() + slack, "message {id} was stopped at {:.1} s, after its clip had ended", secs(cut));
    }
    for m in &t.msgs {
        let end = secs(m.start) + m.length.unwrap();
        let until = m.stopped.map_or(end, |s| secs(s).min(end)) - slack;
        let gaps: Vec<f64> = t.frames.iter().filter(|f| secs(f.tick) > secs(m.start) + 0.3 && secs(f.tick) < until && !f.playing.contains(&m.id)).map(|f| secs(f.tick)).collect();
        assert!(gaps.is_empty(), "message {} stopped playing early (first at {:.1} s, clip ends {:.1} s)", m.id, gaps[0], end);
    }
    let first_on = t.frames.iter().find(|f| f.cine).map(|f| f.tick).expect("the opening takes the camera");
    let handover = t.frames.iter().find(|f| f.tick > first_on && !f.cine).map(|f| secs(f.tick)).expect("the opening hands over");
    let cap = secs(first_on) + f64::from(sta_mission::cineractive::MAX_HOLD_SECS);
    assert!(handover < cap - 10.0, "the hand-over ({handover:.1} s) comes before the hold cap ({cap:.1} s), by the script's own end");
    let last_end = t.msgs.iter().filter(|m| secs(m.start) < handover).map(|m| secs(m.start) + m.length.unwrap()).fold(0.0, f64::max);
    assert!(handover <= last_end + 5.0, "the hand-over ({handover:.1} s) follows the last message's end ({last_end:.1} s) closely");
    assert!(t.objectives > 0, "the mission's objectives are issued");
}

fn float_env(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
