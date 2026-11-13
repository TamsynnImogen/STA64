//! Backend trait, buses, volumes and the real `kira` backend.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use kira::sound::{PlaybackState, SoundData};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Frame, Panning, Tween};

use crate::media::PcmStream;
use crate::sim_link::Bus;

/// Mixer buses: the four `PlaySound` buses plus the master they all route to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MixBus {
    Master,
    Music,
    Sfx,
    Voice,
    Ui,
}

impl MixBus {
    pub const ALL: [MixBus; 5] = [MixBus::Master, MixBus::Music, MixBus::Sfx, MixBus::Voice, MixBus::Ui];
}

impl From<Bus> for MixBus {
    fn from(b: Bus) -> Self {
        match b {
            Bus::Music => MixBus::Music,
            Bus::Sfx => MixBus::Sfx,
            Bus::Voice => MixBus::Voice,
            Bus::Ui => MixBus::Ui,
        }
    }
}

/// Linear volumes (0 = silent, 1 = full); defaults until a settings struct exists.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct Volumes {
    pub master: f32,
    pub music: f32,
    pub sfx: f32,
    pub voice: f32,
    pub ui: f32,
}

impl Default for Volumes {
    fn default() -> Self {
        Self { master: 1.0, music: 0.7, sfx: 1.0, voice: 1.0, ui: 1.0 }
    }
}

impl Volumes {
    pub fn get(&self, b: MixBus) -> f32 {
        match b {
            MixBus::Master => self.master,
            MixBus::Music => self.music,
            MixBus::Sfx => self.sfx,
            MixBus::Voice => self.voice,
            MixBus::Ui => self.ui,
        }
    }
}

/// Decoded stereo audio, cheap to clone.
#[derive(Debug, Clone)]
pub struct Sound {
    pub name: String,
    pub sample_rate: u32,
    pub frames: Arc<[[f32; 2]]>,
}

/// What the game needs from an audio device.
pub trait AudioBackend {
    fn name(&self) -> &'static str;
    /// Play `sound` on `bus`; `at` is a map-space position, `None` for non-positional.
    fn play(&mut self, sound: &Sound, bus: Bus, at: Option<[f32; 3]>);
    /// Whether a bus still has audible playback (including an in-progress stop).
    fn bus_busy(&self, bus: Bus) -> bool;
    /// Advance a headless backend's playback clock; real devices use their own clock.
    fn advance(&mut self, _seconds: f64) {}
    /// Start pulling `stream` (a movie's audio, MED-03) at the device's pace on `bus`;
    /// the backend calls `stream.mark_attached()` when it will. The default takes nothing,
    /// and the movie then runs on the wall clock, silent.
    fn play_stream(&mut self, _stream: &PcmStream, _bus: Bus) {}
    fn set_bus_volume(&mut self, bus: MixBus, volume: f32);
    fn stop_all(&mut self);
    /// Stop what is playing on one bus (music switches tracks without cutting sfx).
    fn stop_bus(&mut self, bus: Bus);
    /// Listener position and its right-hand direction.
    fn set_listener(&mut self, pos: [f32; 3], right: [f32; 3]);
}

/// The mixer: owns the backend (a non-send resource: audio devices are not `Send` everywhere).
pub struct Mixer {
    pub backend: Box<dyn AudioBackend>,
}

impl Mixer {
    pub fn new(backend: Box<dyn AudioBackend>) -> Self {
        Self { backend }
    }
}

/// Distance at which positional sounds are 20 dB down and fully panned.
const FALLOFF: f32 = 2000.0;

/// Real output through `kira`: one sub-track per bus, all routed to the main (master) track.
pub struct KiraAudio {
    manager: AudioManager<DefaultBackend>,
    tracks: HashMap<Bus, TrackHandle>,
    playing: Vec<(Bus, StaticSoundHandle)>,
    listener: ([f32; 3], [f32; 3]),
}

impl KiraAudio {
    /// Fails (rather than panics) when there is no sound device.
    pub fn new() -> Result<Self, String> {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()).map_err(|e| e.to_string())?;
        let mut tracks = HashMap::new();
        for bus in [Bus::Music, Bus::Sfx, Bus::Voice, Bus::Ui] {
            tracks.insert(bus, manager.add_sub_track(TrackBuilder::new()).map_err(|e| e.to_string())?);
        }
        Ok(Self { manager, tracks, playing: Vec::new(), listener: ([0.0; 3], [1.0, 0.0, 0.0]) })
    }
}

fn db(v: f32) -> Decibels {
    if v <= 0.001 { Decibels::SILENCE } else { Decibels(20.0 * v.log10()) }
}

