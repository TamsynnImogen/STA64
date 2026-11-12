//! MP4 movies (MED-02): H.264 video and AAC-LC audio, pure Rust.
//!
//! Container parsing is `oxideav-mp4`, video is `oxideav-h264`, audio is
//! `oxideav-aac` (all MIT, no C code, no FFmpeg). The design follows
//! A2FOExtensions' `a2fo_media` reader: the video and the audio track are
//! each read through their own demuxer over the one file, so each track keeps
//! its own cursor, can seek on its own and decodes at its own pace. The two
//! output streams are merged by presentation time.
//!
//! * Video: frames come out in presentation order as [`Picture::Yuv420`]
//!   (8-bit 4:2:0 only; other layouts are [`MediaError::Unsupported`]). The
//!   frame index and duration come from the track's sample table, so they are
//!   exact even with B-frames.
//! * Audio: AAC-LC decodes to interleaved `f32`. The encoder delay (priming)
//!   and end padding are removed using the container's gapless information
//!   (`iTunSMPB` or the edit list), so the PCM starts at the movie's time 0
//!   and is exactly the stated length.
//! * Seek: video seeks to the sync sample at or before the target and decodes
//!   forward, dropping frames before the target; audio restarts two packets
//!   early (AAC needs one frame of overlap) and drops samples before the
//!   target.
//! * Other codecs (the installed Armada II `Intro.mp4` is AV1) are reported as
//!   [`MediaError::Unsupported`] when the file is opened.

use crate::{
    AudioChunk, AudioInfo, ColorSpec, Container, Matrix, MediaError, MediaEvent, MediaSource,
    Picture, Plane, Rational, ReadSeek, Result, StreamInfo, VideoFrame, VideoInfo,
};
use oxideav_core as core;
use oxideav_core::{Decoder, Demuxer, Frame, MediaType, PixelFormat, RuntimeContext, SampleFormat};
use oxideav_mp4::demux::{GaplessTiming, Mp4Demuxer};
use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// AAC frame length in samples; seeks back this many packets of preroll.
const AAC_FRAME: u64 = 1024;
const AAC_PREROLL_PACKETS: i64 = 2;

/// One file, many independent read positions: each demuxer gets its own
/// [`SharedReader`] so they can interleave reads without disturbing each
/// other.
struct SharedReader {
    inner: Arc<Mutex<Box<dyn ReadSeek>>>,
    pos: u64,
}

impl Read for SharedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| std::io::Error::other("reader poisoned"))?;
        g.seek(SeekFrom::Start(self.pos))?;
        let n = g.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for SharedReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let new = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => {
                let mut g = self
                    .inner
                    .lock()
                    .map_err(|_| std::io::Error::other("reader poisoned"))?;
                let end = g.seek(SeekFrom::End(0))?;
                end.checked_add_signed(d)
            }
        };
        match new {
            Some(p) => {
                self.pos = p;
                Ok(p)
            }
            None => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before start",
            )),
        }
    }
}

fn corrupt(e: core::Error) -> MediaError {
    match e {
        core::Error::Io(e) => MediaError::Io(e),
        core::Error::Unsupported(m) | core::Error::CodecNotFound(m) => MediaError::Unsupported(m),
        other => MediaError::Corrupt(other.to_string()),
    }
}

fn registry() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_h264::register(&mut ctx);
    oxideav_aac::register(&mut ctx);
    ctx
}

fn open_demuxer(
    shared: &Arc<Mutex<Box<dyn ReadSeek>>>,
    ctx: &RuntimeContext,
) -> Result<Mp4Demuxer> {
    let r = SharedReader {
        inner: shared.clone(),
        pos: 0,
    };
    oxideav_mp4::demux::open_typed(Box::new(r), &ctx.codecs).map_err(corrupt)
}

/// `ticks` of `num/den` seconds each as a [`Duration`] (negative is zero).
fn ticks_to_duration(ticks: i64, num: i64, den: i64) -> Duration {
    if ticks <= 0 || den <= 0 || num <= 0 {
        return Duration::ZERO;
    }
    let ns = ticks as u128 * num as u128 * 1_000_000_000 / den as u128;
    Duration::from_nanos(ns.min(u64::MAX as u128) as u64)
}

