//! The marks file of a `--sim-only` / `--headless` bench run
//! (docs/TESTBENCH.md, "Marks").
//!
//! Sim marks come from the sim's own events through the same derivation the
//! in-process runner uses (`sta_bench::marks`), so the two agree line for
//! line. Render marks (culled/LOD counts, FX spawn/despawn) have no source in
//! the renderer yet; when one lands it appends its lines here, after the sim
//! marks, as `render.*` kinds.
//!
//! The file is written to `$STA64_BENCH_OUT/marks` when that variable names a
//! directory (the bench report's case directory); otherwise nothing is written.

use std::path::{Path, PathBuf};

use sta_bench::run::Bench;

/// The marks file's text for the run so far.
pub fn text(bench: &Bench) -> String {
    sta_bench::run::marks::render(bench, &bench.marks())
}

/// Where the report wants the marks file, if anywhere.
fn out_dir() -> Option<PathBuf> {
    std::env::var_os("STA64_BENCH_OUT").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

pub fn write_to(bench: &Bench, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("marks"), text(bench))
}

/// Write the marks file if the environment asks for one.
pub fn write_if_requested(bench: &Bench) {
    if let Some(dir) = out_dir()
        && let Err(e) = write_to(bench, &dir)
    {
        eprintln!("marks: {}: {e}", dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_bench::parse::SceneFile;
    use sta_bench::run::RunConfig;
    use std::sync::Arc;

    fn firing() -> Bench {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/scenes/combat-firing.ini");
        let scene = SceneFile::parse(&std::fs::read_to_string(p).unwrap()).unwrap();
        Bench::build(Arc::new(RunConfig::fake()), &scene).unwrap()
    }

    #[test]
    fn text_has_tick_kind_and_named_objects() {
        let mut b = firing();
        while b.tick() < 20 {
            assert!(b.advance());
        }
        let t = text(&b);
        assert!(t.contains("     0 fire.start obj=shooter weapon="), "{t}");
    }

    #[test]
    fn writes_marks_file() {
        let dir = std::env::temp_dir().join(format!("sta64-marks-{}", std::process::id()));
        let mut b = firing();
        while b.tick() < 20 {
            assert!(b.advance());
        }
        write_to(&b, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("marks")).unwrap(), text(&b));
        let _ = std::fs::remove_dir_all(dir);
    }
}
