//! STA64's own mod archives (`.sta`), as specified in `docs/STA_ARCHIVE.md`.
//!
//! [`StaArchive::open`] reads the header and the index and checks everything
//! the spec's "Reading safely" lists before anything is served: ranges,
//! overlaps, paths, key order and uniqueness, and the archive identity
//! (`content_hash`). File data is read on demand ([`StaArchive::read`]),
//! decoded (stored or one Zstandard frame that must end exactly at its stored
//! size and decode to exactly its size) and checked against its SHA-256.
//!
//! Signatures are not checked here (CON-19); [`StaArchive::is_signed`] says
//! whether a block is present.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

pub const MAGIC: &[u8; 8] = b"STA64ARC";
pub const MAJOR: u16 = 1;
pub const HEADER_SIZE: u64 = 64;
/// Bytes per entry record in 1.0; a later minor may grow records.
pub const ENTRY_SIZE_1_0: usize = 72;
pub const MAX_PATH: usize = 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StaError {
    #[error("not a .sta archive")]
    BadMagic,
    #[error("archive version {0}.{1} (this reader knows {MAJOR}.x)")]
    Version(u16, u16),
    #[error("corrupt archive: {0}")]
    Corrupt(String),
    #[error("{path}: bad path: {why}")]
    BadPath { path: String, why: &'static str },
    #[error("{path}: SHA-256 does not match (damaged or modified)")]
    HashMismatch { path: String },
    #[error("{0}: is a whiteout, it has no data")]
    Whiteout(String),
    #[error("io: {0}")]
    Io(String),
}

fn corrupt(s: impl Into<String>) -> StaError {
    StaError::Corrupt(s.into())
}

/// How an entry's bytes are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    Stored,
    Zstd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    /// Hides this path in the mods below; has no data.
    Whiteout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaEntry {
    /// As written in the archive.
    pub path: String,
    /// ASCII lower case of `path`: what lookups compare.
    pub key: String,
    pub storage: Storage,
    pub kind: Kind,
    pub offset: u64,
    pub stored_size: u64,
    pub size: u64,
    pub sha256: [u8; 32],
}

enum Backing {
    Memory(Vec<u8>),
    File(Mutex<File>),
}

pub struct StaArchive {
    backing: Backing,
    major: u16,
    minor: u16,
    signature_offset: u64,
    content_hash: [u8; 32],
    entries: Vec<StaEntry>,
}

impl std::fmt::Debug for StaArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaArchive").field("version", &(self.major, self.minor)).field("entries", &self.entries.len()).finish()
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// The spec's path rules; returns the key.
pub fn check_path(path: &str) -> Result<String, StaError> {
    let bad = |why| StaError::BadPath { path: path.to_owned(), why };
    if path.is_empty() || path.len() > MAX_PATH {
        return Err(bad("empty or longer than 1024 bytes"));
    }
    if path.chars().any(|c| c == '\\' || c == ':' || c.is_control()) {
        return Err(bad("contains \\, : or a control character"));
    }
    if path.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return Err(bad("absolute, or an empty, . or .. component"));
    }
    Ok(path.to_ascii_lowercase())
}

/// The archive identity over entries in key order (spec "Archive identity").
pub fn content_hash<'a>(entries: impl IntoIterator<Item = (&'a str, Kind, u64, &'a [u8; 32])>) -> [u8; 32] {
    let mut h = Sha256::new();
    for (key, kind, size, sha) in entries {
        h.update(key.as_bytes());
        h.update([0u8, matches!(kind, Kind::Whiteout) as u8]);
        h.update(size.to_le_bytes());
        h.update(sha);
    }
    h.finalize().into()
}

/// Decode one Zstandard frame that must fill `stored` exactly and give exactly `size` bytes.
fn zstd_exact(stored: &[u8], size: u64, what: &str) -> Result<Vec<u8>, StaError> {
    let mut src = stored;
    let mut dec = ruzstd::decoding::StreamingDecoder::new(&mut src).map_err(|e| corrupt(format!("{what}: Zstandard: {e}")))?;
    let mut out = Vec::with_capacity(size.min(1 << 26) as usize);
    (&mut dec).take(size + 1).read_to_end(&mut out).map_err(|e| corrupt(format!("{what}: Zstandard: {e}")))?;
    drop(dec);
    if out.len() as u64 != size {
        return Err(corrupt(format!("{what}: decodes to {} bytes, expected {size}", out.len())));
    }
    if !src.is_empty() {
        return Err(corrupt(format!("{what}: Zstandard frame ends {} bytes before its stored size", src.len())));
    }
    Ok(out)
}