fn duration_to_ticks(d: Duration, num: i64, den: i64) -> i64 {
    if num <= 0 {
        return 0;
    }
    let t = d.as_nanos() * den.max(0) as u128 / (num as u128 * 1_000_000_000);
    t.min(i64::MAX as u128) as i64
}

// ---------------------------------------------------------------- video

struct VideoPipe {
    demux: Mp4Demuxer,
    decoder: Box<dyn Decoder>,
    stream: u32,
    tb: (i64, i64),
    width: u32,
    height: u32,
    /// Presentation times (stream ticks, packet timeline) in display order.
    pts: Vec<i64>,
    /// Display durations in ticks, parallel to `pts`.
    durations: Vec<i64>,
    /// Frames the decoder has returned since the last (re)start.
    out_rank: usize,
    /// Rank in `pts` of the first frame the decoder will return.
    base_rank: usize,
    /// Drop decoded frames before this tick.
    min_pts: i64,
    pending: VecDeque<VideoFrame>,
    flushed: bool,
    eof: bool,
    /// Presentation times of the sync samples, sorted.
    sync_pts: Vec<i64>,
    ctx_params: core::CodecParameters,
}

impl VideoPipe {
    fn index_of(&self, pts: i64) -> usize {
        match self.pts.binary_search(&pts) {
            Ok(i) => i,
            Err(i) => i.min(self.pts.len().saturating_sub(1)),
        }
    }

    fn duration_at(&self, ts: i64) -> Duration {
        ticks_to_duration(ts, self.tb.0, self.tb.1)
    }

