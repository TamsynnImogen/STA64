//! Writing `.sta` archives (`docs/STA_ARCHIVE.md` 1.0), reproducibly.
//!
//! - Entries in key order (ASCII lower case of the path), data in index
//!   order, each file's data starting at a multiple of 16, no timestamps:
//!   the same input packs to the same bytes, whatever order it came in.
//! - One fixed Zstandard level (`ruzstd`'s `Fastest`, about zstd level 1,
//!   the only one it implements so far). A file is stored when compressing
//!   it does not make it smaller; [`Compression::None`] stores everything.
//! - The index is compressed the same way (header flag bit 0) when that is
//!   smaller.
//! - `content_hash` covers paths and contents only, so stored and compressed
//!   packings of the same files share it.
//!
//! Paths follow the spec's rules ([`crate::sta::check_path`]); two paths that
//! differ only in case are refused, since lookups would find one.

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::sta::{ENTRY_SIZE_1_0, HEADER_SIZE, Kind, MAGIC, MAJOR, StaError, check_path, content_hash};

/// What to put at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Data(Vec<u8>),
    /// Hide this path in the mods below.
    Whiteout,
}

/// Loads one file's bytes when [`pack_to`] reaches it.
pub type Loader<'a> = Box<dyn FnOnce() -> std::io::Result<Vec<u8>> + 'a>;

/// What to put at a path, loaded only when it is written ([`pack_to`]), so
/// a mod of any size packs with about one file in memory.
pub enum Item<'a> {
    Data(Vec<u8>),
    Load(Loader<'a>),
    Whiteout,
}

impl From<Content> for Item<'_> {
    fn from(c: Content) -> Self {
        match c {
            Content::Data(d) => Item::Data(d),
            Content::Whiteout => Item::Whiteout,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// Zstandard when smaller, else stored.
    #[default]
    Auto,
    /// Everything stored (index too).
    None,
}

fn zstd(data: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
}

fn io(e: std::io::Error) -> StaError {
    StaError::Io(e.to_string())
}

struct Written {
    path: String,
    key: String,
    kind: Kind,
    zstd: bool,
    offset: u64,
    stored: u64,
    size: u64,
    sha: [u8; 32],
}

/// Build an archive in memory from `(path, content)` pairs, in any order.
pub fn pack(files: Vec<(String, Content)>, compression: Compression) -> Result<Vec<u8>, StaError> {
    let mut out = std::io::Cursor::new(Vec::new());
    pack_to(&mut out, files.into_iter().map(|(p, c)| (p, c.into())).collect(), compression)?;
    Ok(out.into_inner())
}

/// Write an archive to `out` (from its current position), loading each item
/// only when it is written. Returns the archive's `content_hash`. The bytes
/// are exactly those [`pack`] gives for the same files.
pub fn pack_to<W: Write + Seek>(out: &mut W, files: Vec<(String, Item<'_>)>, compression: Compression) -> Result<[u8; 32], StaError> {
    let mut items = Vec::with_capacity(files.len());
    for (path, item) in files {
        let key = check_path(&path)?;
        items.push((key, path, item));
    }
    items.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    for w in items.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(StaError::BadPath { path: w[1].1.clone(), why: "same path as another file, ignoring case" });
        }
    }

    let base = out.stream_position().map_err(io)?;
    out.write_all(&[0u8; HEADER_SIZE as usize]).map_err(io)?;
    let mut pos = HEADER_SIZE;
    // Data, 16-byte aligned, in index order, one file at a time.
    let mut written = Vec::with_capacity(items.len());
    for (key, path, item) in items {
        let data = match item {
            Item::Whiteout => {
                written.push(Written { path, key, kind: Kind::Whiteout, zstd: false, offset: 0, stored: 0, size: 0, sha: [0; 32] });
                continue;
            }
            Item::Data(d) => d,
            Item::Load(f) => f().map_err(|e| StaError::Io(format!("{path}: {e}")))?,
        };
        let sha: [u8; 32] = Sha256::digest(&data).into();
        let size = data.len() as u64;
        let packed = (compression == Compression::Auto && !data.is_empty()).then(|| zstd(&data)).filter(|z| z.len() < data.len());
        let pad = pos.next_multiple_of(16) - pos;
        out.write_all(&[0u8; 16][..pad as usize]).map_err(io)?;
        pos += pad;
        let (is_zstd, stored) = match &packed {
            Some(z) => (true, z.as_slice()),
            None => (false, data.as_slice()),
        };
        out.write_all(stored).map_err(io)?;
        written.push(Written { path, key, kind: Kind::File, zstd: is_zstd, offset: pos, stored: stored.len() as u64, size, sha });
        pos += stored.len() as u64;
    }

    // Index: count, record size, content_hash, records, string pool.
    let hash = content_hash(written.iter().map(|w| (w.key.as_str(), w.kind, w.size, &w.sha)));
    let mut index = Vec::with_capacity(40 + written.len() * (ENTRY_SIZE_1_0 + 32));
    index.extend_from_slice(&(written.len() as u32).to_le_bytes());
    index.extend_from_slice(&(ENTRY_SIZE_1_0 as u32).to_le_bytes());
    index.extend_from_slice(&hash);
    let mut pool = Vec::new();
    for w in &written {
        let path_offset = u32::try_from(pool.len()).map_err(|_| StaError::Corrupt("string pool over 4 GiB".into()))?;
        let mut r = [0u8; ENTRY_SIZE_1_0];
        r[0..4].copy_from_slice(&path_offset.to_le_bytes());
        r[4..6].copy_from_slice(&(w.path.len() as u16).to_le_bytes());
        r[6] = w.zstd as u8;
        r[7] = (w.kind == Kind::Whiteout) as u8;
        r[8..16].copy_from_slice(&w.offset.to_le_bytes());
        r[16..24].copy_from_slice(&w.stored.to_le_bytes());
        r[24..32].copy_from_slice(&w.size.to_le_bytes());
        r[32..64].copy_from_slice(&w.sha);
        index.extend_from_slice(&r);
        pool.extend_from_slice(w.path.as_bytes());
    }
    index.extend_from_slice(&pool);
    let index_size = index.len() as u64;
    let packed_index = (compression == Compression::Auto).then(|| zstd(&index)).filter(|z| z.len() < index.len());
    let (flags, stored_index) = match packed_index {
        Some(z) => (1u32, z),
        None => (0u32, index),
    };
    let index_offset = pos;
    out.write_all(&stored_index).map_err(io)?;
    let end = out.stream_position().map_err(io)?;

    let mut header = [0u8; HEADER_SIZE as usize];
    header[..8].copy_from_slice(MAGIC);
    header[8..10].copy_from_slice(&MAJOR.to_le_bytes());
    header[10..12].copy_from_slice(&0u16.to_le_bytes());
    header[12..16].copy_from_slice(&flags.to_le_bytes());
    header[16..24].copy_from_slice(&index_offset.to_le_bytes());
    header[24..32].copy_from_slice(&(stored_index.len() as u64).to_le_bytes());
    header[32..40].copy_from_slice(&index_size.to_le_bytes());
    // signature_offset (40..48) and reserved (48..64) stay 0.
    out.seek(SeekFrom::Start(base)).map_err(io)?;
    out.write_all(&header).map_err(io)?;
    out.seek(SeekFrom::Start(end)).map_err(io)?;
    Ok(hash)
}

