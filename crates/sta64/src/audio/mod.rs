//! Audio: buses, volumes, `PlaySound` playback (AUD-01, docs/WORKPLAN.md S8/D5).
//!
//! `kira` is used directly. [`AudioPlugin`] drains [`PlaySound`](crate::sim_link::PlaySound)
//! in `GameSet::Audio` and hands the sound to the [`Mixer`]'s backend: [`KiraAudio`]
//! normally, [`NullAudio`] (records calls) when headless or when no sound device
//! opens. Name to sound resolution is `sta_assets::audio` (CON-10): [`BankResolver`]
//! wraps it and `plugins.rs` installs it when there is an install to read. Without
//! one the [`StubResolver`] stays and every sound resolves to nothing (one log
//! line, AUD-06). Generated tones named `audio-test:<hz>` are heard only under
//! `--audio-test` ([`AudioTestTones`]).

// The API is for AUD-02..05 and tests; the binary does not use all of it yet.
#![allow(dead_code, unused_imports)]

mod mixer;
mod music;
mod null;
mod resilient;
mod sfx;
mod ui;
mod voices;

use std::collections::HashSet;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::time::Real;

pub use mixer::{AudioBackend, KiraAudio, MixBus, Mixer, Sound, StreamFault, Volumes};
pub use resilient::{Policy, Resilient};
pub use music::{Context as MusicContext, MusicConfig, MusicPlugin};
pub use null::{Call, CallLog, NullAudio};
pub use sfx::{SfxConfig, SfxNames, SfxPlugin, VoiceLimiter};
pub use ui::{UiContext, UiSound, UiSoundPlugin, UiSounds};
pub use voices::{VoiceConfig, VoiceNames, VoicePlugin};

use crate::app::{GameSet, Mode};
use crate::sim_link::{Bus, PlaySound, StopSounds};

/// Turns a bare sound name into decoded audio.
pub trait SoundResolver: Send + Sync {
    fn resolve(&mut self, name: &str) -> Option<Sound>;

    /// The race whose lines event names pick (`federation`, `klingon`, ...); `None` takes
    /// any. [`VoicePlugin`] sets it from the local player's units. Resolvers that do not
    /// know races ignore it.
    fn set_race(&mut self, _race: Option<&str>) {}

    /// The race last given to [`SoundResolver::set_race`].
    fn race(&self) -> Option<String> {
        None
    }
}

/// Resolves nothing and says so once: without an install there is no sound data (AUD-06).
#[derive(Default)]
pub struct StubResolver {
    warned: bool,
}

impl SoundResolver for StubResolver {
    fn resolve(&mut self, name: &str) -> Option<Sound> {
        if !std::mem::replace(&mut self.warned, true) {
            info!("audio: no game install, so sound is silent (first request: {name:?})");
        }
        None
    }
}

/// Whether generated `audio-test:<hz>` tones are audible: only for the manual
/// `sta64 --audio-test` check. The plugin sets it from the command line unless a
/// test inserted it first.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioTestTones(pub bool);

#[derive(Resource)]
pub struct Resolver(pub Box<dyn SoundResolver>);

/// The real resolver (CON-10): file names and event names through the install's
/// sound tables, decoded once and cached. Owns its own mount of the install
/// (the `Vfs` a viewer holds is a resource the resolver trait cannot reach).
pub struct BankResolver {
    vfs: sta_vfs::Vfs,
    bank: sta_assets::audio::SoundBank,
    /// Race tag for event names (`federation`, `klingon`, ...); `None` takes any.
    pub race: Option<String>,
    /// Rotates through an event's sounds, one step per resolve.
    variant: usize,
    warned: HashSet<String>,
}

impl BankResolver {
    pub fn new(vfs: sta_vfs::Vfs) -> Self {
        let bank = sta_assets::audio::SoundBank::load(&vfs);
        Self { vfs, bank, race: None, variant: 0, warned: HashSet::new() }
    }

    /// The UI sound names this install's `RTS_CFG.h` gives, limited to files it has (AUD-05/06).
    pub fn ui_sounds(&self) -> UiSounds {
        let asked = sta_content::layered::cfg_h(&self.vfs, "RTS_CFG.h").map_or_else(|| UiSounds::from_cfg(&sta_assets::cfg_h::CfgH::parse("")), |c| UiSounds::from_cfg(&c));
        asked.existing(|n| ui::sound_exists(&self.vfs, n))
    }