    /// Next frame at or after `min_pts`, in display order.
    fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        loop {
            if let Some(f) = self.pending.pop_front() {
                return Ok(Some(f));
            }
            if self.eof {
                return Ok(None);
            }
            match self.demux.next_packet() {
                Ok(p) => {
                    if p.stream_index != self.stream {
                        continue;
                    }
                    self.decoder.send_packet(&p).map_err(corrupt)?;
                    self.drain()?;
                }
                Err(core::Error::Eof) => {
                    if !self.flushed {
                        self.flushed = true;
                        self.decoder.flush().map_err(corrupt)?;
                        self.drain()?;
                    }
                    self.eof = true;
                }
                Err(e) => return Err(corrupt(e)),
            }
        }
    }

    fn drain(&mut self) -> Result<()> {
        loop {
            match self.decoder.receive_frame() {
                Ok(Frame::Video(v)) => {
                    let rank = self.base_rank + self.out_rank;
                    self.out_rank += 1;
                    let ts = match v.pts {
                        Some(t) => t,
                        None => *self.pts.get(rank).unwrap_or(&0),
                    };
                    if ts < 0 || ts < self.min_pts {
                        continue;
                    }
                    let index = self.index_of(ts);
                    let picture = self.picture(&v)?;
                    let dur = self.durations.get(index).copied().unwrap_or(0);
                    self.pending.push_back(VideoFrame {
                        index: index as u64,
                        pts: self.duration_at(ts),
                        duration: self.duration_at(dur),
                        keyframe: self.sync_pts.binary_search(&ts).is_ok(),
                        picture,
                    });
                }
                Ok(_) => {}
                Err(core::Error::NeedMore) | Err(core::Error::Eof) => return Ok(()),
                Err(e) => return Err(corrupt(e)),
            }
        }
    }

    fn picture(&mut self, v: &core::VideoFrame) -> Result<Picture> {
        let mut params = self.ctx_params.clone();
        if let Some(out) = self.decoder.output_params() {
            params.merge_output_params(&out);
        }
        match params.pixel_format {
            Some(PixelFormat::Yuv420P) | None => {}
            Some(other) => {
                return Err(MediaError::Unsupported(format!(
                    "h264 pixel format {other:?} (only 8-bit 4:2:0 is decoded)"
                )));
            }
        }
        let planes = v.image_planes();
        if planes.len() < 3 {
            return Err(MediaError::Corrupt(format!(
                "decoded frame has {} planes",
                planes.len()
            )));
        }
        let (w, h) = (
            params.width.unwrap_or(self.width),
            params.height.unwrap_or(self.height),
        );
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let take = |p: &core::VideoPlane, pw: u32, ph: u32| -> Result<Plane> {
            let (pw_us, ph_us) = (pw as usize, ph as usize);
            if p.stride < pw_us || p.data.len() < p.stride * (ph_us - 1) + pw_us {
                return Err(MediaError::Corrupt(
                    "decoded plane smaller than the picture".into(),
                ));
            }
            let mut data = Vec::with_capacity(pw_us * ph_us);
            for row in 0..ph_us {
                data.extend_from_slice(&p.data[row * p.stride..row * p.stride + pw_us]);
            }
            Ok(Plane::packed(pw, ph, 1, data))
        };
        Ok(Picture::Yuv420 {
            y: take(&planes[0], w, h)?,
            u: take(&planes[1], cw, ch)?,
            v: take(&planes[2], cw, ch)?,
            a: None,
            color: self.color_of(v, &params, w, h),
        })
    }

    fn color_of(
        &self,
        v: &core::VideoFrame,
        params: &core::CodecParameters,
        w: u32,
        h: u32,
    ) -> ColorSpec {
        let signal = v.color_signal().unwrap_or(params.color_signal);
        let range = if signal.range.is_unspecified() {
            params.resolved_color_range()
        } else {
            signal.range
        };
        let code = if signal.matrix.0 == 2 {
            params.color_signal.matrix.0
        } else {
            signal.matrix.0
        };
        let matrix = match code {
            1 => Matrix::Bt709,
            5..=7 => Matrix::Bt601,
            // Unspecified: HD is Rec. 709, SD Rec. 601 (the usual player rule).
            _ if w >= 1280 || h > 576 => Matrix::Bt709,
            _ => Matrix::Bt601,
        };
        ColorSpec {
            matrix,
            full_range: range == core::ColorRange::Full,
        }
    }

    /// Reposition so the next frame returned is the one shown at `to`.
    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let target_ticks = duration_to_ticks(to, self.tb.0, self.tb.1);
        // The frame shown at `target`: the last whose pts <= target.
        let idx = match self.pts.binary_search(&target_ticks) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        let frame_pts = self.pts[idx];
        let landed = self
            .demux
            .seek_to(self.stream, frame_pts)
            .map_err(corrupt)?;
        self.decoder.reset().map_err(corrupt)?;
        self.pending.clear();
        self.flushed = false;
        self.eof = false;
        self.out_rank = 0;
        self.base_rank = self.index_of(landed);
        self.min_pts = frame_pts;
        Ok(self.duration_at(frame_pts))
    }
}

// ---------------------------------------------------------------- audio

struct AudioPipe {
    demux: Mp4Demuxer,
    decoder: Box<dyn Decoder>,
    stream: u32,
    track: usize,
    sample_rate: u32,
    channels: u16,
    tb: (i64, i64),
    /// Output-timeline index (sample frames from movie time 0) of the next
    /// decoded sample.
    cursor: i64,
    /// Drop output before this sample frame.
    skip_until: u64,
    /// Sample frames after which output ends (priming and padding removed).
    end_frame: Option<u64>,
    pending: VecDeque<AudioChunk>,
    flushed: bool,
    eof: bool,
    params: core::CodecParameters,
    adts: AdtsWrap,
}

/// Header fields of an ADTS frame, from an AudioSpecificConfig.
struct AdtsWrap {
    /// ADTS profile (audio object type - 1), sampling index, channel config.
    profile: u8,
    freq_index: u8,
    channel_config: u8,
}