impl AudioBackend for KiraAudio {
    fn name(&self) -> &'static str {
        "kira"
    }

    fn play(&mut self, sound: &Sound, bus: Bus, at: Option<[f32; 3]>) {
        let frames: Arc<[Frame]> = sound.frames.iter().map(|f| Frame::new(f[0], f[1])).collect();
        let mut settings = StaticSoundSettings::new();
        if let Some(p) = at {
            // Pan by the listener's right vector and fade with distance.
            let (lp, r) = self.listener;
            let d = [p[0] - lp[0], p[1] - lp[1], p[2] - lp[2]];
            let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let far = (dist / FALLOFF).min(1.0);
            let side = if dist > 1e-3 { (d[0] * r[0] + d[1] * r[1] + d[2] * r[2]) / dist } else { 0.0 };
            settings = settings.panning(Panning((side * far).clamp(-1.0, 1.0))).volume(Decibels(-20.0 * far));
        }
        let data = StaticSoundData { sample_rate: sound.sample_rate, frames, settings, slice: None };
        self.playing.retain(|(_, h)| h.state() != PlaybackState::Stopped);
        if let Some(t) = self.tracks.get_mut(&bus) {
            match t.play(data) {
                Ok(h) => self.playing.push((bus, h)),
                Err(e) => warn!("audio: playing {:?} failed: {e}", sound.name),
            }
        }
    }

    fn play_stream(&mut self, stream: &PcmStream, bus: Bus) {
        let Some(t) = self.tracks.get_mut(&bus) else { return };
        match t.play(StreamData(stream.clone())) {
            Ok(()) => stream.mark_attached(),
            Err(e) => warn!("audio: starting a movie's sound failed: {e}"),
        }
    }

    fn bus_busy(&self, bus: Bus) -> bool {
        self.playing.iter().any(|(b, handle)| *b == bus && handle.state() != PlaybackState::Stopped)
    }

    fn set_bus_volume(&mut self, bus: MixBus, volume: f32) {
        let v = db(volume);
        let t = Tween::default();
        match bus {
            MixBus::Master => self.manager.main_track().set_volume(v, t),
            MixBus::Music => self.tracks.get_mut(&Bus::Music).unwrap().set_volume(v, t),
            MixBus::Sfx => self.tracks.get_mut(&Bus::Sfx).unwrap().set_volume(v, t),
            MixBus::Voice => self.tracks.get_mut(&Bus::Voice).unwrap().set_volume(v, t),
            MixBus::Ui => self.tracks.get_mut(&Bus::Ui).unwrap().set_volume(v, t),
        }
    }

    fn stop_all(&mut self) {
        for (_, h) in &mut self.playing {
            h.stop(Tween::default());
        }
        // A stop can fade asynchronously. Retain voice handles until Kira
        // reports Stopped so a replacement cannot overlap the fading line.
        self.playing.retain(|(bus, h)| *bus == Bus::Voice && h.state() != PlaybackState::Stopped);
    }

    fn stop_bus(&mut self, bus: Bus) {
        self.playing.retain_mut(|(b, h)| {
            if *b == bus {
                h.stop(Tween::default());
            }
            *b != bus || (bus == Bus::Voice && h.state() != PlaybackState::Stopped)
        });
    }

    fn set_listener(&mut self, pos: [f32; 3], right: [f32; 3]) {
        self.listener = (pos, right);
    }
}

/// A movie's audio as a `kira` sound: pulls from the stream and resamples it
/// (linear) to the device rate. Silent while the stream is starved; finished
/// when it is drained or cancelled.
struct StreamData(PcmStream);

impl SoundData for StreamData {
    type Error = std::convert::Infallible;
    type Handle = ();

    fn into_sound(self) -> Result<(Box<dyn kira::sound::Sound>, ()), Self::Error> {
        let rate = self.0.sample_rate() as f64;
        Ok((Box::new(StreamSound { stream: self.0, rate, local: vec![[0.0; 2]; 512], len: 0, at: 0, a: [0.0; 2], b: [0.0; 2], frac: 1.0 }), ()))
    }
}

struct StreamSound {
    stream: PcmStream,
    rate: f64,
    local: Vec<[f32; 2]>,
    len: usize,
    at: usize,
    a: [f32; 2],
    b: [f32; 2],
    /// Position between `a` and `b`; `>= 1` means the next source frame is needed.
    frac: f64,
}

impl StreamSound {
    fn next_source(&mut self) -> Option<[f32; 2]> {
        if self.at >= self.len {
            self.len = self.stream.pull(&mut self.local);
            self.at = 0;
            if self.len == 0 {
                return None;
            }
        }
        let f = self.local[self.at];
        self.at += 1;
        Some(f)
    }
}

impl kira::sound::Sound for StreamSound {
    fn process(&mut self, out: &mut [Frame], dt: f64, _info: &kira::info::Info) {
        let step = dt * self.rate;
        for o in out {
            while self.frac >= 1.0 {
                match self.next_source() {
                    Some(f) => {
                        self.a = self.b;
                        self.b = f;
                        self.frac -= 1.0;
                    }
                    None => break,
                }
            }
            if self.frac >= 1.0 {
                *o = Frame::ZERO; // starved: hold the clock until data arrives
                continue;
            }
            let t = self.frac as f32;
            *o = Frame::new(self.a[0] + (self.b[0] - self.a[0]) * t, self.a[1] + (self.b[1] - self.a[1]) * t);
            self.frac += step;
        }
    }

    fn finished(&self) -> bool {
        self.stream.is_cancelled() || (self.stream.is_drained() && self.at >= self.len && self.frac >= 1.0)
    }
}
