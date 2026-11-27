//! Thin file helpers over the byte functions (ADR 0010 V3): atomic writes,
//! reading, and a listing that reads headers only. The directory is always
//! the caller's (the client passes `saves/` in the user data directory); this
//! module has no directory policy. Paths are joined by component and the
//! format is binary, so nothing here assumes a path separator or a line
//! ending.
//!
//! **Durability, plainly.** A write goes to a temporary file in the same
//! folder (a name of its own per write, so concurrent writers do not touch
//! each other's), the file is flushed to disk (`sync_all`) before it is
//! renamed over its target, and on Unix the folder is then synced too
//! (best effort). On Windows the folder is not synced. After a power loss
//! the new name may be missing, in which case the earlier save is still
//! whole; a half-written file is never under a save's name.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sta_content::ContentDb;

use super::header::Header;
use super::{MAX_FILE, SaveError, header_len_hint, read_header};

/// File name extension of a save (without the dot).
pub const EXTENSION: &str = "stasave";
const TEMP_SUFFIX: &str = ".tmp";

fn io_err(what: &str, e: &io::Error) -> SaveError {
    SaveError::Io(format!("{what}: {e}"))
}

/// A generated file name for a save, from what it says about itself: made-at
/// time and tick, no description and no character a path treats specially.
pub fn generated_name(header: &Header) -> String {
    format!("save-{:012}-t{}.{EXTENSION}", header.made_at, header.tick)
}

fn check_name(name: &str) -> Result<(), SaveError> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.ends_with(&format!(".{EXTENSION}"))
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(SaveError::Invalid(format!("\"{name}\" is not a save file name (letters, digits, - _ . and the .{EXTENSION} extension)")))
    }
}

