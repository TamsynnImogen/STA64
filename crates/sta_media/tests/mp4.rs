//! MP4 decoding (MED-02) against small synthetic fixtures; see
//! `tests/fixtures/README.md` for how they were made and the ffprobe numbers
//! asserted here.
#![cfg(feature = "mp4")]

use sta_media::mp4::Mp4Source;
use sta_media::{AudioChunk, MediaError, MediaEvent, MediaSource, Movie, Picture, VideoFrame};
use std::io::Cursor;
use std::time::Duration;

const AV: &[u8] = include_bytes!("fixtures/h264_aac_160x120.mp4");
const SILENT: &[u8] = include_bytes!("fixtures/h264_silent_96x64.mp4");
const AV1: &[u8] = include_bytes!("fixtures/av1_64x64.mp4");

// ffprobe: 160x120, 25 fps, 75 frames, keyframes every 25 (-g 25, B-frames);
// AAC-LC 44.1 kHz stereo; `ffmpeg -f f32le` decodes 132300 sample frames
// (3.000 s exactly, priming and padding removed).
const AV_FRAMES: u64 = 75;
const AV_AUDIO_FRAMES: u64 = 132_300;

fn open(bytes: &[u8]) -> Mp4Source {
    Mp4Source::open(Box::new(Cursor::new(bytes.to_vec()))).expect("open")
}

fn drain(src: &mut dyn MediaSource) -> (Vec<VideoFrame>, Vec<AudioChunk>) {
    let (mut v, mut a) = (Vec::new(), Vec::new());
    while let Some(e) = src.next_event().expect("decode") {
        match e {
            MediaEvent::Video(f) => v.push(f),
            MediaEvent::Audio(c) => a.push(c),
        }
    }
    (v, a)
}

fn assert_contiguous(audio: &[AudioChunk], first: u64) -> u64 {
    let mut next = first;
    for c in audio {
        assert_eq!(c.first_frame, next, "audio chunks are gap-free");
        assert_eq!(c.channels, 2);
        assert!(c.samples.iter().all(|s| s.abs() <= 1.0));
        next += c.frames() as u64;
    }
    next
}

#[test]
fn stream_info() {
    let src = open(AV);
    let info = src.info();
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height), (160, 120));
    assert_eq!(v.frame_rate.as_f64(), 25.0);
    assert_eq!(v.frame_count, Some(AV_FRAMES));
    assert_eq!(v.codec, "h264");
    assert_eq!(info.audio.len(), 1);
    assert_eq!(
        (info.audio[0].sample_rate, info.audio[0].channels),
        (44_100, 2)
    );
    assert_eq!(info.audio[0].total_frames, Some(AV_AUDIO_FRAMES));
    assert_eq!(info.duration, Some(Duration::from_secs(3)));
}

#[test]
fn decodes_every_frame_and_sample() {
    let mut src = open(AV);
    let (video, audio) = drain(&mut src);
    assert_eq!(video.len() as u64, AV_FRAMES);
    for (i, f) in video.iter().enumerate() {
        assert_eq!(f.index, i as u64);
        assert_eq!(f.pts, Duration::from_millis(40 * i as u64), "frame {i}");
        assert_eq!(f.duration, Duration::from_millis(40));
        assert_eq!(f.keyframe, i % 25 == 0, "frame {i}");
        let Picture::Yuv420 { y, u, .. } = &f.picture else {
            panic!("not 4:2:0")
        };
        assert_eq!((y.width, y.height, u.width, u.height), (160, 120, 80, 60));
    }
    // testsrc is not flat.
    let Picture::Yuv420 { y, .. } = &video[10].picture else {
        unreachable!()
    };
    assert!(y.data.iter().any(|&b| b != y.data[0]));
    assert_eq!(video[0].to_rgba().len(), 160 * 120 * 4);
    assert_eq!(assert_contiguous(&audio, 0), AV_AUDIO_FRAMES);
    // 440 Hz sine at 0.125 amplitude (ffmpeg's default): audible, not silent.
    let peak = audio
        .iter()
        .flat_map(|c| c.samples.iter())
        .fold(0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "peak {peak}");
}

