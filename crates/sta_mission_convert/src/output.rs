//! Where the command line writes and what it deletes (LUA-15f).
//!
//! An [`OutDir`] is the output folder: `<out>/missions/<stem>.lua`, and a manifest
//! `<out>/.sta_mission_convert.manifest` of the files this tool wrote (a SHA-256 of the content
//! and the name). The rules, all checked on resolved paths (see [`crate::paths`]):
//!
//! - the tool deletes **only** files the manifest lists, only while they are still regular files
//!   inside `<out>/missions` with exactly the content it wrote (a changed file is the player's:
//!   foreign); a full run deletes those before it converts, so a killed full run loses the earlier
//!   set;
//! - it refuses an output folder that holds `.lua` files in `missions/` that it did not write (or
//!   that changed since), unless `--keep-foreign` says to go on and leave them alone; a file it
//!   would have to overwrite that is foreign is a failed mission, never overwritten; a `.lua` file
//!   whose first line is the Python tool's header (`-- <mission>: converted by tools/<x>_to_lua.py
//!   (LUA-nn) from ...`) is an earlier conversion by the Python wrapper: no hash says whether the
//!   player edited it, so it is **not deleted** but renamed in place to `<name>.old` (`.old2`, ...;
//!   a name that is never a mission script and never overwrites anything), by a full run, or by a
//!   write that would replace it;
//! - it refuses a symbolic link as the output folder, as `missions/`, as the manifest or as a file
//!   it would replace or delete (such a file is not ours);
//! - every write goes through [`OutDir::write`], which re-checks the folder right before it,
//!   writes a temporary file in the same folder (created exclusively) and renames it into place, so
//!   an interrupted run leaves no half file under a name the game or the manifest trusts;
//! - the manifest is updated before the file is renamed (the old and the new hash are both listed
//!   until the run ends), so a file written by a killed run is still recognised as ours.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The manifest's file name, in the output folder.
pub const MANIFEST: &str = ".sta_mission_convert.manifest";
const MANIFEST_HEAD: &str = "sta_mission_convert-manifest 1";

fn sha_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `.<safe name>.tmp-<pid>-<n>`: the name of a temporary file this tool makes ([`OutDir::atomic`]).
fn is_own_temp(n: &str) -> bool {
    let Some(rest) = n.strip_prefix('.') else { return false };
    let Some((name, tail)) = rest.rsplit_once(".tmp-") else { return false };
    let mut nums = tail.split('-');
    let digits = |x: Option<&str>| x.is_some_and(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()));
    safe_name(name) && digits(nums.next()) && digits(nums.next()) && nums.next().is_none()
}

/// Whether `bytes` start with the header line the Python converters write:
/// `-- <mission>: converted by tools/<x>_to_lua.py (LUA-nn) from <file>...`.
fn python_header(bytes: &[u8]) -> bool {
    let end = bytes.iter().take(2048).position(|&b| b == b'\n').unwrap_or(bytes.len().min(2048));
    let line = String::from_utf8_lossy(&bytes[..end]);
    line.starts_with("-- ")
        && line.contains(": converted by tools/")
        && line.contains("_to_lua.py (LUA-")
        && line.contains(") from ")
}

/// Why a file could not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The target exists and is not this tool's: that mission fails, the run goes on.
    Collision(String),
    /// Anything else: the run stops.
    Fatal(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::Collision(m) | Self::Fatal(m)) = self;
        f.write_str(m)
    }
}

impl From<String> for WriteError {
    fn from(m: String) -> Self {
        Self::Fatal(m)
    }
}

/// A file name the tool may create: one component, no separators, no drive or stream syntax.
fn safe_name(n: &str) -> bool {
    !n.is_empty()
        && n != "."
        && n != ".."
        && !n.contains(['/', '\\', ':', '\0'])
        && n.len() <= 200
}

/// The output folder with its manifest.
#[derive(Debug)]
pub struct OutDir {
    /// The resolved folder.
    root: PathBuf,
    /// `(hash, name)` of the files the tool wrote: `name` is a file in `missions/`.
    manifest: Vec<(String, String)>,
    keep_foreign: bool,
    /// Files of the Python tool moved aside in this run, `(old name, new name)`.
    moved: Vec<(String, String)>,
}

impl OutDir {
    fn missions(&self) -> PathBuf {
        self.root.join("missions")
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join(MANIFEST)
    }

