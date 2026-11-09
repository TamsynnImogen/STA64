//! Sounds (CON-10): lookup by bare name, decoding, and the sound tables.
//!
//! The engine and its mods name sounds as `"phaser.wav"` or by event
//! (`"EnemyEngaged"`); files live under `sounds/effects`, `sounds/voiceover`,
//! `sounds/music` and (Armada I) `wav`. Fleet Operations ships many of them
//! as Ogg Vorbis and the Roots mods as MP3, so lookup works by stem:
//!
//! - a file in a higher-priority mod layer always wins;
//! - within one layer the newest format wins: OGG > WAV > MP3 (so `x.wav`
//!   asked for is served by `x.ogg` when both sit in one layer, as for textures).
//!
//! Decoding is pure Rust and permissively licensed: WAV is parsed here (8, 16,
//! 24 and 32 bit PCM and 32 bit float, tolerating the truncated `data` sizes
//! old tools wrote), Ogg Vorbis by `lewton` (MIT OR Apache-2.0) and MPEG audio
//! (MP3, with MP1/MP2) by `nanomp3` (MIT OR Apache-2.0). Everything comes out
//! as [`Audio`]: stereo `f32` frames at the file's own sample rate.
//!
//! Tables: [`Events`] reads `events.dat` (Armada II, Fleet Operations) and
//! `events.odf` (Armada I): named events with per-race sound lists. [`AudioParams`]
//! reads `audio.odf` (3D sound distances). [`SoundBank`] ties lookup, tables
//! and a decode cache together and is what the game's sound resolver wraps.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use sta_vfs::Vfs;

use crate::find_best;

/// An audio file format STA64 plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundFormat {
    Ogg,
    Wav,
    Mp3,
}

impl SoundFormat {
    /// Within one mod layer, best first.
    pub const PREFERENCE: [SoundFormat; 3] = [Self::Ogg, Self::Wav, Self::Mp3];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Ogg => "ogg",
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        Self::PREFERENCE.into_iter().find(|f| f.extension().eq_ignore_ascii_case(ext))
    }

    /// The format a byte stream starts like.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"OggS") {
            Some(Self::Ogg)
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            Some(Self::Wav)
        } else if bytes.starts_with(b"ID3") || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0) {
            Some(Self::Mp3)
        } else {
            None
        }
    }
}

/// Folders sounds live in, in lookup order when one stem is in several.
pub const SOUND_DIRS: [&str; 7] = [
    "sounds/effects",
    "sounds/voiceover/ingame",
    "sounds/voiceover/cineractive",
    "sounds/voiceover",
    "sounds/music",
    "sounds",
    "wav",
];

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("not a WAV, Ogg or MP3 file")]
    UnknownFormat,
    #[error("wav: {0}")]
    Wav(String),
    #[error("ogg: {0}")]
    Ogg(String),
    #[error("mp3: {0}")]
    Mp3(String),
    #[error("{0}")]
    Io(String),
}

/// Decoded audio: stereo frames at the source's sample rate. Mono sources
/// repeat their sample in both channels; more than two channels keep the
/// first two. Cheap to clone.
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub sample_rate: u32,
    pub frames: Arc<[[f32; 2]]>,
}

impl Audio {
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 { 0.0 } else { self.frames.len() as f32 / self.sample_rate as f32 }
    }
}

fn frames_from_interleaved(samples: impl Iterator<Item = f32>, channels: usize) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    let mut frame = [0.0f32; 2];
    let mut n = 0usize;
    for s in samples {
        if n < 2 {
            frame[n] = s;
        }
        n += 1;
        if n == channels {
            if channels == 1 {
                frame[1] = frame[0];
            }
            out.push(frame);
            n = 0;
        }
    }
    out
}

/// Decode a whole file, the format found from its first bytes.
pub fn decode(bytes: &[u8]) -> Result<Audio, AudioError> {
    match SoundFormat::sniff(bytes).ok_or(AudioError::UnknownFormat)? {
        SoundFormat::Wav => decode_wav(bytes),
        SoundFormat::Ogg => decode_ogg(bytes),
        SoundFormat::Mp3 => decode_mp3(bytes),
    }
}