impl AdtsWrap {
    fn from_asc(asc: &[u8]) -> Result<Self> {
        if asc.len() < 2 {
            return Err(MediaError::Corrupt(
                "aac track without an AudioSpecificConfig".into(),
            ));
        }
        let object = asc[0] >> 3;
        let freq_index = ((asc[0] & 7) << 1) | (asc[1] >> 7);
        let channel_config = (asc[1] >> 3) & 15;
        if object == 0 || object > 4 {
            return Err(MediaError::Unsupported(format!(
                "aac audio object type {object} (only AAC-LC family is decoded)"
            )));
        }
        if freq_index >= 13 {
            return Err(MediaError::Unsupported(
                "aac with an explicit sampling frequency".into(),
            ));
        }
        if channel_config == 0 || channel_config > 7 {
            return Err(MediaError::Unsupported(format!(
                "aac channel configuration {channel_config}"
            )));
        }
        Ok(Self {
            profile: object - 1,
            freq_index,
            channel_config,
        })
    }

    fn wrap(&self, raw: &[u8]) -> Result<Vec<u8>> {
        let len = raw.len() + 7;
        if len > 0x1fff {
            return Err(MediaError::Corrupt("aac access unit too large".into()));
        }
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(&[
            0xff,
            0xf1, // MPEG-4, layer 0, no CRC
            (self.profile << 6) | (self.freq_index << 2) | (self.channel_config >> 2),
            ((self.channel_config & 3) << 6) | ((len >> 11) & 3) as u8,
            ((len >> 3) & 0xff) as u8,
            (((len & 7) << 5) as u8) | 0x1f,
            0xfc,
        ]);
        out.extend_from_slice(raw);
        Ok(out)
    }
}

impl AudioPipe {
    fn next_chunk(&mut self) -> Result<Option<AudioChunk>> {
        loop {
            if let Some(c) = self.pending.pop_front() {
                return Ok(Some(c));
            }
            if self.eof {
                return Ok(None);
            }
            match self.demux.next_packet() {
                Ok(p) => {
                    if p.stream_index != self.stream {
                        continue;
                    }
                    self.decode(p)?;
                }
                Err(core::Error::Eof) => {
                    if !self.flushed {
                        self.flushed = true;
                        self.decoder.flush().map_err(corrupt)?;
                        self.drain()?;
                    }
                    self.eof = true;
                }
                Err(e) => return Err(corrupt(e)),
            }
        }
    }

    fn decode(&mut self, mut p: core::Packet) -> Result<()> {
        // `oxideav-aac` 0.1.7 reads ADTS (or LOAS) only, so each raw access
        // unit from the container is given an ADTS header built from the
        // AudioSpecificConfig.
        p.data = self.adts.wrap(&p.data)?;
        if let Some(t) = p.pts {
            self.cursor = self.ticks_to_frames(t);
        }
        self.decoder.send_packet(&p).map_err(corrupt)?;
        self.drain()
    }

    /// Packet times are `tb` seconds; the output timeline is sample frames.
    fn ticks_to_frames(&self, ticks: i64) -> i64 {
        (ticks as i128 * self.tb.0 as i128 * self.sample_rate as i128 / self.tb.1.max(1) as i128)
            as i64
    }

    fn frames_to_ticks(&self, frames: i64) -> i64 {
        (frames as i128 * self.tb.1 as i128
            / (self.tb.0.max(1) as i128 * self.sample_rate.max(1) as i128)) as i64
    }

    fn drain(&mut self) -> Result<()> {
        loop {
            match self.decoder.receive_frame() {
                Ok(Frame::Audio(a)) => {
                    let mut params = self.params.clone();
                    if let Some(out) = self.decoder.output_params() {
                        params.merge_output_params(&out);
                    }
                    let ch = params.resolved_channels().unwrap_or(self.channels) as usize;
                    let n = a.samples as usize;
                    let mut samples = to_interleaved_f32(&a, params.sample_format, ch, n)?;
                    let first = self.cursor;
                    self.cursor += n as i64;
                    // Keep only [max(0, skip_until), end): drops the encoder
                    // delay, the end padding and, after a seek, the preroll.
                    let floor = (self.skip_until as i64).max(0);
                    let lo = (floor - first).clamp(0, n as i64) as usize;
                    let mut hi = n;
                    if let Some(end) = self.end_frame {
                        hi = (end as i64 - first).clamp(0, n as i64) as usize;
                    }
                    if hi > lo {
                        let emit_first = first + lo as i64;
                        let samples: Vec<f32> = samples.drain(lo * ch..hi * ch).collect();
                        self.pending.push_back(AudioChunk {
                            track: self.track,
                            pts: ticks_to_duration(emit_first, 1, self.sample_rate as i64),
                            first_frame: emit_first as u64,
                            sample_rate: self.sample_rate,
                            channels: ch as u16,
                            samples,
                        });
                    }
                }
                Ok(_) => {}
                Err(core::Error::NeedMore) | Err(core::Error::Eof) => return Ok(()),
                Err(e) => return Err(corrupt(e)),
            }
        }
    }

