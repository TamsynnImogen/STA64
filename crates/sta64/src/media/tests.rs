//! Playback tests on a synthetic movie source: no game data, no device, no window.
//!
//! The source writes the frame number into the picture's red channel and
//! interleaves a tone per frame like a Bink file does, so a test can tell what is on screen
//! at any update. Time is stepped by hand (a fixed `dt` per update) and the silent audio
//! backend consumes the movie's sound at that same virtual rate, which is the audio clock.

use std::time::Duration;

use bevy::time::TimeUpdateStrategy;
use sta_media::{AudioChunk, AudioInfo, Container, MediaError, MediaEvent, MediaSource, Movie, Picture, Plane, Rational, StreamInfo, VideoFrame, VideoInfo};

use super::locate::Opened;
use super::*;
use crate::audio::{AudioPlugin, Call, CallLog, NullAudio};

const FPS: u32 = 30;
const RATE: u32 = 8000;

struct Fake {
    info: StreamInfo,
    frames: u64,
    step: u64,
    audio: bool,
    /// Sound is delivered for the first this-many frames only.
    audio_until: u64,
}

impl Fake {
    fn movie(frames: u64, audio: bool) -> Movie {
        Self::movie_until(frames, audio, u64::MAX)
    }

    fn movie_until(frames: u64, audio: bool, audio_until: u64) -> Movie {
        let info = StreamInfo {
            container: Container::Bink,
            video: Some(VideoInfo { width: 4, height: 2, frame_rate: Rational::new(FPS, 1), frame_count: Some(frames), codec: "fake".into(), has_alpha: false }),
            audio: if audio { vec![AudioInfo { sample_rate: RATE, channels: 1, codec: "fake".into(), total_frames: None }] } else { vec![] },
            duration: None,
        };
        Movie::from_source(Box::new(Fake { info, frames, step: 0, audio, audio_until }))
    }
}

impl MediaSource for Fake {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn next_event(&mut self) -> sta_media::Result<Option<MediaEvent>> {
        let (i, audio_turn) = (self.step / 2, self.step % 2 == 1);
        if i >= self.frames {
            return Ok(None);
        }
        let fps = Rational::new(FPS, 1);
        if !audio_turn {
            self.step += if self.audio { 1 } else { 2 };
            let px: Vec<u8> = (0..8).flat_map(|_| [i as u8, 0, 0, 255]).collect();
            return Ok(Some(MediaEvent::Video(VideoFrame {
                index: i,
                pts: fps.periods_to_duration(i),
                duration: fps.periods_to_duration(1),
                keyframe: true,
                picture: Picture::Rgba(Plane::packed(4, 2, 4, px)),
            })));
        }
        self.step += 1;
        if i >= self.audio_until {
            return self.next_event();
        }
        let n = (RATE / FPS) as usize;
        Ok(Some(MediaEvent::Audio(AudioChunk {
            track: 0,
            pts: fps.periods_to_duration(i),
            first_frame: i * n as u64,
            sample_rate: RATE,
            channels: 1,
            samples: vec![0.1; n],
        })))
    }

    fn seek(&mut self, _to: Duration) -> sta_media::Result<Duration> {
        Err(MediaError::Unsupported("fake".into()))
    }
}

#[derive(Resource, Default)]
struct Finished(Vec<(MovieId, String, MovieOutcome)>);

fn collect(mut done: MessageReader<MovieFinished>, mut log: ResMut<Finished>) {
    for d in done.read() {
        log.0.push((d.id, d.name.clone(), d.outcome.clone()));
    }
}

/// An app with the media plugin, a silent mixer and a stepped clock. Names open as fake movies:
/// `Nf` frames long with sound, `Nv` without, anything else is missing.
fn app() -> (App, CallLog) {
    let (null, log) = NullAudio::new();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / FPS as f64)))
        .insert_non_send(Mixer::new(Box::new(null)))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Finished>()
        .insert_resource(MovieOpener(Box::new(|name: &str| {
            let (n, kind) = name.split_at(name.len() - 1);
            match (n.parse::<u64>(), kind) {
                (Ok(n), "f") => Ok(Opened { movie: Fake::movie(n, true), path: name.into(), sidecar: None }),
                (Ok(n), "a") => Ok(Opened { movie: Fake::movie_until(n, true, 20), path: name.into(), sidecar: None }),
                (Ok(n), "s") => Ok(Opened { movie: Fake::movie(n, false), path: name.into(), sidecar: Some((RATE, vec![[0.0f32; 2]; RATE as usize * 5].into())) }),
                (Ok(n), "v") => Ok(Opened { movie: Fake::movie(n, false), path: name.into(), sidecar: None }),
                _ => Err(format!("{name}: no such movie")),
            }
        }) as Box<OpenFn>))
        .add_plugins(AudioPlugin::default())
        .add_plugins(MediaPlugin)
        .add_systems(Last, collect);
    crate::app::configure_sets(&mut app);
    app.update();
    log.drain();
    (app, log)
}

