//! `sta-archive`: look into, unpack, check and build STA64 `.sta` archives,
//! Fleet Operations `.fpq` (v1: FO 3.x / FPQEdit 3.2.7; v2: FO 4.0 / Roots /
//! FPQEdit 4.0) and Armada I `.zfs` archives, and convert a mod folder into
//! one `.sta`. The format is detected from the file itself.
//!
//! An archive inside an archive is named with `::`, e.g.
//! `Data/ODF.fpq::odf/system/ODF.fpq` (Roots ships one).

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use sta_fpq::write::Compression;
use sta_fpq::Fpq;
use sta_vfs::sta::{Kind, StaArchive, Storage};
use sta_vfs::zfs::{self, Zfs, ZfsFile};

/// `println!` that stops quietly when stdout is closed (`sta-archive info x | head`).
macro_rules! outln {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}

const USAGE: &str = "\
sta-archive: STA64 .sta, Fleet Operations .fpq (v1, v2) and Armada I .zfs archives

usage:
  sta-archive info    <archive>
  sta-archive list    <archive> [-l]
  sta-archive find    <archive> <name>
  sta-archive cat     <archive> <path>
  sta-archive extract <archive> <out-dir> [--force] [pattern...]
  sta-archive verify  <archive>
  sta-archive pack    <dir> <out> --format sta|fpq1|fpq2|zfs [--raw] [--key 0xHEX] [--force]
  sta-archive convert <mod-dir> <out.sta> [--raw] [--force]

<archive> may name an archive inside an archive: outer.fpq::path/inner.fpq
patterns match archive paths case-insensitively; '*' is any run of characters
fpq1 = FO 3.x (FPQEdit 3.2.7), fpq2 = FO 4.0 / Roots (FPQEdit 4.0)
convert packs a mod folder (loose files, its .fpq/.zfs unpacked) into one .sta,
writing manifest.toml from info.ini when the mod has none (docs/STA_ARCHIVE.md)";

enum Archive {
    Fpq(Fpq),
    Zfs(Zfs),
    Sta(StaArchive),
}

struct Entry {
    path: String,
    size: u64,
    stored: u64,
    how: &'static str,
}

impl Archive {
    fn from_bytes(bytes: Vec<u8>) -> Result<Archive, String> {
        if bytes.get(..8) == Some(sta_vfs::sta::MAGIC.as_slice()) {
            return StaArchive::from_bytes(bytes).map(Archive::Sta).map_err(|e| e.to_string());
        }
        match bytes.get(..4) {
            Some(b"FPQ\0") => Fpq::from_bytes(bytes).map(Archive::Fpq).map_err(|e| e.to_string()),
            Some(b"ZFSF") => Zfs::from_bytes(bytes).map(Archive::Zfs).map_err(|e| e.to_string()),
            _ => Err("not a .sta, .fpq or .zfs archive".into()),
        }
    }

