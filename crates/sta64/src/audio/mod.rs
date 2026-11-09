//! Audio: buses, volumes, `PlaySound` playback (AUD-01, docs/WORKPLAN.md S8/D5).
//!
//! `kira` is used directly. [`AudioPlugin`] drains [`PlaySound`](crate::sim_link::PlaySound)
//! in `GameSet::Audio` and hands the sound to the [`Mixer`]'s backend: [`KiraAudio`]
//! normally, [`NullAudio`] (records calls) when headless or when no sound device
//! opens. Name to sound resolution is CON-10's job; until then [`SoundResolver`]
//! is a stub, except for generated test tones named `audio-test:<hz>`.

// The API is for AUD-02..05 and tests; the binary does not use all of it yet.
#![allow(dead_code, unused_imports)]

mod mixer;
mod music;
mod null;

use std::collections::HashSet;
use std::sync::Arc;

use bevy::prelude::*;

pub use mixer::{AudioBackend, KiraAudio, MixBus, Mixer, Sound, Volumes};
pub use music::{Context as MusicContext, MusicConfig, MusicPlugin};
pub use null::{Call, CallLog, NullAudio};

use crate::app::{GameSet, Mode};
use crate::sim_link::{Bus, PlaySound};

/// Turns a bare sound name into decoded audio.
pub trait SoundResolver: Send + Sync {
    fn resolve(&mut self, name: &str) -> Option<Sound>;
}

/// Resolves nothing; logs each unknown name once.
#[derive(Default)]
pub struct StubResolver {
    warned: HashSet<String>,
}

impl SoundResolver for StubResolver {
    fn resolve(&mut self, name: &str) -> Option<Sound> {
        if self.warned.insert(name.to_string()) {
            info!("audio: no resolver for sound {name:?} yet");
        }
        None
    }
}

#[derive(Resource)]
pub struct Resolver(pub Box<dyn SoundResolver>);

/// A short sine tone with a fade in and out.
pub fn tone(hz: f32) -> Sound {
    let rate = 44_100u32;
    let n = (rate as f32 * 0.4) as usize;
    let frames = (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let env = (i as f32 / 800.0).min(1.0) * ((n - i) as f32 / 800.0).min(1.0);
            let s = (t * hz * std::f32::consts::TAU).sin() * 0.3 * env;
            [s, s]
        })
        .collect::<Vec<_>>();
    Sound { name: format!("audio-test:{hz}"), sample_rate: rate, frames: Arc::from(frames) }
}

fn builtin(name: &str) -> Option<Sound> {
    tone(name.strip_prefix("audio-test:")?.parse().ok()?).into()
}

/// The backend for a launch mode: Kira with a window, Null without or when the device fails.
pub fn choose_backend(mode: Mode) -> Box<dyn AudioBackend> {
    if !mode.has_window() {
        return Box::new(NullAudio::new().0);
    }
    match KiraAudio::new() {
        Ok(k) => Box::new(k),
        Err(e) => {
            warn!("audio: no output device ({e}); sound is off");
            Box::new(NullAudio::new().0)
        }
    }
}

/// Adds the mixer (unless one is already inserted), volumes and the playback systems.
pub struct AudioPlugin {
    pub mode: Mode,
}

impl Default for AudioPlugin {
    fn default() -> Self {
        Self { mode: Mode::Viewer }
    }
}

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        if app.world().get_non_send::<Mixer>().is_none() {
            app.insert_non_send(Mixer::new(choose_backend(self.mode)));
        }
        app.add_plugins(MusicPlugin)
            .add_message::<PlaySound>()
            .init_resource::<Volumes>()
            .insert_resource(Resolver(Box::new(StubResolver::default())))
            .add_systems(Update, (sync_volumes, sync_listener, play_sounds, audio_test_key).chain().in_set(GameSet::Audio));
        // Manual check: `sta64 --audio-test` plays a tone on each bus at startup, F9 repeats it.
        if std::env::args().any(|a| a == "--audio-test") {
            app.add_systems(Startup, test_tones);
        }
    }
}