fn le16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// RIFF/WAVE: PCM 8-32 bit and float, also in a `WAVE_FORMAT_EXTENSIBLE` header.
pub fn decode_wav(bytes: &[u8]) -> Result<Audio, AudioError> {
    let err = |s: &str| AudioError::Wav(s.to_owned());
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(err("not a RIFF/WAVE file"));
    }
    /// Format tag, channels, sample rate, bits per sample.
    type Fmt = (u16, u16, u32, u16);
    let (mut fmt, mut data): (Option<Fmt>, Option<&[u8]>) = (None, None);
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = le32(&bytes[pos + 4..]) as usize;
        let body = pos + 8;
        // A `data` size past the end of the file (streamed or cut-off files) means "the rest".
        let end = body.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " if end - body >= 16 => {
                let c = &bytes[body..end];
                let mut tag = le16(c);
                if tag == 0xFFFE && c.len() >= 26 {
                    tag = le16(&c[24..]);
                }
                fmt = Some((tag, le16(&c[2..]), le32(&c[4..]), le16(&c[14..])));
            }
            b"data" => {
                data = Some(&bytes[body..end]);
                if fmt.is_some() {
                    break;
                }
            }
            _ => {}
        }
        pos = end + (size & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or_else(|| err("no fmt chunk"))?;
    // A header with no `data` chunk is a valid silent file (stock `empty.wav`).
    let data = data.unwrap_or(&[]);
    if channels == 0 || rate == 0 {
        return Err(err("zero channels or sample rate"));
    }
    let ch = channels as usize;
    let samples: Box<dyn Iterator<Item = f32>> = match (tag, bits) {
        (1, 8) => Box::new(data.iter().map(|&b| (b as f32 - 128.0) / 128.0)),
        (1, 16) => Box::new(data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)),
        (1, 24) => Box::new(data.chunks_exact(3).map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0)),
        (1, 32) => Box::new(data.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0)),
        (3, 32) => Box::new(data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))),
        _ => return Err(AudioError::Wav(format!("unsupported format tag {tag}, {bits} bit"))),
    };
    Ok(Audio { sample_rate: rate, frames: frames_from_interleaved(samples, ch).into() })
}

/// Ogg Vorbis, via `lewton`.
pub fn decode_ogg(bytes: &[u8]) -> Result<Audio, AudioError> {
    let e = |e: lewton::VorbisError| AudioError::Ogg(format!("{e} ({e:?})"));
    let mut rd = lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes)).map_err(e)?;
    let channels = rd.ident_hdr.audio_channels as usize;
    let rate = rd.ident_hdr.audio_sample_rate;
    if channels == 0 || rate == 0 {
        return Err(AudioError::Ogg("zero channels or sample rate".into()));
    }
    let mut out = Vec::new();
    loop {
        match rd.read_dec_packet_itl() {
            Ok(Some(packet)) => out.extend(frames_from_interleaved(packet.iter().map(|&s| s as f32 / 32768.0), channels)),
            Ok(None) => break,
            // A damaged or cut-off audio packet (some Roots files end in one): skip it
            // as libvorbis does and keep what decodes. Header and I/O errors stay fatal.
            Err(lewton::VorbisError::BadAudio(_)) if !out.is_empty() => continue,
            Err(err) => return Err(e(err)),
        }
    }
    Ok(Audio { sample_rate: rate, frames: out.into() })
}

/// MPEG audio (MP3 and MP1/MP2), via `nanomp3`: tags skipped, encoder delay trimmed.
pub fn decode_mp3(bytes: &[u8]) -> Result<Audio, AudioError> {
    let d = nanomp3::decode_all::<f32>(bytes);
    let channels = d.channels.map_or(0, |c| c as usize);
    if channels == 0 || d.sample_rate == 0 || d.samples.is_empty() {
        return Err(AudioError::Mp3("no decodable MPEG audio frames".into()));
    }
    Ok(Audio { sample_rate: d.sample_rate, frames: frames_from_interleaved(d.samples.into_iter(), channels).into() })
}

/// The best file for a sound named `name`: the VFS path and its format.
/// `name` may carry any extension (or none) and a folder; only its stem counts.
pub fn find_sound(vfs: &Vfs, name: &str) -> Option<(String, SoundFormat)> {
    let formats = SoundFormat::PREFERENCE.map(|f| (f, f.extension()));
    find_best(vfs, name, &SOUND_DIRS, &formats)
}

