//! Movie decoding for STA64: Bink 1 (`.bik`) and MP4 (`.mp4`).
//!
//! This crate turns movie bytes into video frames (planar Y'CbCr or RGBA)
//! and interleaved `f32` PCM, with timestamps, seeking and end-of-stream. It
//! knows nothing about the engine: playback, presentation, mixing and the
//! choice between a `.bik` and a same-stem `.mp4` belong to the client
//! (`sta64/src/media/`, MED-03).
//!
//! ```no_run
//! # fn main() -> sta_media::Result<()> {
//! let bytes = std::fs::read("Intro.bik")?;
//! let mut movie = sta_media::Movie::open_bytes(bytes)?;
//! while let Some(frame) = movie.next_video_frame()? {
//!     let rgba = frame.to_rgba(); // width * height * 4
//!     for chunk in movie.take_audio() {
//!         // chunk.samples: interleaved f32, chunk.channels wide
//!     }
//! #   let _ = rgba;
//! }
//! # Ok(()) }
//! ```
//!
//! Packages: the seam (this file), MED-01 (`src/bink/`), MED-02 (`src/mp4.rs`).

use std::collections::VecDeque;
use std::io::{Cursor, Read, Seek};
use std::time::Duration;

#[cfg(feature = "bink")]
pub mod bink;
#[cfg(feature = "mp4")]
pub mod mp4;

/// The result type of this crate.
pub type Result<T> = std::result::Result<T, MediaError>;

/// Why a movie could not be opened or decoded.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    /// The first bytes are not a container this crate reads.
    #[error("not a Bink or MP4 movie")]
    UnknownFormat,
    /// A recognised container using a codec, revision or feature this build
    /// does not decode (named in the message).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The file is damaged or truncated (what and where in the message).
    #[error("corrupt movie: {0}")]
    Corrupt(String),
    /// [`Movie::open_vfs`] found no such path in any layer.
    #[error("{0}: not found")]
    NotFound(String),
}

/// A container format this crate recognises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    /// RAD Bink 1 (`BIK` + revision byte).
    Bink,
    /// ISO base media file (`.mp4`, `.mov`, `.m4v`).
    Mp4,
}

/// Recognise a container from the first bytes of a file (16 are enough).
pub fn sniff(head: &[u8]) -> Option<Container> {
    if head.len() >= 4 && &head[..3] == b"BIK" {
        return Some(Container::Bink);
    }
    if head.len() >= 8 {
        let tag = &head[4..8];
        if matches!(
            tag,
            b"ftyp" | b"moov" | b"mdat" | b"free" | b"wide" | b"skip"
        ) {
            return Some(Container::Mp4);
        }
    }
    None
}

/// An exact ratio, e.g. a frame rate of 30000/1001.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

impl Rational {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }

    pub fn as_f64(self) -> f64 {
        if self.den == 0 {
            0.0
        } else {
            self.num as f64 / self.den as f64
        }
    }

    /// Time of `count` periods of this rate (frames at a frame rate), exact
    /// to the nanosecond, saturating on overflow.
    pub fn periods_to_duration(self, count: u64) -> Duration {
        if self.num == 0 {
            return Duration::ZERO;
        }
        let nanos = count as u128 * self.den as u128 * 1_000_000_000 / self.num as u128;
        Duration::from_nanos(nanos.min(u64::MAX as u128) as u64)
    }

    /// Whole periods of this rate that fit in `t` (the frame index shown at
    /// time `t`).
    pub fn duration_to_periods(self, t: Duration) -> u64 {
        if self.den == 0 {
            return 0;
        }
        let p = t.as_nanos() * self.num as u128 / (self.den as u128 * 1_000_000_000);
        p.min(u64::MAX as u128) as u64
    }
}

/// The video track of a movie.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    /// Display size in pixels.
    pub width: u32,
    pub height: u32,
    /// Frames per second.
    pub frame_rate: Rational,
    /// Frames in the file, when the container states it.
    pub frame_count: Option<u64>,
    /// Codec name for logs: `"bink"`, `"h264"`, ...
    pub codec: String,
    /// Frames carry an alpha plane.
    pub has_alpha: bool,
}

/// One audio track of a movie.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
    /// Codec name for logs: `"bink-rdft"`, `"bink-dct"`, `"aac"`, ...
    pub codec: String,
    /// Sample frames (per channel) the track decodes to, when known up front.
    pub total_frames: Option<u64>,
}

/// What a movie holds.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub container: Container,
    pub video: Option<VideoInfo>,
    /// Audio tracks in file order; [`AudioChunk::track`] indexes this.
    pub audio: Vec<AudioInfo>,
    /// Running time, when known up front.
    pub duration: Option<Duration>,
}