/// A temporary name no other write has: the process and a counter.
fn temp_path(dir: &Path, file_name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    dir.join(format!(".{file_name}.{}-{}{TEMP_SUFFIX}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

/// Writes the temporary file, flushed to disk.
fn write_temp(dir: &Path, file_name: &str, bytes: &[u8]) -> Result<PathBuf, SaveError> {
    check_name(file_name)?;
    std::fs::create_dir_all(dir).map_err(|e| io_err("cannot create the saves folder", &e))?;
    let temp = temp_path(dir, file_name);
    let written = (|| {
        let mut f = std::fs::File::create(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    match written {
        Ok(()) => Ok(temp),
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(io_err("cannot write the save", &e))
        }
    }
}

/// Best-effort sync of the folder after a rename (Unix; nothing on Windows).
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Writes `bytes` as `dir/file_name` atomically: into a temporary file in the
/// same folder, flushed to disk, then renamed over the target. A write that
/// is interrupted leaves an earlier file of that name whole, and perhaps a
/// leftover temporary file, which a listing skips. Creates `dir` if it is
/// missing. A second writer of the same name wins or loses as a whole.
pub fn write_atomic(dir: &Path, file_name: &str, bytes: &[u8]) -> Result<PathBuf, SaveError> {
    write_atomic_with(dir, file_name, bytes, &|from, to| std::fs::rename(from, to))
}

/// As [`write_atomic`], with the final rename supplied: the seam a test uses
/// to interrupt the write at its last step.
pub fn write_atomic_with(
    dir: &Path,
    file_name: &str,
    bytes: &[u8],
    rename: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<PathBuf, SaveError> {
    let temp = write_temp(dir, file_name, bytes)?;
    let target = dir.join(file_name);
    if let Err(e) = rename(&temp, &target) {
        let _ = std::fs::remove_file(&temp);
        return Err(io_err("cannot write the save", &e));
    }
    sync_dir(dir);
    Ok(target)
}

/// Writes a new save under a generated name ([`generated_name`]), never over
/// an existing file, also against concurrent writers: the finished temporary
/// file is hard-linked to the free name (which fails if the name is taken),
/// then the temporary name is removed. A clash gets `-2`, `-3`, ... before
/// the extension. Where the folder cannot hard-link (some network and FAT
/// volumes) the final name is first reserved with an exclusive create (atomic
/// on every platform, and it fails if the name exists), and the finished
/// temporary file is then renamed over that reservation, which is our own
/// empty file. The guarantee is the same on both paths: no existing file is
/// overwritten and the returned path is the file that was written, also
/// against concurrent writers. On the fallback path the reserved name is an
/// empty file for the moment before the rename (a listing may show it as not
/// a save), and a crash in that moment leaves it behind.
pub fn write_new(dir: &Path, bytes: &[u8]) -> Result<PathBuf, SaveError> {
    write_new_with(dir, bytes, &|from, to| std::fs::hard_link(from, to))
}

/// As [`write_new`], with the hard-link step supplied: the seam a test uses
/// to force the fallback (a link step that fails with anything but
/// `AlreadyExists` selects it).
pub fn write_new_with(dir: &Path, bytes: &[u8], link: &dyn Fn(&Path, &Path) -> io::Result<()>) -> Result<PathBuf, SaveError> {
    let name = generated_name(&read_header(bytes)?);
    let stem = name.strip_suffix(&format!(".{EXTENSION}")).unwrap_or(&name).to_owned();
    let temp = write_temp(dir, &name, bytes)?;
    let mut candidate = name;
    let mut n = 1u32;
    let mut use_link = true;
    loop {
        let target = dir.join(&candidate);
        if use_link {
            match link(&temp, &target) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&temp);
                    sync_dir(dir);
                    return Ok(target);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                // No hard links here: reserve names exclusively from now on.
                Err(_) => {
                    use_link = false;
                    continue;
                }
            }
        } else {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
                Ok(reserved) => {
                    drop(reserved);
                    if let Err(e) = std::fs::rename(&temp, &target) {
                        let _ = std::fs::remove_file(&target);
                        let _ = std::fs::remove_file(&temp);
                        return Err(io_err("cannot write the save", &e));
                    }
                    sync_dir(dir);
                    return Ok(target);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => {
                    let _ = std::fs::remove_file(&temp);
                    return Err(io_err("cannot write the save", &e));
                }
            }
        }
        n += 1;
        candidate = format!("{stem}-{n}.{EXTENSION}");
    }
}

/// The bytes of a save file. A file over [`MAX_FILE`] is refused as damaged
/// without being read.
pub fn read_file(path: &Path) -> Result<Vec<u8>, SaveError> {
    let len = std::fs::metadata(path).map_err(|e| io_err("cannot read the save", &e))?.len();
    if len > MAX_FILE as u64 {
        return Err(SaveError::Damaged("the file is larger than any save".into()));
    }
    std::fs::read(path).map_err(|e| io_err("cannot read the save", &e))
}

/// One file of a listing.
#[derive(Debug)]
pub struct Listing {
    pub file_name: String,
    pub path: PathBuf,
    /// The header, or why the file cannot be loaded here (the load list shows
    /// the refused ones greyed with this reason, never hides them). From the
    /// header alone: a file whose later bytes are damaged, or whose map has
    /// changed, is refused by [`super::load`].
    pub outcome: Result<Header, SaveError>,
}

/// Lists the saves in `dir` by file name, reading only the start of each
/// file. Temporary files of an interrupted write and files of other kinds
/// are skipped; files this build cannot load are listed with their reason.
/// `content`, when known, makes a save made on other content refused. A
/// missing folder is an empty list. A directory entry that cannot be read
/// does not abort the listing: it is listed with [`SaveError::Io`] under the
/// name `?` (its own name is not known).
pub fn list(dir: &Path, content: Option<&ContentDb>) -> Result<Vec<Listing>, SaveError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err("cannot read the saves folder", &e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                out.push(Listing { file_name: "?".into(), path: dir.to_path_buf(), outcome: Err(io_err("cannot read a folder entry", &e)) });
                continue;
            }
        };
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        if file_name.ends_with(TEMP_SUFFIX) || file_name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) || !path.is_file() {
            continue;
        }
        let outcome = read_prefix(&path).and_then(|prefix| {
            let header = read_header(&prefix)?;
            match header.refusal(content) {
                Some(why) => Err(why),
                None => Ok(header),
            }
        });
        out.push(Listing { file_name, path, outcome });
    }
    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(out)
}

fn read_prefix(path: &Path) -> Result<Vec<u8>, SaveError> {
    let f = std::fs::File::open(path).map_err(|e| io_err("cannot read the save", &e))?;
    let mut prefix = Vec::new();
    f.take(header_len_hint() as u64).read_to_end(&mut prefix).map_err(|e| io_err("cannot read the save", &e))?;
    Ok(prefix)
}