/// Every sound file visible in the VFS (VFS paths, sorted, deduplicated).
pub fn list_sounds(vfs: &Vfs) -> Vec<String> {
    let mut out: Vec<String> = SOUND_DIRS
        .iter()
        .flat_map(|d| vfs.list(d))
        .filter(|p| p.rsplit_once('.').is_some_and(|(_, e)| SoundFormat::from_extension(e).is_some()))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Find and decode `name`.
pub fn load_sound(vfs: &Vfs, name: &str) -> Option<Result<Audio, AudioError>> {
    let (path, _) = find_sound(vfs, name)?;
    Some(match vfs.read(&path)? {
        Ok(bytes) => decode(&bytes),
        Err(e) => Err(AudioError::Io(format!("{path}: {e}"))),
    })
}

// ---- events.dat / events.odf ------------------------------------------------

/// One line of an event's sound table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundEntry {
    /// The `<race>` tag, lower case; `None` plays for every race.
    pub race: Option<String>,
    /// File name as written (`"gvofer01.wav"`).
    pub file: String,
}

/// A named event and what it plays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Event {
    pub name: String,
    pub sounds: Vec<SoundEntry>,
    /// `SoundPriority`.
    pub priority: Option<i32>,
    /// `Text`: the on-screen message (a label or literal).
    pub text: Option<String>,
    /// `TriggerInterval`: minimum seconds between two plays.
    pub trigger_interval: Option<f32>,
}

/// The event table. Names are case-insensitive; if a name repeats, the first wins.
#[derive(Debug, Clone, Default)]
pub struct Events {
    events: Vec<Event>,
    index: HashMap<String, usize>,
}

/// Cut a `#` or `//` comment off a line, outside quotes.
fn strip_comment(line: &str) -> &str {
    let (mut quoted, b) = (false, line.as_bytes());
    for i in 0..b.len() {
        match b[i] {
            b'"' => quoted = !quoted,
            b'#' if !quoted => return &line[..i],
            b'/' if !quoted && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
    }
    line
}

fn quoted(s: &str) -> Option<&str> {
    let s = s.trim();
    let s = s.strip_prefix('"')?;
    Some(&s[..s.find('"')?])
}

impl Events {
    pub fn parse(text: &str) -> Events {
        let mut ev = Events::default();
        let mut cur: Option<Event> = None;
        for raw in text.lines() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            // `Name{` opens an event, `}` closes it; the brace may share a line.
            let mut rest = line;
            if cur.is_none() {
                let Some((name, after)) = rest.split_once('{') else { continue };
                cur = Some(Event { name: name.trim().to_owned(), ..Event::default() });
                rest = after.trim();
                if rest.is_empty() {
                    continue;
                }
            }
            let closes = rest.ends_with('}');
            let body = rest.trim_end_matches('}').trim();
            if let (Some(e), false) = (cur.as_mut(), body.is_empty()) {
                let (word, args) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
                match word.to_ascii_lowercase().as_str() {
                    "sound" => {
                        let args = args.trim();
                        let (race, file) = match args.strip_prefix('<').and_then(|a| a.split_once('>')) {
                            Some((r, f)) => (Some(r.trim().to_ascii_lowercase()), f),
                            None => (None, args),
                        };
                        if let Some(file) = quoted(file) {
                            e.sounds.push(SoundEntry { race, file: file.to_owned() });
                        }
                    }
                    "soundpriority" => e.priority = args.trim().parse().ok(),
                    "triggerinterval" => e.trigger_interval = args.trim().parse().ok(),
                    "text" => e.text = quoted(args).map(str::to_owned),
                    _ => {}
                }
            }
            if closes && let Some(e) = cur.take() {
                let key = e.name.to_ascii_lowercase();
                if !ev.index.contains_key(&key) {
                    ev.index.insert(key, ev.events.len());
                    ev.events.push(e);
                }
            }
        }
        ev
    }

    /// The active events file: `events.dat` (Armada II, Fleet Operations), else
    /// `events.odf` (Armada I).
    pub fn load(vfs: &Vfs) -> Events {
        ["events.dat", "events.odf"]
            .iter()
            .find_map(|p| vfs.read(p)?.ok())
            .map(|b| Events::parse(&String::from_utf8_lossy(&b)))
            .unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Event> {
        self.index.get(&name.to_ascii_lowercase()).map(|&i| &self.events[i])
    }

    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.events.iter()
    }

    /// The sound `variant` (any number; it wraps) of event `name` for `race`.
    /// Entries tagged with the race are preferred, then untagged ones, then all.
    pub fn pick(&self, name: &str, race: Option<&str>, variant: usize) -> Option<&SoundEntry> {
        let e = self.get(name)?;
        let tagged: Vec<&SoundEntry> = race
            .map(|r| e.sounds.iter().filter(|s| s.race.as_deref().is_some_and(|x| race_matches(x, r))).collect())
            .unwrap_or_default();
        let pool = if !tagged.is_empty() {
            tagged
        } else {
            let plain: Vec<&SoundEntry> = e.sounds.iter().filter(|s| s.race.is_none()).collect();
            if plain.is_empty() { e.sounds.iter().collect() } else { plain }
        };
        (!pool.is_empty()).then(|| pool[variant % pool.len()])
    }
}