    /// Looks at the folder (`root` resolved, `given` as the user wrote it) and refuses what the
    /// rules above refuse. Writes and deletes nothing.
    pub fn inspect(given: &Path, root: PathBuf, keep_foreign: bool) -> Result<OutDir, String> {
        let shown = given.display();
        if let Ok(abs) = std::path::absolute(given)
            && fs::symlink_metadata(&abs).is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(format!("output folder {shown} is a symbolic link: the tool does not write through links"));
        }
        let mut me = OutDir { root, manifest: Vec::new(), keep_foreign, moved: Vec::new() };
        match fs::symlink_metadata(&me.root) {
            Ok(m) if !m.is_dir() => return Err(format!("output folder {shown} exists and is not a folder")),
            Ok(_) => {}
            Err(_) => return Ok(me),
        }
        me.read_manifest(&shown.to_string())?;
        let mdir = me.missions();
        match fs::symlink_metadata(&mdir) {
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                return Err(format!("{}/missions is a symbolic link or not a folder: refused", shown));
            }
            Ok(_) => {}
            Err(_) => return Ok(me),
        }
        let foreign = me.foreign_files()?;
        if !foreign.is_empty() && !keep_foreign {
            return Err(format!(
                "output folder {shown} holds {} .lua file(s) in missions/ that this tool did not write (or that changed \
                 since it wrote them); it will not delete them. Use another folder, remove them yourself, or pass \
                 --keep-foreign to convert into this folder and leave them untouched (a mission whose file would \
                 collide with one of them is then an error)",
                foreign.len()
            ));
        }
        Ok(me)
    }

    fn read_manifest(&mut self, shown: &str) -> Result<(), String> {
        let p = self.manifest_path();
        match fs::symlink_metadata(&p) {
            Err(_) => return Ok(()),
            Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
                return Err(format!("{shown}/{MANIFEST} is a symbolic link or not a file: refused"));
            }
            Ok(_) => {}
        }
        let text = fs::read(&p).map_err(|e| format!("{shown}/{MANIFEST}: {e}"))?;
        let text = String::from_utf8_lossy(&text).replace('\r', "");
        let bad = || {
            format!(
                "{shown}/{MANIFEST} is damaged: the tool cannot tell which files it wrote. Remove the folder's \
                 missions/ files and the manifest yourself, or use another folder"
            )
        };
        let mut lines = text.lines();
        if lines.next() != Some(MANIFEST_HEAD) {
            return Err(bad());
        }
        for l in lines {
            let (h, n) = l.split_once("  ").ok_or_else(bad)?;
            if !is_hash(h) || !safe_name(n) {
                return Err(bad());
            }
            self.manifest.push((h.to_owned(), n.to_owned()));
        }
        Ok(())
    }

    /// Whether the regular file `name` in `missions/` has content the manifest lists for it.
    fn is_ours(&self, name: &str, path: &Path) -> bool {
        let Ok(m) = fs::symlink_metadata(path) else { return false };
        if !m.is_file() {
            return false;
        }
        let Ok(bytes) = fs::read(path) else { return false };
        let h = sha_hex(&bytes);
        self.manifest.iter().any(|(mh, mn)| *mh == h && mn == name)
    }

    /// Whether `path` is a regular file whose first line is the Python tool's header: an earlier
    /// conversion by it (maybe edited since: there is no hash to tell), moved aside, never deleted.
    fn is_python_output(&self, path: &Path) -> bool {
        let Ok(m) = fs::symlink_metadata(path) else { return false };
        if !m.is_file() {
            return false;
        }
        fs::read(path).is_ok_and(|b| python_header(&b))
    }

    /// Renames `missions/<name>` to a name the tool never writes and the game never lists
    /// (`<name>.old`, `<name>.old2`, ...): nothing is overwritten.
    fn move_aside(&mut self, name: &str) -> Result<(), String> {
        let mdir = self.missions();
        for k in 1.. {
            let new = if k == 1 { format!("{name}.old") } else { format!("{name}.old{k}") };
            let dest = mdir.join(&new);
            if fs::symlink_metadata(&dest).is_ok() {
                continue;
            }
            // reserve the name exclusively, then rename over our own empty reservation
            if fs::OpenOptions::new().write(true).create_new(true).open(&dest).is_err() {
                continue;
            }
            self.check()?;
            if let Err(e) = fs::rename(mdir.join(name), &dest) {
                let _ = fs::remove_file(&dest);
                return Err(format!("{}: {e}", mdir.join(name).display()));
            }
            self.moved.push((name.to_owned(), new));
            return Ok(());
        }
        unreachable!("an unused name exists")
    }

    /// The files moved aside since the last call: `(old name, new name)`.
    pub fn take_moved(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.moved)
    }

    /// How many of the files in `missions/` are this tool's (what a summary may count).
    pub fn count_ours(&self) -> usize {
        let Ok(rd) = fs::read_dir(self.missions()) else { return 0 };
        rd.flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.ends_with(".lua") && self.is_ours(&n, &e.path())
            })
            .count()
    }

    /// The `.lua` entries of `missions/` that are not ours (links and folders included).
    fn foreign_files(&self) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        let rd = fs::read_dir(self.missions()).map_err(|e| format!("{}: {e}", self.missions().display()))?;
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.to_lowercase().ends_with(".lua") && !self.is_ours(&n, &e.path()) && !self.is_python_output(&e.path()) {
                out.push(n);
            }
        }
        out.sort();
        Ok(out)
    }

    /// Re-checks that the folder is still the resolved one with no link in the way, right before
    /// a write or a delete.
    fn check(&self) -> Result<(), String> {
        let again = crate::paths::resolve(&self.root)?;
        if again != self.root {
            return Err(format!("output folder {} changed while the tool ran: refused", self.root.display()));
        }
        if let Ok(m) = fs::symlink_metadata(self.missions())
            && (m.file_type().is_symlink() || !m.is_dir())
        {
            return Err(format!("{}/missions is a symbolic link or not a folder: refused", self.root.display()));
        }
        Ok(())
    }

    fn save_manifest(&self) -> Result<(), String> {
        let mut lines: Vec<String> = self.manifest.iter().map(|(h, n)| format!("{h}  {n}")).collect();
        lines.sort();
        lines.dedup();
        let text = format!("{MANIFEST_HEAD}\n{}\n", lines.join("\n")).replace("\n\n", "\n");
        self.atomic(&self.manifest_path(), text.as_bytes()).map_err(|e| format!("{}: {e}", self.manifest_path().display()))
    }

    /// Writes `bytes` to `target` through a temporary file in its folder and a rename.
    fn atomic(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write;
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = target.parent().expect("a target has a folder");
        let name = target.file_name().expect("a target has a name").to_string_lossy().into_owned();
        let tmp = dir.join(format!(
            ".{name}.tmp-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let r = f.write_all(bytes).and_then(|()| f.flush()).and_then(|()| {
            drop(f);
            fs::rename(&tmp, target)
        });
        if r.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        r
    }

    /// A full run: removes the files the manifest lists (regular files with the content written),
    /// and the leftover temporary files of a killed run; then the manifest lists what remains.
    pub fn wipe(&mut self) -> Result<(), String> {
        self.check()?;
        let mdir = self.missions();
        if mdir.is_dir() {
            if let Ok(rd) = fs::read_dir(&mdir) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if is_own_temp(&n)
                        && e.file_type().is_ok_and(|t| t.is_file())
                        && e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 3600)
                    {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
            // the manifest's files go; the `.lua` files of the Python tool's earlier conversion, which
            // may have been edited since, are moved aside
            let mut names: BTreeSet<String> = self.manifest.iter().map(|(_, n)| n.clone()).collect();
            if let Ok(rd) = fs::read_dir(&mdir) {
                names.extend(
                    rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.to_lowercase().ends_with(".lua")),
                );
            }
            for n in names {
                let p = mdir.join(&n);
                if self.is_ours(&n, &p) {
                    self.check()?;
                    fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                } else if n.to_lowercase().ends_with(".lua") && self.is_python_output(&p) {
                    self.move_aside(&n)?;
                }
            }
        }
        self.manifest.clear();
        if self.manifest_path().exists() {
            self.save_manifest()?;
        }
        Ok(())
    }

    /// Writes `missions/<name>` atomically (see the module documentation).
    pub fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), WriteError> {
        if !safe_name(name) {
            return Err(WriteError::Fatal(format!("refusing to write a file named {name:?}")));
        }
        // the folder is checked before it is created, and after
        self.check()?;
        fs::create_dir_all(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        self.check()?;
        let mdir = self.missions();
        if fs::symlink_metadata(&mdir).is_err() {
            fs::create_dir(&mdir).map_err(|e| format!("{}: {e}", mdir.display()))?;
        }
        self.check()?;
        let target = mdir.join(name);
        match fs::symlink_metadata(&target) {
            Err(_) => {}
            Ok(_) if self.is_ours(name, &target) => {}
            Ok(_) if self.is_python_output(&target) => self.move_aside(name)?,
            Ok(_) => {
                return Err(WriteError::Collision(format!(
                    "{} exists and this tool did not write it (or it changed since): refusing to overwrite it",
                    target.display()
                )));
            }
        }
        let h = sha_hex(bytes);
        // listed before the file appears: a killed run's file is still recognised as ours
        if !self.manifest.iter().any(|(mh, mn)| *mh == h && mn == name) {
            self.manifest.push((h.clone(), name.to_owned()));
            self.save_manifest()?;
        }
        self.check()?;
        self.atomic(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
        // the old content of this name is no longer ours to find
        self.manifest.retain(|(mh, mn)| mn != name || *mh == h);
        Ok(())
    }

    /// Ends the run: the manifest lists exactly the files written (and kept).
    pub fn finish(&self) -> Result<(), String> {
        if self.manifest.is_empty() && !self.manifest_path().exists() {
            return Ok(());
        }
        self.check()?;
        self.save_manifest()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn keep_foreign(&self) -> bool {
        self.keep_foreign
    }
}