fn walk_files(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let ft = e.file_type()?;
        let p = e.path();
        if ft.is_dir() {
            walk_files(&p, root, out)?;
        } else if ft.is_file() {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.push((rel, p));
        }
    }
    Ok(())
}

/// Every file under `root` (recursively, not following directory symlinks),
/// as archive paths with `/`, read now. Order does not matter to [`pack`].
pub fn files_in_dir(root: &Path) -> std::io::Result<Vec<(String, Content)>> {
    let mut paths = Vec::new();
    walk_files(root, root, &mut paths)?;
    paths.into_iter().map(|(rel, p)| Ok((rel, Content::Data(std::fs::read(&p)?)))).collect()
}

/// As [`files_in_dir`], but each file is read only when [`pack_to`] writes it.
pub fn items_in_dir(root: &Path) -> std::io::Result<Vec<(String, Item<'static>)>> {
    let mut paths = Vec::new();
    walk_files(root, root, &mut paths)?;
    Ok(paths.into_iter().map(|(rel, p)| (rel, Item::Load(Box::new(move || std::fs::read(p))))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sta::{StaArchive, Storage};

    fn noise(n: usize) -> Vec<u8> {
        // Deterministic, incompressible-looking bytes.
        let mut x: u32 = 0x1234_5678;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    fn sample() -> Vec<(String, Content)> {
        vec![
            ("odf/ships/Fed_Akira.odf".into(), Content::Data(b"classLabel = \"craft\"\r\n".repeat(200))),
            ("manifest.toml".into(), Content::Data(b"[mod]\nid = \"x\"\n".to_vec())),
            ("assets/noise.bin".into(), Content::Data(noise(4096))),
            ("assets/empty.bin".into(), Content::Data(Vec::new())),
            ("odf/ships/fdefiant.odf".into(), Content::Whiteout),
        ]
    }

    #[test]
    fn round_trips() {
        for c in [Compression::Auto, Compression::None] {
            let a = StaArchive::from_bytes(pack(sample(), c).unwrap()).unwrap();
            assert_eq!(a.len(), 5);
            for (path, content) in sample() {
                let i = a.find(&path).unwrap();
                assert_eq!(a.entry(i).path, path);
                match content {
                    Content::Data(d) => assert_eq!(a.read(i).unwrap(), d, "{path}"),
                    Content::Whiteout => assert_eq!(a.entry(i).kind, Kind::Whiteout),
                }
            }
        }
    }

    #[test]
    fn reproducible_whatever_the_input_order() {
        let a = pack(sample(), Compression::Auto).unwrap();
        let mut rev = sample();
        rev.reverse();
        assert_eq!(pack(rev, Compression::Auto).unwrap(), a);
        assert_eq!(pack(sample(), Compression::Auto).unwrap(), a);
    }

    #[test]
    fn identity_is_the_same_stored_or_compressed() {
        let z = StaArchive::from_bytes(pack(sample(), Compression::Auto).unwrap()).unwrap();
        let s = StaArchive::from_bytes(pack(sample(), Compression::None).unwrap()).unwrap();
        assert_ne!(pack(sample(), Compression::Auto).unwrap(), pack(sample(), Compression::None).unwrap());
        assert_eq!(z.content_hash(), s.content_hash());
    }

    #[test]
    fn stores_what_does_not_shrink() {
        let a = StaArchive::from_bytes(pack(sample(), Compression::Auto).unwrap()).unwrap();
        let st = |p: &str| a.entry(a.find(p).unwrap()).storage;
        assert_eq!(st("odf/ships/fed_akira.odf"), Storage::Zstd);
        assert_eq!(st("assets/noise.bin"), Storage::Stored);
        assert_eq!(st("assets/empty.bin"), Storage::Stored);
        // Data starts on 16-byte boundaries.
        assert!(a.entries().iter().filter(|e| e.kind == Kind::File).all(|e| e.offset % 16 == 0));
    }

    #[test]
    fn refuses_bad_and_clashing_paths() {
        let clash = vec![("a/B.odf".to_string(), Content::Data(vec![1])), ("A/b.ODF".to_string(), Content::Data(vec![2]))];
        assert!(matches!(pack(clash, Compression::Auto), Err(StaError::BadPath { .. })));
        for bad in ["../x", "/abs", "a\\b", "a//b"] {
            assert!(pack(vec![(bad.into(), Content::Data(vec![]))], Compression::Auto).is_err(), "{bad}");
        }
    }

    #[test]
    fn streaming_gives_the_same_bytes() {
        let want = pack(sample(), Compression::Auto).unwrap();
        let items: Vec<(String, Item)> = sample()
            .into_iter()
            .map(|(p, c)| match c {
                Content::Data(d) => (p, Item::Load(Box::new(move || Ok(d)))),
                Content::Whiteout => (p, Item::Whiteout),
            })
            .collect();
        let mut f = std::io::Cursor::new(Vec::new());
        let hash = pack_to(&mut f, items, Compression::Auto).unwrap();
        assert_eq!(f.into_inner(), want);
        assert_eq!(StaArchive::from_bytes(want).unwrap().content_hash(), hash);
        // A loader's error names the file.
        let failing = vec![("a.odf".to_string(), Item::Load(Box::new(|| Err(std::io::Error::other("gone")))))];
        let e = pack_to(&mut std::io::Cursor::new(Vec::new()), failing, Compression::Auto).unwrap_err();
        assert!(e.to_string().contains("a.odf: gone"), "{e}");
    }

    #[test]
    fn empty_archive() {
        let a = StaArchive::from_bytes(pack(Vec::new(), Compression::Auto).unwrap()).unwrap();
        assert!(a.is_empty());
    }

    /// With STA64_TEST_STA_PACK_DIR naming a mod folder: pack it twice (same
    /// bytes), read every file back, compare sizes with the folder.
    #[test]
    fn real_folder_if_available() {
        let Some(dir) = std::env::var_os("STA64_TEST_STA_PACK_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        let files = files_in_dir(&dir).unwrap();
        let raw: u64 = files.iter().map(|(_, c)| if let Content::Data(d) = c { d.len() as u64 } else { 0 }).sum();
        let t = std::time::Instant::now();
        let one = pack(files.clone(), Compression::Auto).unwrap();
        let took = t.elapsed();
        assert_eq!(pack(files.clone(), Compression::Auto).unwrap(), one, "reproducible");
        let a = StaArchive::from_bytes(one.clone()).unwrap();
        for (path, c) in &files {
            if let Content::Data(d) = c {
                assert_eq!(&a.read(a.find(path).unwrap()).unwrap(), d, "{path}");
            }
        }
        let zstd = a.entries().iter().filter(|e| e.storage == Storage::Zstd).count();
        eprintln!("{}: {} files, {raw} bytes -> {} bytes ({:.0}%), {zstd} compressed, packed in {took:?}", dir.display(), files.len(), one.len(), one.len() as f64 * 100.0 / raw.max(1) as f64);
    }

    #[test]
    fn same_folder_packs_to_the_same_bytes() {
        let dir = std::env::temp_dir().join(format!("sta_write_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (p, d) in [("manifest.toml", &b"[mod]\nid = \"d\"\n"[..]), ("odf/ships/a.odf", b"x = 1\r\n"), ("assets/t/b.bin", b"0123456789")] {
            let f = dir.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, d).unwrap();
        }
        let one = pack(files_in_dir(&dir).unwrap(), Compression::Auto).unwrap();
        let two = pack(files_in_dir(&dir).unwrap(), Compression::Auto).unwrap();
        assert_eq!(one, two);
        let a = StaArchive::from_bytes(one).unwrap();
        assert_eq!(a.read(a.find("assets/t/b.bin").unwrap()).unwrap(), b"0123456789");
        let _ = std::fs::remove_dir_all(dir);
    }
}
