//! Bink 1 movies (`BIKf` .. `BIKi`), video and audio (MED-01).
//!
//! Container: a 44-byte header, per-track tables, a frame index, then one
//! packet per frame holding each audio track's record followed by the video
//! data. The decoder is written for this crate from public descriptions of
//! the format, chiefly the MIT-licensed clean-room specifications published
//! with the `mercs2_bink` crate (`SPEC-video.md`, `SPEC-audio.md`); no
//! GPL/LGPL decoder source was used. Revisions before `i` differ in the
//! colour bundle coding and plane size prefixes, and revisions before `h`
//! in the chroma plane order. Installed movies are `BIKf` and `BIKi`; `g`
//! and `h` are accepted but untested.

mod audio;
mod bits;
mod tables;
mod video;

use crate::{
    AudioChunk, AudioInfo, ColorSpec, Container, Matrix, MediaError, MediaEvent, MediaSource,
    Picture, Plane, Rational, ReadSeek, Result, StreamInfo, VideoFrame, VideoInfo,
};
use audio::{AudioDecoder, TrackFormat};
use std::sync::Arc;
use std::time::Duration;
use video::VideoDecoder;

/// Largest frame side decoded. The format allows 32767, which would make a
/// tiny crafted file allocate gigabytes; the installed movies are at most
/// 1104x848.
pub const MAX_SIDE: u32 = 4096;

/// The parsed container of a Bink file.
#[derive(Clone, Debug, PartialEq)]
pub struct BinkHeader {
    /// Revision letter (`b'f'`, `b'i'`, ...).
    pub revision: u8,
    pub frames: u32,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    pub video_flags: u32,
    /// Per audio track: (flags, id).
    pub tracks: Vec<(u32, u32)>,
    /// `frames + 1` entries; bit 0 marks a key frame.
    index: Vec<u32>,
}

impl BinkHeader {
    /// Parse the header and frame index from the start of a file.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let u32_at = |o: usize| -> Result<u32> {
            bytes
                .get(o..o + 4)
                .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
                .ok_or_else(|| MediaError::Corrupt(format!("Bink header truncated at {o}")))
        };
        if bytes.len() < 44 || &bytes[..3] != b"BIK" {
            return Err(MediaError::UnknownFormat);
        }
        let revision = bytes[3];
        if !(b'f'..=b'i').contains(&revision) {
            return Err(MediaError::Unsupported(format!(
                "Bink revision {:?}",
                revision as char
            )));
        }
        let frames = u32_at(8)?;
        let index_count = u32_at(16)?;
        let (width, height) = (u32_at(20)?, u32_at(24)?);
        let fps = Rational::new(u32_at(28)?, u32_at(32)?);
        let video_flags = u32_at(36)?;
        let n = u32_at(40)? as usize;
        if frames == 0 || index_count < frames || index_count > 1_000_000 {
            return Err(MediaError::Corrupt(format!(
                "{frames} frames, {index_count} index entries"
            )));
        }
        if width == 0
            || height == 0
            || width > 0x7fff
            || height > 0x7fff
            || fps.num == 0
            || fps.den == 0
        {
            return Err(MediaError::Corrupt(format!(
                "{width}x{height} at {}/{} fps",
                fps.num, fps.den
            )));
        }
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(MediaError::Unsupported(format!(
                "Bink frame {width}x{height} (at most {MAX_SIDE}x{MAX_SIDE} is decoded)"
            )));
        }
        if n > 256 {
            return Err(MediaError::Corrupt(format!("{n} audio tracks")));
        }
        // Every frame packet holds a length word per track: bounds the
        // per-track tables built from the header before any packet is read.
        if (frames as u64 + 1) * (n as u64 + 1) * 4 > bytes.len() as u64 {
            return Err(MediaError::Corrupt(format!(
                "{frames} frames of {n} tracks cannot fit in {} bytes",
                bytes.len()
            )));
        }
        let mut tracks = Vec::with_capacity(n);
        for t in 0..n {
            tracks.push((u32_at(44 + 4 * (n + t))?, u32_at(44 + 4 * (2 * n + t))?));
        }
        let base = 44 + 12 * n;
        let mut index = Vec::with_capacity(frames as usize + 1);
        for i in 0..=frames as usize {
            index.push(u32_at(base + 4 * i)?);
        }
        for i in 0..frames as usize {
            let (a, b) = (index[i] & !1, index[i + 1] & !1);
            if b <= a || b as usize > bytes.len() {
                return Err(MediaError::Corrupt(format!(
                    "frame {i} spans {a}..{b} of {}",
                    bytes.len()
                )));
            }
        }
        Ok(Self {
            revision,
            frames,
            width,
            height,
            fps,
            video_flags,
            tracks,
            index,
        })
    }

    /// Byte range of frame `i`'s packet.
    pub fn frame_range(&self, i: usize) -> std::ops::Range<usize> {
        (self.index[i] & !1) as usize..(self.index[i + 1] & !1) as usize
    }

    pub fn is_keyframe(&self, i: usize) -> bool {
        self.index[i] & 1 != 0
    }
}

