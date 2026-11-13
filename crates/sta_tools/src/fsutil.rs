//! File plumbing shared by the command-line tools: finding files, writing
//! them atomically and escaping JSON. Paths are `Path`s throughout (no
//! string splitting on `/`), and file contents are bytes: the tools never
//! change a file's line endings or encoding on their own.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Files below `path` (or `path` itself when it is a file) whose extension is
/// `ext` (ASCII case-insensitive), sorted by name within each folder. Symlinks
/// are skipped and reported in `skipped`: a loop would not end, and a rewrite
/// would replace the link instead of its target.
pub fn collect(path: &Path, ext: &str, out: &mut Vec<PathBuf>, skipped: &mut Vec<PathBuf>) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        skipped.push(path.to_owned());
    } else if meta.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            collect(&e.path(), ext, out, skipped)?;
        }
    } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
        out.push(path.to_owned());
    }
    Ok(())
}

/// Write `bytes` over `target` atomically: a new temp file beside it (never
/// a followed link), the target's permissions, flushed to disk, then renamed.
/// The temp file is removed when anything fails.
pub fn write_atomically(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_owned();
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    name.push(format!(
        ".fmt-tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let tmp = target.with_file_name(name);
    let perms = fs::metadata(target)?.permissions();
    if perms.readonly() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "target is read-only"));
    }
    // Created first, so the cleanup below only ever removes our own file.
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let result = (|| {
        f.write_all(bytes)?;
        f.set_permissions(perms)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// `s` as a JSON string literal.
pub fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("sta_tools-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn collect_sorts_filters_by_extension_and_skips_links() {
        let d = scratch("collect");
        fs::create_dir(d.join("sub")).unwrap();
        fs::write(d.join("b.ODF"), b"").unwrap();
        fs::write(d.join("a.odf"), b"").unwrap();
        fs::write(d.join("c.txt"), b"").unwrap();
        fs::write(d.join("sub").join("d.odf"), b"").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(d.join("a.odf"), d.join("link.odf")).unwrap();
        let (mut out, mut skipped) = (Vec::new(), Vec::new());
        collect(&d, "odf", &mut out, &mut skipped).unwrap();
        let names: Vec<_> = out.iter().map(|p| p.strip_prefix(&d).unwrap().to_owned()).collect();
        assert_eq!(
            names,
            [PathBuf::from("a.odf"), PathBuf::from("b.ODF"), PathBuf::from("sub").join("d.odf")]
        );
        #[cfg(unix)]
        assert_eq!(skipped.len(), 1);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn atomic_write_replaces_bytes_exactly_and_leaves_no_temp_file() {
        let d = scratch("atomic");
        let f = d.join("x.odf");
        fs::write(&f, b"old").unwrap();
        write_atomically(&f, b"a\r\nb\n\xff").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"a\r\nb\n\xff");
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1);
        // A target that does not exist fails and leaves nothing behind.
        assert!(write_atomically(&d.join("missing.odf"), b"x").is_err());
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn json_escapes() {
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
    }
}