    /// Mount `root` (+ `active_mod`) as the game does; `None` and a warning when it fails.
    pub fn mount(root: &std::path::Path, active_mod: Option<&str>) -> Option<Self> {
        match sta_vfs::Vfs::mount_install(root, active_mod) {
            Ok(vfs) => Some(Self::new(vfs)),
            Err(e) => {
                warn!("audio: cannot mount {} for sound ({e}); sound is off", root.display());
                None
            }
        }
    }
}

impl SoundResolver for BankResolver {
    fn set_race(&mut self, race: Option<&str>) {
        self.race = race.map(str::to_owned);
    }

    fn race(&self) -> Option<String> {
        self.race.clone()
    }

    fn resolve(&mut self, name: &str) -> Option<Sound> {
        self.variant = self.variant.wrapping_add(1);
        match self.bank.audio(&self.vfs, name, self.race.as_deref(), self.variant) {
            Some(a) => Some(Sound { name: name.to_owned(), sample_rate: a.sample_rate, frames: a.frames.clone() }),
            None => {
                if self.warned.insert(name.to_owned()) {
                    match self.bank.failures.last() {
                        Some((path, why)) if self.bank.resolve(&self.vfs, name, self.race.as_deref(), 0).is_some() => {
                            warn!("audio: {name:?} ({path}) does not decode: {why}")
                        }
                        _ => info!("audio: no sound named {name:?}"),
                    }
                }
                None
            }
        }
    }
}

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
    // Opens now; a missing device or a later stream error leaves null audio and retries.
    Box::new(Resilient::new(Box::new(|| KiraAudio::new().map(|k| Box::new(k) as Box<dyn AudioBackend>)), Policy::default()))
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
        let tones = std::env::args().any(|a| a == "--audio-test");
        app.world_mut().get_resource_or_insert_with(|| AudioTestTones(tones));
        app.add_plugins((MusicPlugin, UiSoundPlugin, SfxPlugin, VoicePlugin))
            .add_message::<PlaySound>()
            .add_message::<StopSounds>()
            .init_resource::<Volumes>()
            .insert_resource(Resolver(Box::new(StubResolver::default())))
            .add_message::<crate::frontend::screens::ModChosen>()
            .add_systems(Update, remount_sound.in_set(GameSet::Audio).before(play_sounds))
            .add_systems(Update, (sync_volumes, sync_listener, play_sounds, stop_sounds, audio_test_key).chain().in_set(GameSet::Audio));
        // Manual check: `sta64 --audio-test` plays a tone on each bus at startup, F9 repeats it.
        if tones {
            app.add_systems(Startup, test_tones);
        }
    }
}