impl StaArchive {
    /// Open a file; data is read from it on demand.
    pub fn open(path: &Path) -> Result<StaArchive, StaError> {
        let mut f = File::open(path).map_err(|e| StaError::Io(e.to_string()))?;
        let len = f.metadata().map_err(|e| StaError::Io(e.to_string()))?.len();
        let mut header = [0u8; HEADER_SIZE as usize];
        if len < HEADER_SIZE {
            return Err(StaError::BadMagic);
        }
        f.read_exact(&mut header).map_err(|e| StaError::Io(e.to_string()))?;
        let (index_offset, index_stored) = Self::check_header(&header, len)?;
        let mut index = vec![0u8; index_stored as usize];
        f.seek(SeekFrom::Start(index_offset)).and_then(|_| f.read_exact(&mut index)).map_err(|e| StaError::Io(e.to_string()))?;
        Self::build(&header, &index, Backing::File(Mutex::new(f)))
    }

    /// An archive held in memory (tests, or one nested in another archive).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<StaArchive, StaError> {
        if bytes.len() < HEADER_SIZE as usize {
            return Err(StaError::BadMagic);
        }
        let header: [u8; 64] = bytes[..64].try_into().unwrap();
        let (index_offset, index_stored) = Self::check_header(&header, bytes.len() as u64)?;
        let index = bytes[index_offset as usize..(index_offset + index_stored) as usize].to_vec();
        Self::build(&header, &index, Backing::Memory(bytes))
    }

    /// Magic, version and the index/signature ranges. Returns (index offset, stored size).
    fn check_header(h: &[u8; 64], file_len: u64) -> Result<(u64, u64), StaError> {
        if &h[..8] != MAGIC {
            return Err(StaError::BadMagic);
        }
        let (major, minor) = (u16_at(h, 8), u16_at(h, 10));
        if major != MAJOR {
            return Err(StaError::Version(major, minor));
        }
        let (io, is, sig) = (u64_at(h, 16), u64_at(h, 24), u64_at(h, 40));
        let index_end = io.checked_add(is).ok_or_else(|| corrupt("index range overflows"))?;
        if io < HEADER_SIZE || index_end > file_len {
            return Err(corrupt(format!("index {io}..{index_end} outside the file ({file_len} bytes)")));
        }
        if sig != 0 && (sig < index_end || sig >= file_len) {
            return Err(corrupt("signature block overlaps the index or lies past the end"));
        }
        if sig == 0 && index_end != file_len {
            return Err(corrupt(format!("{} bytes after the index", file_len - index_end)));
        }
        Ok((io, is))
    }

    fn build(h: &[u8; 64], stored_index: &[u8], backing: Backing) -> Result<StaArchive, StaError> {
        let (major, minor, flags) = (u16_at(h, 8), u16_at(h, 10), u32_at(h, 12));
        let (index_offset, index_size, signature_offset) = (u64_at(h, 16), u64_at(h, 32), u64_at(h, 40));
        let index = if flags & 1 != 0 {
            zstd_exact(stored_index, index_size, "index")?
        } else {
            if stored_index.len() as u64 != index_size {
                return Err(corrupt("uncompressed index: stored and unpacked sizes differ"));
            }
            stored_index.to_vec()
        };
        if index.len() < 40 {
            return Err(corrupt("index shorter than its header"));
        }
        let count = u32_at(&index, 0) as usize;
        let rec = u32_at(&index, 4) as usize;
        let stated: [u8; 32] = index[8..40].try_into().unwrap();
        if rec < ENTRY_SIZE_1_0 {
            return Err(corrupt(format!("entry records of {rec} bytes (at least {ENTRY_SIZE_1_0})")));
        }
        let table_end = count.checked_mul(rec).and_then(|t| t.checked_add(40)).ok_or_else(|| corrupt("entry table overflows"))?;
        if table_end > index.len() {
            return Err(corrupt("entry table longer than the index"));
        }
        let pool = &index[table_end..];

        let mut entries: Vec<StaEntry> = Vec::with_capacity(count);
        for i in 0..count {
            let r = &index[40 + i * rec..40 + i * rec + ENTRY_SIZE_1_0];
            let (po, pl) = (u32_at(r, 0) as usize, u16_at(r, 4) as usize);
            let raw = pool.get(po..po + pl).ok_or_else(|| corrupt(format!("entry {i}: path outside the string pool")))?;
            let path = std::str::from_utf8(raw).map_err(|_| corrupt(format!("entry {i}: path is not UTF-8")))?.to_owned();
            let key = check_path(&path)?;
            let storage = match r[6] {
                0 => Storage::Stored,
                1 => Storage::Zstd,
                s => return Err(corrupt(format!("{path}: unknown storage {s}"))),
            };
            let kind = match r[7] {
                0 => Kind::File,
                1 => Kind::Whiteout,
                k => return Err(corrupt(format!("{path}: unknown kind {k}"))),
            };
            let (offset, stored_size, size) = (u64_at(r, 8), u64_at(r, 16), u64_at(r, 24));
            let sha256: [u8; 32] = r[32..64].try_into().unwrap();
            match kind {
                Kind::Whiteout if (stored_size, size, sha256) != (0, 0, [0; 32]) => {
                    return Err(corrupt(format!("{path}: whiteout with data")));
                }
                Kind::File => {
                    let end = offset.checked_add(stored_size).ok_or_else(|| corrupt(format!("{path}: data range overflows")))?;
                    if offset < HEADER_SIZE || end > index_offset {
                        return Err(corrupt(format!("{path}: data {offset}..{end} outside the data region")));
                    }
                    if storage == Storage::Stored && stored_size != size {
                        return Err(corrupt(format!("{path}: stored entry with stored size {stored_size} != size {size}")));
                    }
                }
                _ => {}
            }
            if let Some(prev) = entries.last() {
                if prev.key == key {
                    return Err(corrupt(format!("{path}: same key as {}", prev.path)));
                }
                if prev.key.as_bytes() > key.as_bytes() {
                    return Err(corrupt(format!("{path}: entries not sorted by key")));
                }
            }
            entries.push(StaEntry { path, key, storage, kind, offset, stored_size, size, sha256 });
        }

        // Data ranges may not overlap one another.
        let mut ranges: Vec<(u64, u64, &str)> = entries
            .iter()
            .filter(|e| e.kind == Kind::File && e.stored_size > 0)
            .map(|e| (e.offset, e.offset + e.stored_size, e.path.as_str()))
            .collect();
        ranges.sort();
        for w in ranges.windows(2) {
            if w[1].0 < w[0].1 {
                return Err(corrupt(format!("{} and {} share bytes", w[0].2, w[1].2)));
            }
        }

        let computed = content_hash(entries.iter().map(|e| (e.key.as_str(), e.kind, e.size, &e.sha256)));
        if computed != stated {
            return Err(corrupt("content_hash does not match the index (damaged or modified)"));
        }
        Ok(StaArchive { backing, major, minor, signature_offset, content_hash: stated, entries })
    }

    pub fn version(&self) -> (u16, u16) {
        (self.major, self.minor)
    }

    /// The archive identity (spec "Archive identity").
    pub fn content_hash(&self) -> [u8; 32] {
        self.content_hash
    }

    pub fn is_signed(&self) -> bool {
        self.signature_offset != 0
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entry(&self, index: usize) -> &StaEntry {
        &self.entries[index]
    }

    pub fn entries(&self) -> &[StaEntry] {
        &self.entries
    }

    /// By path, case-insensitively (binary search on the key).
    pub fn find(&self, path: &str) -> Option<usize> {
        let key = path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase();
        self.entries.binary_search_by(|e| e.key.as_bytes().cmp(key.as_bytes())).ok()
    }

    fn raw(&self, offset: u64, len: u64) -> Result<Vec<u8>, StaError> {
        match &self.backing {
            Backing::Memory(b) => Ok(b[offset as usize..(offset + len) as usize].to_vec()),
            Backing::File(f) => {
                let mut f = f.lock().map_err(|_| StaError::Io("archive file lock poisoned".into()))?;
                let mut buf = vec![0u8; len as usize];
                f.seek(SeekFrom::Start(offset)).and_then(|_| f.read_exact(&mut buf)).map_err(|e| StaError::Io(e.to_string()))?;
                Ok(buf)
            }
        }
    }

    /// The entry's bytes, decoded and checked against its SHA-256.
    pub fn read(&self, index: usize) -> Result<Vec<u8>, StaError> {
        let e = &self.entries[index];
        if e.kind == Kind::Whiteout {
            return Err(StaError::Whiteout(e.path.clone()));
        }
        let stored = self.raw(e.offset, e.stored_size)?;
        let data = match e.storage {
            Storage::Stored => stored,
            Storage::Zstd => zstd_exact(&stored, e.size, &e.path)?,
        };
        let sha: [u8; 32] = Sha256::digest(&data).into();
        if sha != e.sha256 {
            return Err(StaError::HashMismatch { path: e.path.clone() });
        }
        Ok(data)
    }
}