    /// `outer::inner::...`: open the outer file, then each nested entry.
    fn open(spec: &str) -> Result<Archive, String> {
        let mut parts = spec.split("::");
        let file = parts.next().unwrap_or_default();
        // A top-level .sta is read on demand, not loaded whole.
        let mut magic = [0u8; 8];
        let is_sta = std::fs::File::open(file).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic)).is_ok()
            && &magic == sta_vfs::sta::MAGIC;
        let mut a = if is_sta {
            Archive::Sta(StaArchive::open(Path::new(file)).map_err(|e| format!("{file}: {e}"))?)
        } else {
            let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            Archive::from_bytes(bytes).map_err(|e| format!("{file}: {e}"))?
        };
        for inner in parts {
            let i = a.index_of(inner).ok_or_else(|| format!("{inner}: not in the archive"))?;
            a = Archive::from_bytes(a.read(i)?).map_err(|e| format!("{inner}: {e}"))?;
        }
        Ok(a)
    }

    fn len(&self) -> usize {
        match self {
            Archive::Fpq(f) => f.len(),
            Archive::Zfs(z) => z.len(),
            Archive::Sta(a) => a.len(),
        }
    }

    fn entry(&self, i: usize) -> Result<Entry, String> {
        Ok(match self {
            Archive::Fpq(f) => {
                let e = f.entry(i);
                Entry {
                    path: f.path(i).map_err(|e| e.to_string())?,
                    size: e.size as u64,
                    stored: e.stored_size as u64,
                    how: if e.compression == 1 { "lzo" } else { "raw" },
                }
            }
            Archive::Zfs(z) => {
                let e = z.entry(i);
                Entry {
                    path: e.name.clone(),
                    size: e.size as u64,
                    stored: e.size as u64,
                    how: match (e.flags, z.key()) {
                        (0, 0) => "raw",
                        (0, _) => "xor",
                        _ => "unknown",
                    },
                }
            }
            Archive::Sta(a) => {
                let e = a.entry(i);
                Entry {
                    path: e.path.clone(),
                    size: e.size,
                    stored: e.stored_size,
                    how: match (e.kind, e.storage) {
                        (Kind::Whiteout, _) => "hide",
                        (_, Storage::Zstd) => "zstd",
                        _ => "raw",
                    },
                }
            }
        })
    }

    fn read(&self, i: usize) -> Result<Vec<u8>, String> {
        match self {
            Archive::Fpq(f) => f.read(i).map_err(|e| e.to_string()),
            Archive::Zfs(z) => z.read(i).map_err(|e| e.to_string()),
            Archive::Sta(a) => a.read(i).map_err(|e| e.to_string()),
        }
    }

    /// By full path, case-insensitively.
    fn index_of(&self, path: &str) -> Option<usize> {
        if let Archive::Sta(a) = self {
            return a.find(path);
        }
        let want = norm(path);
        (0..self.len()).find(|&i| self.entry(i).is_ok_and(|e| norm(&e.path) == want))
    }
}

fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

/// `*` matches any run of characters; case-insensitive; whole path.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (norm(pattern), norm(text));
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == t;
    }
    let mut rest = t.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(part) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(at) => rest = &rest[at + part.len()..],
                None => return false,
            }
        }
    }
    true
}

/// `out` joined with an archive path, refusing anything that could leave `out`.
fn safe_join(out: &Path, path: &str) -> Result<PathBuf, String> {
    let rel = Path::new(&*path.replace('\\', "/")).to_path_buf();
    let mut dest = out.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(s) if !s.to_string_lossy().contains(':') => dest.push(s),
            Component::CurDir => {}
            _ => return Err(format!("{path}: unsafe path, skipped")),
        }
    }
    if dest == out {
        return Err(format!("{path:?}: empty path, skipped"));
    }
    Ok(dest)
}

fn human(n: u64) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KiB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