#[test]
fn events_are_in_time_order() {
    let mut src = open(AV);
    let mut last = Duration::ZERO;
    while let Some(e) = src.next_event().unwrap() {
        let t = match e {
            MediaEvent::Video(f) => f.pts,
            MediaEvent::Audio(c) => c.pts,
        };
        assert!(t >= last, "{t:?} after {last:?}");
        last = t;
    }
}

#[test]
fn end_of_stream_is_sticky() {
    let mut movie = Movie::open_bytes(AV.to_vec()).unwrap();
    let mut frames = 0;
    while movie.next_video_frame().unwrap().is_some() {
        frames += 1;
    }
    assert_eq!(frames, AV_FRAMES);
    let audio: u64 = movie.take_audio().iter().map(|c| c.frames() as u64).sum();
    assert!(audio <= AV_AUDIO_FRAMES);
    assert!(movie.next_event().unwrap().is_none());
    assert!(movie.is_finished());
    assert!(movie.next_event().unwrap().is_none());
}

#[test]
fn seek_to_a_non_key_frame() {
    let mut reference = open(AV);
    let (all_video, all_audio) = drain(&mut reference);

    let mut src = open(AV);
    // Frame 32 is shown from 1.28 s to 1.32 s; 1.30 s is inside it. Its
    // preceding key frame is frame 25.
    let at = src.seek(Duration::from_millis(1300)).unwrap();
    assert_eq!(at, Duration::from_millis(1280));
    let (video, audio) = drain(&mut src);
    assert_eq!(video.len() as u64, AV_FRAMES - 32);
    assert_eq!(video[0].index, 32);
    assert_eq!(video[0].pts, at);
    // Decoding forward from the key frame reproduces the straight decode.
    assert_eq!(video[0].picture, all_video[32].picture);
    assert_eq!(video.last().unwrap().picture, all_video[74].picture);
    // Audio resumes exactly at the returned position and runs to the end.
    let start = (44_100.0 * 1.28) as u64;
    assert_eq!(audio[0].first_frame, start);
    assert_eq!(assert_contiguous(&audio, start), AV_AUDIO_FRAMES);
    // ... and matches the straight decode of the same samples (AAC preroll).
    let straight: Vec<f32> = all_audio
        .iter()
        .flat_map(|c| c.samples.iter().copied())
        .collect();
    let after: Vec<f32> = audio
        .iter()
        .flat_map(|c| c.samples.iter().copied())
        .collect();
    let tail = &straight[start as usize * 2..];
    assert_eq!(tail.len(), after.len());
    let worst = tail
        .iter()
        .zip(&after)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(worst < 1e-3, "worst sample difference {worst}");
}

#[test]
fn seek_to_key_frame_start_end_and_back() {
    let mut src = open(AV);
    let _ = drain(&mut src);
    // From the end of stream back to the start.
    assert_eq!(src.seek(Duration::ZERO).unwrap(), Duration::ZERO);
    let (video, audio) = drain(&mut src);
    assert_eq!(video.len() as u64, AV_FRAMES);
    assert_eq!(assert_contiguous(&audio, 0), AV_AUDIO_FRAMES);
    // Exactly on a key frame.
    assert_eq!(
        src.seek(Duration::from_secs(1)).unwrap(),
        Duration::from_secs(1)
    );
    let (video, _) = drain(&mut src);
    assert_eq!(video[0].index, 25);
    assert!(video[0].keyframe);
    // Past the end clamps to the last frame.
    let at = src.seek(Duration::from_secs(60)).unwrap();
    assert_eq!(at, Duration::from_millis(40 * 74));
    let (video, _) = drain(&mut src);
    assert_eq!(video.len(), 1);
    assert_eq!(video[0].index, 74);
}