/// `federation` and `unitedfederation` name one race; likewise `klingon`/`klingonempire`, `romulan`/`romulanempire`.
fn race_matches(tag: &str, race: &str) -> bool {
    fn norm(s: &str) -> String {
        let s = s.to_ascii_lowercase();
        let s = s.strip_prefix("united").unwrap_or(&s);
        s.strip_suffix("empire").unwrap_or(s).to_owned()
    }
    norm(tag) == norm(race)
}

// ---- audio.odf -------------------------------------------------------------

/// `audio.odf`: the 3D sound environment. Defaults are the stock values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioParams {
    pub sample_rate: u32,
    pub bits_per_sample: u32,
    pub distance_factor: f32,
    pub doppler_factor: f32,
    pub rolloff_factor: f32,
    pub minimum_distance_3d: f32,
    pub maximum_distance_3d: f32,
    pub stream_buffer_length: f32,
    pub stream_buffer_sections: u32,
}

impl Default for AudioParams {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            bits_per_sample: 16,
            distance_factor: 0.01,
            doppler_factor: 1.0,
            rolloff_factor: 0.15,
            minimum_distance_3d: 50.0,
            maximum_distance_3d: 3000.0,
            stream_buffer_length: 2.0,
            stream_buffer_sections: 2,
        }
    }
}

impl AudioParams {
    pub fn parse(text: &str) -> AudioParams {
        let mut p = AudioParams::default();
        for line in text.lines() {
            let line = strip_comment(line);
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim().trim_matches('"');
            let f = || v.parse::<f32>().ok();
            let u = || v.parse::<f32>().ok().map(|x| x as u32);
            match k.trim().to_ascii_lowercase().as_str() {
                "samplerate" => p.sample_rate = u().unwrap_or(p.sample_rate),
                "bitspersample" => p.bits_per_sample = u().unwrap_or(p.bits_per_sample),
                "distancefactor" => p.distance_factor = f().unwrap_or(p.distance_factor),
                "dopplerfactor" => p.doppler_factor = f().unwrap_or(p.doppler_factor),
                "rollofffactor" => p.rolloff_factor = f().unwrap_or(p.rolloff_factor),
                "minimumdistance3d" => p.minimum_distance_3d = f().unwrap_or(p.minimum_distance_3d),
                "maximumdistance3d" => p.maximum_distance_3d = f().unwrap_or(p.maximum_distance_3d),
                "streambufferlength" => p.stream_buffer_length = f().unwrap_or(p.stream_buffer_length),
                "streambuffersections" => p.stream_buffer_sections = u().unwrap_or(p.stream_buffer_sections),
                _ => {}
            }
        }
        p
    }

    /// The highest-layer `audio.odf` wherever the game keeps it (`odf/other/`); defaults when none.
    pub fn load(vfs: &Vfs) -> AudioParams {
        let path = vfs.list("odf").into_iter().find(|p| p.rsplit('/').next() == Some("audio.odf"));
        path.and_then(|p| vfs.read(&p)?.ok())
            .map(|b| AudioParams::parse(&String::from_utf8_lossy(&b)))
            .unwrap_or_default()
    }
}

// ---- the bank ----------------------------------------------------------------

/// Where a sound name led.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// VFS path of the chosen file.
    pub path: String,
    pub format: SoundFormat,
    /// The event the name was, when it named one.
    pub event: Option<String>,
}

/// Sound lookup, tables and a decode cache over one mounted install.
///
/// Mounted data is passed per call (the bank does not own the [`Vfs`]), so one
/// bank works with whichever mount the game holds.
#[derive(Debug, Default)]
pub struct SoundBank {
    pub events: Events,
    pub params: AudioParams,
    cache: HashMap<String, Option<Arc<Audio>>>,
    /// Files that failed to decode, with the reason; for logs and reports.
    pub failures: Vec<(String, String)>,
}