fn info(spec: &str) -> Result<(), String> {
    let a = Archive::open(spec)?;
    let (mut size, mut stored, mut lzo, mut nested) = (0u64, 0u64, 0usize, 0usize);
    for i in 0..a.len() {
        let e = a.entry(i)?;
        size += e.size;
        stored += e.stored;
        lzo += (e.how == "lzo") as usize;
        let p = e.path.to_ascii_lowercase();
        nested += (p.ends_with(".fpq") || p.ends_with(".zfs")) as usize;
    }
    match &a {
        Archive::Fpq(f) => {
            let tool = if f.version() == 1 { "FO 3.x, FPQEdit 3.2.7" } else { "FO 4.0 / Roots, FPQEdit 4.0" };
            outln!("format:  FPQ v{} ({tool})", f.version());
            outln!("files:   {} ({lzo} LZO, {} raw) in {} folders", f.len(), f.len() - lzo, f.folder_count());
            outln!("hash:    {} slots", f.hash_slots());
        }
        Archive::Zfs(z) => {
            outln!("format:  ZFS v1 (Armada I)");
            outln!("files:   {}", z.len());
            outln!("key:     {:#010x}{}", z.key(), if z.key() == 0 { " (not encrypted)" } else { "" });
        }
        Archive::Sta(a) => {
            let (v, m) = a.version();
            let hidden = a.entries().iter().filter(|e| e.kind == Kind::Whiteout).count();
            let zstd = a.entries().iter().filter(|e| e.storage == Storage::Zstd).count();
            outln!("format:  STA {v}.{m} (STA64)");
            outln!("files:   {} ({zstd} Zstandard, {} stored, {hidden} whiteouts)", a.len() - hidden, a.len() - hidden - zstd);
            outln!("id:      {}", hex(&a.content_hash()));
            outln!("signed:  {}", if a.is_signed() { "yes (not checked: CON-19)" } else { "no" });
            if let Some(i) = a.find("manifest.toml") {
                let m = a.read(i).map_err(|e| e.to_string())?;
                for line in String::from_utf8_lossy(&m).lines().filter(|l| l.starts_with("name") || l.starts_with("version") || l.starts_with("id")) {
                    outln!("  {line}");
                }
            }
        }
    }
    outln!("size:    {} unpacked, {} stored", human(size), human(stored));
    if nested > 0 {
        outln!("nested:  {nested} archive(s) inside; open with <archive>::<path>");
    }
    Ok(())
}

fn list(spec: &str, long: bool) -> Result<(), String> {
    let a = Archive::open(spec)?;
    let mut out = std::io::stdout().lock();
    for i in 0..a.len() {
        let e = a.entry(i)?;
        let line = if long { format!("{:>10} {:>10} {:<4} {}", e.size, e.stored, e.how, e.path) } else { e.path };
        if writeln!(out, "{line}").is_err() {
            break; // closed pipe
        }
    }
    Ok(())
}

fn find(spec: &str, name: &str) -> Result<(), String> {
    let a = Archive::open(spec)?;
    let i = match &a {
        // The game's own lookup: hash of the basename.
        Archive::Fpq(f) => f.find(name.rsplit(['/', '\\']).next().unwrap_or(name)),
        Archive::Zfs(_) => (0..a.len()).find(|&i| a.entry(i).is_ok_and(|e| e.path.eq_ignore_ascii_case(name))),
        // Full path first, then by file name anywhere (as the ODF index finds ODFs).
        Archive::Sta(s) => s.find(name).or_else(|| {
            let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
            s.entries().iter().position(|e| e.key.rsplit('/').next() == Some(base.as_str()))
        }),
    };
    match i {
        Some(i) => {
            let e = a.entry(i)?;
            outln!("{} ({} bytes, {})", e.path, e.size, e.how);
            Ok(())
        }
        None => Err(format!("{name}: not found")),
    }
}

fn cat(spec: &str, path: &str) -> Result<(), String> {
    let a = Archive::open(spec)?;
    let i = a.index_of(path).ok_or_else(|| format!("{path}: not in the archive"))?;
    std::io::stdout().write_all(&a.read(i)?).map_err(|e| e.to_string())
}

fn extract(spec: &str, out: &Path, force: bool, patterns: &[String]) -> Result<(), String> {
    let a = Archive::open(spec)?;
    let (mut n, mut skipped) = (0usize, 0usize);
    for i in 0..a.len() {
        let e = a.entry(i)?;
        if !patterns.is_empty() && !patterns.iter().any(|p| glob(p, &e.path)) {
            continue;
        }
        if e.how == "hide" {
            continue; // a whiteout has no file to write
        }
        let dest = match safe_join(out, &e.path) {
            Ok(d) => d,
            Err(msg) => {
                eprintln!("{msg}");
                skipped += 1;
                continue;
            }
        };
        if dest.exists() && !force {
            eprintln!("{}: exists, skipped (use --force)", dest.display());
            skipped += 1;
            continue;
        }
        let data = a.read(i)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&dest, data).map_err(|e| format!("{}: {e}", dest.display()))?;
        n += 1;
    }
    eprintln!("extracted {n} file(s) to {}{}", out.display(), if skipped > 0 { format!(", {skipped} skipped") } else { String::new() });
    Ok(())
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            walk(&p, root, out)?;
        } else if ft.is_file() {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, p));
        }
    }
    Ok(())
}

