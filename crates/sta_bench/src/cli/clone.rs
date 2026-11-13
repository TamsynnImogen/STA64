//! Private clones of an install for a session (docs/TESTBENCH.md, "Sessions
//! and reports").
//!
//! `sta64` only reads an install, but a session is the agent's to do with as it
//! pleases, so it mounts a clone it owns: two sessions never see each other's
//! files and the user's install is never written through a session.
//!
//! A clone is a directory tree of its own. Each file is a reflink when the
//! file system can (`cp --reflink=always`: instant, copy-on-write, so a write
//! in the clone cannot reach the source), a plain copy when it is small
//! (writable data: config, ODF, saves), and otherwise a symlink to the source,
//! so a multi-gigabyte install does not fill the disk. Symlinked files are
//! the large read-only assets; `session stop` checks that the source is
//! unchanged against the manifest taken at the start and says so when it is
//! not.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Files up to this many bytes are copied when reflinks are not available;
/// bigger ones are symlinked. `STA64_BENCH_CLONE_COPY_MAX` overrides (bytes).
const COPY_MAX: u64 = 4 << 20;

fn copy_max() -> u64 {
    std::env::var("STA64_BENCH_CLONE_COPY_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(COPY_MAX)
}

fn reflink(src: &Path, dst: &Path) -> bool {
    cfg!(unix)
        && Command::new("cp")
            .arg("--reflink=always")
            .arg(src)
            .arg(dst)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

/// What a clone did with each file, for the session's start line.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct CloneStats {
    pub reflinked: usize,
    pub copied: usize,
    pub linked: usize,
}

/// Clone `src` into the new directory `dst`.
pub(crate) fn clone_install(src: &Path, dst: &Path) -> io::Result<CloneStats> {
    clone_with(src, dst, copy_max())
}

fn clone_with(src: &Path, dst: &Path, max: u64) -> io::Result<CloneStats> {
    let mut stats = CloneStats::default();
    // Once a reflink fails the file system cannot do them; stop trying.
    let mut try_reflink = true;
    walk(src, dst, &mut |from, to, len| {
        if try_reflink && len > 0 {
            if reflink(from, to) {
                stats.reflinked += 1;
                return Ok(());
            }
            try_reflink = false;
        }
        if len <= max || !cfg!(unix) {
            fs::copy(from, to)?;
            stats.copied += 1;
        } else {
            #[cfg(unix)]
            std::os::unix::fs::symlink(fs::canonicalize(from)?, to)?;
            stats.linked += 1;
        }
        Ok(())
    })?;
    Ok(stats)
}

fn walk(src: &Path, dst: &Path, file: &mut dyn FnMut(&Path, &Path, u64) -> io::Result<()>) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    let mut entries: Vec<_> = fs::read_dir(src)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let (from, to) = (e.path(), dst.join(e.file_name()));
        let meta = fs::metadata(&from)?; // follows links: a linked file is cloned as a file
        if meta.is_dir() {
            walk(&from, &to, file)?;
        } else {
            file(&from, &to, meta.len())?;
        }
    }
    Ok(())
}

/// `relative path -> (size, modified time in ns)` of every file under `root`.
pub(crate) fn manifest(root: &Path) -> BTreeMap<PathBuf, (u64, u128)> {
    fn rec(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, (u64, u128)>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            let Ok(m) = fs::metadata(&p) else { continue };
            if m.is_dir() {
                rec(root, &p, out);
            } else {
                let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
                out.insert(p.strip_prefix(root).unwrap_or(&p).to_owned(), (m.len(), t));
            }
        }
    }
    let mut out = BTreeMap::new();
    rec(root, root, &mut out);
    out
}

/// The manifest as text, one `size mtime path` line per file.
pub(crate) fn manifest_text(root: &Path) -> String {
    manifest(root).iter().map(|(p, (n, t))| format!("{n} {t} {}\n", p.display())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sta-bench-clone-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        manifest(root).keys().map(|p| (p.clone(), fs::read(root.join(p)).unwrap())).collect()
    }

    fn synthetic(root: &Path) {
        fs::create_dir_all(root.join("data/ODF")).unwrap();
        fs::write(root.join("data/ODF/ship.odf"), "[GameObjectClass]\nclassLabel = craft\n").unwrap();
        fs::write(root.join("RTS_CFG.h"), "// cfg\n").unwrap();
        fs::write(root.join("big.pak"), vec![7u8; 64 << 10]).unwrap();
    }

    #[test]
    fn clones_do_not_see_each_others_writes_and_leave_the_source_alone() {
        let base = tmp("iso");
        let src = base.join("install");
        synthetic(&src);
        let before = tree(&src);
        let manifest_before = manifest_text(&src);

        // A tiny copy threshold so the big file is a symlink on file systems without reflinks.
        let (a, b) = (base.join("a"), base.join("b"));
        clone_with(&src, &a, 1024).unwrap();
        clone_with(&src, &b, 1024).unwrap();

        // Writable data: replaced by a session in a, new file in a.
        fs::write(a.join("RTS_CFG.h"), "// changed by session a\n").unwrap();
        fs::write(a.join("data/ODF/ship.odf"), "changed").unwrap();
        fs::write(a.join("saves.dat"), "a only").unwrap();

        assert_eq!(fs::read_to_string(b.join("RTS_CFG.h")).unwrap(), "// cfg\n");
        assert!(!b.join("saves.dat").exists());
        assert_eq!(fs::read(b.join("big.pak")).unwrap(), vec![7u8; 64 << 10]);
        assert_eq!(tree(&src), before, "source bytes unchanged");
        assert_eq!(manifest_text(&src), manifest_before, "source sizes and mtimes unchanged");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn manifest_notices_a_write_to_the_source() {
        let base = tmp("man");
        let src = base.join("install");
        synthetic(&src);
        let m = manifest_text(&src);
        fs::write(src.join("RTS_CFG.h"), "// longer now\n").unwrap();
        assert_ne!(manifest_text(&src), m);
        let _ = fs::remove_dir_all(base);
    }
}