#[test]
fn movie_seek_clears_end_of_stream() {
    let mut movie = Movie::open_bytes(AV.to_vec()).unwrap();
    while movie.next_video_frame().unwrap().is_some() {}
    while movie.next_event().unwrap().is_some() {}
    assert!(movie.is_finished());
    let at = movie.seek(Duration::from_millis(500)).unwrap();
    assert_eq!(at, Duration::from_millis(480));
    assert!(!movie.is_finished());
    let f = movie.next_video_frame().unwrap().unwrap();
    assert_eq!(f.index, 12);
}

#[test]
fn silent_movie() {
    let mut src = open(SILENT);
    let info = src.info().clone();
    assert!(info.audio.is_empty());
    let v = info.video.unwrap();
    assert_eq!((v.width, v.height, v.frame_count), (96, 64, Some(60)));
    assert_eq!(v.frame_rate.as_f64(), 30.0);
    let (video, audio) = drain(&mut src);
    assert_eq!(video.len(), 60);
    assert!(audio.is_empty());
    // Seek in a movie with keyframes every 15 frames: frame 44.
    let at = src.seek(Duration::from_millis(1470)).unwrap();
    assert_eq!(at.as_millis(), 1466);
    let (video, _) = drain(&mut src);
    assert_eq!(video[0].index, 44);
    assert_eq!(video.len(), 16);
}

#[test]
fn av1_is_reported_unsupported() {
    match Mp4Source::open(Box::new(Cursor::new(AV1.to_vec()))) {
        Err(MediaError::Unsupported(m)) => assert!(m.contains("av01") || m.contains("av1"), "{m}"),
        Err(e) => panic!("wrong error {e}"),
        Ok(_) => panic!("AV1 must not open"),
    }
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    assert!(Mp4Source::open(Box::new(Cursor::new(vec![0u8; 64]))).is_err());
    let truncated = &AV[..AV.len() / 2];
    if let Ok(mut s) = Mp4Source::open(Box::new(Cursor::new(truncated.to_vec()))) {
        while let Ok(Some(_)) = s.next_event() {}
    }
}

/// Opens every `.mp4` under `STA64_TEST_MP4_DIR` and decodes it. A file whose
/// codec is not H.264/AAC (the installed Armada II `Intro.mp4` is AV1) is
/// reported and passes; an H.264 or AAC file that fails, including one in a
/// profile this build does not decode, fails the test. Skipped without the
/// variable. Run with `--nocapture` to see the per-file lines.
#[test]
fn installed_movies() {
    let dir = std::env::var_os("STA64_TEST_MP4_DIR").map(std::path::PathBuf::from);
    let Some(dir) = dir else {
        eprintln!("installed_movies: no install directory, skipped");
        return;
    };
    let mut files = Vec::new();
    collect(&dir, &mut files, 0);
    files.sort();
    if files.is_empty() {
        eprintln!("installed_movies: no .mp4 under {}", dir.display());
    }
    for path in files {
        let file = std::fs::File::open(&path).unwrap();
        match Mp4Source::open(Box::new(file)) {
            Err(MediaError::Unsupported(why))
                if why.starts_with("mp4 video codec") || why.starts_with("mp4 audio codec") =>
            {
                println!("{}: unsupported ({why})", path.display())
            }
            Err(e) => panic!("{}: {e}", path.display()),
            Ok(mut src) => {
                let expect = src.info().video.as_ref().and_then(|v| v.frame_count);
                let (mut frames, mut samples) = (0u64, 0u64);
                while let Some(e) = src
                    .next_event()
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                {
                    match e {
                        MediaEvent::Video(_) => frames += 1,
                        MediaEvent::Audio(c) => samples += c.frames() as u64,
                    }
                }
                println!(
                    "{}: decoded {frames} frames, {samples} audio frames",
                    path.display()
                );
                assert_eq!(Some(frames), expect, "{}", path.display());
            }
        }
    }
}

fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, depth: u32) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 4 {
            collect(&p, out, depth + 1);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")) {
            out.push(p);
        }
    }
}