impl SoundBank {
    pub fn load(vfs: &Vfs) -> SoundBank {
        SoundBank { events: Events::load(vfs), params: AudioParams::load(vfs), ..Default::default() }
    }

    /// Resolve a name to a file without decoding it. A name with an audio
    /// extension is a file. Any other name is an event first (its `variant`th
    /// sound for `race`), then a file stem.
    pub fn resolve(&self, vfs: &Vfs, name: &str, race: Option<&str>, variant: usize) -> Option<Resolved> {
        let name = name.trim();
        let has_ext = name.rsplit_once('.').is_some_and(|(_, e)| SoundFormat::from_extension(e).is_some());
        if !has_ext && let Some(e) = self.events.get(name) {
            let entry = self.events.pick(name, race, variant)?;
            let (path, format) = find_sound(vfs, &entry.file)?;
            return Some(Resolved { path, format, event: Some(e.name.clone()) });
        }
        let (path, format) = find_sound(vfs, name)?;
        Some(Resolved { path, format, event: None })
    }

    /// Resolve, read and decode; decoded sounds are cached by file. `None` when
    /// the name does not resolve or the file does not decode (recorded in
    /// [`SoundBank::failures`]).
    pub fn audio(&mut self, vfs: &Vfs, name: &str, race: Option<&str>, variant: usize) -> Option<Arc<Audio>> {
        let r = self.resolve(vfs, name, race, variant)?;
        if let Some(hit) = self.cache.get(&r.path) {
            return hit.clone();
        }
        let decoded = match vfs.read(&r.path) {
            Some(Ok(bytes)) => decode(&bytes).map_err(|e| e.to_string()),
            Some(Err(e)) => Err(e.to_string()),
            None => Err("vanished from the VFS".into()),
        };
        let audio = match decoded {
            Ok(a) => Some(Arc::new(a)),
            Err(e) => {
                self.failures.push((r.path.clone(), e));
                None
            }
        };
        self.cache.insert(r.path, audio.clone());
        audio
    }

    /// Drop decoded sounds (after a mod switch, or to free memory).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    fn wav(tag: u16, ch: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut v = b"RIFF".to_vec();
        v.extend((36 + data.len() as u32).to_le_bytes());
        v.extend(b"WAVEfmt ");
        v.extend(16u32.to_le_bytes());
        v.extend(tag.to_le_bytes());
        v.extend(ch.to_le_bytes());
        v.extend(rate.to_le_bytes());
        v.extend((rate * ch as u32 * bits as u32 / 8).to_le_bytes());
        v.extend((ch * bits / 8).to_le_bytes());
        v.extend(bits.to_le_bytes());
        v.extend(b"data");
        v.extend((data.len() as u32).to_le_bytes());
        v.extend(data);
        v
    }

    #[test]
    fn wav_8_bit_mono_is_unsigned_and_duplicated() {
        let a = decode(&wav(1, 1, 22_050, 8, &[128, 255, 0])).unwrap();
        assert_eq!(a.sample_rate, 22_050);
        assert_eq!(&*a.frames, &[[0.0, 0.0], [127.0 / 128.0; 2], [-1.0, -1.0]]);
    }

    #[test]
    fn wav_16_bit_stereo_and_extra_channels() {
        let d: Vec<u8> = [16384i16, -16384, 100, 200].iter().flat_map(|s| s.to_le_bytes()).collect();
        let a = decode(&wav(1, 2, 44_100, 16, &d)).unwrap();
        assert_eq!(&*a.frames, &[[0.5, -0.5], [100.0 / 32768.0, 200.0 / 32768.0]]);
        let d: Vec<u8> = [1i16, 2, 3, 4, 5, 6].iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(decode(&wav(1, 3, 8000, 16, &d)).unwrap().frames.len(), 2, "a frame per three samples");
    }

    #[test]
    fn wav_data_size_past_the_end_and_odd_tail() {
        let mut w = wav(1, 1, 8000, 16, &[0, 64, 0, 192, 7]);
        let n = w.len();
        w[n - 5 - 4..n - 5].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        let a = decode(&w).unwrap();
        assert_eq!(a.frames.len(), 2, "the stray last byte is dropped");
    }

    #[test]
    fn wav_rejects_what_it_cannot_play() {
        assert!(matches!(decode(&wav(2, 1, 8000, 4, &[0; 8])), Err(AudioError::Wav(_)))); // ADPCM
        assert!(matches!(decode(b"nope nope nope"), Err(AudioError::UnknownFormat)));
        assert!(matches!(decode(b"RIFF\0\0\0\0WAVE"), Err(AudioError::Wav(_))));
    }

    #[test]
    fn wav_header_without_data_is_silence() {
        let w = &wav(1, 1, 22_050, 16, &[])[..36];
        let a = decode(w).unwrap();
        assert_eq!((a.sample_rate, a.frames.len()), (22_050, 0));
    }

    #[test]
    fn formats_are_sniffed() {
        assert_eq!(SoundFormat::sniff(b"OggS...."), Some(SoundFormat::Ogg));
        assert_eq!(SoundFormat::sniff(b"ID3\x03"), Some(SoundFormat::Mp3));
        assert_eq!(SoundFormat::sniff(&[0xFF, 0xFB, 0x90]), Some(SoundFormat::Mp3));
        assert_eq!(SoundFormat::from_extension("OGG"), Some(SoundFormat::Ogg));
        assert_eq!(SoundFormat::from_extension("flac"), None);
    }

    #[test]
    fn garbage_ogg_and_mp3_are_errors_not_panics() {
        assert!(matches!(decode(b"OggS\0\0\0\0\0\0\0\0\0\0\0\0"), Err(AudioError::Ogg(_))));
        assert!(matches!(decode(b"ID3\x03\0\0\0\0\0\0xxxxxxxx"), Err(AudioError::Mp3(_))));
    }

    const EVENTS: &str = r#"#EVENT data file
