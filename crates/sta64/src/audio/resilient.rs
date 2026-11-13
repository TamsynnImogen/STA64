//! `Resilient`: an output device that survives stream errors.
//!
//! Wraps whatever opens a device (an [`Opener`]) behind the [`AudioBackend`] trait.
//! Rules, all driven by the `advance` clock (the frame's real delta, so tests need no sleeping):
//! - errors the device reports are drained at most [`Policy::max_poll`] per frame and logged
//!   at most once per [`Policy::log_gap`] seconds, with a count (never a line per error);
//! - an error the stream does not survive (or [`Policy::benign_limit`] glitches) tears the
//!   device down at once (a stream stuck on `POLLERR` otherwise spins), and the output is
//!   reopened after 1 s, 2 s, 4 s ... up to [`Policy::backoff_cap`];
//! - after [`Policy::max_attempts`] failed opens it falls back to null audio with one log
//!   line and tries again every [`Policy::fallback_retry`] seconds;
//! - bus volumes, the listener and the current music track (from its start) are restored on
//!   every reopen; sound calls are accepted in every state and never wait for a device.
//!
//! The one thing that can take time is the opener itself (a device open, on the main
//! thread), and it runs at most once per backoff step.

use super::mixer::{AudioBackend, MixBus, Sound, StreamFault};
use super::null::{CallLog, NullAudio};
use crate::media::PcmStream;
use crate::sim_link::Bus;
use bevy::prelude::*;

/// Opens an output device; `Err` carries the reason.
pub type Opener = Box<dyn FnMut() -> Result<Box<dyn AudioBackend>, String>>;

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// Stream errors drained per `advance` call.
    pub max_poll: usize,
    /// At most one error line per this many seconds.
    pub log_gap: f64,
    /// Delay before the first reopen; doubles per failed attempt.
    pub backoff_first: f64,
    pub backoff_cap: f64,
    /// Failed opens in a row before falling back to null audio.
    pub max_attempts: u32,
    /// Seconds between opens while on null audio.
    pub fallback_retry: f64,
    /// Survivable glitches since the last open that still force a reopen.
    pub benign_limit: u64,
    /// Seconds a reopened device must stay quiet before its failures stop counting toward
    /// `max_attempts` (a device that opens and fails again at once must not reopen forever).
    pub stable_after: f64,
}

