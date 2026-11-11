//! `NullAudio`: records every request instead of making sound (headless runs,
//! CI, tests of everything that sends `PlaySound`).

use std::sync::{Arc, Mutex};

use super::mixer::{AudioBackend, MixBus, Sound};
use crate::sim_link::Bus;

/// One request the backend received.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Play { name: String, bus: Bus, at: Option<[f32; 3]> },
    BusVolume { bus: MixBus, volume: f32 },
    StopAll,
    StopBus { bus: Bus },
    Listener { pos: [f32; 3], right: [f32; 3] },
}

/// Shared view of the recorded calls; keep it after handing the backend to the mixer.
#[derive(Clone, Default)]
pub struct CallLog(Arc<Mutex<Vec<Call>>>);

impl CallLog {
    pub fn calls(&self) -> Vec<Call> {
        self.0.lock().unwrap().clone()
    }
    /// Take and clear the recorded calls.
    pub fn drain(&self) -> Vec<Call> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[derive(Default)]
pub struct NullAudio {
    log: CallLog,
    voice_remaining: f64,
}

impl NullAudio {
    pub fn new() -> (Self, CallLog) {
        let log = CallLog::default();
        (Self { log: log.clone(), voice_remaining: 0.0 }, log)
    }
    fn push(&self, c: Call) {
        self.log.0.lock().unwrap().push(c);
    }
}

impl AudioBackend for NullAudio {
    fn name(&self) -> &'static str {
        "null"
    }
    fn play(&mut self, sound: &Sound, bus: Bus, at: Option<[f32; 3]>) {
        if bus == Bus::Voice {
            self.voice_remaining = if sound.sample_rate == 0 { 0.0 } else { sound.frames.len() as f64 / sound.sample_rate as f64 };
        }
        self.push(Call::Play { name: sound.name.clone(), bus, at });
    }
    fn bus_busy(&self, bus: Bus) -> bool {
        bus == Bus::Voice && self.voice_remaining > 0.0
    }
    fn advance(&mut self, seconds: f64) {
        if seconds.is_finite() && seconds > 0.0 {
            self.voice_remaining = (self.voice_remaining - seconds).max(0.0);
        }
    }
    fn set_bus_volume(&mut self, bus: MixBus, volume: f32) {
        self.push(Call::BusVolume { bus, volume });
    }
    fn stop_all(&mut self) {
        self.voice_remaining = 0.0;
        self.push(Call::StopAll);
    }
    fn stop_bus(&mut self, bus: Bus) {
        if bus == Bus::Voice {
            self.voice_remaining = 0.0;
        }
        self.push(Call::StopBus { bus });
    }
    fn set_listener(&mut self, pos: [f32; 3], right: [f32; 3]) {
        self.push(Call::Listener { pos, right });
    }
}