/// One decodable audio track and where its cursor is.
struct Track {
    /// Position in the header's track table (records are in this order).
    slot: usize,
    decoder: AudioDecoder,
    /// Sample frames before each movie frame's record (`frames + 1` entries).
    starts: Vec<u64>,
    /// Next movie frame whose record to decode.
    next: usize,
    /// Drop sample frames before this one (after a seek).
    trim_until: u64,
}

/// A Bink movie being decoded.
pub struct BinkSource {
    bytes: Arc<Vec<u8>>,
    header: BinkHeader,
    info: StreamInfo,
    video: VideoDecoder,
    /// Next video frame to output.
    next_video: usize,
    /// Next video frame to decode (below `next_video` while catching up after a seek).
    decoded_video: usize,
    tracks: Vec<Track>,
}

impl BinkSource {
    /// Read the whole movie from `reader` and parse its container.
    pub fn open(mut reader: Box<dyn ReadSeek>) -> Result<Self> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Self::from_bytes(bytes)
    }

    /// Parse a movie held in memory.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let header = BinkHeader::parse(&bytes)?;
        let frames = header.frames as usize;
        let mut tracks = Vec::new();
        let mut audio = Vec::new();
        for (slot, &(flags, _id)) in header.tracks.iter().enumerate() {
            let Some(format) = TrackFormat::from_flags(flags) else {
                continue;
            };
            if format.rate == 0 {
                continue;
            }
            tracks.push(Track {
                slot,
                decoder: AudioDecoder::new(format),
                starts: vec![0; frames + 1],
                next: 0,
                trim_until: 0,
            });
        }
        // Sample positions of each record, from the records' decoded sizes.
        for f in 0..frames {
            let range = header.frame_range(f);
            let mut pos = range.start;
            let mut slot_bytes = vec![0u32; header.tracks.len()];
            for sb in slot_bytes.iter_mut() {
                let len =
                    read_u32(&bytes, pos, range.end).ok_or_else(|| corrupt_record(f))? as usize;
                if len != 0 {
                    *sb = read_u32(&bytes, pos + 4, range.end).ok_or_else(|| corrupt_record(f))?;
                }
                pos = pos
                    .checked_add(4 + len)
                    .filter(|&p| p <= range.end)
                    .ok_or_else(|| corrupt_record(f))?;
            }
            for t in &mut tracks {
                let fmt = t.decoder.format;
                let frames_in = slot_bytes[t.slot] as u64
                    / (fmt.bytes_per_sample() as u64 * fmt.channels as u64);
                t.starts[f + 1] = t.starts[f] + frames_in;
            }
        }
        for t in &tracks {
            let fmt = t.decoder.format;
            audio.push(AudioInfo {
                sample_rate: fmt.rate,
                channels: fmt.channels,
                codec: if fmt.dct { "bink-dct" } else { "bink-rdft" }.into(),
                total_frames: Some(t.starts[frames]),
            });
        }
        let video = VideoDecoder::new(
            header.revision,
            header.video_flags,
            header.width as usize,
            header.height as usize,
        );
        let info = StreamInfo {
            container: Container::Bink,
            video: Some(VideoInfo {
                width: header.width,
                height: header.height,
                frame_rate: header.fps,
                frame_count: Some(header.frames as u64),
                codec: format!("bink-{}", header.revision as char),
                has_alpha: video.has_alpha(),
            }),
            audio,
            duration: Some(header.fps.periods_to_duration(header.frames as u64)),
        };
        Ok(Self {
            bytes: Arc::new(bytes),
            header,
            info,
            video,
            next_video: 0,
            decoded_video: 0,
            tracks,
        })
    }

    pub fn header(&self) -> &BinkHeader {
        &self.header
    }

    /// Byte range of the record (after its length word) of header slot
    /// `slot` in frame `f`; `None` when the track has no audio there.
    fn record_range(&self, f: usize, slot: usize) -> Option<std::ops::Range<usize>> {
        let range = self.header.frame_range(f);
        let mut pos = range.start;
        for s in 0..=slot {
            let len = read_u32(&self.bytes, pos, range.end)? as usize;
            if s == slot {
                return (len != 0).then_some(pos + 4..pos + 4 + len);
            }
            pos += 4 + len;
        }
        None
    }

    fn video_range(&self, f: usize) -> Result<std::ops::Range<usize>> {
        let range = self.header.frame_range(f);
        let mut pos = range.start;
        for _ in 0..self.header.tracks.len() {
            pos += 4 + read_u32(&self.bytes, pos, range.end).ok_or_else(|| corrupt_record(f))?
                as usize;
        }
        Ok(pos..range.end)
    }

    fn decode_video(&mut self, f: usize) -> Result<()> {
        let range = self.video_range(f)?;
        let bytes = Arc::clone(&self.bytes);
        self.video
            .decode(&bytes[range])
            .map_err(|e| MediaError::Corrupt(format!("video frame {f}: {e}")))?;
        self.decoded_video = f + 1;
        Ok(())
    }

    fn picture(&self) -> Picture {
        let (y, u, v, a) = self.video.planes();
        let (w, h) = (self.header.width, self.header.height);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let crop = |p: &video::PlaneBuf, w: u32, h: u32| {
            let mut data = Vec::with_capacity((w * h) as usize);
            for r in 0..h as usize {
                data.extend_from_slice(&p.data[r * p.pitch..r * p.pitch + w as usize]);
            }
            Plane::packed(w, h, 1, data)
        };
        Picture::Yuv420 {
            y: crop(y, w, h),
            u: crop(u, cw, ch),
            v: crop(v, cw, ch),
            a: a.map(|a| crop(a, w, h)),
            color: ColorSpec {
                matrix: Matrix::Bt601,
                full_range: false,
            },
        }
    }
}

