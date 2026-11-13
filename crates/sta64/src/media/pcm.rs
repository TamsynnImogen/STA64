//! A shared queue of decoded audio between a movie's decoder thread and the
//! mixer (MED-03).
//!
//! The decoder pushes interleaved PCM as it decodes; an audio backend that
//! supports streams (`AudioBackend::play_stream`) pulls frames at the
//! device's pace. The count of frames pulled is the movie's audio clock: the
//! player shows the video frame whose time has been reached by the audio, so
//! picture and sound cannot drift apart. A backend that does not take the
//! stream leaves [`PcmStream::is_attached`] false and the player falls back to
//! the wall clock.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cloneable handle on one movie's audio queue (stereo `f32` at the movie's rate).
#[derive(Clone)]
pub struct PcmStream(Arc<Inner>);

struct Inner {
    sample_rate: u32,
    buf: Mutex<VecDeque<[f32; 2]>>,
    queued: AtomicUsize,
    played: AtomicU64,
    closed: AtomicBool,
    cancelled: AtomicBool,
    attached: AtomicBool,
}

impl std::fmt::Debug for PcmStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PcmStream")
            .field("sample_rate", &self.0.sample_rate)
            .field("queued", &self.0.queued.load(Ordering::Relaxed))
            .field("played", &self.0.played.load(Ordering::Relaxed))
            .finish()
    }
}

impl PcmStream {
    pub fn new(sample_rate: u32) -> Self {
        Self(Arc::new(Inner {
            sample_rate: sample_rate.max(1),
            buf: Mutex::new(VecDeque::new()),
            queued: AtomicUsize::new(0),
            played: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            attached: AtomicBool::new(false),
        }))
    }

    pub fn sample_rate(&self) -> u32 {
        self.0.sample_rate
    }

    /// Queue stereo frames. Ignored once the stream is cancelled.
    pub fn push(&self, frames: &[[f32; 2]]) {
        if frames.is_empty() || self.is_cancelled() {
            return;
        }
        let mut buf = self.0.buf.lock().unwrap_or_else(|e| e.into_inner());
        buf.extend(frames.iter().copied());
        self.0.queued.store(buf.len(), Ordering::Release);
    }

    /// Queue interleaved samples of `channels` channels as stereo: mono is
    /// doubled, more than two channels keep the first two.
    pub fn push_interleaved(&self, samples: &[f32], channels: u16) {
        let ch = channels.max(1) as usize;
        let frames: Vec<[f32; 2]> = samples
            .chunks_exact(ch)
            .map(|f| if ch == 1 { [f[0], f[0]] } else { [f[0], f[1]] })
            .collect();
        self.push(&frames);
    }

    /// No more audio will be pushed; the stream ends once drained.
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Release);
    }

    /// Drop what is queued and tell the consumer to stop.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.0.closed.store(true, Ordering::Release);
        let mut buf = self.0.buf.lock().unwrap_or_else(|e| e.into_inner());
        buf.clear();
        self.0.queued.store(0, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    /// Move up to `out.len()` queued frames into `out`; returns how many. Each
    /// frame moved counts as played.
    pub fn pull(&self, out: &mut [[f32; 2]]) -> usize {
        let mut buf = self.0.buf.lock().unwrap_or_else(|e| e.into_inner());
        let n = out.len().min(buf.len());
        for (slot, f) in out.iter_mut().zip(buf.drain(..n)) {
            *slot = f;
        }
        self.0.queued.store(buf.len(), Ordering::Release);
        self.0.played.fetch_add(n as u64, Ordering::AcqRel);
        n
    }

    /// Frames waiting to be pulled.
    pub fn queued_frames(&self) -> usize {
        self.0.queued.load(Ordering::Acquire)
    }

    pub fn queued_secs(&self) -> f64 {
        self.queued_frames() as f64 / self.0.sample_rate as f64
    }

    /// How far the consumer has played: the audio clock.
    pub fn position(&self) -> Duration {
        let n = self.0.played.load(Ordering::Acquire) as u128;
        Duration::from_nanos((n * 1_000_000_000 / self.0.sample_rate as u128) as u64)
    }

    /// Closed (or cancelled) and everything pulled.
    pub fn is_drained(&self) -> bool {
        self.0.closed.load(Ordering::Acquire) && self.queued_frames() == 0
    }

    /// A backend took this stream and will pull from it.
    pub fn mark_attached(&self) {
        self.0.attached.store(true, Ordering::Release);
    }

    pub fn is_attached(&self) -> bool {
        self.0.attached.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_is_doubled_and_position_counts_pulled_frames() {
        let s = PcmStream::new(10);
        s.push_interleaved(&[0.5, -0.5, 0.25], 1);
        assert_eq!(s.queued_frames(), 3);
        let mut out = [[0.0; 2]; 2];
        assert_eq!(s.pull(&mut out), 2);
        assert_eq!(out, [[0.5, 0.5], [-0.5, -0.5]]);
        assert_eq!(s.position(), Duration::from_millis(200));
        assert!(!s.is_drained());
        s.close();
        assert_eq!(s.pull(&mut out), 1);
        assert!(s.is_drained());
    }

    #[test]
    fn surround_keeps_the_front_pair_and_cancel_empties() {
        let s = PcmStream::new(8000);
        s.push_interleaved(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3);
        let mut out = [[0.0; 2]; 4];
        assert_eq!(s.pull(&mut out), 2);
        assert_eq!(&out[..2], &[[1.0, 2.0], [4.0, 5.0]]);
        s.push(&[[0.0; 2]; 10]);
        s.cancel();
        assert!(s.is_cancelled() && s.is_drained());
        s.push(&[[0.0; 2]; 10]);
        assert_eq!(s.queued_frames(), 0);
    }
}