/// A minimal writer for tests: the real, reproducible one is CON-17.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) enum T<'a> {
        Stored(&'a str, &'a [u8]),
        Zstd(&'a str, &'a [u8]),
        Whiteout(&'a str),
    }

    /// Build an archive; entries are sorted by key here. `compress_index` sets flag bit 0.
    pub(crate) fn build(items: &[T], compress_index: bool) -> Vec<u8> {
        struct E {
            path: String,
            key: String,
            storage: u8,
            kind: u8,
            data: Vec<u8>,
            size: u64,
            sha: [u8; 32],
        }
        let mut es: Vec<E> = items
            .iter()
            .map(|t| match t {
                T::Stored(p, d) | T::Zstd(p, d) => {
                    let z = matches!(t, T::Zstd(..));
                    let data = if z { ruzstd::encoding::compress_to_vec(*d, ruzstd::encoding::CompressionLevel::Fastest) } else { d.to_vec() };
                    E { path: p.to_string(), key: p.to_ascii_lowercase(), storage: z as u8, kind: 0, data, size: d.len() as u64, sha: Sha256::digest(d).into() }
                }
                T::Whiteout(p) => E { path: p.to_string(), key: p.to_ascii_lowercase(), storage: 0, kind: 1, data: vec![], size: 0, sha: [0; 32] },
            })
            .collect();
        es.sort_by(|a, b| a.key.cmp(&b.key));
        let mut out = vec![0u8; 64];
        let mut offsets = Vec::new();
        for e in &es {
            while !out.len().is_multiple_of(16) {
                out.push(0);
            }
            offsets.push(if e.kind == 1 { 0 } else { out.len() as u64 });
            out.extend_from_slice(&e.data);
        }
        let mut pool = Vec::new();
        let mut index = Vec::new();
        index.extend_from_slice(&(es.len() as u32).to_le_bytes());
        index.extend_from_slice(&(ENTRY_SIZE_1_0 as u32).to_le_bytes());
        let ch = content_hash(es.iter().map(|e| (e.key.as_str(), if e.kind == 1 { Kind::Whiteout } else { Kind::File }, e.size, &e.sha)));
        index.extend_from_slice(&ch);
        for (e, off) in es.iter().zip(&offsets) {
            let mut r = vec![0u8; ENTRY_SIZE_1_0];
            r[0..4].copy_from_slice(&(pool.len() as u32).to_le_bytes());
            r[4..6].copy_from_slice(&(e.path.len() as u16).to_le_bytes());
            r[6] = e.storage;
            r[7] = e.kind;
            r[8..16].copy_from_slice(&off.to_le_bytes());
            r[16..24].copy_from_slice(&(e.data.len() as u64).to_le_bytes());
            r[24..32].copy_from_slice(&e.size.to_le_bytes());
            r[32..64].copy_from_slice(&e.sha);
            index.extend_from_slice(&r);
            pool.extend_from_slice(e.path.as_bytes());
        }
        index.extend_from_slice(&pool);
        let index_size = index.len() as u64;
        let stored = if compress_index { ruzstd::encoding::compress_to_vec(&index[..], ruzstd::encoding::CompressionLevel::Fastest) } else { index };
        let io = out.len() as u64;
        out.extend_from_slice(&stored);
        out[..8].copy_from_slice(MAGIC);
        out[8..10].copy_from_slice(&MAJOR.to_le_bytes());
        out[12..16].copy_from_slice(&(compress_index as u32).to_le_bytes());
        out[16..24].copy_from_slice(&io.to_le_bytes());
        out[24..32].copy_from_slice(&(stored.len() as u64).to_le_bytes());
        out[32..40].copy_from_slice(&index_size.to_le_bytes());
        out
    }

    /// `fixtures/sta/Data/Mods/StaMod/stamod.sta`, used by `sta_content`'s ODF
    /// index test. Kept equal to this build; `STA64_WRITE_FIXTURES=1` rewrites it.
    pub(crate) fn fixture_archive() -> Vec<u8> {
        build(
            &[
                T::Stored("manifest.toml", b"[mod]\nid = \"stamod\"\nname = \"STA fixture\"\nversion = \"0.1.0\"\nkind = \"addon\"\n"),
                T::Zstd("odf/ships/fakira.odf", b"#include \"fakira.odf\"\nmaxHealth = 1500\nunitName = \"Akira (sta)\"\n"),
                T::Zstd("odf/my_pack/deep/fnew.odf", b"classLabel = \"craft\"\nunitName = \"New ship\"\n"),
                T::Whiteout("odf/ships/fdefiant.odf"),
            ],
            true,
        )
    }

    #[test]
    fn fixture_is_current() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sta/Data/Mods/StaMod/stamod.sta");
        let want = fixture_archive();
        if std::env::var_os("STA64_WRITE_FIXTURES").is_some() {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, &want).unwrap();
        }
        assert_eq!(std::fs::read(&p).unwrap(), want, "rerun with STA64_WRITE_FIXTURES=1");
        StaArchive::open(&p).unwrap();
    }

    fn sample(compress_index: bool) -> Vec<u8> {
        let big = b"classLabel = \"craft\"\r\n".repeat(400);
        build(
            &[
                T::Stored("manifest.toml", b"[mod]\nid = \"test\"\n"),
                T::Zstd("odf/ships/Fed_Akira.odf", &big),
                T::Stored("assets/empty.bin", b""),
                T::Whiteout("odf/ships/fdefiant.odf"),
            ],
            compress_index,
        )
    }

    #[test]
    fn reads_both_index_forms() {
        for ci in [false, true] {
            let a = StaArchive::from_bytes(sample(ci)).unwrap();
            assert_eq!((a.version(), a.len(), a.is_signed()), ((1, 0), 4, false));
            let i = a.find("ODF/Ships/fed_akira.ODF").unwrap();
            assert_eq!(a.entry(i).path, "odf/ships/Fed_Akira.odf");
            assert_eq!(a.entry(i).storage, Storage::Zstd);
            assert_eq!(a.read(i).unwrap(), b"classLabel = \"craft\"\r\n".repeat(400));
            assert_eq!(a.read(a.find("manifest.toml").unwrap()).unwrap(), b"[mod]\nid = \"test\"\n");
            assert_eq!(a.read(a.find("assets/empty.bin").unwrap()).unwrap(), b"");
            let w = a.find("odf/ships/fdefiant.odf").unwrap();
            assert_eq!(a.entry(w).kind, Kind::Whiteout);
            assert!(matches!(a.read(w), Err(StaError::Whiteout(_))));
            assert_eq!(a.find("nope"), None);
        }
        // Identity ignores how the index was stored.
        assert_eq!(StaArchive::from_bytes(sample(false)).unwrap().content_hash(), StaArchive::from_bytes(sample(true)).unwrap().content_hash());
    }

    #[test]
    fn identity_ignores_compression() {
        let a = build(&[T::Stored("a.odf", b"x = 1\r\n")], false);
        let b = build(&[T::Zstd("a.odf", b"x = 1\r\n")], false);
        assert_eq!(StaArchive::from_bytes(a).unwrap().content_hash(), StaArchive::from_bytes(b).unwrap().content_hash());
    }

    #[test]
    fn opens_from_disk_and_reads_on_demand() {
        let p = std::env::temp_dir().join(format!("sta_vfs_sta_{}.sta", std::process::id()));
        std::fs::write(&p, sample(true)).unwrap();
        let a = StaArchive::open(&p).unwrap();
        assert_eq!(a.read(a.find("manifest.toml").unwrap()).unwrap(), b"[mod]\nid = \"test\"\n");
        let _ = std::fs::remove_file(p);
    }

    // ---- "Reading safely": each refusal.

    fn err(bytes: Vec<u8>) -> String {
        StaArchive::from_bytes(bytes).unwrap_err().to_string()
    }

    /// Byte offset of entry record `i` in an uncompressed-index archive.
    fn rec_at(b: &[u8], i: usize) -> usize {
        u64_at(b, 16) as usize + 40 + i * ENTRY_SIZE_1_0
    }

    #[test]
    fn refuses_bad_header() {
        assert_eq!(StaArchive::from_bytes(b"FPQ\0".to_vec()).unwrap_err(), StaError::BadMagic);
        let mut b = sample(false);
        b[8] = 2;
        assert!(err(b).contains("version 2.0"));
        let mut b = sample(false);
        b.push(0); // trailing bytes, unsigned
        assert!(err(b).contains("after the index"));
        let mut b = sample(false);
        let len = b.len() as u64;
        b[24..32].copy_from_slice(&len.to_le_bytes()); // index past the end
        assert!(err(b).contains("outside the file"));
        let mut b = sample(false);
        b[16..24].copy_from_slice(&10u64.to_le_bytes()); // index overlapping the header
        assert!(err(b).contains("outside the file"));
        let mut b = sample(false);
        b[40..48].copy_from_slice(&70u64.to_le_bytes()); // signature inside the data
        assert!(err(b).contains("signature block"));
    }

    #[test]
    fn refuses_bad_ranges_and_frames() {
        // Data past the data region (into the index).
        let mut b = sample(false);
        let r = rec_at(&b, 2); // sorted: assets/empty.bin, manifest.toml, odf/ships/fdefiant.odf, odf/ships/fed_akira.odf
        let io = u64_at(&b, 16);
        b[r + 8..r + 16].copy_from_slice(&io.to_le_bytes());
        b[r + 6] = 0;
        b[r + 7] = 0;
        b[r + 16..r + 24].copy_from_slice(&8u64.to_le_bytes());
        b[r + 24..r + 32].copy_from_slice(&8u64.to_le_bytes());
        assert!(err(b).contains("outside the data region"));
        // Two entries sharing bytes.
        let mut b = build(&[T::Stored("a", b"0123456789abcdef0123"), T::Stored("b", b"xyz")], false);
        let (ra, rb) = (rec_at(&b, 0), rec_at(&b, 1));
        let off_a = u64_at(&b, ra + 8);
        b[rb + 8..rb + 16].copy_from_slice(&(off_a + 4).to_le_bytes());
        assert!(err(b).contains("share bytes"));
        // A frame shorter than its stored size, and one decoding to the wrong size.
        let big = b"0123456789".repeat(100);
        let b = build(&[T::Zstd("z", &big)], false);
        let r = rec_at(&b, 0);
        let mut longer = b.clone();
        let st = u64_at(&b, r + 16);
        let off = u64_at(&b, r + 8);
        // Claim one more stored byte; it must still fit before the index, so steal padding by moving the index.
        let io = u64_at(&b, 16) as usize;
        longer.insert(io, 0);
        let io2 = (io + 1) as u64;
        longer[16..24].copy_from_slice(&io2.to_le_bytes());
        let r2 = r + 1;
        longer[r2 + 16..r2 + 24].copy_from_slice(&(st + 1).to_le_bytes());
        let a = StaArchive::from_bytes(longer).unwrap();
        assert!(a.read(0).unwrap_err().to_string().contains("ends 1 bytes before"));
        let mut wrong = b.clone();
        wrong[r + 24..r + 32].copy_from_slice(&999u64.to_le_bytes());
        // Size is part of the identity, so fix the stated hash too, to reach the decode check.
        fix_content_hash(&mut wrong);
        let a = StaArchive::from_bytes(wrong).unwrap();
        assert!(a.read(0).unwrap_err().to_string().contains("expected 999"), "offset {off}");
        // A stored entry whose sizes disagree.
        let mut b = build(&[T::Stored("s", b"abcd")], false);
        let r = rec_at(&b, 0);
        b[r + 24..r + 32].copy_from_slice(&5u64.to_le_bytes());
        assert!(err(b).contains("stored size 4 != size 5"));
    }

    fn fix_content_hash(b: &mut [u8]) {
        let io = u64_at(b, 16) as usize;
        let count = u32_at(b, io) as usize;
        let mut keys = Vec::new();
        let pool_at = io + 40 + count * ENTRY_SIZE_1_0;
        for i in 0..count {
            let r = io + 40 + i * ENTRY_SIZE_1_0;
            let (po, pl) = (u32_at(b, r) as usize, u16_at(b, r + 4) as usize);
            let key = String::from_utf8(b[pool_at + po..pool_at + po + pl].to_vec()).unwrap().to_ascii_lowercase();
            let kind = if b[r + 7] == 1 { Kind::Whiteout } else { Kind::File };
            keys.push((key, kind, u64_at(b, r + 24), <[u8; 32]>::try_from(&b[r + 32..r + 64]).unwrap()));
        }
        let h = content_hash(keys.iter().map(|(k, kind, s, sha)| (k.as_str(), *kind, *s, sha)));
        b[io + 8..io + 40].copy_from_slice(&h);
    }

    #[test]
    fn refuses_bad_paths_and_order() {
        for bad in ["../x", "/abs", "a//b", "a/./b", "c:/x", "a\\b", "x\u{1}y", ""] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
        assert!(check_path(&"a".repeat(1025)).is_err());
        assert_eq!(check_path("Odf/Ships/A.odf").unwrap(), "odf/ships/a.odf");
        // A bad path inside an archive.
        let mut b = build(&[T::Stored("ab", b"1")], false);
        let pool = u64_at(&b, 16) as usize + 40 + ENTRY_SIZE_1_0;
        b[pool..pool + 2].copy_from_slice(b"..");
        assert!(err(b).contains("bad path"));
        // Duplicate keys (differing only in case) and unsorted records.
        let mut b = build(&[T::Stored("aa", b"1"), T::Stored("ab", b"2")], false);
        let pool = u64_at(&b, 16) as usize + 40 + 2 * ENTRY_SIZE_1_0;
        b[pool + 2..pool + 4].copy_from_slice(b"AA");
        assert!(err(b).contains("same key"));
        let mut b = build(&[T::Stored("aa", b"1"), T::Stored("ab", b"2")], false);
        b[pool..pool + 2].copy_from_slice(b"zz");
        assert!(err(b).contains("not sorted"));
    }

    #[test]
    fn detects_tampering() {
        // A changed byte of file data: that file fails its SHA-256, the rest still read.
        let mut b = build(&[T::Stored("a", b"hello"), T::Stored("b", b"world")], false);
        let off = u64_at(&b, rec_at(&b, 0) + 8) as usize;
        b[off] ^= 1;
        let a = StaArchive::from_bytes(b).unwrap();
        assert!(matches!(a.read(0), Err(StaError::HashMismatch { .. })));
        assert_eq!(a.read(1).unwrap(), b"world");
        // A changed per-file hash in the index: the archive identity no longer matches.
        let mut b = build(&[T::Stored("a", b"hello")], false);
        let r = rec_at(&b, 0);
        b[r + 40] ^= 1;
        assert!(err(b).contains("content_hash"));
        // A whiteout carrying data.
        let mut b = build(&[T::Whiteout("w")], false);
        let r = rec_at(&b, 0);
        b[r + 24] = 1;
        assert!(err(b).contains("whiteout with data"));
    }
}
