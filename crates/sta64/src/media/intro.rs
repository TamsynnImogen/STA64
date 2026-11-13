//! The movies a game plays when it starts (MED-05).
//!
//! # Where the sequence comes from
//!
//! * **The title movie** is `cfgINTRO_MOVIE_NAME` in `RTS_CFG.h` (Armada II,
//!   Fleet Operations and Dominion War all define it as `"Intro.bik"`), read through the mounted
//!   layers, so a mod's `RTS_CFG.h` can name another movie, or an empty string for none. A header
//!   without the key (Armada I's) means the movie the executable names itself, `STIntro.bik`.
//! * **The logo reels** before it are not in any data file: `Armada2.exe` holds the
//!   three names `Activision.bik`, `Paramount.bik`, `MadDocSoftware.bik` in that order
//!   (the order a real run opens them, see `BinkProxy.log` beside the stock install). That table is the
//!   built-in default; a reel the install does not have is left out, and an
//!   installed file of the same name (a mod's, or an `.mp4` of the same stem) replaces it through the VFS.
//! * **Skippable**: every one, like the original, by Esc (and Space, Enter, a click).
//! * **`--nointro`**: the executables' own `nointro` argument. [`IntroSettings::enabled`] is
//!   the same switch as a setting.
//!
//! The movies are queued all at once as fullscreen [`PlayMovie`] requests, which the
//! player runs in order; the front end loads behind them.

use bevy::prelude::*;
use sta_vfs::Vfs;

use super::{MovieIds, MovieTarget, PlayMovie, locate};
use crate::viewer::Mounted;

/// The logo reels `Armada2.exe` plays before the title movie, in its order.
pub const LOGO_REELS: [&str; 3] = ["Activision.bik", "Paramount.bik", "MadDocSoftware.bik"];
/// The title movie `Armada.exe` (Armada I) names itself; it has no `RTS_CFG.h` key.
pub const FALLBACK_TITLE: &str = "STIntro.bik";
/// The `RTS_CFG.h` key naming the title movie.
pub const TITLE_KEY: &str = "cfgINTRO_MOVIE_NAME";

/// Whether the intro plays. `--nointro` clears it; a settings screen can too.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntroSettings {
    pub enabled: bool,
}

impl Default for IntroSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Whether the install has a movie by this name (any of its candidate paths).
fn exists(vfs: &Vfs, name: &str) -> bool {
    locate::candidates(name).iter().any(|p| vfs.resolve(p).is_some())
}

/// The title movie `RTS_CFG.h` names: `Some("")` for none, `None` when it does not say.
pub fn title_from_cfg(vfs: &Vfs) -> Option<String> {
    let cfg = sta_content::layered::cfg_h(vfs, "RTS_CFG.h")?;
    let raw = cfg.raw(TITLE_KEY)?.trim();
    Some(raw.trim_matches('"').trim().to_owned())
}

/// The intro movies this install plays, in order: logo reels it has, then the title movie.
pub fn sequence(vfs: &Vfs) -> Vec<String> {
    let mut out: Vec<String> = LOGO_REELS.iter().filter(|n| exists(vfs, n)).map(|n| (*n).to_owned()).collect();
    let title = title_from_cfg(vfs).unwrap_or_else(|| FALLBACK_TITLE.to_owned());
    if !title.is_empty() {
        if exists(vfs, &title) {
            out.push(title);
        } else {
            info!("intro: the title movie {title:?} is not in this install");
        }
    }
    out
}

/// Queues the intro at startup unless [`IntroSettings::enabled`] is off.
pub struct IntroPlugin;

impl Plugin for IntroPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<IntroSettings>().add_systems(Startup, queue_intro);
    }
}

fn queue_intro(settings: Res<IntroSettings>, vfs: Option<Res<Mounted>>, mut ids: ResMut<MovieIds>, mut play: MessageWriter<PlayMovie>) {
    if !settings.enabled {
        info!("intro: skipped (--nointro or the setting)");
        return;
    }
    let Some(vfs) = vfs else { return };
    let names = sequence(&vfs.0);
    info!("intro: {}", if names.is_empty() { "no movies in this install".to_owned() } else { names.join(", ") });
    for name in names {
        play.write(PlayMovie { id: ids.next(), name, target: MovieTarget::Fullscreen, skippable: true });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn vfs_with(files: &[(&str, &str)]) -> (std::path::PathBuf, Vfs) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("sta64-intro-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        for (p, text) in files {
            let path = dir.join(p);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.clone()).unwrap();
        (dir, vfs)
    }

    #[test]
    fn logos_then_the_title_the_cfg_names() {
        let (dir, vfs) = vfs_with(&[
            ("RTS_CFG.h", "#define cfgINTRO_MOVIE_NAME \"Opening.bik\"\n"),
            ("animations/Activision.bik", ""),
            ("animations/MadDocSoftware.bik", ""),
            ("animations/opening.bik", ""),
            ("animations/Intro.bik", ""),
        ]);
        // Paramount.bik is missing and left out; the cfg's title wins over Intro.bik.
        assert_eq!(sequence(&vfs), ["Activision.bik", "MadDocSoftware.bik", "Opening.bik"]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_header_without_the_key_means_stintro_and_an_empty_one_means_none() {
        let (dir, vfs) = vfs_with(&[("RTS_CFG.h", "int X = 1;\n"), ("animations/STIntro.bik", "")]);
        assert_eq!(sequence(&vfs), ["STIntro.bik"]);
        fs::remove_dir_all(dir).ok();
        let (dir, vfs) = vfs_with(&[("RTS_CFG.h", "#define cfgINTRO_MOVIE_NAME \"\"\n"), ("animations/Paramount.bik", "")]);
        assert_eq!(sequence(&vfs), ["Paramount.bik"]);
        fs::remove_dir_all(dir).ok();
    }
}