/// The front end's "use mod" remounts the install (`game::flow`); the sound tables follow. The
/// resolver owns its own mount, so without this it keeps the events and files of the mount the
/// game started with and every event name the new mod's ODFs give misses (the Classic mod's
/// units went silent on a Roots start). The race the voices set is kept.
fn remount_sound(mut chosen: MessageReader<crate::frontend::screens::ModChosen>, flow: Option<Res<crate::game::flow::FlowConfig>>, mut resolver: ResMut<Resolver>, mut commands: Commands) {
    let Some(crate::frontend::screens::ModChosen(folder)) = chosen.read().last().cloned() else { return };
    let Some(root) = flow.and_then(|f| f.install_root.clone()) else { return };
    let Some(mut fresh) = BankResolver::mount(&root, folder.as_deref()) else { return };
    fresh.race = resolver.0.race();
    commands.insert_resource(fresh.ui_sounds());
    resolver.0 = Box::new(fresh);
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

fn sync_listener(cam: Query<&GlobalTransform, crate::map::MainCamera>, mut mixer: NonSendMut<Mixer>) {
    if let Some(t) = cam.iter().next() {
        // Sounds are placed in map space; the renderer mirrors Z, so mirror the camera back.
        let (p, r) = (t.translation(), t.right());
        mixer.backend.set_listener([p.x, p.y, -p.z], [r.x, r.y, -r.z]);
    }
}

fn play_sounds(mut msgs: MessageReader<PlaySound>, mut mixer: NonSendMut<Mixer>, mut resolver: ResMut<Resolver>, tones: Res<AudioTestTones>, time: Option<Res<Time<Real>>>) {
    mixer.backend.advance(time.map_or(0.0, |t| t.delta_secs_f64()));
    for m in msgs.read() {
        // Admission belongs here so advisor, mission and unit lines share the
        // same voice bus. A rejected line is consumed, never queued or decoded.
        if m.bus == Bus::Voice && mixer.backend.bus_busy(Bus::Voice) {
            continue;
        }
        if let Some(sound) = tones.0.then(|| builtin(&m.name)).flatten().or_else(|| resolver.0.resolve(&m.name)) {
            mixer.backend.play(&sound, m.bus, m.at.map(|p| p.to_f32()));
        }
    }
}

/// A mission's `Media:stopSounds()`: what plays on the bus stops (not queued, not decoded).
fn stop_sounds(mut msgs: MessageReader<StopSounds>, mut mixer: NonSendMut<Mixer>) {
    for m in msgs.read() {
        mixer.backend.stop_bus(m.bus);
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
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::Vec3 as SimVec3;
    use std::time::Duration;

    struct LongVoice;

    impl SoundResolver for LongVoice {
        fn resolve(&mut self, name: &str) -> Option<Sound> {
            (name != "missing").then(|| Sound {
                name: name.to_owned(),
                sample_rate: 10,
                frames: Arc::from(vec![[0.0, 0.0]; 20]),
            })
        }
    }

    fn voice_app() -> (App, CallLog) {
        let (null, log) = NullAudio::new();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(500)))
            .insert_non_send(Mixer::new(Box::new(null)))
            .add_plugins(AudioPlugin::default())
            .insert_resource(Resolver(Box::new(LongVoice)));
        app.update();
        log.drain();
        (app, log)
    }

    fn request(app: &mut App, name: &str, bus: Bus) {
        app.world_mut().write_message(PlaySound { name: name.into(), at: None, bus });
    }

    #[test]
    fn voice_bus_discards_overlaps_until_exact_completion() {
        let (mut app, log) = voice_app();
        request(&mut app, "first", Bus::Voice);
        request(&mut app, "overlap", Bus::Voice);
        request(&mut app, "ui", Bus::Ui);
        app.update();
        assert_eq!(log.drain(), vec![
            Call::Play { name: "first".into(), bus: Bus::Voice, at: None },
            Call::Play { name: "ui".into(), bus: Bus::Ui, at: None },
        ]);
        request(&mut app, "late_overlap", Bus::Voice);
        app.update(); // 0.5 of the 2.0-second clip has elapsed.
        assert!(log.drain().is_empty());
        app.update();
        app.update(); // 1.5 seconds elapsed; no queued line starts.
        assert!(log.drain().is_empty());
        request(&mut app, "after", Bus::Voice);
        app.update(); // Exactly 2.0 seconds elapsed.
        assert_eq!(log.drain(), vec![Call::Play { name: "after".into(), bus: Bus::Voice, at: None }]);
        app.update();
        assert!(log.drain().is_empty(), "discarded lines never replay");
    }

    #[test]
    fn missing_audio_and_explicit_stops_release_voice_admission() {
        let (mut app, log) = voice_app();
        request(&mut app, "missing", Bus::Voice);
        request(&mut app, "valid", Bus::Voice);
        app.update();
        assert_eq!(log.drain(), vec![Call::Play { name: "valid".into(), bus: Bus::Voice, at: None }]);
        app.world_mut().non_send_mut::<Mixer>().backend.stop_bus(Bus::Voice);
        log.drain();
        request(&mut app, "after_stop_bus", Bus::Voice);
        app.update();
        assert_eq!(log.drain(), vec![Call::Play { name: "after_stop_bus".into(), bus: Bus::Voice, at: None }]);
        app.world_mut().non_send_mut::<Mixer>().backend.stop_all();
        log.drain();
        request(&mut app, "after_stop_all", Bus::Voice);
        app.update();
        assert_eq!(log.drain(), vec![Call::Play { name: "after_stop_all".into(), bus: Bus::Voice, at: None }]);
    }

    fn app() -> (App, CallLog) {
        let (null, log) = NullAudio::new();
        let mut app = App::new();
        app.insert_non_send(Mixer::new(Box::new(null))).insert_resource(AudioTestTones(true));
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

    /// A mono 8 kHz 8-bit wav of `n` samples (authored here, not a game file).
    fn wav(n: usize) -> Vec<u8> {
        let mut w = format!("RIFF\0\0\0\0WAVEfmt ").into_bytes();
        w.extend([16, 0, 0, 0, 1, 0, 1, 0, 0x40, 0x1f, 0, 0, 0x40, 0x1f, 0, 0, 1, 0, 8, 0]);
        w.extend(b"data");
        w.extend((n as u32).to_le_bytes());
        w.extend(std::iter::repeat_n(200u8, n));
        w
    }

    /// The front end's mod switch (`ModChosen`) moves the resolver to the new mount and back:
    /// a Classic-style mod whose event names the base game lacks must not stay silent.
    #[test]
    fn a_mod_switch_remounts_the_sound_tables() {
        use crate::frontend::screens::ModChosen;
        let dir = std::env::temp_dir().join(format!("sta64-voices-remount-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = dir.join("Data");
        let modd = data.join("Mods/Plain Mod");
        for d in [data.join("sounds/voiceover/ingame"), modd.join("sounds/voiceover/ingame")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(data.join("sounds/voiceover/ingame/basehail.wav"), wav(3)).unwrap();
        std::fs::write(modd.join("sounds/voiceover/ingame/modhail.wav"), wav(5)).unwrap();
        std::fs::write(data.join("events.dat"), "BaseHail{\nTable\n\tSound <unitedfederation> \"basehail.wav\"\n}\n").unwrap();
        std::fs::write(modd.join("events.dat"), "ModHail{\nTable\n\tSound <federation> \"modhail.wav\"\n}\n").unwrap();
        std::fs::write(modd.join("info.ini"), "[Mod]\nstandalone = 1\n").unwrap();

        let (null, _log) = NullAudio::new();
        let mut app = App::new();
        app.insert_non_send(Mixer::new(Box::new(null)));
        app.add_plugins(AudioPlugin::default());
        app.insert_resource(crate::game::flow::FlowConfig { install_root: Some(dir.clone()) });
        let mut base = BankResolver::mount(&dir, None).unwrap();
        base.set_race(Some("federation"));
        app.insert_resource(Resolver(Box::new(base)));
        app.update();
        let frames = |app: &mut App, name: &str| app.world_mut().resource_mut::<Resolver>().0.resolve(name).map(|s| s.frames.len());
        assert_eq!((frames(&mut app, "BaseHail"), frames(&mut app, "ModHail")), (Some(3), None));

        app.world_mut().write_message(ModChosen(Some("Plain Mod".into())));
        app.update();
        assert_eq!((frames(&mut app, "BaseHail"), frames(&mut app, "ModHail")), (None, Some(5)), "the mod's tables after the switch");
        assert_eq!(app.world().resource::<Resolver>().0.race().as_deref(), Some("federation"), "the race the voices set survives");

        app.world_mut().write_message(ModChosen(None));
        app.update();
        assert_eq!((frames(&mut app, "BaseHail"), frames(&mut app, "ModHail")), (Some(3), None), "and back to the base game");
        std::fs::remove_dir_all(dir).ok();
    }

    /// Install-gated (`STA64_TEST_INSTALL`, a Roots install with the STA2 Classic mod; override the
    /// folder with `STA64_TEST_MOD`): base mount, switch to the mod, switch back. For every race
    /// of the Instant Action list, the select and acknowledge events of its units resolve through
    /// the live resolver as they do on a fresh mount of the same layers. Prints the table.
    #[test]
    fn real_install_voices_follow_the_front_end_mod_switch() {
        use crate::frontend::screens::ModChosen;
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let modname = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty()).unwrap_or_else(|| "STA2 Classic Mod".into());
        let root = std::path::PathBuf::from(root);
        let Ok(mod_vfs) = sta_vfs::Vfs::mount_install(&root, Some(&modname)) else {
            eprintln!("SKIP: mod {modname:?} does not mount under {}", root.display());
            return;
        };
        let (index, _) = sta_content::OdfIndex::build(&mod_vfs);
        let (db, _) = sta_content::ContentDb::load(&mod_vfs, &index).unwrap();
        let (races, _) = sta_content::Races::load(&mod_vfs, &index);
        let names = |d: &sta_content::UnitDef| d.sounds.select.iter().chain(&d.sounds.ack_move).chain(&d.sounds.ack_attack).cloned().collect::<Vec<_>>();
        // How many of a race's voice references play through `r`.
        let count = |r: &mut dyn SoundResolver, race: &str| {
            r.set_race(Some(race));
            let norm = |s: &str| s.to_ascii_lowercase().trim_start_matches("united").trim_end_matches("empire").to_owned();
            let all: Vec<String> = db.units().filter(|d| norm(&d.race) == norm(race)).flat_map(names).collect();
            (all.iter().filter(|n| r.resolve(n).is_some()).count(), all.len())
        };
        let fresh = |m: Option<&str>| BankResolver::mount(&root, m).expect("mount");
        let (null, _log) = NullAudio::new();
        let mut app = App::new();
        app.insert_non_send(Mixer::new(Box::new(null)));
        app.add_plugins(AudioPlugin::default());
        app.insert_resource(crate::game::flow::FlowConfig { install_root: Some(root.clone()) });
        app.insert_resource(Resolver(Box::new(fresh(None))));
        app.update();
        let race_list = sta_content::race::skirmish_races(&races, &db);
        assert!(!race_list.is_empty());
        let live = |app: &mut App, race: &str| count(&mut *app.world_mut().resource_mut::<Resolver>().0, race);
        let before: Vec<_> = race_list.iter().map(|r| live(&mut app, r)).collect();
        app.world_mut().write_message(ModChosen(Some(modname.clone())));
        app.update();
        let after: Vec<_> = race_list.iter().map(|r| live(&mut app, r)).collect();
        app.world_mut().write_message(ModChosen(None));
        app.update();
        let back: Vec<_> = race_list.iter().map(|r| live(&mut app, r)).collect();
        for (i, race) in race_list.iter().enumerate() {
            let expected = count(&mut fresh(Some(&modname)), race);
            eprintln!("| {modname} | {race} | on base mount: {}/{} | after the switch: {}/{} | fresh mount: {}/{} | switched back: {}/{} |", before[i].0, before[i].1, after[i].0, after[i].1, expected.0, expected.1, back[i].0, back[i].1);
            assert!(expected.0 > 0, "{race} has no voice that resolves on the {modname} mount");
            assert_eq!(after[i].0, expected.0, "{race}: after the switch the live resolver reads the mod's tables");
        }
        // Switching back restores the base game's tables.
        for (i, race) in race_list.iter().enumerate() {
            assert_eq!(back[i], count(&mut fresh(None), race), "{race}: after switching back");
        }
    }

    #[test]
    fn bank_resolver_finds_files_and_events_and_plays_them() {
        let dir = std::env::temp_dir().join(format!("sta64-con10-hook-{}", std::process::id()));
        let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x40\x1f\0\0\x01\0\x08\0data\x04\0\0\0".to_vec();
        wav.extend([128, 255, 0, 128]);
        std::fs::create_dir_all(dir.join("sounds/effects")).unwrap();
        std::fs::write(dir.join("sounds/effects/ping.wav"), &wav).unwrap();
        std::fs::write(dir.join("events.dat"), "Chirp{\nTable\n\tSound <borg> \"ping.wav\"\n}\n").unwrap();
        let mut vfs = sta_vfs::Vfs::default();
        vfs.push_dir("root".into(), dir.clone()).unwrap();

        let mut r = BankResolver::new(vfs);
        let s = r.resolve("PING.WAV").expect("file by name");
        assert_eq!((s.sample_rate, s.frames.len()), (8000, 4));
        assert_eq!(r.resolve("chirp").map(|s| s.frames.len()), Some(4), "event by name");
        assert!(r.resolve("nope.wav").is_none());

        let (null, log) = NullAudio::new();
        let mut app = App::new();
        app.insert_non_send(Mixer::new(Box::new(null)));
        app.add_plugins(AudioPlugin::default());
        app.insert_resource(Resolver(Box::new(r)));
        app.update();
        log.drain();
        app.world_mut().write_message(PlaySound { name: "ping.wav".into(), at: None, bus: Bus::Ui });
        app.update();
        assert_eq!(log.drain(), vec![Call::Play { name: "ping.wav".into(), bus: Bus::Ui, at: None }]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn without_an_install_or_audio_test_every_sound_is_silent() {
        let (null, log) = NullAudio::new();
        let mut app = App::new();
        app.insert_non_send(Mixer::new(Box::new(null)));
        app.add_plugins(AudioPlugin::default());
        app.update();
        log.drain();
        assert!(!app.world().resource::<AudioTestTones>().0, "tones are off without --audio-test");
        for bus in [Bus::Music, Bus::Sfx, Bus::Voice, Bus::Ui] {
            app.world_mut().write_message(PlaySound { name: "audio-test:440".into(), at: None, bus });
            app.world_mut().write_message(PlaySound { name: "click.wav".into(), at: None, bus });
        }
        app.update();
        assert!(log.calls().is_empty(), "nothing resolves, no tone plays");
    }

    #[test]
    fn the_stub_logs_once_and_resolves_nothing() {
        let mut r = StubResolver::default();
        assert!(r.resolve("a.wav").is_none() && r.resolve("b.wav").is_none());
        assert!(r.warned);
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

    // No test opens a real output device here: the open/recover logic is in `resilient`'s tests.
}
