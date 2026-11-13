//! `sta-bench capture list|check` (TOOL-14): find playtest captures and
//! replay one headless to its tick, checking the digest.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{Fail, Opts, Outcome};
use crate::capture::{self, Capture, CaptureError, StartMode};

pub(crate) fn capture_cmd(o: &Opts) -> Outcome {
    match o.rest.first().map(String::as_str) {
        Some("list") => list(o),
        Some("check") => check(o),
        _ => Err(Fail::Usage("capture: `list` or `check <folder>`".into())),
    }
}

fn root(o: &Opts) -> PathBuf {
    o.out.clone().unwrap_or_else(capture::default_capture_dir)
}

fn list(o: &Opts) -> Outcome {
    let root = root(o);
    println!("captures in {} (newest first):", root.display());
    for dir in capture::list(&root) {
        match Capture::load(&dir) {
            Ok(c) => println!(
                "  {}  tick {}  {}  {} ({:?}){}",
                dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                c.index.tick,
                c.index.map,
                c.index.profile,
                c.index.start,
                if c.note.trim().is_empty() { String::new() } else { format!("  \"{}\"", c.note.trim().lines().next().unwrap_or("")) }
            ),
            Err(e) => println!("  {}  unreadable: {e}", dir.display()),
        }
    }
    Ok(0)
}

/// A folder named on the command line: a path, or a name (or `latest`) under the capture dir.
fn folder(o: &Opts, arg: &str) -> Result<PathBuf, Fail> {
    let p = Path::new(arg);
    if p.join(capture::INDEX_FILE).is_file() {
        return Ok(p.to_owned());
    }
    let root = root(o);
    if arg == "latest" {
        return capture::list(&root).into_iter().next().ok_or_else(|| Fail::Error(format!("no captures in {}", root.display())));
    }
    let q = root.join(arg);
    if q.join(capture::INDEX_FILE).is_file() { Ok(q) } else { Err(Fail::Error(format!("{arg}: not a capture folder (no {})", capture::INDEX_FILE))) }
}

fn check(o: &Opts) -> Outcome {
    let arg = o.rest.get(1).ok_or_else(|| Fail::Usage("capture check: name a capture folder (or `latest`)".into()))?;
    let dir = folder(o, arg)?;
    let cap = Capture::load(&dir).map_err(|e| Fail::Error(e.to_string()))?;
    let i = &cap.index;
    println!("{}: map {} tick {} digest {} ({:?}, {} commands)", dir.display(), i.map, i.tick, i.digest, i.start, cap.replay.commands.len());
    // Where the content comes from: the flags, else the mount the capture recorded.
    let install = o.install.clone().or_else(|| i.mount.install_root.as_ref().map(PathBuf::from));
    let active = o.mod_name.clone().or_else(|| i.mount.active_mod.clone());
    let (content, races) = if o.fake {
        (Arc::new(crate::run::fake_content()), None)
    } else {
        let Some(install) = install else {
            return Err(Fail::Error("the capture records no install: pass --install <root> [--mod <name>], or --fake for a capture of the bench's fake content".into()));
        };
        let (db, races) = capture::load_install(&install, active.as_deref()).map_err(|e| Fail::Error(e.to_string()))?;
        (db, Some(races))
    };
    // Fake content has no race files: its starts come from the default loadout.
    let races = if i.start == StartMode::Starts { races.or_else(|| Some(Default::default())) } else { None };
    match cap.verify(content, races.as_ref()) {
        Ok(v) => {
            println!("PASS  replayed to tick {} with digest {:#018x}{}", v.tick, v.digest, if v.snapshot_checked { "; snapshot restores to the same digest" } else { "" });
            Ok(0)
        }
        Err(e @ (CaptureError::Replay(sta_sim::replay::ReplayError::Desync { .. }) | CaptureError::SnapshotDigest { .. })) => {
            println!("FAIL  {e}");
            Ok(1)
        }
        // Wrong content, mod, build or a damaged file: the capture was not tested at all.
        Err(e @ (CaptureError::Replay(_) | CaptureError::Snapshot(_) | CaptureError::NeedsRaces)) => {
            println!("ERROR {e}  (not a desync: the capture could not be replayed here)");
            Ok(3)
        }
        Err(e) => Err(Fail::Error(e.to_string())),
    }
}