    /// Resume at sample frame `frame` (clamped to the track).
    fn seek(&mut self, frame: u64) -> Result<u64> {
        let frame = self.end_frame.map_or(frame, |e| frame.min(e));
        let back = AAC_PREROLL_PACKETS * AAC_FRAME as i64;
        let ticks = self.frames_to_ticks(frame as i64 - back);
        self.demux.seek_to(self.stream, ticks).map_err(corrupt)?;
        self.decoder.reset().map_err(corrupt)?;
        self.pending.clear();
        self.flushed = false;
        self.eof = false;
        self.skip_until = frame;
        Ok(frame)
    }
}

fn to_interleaved_f32(
    a: &core::AudioFrame,
    fmt: Option<SampleFormat>,
    ch: usize,
    n: usize,
) -> Result<Vec<f32>> {
    let fmt = fmt.unwrap_or(SampleFormat::S16);
    let bad = || MediaError::Corrupt("decoded audio buffer is too short".into());
    let mut out = vec![0f32; n * ch];
    match fmt {
        SampleFormat::S16 => {
            let d = a.data.first().ok_or_else(bad)?;
            if d.len() < n * ch * 2 {
                return Err(bad());
            }
            for (o, b) in out.iter_mut().zip(d.chunks_exact(2)) {
                *o = i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0;
            }
        }
        SampleFormat::F32 => {
            let d = a.data.first().ok_or_else(bad)?;
            if d.len() < n * ch * 4 {
                return Err(bad());
            }
            for (o, b) in out.iter_mut().zip(d.chunks_exact(4)) {
                *o = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            }
        }
        SampleFormat::F32P | SampleFormat::S16P => {
            if a.data.len() < ch {
                return Err(bad());
            }
            let w = if fmt == SampleFormat::F32P { 4 } else { 2 };
            for (c, plane) in a.data.iter().take(ch).enumerate() {
                if plane.len() < n * w {
                    return Err(bad());
                }
                for (i, b) in plane.chunks_exact(w).take(n).enumerate() {
                    out[i * ch + c] = if w == 4 {
                        f32::from_le_bytes([b[0], b[1], b[2], b[3]])
                    } else {
                        i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0
                    };
                }
            }
        }
        other => {
            return Err(MediaError::Unsupported(format!(
                "aac output sample format {other:?}"
            )));
        }
    }
    Ok(out)
}

// --------------------------------------------------------------- source

/// An MP4 movie being decoded.
pub struct Mp4Source {
    info: StreamInfo,
    video: Option<VideoPipe>,
    audio: Option<AudioPipe>,
    next_video: Option<VideoFrame>,
    next_audio: Option<AudioChunk>,
    primed: bool,
}