fn play(app: &mut App, name: &str, target: MovieTarget, skippable: bool) -> MovieId {
    let id = app.world_mut().resource_mut::<MovieIds>().next();
    app.world_mut().write_message(PlayMovie { id, name: name.into(), target, skippable });
    id
}

fn run_until(app: &mut App, what: &str, mut done: impl FnMut(&mut App) -> bool) -> usize {
    for n in 0..4000 {
        app.update();
        if done(app) {
            return n;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("never: {what}");
}

fn finished(app: &App) -> Vec<(MovieId, String, MovieOutcome)> {
    app.world().resource::<Finished>().0.clone()
}

fn panel(app: &mut App) -> Handle<Image> {
    app.world_mut().resource_mut::<Assets<Image>>().add(blank(1, 1))
}

fn red(app: &App, h: &Handle<Image>) -> u8 {
    app.world().resource::<Assets<Image>>().get(h).and_then(|i| i.data.as_ref()).map_or(255, |d| d[0])
}

#[test]
fn plays_a_movie_into_a_panel_in_step_with_its_sound_and_reports_done() {
    let (mut app, log) = app();
    let h = panel(&mut app);
    let id = play(&mut app, "60f", MovieTarget::Panel(h.clone()), false);
    let mut started = None;
    let mut seen = Vec::new();
    let n = run_until(&mut app, "60 frames play out", |a| {
        let log_now = log.calls();
        if started.is_none() && log_now.iter().any(|c| matches!(c, Call::PlayStream { .. })) {
            started = Some(0usize);
        }
        if let Some(s) = &mut started {
            *s += 1;
            seen.push(red(a, &h));
        }
        !finished(a).is_empty()
    });
    assert_eq!(finished(&app), [(id, "60f".to_owned(), MovieOutcome::Completed)]);
    assert!(log.calls().contains(&Call::PlayStream { sample_rate: RATE, bus: MOVIE_BUS }));
    // 60 frames at 30 fps from a sound consumed at the virtual rate: about 60 updates after the start.
    let played = started.unwrap();
    assert!((54..=70).contains(&played), "{played} updates of playback (total {n})");
    // The panel goes dark when the movie ends; that last sample is not a picture.
    if seen.last() == Some(&0) {
        seen.pop();
    }
    // The picture only moves forward and ends on the last frames.
    assert!(seen.windows(2).all(|w| w[0] <= w[1] || w[0] == 255), "{seen:?}");
    assert!(seen.iter().copied().filter(|r| *r != 255).max().unwrap() >= 57, "{seen:?}");
    // Not much more than one frame step apart from the audio clock at any update.
    let mut prev = 0i32;
    for r in seen.iter().filter(|r| **r != 255 && **r != 0) {
        assert!((*r as i32 - prev).abs() <= 3, "jumped to {r} from {prev}: {seen:?}");
        prev = *r as i32;
    }
}

#[test]
fn a_silent_movie_runs_on_the_wall_clock() {
    let (mut app, log) = app();
    let h = panel(&mut app);
    play(&mut app, "30v", MovieTarget::Panel(h), false);
    run_until(&mut app, "silent movie ends", |a| !finished(a).is_empty());
    assert_eq!(finished(&app)[0].2, MovieOutcome::Completed);
    assert!(!log.calls().iter().any(|c| matches!(c, Call::PlayStream { .. })));
}

#[test]
fn missing_and_unsupported_movies_fail_soft_and_the_queue_goes_on() {
    let (mut app, _log) = app();
    let a = play(&mut app, "nothing", MovieTarget::Fullscreen, true);
    let b = play(&mut app, "20f", MovieTarget::Fullscreen, true);
    let c = play(&mut app, "also-missing", MovieTarget::Fullscreen, true);
    run_until(&mut app, "all three report", |a| finished(a).len() == 3);
    let f = finished(&app);
    assert_eq!(f.iter().map(|x| x.0).collect::<Vec<_>>(), [a, b, c], "in request order");
    assert!(matches!(f[0].2, MovieOutcome::Failed(_)) && matches!(f[2].2, MovieOutcome::Failed(_)));
    assert!(f.iter().all(|x| x.2.is_done()));
    assert_eq!(f[1].2, MovieOutcome::Completed);
    // The overlay is gone and nothing is left playing.
    assert!(!app.world().resource::<Movies>().busy());
    assert_eq!(app.world_mut().query::<&MovieOverlay>().iter(app.world()).count(), 0);
}

#[test]
fn fullscreen_movies_play_one_after_another() {
    let (mut app, _log) = app();
    let first = play(&mut app, "30f", MovieTarget::Fullscreen, true);
    let second = play(&mut app, "30f", MovieTarget::Fullscreen, true);
    app.update();
    app.update();
    assert_eq!(app.world_mut().query::<&MovieOverlay>().iter(app.world()).count(), 1, "one overlay at a time");
    assert_eq!(app.world().resource::<Movies>().active_ids(), [first, second]);
    run_until(&mut app, "both end", |a| finished(a).len() == 2);
    assert_eq!(finished(&app).iter().map(|x| x.0).collect::<Vec<_>>(), [first, second]);
}

#[test]
fn escape_skips_one_skippable_movie_and_the_next_still_plays() {
    let (mut app, _log) = app();
    let first = play(&mut app, "3000f", MovieTarget::Fullscreen, true);
    let second = play(&mut app, "10f", MovieTarget::Fullscreen, true);
    run_until(&mut app, "first is on screen", |a| a.world().resource::<Movies>().fullscreen.as_ref().is_some_and(|p| matches!(p.phase, Phase::Playing)));
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
    run_until(&mut app, "both report", |a| finished(a).len() == 2);
    let f = finished(&app);
    assert_eq!((f[0].0, &f[0].2), (first, &MovieOutcome::Skipped));
    assert_eq!((f[1].0, &f[1].2), (second, &MovieOutcome::Completed));
    // The key press was taken so the screen under the movie does not see it.
    assert!(!app.world().resource::<ButtonInput<KeyCode>>().just_pressed(KeyCode::Escape));
}

#[test]
fn a_movie_that_may_not_be_skipped_ignores_escape() {
    let (mut app, _log) = app();
    play(&mut app, "40f", MovieTarget::Fullscreen, false);
    run_until(&mut app, "playing", |a| a.world().resource::<Movies>().fullscreen.as_ref().is_some_and(|p| matches!(p.phase, Phase::Playing)));
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
    run_until(&mut app, "ends", |a| !finished(a).is_empty());
    assert_eq!(finished(&app)[0].2, MovieOutcome::Completed);
}

#[test]
fn stop_reports_playing_and_waiting_movies_and_shuts_their_threads_and_sound() {
    let (mut app, log) = app();
    let h = panel(&mut app);
    let panel_id = play(&mut app, "3000f", MovieTarget::Panel(h), false);
    let full = play(&mut app, "3000f", MovieTarget::Fullscreen, true);
    let waiting = play(&mut app, "3000f", MovieTarget::Fullscreen, true);
    run_until(&mut app, "playing", |a| a.world().resource::<Movies>().fullscreen.as_ref().is_some_and(|p| matches!(p.phase, Phase::Playing)));
    let streams = log.calls().iter().filter(|c| matches!(c, Call::PlayStream { .. })).count();
    assert!(streams >= 1);
    app.world_mut().write_message(StopMovie(None));
    run_until(&mut app, "all stopped", |a| finished(a).len() == 3);
    let mut ids: Vec<_> = finished(&app).into_iter().map(|x| (x.0, x.2)).collect();
    ids.sort_by_key(|x| x.0.0);
    assert_eq!(ids, [(panel_id, MovieOutcome::Stopped), (full, MovieOutcome::Stopped), (waiting, MovieOutcome::Stopped)]);
    assert!(!app.world().resource::<Movies>().busy());
    assert_eq!(app.world_mut().query::<&MovieOverlay>().iter(app.world()).count(), 0);
}

#[test]
fn mission_movie_events_play_and_report_done() {
    use sta_sim::SimEvent;
    use sta_sim::present::Presentation;
    let (mut app, _log) = app();
    let screen = panel(&mut app);
    app.insert_resource(PanelScreen(screen.clone()));
    let ev = |p| crate::sim_link::SimMsg(SimEvent::Present(p));
    app.world_mut().write_message(ev(Presentation::Movie { name: "40f".into(), in_panel: true, skippable: false }));
    app.world_mut().write_message(ev(Presentation::Movie { name: "10f".into(), in_panel: false, skippable: true }));
    run_until(&mut app, "both report", |a| finished(a).len() == 2);
    let f = finished(&app);
    assert!(f.iter().all(|x| x.2 == MovieOutcome::Completed), "{f:?}");
    // The in-panel movie went into the shared viewscreen image and the other into the overlay.
    let img = app.world().resource::<Assets<Image>>().get(&screen).unwrap();
    assert_eq!((img.width(), img.height()), (4, 2));
    // StopMovie from a script stops what is playing.
    app.world_mut().write_message(ev(Presentation::Movie { name: "3000f".into(), in_panel: false, skippable: true }));
    app.update();
    app.world_mut().write_message(ev(Presentation::StopMovie));
    run_until(&mut app, "stopped", |a| finished(a).len() == 3);
    assert_eq!(finished(&app)[2].2, MovieOutcome::Stopped);
}

#[test]
fn music_is_ducked_during_a_fullscreen_movie_and_restored() {
    let (mut app, _log) = app();
    let before = app.world().resource::<Volumes>().music;
    assert!(before > 0.0);
    play(&mut app, "40f", MovieTarget::Fullscreen, true);
    app.update();
    app.update();
    assert_eq!(app.world().resource::<Volumes>().music, 0.0);
    run_until(&mut app, "ends", |a| !finished(a).is_empty());
    app.update();
    assert_eq!(app.world().resource::<Volumes>().music, before);
}

fn intro_app(enabled: bool, files: &[(&str, &[u8])]) -> (App, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("sta64-intro-app-{}-{enabled}", std::process::id()));
    for (p, bytes) in files {
        let path = dir.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let mut vfs = sta_vfs::Vfs::default();
    vfs.push_dir("root".into(), dir.clone()).unwrap();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .insert_resource(Mounted(vfs))
        .insert_resource(IntroSettings { enabled })
        .init_resource::<Finished>()
        .add_plugins((MediaPlugin, IntroPlugin))
        .add_systems(Last, collect);
    crate::app::configure_sets(&mut app);
    (app, dir)
}

#[test]
fn startup_queues_the_installs_intro_in_order_and_broken_files_do_not_stop_it() {
    // Not movies: each opens as UnknownFormat, so each fails soft and the next one still runs.
    let files: [(&str, &[u8]); 4] = [
        ("RTS_CFG.h", b"#define cfgINTRO_MOVIE_NAME \"Opening.bik\"\n"),
        ("animations/Activision.bik", b"not a movie"),
        ("animations/Paramount.bik", b"BIK"),
        ("animations/Opening.bik", b"\0\0\0\0ftyp but truncated"),
    ];
    let (mut app, dir) = intro_app(true, &files);
    run_until(&mut app, "three failures reported", |a| finished(a).len() == 3);
    let f = finished(&app);
    assert_eq!(f.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), ["Activision.bik", "Paramount.bik", "Opening.bik"]);
    assert!(f.iter().all(|x| matches!(x.2, MovieOutcome::Failed(_)) && x.2.is_done()), "{f:?}");
    assert!(!app.world().resource::<Movies>().busy());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn nointro_and_the_setting_queue_nothing() {
    let files: [(&str, &[u8]); 1] = [("animations/Activision.bik", b"x")];
    let (mut app, dir) = intro_app(false, &files);
    for _ in 0..10 {
        app.update();
    }
    assert!(finished(&app).is_empty() && !app.world().resource::<Movies>().busy());
    std::fs::remove_dir_all(dir).ok();
}

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, empty for none): the install's own intro
/// sequence opens and its movies decode and play through the real player, on real Bink files.
#[test]
fn install_intro_sequence_plays() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let mod_name = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), mod_name.as_deref()).expect("mount");
    let names = intro::sequence(&vfs);
    eprintln!("intro sequence of {root}: {names:?}");
    let (null, _log) = NullAudio::new();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / FPS as f64)))
        .insert_non_send(Mixer::new(Box::new(null)))
        .init_resource::<Finished>()
        .insert_resource(Mounted(vfs))
        .add_plugins(AudioPlugin::default())
        .add_plugins(MediaPlugin)
        .add_systems(Last, collect);
    crate::app::configure_sets(&mut app);
    for (i, name) in names.iter().enumerate() {
        let id = play(&mut app, name, MovieTarget::Fullscreen, true);
        // The long title movie is stopped after a few seconds of picture; the short reels play to their end.
        let mut shown_after = 0;
        for _ in 0..40_000 {
            app.update();
            std::thread::sleep(Duration::from_micros(500));
            let m = app.world().resource::<Movies>();
            if let Some(p) = &m.fullscreen {
                shown_after = p.shown;
                if p.shown >= 90 && i == names.len() - 1 {
                    app.world_mut().write_message(StopMovie(Some(id)));
                }
            }
            if finished(&app).len() > i {
                break;
            }
        }
        let f = finished(&app);
        assert_eq!(f.len(), i + 1, "{name} never ended");
        assert!(matches!(f[i].2, MovieOutcome::Completed | MovieOutcome::Stopped), "{name}: {:?}", f[i].2);
        eprintln!("{name}: {:?} after {shown_after} pictures", f[i].2);
    }
}

