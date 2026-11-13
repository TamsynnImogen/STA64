//! Finding and opening the movie a name refers to (MED-03).
//!
//! A name is what the game's data gives: `Intro.bik` (`RTS_CFG.h`
//! `cfgINTRO_MOVIE_NAME`), a mission's `a2_fed01_meet_the_borg.bik`, an
//! Armada I `addon\single01.bik`. The original executables look in
//! `animations\` (`animations\%s` in `Armada2.exe`), and the movie set can
//! be replaced file by file through the mounted layers. Following A2FOVideo, a
//! same-stem `.mp4` is tried before the named file, and a `.wav` beside a movie
//! without sound is its soundtrack. A candidate that does not open
//! (`Unsupported`, corrupt) is skipped for the next one, so an AV1 `Intro.mp4`
//! falls back to `Intro.bik`.

use std::sync::Arc;

use sta_media::Movie;
use sta_vfs::Vfs;

/// Directories searched, in order, for a name without one of its own.
const DIRS: [&str; 3] = ["animations/", "", "movie/"];

/// An opened movie, with the audio of a same-stem `.wav` when it has no sound of its own.
pub struct Opened {
    pub movie: Movie,
    /// The VFS path it came from, for logs.
    pub path: String,
    pub sidecar: Option<(u32, Arc<[[f32; 2]]>)>,
}

/// The VFS paths to try for `name`, best first.
pub fn candidates(name: &str) -> Vec<String> {
    let norm = sta_vfs::normalize(name);
    let (dir, file) = match norm.rsplit_once('/') {
        Some((d, f)) => (Some(format!("{d}/")), f.to_owned()),
        None => (None, norm.clone()),
    };
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) => (s.to_owned(), Some(e.to_owned())),
        None => (file.clone(), None),
    };
    let named = if ext.is_some() { file.clone() } else { format!("{stem}.bik") };
    let dirs: Vec<String> = match dir {
        Some(d) => vec![d],
        None => DIRS.iter().map(|d| (*d).to_owned()).collect(),
    };
    let mut out: Vec<String> = Vec::new();
    for d in dirs {
        for f in [format!("{stem}.mp4"), named.clone(), format!("{stem}.bik")] {
            let p = format!("{d}{f}");
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// Open the best candidate for `name`. `Err` is a one-line reason for the log.
pub fn open(vfs: &Vfs, name: &str) -> Result<Opened, String> {
    let mut failures = Vec::new();
    for path in candidates(name) {
        if vfs.resolve(&path).is_none() {
            continue;
        }
        let bytes = match vfs.read(&path) {
            Some(Ok(b)) => b,
            Some(Err(e)) => {
                failures.push(format!("{path}: {e}"));
                continue;
            }
            None => continue,
        };
        match Movie::open_bytes(bytes) {
            Ok(movie) => {
                let sidecar = if movie.info().audio.is_empty() { sidecar(vfs, &path) } else { None };
                return Ok(Opened { movie, path, sidecar });
            }
            Err(e) => failures.push(format!("{path}: {e}")),
        }
    }
    if failures.is_empty() { Err(format!("{name}: no such movie")) } else { Err(failures.join("; ")) }
}

fn sidecar(vfs: &Vfs, movie_path: &str) -> Option<(u32, Arc<[[f32; 2]]>)> {
    let wav = format!("{}.wav", movie_path.rsplit_once('.').map_or(movie_path, |(s, _)| s));
    let bytes = vfs.read(&wav)?.ok()?;
    let a = sta_assets::audio::decode(&bytes).ok()?;
    Some((a.sample_rate, a.frames))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_stem_mp4_comes_first_and_dirs_follow_the_game() {
        let c = candidates("Intro.bik");
        assert_eq!(&c[..3], ["animations/intro.mp4", "animations/intro.bik", "intro.mp4"]);
        assert!(c.contains(&"movie/intro.bik".to_owned()));
        let c = candidates("addon\\Single01.bik");
        assert_eq!(c, ["addon/single01.mp4", "addon/single01.bik"]);
        assert_eq!(candidates("STIntro")[1], "animations/stintro.bik");
    }
}