/// One 8-bit image plane: `height` rows of `stride` bytes, of which the
/// first `width` (times bytes per pixel) are the picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub data: Vec<u8>,
}

impl Plane {
    /// A tightly packed plane (`stride == width * bytes_per_pixel`).
    pub fn packed(width: u32, height: u32, bytes_per_pixel: usize, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            stride: width as usize * bytes_per_pixel,
            data,
        }
    }

    /// Row `y`, `stride` bytes (the last row may be shorter if the producer
    /// trimmed padding).
    pub fn row(&self, y: u32) -> &[u8] {
        let start = y as usize * self.stride;
        let end = (start + self.stride).min(self.data.len());
        &self.data[start..end]
    }
}

/// Y'CbCr → R'G'B' matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Matrix {
    Bt601,
    Bt709,
}

/// How to turn a planar picture into RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColorSpec {
    pub matrix: Matrix,
    /// `true`: 0..255 ("full"/"JPEG" range); `false`: Y 16..235, C 16..240.
    pub full_range: bool,
}

/// Pixels of one decoded frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Picture {
    /// Planar 4:2:0: chroma planes are `ceil(w/2) x ceil(h/2)`. `a`, when
    /// present, is a full-size alpha plane.
    Yuv420 {
        y: Plane,
        u: Plane,
        v: Plane,
        a: Option<Plane>,
        color: ColorSpec,
    },
    /// Packed 8-bit RGBA (`bytes per pixel` 4).
    Rgba(Plane),
}

/// One decoded video frame.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoFrame {
    /// 0-based presentation index.
    pub index: u64,
    /// Presentation time from the start of the movie.
    pub pts: Duration,
    /// How long it stays on screen.
    pub duration: Duration,
    /// The frame is a key frame (a seek can land on it).
    pub keyframe: bool,
    pub picture: Picture,
}

impl VideoFrame {
    pub fn width(&self) -> u32 {
        match &self.picture {
            Picture::Yuv420 { y, .. } => y.width,
            Picture::Rgba(p) => p.width,
        }
    }

    pub fn height(&self) -> u32 {
        match &self.picture {
            Picture::Yuv420 { y, .. } => y.height,
            Picture::Rgba(p) => p.height,
        }
    }

    /// The frame as tightly packed RGBA (`width * height * 4` bytes). Alpha
    /// comes from the alpha plane, else 255.
    pub fn to_rgba(&self) -> Vec<u8> {
        match &self.picture {
            Picture::Rgba(p) => {
                let row_bytes = p.width as usize * 4;
                let mut out = Vec::with_capacity(row_bytes * p.height as usize);
                for y in 0..p.height {
                    out.extend_from_slice(&p.row(y)[..row_bytes]);
                }
                out
            }
            Picture::Yuv420 { y, u, v, a, color } => yuv420_to_rgba(y, u, v, a.as_ref(), *color),
        }
    }
}

/// Convert planar 4:2:0 to packed RGBA with 16.16 fixed-point coefficients.
pub fn yuv420_to_rgba(
    y: &Plane,
    u: &Plane,
    v: &Plane,
    a: Option<&Plane>,
    color: ColorSpec,
) -> Vec<u8> {
    // Coefficients for R = Y' + cr_r*Cr, G = Y' - cb_g*Cb - cr_g*Cr, B = Y' + cb_b*Cb
    // (Kr, Kb from the matrix), scaled for the range.
    let (kr, kb) = match color.matrix {
        Matrix::Bt601 => (0.299f64, 0.114f64),
        Matrix::Bt709 => (0.2126f64, 0.0722f64),
    };
    let kg = 1.0 - kr - kb;
    let (y_scale, c_scale, y_off) = if color.full_range {
        (1.0, 1.0, 0)
    } else {
        (255.0 / 219.0, 255.0 / 224.0, 16)
    };
    let f = |x: f64| (x * 65536.0).round() as i32;
    let ys = f(y_scale);
    let cr_r = f(2.0 * (1.0 - kr) * c_scale);
    let cb_b = f(2.0 * (1.0 - kb) * c_scale);
    let cb_g = f(2.0 * (1.0 - kb) * kb / kg * c_scale);
    let cr_g = f(2.0 * (1.0 - kr) * kr / kg * c_scale);
    let clamp = |x: i32| ((x + 0x8000) >> 16).clamp(0, 255) as u8;

    let (w, h) = (y.width as usize, y.height as usize);
    let mut out = vec![0u8; w * h * 4];
    for row in 0..h {
        let yr = y.row(row as u32);
        let ur = u.row((row / 2) as u32);
        let vr = v.row((row / 2) as u32);
        let ar = a.map(|a| a.row(row as u32));
        let o = &mut out[row * w * 4..(row + 1) * w * 4];
        for col in 0..w {
            let yy = (yr[col] as i32 - y_off) * ys;
            let cb = ur[col / 2] as i32 - 128;
            let cr = vr[col / 2] as i32 - 128;
            let px = &mut o[col * 4..col * 4 + 4];
            px[0] = clamp(yy + cr_r * cr);
            px[1] = clamp(yy - cb_g * cb - cr_g * cr);
            px[2] = clamp(yy + cb_b * cb);
            px[3] = ar.map_or(255, |r| r[col]);
        }
    }
    out
}

