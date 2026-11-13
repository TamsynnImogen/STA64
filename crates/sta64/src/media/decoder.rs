//! The decoder thread: one per playing movie (MED-03).
//!
//! The thread owns the [`Movie`], converts each video frame to RGBA and pushes
//! audio into the movie's [`PcmStream`]. The frame loop only receives finished
//! frames from a channel, so decoding never blocks it. The thread runs ahead of
//! the player's clock by a bounded amount (video seconds, queued frames and
//! queued audio), published through [`Shared`].

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use sta_media::{MediaEvent, Movie};

use super::pcm::PcmStream;

/// Video the thread decodes beyond the player's clock.
const VIDEO_AHEAD: Duration = Duration::from_millis(600);
/// Decoded frames waiting at most (a bound on memory whatever the clock does).
const MAX_QUEUED_FRAMES: usize = 90;
/// The thread stops decoding while this much audio is queued...
const AUDIO_FULL_SECS: f64 = 2.0;
/// ...and keeps decoding video ahead of the clock while less than this is queued,
/// so a late-interleaved audio track cannot stall the audio clock.
const AUDIO_LOW_SECS: f64 = 0.3;

/// One decoded picture.
pub struct Frame {
    pub pts: Duration,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub enum Msg {
    Frame(Frame),
    /// Every track has ended.
    End,
    /// Decoding stopped on an error (the message is logged and reported).
    Failed(String),
}

/// State shared between the player and the thread.
#[derive(Default)]
pub struct Shared {
    /// The player's clock in microseconds.
    pub clock_us: AtomicU64,
    pub cancel: AtomicBool,
    /// Frames sent and not yet received.
    pub queued: AtomicUsize,
}

/// A running decoder thread and the receiving end of its frames.
pub struct Decoder {
    rx: Mutex<Receiver<Msg>>,
    shared: Arc<Shared>,
    join: Option<JoinHandle<()>>,
    /// End or failure was delivered; nothing follows it.
    over: AtomicBool,
}

impl Decoder {
    /// Start decoding `movie`. `audio` is its audio queue (track 0, resampled by
    /// the mixer); `sidecar` is audio from a file beside a silent movie, queued
    /// whole.
    pub fn spawn(movie: Movie, audio: Option<PcmStream>, sidecar: Option<(u32, Arc<[[f32; 2]]>)>) -> Decoder {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared::default());
        let s = shared.clone();
        let (tx2, audio2) = (tx.clone(), audio.clone());
        let join = std::thread::Builder::new()
            .name("sta64-movie".into())
            .spawn(move || {
                // A panic past `Movie`'s own guard (frame conversion) must end the movie, not hang it.
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(movie, tx, &s, audio, sidecar))).is_err() {
                    if let Some(a) = &audio2 {
                        a.close();
                    }
                    let _ = tx2.send(Msg::Failed("decoder thread panicked".into()));
                }
            })
            .ok();
        Decoder { rx: Mutex::new(rx), shared, join, over: AtomicBool::new(false) }
    }

    /// The next message, if the thread has produced one.
    pub fn try_recv(&self) -> Option<Msg> {
        if self.over.load(Ordering::Acquire) {
            return None;
        }
        let rx = self.rx.lock().unwrap_or_else(|e| e.into_inner());
        match rx.try_recv() {
            Ok(m) => {
                if matches!(m, Msg::Frame(_)) {
                    self.shared.queued.fetch_sub(1, Ordering::AcqRel);
                } else {
                    self.over.store(true, Ordering::Release);
                }
                Some(m)
            }
            Err(TryRecvError::Empty) => None,
            // The thread is gone without a final message (it panicked or never started).
            Err(TryRecvError::Disconnected) => {
                if self.join.as_ref().is_none_or(|j| j.is_finished()) {
                    self.over.store(true, Ordering::Release);
                    Some(Msg::Failed("decoder thread ended unexpectedly".into()))
                } else {
                    None
                }
            }
        }
    }

    pub fn set_clock(&self, t: Duration) {
        self.shared.clock_us.store(t.as_micros() as u64, Ordering::Release);
    }

    /// Ask the thread to stop; it exits at its next step.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Release);
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.cancel();
        // The thread notices within one throttle sleep or one decoded event; joining
        // here would block the frame loop on a long decode step, so it is left to exit.
        drop(self.join.take());
    }
}

fn run(mut movie: Movie, tx: Sender<Msg>, shared: &Shared, audio: Option<PcmStream>, sidecar: Option<(u32, Arc<[[f32; 2]]>)>) {
    // A sidecar soundtrack is fed in as the sound plays, like decoded audio, so it neither
    // holds the throttle shut nor sits in the queue whole.
    let mut fed = 0usize;
    let mut feed = |stream: &PcmStream, whole: bool| {
        if let Some((rate, frames)) = &sidecar {
            let chunk = (*rate as usize / 4).max(1);
            while fed < frames.len() && (whole || stream.queued_secs() < 0.5) {
                let end = (fed + chunk).min(frames.len());
                stream.push(&frames[fed..end]);
                fed = end;
                if !whole {
                    break;
                }
            }
        }
    };
    let sidecar_on = sidecar.is_some();
    let mut last_pts = Duration::ZERO;
    let end = loop {
        // Wait while the player is far enough behind.
        loop {
            if shared.cancel.load(Ordering::Acquire) {
                break;
            }
            let clock = Duration::from_micros(shared.clock_us.load(Ordering::Acquire));
            let audio_secs = audio.as_ref().map(PcmStream::queued_secs);
            let audio_full = audio_secs.is_some_and(|s| s > AUDIO_FULL_SECS);
            let video_ahead = last_pts.saturating_sub(clock) > VIDEO_AHEAD && audio_secs.is_none_or(|s| s > AUDIO_LOW_SECS);
            if audio_full || video_ahead || shared.queued.load(Ordering::Acquire) >= MAX_QUEUED_FRAMES {
                std::thread::sleep(Duration::from_millis(4));
            } else {
                break;
            }
        }
        if shared.cancel.load(Ordering::Acquire) {
            break None;
        }
        if let Some(a) = &audio {
            feed(a, false);
        }
        match movie.next_event() {
            Ok(Some(MediaEvent::Video(f))) => {
                last_pts = f.pts;
                let frame = Frame { pts: f.pts, width: f.width(), height: f.height(), rgba: f.to_rgba() };
                shared.queued.fetch_add(1, Ordering::AcqRel);
                if tx.send(Msg::Frame(frame)).is_err() {
                    break None;
                }
            }
            Ok(Some(MediaEvent::Audio(c))) => {
                // Track 0 only; a sidecar file stands in for a movie without sound.
                if c.track == 0 && !sidecar_on && let Some(a) = &audio {
                    a.push_interleaved(&c.samples, c.channels);
                }
            }
            Ok(None) => break Some(Msg::End),
            Err(e) => break Some(Msg::Failed(e.to_string())),
        }
    };
    match (&audio, end.is_some()) {
        (Some(a), true) => {
            feed(a, true);
            a.close()
        }
        (Some(a), false) => a.cancel(),
        _ => {}
    }
    if let Some(m) = end {
        let _ = tx.send(m);
    }
}