/// A backend that takes no streams, like a missing device.
struct Deaf;

impl crate::audio::AudioBackend for Deaf {
    fn name(&self) -> &'static str {
        "deaf"
    }
    fn play(&mut self, _: &crate::audio::Sound, _: Bus, _: Option<[f32; 3]>) {}
    fn bus_busy(&self, _: Bus) -> bool {
        false
    }
    fn set_bus_volume(&mut self, _: crate::audio::MixBus, _: f32) {}
    fn stop_all(&mut self) {}
    fn stop_bus(&mut self, _: Bus) {}
    fn set_listener(&mut self, _: [f32; 3], _: [f32; 3]) {}
}

#[test]
fn a_backend_that_takes_no_stream_does_not_hold_the_movie_back() {
    let (mut app, _log) = app();
    app.insert_non_send(Mixer::new(Box::new(Deaf)));
    let h = panel(&mut app);
    // Long enough that two seconds of queued sound would have stalled the decoder.
    play(&mut app, "300f", MovieTarget::Panel(h.clone()), false);
    run_until(&mut app, "ends on the wall clock", |a| !finished(a).is_empty());
    assert_eq!(finished(&app)[0].2, MovieOutcome::Completed);
    assert!(red(&app, &h) >= 250 || red(&app, &h) < 50, "last frame shown or panel blanked");
}