fn pack(dir: &Path, out: &Path, format: &str, raw: bool, key: u32, force: bool) -> Result<(), String> {
    if out.exists() && !force {
        return Err(format!("{}: exists (use --force)", out.display()));
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    files.sort_by_key(|(rel, _)| rel.to_ascii_lowercase());
    let bytes = match format {
        "sta" => {
            let items = sta_vfs::sta_write::items_in_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let n = items.len();
            let size = write_sta(out, items, raw)?;
            eprintln!("packed {n} file(s) into {} (sta, {})", out.display(), human(size));
            return Ok(());
        }
        "fpq1" | "fpq2" => {
            let version = if format == "fpq1" { 1 } else { 2 };
            let mut entries = Vec::with_capacity(files.len());
            for (rel, p) in &files {
                entries.push((rel.clone(), std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?));
            }
            let comp = if raw { Compression::None } else { Compression::Auto };
            sta_fpq::write::write(version, &entries, comp).map_err(|e| e.to_string())?
        }
        "zfs" => {
            let mut entries = Vec::with_capacity(files.len());
            for (rel, p) in &files {
                if rel.contains('/') {
                    return Err(format!("{rel}: ZFS archives hold no folders; put the files at the top of {}", dir.display()));
                }
                let time = std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as u32)
                    .unwrap_or(0);
                entries.push(ZfsFile { name: rel.clone(), data: std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?, time });
            }
            zfs::write(&entries, key).map_err(|e| e.to_string())?
        }
        f => return Err(format!("unknown format {f:?} (sta, fpq1, fpq2 or zfs)")),
    };
    write_checked(out, bytes)?;
    eprintln!("packed {} file(s) into {} ({format})", files.len(), out.display());
    Ok(())
}