/// A run of decoded audio for one track.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioChunk {
    /// Index into [`StreamInfo::audio`].
    pub track: usize,
    /// Presentation time of the first sample frame.
    pub pts: Duration,
    /// Sample-frame index of the first sample frame since the movie start.
    pub first_frame: u64,
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples in -1.0..=1.0, `frames() * channels` long.
    pub samples: Vec<f32>,
}

impl AudioChunk {
    /// Sample frames (samples per channel) in this chunk.
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }
}

/// One unit of decoded output, in stream order.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaEvent {
    Video(VideoFrame),
    Audio(AudioChunk),
}

/// A readable, seekable byte source.
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// The decoder interface each container implements (Bink, MP4). Consumers
/// normally use [`Movie`], which adds an audio queue on top.
pub trait MediaSource: Send {
    fn info(&self) -> &StreamInfo;

    /// The next decoded video frame or audio chunk in stream order; `None`
    /// once every track has ended. After an error the source's state is
    /// unspecified until the next [`MediaSource::seek`]; [`Movie`] treats an
    /// error as the end of the movie.
    fn next_event(&mut self) -> Result<Option<MediaEvent>>;

    /// Reposition so that the next video frame is the one shown at `to`
    /// (clamped to the movie), decoding forward from the preceding key frame
    /// as needed. Audio resumes at the same time, trimmed to start no later
    /// than the returned position. Returns the presentation time of the next
    /// video frame (or of the next audio sample for a movie without video).
    fn seek(&mut self, to: Duration) -> Result<Duration>;
}

/// An open movie: a [`MediaSource`] plus a queue for audio decoded while
/// fetching video frames.
pub struct Movie {
    source: Box<dyn MediaSource>,
    audio: VecDeque<AudioChunk>,
    video_done: bool,
    finished: bool,
}

impl std::fmt::Debug for Movie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Movie")
            .field("info", self.source.info())
            .field("finished", &self.finished)
            .finish()
    }
}

impl Movie {
    /// Wrap an already opened source.
    pub fn from_source(source: Box<dyn MediaSource>) -> Self {
        let video_done = source.info().video.is_none();
        Self {
            source,
            audio: VecDeque::new(),
            video_done,
            finished: false,
        }
    }

    /// Open a movie held in memory, the container found from its first bytes.
    pub fn open_bytes(bytes: Vec<u8>) -> Result<Self> {
        // Bink keeps the whole file: hand the buffer over without a copy.
        #[cfg(feature = "bink")]
        if sniff(&bytes) == Some(Container::Bink) {
            return guard(|| bink::BinkSource::from_bytes(bytes))
                .map(|s| Self::from_source(Box::new(s)));
        }
        Self::open_reader(Box::new(Cursor::new(bytes)))
    }

    /// Open a movie from a seekable reader, the container found from its
    /// first bytes.
    pub fn open_reader(mut reader: Box<dyn ReadSeek>) -> Result<Self> {
        let mut head = [0u8; 16];
        let n = read_up_to(&mut reader, &mut head)?;
        reader.seek(std::io::SeekFrom::Start(0))?;
        let source: Box<dyn MediaSource> = guard(move || -> Result<Box<dyn MediaSource>> {
            match sniff(&head[..n]) {
                #[cfg(feature = "bink")]
                Some(Container::Bink) => Ok(Box::new(bink::BinkSource::open(reader)?)),
                #[cfg(feature = "mp4")]
                Some(Container::Mp4) => Ok(Box::new(mp4::Mp4Source::open(reader)?)),
                #[allow(unreachable_patterns)]
                Some(c) => Err(MediaError::Unsupported(format!("{c:?} support not built"))),
                None => Err(MediaError::UnknownFormat),
            }
        })?;
        Ok(Self::from_source(source))
    }

    /// Open `path` from the install/mod layers.
    pub fn open_vfs(vfs: &sta_vfs::Vfs, path: &str) -> Result<Self> {
        match vfs.read(path) {
            Some(bytes) => Self::open_bytes(bytes?),
            None => Err(MediaError::NotFound(path.to_owned())),
        }
    }

    pub fn info(&self) -> &StreamInfo {
        self.source.info()
    }

