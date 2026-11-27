//! Thin file helpers over the byte functions (ADR 0010 V3): atomic writes,
//! reading, and a listing that reads headers only. The directory is always
//! the caller's (the client passes `saves/` in the user data directory); this
//! module has no directory policy. Paths are joined by component and the
//! format is binary, so nothing here assumes a path separator or a line
//! ending.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sta_content::ContentDb;

use super::header::Header;
use super::{SaveError, header_len_hint, read_header};

/// File name extension of a save (without the dot).
pub const EXTENSION: &str = "stasave";
const TEMP_SUFFIX: &str = ".tmp";
/// Larger files are refused as damaged without being read.
const MAX_FILE: u64 = 1 << 30;

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

/// Writes `bytes` as `dir/file_name` atomically: into a temporary file in the
/// same folder, flushed to disk, then renamed over the target. A write that
/// is interrupted leaves an earlier file of that name whole, and a leftover
/// temporary file, which a listing skips. Creates `dir` if it is missing.
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
    check_name(file_name)?;
    std::fs::create_dir_all(dir).map_err(|e| io_err("cannot create the saves folder", &e))?;
    let target = dir.join(file_name);
    let temp = dir.join(format!("{file_name}{TEMP_SUFFIX}"));
    let written = (|| {
        let mut f = std::fs::File::create(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        rename(&temp, &target)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(io_err("cannot write the save", &e));
    }
    Ok(target)
}

/// Writes a new save under a generated name ([`generated_name`]), never over
/// an existing file: a clash gets `-2`, `-3`, ... before the extension.
pub fn write_new(dir: &Path, bytes: &[u8]) -> Result<PathBuf, SaveError> {
    let name = generated_name(&read_header(bytes)?);
    let stem = name.strip_suffix(&format!(".{EXTENSION}")).unwrap_or(&name).to_owned();
    let mut candidate = name;
    let mut n = 1u32;
    while dir.join(&candidate).exists() {
        n += 1;
        candidate = format!("{stem}-{n}.{EXTENSION}");
    }
    write_atomic(dir, &candidate, bytes)
}

/// The bytes of a save file. A file over 1 GiB is refused as damaged.
pub fn read_file(path: &Path) -> Result<Vec<u8>, SaveError> {
    let len = std::fs::metadata(path).map_err(|e| io_err("cannot read the save", &e))?.len();
    if len > MAX_FILE {
        return Err(SaveError::Damaged("the file is far larger than any save".into()));
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
/// missing folder is an empty list.
pub fn list(dir: &Path, content: Option<&ContentDb>) -> Result<Vec<Listing>, SaveError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err("cannot read the saves folder", &e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io_err("cannot read the saves folder", &e))?;
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