InvalidEvent { #first
}

FerengiArrives{
Table
	Sound "gvofer01.wav"
	Sound "gvofer02.wav" // second
Text "Ferengi # stole one."
}
CannotRepair{
Table
	Sound <borg> "gvobcv77.wav"
	Sound <unitedfederation> "gvofcv24.wav"
	Sound <klingonempire> "gvokcv77.wav"
	SaveLocation
SoundPriority 2
TriggerInterval 30
}
OneLine{ Sound "x.wav" }
CANNOTREPAIR{
	Sound "dupe.wav"
}
"#;

    #[test]
    fn events_parse() {
        let ev = Events::parse(EVENTS);
        assert_eq!(ev.len(), 4, "the duplicate name is dropped");
        let f = ev.get("ferengiarrives").unwrap();
        assert_eq!(f.sounds.len(), 2);
        assert_eq!(f.text.as_deref(), Some("Ferengi # stole one."));
        let c = ev.get("CannotRepair").unwrap();
        assert_eq!((c.priority, c.trigger_interval), (Some(2), Some(30.0)));
        assert_eq!(c.sounds[1], SoundEntry { race: Some("unitedfederation".into()), file: "gvofcv24.wav".into() });
        assert_eq!(ev.get("OneLine").unwrap().sounds.len(), 1);
        assert!(ev.get("InvalidEvent").unwrap().sounds.is_empty());
    }

    #[test]
    fn event_picks_by_race_then_untagged_then_any() {
        let ev = Events::parse(EVENTS);
        let pick = |n: &str, r: Option<&str>, v| ev.pick(n, r, v).map(|s| s.file.as_str());
        assert_eq!(pick("CannotRepair", Some("federation"), 0), Some("gvofcv24.wav"));
        assert_eq!(pick("CannotRepair", Some("Klingon"), 5), Some("gvokcv77.wav"));
        assert_eq!(pick("CannotRepair", Some("romulan"), 0), Some("gvobcv77.wav"), "no tagged match, no untagged: any");
        assert_eq!(pick("FerengiArrives", Some("borg"), 3), Some("gvofer02.wav"), "variants wrap");
        assert_eq!(pick("nope", None, 0), None);
    }

    #[test]
    fn audio_params_parse_and_default() {
        let p = AudioParams::parse("sampleRate = 22050\nrolloffFactor = 0.3 // x\nbogus = 1\n");
        assert_eq!((p.sample_rate, p.rolloff_factor, p.maximum_distance_3d), (22_050, 0.3, 3000.0));
    }

    fn write(dir: &Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn lookup_prefers_layer_then_ogg_wav_mp3() {
        let dir = std::env::temp_dir().join(format!("sta64-con10-{}", std::process::id()));
        let (base, modd) = (dir.join("base"), dir.join("mod"));
        let w = wav(1, 1, 8000, 8, &[128; 4]);
        for ext in ["wav", "mp3", "ogg"] {
            write(&base, &format!("sounds/effects/beep.{ext}"), &w);
        }
        write(&base, "sounds/effects/only_wav.wav", &w);
        write(&base, "sounds/music/tune.mp3", &w);
        write(&base, "wav/intro.wav", &w);
        write(&modd, "sounds/effects/beep.wav", &w);
        write(&base, "events.dat", b"Ping{\nTable\n Sound <borg> \"beep.wav\"\n}\n");
        let mut vfs = Vfs::default();
        vfs.push_dir("mod".into(), modd).unwrap();
        vfs.push_dir("base".into(), base).unwrap();

        assert_eq!(find_sound(&vfs, "beep.ogg"), Some(("sounds/effects/beep.wav".into(), SoundFormat::Wav)), "mod layer beats a better format below");
        let mut vfs2 = Vfs::default();
        vfs2.push_dir("base".into(), dir.join("base")).unwrap();
        assert_eq!(find_sound(&vfs2, "BEEP.WAV"), Some(("sounds/effects/beep.ogg".into(), SoundFormat::Ogg)), "ogg > wav > mp3, any case");
        assert_eq!(find_sound(&vfs2, "only_wav").unwrap().1, SoundFormat::Wav);
        assert_eq!(find_sound(&vfs2, "tune.wav").unwrap().0, "sounds/music/tune.mp3");
        assert_eq!(find_sound(&vfs2, "intro.wav").unwrap().0, "wav/intro.wav");
        assert_eq!(find_sound(&vfs2, "missing.wav"), None);
        assert_eq!(list_sounds(&vfs2).len(), 6);

        let mut bank = SoundBank::load(&vfs);
        let r = bank.resolve(&vfs, "ping", Some("borg"), 0).unwrap();
        assert_eq!((r.event.as_deref(), r.path.as_str()), (Some("Ping"), "sounds/effects/beep.wav"));
        assert_eq!(bank.resolve(&vfs, "beep.wav", None, 0).unwrap().event, None);
        let a = bank.audio(&vfs, "beep.wav", None, 0).unwrap();
        assert_eq!(a.frames.len(), 4);
        assert!(Arc::ptr_eq(&a.frames, &bank.audio(&vfs, "beep", None, 0).unwrap().frames), "decoded once");
        // The base layer's beep.ogg is really a WAV payload: decoding it is reported, not fatal.
        let mut bank2 = SoundBank::load(&vfs2);
        assert!(bank2.audio(&vfs2, "beep.wav", None, 0).is_some(), "content is sniffed, not trusted from the extension");
        std::fs::remove_dir_all(dir).ok();
    }

    // ---- install-gated ---------------------------------------------------------
    //
    // - STA64_TEST_INSTALL (+ STA64_TEST_MOD): a Fleet Operations / Roots install.
    // - STA64_TEST_AUDIO_DIRS: plain game folders (A1 / A2 retail), each mounted
    //   as one layer; separated by the platform's path separator.

    fn mounts() -> Vec<(String, Vfs)> {
        let mut out = Vec::new();
        if let Ok(root) = std::env::var("STA64_TEST_INSTALL") {
            let m = std::env::var("STA64_TEST_MOD").ok();
            let vfs = Vfs::mount_install(Path::new(&root), m.as_deref()).unwrap();
            out.push((format!("{} [{}]", root, m.as_deref().unwrap_or("Data")), vfs));
        }
        if let Some(dirs) = std::env::var_os("STA64_TEST_AUDIO_DIRS") {
            for dir in std::env::split_paths(&dirs) {
                let mut vfs = Vfs::default();
                vfs.push_dir("root".into(), PathBuf::from(&dir)).unwrap();
                out.push((dir.display().to_string(), vfs));
            }
        }
        out
    }

    fn audio_ext(s: &str) -> bool {
        s.rsplit_once('.').is_some_and(|(_, e)| SoundFormat::from_extension(e).is_some())
    }

    /// ASCII runs in a binary that name a sound file (mission DLLs call them by name).
    fn sound_strings(bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for run in bytes.split(|b| !(0x21..0x7F).contains(b)) {
            if (5..=64).contains(&run.len()) {
                let s = String::from_utf8_lossy(run);
                if audio_ext(&s) && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)) {
                    out.push(s.into_owned());
                }
            }
        }
        out
    }

    #[test]
    fn every_installed_sound_decodes_and_every_named_sound_resolves() {
        for (label, vfs) in mounts() {
            // 1. Every installed WAV, MP3 and OGG decodes.
            let mut by_format: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
            let (mut failed, mut frames, mut empty) = (Vec::new(), 0usize, 0usize);
            let files = list_sounds(&vfs);
            for p in &files {
                let ext = SoundFormat::from_extension(p.rsplit_once('.').unwrap().1).unwrap();
                let bytes = vfs.read(p).unwrap().unwrap();
                match decode(&bytes) {
                    // Only a WAV may be empty (the stock silent `empty.wav`).
                    Ok(a) if (!a.frames.is_empty() || ext == SoundFormat::Wav) && a.sample_rate >= 4000 => {
                        by_format.entry(ext.extension()).or_default().0 += 1;
                        frames += a.frames.len();
                        empty += a.frames.is_empty() as usize;
                    }
                    Ok(_) => failed.push(format!("{p}: empty or odd rate")),
                    Err(e) => {
                        by_format.entry(ext.extension()).or_default().1 += 1;
                        failed.push(format!("{p}: {e}"));
                    }
                }
            }
            eprintln!("{label}: {} sound files, (decoded, failed) by format {by_format:?}, {frames} frames ({empty} silent)", files.len());
            for f in failed.iter().take(10) {
                eprintln!("  fail {f}");
            }

            // 2. Every event sound resolves.
            let bank = SoundBank::load(&vfs);
            let mut missing_events = BTreeSet::new();
            let mut entries = 0;
            for e in bank.events.iter() {
                for s in &e.sounds {
                    entries += 1;
                    if find_sound(&vfs, &s.file).is_none() {
                        missing_events.insert(s.file.to_ascii_lowercase());
                    }
                }
            }
            eprintln!(
                "  events: {} events, {entries} sound entries, {} distinct missing; audio.odf sampleRate {}",
                bank.events.len(),
                missing_events.len(),
                bank.params.sample_rate
            );
            for m in missing_events.iter().take(8) {
                eprintln!("    missing {m}");
            }

            // 3. Every sound an ODF names resolves (by file, or by event for event keys).
            let (mut odf_names, mut odf_missing, mut odfs) = (BTreeSet::new(), BTreeSet::new(), 0);
            for p in vfs.list("odf").into_iter().filter(|p| p.ends_with(".odf")) {
                let Some(Ok(b)) = vfs.read(&p) else { continue };
                odfs += 1;
                for en in sta_odf::parse(&String::from_utf8_lossy(&b)).entries {
                    let v = en.value.trim().trim_matches('"');
                    let key = en.key.to_ascii_lowercase();
                    let event_key = matches!(key.as_str(), "eventselect" | "eventacknowledge" | "ambientsound");
                    if v.is_empty() || !(audio_ext(v) || event_key) {
                        continue;
                    }
                    odf_names.insert(v.to_ascii_lowercase());
                    if bank.resolve(&vfs, v, None, 0).is_none() {
                        odf_missing.insert(format!("{v} ({key})"));
                    }
                }
            }
            eprintln!("  odfs: {odfs} files name {} distinct sounds, {} missing", odf_names.len(), odf_missing.len());
            for m in odf_missing.iter().take(8) {
                eprintln!("    missing {m}");
            }

            // 4. Every sound the mission DLLs name resolves.
            let (mut dlls, mut dll_names, mut dll_missing) = (0, BTreeSet::new(), BTreeSet::new());
            for dir in ["missions", "dll"] {
                for p in vfs.list(dir).into_iter().filter(|p| [".dll", ".dsl", ".drl"].iter().any(|e| p.ends_with(e))) {
                    let Some(Ok(b)) = vfs.read(&p) else { continue };
                    dlls += 1;
                    for s in sound_strings(&b) {
                        dll_names.insert(s.to_ascii_lowercase());
                        if find_sound(&vfs, &s).is_none() {
                            dll_missing.insert(s.to_ascii_lowercase());
                        }
                    }
                }
            }
            eprintln!("  mission DLLs: {dlls} read, name {} distinct sounds, {} missing", dll_names.len(), dll_missing.len());
            for m in dll_missing.iter().take(8) {
                eprintln!("    missing {m}");
            }

            assert!(!files.is_empty(), "{label}: no sound files found");
            assert!(failed.is_empty(), "{label}: {} files failed to decode, first: {:?}", failed.len(), failed.first());
        }
    }
}