impl Default for Policy {
    fn default() -> Self {
        Self { max_poll: 64, log_gap: 5.0, backoff_first: 1.0, backoff_cap: 8.0, max_attempts: 5, fallback_retry: 60.0, benign_limit: 200, stable_after: 30.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    /// The device is open.
    Live,
    /// Torn down; `attempt` failed opens so far, the next at clock time `at`.
    Retry { attempt: u32, at: f64 },
    /// Null audio after too many failures; the next open at `at`.
    Fallback { at: f64 },
}

/// Folds a stream of errors into rare log lines.
#[derive(Default)]
struct ErrorLog {
    pending: u64,
    last: String,
    last_at: Option<f64>,
}

impl ErrorLog {
    fn add(&mut self, n: u64, message: Option<&str>) {
        self.pending += n;
        if let Some(m) = message {
            self.last.clear();
            self.last.push_str(m);
        }
    }

    /// The line to print now, if one is due: the first error at once, then at most one per `gap`.
    fn due(&mut self, now: f64, gap: f64) -> Option<String> {
        if self.pending == 0 || self.last_at.is_some_and(|t| now - t < gap) {
            return None;
        }
        let line = match self.last_at {
            None if self.pending == 1 => format!("audio: output stream error: {}", self.last),
            None => format!("audio: {} output stream errors, first: {}", self.pending, self.last),
            Some(t) => format!("audio: {} more output stream errors in the last {:.0} s, latest: {}", self.pending, now - t, self.last),
        };
        self.pending = 0;
        self.last_at = Some(now);
        Some(line)
    }
}

pub struct Resilient {
    open: Opener,
    policy: Policy,
    inner: Box<dyn AudioBackend>,
    placeholder_log: Option<CallLog>,
    state: State,
    clock: f64,
    log: ErrorLog,
    glitches: u64,
    glitch_window_at: f64,
    /// Reopens since the device last stayed quiet for `stable_after`.
    streak: u32,
    live_since: f64,
    music_restored: bool,
    streams: Vec<(PcmStream, Bus)>,
    volumes: [Option<f32>; 5],
    listener: Option<([f32; 3], [f32; 3])>,
    music: Option<(Sound, f64)>,
    faults: Vec<StreamFault>,
    /// Lines this wrapper has logged (tests count them).
    pub lines_logged: u64,
    /// Successful reopens (after the first open).
    pub reopens: u64,
}

fn slot(b: MixBus) -> usize {
    MixBus::ALL.iter().position(|x| *x == b).unwrap_or(0)
}

impl Resilient {
    /// Opens a device now. If that fails it starts on null audio (one log line) and retries
    /// after [`Policy::fallback_retry`].
    pub fn new(mut open: Opener, policy: Policy) -> Self {
        let (inner, placeholder_log, state) = match open() {
            Ok(d) => (d, None, State::Live),
            Err(e) => {
                warn!("audio: no output device ({e}); sound is off, retrying every {:.0} s", policy.fallback_retry);
                let (n, log) = NullAudio::new();
                (Box::new(n) as Box<dyn AudioBackend>, Some(log), State::Fallback { at: policy.fallback_retry })
            }
        };
        let lines_logged = u64::from(placeholder_log.is_some());
        Self {
            open,
            policy,
            inner,
            placeholder_log,
            state,
            clock: 0.0,
            log: ErrorLog::default(),
            glitches: 0,
            glitch_window_at: 0.0,
            streak: 0,
            live_since: 0.0,
            music_restored: false,
            streams: Vec::new(),
            volumes: [None; 5],
            listener: None,
            music: None,
            faults: Vec::new(),
            lines_logged,
            reopens: 0,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    fn say(&mut self, line: String, error: bool) {
        self.lines_logged += 1;
        if error {
            error!("{line}");
        } else {
            warn!("{line}");
        }
    }

    fn backoff(&self, attempt: u32) -> f64 {
        (self.policy.backoff_first * 2f64.powi(attempt.min(30) as i32)).min(self.policy.backoff_cap)
    }

    /// Drop the device (stops a spinning stream) and keep accepting calls on null audio.
    fn tear_down(&mut self) {
        let (n, log) = NullAudio::new();
        self.inner = Box::new(n);
        self.placeholder_log = Some(log);
        // A movie's audio keeps its clock on null audio instead of freezing the decoder.
        self.streams.retain(|(s, _)| !s.is_cancelled() && !s.is_drained());
        for (s, bus) in &self.streams {
            self.inner.play_stream(s, *bus);
        }
    }

    fn restore(&mut self) {
        for b in MixBus::ALL {
            if let Some(v) = self.volumes[slot(b)] {
                self.inner.set_bus_volume(b, v);
            }
        }
        if let Some((p, r)) = self.listener {
            self.inner.set_listener(p, r);
        }
        if let Some((s, started)) = &self.music {
            if (self.clock - started) * (s.sample_rate as f64) < s.frames.len() as f64 {
                let s = s.clone();
                self.inner.play(&s, Bus::Music, None);
                self.music_restored = true;
            } else {
                self.music = None;
            }
        }
    }

    fn after_open(&mut self) {
        self.live_since = self.clock;
        self.restore();
        self.streams.retain(|(s, _)| !s.is_cancelled() && !s.is_drained());
        for (s, bus) in &self.streams {
            self.inner.play_stream(s, *bus);
        }
    }

    fn try_open(&mut self) -> Result<(), String> {
        let d = (self.open)()?;
        self.inner = d;
        self.placeholder_log = None;
        self.glitches = 0;
        self.state = State::Live;
        self.reopens += 1;
        self.after_open();
        Ok(())
    }

    fn step_reopen(&mut self) {
        match self.state {
            State::Live => {}
            State::Retry { attempt, at } if self.clock >= at => match self.try_open() {
                Ok(()) => self.say(format!("audio: output reopened (attempt {})", attempt + 1), false),
                Err(e) if attempt + 1 >= self.policy.max_attempts => {
                    self.state = State::Fallback { at: self.clock + self.policy.fallback_retry };
                    self.say(format!("audio: output did not reopen after {} attempts ({e}); sound is off, retrying every {:.0} s", attempt + 1, self.policy.fallback_retry), true);
                }
                Err(e) => {
                    let wait = self.backoff(attempt + 1);
                    self.state = State::Retry { attempt: attempt + 1, at: self.clock + wait };
                    self.say(format!("audio: reopening the output failed ({e}); next try in {wait:.1} s"), false);
                }
            },
            State::Fallback { at } if self.clock >= at => match self.try_open() {
                Ok(()) => self.say("audio: output is back".to_owned(), false),
                Err(_) => self.state = State::Fallback { at: self.clock + self.policy.fallback_retry },
            },
            _ => {}
        }
    }

    fn poll(&mut self) {
        if self.state != State::Live {
            return;
        }
        if self.streak > 0 && self.clock - self.live_since >= self.policy.stable_after {
            self.streak = 0;
        }
        if self.clock - self.glitch_window_at >= 60.0 {
            self.glitch_window_at = self.clock;
            self.glitches = 0; // slow xruns are normal; only a burst forces a reopen
        }
        self.faults.clear();
        let dropped = self.inner.poll_errors(self.policy.max_poll, &mut self.faults);
        if self.faults.is_empty() && dropped == 0 {
            return;
        }
        let mut fatal = false;
        for f in std::mem::take(&mut self.faults) {
            self.log.add(1, Some(&f.message));
            if f.benign {
                self.glitches += 1;
            } else {
                fatal = true;
            }
        }
        self.log.add(dropped, None);
        self.glitches += dropped;
        if fatal || self.glitches >= self.policy.benign_limit {
            self.tear_down();
            let attempt = self.streak;
            self.streak += 1;
            if attempt >= self.policy.max_attempts {
                self.state = State::Fallback { at: self.clock + self.policy.fallback_retry };
                self.say(format!("audio: the output failed again {attempt} times in a row; sound is off, retrying every {:.0} s", self.policy.fallback_retry), true);
            } else {
                self.state = State::Retry { attempt, at: self.clock + self.backoff(attempt) };
            }
        }
    }
}

impl AudioBackend for Resilient {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn play(&mut self, sound: &Sound, bus: Bus, at: Option<[f32; 3]>) {
        if bus == Bus::Music {
            self.music = Some((sound.clone(), self.clock));
            if std::mem::take(&mut self.music_restored) {
                self.inner.stop_bus(Bus::Music); // the restored track must not overlap the next
            }
        }
        self.inner.play(sound, bus, at);
    }

    fn bus_busy(&self, bus: Bus) -> bool {
        self.inner.bus_busy(bus)
    }

    fn advance(&mut self, seconds: f64) {
        if seconds.is_finite() && seconds > 0.0 {
            self.clock += seconds;
        }
        self.inner.advance(seconds);
        if let Some(log) = &self.placeholder_log {
            log.drain(); // null audio records every call; nobody reads them here
        }
        self.poll();
        if let Some(line) = self.log.due(self.clock, self.policy.log_gap) {
            self.say(line, true);
        }
        self.step_reopen();
    }

    fn play_stream(&mut self, stream: &PcmStream, bus: Bus) {
        self.inner.play_stream(stream, bus);
        self.streams.push((stream.clone(), bus));
    }

    fn set_bus_volume(&mut self, bus: MixBus, volume: f32) {
        self.volumes[slot(bus)] = Some(volume);
        self.inner.set_bus_volume(bus, volume);
    }

    fn stop_all(&mut self) {
        self.music = None;
        self.music_restored = false;
        self.inner.stop_all();
    }

    fn stop_bus(&mut self, bus: Bus) {
        if bus == Bus::Music {
            self.music = None;
            self.music_restored = false;
        }
        self.inner.stop_bus(bus);
    }

    fn set_listener(&mut self, pos: [f32; 3], right: [f32; 3]) {
        self.listener = Some((pos, right));
        self.inner.set_listener(pos, right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::sync::Arc;

    /// What the fake devices share with the test.
    #[derive(Default)]
    struct World {
        /// Errors the current device will report.
        faults: VecDeque<StreamFault>,
        /// Outcome of each open, front first; empty means success.
        opens: VecDeque<Result<(), String>>,
        opened: u32,
        dropped_devices: u32,
        polls: Vec<usize>,
        volumes: Vec<(u32, MixBus, f32)>,
        plays: Vec<(u32, String, Bus)>,
    }

    struct Fake {
        id: u32,
        w: Rc<RefCell<World>>,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.w.borrow_mut().dropped_devices += 1;
        }
    }

    impl AudioBackend for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn play(&mut self, s: &Sound, bus: Bus, _: Option<[f32; 3]>) {
            self.w.borrow_mut().plays.push((self.id, s.name.clone(), bus));
        }
        fn bus_busy(&self, _: Bus) -> bool {
            false
        }
        fn set_bus_volume(&mut self, bus: MixBus, v: f32) {
            self.w.borrow_mut().volumes.push((self.id, bus, v));
        }
        fn stop_all(&mut self) {}
        fn stop_bus(&mut self, _: Bus) {}
        fn set_listener(&mut self, _: [f32; 3], _: [f32; 3]) {}
        fn poll_errors(&mut self, max: usize, out: &mut Vec<StreamFault>) -> u64 {
            let mut w = self.w.borrow_mut();
            let n = max.min(w.faults.len());
            w.polls.push(n);
            out.extend(w.faults.drain(..n));
            0
        }
    }

    fn opener(w: &Rc<RefCell<World>>) -> Opener {
        let w = w.clone();
        Box::new(move || {
            let r = w.borrow_mut().opens.pop_front().unwrap_or(Ok(()));
            r?;
            let id = {
                let mut b = w.borrow_mut();
                b.opened += 1;
                b.faults.clear(); // errors belong to the device that raised them
                b.opened
            };
            Ok(Box::new(Fake { id, w: w.clone() }) as Box<dyn AudioBackend>)
        })
    }

    fn fault(msg: &str, benign: bool) -> StreamFault {
        StreamFault { message: msg.to_owned(), benign }
    }

    fn snd(name: &str, secs: usize) -> Sound {
        Sound { name: name.to_owned(), sample_rate: 10, frames: Arc::from(vec![[0.0f32; 2]; secs * 10]) }
    }

    fn run(r: &mut Resilient, seconds: f64, step: f64) {
        let mut t = 0.0;
        while t < seconds {
            r.advance(step);
            t += step;
        }
    }

    fn make(w: &Rc<RefCell<World>>) -> Resilient {
        Resilient::new(opener(w), Policy::default())
    }

    #[test]
    fn one_error_one_line_and_a_reopen() {
        let w = Rc::new(RefCell::new(World::default()));
        let mut r = make(&w);
        assert_eq!(r.state(), State::Live);
        w.borrow_mut().faults.push_back(fault("alsa::poll() returned POLLERR", false));
        r.advance(0.016);
        assert_eq!(r.lines_logged, 1);
        assert!(matches!(r.state(), State::Retry { attempt: 0, .. }));
        assert_eq!(w.borrow().dropped_devices, 1, "the failing stream is torn down at once");
        run(&mut r, 1.5, 0.016);
        assert_eq!(r.state(), State::Live);
        assert_eq!(r.reopens, 1);
        assert_eq!(w.borrow().opened, 2);
    }

    #[test]
    fn a_burst_of_errors_is_bounded() {
        let w = Rc::new(RefCell::new(World::default()));
        let mut r = make(&w);
        // Glitches the stream survives, then a steady flood for a minute of frames.
        r.policy = Policy { benign_limit: u64::MAX, ..Policy::default() };
        w.borrow_mut().faults.extend((0..10_000).map(|_| fault("xrun", true)));
        let before = r.lines_logged;
        run(&mut r, 60.0, 0.016);
        assert_eq!(r.state(), State::Live, "glitches alone do not reopen below the limit");
        // 60 s at one line per 5 s, at most.
        assert!(r.lines_logged - before <= 13, "{} lines", r.lines_logged - before);
        let w = w.borrow();
        assert!(w.polls.iter().all(|n| *n <= 64), "per-frame work is capped");
        assert!(w.faults.is_empty());
    }

    #[test]
    fn a_flood_of_fatal_errors_reopens_once() {
        let w = Rc::new(RefCell::new(World::default()));
        let mut r = make(&w);
        w.borrow_mut().faults.extend((0..10_000).map(|_| fault("POLLERR", false)));
        run(&mut r, 3.0, 0.016);
        // The device that flooded is gone after its first poll; the new one is quiet.
        assert_eq!(r.reopens, 1);
        assert!(r.lines_logged <= 3, "{} lines", r.lines_logged);
        assert!(w.borrow().polls.iter().all(|n| *n <= 64));
    }

    #[test]
    fn repeated_failures_fall_back_to_null_with_one_line() {
        let w = Rc::new(RefCell::new(World::default()));
        let mut r = make(&w);
        w.borrow_mut().opens.extend((0..5).map(|_| Err("no device".to_owned())));
        w.borrow_mut().faults.push_back(fault("device lost", false));
        r.advance(0.016);
        run(&mut r, 40.0, 0.1);
        assert!(matches!(r.state(), State::Fallback { .. }), "{:?}", r.state());
        assert_eq!(r.name(), "null");
        // 1 error line + 4 "failed, next try" + 1 fallback line.
        assert_eq!(r.lines_logged, 6);
        // Sound calls are still accepted, and nothing piles up.
        for _ in 0..1000 {
            r.play(&snd("sfx", 1), Bus::Sfx, None);
            r.advance(0.016);
        }
        r.set_bus_volume(MixBus::Sfx, 0.5);
        assert_eq!(r.lines_logged, 6);
        // The next retry (device is back) recovers.
        run(&mut r, 70.0, 0.5);
        assert_eq!(r.state(), State::Live);
        assert_eq!(r.name(), "fake");
    }

    #[test]
    fn no_device_at_start_is_one_line_then_a_quiet_retry() {
        let w = Rc::new(RefCell::new(World::default()));
        w.borrow_mut().opens.push_back(Err("none".to_owned()));
        let mut r = make(&w);
        assert_eq!(r.lines_logged, 1);
        r.play(&snd("a", 1), Bus::Ui, None);
        run(&mut r, 59.0, 0.5);
        assert_eq!(r.name(), "null");
        run(&mut r, 2.0, 0.5);
        assert_eq!(r.state(), State::Live);
        assert_eq!(r.lines_logged, 2, "the retry says it is back, once");
    }

    #[test]
    fn recovery_restores_volumes_listener_and_music() {
        let w = Rc::new(RefCell::new(World::default()));
        let mut r = make(&w);
        r.set_bus_volume(MixBus::Master, 0.4);
        r.set_bus_volume(MixBus::Music, 0.2);
        r.set_listener([1.0; 3], [1.0, 0.0, 0.0]);
        r.play(&snd("theme", 30), Bus::Music, None);
        r.play(&snd("zap", 1), Bus::Sfx, None);
        w.borrow_mut().faults.push_back(fault("lost", false));
        run(&mut r, 1.5, 0.1);
        assert_eq!(r.state(), State::Live);
        let w = w.borrow();
        let on_new: Vec<_> = w.volumes.iter().filter(|v| v.0 == 2).collect();
        assert!(on_new.iter().any(|v| v.1 == MixBus::Master && v.2 == 0.4));
        assert!(on_new.iter().any(|v| v.1 == MixBus::Music && v.2 == 0.2));
        // The music restarts; the short effect is not replayed.
        assert!(w.plays.iter().any(|p| p.0 == 2 && p.1 == "theme"));
        assert!(!w.plays.iter().any(|p| p.0 == 2 && p.1 == "zap"));
    }

    #[test]
    fn finished_music_is_not_replayed() {
        let w = Rc::new(RefCell::new(World::default()));
        let mut r = make(&w);
        r.play(&snd("theme", 1), Bus::Music, None);
        run(&mut r, 3.0, 0.1);
        w.borrow_mut().faults.push_back(fault("lost", false));
        run(&mut r, 2.0, 0.1);
        assert_eq!(r.state(), State::Live);
        assert!(!w.borrow().plays.iter().any(|p| p.0 == 2));
    }

    #[test]
    fn a_device_that_fails_right_after_opening_ends_on_null_audio() {
        let w = Rc::new(RefCell::new(World::default()));
        let mut r = make(&w);
        // Every device opens fine and faults on its first poll.
        for _ in 0..100 {
            w.borrow_mut().faults.push_back(fault("POLLERR", false));
            r.advance(0.5);
        }
        assert!(matches!(r.state(), State::Fallback { .. }), "{:?}", r.state());
        assert_eq!(r.name(), "null");
        assert!(r.reopens <= u64::from(Policy::default().max_attempts), "{} reopens", r.reopens);
        assert!(r.lines_logged <= 25, "{} lines", r.lines_logged);
    }

    #[test]
    fn a_device_quiet_for_a_while_gets_its_attempts_back() {
        let w = Rc::new(RefCell::new(World::default()));
        let mut r = make(&w);
        w.borrow_mut().faults.push_back(fault("x", false));
        run(&mut r, 3.0, 0.1);
        assert_eq!(r.streak, 1);
        run(&mut r, 40.0, 0.5);
        assert_eq!(r.streak, 0);
    }

    #[test]
    fn a_second_track_after_a_restore_stops_the_restored_one() {
        let w = Rc::new(RefCell::new(World::default()));
        let mut r = make(&w);
        r.play(&snd("a", 60), Bus::Music, None);
        w.borrow_mut().faults.push_back(fault("x", false));
        run(&mut r, 3.0, 0.1);
        assert!(r.music_restored);
        r.play(&snd("b", 60), Bus::Music, None);
        assert!(!r.music_restored);
    }

    #[test]
    fn error_log_folds_into_counted_summaries() {
        let mut l = ErrorLog::default();
        l.add(1, Some("x"));
        assert!(l.due(0.0, 5.0).unwrap().contains("x"));
        l.add(500, Some("y"));
        assert!(l.due(1.0, 5.0).is_none());
        let s = l.due(5.0, 5.0).unwrap();
        assert!(s.contains("500") && s.contains("y"), "{s}");
        assert!(l.due(11.0, 5.0).is_none(), "nothing pending, nothing printed");
    }
}
