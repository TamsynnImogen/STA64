//! Install-gated Bink checks (MED-01). Game movies are never committed; these
//! tests read them from the player's installs and compare against hashes.
//!
//! `STA64_TEST_MOVIE_DIRS` lists folders (separated like `PATH`) searched
//! recursively for `.bik` files, e.g. the A1, A2 and Fleet Operations install
//! roots. Without it every test here passes without checking anything.
//! Run with `--release`: a full pass decodes about 44 000 frames.
//!
//! ```sh
//! STA64_TEST_MOVIE_DIRS="$A1:$A2:$FO" cargo test -p sta_media --release --test bink_install -- --nocapture
//! ```

use sha2::{Digest, Sha256};
use sta_media::{MediaEvent, Movie, Picture, VideoFrame};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn movie_dirs() -> Option<Vec<PathBuf>> {
    let dirs = std::env::var_os("STA64_TEST_MOVIE_DIRS")?;
    Some(
        std::env::split_paths(&dirs)
            .filter(|p| !p.as_os_str().is_empty())
            .collect(),
    )
}

fn find_biks(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            find_biks(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("bik")) {
            out.push(p);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_frame(h: &mut Sha256, f: &VideoFrame) {
    let Picture::Yuv420 { y, u, v, .. } = &f.picture else {
        panic!("Bink frames are planar")
    };
    for p in [y, u, v] {
        for r in 0..p.height {
            h.update(&p.row(r)[..p.width as usize]);
        }
    }
}

fn pcm_bytes(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| ((s * 32768.0).round() as i16).to_le_bytes())
        .collect()
}

/// Hash of the first `frames` frames (cropped yuv420p, as ffmpeg writes it)
/// and of each track's PCM (s16le) decoded up to that point.
fn decode_hashes(movie: &mut Movie, frames: u64) -> (String, Vec<String>) {
    let mut video = Sha256::new();
    let mut audio: Vec<Sha256> = movie.info().audio.iter().map(|_| Sha256::new()).collect();
    let mut seen = 0;
    while seen < frames {
        match movie.next_event().expect("decodes") {
            Some(MediaEvent::Video(f)) => {
                hash_frame(&mut video, &f);
                seen += 1;
            }
            Some(MediaEvent::Audio(c)) => audio[c.track].update(pcm_bytes(&c.samples)),
            None => break,
        }
    }
    (
        hex(&video.finalize()),
        audio.into_iter().map(|h| hex(&h.finalize())).collect(),
    )
}

/// Goldens: file (sha256 prefix of the whole file, name), frames hashed,
/// video hash, per-track audio hashes. The video hashes equal ffmpeg's
/// `-f rawvideo -pix_fmt yuv420p` output for the same frames; the audio
/// hashes are this decoder's (within one LSB of ffmpeg).
const GOLDENS: &[(&str, &str, u64, &str, &[&str])] = &[
    // FO Roots, STA2 Classic mod: BIKi 124x124, no audio.
    (
        "516dcd3d9a7cfd45",
        "klingonsymbol.bik",
        30,
        "6193212c004f918afe6e450c0cdccef37ffb002ec3ad77b9535376f71c8256c9",
        &[],
    ),
    // Armada I: BIKf 320x240, no audio.
    (
        "4416891e84158491",
        "Worf.bik",
        60,
        "a4582162394a1889020a4626d3aa1b4fcdf3aac6a629757fd17f451c951fb4d4",
        &[],
    ),
    // Armada II: BIKi 160x120, RDFT 22 050 Hz mono, 8-bit track.
    (
        "f030891bb89b8e55",
        "a2_fed02_first_shipment.bik",
        26,
        "338e858bbd70ce31ab9a391b9eafd49e8b7d708b5fe9cd1903b1bb8a5f895a4a",
        &["bd5ba74c599cb93b573112cff475eef64e903e3ffa991666f8e1e5706ee7dcfc"],
    ),
    // Armada I: BIKf 160x160, RDFT 44 100 Hz mono 16-bit.
    (
        "2f153f4691ddc648",
        "Gowron.bik",
        193,
        "c413a1238536fb715561212e1204655dcf87b3494961011909adff944f36dbf9",
        &["9cdb05afd5624e48e58ff3c11ce92a2d4bc36398dc1d2e0bf3fd62e591a2b4e8"],
    ),
    // Armada II: BIKi 640x480, RDFT 22 050 Hz stereo.
    (
        "8f3338b7a5aa1b8f",
        "Activision.bik",
        151,
        "a6b0e21636150f6c2cb7ef9a6833856c7c93b6fe5a9bc68bdbd1d3e93069e8de",
        &["cbf0e7c13d843ecc261abaf10cb53687f4c95284e0e1d7eea3b1471e54bcb5ff"],
    ),
    // Armada I: BIKf 320x240, RDFT 44 100 Hz stereo (first 60 frames).
    (
        "fe85b3d4db1470c1",
        "STIntro.bik",
        60,
        "6c33b25c0edf58d5edd0d3f369ec4ae1b779df2f22e8dec133315afe845285f8",
        &["acaccbe2208a138ba2e7e2923a69815d3282e1e5869afb461f4acf44ebf733ba"],
    ),
    // FO Roots, STA2 Classic mod: BIKi 1104x848, DCT 44 100 Hz stereo (first 60 frames).
    (
        "24e284905c614666",
        "Intro.bik",
        60,
        "9f1ffd7bdef4aa754aeab61d0374cb964c25708f401592889be05961950a6342",
        &["25549bf366070c91144acc11833feb428923a68adadc0cbefef8a6b24ac4983f"],
    ),
];

#[test]
fn golden_hashes_of_installed_movies() {
    let Some(dirs) = movie_dirs() else {
        eprintln!("STA64_TEST_MOVIE_DIRS not set; skipping");
        return;
    };
    let mut files = Vec::new();
    for d in &dirs {
        find_biks(d, &mut files);
    }
    let mut checked = 0;
    for &(file_sha, name, frames, video, audio) in GOLDENS {
        let found = files.iter().find_map(|p| {
            if !p.file_name()?.to_str()?.eq_ignore_ascii_case(name) {
                return None;
            }
            let bytes = std::fs::read(p).ok()?;
            hex(&Sha256::digest(&bytes))
                .starts_with(file_sha)
                .then_some(bytes)
        });
        let Some(bytes) = found else {
            eprintln!("{name} ({file_sha}) not installed; skipped");
            continue;
        };
        let mut movie = Movie::open_bytes(bytes).expect("opens");
        let (v, a) = decode_hashes(&mut movie, frames);
        assert_eq!(v, video, "{name}: video hash of {frames} frames");
        assert_eq!(a, audio, "{name}: audio hashes");
        checked += 1;
    }
    eprintln!("{checked}/{} golden movies checked", GOLDENS.len());
    assert!(
        checked > 0,
        "STA64_TEST_MOVIE_DIRS is set but holds none of the golden movies"
    );
}

#[test]
fn every_installed_bik_decodes_to_its_stated_length() {
    let Some(dirs) = movie_dirs() else {
        eprintln!("STA64_TEST_MOVIE_DIRS not set; skipping");
        return;
    };
    let mut files = Vec::new();
    for d in &dirs {
        find_biks(d, &mut files);
    }
    assert!(!files.is_empty(), "no .bik files under {dirs:?}");
    let mut failures = Vec::new();
    let (mut total_frames, mut total_samples) = (0u64, 0u64);
    for path in &files {
        let result = (|| -> Result<String, String> {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let mut movie = Movie::open_bytes(bytes).map_err(|e| e.to_string())?;
            let info = movie.info().clone();
            let stated = info.video.as_ref().and_then(|v| v.frame_count).unwrap_or(0);
            let mut frames = 0u64;
            let mut samples = vec![0u64; info.audio.len()];
            while let Some(e) = movie.next_event().map_err(|e| e.to_string())? {
                match e {
                    MediaEvent::Video(f) => {
                        if f.index != frames {
                            return Err(format!("frame {} out of order at {frames}", f.index));
                        }
                        frames += 1;
                    }
                    MediaEvent::Audio(c) => samples[c.track] += c.frames() as u64,
                }
            }
            if frames != stated {
                return Err(format!("{frames} of {stated} frames"));
            }
            let mut line = format!("{frames} frames");
            for (t, a) in info.audio.iter().enumerate() {
                let want = a.total_frames.unwrap_or(0);
                if samples[t] != want {
                    return Err(format!("track {t}: {} of {want} samples", samples[t]));
                }
                line += &format!(
                    ", track {t} {} Hz x{} {}: {} samples",
                    a.sample_rate, a.channels, a.codec, want
                );
                total_samples += want;
            }
            total_frames += frames;
            Ok(line)
        })();
        match result {
            Ok(line) => eprintln!("ok   {}: {line}", path.display()),
            Err(e) => {
                eprintln!("FAIL {}: {e}", path.display());
                failures.push(format!("{}: {e}", path.display()));
            }
        }
    }
    eprintln!(
        "{} files, {total_frames} frames, {total_samples} audio sample frames, {} failures",
        files.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn seeking_an_installed_movie_matches_sequential_decoding() {
    let Some(dirs) = movie_dirs() else {
        eprintln!("STA64_TEST_MOVIE_DIRS not set; skipping");
        return;
    };
    let mut files = Vec::new();
    for d in &dirs {
        find_biks(d, &mut files);
    }
    // The smallest movie with audio and at least 40 frames.
    let pick = files
        .iter()
        .filter_map(|p| {
            let bytes = std::fs::read(p).ok()?;
            let movie = Movie::open_bytes(bytes.clone()).ok()?;
            let info = movie.info();
            (!info.audio.is_empty() && info.video.as_ref()?.frame_count? >= 40).then_some((
                bytes.len(),
                p.clone(),
                bytes,
            ))
        })
        .min_by_key(|(len, p, _)| (*len, p.clone()));
    let Some((_, path, bytes)) = pick else {
        eprintln!("no movie with audio installed; skipping");
        return;
    };
    eprintln!("seeking in {}", path.display());
    let mut movie = Movie::open_bytes(bytes.clone()).unwrap();
    let mut frames = Vec::new();
    let mut pcm: Vec<Vec<f32>> = vec![Vec::new(); movie.info().audio.len()];
    while let Some(f) = movie.next_video_frame().unwrap() {
        frames.push(f);
        for c in movie.take_audio() {
            pcm[c.track].extend_from_slice(&c.samples);
        }
    }
    for c in movie.take_audio() {
        pcm[c.track].extend_from_slice(&c.samples);
    }
    let fps = movie.info().video.as_ref().unwrap().frame_rate;
    for target in [frames.len() as u64 / 2, 3, frames.len() as u64 - 1, 0] {
        let at = movie
            .seek(fps.periods_to_duration(target) + Duration::from_micros(10))
            .unwrap();
        assert_eq!(at, fps.periods_to_duration(target));
        let f = movie
            .next_video_frame()
            .unwrap()
            .expect("a frame after seeking");
        assert_eq!(f.index, target);
        assert_eq!(
            f.picture, frames[target as usize].picture,
            "frame {target} after seeking"
        );
        let audio = movie.take_audio();
        if let Some(first) = audio.first() {
            let rate = movie.info().audio[first.track].sample_rate;
            let want = sta_media::Rational::new(rate, 1).duration_to_periods(at);
            assert_eq!(first.first_frame, want, "audio resumes at the seek target");
            // Bit-identical to sequential decoding, crossfade included.
            let start = (want * first.channels as u64) as usize;
            let seq = &pcm[first.track][start..start + first.samples.len()];
            assert_eq!(first.samples, seq, "audio after seeking to frame {target}");
        }
    }
    // Run to the end again: end of stream is reported and sticks.
    while movie.next_video_frame().unwrap().is_some() {}
    let _ = movie.take_audio();
    assert!(movie.is_finished());
    assert!(movie.next_event().unwrap().is_none());
}