impl Mp4Source {
    /// Read the container header from `reader`.
    pub fn open(reader: Box<dyn ReadSeek>) -> Result<Self> {
        let shared = Arc::new(Mutex::new(reader));
        let ctx = registry();
        let probe = open_demuxer(&shared, &ctx)?;
        let streams: Vec<core::StreamInfo> = probe.streams().to_vec();
        let vstream = streams
            .iter()
            .find(|s| s.params.media_type == MediaType::Video)
            .cloned();
        let astream = streams
            .iter()
            .find(|s| s.params.media_type == MediaType::Audio)
            .cloned();
        if vstream.is_none() && astream.is_none() {
            return Err(MediaError::Unsupported(
                "mp4 without a video or audio track".into(),
            ));
        }

        let mut video = None;
        if let Some(s) = &vstream {
            let id = s.params.codec_id.to_string();
            if id != "h264" {
                return Err(MediaError::Unsupported(format!(
                    "mp4 video codec {id} (only h264 is decoded)"
                )));
            }
            video = Some(Self::open_video(&shared, &ctx, s)?);
        }
        let mut audio = None;
        let mut audio_infos = Vec::new();
        if let Some(s) = &astream {
            let id = s.params.codec_id.to_string();
            if id != "aac" {
                return Err(MediaError::Unsupported(format!(
                    "mp4 audio codec {id} (only aac is decoded)"
                )));
            }
            let pipe = Self::open_audio(&shared, &ctx, s)?;
            audio_infos.push(AudioInfo {
                sample_rate: pipe.sample_rate,
                channels: pipe.channels,
                codec: "aac".into(),
                total_frames: pipe.end_frame,
            });
            audio = Some(pipe);
        }

        let video_info = video.as_ref().map(|v| VideoInfo {
            width: v.width,
            height: v.height,
            frame_rate: frame_rate(v),
            frame_count: Some(v.pts.len() as u64),
            codec: "h264".into(),
            has_alpha: false,
        });
        let duration = video
            .as_ref()
            .map(|v| {
                let last = v.pts.len().saturating_sub(1);
                v.duration_at(
                    v.pts.get(last).copied().unwrap_or(0)
                        + v.durations.get(last).copied().unwrap_or(0),
                )
            })
            .or_else(|| {
                audio
                    .as_ref()
                    .and_then(|a| a.end_frame)
                    .map(|n| ticks_to_duration(n as i64, 1, audio_rate(&audio_infos) as i64))
            });
        Ok(Self {
            info: StreamInfo {
                container: Container::Mp4,
                video: video_info,
                audio: audio_infos,
                duration,
            },
            video,
            audio,
            next_video: None,
            next_audio: None,
            primed: false,
        })
    }

    fn open_video(
        shared: &Arc<Mutex<Box<dyn ReadSeek>>>,
        ctx: &RuntimeContext,
        s: &core::StreamInfo,
    ) -> Result<VideoPipe> {
        let mut demux = open_demuxer(shared, ctx)?;
        demux.set_active_streams(&[s.index]);
        let mut decoder = ctx.codecs.first_decoder(&s.params).map_err(corrupt)?;
        // Threading inside the H.264 decoder (a hint it may ignore).
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
        decoder.set_execution_context(&core::ExecutionContext::with_threads(threads));
        let tb = (s.time_base.0.num, s.time_base.0.den);
        let samples = demux.samples(s.index);
        if samples.is_empty() {
            return Err(MediaError::Corrupt("video track has no samples".into()));
        }
        // Time shift between the media timeline and the packet timeline
        // (edit list), read off the first packet.
        let first = loop {
            match demux.next_packet() {
                Ok(p) if p.stream_index == s.index => break p,
                Ok(_) => continue,
                Err(e) => return Err(corrupt(e)),
            }
        };
        let delta = match first.pts {
            Some(p) => p - samples[0].composition_time,
            None => 0,
        };
        demux.seek_to(s.index, i64::MIN / 2).map_err(corrupt)?;
        let mut sync_pts: Vec<i64> = samples
            .iter()
            .filter(|x| x.sync)
            .map(|x| x.composition_time + delta)
            .collect();
        sync_pts.sort_unstable();
        let mut order: Vec<(i64, i64)> = samples
            .iter()
            .map(|x| (x.composition_time + delta, x.duration))
            .collect();
        order.sort_by_key(|x| x.0);
        let pts: Vec<i64> = order.iter().map(|x| x.0).collect();
        let mut durations: Vec<i64> = order.iter().map(|x| x.1).collect();
        // Display duration: until the next frame's time (the last keeps its own).
        for i in 0..pts.len().saturating_sub(1) {
            durations[i] = pts[i + 1] - pts[i];
        }
        Ok(VideoPipe {
            demux,
            decoder,
            stream: s.index,
            tb,
            width: s.params.width.unwrap_or(0),
            height: s.params.height.unwrap_or(0),
            pts,
            durations,
            out_rank: 0,
            base_rank: 0,
            min_pts: i64::MIN,
            pending: VecDeque::new(),
            flushed: false,
            eof: false,
            sync_pts,
            ctx_params: s.params.clone(),
        })
    }