#[test]
fn sound_shorter_than_the_picture_lets_the_picture_go_on() {
    let (mut app, _log) = app();
    let h = panel(&mut app);
    // Sound for 20 frames, picture for 200: more than the decoder's frame bound ahead of the sound clock.
    play(&mut app, "200a", MovieTarget::Panel(h), false);
    run_until(&mut app, "ends although the sound stopped", |a| !finished(a).is_empty());
    assert_eq!(finished(&app)[0].2, MovieOutcome::Completed);
}

#[test]
fn a_soundtrack_file_beside_a_silent_movie_does_not_starve_the_picture() {
    let (mut app, log) = app();
    let h = panel(&mut app);
    play(&mut app, "60s", MovieTarget::Panel(h.clone()), false);
    let mut top = 0u8;
    run_until(&mut app, "ends", |a| {
        let r = red(a, &h);
        if r != 255 {
            top = top.max(r);
        }
        !finished(a).is_empty()
    });
    assert_eq!(finished(&app)[0].2, MovieOutcome::Completed);
    assert!(top >= 55, "pictures reached frame {top}");
    assert!(log.calls().contains(&Call::PlayStream { sample_rate: RATE, bus: MOVIE_BUS }));
}

#[test]
fn a_second_movie_for_one_panel_replaces_the_first() {
    let (mut app, _log) = app();
    let h = panel(&mut app);
    let first = play(&mut app, "3000f", MovieTarget::Panel(h.clone()), false);
    app.update();
    app.update();
    let second = play(&mut app, "20f", MovieTarget::Panel(h), false);
    run_until(&mut app, "both report", |a| finished(a).len() == 2);
    let f = finished(&app);
    assert_eq!((f[0].0, &f[0].2), (first, &MovieOutcome::Stopped));
    assert_eq!((f[1].0, &f[1].2), (second, &MovieOutcome::Completed));
}

#[test]
fn input_is_taken_while_an_unskippable_movie_plays_and_the_next_starts_in_the_same_frame() {
    let (mut app, _log) = app();
    play(&mut app, "30f", MovieTarget::Fullscreen, false);
    let second = play(&mut app, "30f", MovieTarget::Fullscreen, false);
    app.update();
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyM);
    app.update();
    assert!(!app.world().resource::<ButtonInput<KeyCode>>().just_pressed(KeyCode::KeyM));
    // Never a frame with no fullscreen movie between the two.
    let mut gap = false;
    run_until(&mut app, "both end", |a| {
        let m = a.world().resource::<Movies>();
        gap |= finished(a).len() == 1 && m.fullscreen.is_none();
        finished(a).len() == 2
    });
    assert!(!gap, "no frame between the movies");
    assert_eq!(finished(&app)[1].0, second);
}
