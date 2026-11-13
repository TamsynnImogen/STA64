//! `sta-bench desync <a.star> <b.star>` (NET-06): where two peers' games
//! first part. Re-simulates both desync replays (light crates only, no
//! `sta64`) and prints the first differing tick, the sections and the
//! entity fields that differ there, and the commands before it
//! (`sta_net::diagnose`; docs/NET.md, section 7).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_content::{ContentDb, Races};
use sta_net::diagnose::{self, DiagnoseError, Options, Side};
use sta_sim::Sim;
use sta_sim::replay::{Replay, StartRec};

use super::{Fail, Opts, Outcome};
use crate::capture;

/// The sidecar `<replay>.desync.txt` the peer wrote beside a replay, if there is one.
fn sidecar(replay: &Path) -> Option<String> {
    let mut name = replay.file_name()?.to_os_string();
    name.push(".desync.txt");
    std::fs::read_to_string(replay.with_file_name(name)).ok()
}

fn load(path: &Path) -> Result<(String, Replay, Option<String>), DiagnoseError> {
    let label = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let bytes = std::fs::read(path).map_err(|e| DiagnoseError::Replay { label: label.clone(), why: e.to_string() })?;
    let replay = diagnose::read_replay(&label, &bytes)?;
    Ok((label, replay, sidecar(path)))
}

/// The content a replay ran on: the install (`--install`), or the bench's fake
/// content (`--fake`) when its hash matches the replays'.
fn content(o: &Opts, want: u64) -> Result<(Arc<ContentDb>, Races), Fail> {
    if o.fake {
        for db in [crate::run::fake_content(), sta_content::fake::db()] {
            if db.hash() == want {
                return Ok((Arc::new(db), Races::default()));
            }
        }
        return Err(Fail::Error(format!("--fake: neither fake content has the replays' content hash {want:#018x}")));
    }
    let Some(install) = &o.install else {
        return Err(Fail::Error("the replays record no install: pass --install <root> [--mod <name>], or --fake for replays of the bench's fake content".into()));
    };
    let (db, races) = capture::load_install(install, o.mod_name.as_deref()).map_err(|e| Fail::Error(e.to_string()))?;
    Ok((db, races))
}

fn side(label: String, replay: Replay, sidecar: Option<String>, content: &Arc<ContentDb>, races: &Races) -> Result<Side, DiagnoseError> {
    let setup = replay.setup(content.clone()).map_err(|e| DiagnoseError::Replay { label: label.clone(), why: format!("{e:?}") })?;
    let sim = match replay.starts {
        StartRec::Bare => Sim::from_setup(&setup),
        StartRec::Races { .. } => Sim::from_setup_with_starts(&setup, races).0,
    };
    Ok(Side { label, replay, sim, sidecar })
}

pub(crate) fn desync(o: &Opts) -> Outcome {
    let [a, b] = match o.rest.as_slice() {
        [a, b] => [PathBuf::from(a), PathBuf::from(b)],
        _ => return Err(Fail::Usage("desync: name the two peers' replays: `desync <a.star> <b.star>`".into())),
    };
    let mut opts = Options::default();
    if let Some(n) = o.history {
        opts.history_turns = n;
    }
    if let Some(n) = o.max_rows {
        opts.max_rows = n;
    }
    opts.ticks_per_turn = o.turn_ticks;
    // Anything that stops the comparison from happening is exit 3, so 1 always means "diverged".
    let cannot = |e: String| {
        eprintln!("ERROR {e}");
        Ok(3)
    };
    let (la, ra, sa) = match load(&a) {
        Ok(x) => x,
        Err(e) => return cannot(e.to_string()),
    };
    let (lb, rb, sb) = match load(&b) {
        Ok(x) => x,
        Err(e) => return cannot(e.to_string()),
    };
    // Different content on the two sides is itself the finding, but it cannot be re-run: say so.
    if ra.content_hash != rb.content_hash {
        return cannot(format!(
            "the replays ran on different content ({:#018x} and {:#018x}): that is the desync, there is no single install to re-run both on",
            ra.content_hash, rb.content_hash
        ));
    }
    let (db, races) = match content(o, ra.content_hash) {
        Ok(x) => x,
        Err(Fail::Error(e)) => return cannot(e),
        Err(f) => return Err(f),
    };
    let sides = side(la, ra, sa, &db, &races).and_then(|x| side(lb, rb, sb, &db, &races).map(|y| (x, y)));
    let report = match sides.and_then(|(x, y)| diagnose::diagnose(x, y, &opts)) {
        Ok(r) => r,
        Err(e) => return cannot(e.to_string()),
    };
    if let Some(path) = &o.json {
        let json = report.to_json();
        if path.as_os_str() == "-" {
            println!("{json}");
            return Ok(i32::from(report.diverged()));
        }
        std::fs::write(path, json + "\n").map_err(|e| Fail::Error(format!("{}: {e}", path.display())))?;
    }
    print!("{}", report.to_markdown());
    Ok(i32::from(report.diverged()))
}