fn sync_volumes(volumes: Res<Volumes>, mut mixer: NonSendMut<Mixer>, mut sent: Local<Option<Volumes>>) {
    if *sent == Some(*volumes) {
        return;
    }
    for b in MixBus::ALL {
        if sent.is_none_or(|s| s.get(b) != volumes.get(b)) {
            mixer.backend.set_bus_volume(b, volumes.get(b));
        }
    }
    *sent = Some(*volumes);
}

fn sync_listener(cam: Query<&GlobalTransform, With<Camera3d>>, mut mixer: NonSendMut<Mixer>) {
    if let Some(t) = cam.iter().next() {
        mixer.backend.set_listener(t.translation().to_array(), t.right().to_array());
    }
}

fn play_sounds(mut msgs: MessageReader<PlaySound>, mut mixer: NonSendMut<Mixer>, mut resolver: ResMut<Resolver>) {
    for m in msgs.read() {
        if let Some(sound) = builtin(&m.name).or_else(|| resolver.0.resolve(&m.name)) {
            mixer.backend.play(&sound, m.bus, m.at.map(|p| p.to_f32()));
        }
    }
}

fn tone_requests() -> impl Iterator<Item = PlaySound> {
    [(Bus::Music, 262), (Bus::Sfx, 330), (Bus::Voice, 392), (Bus::Ui, 523)]
        .into_iter()
        .map(|(bus, hz)| PlaySound { name: format!("audio-test:{hz}"), at: None, bus })
}

fn test_tones(mut out: MessageWriter<PlaySound>) {
    out.write_batch(tone_requests());
}

fn audio_test_key(keys: Option<Res<ButtonInput<KeyCode>>>, mut out: MessageWriter<PlaySound>) {
    if keys.is_some_and(|k| k.just_pressed(KeyCode::F9)) {
        out.write_batch(tone_requests());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::Vec3 as SimVec3;

    fn app() -> (App, CallLog) {
        let (null, log) = NullAudio::new();
        let mut app = App::new();
        app.insert_non_send(Mixer::new(Box::new(null)));
        app.add_plugins(AudioPlugin::default());
        app.update();
        log.drain();
        (app, log)
    }

    fn play(app: &mut App, bus: Bus, at: Option<SimVec3>) {
        app.world_mut().write_message(PlaySound { name: "audio-test:440".into(), at, bus });
        app.update();
    }

    #[test]
    fn each_bus_records_its_call_and_position() {
        let (mut app, log) = app();
        let pos = SimVec3::from_f32([10.0, 0.0, -5.0]);
        for (bus, at) in [(Bus::Music, None), (Bus::Sfx, Some(pos)), (Bus::Voice, None), (Bus::Ui, None)] {
            play(&mut app, bus, at);
            let calls = log.drain();
            assert_eq!(calls, vec![Call::Play { name: "audio-test:440".into(), bus, at: at.map(|p| p.to_f32()) }]);
        }
    }

    #[test]
    fn unknown_names_are_dropped() {
        let (mut app, log) = app();
        app.world_mut().write_message(PlaySound { name: "nope".into(), at: None, bus: Bus::Sfx });
        app.update();
        assert!(log.calls().is_empty());
    }

    #[test]
    fn volume_changes_reach_the_backend() {
        let (mut app, log) = app();
        app.world_mut().resource_mut::<Volumes>().music = 0.25;
        app.update();
        assert_eq!(log.drain(), vec![Call::BusVolume { bus: MixBus::Music, volume: 0.25 }]);
        app.update();
        assert!(log.calls().is_empty());
    }

    #[test]
    fn initial_volumes_are_sent_once() {
        let (null, log) = NullAudio::new();
        let mut app = App::new();
        app.insert_non_send(Mixer::new(Box::new(null)));
        app.add_plugins(AudioPlugin::default());
        app.update();
        assert_eq!(log.drain().len(), MixBus::ALL.len());
    }

    #[test]
    fn headless_picks_null() {
        assert_eq!(choose_backend(Mode::Bench { headless: true, sim_only: false }).name(), "null");
        assert_eq!(choose_backend(Mode::Bench { headless: false, sim_only: true }).name(), "null");
    }

    #[test]
    fn windowed_never_panics_without_a_device() {
        let name = choose_backend(Mode::Viewer).name();
        assert!(name == "kira" || name == "null");
    }
}