    /// The next event in stream order, draining queued audio first.
    pub fn next_event(&mut self) -> Result<Option<MediaEvent>> {
        if let Some(chunk) = self.audio.pop_front() {
            return Ok(Some(MediaEvent::Audio(chunk)));
        }
        self.pull()
    }

    fn pull(&mut self) -> Result<Option<MediaEvent>> {
        if self.finished {
            return Ok(None);
        }
        let source = &mut self.source;
        let event = guard(|| source.next_event());
        if !matches!(event, Ok(Some(_))) {
            // End of stream, or an error: either ends the movie (an error
            // is reported once) until the next seek.
            self.finished = true;
            self.video_done = true;
        }
        event
    }

    /// The next video frame; audio decoded on the way waits in
    /// [`Movie::take_audio`]. `None` at the end of the video track (or at
    /// once for a movie without video); remaining audio is then decoded into
    /// the queue.
    pub fn next_video_frame(&mut self) -> Result<Option<VideoFrame>> {
        if self.video_done {
            self.drain_audio()?;
            return Ok(None);
        }
        loop {
            match self.pull()? {
                Some(MediaEvent::Video(frame)) => return Ok(Some(frame)),
                Some(MediaEvent::Audio(chunk)) => self.audio.push_back(chunk),
                None => return Ok(None),
            }
        }
    }

    fn drain_audio(&mut self) -> Result<()> {
        while let Some(event) = self.pull()? {
            if let MediaEvent::Audio(chunk) = event {
                self.audio.push_back(chunk);
            }
        }
        Ok(())
    }

    /// Audio chunks queued so far, oldest first.
    pub fn take_audio(&mut self) -> Vec<AudioChunk> {
        self.audio.drain(..).collect()
    }

    /// See [`MediaSource::seek`]. Clears queued audio and end-of-stream.
    pub fn seek(&mut self, to: Duration) -> Result<Duration> {
        self.audio.clear();
        let source = &mut self.source;
        let at = guard(|| source.seek(to))?;
        self.finished = false;
        self.video_done = self.source.info().video.is_none();
        Ok(at)
    }

    /// Every track has ended and the audio queue is empty.
    pub fn is_finished(&self) -> bool {
        self.finished && self.audio.is_empty()
    }
}

/// Run decoder code, turning a panic inside it (a bug or a hostile file
/// tripping a dependency) into an error instead of unwinding into the game.
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            let what = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(MediaError::Corrupt(format!("decoder panicked: {what}")))
        }
    }
}

fn read_up_to(r: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_containers() {
        assert_eq!(sniff(b"BIKi\0\0\0\0"), Some(Container::Bink));
        assert_eq!(sniff(b"\0\0\0\x20ftypisom"), Some(Container::Mp4));
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVE"), None);
        assert_eq!(sniff(b"BI"), None);
    }

    #[test]
    fn unknown_bytes_are_refused() {
        assert!(matches!(
            Movie::open_bytes(b"RIFF\0\0\0\0WAVEfmt ".to_vec()),
            Err(MediaError::UnknownFormat)
        ));
    }

    #[test]
    fn rational_time() {
        let r = Rational::new(30000, 1001);
        assert_eq!(r.periods_to_duration(30000), Duration::from_secs(1001));
        assert_eq!(r.duration_to_periods(Duration::from_secs(1001)), 30000);
        assert_eq!(
            Rational::new(15, 1).duration_to_periods(Duration::from_millis(999)),
            14
        );
    }

    #[test]
    fn grey_and_primaries_convert() {
        let plane = |w, h, v| Plane::packed(w, h, 1, vec![v; (w * h) as usize]);
        let limited = ColorSpec {
            matrix: Matrix::Bt601,
            full_range: false,
        };
        let grey = yuv420_to_rgba(
            &plane(2, 2, 126),
            &plane(1, 1, 128),
            &plane(1, 1, 128),
            None,
            limited,
        );
        assert!(grey.chunks(4).all(|p| p == [128, 128, 128, 255]));
        let white = yuv420_to_rgba(
            &plane(1, 1, 235),
            &plane(1, 1, 128),
            &plane(1, 1, 128),
            None,
            limited,
        );
        assert_eq!(white, [255, 255, 255, 255]);
        // Full-range BT.601 red is Y 76, Cb 85, Cr 255.
        let full = ColorSpec {
            matrix: Matrix::Bt601,
            full_range: true,
        };
        let red = yuv420_to_rgba(
            &plane(1, 1, 76),
            &plane(1, 1, 85),
            &plane(1, 1, 255),
            None,
            full,
        );
        assert!(red[0] >= 253 && red[1] <= 2 && red[2] <= 2, "{red:?}");
    }
}