    fn open_audio(
        shared: &Arc<Mutex<Box<dyn ReadSeek>>>,
        ctx: &RuntimeContext,
        s: &core::StreamInfo,
    ) -> Result<AudioPipe> {
        let mut demux = open_demuxer(shared, ctx)?;
        demux.set_active_streams(&[s.index]);
        let decoder = ctx.codecs.first_decoder(&s.params).map_err(corrupt)?;
        let sample_rate = s
            .params
            .sample_rate
            .ok_or_else(|| MediaError::Corrupt("aac track without a sample rate".into()))?;
        let channels = s.params.resolved_channels().unwrap_or(2);
        // Packet times already exclude the priming (the first, wholly
        // pre-roll packet has a negative time), so the stated valid length
        // is where the PCM ends.
        let gapless: Option<GaplessTiming> = demux.gapless_timing(s.index);
        Ok(AudioPipe {
            demux,
            decoder,
            stream: s.index,
            track: 0,
            sample_rate,
            channels,
            tb: (s.time_base.0.num, s.time_base.0.den),
            cursor: 0,
            skip_until: 0,
            end_frame: gapless.map(|g| g.valid),
            pending: VecDeque::new(),
            flushed: false,
            eof: false,
            params: s.params.clone(),
            adts: AdtsWrap::from_asc(&s.params.extradata)?,
        })
    }
}

fn audio_rate(a: &[AudioInfo]) -> u32 {
    a.first().map_or(48_000, |a| a.sample_rate)
}

fn frame_rate(v: &VideoPipe) -> Rational {
    // Frames per second from the average spacing, reduced.
    let n = v.pts.len() as i64;
    let span = v.pts.last().copied().unwrap_or(0) - v.pts.first().copied().unwrap_or(0)
        + v.durations.last().copied().unwrap_or(0);
    if n == 0 || span <= 0 {
        return Rational::new(0, 1);
    }
    let num = n as i128 * v.tb.1 as i128;
    let den = span as i128 * v.tb.0 as i128;
    let g = gcd(num as u128, den as u128).max(1) as i128;
    Rational::new((num / g) as u32, (den / g) as u32)
}

fn gcd(a: u128, b: u128) -> u128 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl MediaSource for Mp4Source {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn next_event(&mut self) -> Result<Option<MediaEvent>> {
        if !self.primed {
            self.fill()?;
            self.primed = true;
        }
        let take_video = match (&self.next_video, &self.next_audio) {
            (None, None) => return Ok(None),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(v), Some(a)) => v.pts <= a.pts,
        };
        let event = if take_video {
            let f = self.next_video.take().map(MediaEvent::Video);
            if let Some(v) = self.video.as_mut() {
                self.next_video = v.next_frame()?;
            }
            f
        } else {
            let c = self.next_audio.take().map(MediaEvent::Audio);
            if let Some(a) = self.audio.as_mut() {
                self.next_audio = a.next_chunk()?;
            }
            c
        };
        Ok(event)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let mut at = to;
        if let Some(v) = self.video.as_mut() {
            at = v.seek(to)?;
        }
        if let Some(a) = self.audio.as_mut() {
            let rate = a.sample_rate as i64;
            let frame = duration_to_ticks(at, 1, rate).max(0) as u64;
            let landed = a.seek(frame)?;
            if self.video.is_none() {
                at = ticks_to_duration(landed as i64, 1, rate);
            }
        }
        self.next_video = None;
        self.next_audio = None;
        self.primed = false;
        Ok(at)
    }
}

impl Mp4Source {
    fn fill(&mut self) -> Result<()> {
        if let Some(v) = self.video.as_mut() {
            self.next_video = v.next_frame()?;
        }
        if let Some(a) = self.audio.as_mut() {
            self.next_audio = a.next_chunk()?;
        }
        Ok(())
    }
}