/// Write next to the target, rename into place (never half an archive), then
/// read every entry back before calling it done.
fn write_checked(out: &Path, bytes: Vec<u8>) -> Result<(), String> {
    let tmp = out.with_extension("sta-archive-tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))?;
    let check = Archive::from_bytes(bytes)?;
    for i in 0..check.len() {
        if check.entry(i)?.how != "hide" {
            check.read(i)?;
        }
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Stream a `.sta` into a temporary file beside `out`, read every file of it
/// back, then rename it into place. Returns the archive's size.
fn write_sta(out: &Path, items: Vec<(String, sta_vfs::sta_write::Item<'_>)>, raw: bool) -> Result<u64, String> {
    let comp = if raw { sta_vfs::sta_write::Compression::None } else { sta_vfs::sta_write::Compression::Auto };
    let tmp = out.with_extension("sta-archive-tmp");
    let result = (|| {
        let f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let mut w = std::io::BufWriter::new(f);
        sta_vfs::sta_write::pack_to(&mut w, items, comp).map_err(|e| e.to_string())?;
        w.into_inner().map_err(|e| e.to_string())?.sync_all().map_err(|e| e.to_string())?;
        let a = StaArchive::open(&tmp).map_err(|e| format!("written archive: {e}"))?;
        for i in 0..a.len() {
            if a.entry(i).kind == Kind::File {
                a.read(i).map_err(|e| format!("written archive: {e}"))?;
            }
        }
        std::fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))?;
        std::fs::metadata(out).map(|m| m.len()).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn convert(dir: &Path, out: &Path, raw: bool, force: bool) -> Result<(), String> {
    if out.exists() && !force {
        return Err(format!("{}: exists (use --force)", out.display()));
    }
    if out.parent() == Some(dir) {
        return Err(format!("{}: write the archive outside the mod folder it is made from", out.display()));
    }
    let m = sta_vfs::convert::ModFolder::open(dir)?;
    let n = m.len();
    let size = write_sta(out, m.items(), raw)?;
    let r = &m.report;
    eprintln!(
        "converted {} into {} ({n} files, {}): {} loose, {} from .sta, {} from .fpq, {} from .zfs, {} shadowed{}",
        dir.display(),
        out.display(),
        human(size),
        r.loose,
        r.from_sta,
        r.from_fpq,
        r.from_zfs,
        r.shadowed,
        if r.generated_manifest { "; manifest.toml generated from info.ini" } else { "" }
    );
    Ok(())
}

fn verify(spec: &str) -> Result<(), String> {
    let a = Archive::open(spec)?;
    let mut bad = Vec::new();
    let mut n = 0;
    for i in 0..a.len() {
        let e = a.entry(i)?;
        if e.how == "hide" {
            continue;
        }
        n += 1;
        if let Err(msg) = a.read(i) {
            bad.push(format!("{}: {msg}", e.path));
        }
    }
    for b in &bad {
        eprintln!("{b}");
    }
    if let Archive::Sta(s) = &a {
        outln!("id {}{}", hex(&s.content_hash()), if s.is_signed() { " (signed; signature not checked yet: CON-19)" } else { "" });
    }
    if bad.is_empty() {
        outln!("ok: {n} file(s) read and checked");
        Ok(())
    } else {
        Err(format!("{} of {n} file(s) failed", bad.len()))
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let pos: Vec<&String> = {
        let mut v = Vec::new();
        let mut skip = false;
        for a in args.iter().skip(1) {
            if skip {
                skip = false;
            } else if a == "--format" || a == "--key" {
                skip = true;
            } else if !a.starts_with("--") && a != "-l" {
                v.push(a);
            }
        }
        v
    };
    let need = |n: usize| if pos.len() < n { Err(USAGE.to_string()) } else { Ok(()) };
    match args.first().map(String::as_str) {
        Some("info") => need(1).and_then(|_| info(pos[0])),
        Some("list") => need(1).and_then(|_| list(pos[0], flag("-l"))),
        Some("find") => need(2).and_then(|_| find(pos[0], pos[1])),
        Some("cat") => need(2).and_then(|_| cat(pos[0], pos[1])),
        Some("extract") => need(2).and_then(|_| {
            let patterns: Vec<String> = pos[2..].iter().map(|s| s.to_string()).collect();
            extract(pos[0], Path::new(pos[1]), flag("--force"), &patterns)
        }),
        Some("verify") => need(1).and_then(|_| verify(pos[0])),
        Some("convert") => need(2).and_then(|_| convert(Path::new(pos[0]), Path::new(pos[1]), flag("--raw"), flag("--force"))),
        Some("pack") => need(2).and_then(|_| {
            let format = value("--format").ok_or("pack needs --format sta|fpq1|fpq2|zfs")?;
            let key = match value("--key") {
                Some(k) => u32::from_str_radix(k.trim_start_matches("0x"), 16).map_err(|_| format!("bad --key {k}"))?,
                None => 0,
            };
            pack(Path::new(pos[0]), Path::new(pos[1]), &format, flag("--raw"), key, flag("--force"))
        }),
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob("*.odf", "odf/system/Fed.ODF"));
        assert!(glob("odf/*/fed*", "ODF/system/fed_akira.odf"));
        assert!(!glob("*.tt", "odf/a.odf"));
        assert!(glob("borg4.tt", "BORG4.TT"));
    }

    #[test]
    fn unsafe_paths_refused() {
        let out = Path::new("/tmp/x");
        assert!(safe_join(out, "../evil").is_err());
        assert!(safe_join(out, "/etc/passwd").is_err());
        assert!(safe_join(out, "C:\\Windows\\x").is_err());
        assert!(safe_join(out, "a/../../b").is_err());
        assert_eq!(safe_join(out, "odf\\system\\fed.odf").unwrap(), Path::new("/tmp/x/odf/system/fed.odf"));
    }
}
