//! A deterministic stand-in for the client, for headless runs of converted
//! missions (LUA-11). The converted scripts hold their next step while a fade,
//! cineractive, audio message, movie or wait-for-Enter is outstanding
//! (`A2.ScriptWaits`); the client reports those as finished (`A2.*Done`). With
//! no client, this host reports each one after its nominal length:
//!
//! - a voice or music message: the length of its WAV file in the VFS (data
//!   size over byte rate), searched by file name under `sounds/`; 3 s if the
//!   file is not found or not a PCM WAV;
//! - a movie: frames over frame rate from the Bink header of
//!   `animations/<name>`; 5 s if not found;
//! - a wait for Enter: 1 s (what the shell releases it on is not measured);
//! - a cineractive start: the next tick (no camera model in the host).
//!
//! Evidence grade: lengths are read from data (file headers); every default
//! above is a stand-in value and says nothing about the original's timing
//! (not measured). Fades need no report: the mission releases them itself.
//! The host reads only the runtime's own `A2.HostPending()` list.

use std::collections::BTreeMap;
use std::sync::Arc;

use sta_core::TICK_HZ;
use sta_vfs::Vfs;

use crate::campaign::ActiveMission;

/// Default message length when its file cannot be read, seconds.
pub const DEFAULT_SOUND_SECONDS: f64 = 3.0;
pub const DEFAULT_MOVIE_SECONDS: f64 = 5.0;
pub const ENTER_KEY_SECONDS: f64 = 1.0;

/// Seconds of PCM audio in a RIFF/WAVE file: `data` bytes over the byte rate.
pub fn wav_seconds(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let (mut rate, mut at) = (0u32, 12usize);
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        match &bytes[at..at + 4] {
            b"fmt " if size >= 12 && at + 8 + 12 <= bytes.len() => {
                rate = u32::from_le_bytes(bytes[at + 16..at + 20].try_into().ok()?);
            }
            b"data" if rate > 0 => return Some(size as f64 / f64::from(rate)),
            _ => {}
        }
        at = at.checked_add(8)?.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}

/// Seconds of a Bink video: frame count over frame rate (header fields).
pub fn bink_seconds(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 36 || &bytes[..3] != b"BIK" {
        return None;
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let (frames, dividend, divisor) = (word(8), word(28), word(32));
    (dividend > 0).then(|| f64::from(frames) * f64::from(divisor) / f64::from(dividend))
}

pub struct StandInHost {
    vfs: Arc<Vfs>,
    /// (kind, id) of what is outstanding -> tick it is reported.
    due: BTreeMap<(String, i64), u64>,
    /// Every report made, in order: (tick, kind, id).
    pub reports: Vec<(u64, String, i64)>,
}

impl StandInHost {
    pub fn new(vfs: Arc<Vfs>) -> StandInHost {
        StandInHost { vfs, due: BTreeMap::new(), reports: Vec::new() }
    }

    /// First file under `dir` named `name` (any case), else None.
    fn find(&self, dir: &str, name: &str) -> Option<Vec<u8>> {
        let want = name.replace('\\', "/").to_ascii_lowercase();
        let want = want.rsplit('/').next().unwrap_or("").to_owned();
        let path = self.vfs.list(dir).into_iter().find(|p| p.rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case(&want)))?;
        self.vfs.read(&path)?.ok()
    }

    fn seconds(&self, kind: &str, name: &str) -> f64 {
        match kind {
            "sound" | "music" => self.find("sounds", name).and_then(|b| wav_seconds(&b)).unwrap_or(DEFAULT_SOUND_SECONDS),
            "movie" => self.find("animations", name).and_then(|b| bink_seconds(&b)).unwrap_or(DEFAULT_MOVIE_SECONDS),
            "key" => ENTER_KEY_SECONDS,
            _ => 0.0,
        }
    }

    /// Call after each `ActiveMission::tick`: schedule what the mission just
    /// started and report what has run its nominal length.
    pub fn after_tick(&mut self, active: &mut ActiveMission) {
        let now = active.sim().tick;
        let pending: Vec<(String, i64, String, u64)> = active
            .runtime()
            .eval("local out = {} for i, p in ipairs(A2.HostPending and A2.HostPending() or {}) do out[i] = p.kind .. '\\t' .. p.id .. '\\t' .. p.name .. '\\t' .. p.tick end return out")
            .map(|l: Vec<String>| {
                l.into_iter()
                    .filter_map(|s| {
                        let mut f = s.splitn(4, '\t');
                        Some((f.next()?.to_owned(), f.next()?.parse().ok()?, f.next()?.to_owned(), f.next()?.parse().ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (kind, id, name, started) in pending {
            let due = started + (self.seconds(&kind, &name) * f64::from(TICK_HZ)).ceil() as u64 + 1;
            self.due.entry((kind, id)).or_insert(due);
        }
        let ready: Vec<(String, i64)> = self.due.iter().filter(|(_, d)| **d <= now).map(|(k, _)| k.clone()).collect();
        for (kind, id) in ready {
            self.due.remove(&(kind.clone(), id));
            let call = match kind.as_str() {
                "sound" | "music" => format!("A2.AudioMessageDone({id})"),
                "movie" => "A2.MovieDone()".to_owned(),
                "key" => format!("A2.EnterKeyDone({id})"),
                "cineractive" => "A2.CineractiveStartDone()".to_owned(),
                _ => continue,
            };
            let _ = active.runtime().eval::<()>(&call);
            self.reports.push((now, kind, id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, data: usize) -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        v.extend(16u32.to_le_bytes());
        v.extend([1, 0, 1, 0]);
        v.extend(rate.to_le_bytes());
        v.extend(rate.to_le_bytes());
        v.extend([1, 0, 8, 0]);
        v.extend(b"data");
        v.extend((data as u32).to_le_bytes());
        v.extend(vec![0x80; data]);
        v
    }

    #[test]
    fn wav_length_is_data_over_byte_rate() {
        assert_eq!(wav_seconds(&wav(22050, 44100)), Some(2.0));
        assert_eq!(wav_seconds(b"not a wave file at all"), None);
        assert_eq!(wav_seconds(&wav(0, 10)), None);
    }

    #[test]
    fn bink_length_is_frames_over_rate() {
        let mut b = vec![0u8; 40];
        b[..4].copy_from_slice(b"BIKi");
        b[8..12].copy_from_slice(&300u32.to_le_bytes());
        b[28..32].copy_from_slice(&30u32.to_le_bytes());
        b[32..36].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(bink_seconds(&b), Some(10.0));
        b[0] = b'X';
        assert_eq!(bink_seconds(&b), None);
    }
}
