//! For the install-gated tests: converts the missions of the player's own install with the Rust
//! converter (`sta_mission_convert`, the library the game itself calls), in this process. No Python
//! and no `objdump`.
//!
//! Converted scripts are game content. They go to a folder under the system's temporary folder
//! that is removed when the [`Conversion`] (or [`TempDir`]) is dropped, also when the test panics;
//! never under the repository or the install, which the converter refuses anyway.
#![allow(dead_code)] // each test file uses its own part

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sta_mission_convert::install::{self, Event, Install, Options, Summary};
use sta_mission_convert::output::OutDir;
use sta_mission_convert::{Pe, disasm};

/// A fresh folder under the system temp folder, removed on drop.
#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("sta64-mission-{tag}-{}-{n}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).expect("temp dir");
        TempDir(p)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a run of the converter gave.
pub struct Run {
    pub summary: Summary,
    /// The lines the converter prints for the missions it converted, in order (the command line
    /// writes the same lines to its console).
    pub console: Vec<String>,
}

impl Run {
    /// The scripts the run wrote: file names in `missions/`, lower case.
    pub fn written(&self) -> BTreeSet<String> {
        self.summary.missions.iter().flat_map(|m| m.written.iter().map(|f| f.to_ascii_lowercase())).collect()
    }

    /// The missions that converted and wrote a script, as the run names them (lower case).
    pub fn converted(&self) -> Vec<String> {
        self.summary.missions.iter().filter(|m| m.converted && !m.written.is_empty()).map(|m| m.mission.to_ascii_lowercase()).collect()
    }
}

/// Converts `only` (every mission when empty) of the install at `install` into the folder `out`
/// (it gets `missions/<name>.lua`), as the game's hook and the command line do. Panics when the
/// install or the folder is refused or the run ends early; a mission that does not convert is in
/// the summary.
pub fn convert_into(install: &Path, out: &Path, only: &[&str], include_unmapped: bool) -> Run {
    let inst = Install::open(install).unwrap_or_else(|e| panic!("the install cannot be converted: {e:?}"));
    let resolved = install::resolve_output(out, &inst, &install::Env::from_process()).expect("the output folder is allowed");
    let mut dir = OutDir::inspect(out, resolved, false).expect("a fresh output folder is usable");
    let opts = Options { only: only.iter().map(|s| (*s).to_owned()).collect(), include_unmapped, ..Options::default() };
    let mut decoder = |_: &Path, pe: &Pe| disasm::disassemble(pe);
    let mut console = Vec::new();
    let summary = install::convert_install(&inst, &opts, &mut dir, None, &mut decoder, &|| false, &mut |e: &Event| {
        if let Event::Console(line) = e {
            console.push(line.clone());
        }
    });
    assert!(summary.aborted.is_none() && summary.finish_error.is_none() && !summary.cancelled, "the conversion ended early");
    Run { summary, console }
}

/// A conversion in a temporary folder of its own.
pub struct Conversion {
    pub run: Run,
    dir: TempDir,
}

impl Conversion {
    /// The folder to mount: it holds `missions/`.
    pub fn dir(&self) -> PathBuf {
        self.dir.path().join("out")
    }
}

/// [`convert_into`] a temporary folder that lives as long as the returned value.
pub fn convert(install: &Path, tag: &str, only: &[&str], include_unmapped: bool) -> Conversion {
    let dir = TempDir::new(tag);
    let run = convert_into(install, &dir.path().join("out"), only, include_unmapped);
    Conversion { run, dir }
}