fn read_u32(bytes: &[u8], pos: usize, end: usize) -> Option<u32> {
    if pos + 4 > end {
        return None;
    }
    bytes
        .get(pos..pos + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn corrupt_record(f: usize) -> MediaError {
    MediaError::Corrupt(format!("audio records of frame {f} overrun the packet"))
}

impl MediaSource for BinkSource {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn next_event(&mut self) -> Result<Option<MediaEvent>> {
        let frames = self.header.frames as usize;
        // Audio of earlier frames (after a seek) and of this frame comes first.
        for ti in 0..self.tracks.len() {
            loop {
                let t = &self.tracks[ti];
                let f = t.next;
                if f >= frames || f > self.next_video {
                    break;
                }
                let slot = t.slot;
                let record = self.record_range(f, slot);
                let bytes = Arc::clone(&self.bytes);
                let t = &mut self.tracks[ti];
                t.next += 1;
                let Some(record) = record else { continue };
                let samples = t
                    .decoder
                    .decode_record(&bytes[record])
                    .map_err(|e| MediaError::Corrupt(format!("frame {f}: {e}")))?;
                let fmt = t.decoder.format;
                let ch = fmt.channels as u64;
                let mut first = t.starts[f];
                let mut s: &[i16] = &samples;
                if first < t.trim_until {
                    let drop = ((t.trim_until - first) * ch).min(s.len() as u64) as usize;
                    s = &s[drop..];
                    first += drop as u64 / ch;
                }
                if s.is_empty() {
                    continue;
                }
                return Ok(Some(MediaEvent::Audio(AudioChunk {
                    track: ti,
                    pts: Rational::new(fmt.rate, 1).periods_to_duration(first),
                    first_frame: first,
                    sample_rate: fmt.rate,
                    channels: fmt.channels,
                    samples: s.iter().map(|&v| v as f32 / 32768.0).collect(),
                })));
            }
        }
        if self.next_video >= frames {
            return Ok(None);
        }
        while self.decoded_video <= self.next_video {
            self.decode_video(self.decoded_video)?;
        }
        let f = self.next_video;
        self.next_video += 1;
        let fps = self.header.fps;
        Ok(Some(MediaEvent::Video(VideoFrame {
            index: f as u64,
            pts: fps.periods_to_duration(f as u64),
            duration: fps.periods_to_duration(f as u64 + 1) - fps.periods_to_duration(f as u64),
            keyframe: self.header.is_keyframe(f),
            picture: self.picture(),
        })))
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let frames = self.header.frames as usize;
        let fps = self.header.fps;
        let target = (fps.duration_to_periods(to) as usize).min(frames - 1);
        let key = (0..=target)
            .rev()
            .find(|&f| self.header.is_keyframe(f))
            .unwrap_or(0);
        // The reference state holds frame `decoded_video - 1`: keep it when it
        // lies between the key frame and the target (a forward seek), else
        // restart at the key frame. The catch-up decoding happens in the next
        // `next_event`; the installed movies have a single key frame (frame
        // 0), so a backward seek re-decodes from the start (about 200 fps
        // at 1104x848 in release).
        if !(self.decoded_video > key && self.decoded_video <= target + 1) {
            self.video.reset();
            self.decoded_video = key;
        }
        self.next_video = target;
        let at = fps.periods_to_duration(target as u64);
        for t in &mut self.tracks {
            let rate = Rational::new(t.decoder.format.rate, 1);
            let sample = rate.duration_to_periods(at);
            // The record holding the target sample: the last frame starting at or before it.
            let f = t.starts[..frames]
                .partition_point(|&s| s <= sample)
                .saturating_sub(1);
            // Start one record early so the crossfade into the target record
            // matches sequential decoding; its samples before the target are
            // dropped.
            t.next = f.saturating_sub(1);
            t.trim_until = sample;
            t.decoder.reset();
        }
        Ok(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-frame, 16x16 movie whose planes are all "fill" blocks.
    fn tiny(revision: u8) -> Vec<u8> {
        // Built with a bit writer mirroring the reader.
        struct W {
            bytes: Vec<u8>,
            bit: usize,
        }
        impl W {
            fn put(&mut self, v: u32, n: u32) {
                for i in 0..n {
                    if self.bit / 8 >= self.bytes.len() {
                        self.bytes.push(0);
                    }
                    if v >> i & 1 != 0 {
                        self.bytes[self.bit / 8] |= 1 << (self.bit % 8);
                    }
                    self.bit += 1;
                }
            }
            fn words(mut self) -> Vec<u8> {
                while !self.bytes.len().is_multiple_of(4) {
                    self.bytes.push(0);
                }
                self.bytes
            }
        }
        // A plane of 8x8 fill blocks of colour 0x5a (BIKi coding), width w.
        let plane = |w: usize, h: usize, colour: u8| {
            let mut b = W {
                bytes: Vec::new(),
                bit: 0,
            };
            for _ in 0..23 {
                b.put(0, 4); // identity trees (t = 0)
            }
            let bw = w / 8;
            let rows = h / 8;
            for row in 0..rows {
                let first = row == 0;
                // block types: fill bit + raw type 6 for the whole row
                b.put(bw as u32, bitlen_t(bw + 511));
                b.put(1, 1);
                b.put(6, 4);
                if first {
                    b.put(0, bitlen_t((w >> 4) + 511)); // sub-types: terminated
                }
                // colours: fill bit + one colour (hi tree 0 identity 4 bits, lo identity 4 bits)
                b.put(bw as u32, bitlen_t(bw * 64 + 511));
                b.put(1, 1);
                b.put((colour >> 4) as u32, 4);
                b.put((colour & 15) as u32, 4);
                if first {
                    b.put(0, bitlen_t(bw * 8 + 511));
                    b.put(0, bitlen_t(bw + 511));
                    b.put(0, bitlen_t(bw + 511));
                    b.put(0, bitlen_t(bw + 511));
                    b.put(0, bitlen_t(bw + 511));
                    b.put(0, bitlen_t(bw * 48 + 511));
                }
            }
            b.words()
        };
        let y = plane(16, 16, 0x5a);
        let a = plane(8, 8, 0x40);
        let bb = plane(8, 8, 0xc0);
        let mut video = Vec::new();
        if revision >= b'i' {
            video.extend_from_slice(&(y.len() as u32 + 4).to_le_bytes());
        }
        video.extend_from_slice(&y);
        video.extend_from_slice(&a);
        video.extend_from_slice(&bb);
        let mut f = Vec::new();
        f.extend_from_slice(b"BIK");
        f.push(revision);
        let start = 44 + 8;
        let end = start + video.len();
        for v in [
            end as u32 - 8,
            1,
            video.len() as u32,
            1,
            16,
            16,
            15,
            1,
            0,
            0,
        ] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.extend_from_slice(&(start as u32 | 1).to_le_bytes());
        f.extend_from_slice(&(end as u32).to_le_bytes());
        f.extend_from_slice(&video);
        f
    }

    fn old_colour(v: u8) -> u8 {
        let v = v as i32;
        let sign = ((v as u8 as i8) >> 7) as i32;
        ((((v & 0x7f) ^ sign) - sign + 0x80) & 0xff) as u8
    }

    fn bitlen_t(x: usize) -> u32 {
        (usize::BITS - x.leading_zeros()).clamp(9, 15)
    }

    #[test]
    fn decodes_a_synthetic_fill_movie() {
        for rev in *b"fi" {
            let mut src = BinkSource::from_bytes(tiny(rev)).unwrap();
            assert_eq!(src.info().video.as_ref().unwrap().frame_count, Some(1));
            let Some(MediaEvent::Video(frame)) = src.next_event().unwrap() else {
                panic!("no frame")
            };
            let Picture::Yuv420 { y, u, v, .. } = &frame.picture else {
                panic!()
            };
            let expect = |c: u8| {
                if rev >= b'i' { c } else { old_colour(c) }
            };
            assert!(
                y.data.iter().all(|&p| p == expect(0x5a)),
                "{:?}",
                &y.data[..4]
            );
            // BIKi codes V first, older revisions U first.
            let (first, second) = if rev >= b'i' { (v, u) } else { (u, v) };
            assert!(first.data.iter().all(|&p| p == expect(0x40)));
            assert!(second.data.iter().all(|&p| p == expect(0xc0)));
            assert!(src.next_event().unwrap().is_none());
            assert_eq!(src.seek(Duration::ZERO).unwrap(), Duration::ZERO);
            assert!(matches!(
                src.next_event().unwrap(),
                Some(MediaEvent::Video(_))
            ));
        }
    }

    /// `tiny`'s frame with one audio track (flags) whose record is `record`.
    fn with_audio(revision: u8, flags: u32, record: &[u8]) -> Vec<u8> {
        let video = tiny(revision)[52..].to_vec();
        let mut packet = (record.len() as u32).to_le_bytes().to_vec();
        packet.extend_from_slice(record);
        packet.extend_from_slice(&video);
        let start = 44 + 12 + 8;
        let end = start + packet.len();
        let mut f = b"BIK".to_vec();
        f.push(revision);
        for v in [
            end as u32 - 8,
            1,
            packet.len() as u32,
            1,
            16,
            16,
            15,
            1,
            0,
            1,
        ] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        for v in [4096u32, flags, 0, start as u32 | 1, end as u32] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.extend_from_slice(&packet);
        f
    }

    /// Decode everything (bounded), returning the event count.
    fn drain(bytes: Vec<u8>) -> Result<usize> {
        let mut src = BinkSource::from_bytes(bytes)?;
        let mut n = 0;
        while src.next_event()?.is_some() {
            n += 1;
            assert!(n < 1000, "runaway event stream");
        }
        Ok(n)
    }

    #[test]
    fn oversized_frames_are_refused_before_allocating() {
        let mut f = tiny(b'i');
        f[20..24].copy_from_slice(&0x7fffu32.to_le_bytes());
        f[24..28].copy_from_slice(&0x7fffu32.to_le_bytes());
        assert!(matches!(
            BinkSource::from_bytes(f),
            Err(MediaError::Unsupported(_))
        ));
    }

    #[test]
    fn track_tables_must_fit_the_file() {
        let mut f = tiny(b'i');
        f[40..44].copy_from_slice(&256u32.to_le_bytes());
        assert!(BinkSource::from_bytes(f).is_err());
    }

    #[test]
    fn a_synthetic_audio_track_decodes() {
        // One RDFT block of silence: 960 samples declared (1920 bytes).
        let mut record = 1920u32.to_le_bytes().to_vec();
        record.extend_from_slice(&[0; 112]);
        let n = drain(with_audio(b'i', 0xc000_5622, &record));
        assert!(n.is_ok(), "{n:?}");
    }

    #[test]
    fn a_huge_declared_audio_size_is_an_error_not_an_allocation() {
        // RDFT mono 22 050 Hz track declaring 2 GiB of samples from 4 bytes.
        let mut record = 0x7fff_fffeu32.to_le_bytes().to_vec();
        record.extend_from_slice(&[0; 4]);
        assert!(drain(with_audio(b'i', 0xc000_5622, &record)).is_err());
    }

    #[test]
    fn an_audio_record_with_undeclared_blocks_is_corrupt() {
        // Declares no samples but carries blocks.
        let mut record = 0u32.to_le_bytes().to_vec();
        record.extend_from_slice(&[0; 64]);
        assert!(drain(with_audio(b'i', 0xc000_5622, &record)).is_err());
    }

    #[test]
    fn mutated_and_truncated_movies_never_panic() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut silence = 1920u32.to_le_bytes().to_vec();
        silence.extend_from_slice(&[0; 112]);
        let bases = [
            tiny(b'i'),
            tiny(b'f'),
            with_audio(b'i', 0xc000_5622, &silence),
        ];
        for base in &bases {
            for cut in 0..base.len() {
                let _ = drain(base[..cut].to_vec());
            }
            for _ in 0..400 {
                let mut f = base.clone();
                for _ in 0..1 + next() % 6 {
                    let i = (next() as usize) % f.len();
                    f[i] = next() as u8;
                }
                let _ = drain(f);
            }
        }
    }
}
