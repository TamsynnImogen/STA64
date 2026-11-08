//! Fleet Operations FPQ archives, versions 1 and 2.
//!
//! - **v2**: Fleet Operations 4.0 / Roots, written by FPQEdit 4.0.
//! - **v1**: Fleet Operations 3.x, written by FPQEdit 3.2.7. The same layout
//!   with no folders (flat names) and a different name hash ([`hash_v1`]).
//!
//! Layout (little endian):
//! - 28-byte header: `FPQ\0`, version, folder count, file count, hash count,
//!   data size, names size.
//! - hash table: `hash_count` x (hash A u32, hash B u32, file index u32).
//! - folders: `folder_count` x (parent i32, name offset u32, name length u32).
//! - files: `file_count` x 25 packed bytes (data offset, size, stored size,
//!   name offset, name length: u32; compression: u8; folder: i32).
//! - name blob, then data blob.
//!
//! Lookup is MPQ-style over the case-insensitive basename: the start slot is
//! the slot hash masked to the (power-of-two) table size, then linear probing;
//! an all-zero slot is empty. v2 stores [`hash`] kinds `0x100` and `0x200` as
//! hash A and B; v1 stores [`hash_v1`] as both. Compression 0 is raw, 1 is
//! LZO1X.

pub mod lzo;

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum FpqError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not an FPQ archive")]
    BadMagic,
    #[error("unsupported FPQ version {0}")]
    Version(u32),
    #[error("corrupt archive: {0}")]
    Corrupt(String),
    #[error("entry {index}: {source}")]
    Lzo { index: usize, source: lzo::LzoError },
}

const HEADER: usize = 28;
const HASH: usize = 12;
const FOLDER: usize = 12;
const FILE: usize = 25;

#[derive(Debug, Clone, Copy)]
pub struct FileEntry {
    pub data_offset: u32,
    pub size: u32,
    pub stored_size: u32,
    name_offset: u32,
    name_len: u32,
    pub compression: u8,
    pub folder: i32,
}

#[derive(Debug, Clone, Copy)]
struct Folder {
    parent: i32,
    name_offset: u32,
    name_len: u32,
}

pub struct Fpq {
    version: u32,
    bytes: Vec<u8>,
    hashes: Vec<(u32, u32, u32)>,
    folders: Vec<Folder>,
    files: Vec<FileEntry>,
    names_at: usize,
    names_len: usize,
    data_at: usize,
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl Fpq {
    pub fn open(path: &Path) -> Result<Self, FpqError> {
        Self::from_bytes(std::fs::read(path)?)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, FpqError> {
        if bytes.len() < HEADER || &bytes[..4] != b"FPQ\0" {
            return Err(FpqError::BadMagic);
        }
        let h = |i: usize| u32_at(&bytes, 4 + i * 4) as usize;
        let (version, n_folders, n_files, n_hash, data_len, names_len) = (h(0), h(1), h(2), h(3), h(4), h(5));
        if !(1..=2).contains(&version) {
            return Err(FpqError::Version(version as u32));
        }
        if n_hash == 0 || !n_hash.is_power_of_two() {
            return Err(FpqError::Corrupt("hash table size is not a power of two".into()));
        }
        let hash_at = HEADER;
        let folder_at = hash_at + n_hash * HASH;
        let file_at = folder_at + n_folders * FOLDER;
        let names_at = file_at + n_files * FILE;
        let data_at = names_at + names_len;
        if data_at + data_len != bytes.len() {
            return Err(FpqError::Corrupt(format!(
                "header describes {} bytes, file has {}",
                data_at + data_len,
                bytes.len()
            )));
        }
        let hashes = (0..n_hash)
            .map(|i| {
                let o = hash_at + i * HASH;
                (u32_at(&bytes, o), u32_at(&bytes, o + 4), u32_at(&bytes, o + 8))
            })
            .collect();
        let folders = (0..n_folders)
            .map(|i| {
                let o = folder_at + i * FOLDER;
                Folder {
                    parent: u32_at(&bytes, o) as i32,
                    name_offset: u32_at(&bytes, o + 4),
                    name_len: u32_at(&bytes, o + 8),
                }
            })
            .collect();
        let files = (0..n_files)
            .map(|i| {
                let o = file_at + i * FILE;
                FileEntry {
                    data_offset: u32_at(&bytes, o),
                    size: u32_at(&bytes, o + 4),
                    stored_size: u32_at(&bytes, o + 8),
                    name_offset: u32_at(&bytes, o + 12),
                    name_len: u32_at(&bytes, o + 16),
                    compression: bytes[o + 20],
                    folder: u32_at(&bytes, o + 21) as i32,
                }
            })
            .collect();
        let fpq = Fpq {
            version: version as u32,
            bytes,
            hashes,
            folders,
            files,
            names_at,
            names_len,
            data_at,
        };
        fpq.validate(data_len)?;
        Ok(fpq)
    }

    fn validate(&self, data_len: usize) -> Result<(), FpqError> {
        for i in 0..self.files.len() {
            let f = self.files[i];
            self.path(i)?;
            if f.data_offset as usize + f.stored_size as usize > data_len {
                return Err(FpqError::Corrupt(format!("entry {i} data out of range")));
            }
            match f.compression {
                0 if f.size != f.stored_size => {
                    return Err(FpqError::Corrupt(format!("entry {i} raw size mismatch")));
                }
                0 | 1 => {}
                c => return Err(FpqError::Corrupt(format!("entry {i} compression {c}"))),
            }
        }
        // Hash slots are only dereferenced in `find`, which range-checks them.
        Ok(())
    }

    fn name(&self, offset: u32, len: u32) -> Result<String, FpqError> {
        let (o, l) = (offset as usize, len as usize);
        if o + l > self.names_len {
            return Err(FpqError::Corrupt("name out of range".into()));
        }
        let s = &self.bytes[self.names_at + o..self.names_at + o + l];
        // Names are Latin-1.
        Ok(s.iter().map(|&b| b as char).collect())
    }

    /// Format version: 1 (FPQEdit 3.2.7) or 2 (FPQEdit 4.0).
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Hash table size (a power of two).
    pub fn hash_slots(&self) -> usize {
        self.hashes.len()
    }

    pub fn folder_count(&self) -> usize {
        self.folders.len()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn entry(&self, index: usize) -> &FileEntry {
        &self.files[index]
    }

    /// Archive-relative path with `/` separators, e.g. `ships/fdefiant.odf`.
    pub fn path(&self, index: usize) -> Result<String, FpqError> {
        let f = self.files[index];
        let mut parts = vec![self.name(f.name_offset, f.name_len)?];
        let mut folder = f.folder;
        let mut steps = 0;
        while folder != -1 {
            let d = self
                .folders
                .get(folder as usize)
                .ok_or_else(|| FpqError::Corrupt(format!("bad folder index {folder}")))?;
            parts.push(self.name(d.name_offset, d.name_len)?);
            folder = d.parent;
            steps += 1;
            if steps > self.folders.len() {
                return Err(FpqError::Corrupt("folder cycle".into()));
            }
        }
        parts.reverse();
        Ok(parts.join("/"))
    }

    /// Find a file by basename via the hash table.
    pub fn find(&self, basename: &str) -> Option<usize> {
        let mask = self.hashes.len() - 1;
        let (start, a, b) = slot_and_hashes(self.version, basename);
        let start = start as usize & mask;
        let mut i = start;
        loop {
            let (ha, hb, idx) = self.hashes[i];
            if ha == a && hb == b && (idx as usize) < self.files.len() {
                return Some(idx as usize);
            }
            i = (i + 1) & mask;
            if i == start {
                return None;
            }
        }
    }

    /// Decoded contents of entry `index`.
    pub fn read(&self, index: usize) -> Result<Vec<u8>, FpqError> {
        let f = self.files[index];
        let start = self.data_at + f.data_offset as usize;
        let stored = &self.bytes[start..start + f.stored_size as usize];
        match f.compression {
            0 => Ok(stored.to_vec()),
            _ => {
                if self.version == 1 {
                    let (size, used) = v1_size_prefix(stored)
                        .ok_or_else(|| FpqError::Corrupt(format!("entry {index}: bad v1 size prefix")))?;
                    if size != f.size {
                        return Err(FpqError::Corrupt(format!("entry {index}: v1 prefix says {size}, entry {}", f.size)));
                    }
                    lzo::decompress_unterminated(&stored[used..], f.size as usize)
                } else {
                    lzo::decompress(stored, f.size as usize)
                }
                .map_err(|source| FpqError::Lzo { index, source })
            }
        }
    }
}

fn crypt_table() -> &'static [u32; 0x500] {
    static TABLE: std::sync::OnceLock<[u32; 0x500]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 0x500];
        let mut seed: u32 = 0x0010_0001;
        for lo in 0..0x100 {
            for hi in 0..5 {
                seed = (seed * 125 + 3) % 0x2A_AAAB;
                let a = (seed & 0xFFFF) << 16;
                seed = (seed * 125 + 3) % 0x2A_AAAB;
                t[hi * 0x100 + lo] = a | (seed & 0xFFFF);
            }
        }
        t
    })
}

/// FPQ v1 puts the unpacked size before each LZO stream: little-endian `u16`
/// chunks of 15 bits each, bit 15 set when another chunk follows (`8c 01` =
/// 396; `a4 f8 0f 00` = 0x78a4 + (0x0f << 15) = 522404). Returns (size, bytes used).
pub fn v1_size_prefix(b: &[u8]) -> Option<(u32, usize)> {
    let (mut value, mut shift, mut at) = (0u64, 0u32, 0usize);
    loop {
        let w = u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]) as u64;
        at += 2;
        value |= (w & 0x7FFF) << shift;
        shift += 15;
        if w & 0x8000 == 0 {
            return u32::try_from(value).ok().map(|v| (v, at));
        }
        if shift > 32 {
            return None;
        }
    }
}

/// The v1 size prefix for `size`.
pub fn v1_size_prefix_bytes(size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut v = size;
    loop {
        let more = v >> 15 != 0;
        let w = (v & 0x7FFF) as u16 | if more { 0x8000 } else { 0 };
        out.extend_from_slice(&w.to_le_bytes());
        v >>= 15;
        if !more {
            return out;
        }
    }
}

/// (slot hash, hash A, hash B) of a basename for archive version `version`.
pub fn slot_and_hashes(version: u32, basename: &str) -> (u32, u32, u32) {
    if version == 1 {
        let h = hash_v1(basename);
        (h, h, h)
    } else {
        (hash(basename, 0), hash(basename, 0x100), hash(basename, 0x200))
    }
}

/// FPQ v1's name hash, as FPQEdit 3.2.7 computes it: [`hash`]'s recurrence
/// with an all-zero crypt table (that build never fills its table, which sits
/// in zero-initialised memory), over the upper-cased name **without its last
/// character** (its loop runs one step short).
pub fn hash_v1(name: &str) -> u32 {
    let (mut s1, mut s2): (u32, u32) = (0x7FED_7FED, 0xEEEE_EEEE);
    let n = name.chars().count();
    for ch in name.chars().take(n.saturating_sub(1)) {
        let mut b = if (ch as u32) < 256 { ch as u32 } else { b'?' as u32 };
        if (b'a' as u32..=b'z' as u32).contains(&b) {
            b -= 0x20;
        }
        s1 = s1.wrapping_add(s2);
        s2 = b.wrapping_add(s1).wrapping_add(s2.wrapping_mul(33)).wrapping_add(3);
    }
    s1
}

/// MPQ-style case-insensitive string hash over Latin-1 bytes.
pub fn hash(name: &str, kind: usize) -> u32 {
    let t = crypt_table();
    let (mut s1, mut s2): (u32, u32) = (0x7FED_7FED, 0xEEEE_EEEE);
    for ch in name.chars() {
        let mut b = if (ch as u32) < 256 { ch as u32 } else { b'?' as u32 };
        if (b'a' as u32..=b'z' as u32).contains(&b) {
            b -= 0x20;
        }
        s1 = t[kind + b as usize] ^ s1.wrapping_add(s2);
        s2 = b.wrapping_add(s1).wrapping_add(s2.wrapping_mul(33)).wrapping_add(3);
    }
    s1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal archive in memory: one raw file, one folder.
    fn tiny() -> Vec<u8> {
        let names = b"shipsa.odf";
        let data = b"classLabel = \"starship\"\n";
        let n_hash = 4usize;
        let mut hashes = vec![(u32::MAX, u32::MAX, u32::MAX); n_hash];
        let slot = hash("a.odf", 0) as usize & (n_hash - 1);
        hashes[slot] = (hash("a.odf", 0x100), hash("a.odf", 0x200), 0);

        let mut b = Vec::new();
        b.extend_from_slice(b"FPQ\0");
        for v in [2u32, 1, 1, n_hash as u32, data.len() as u32, names.len() as u32] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for (x, y, z) in hashes {
            for v in [x, y, z] {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        // folder "ships": parent -1, name 0..5
        for v in [u32::MAX, 0, 5] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // file "a.odf": data 0, sizes, name 5..10, raw, folder 0
        for v in [0u32, data.len() as u32, data.len() as u32, 5, 5] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.push(0);
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(names);
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn reads_synthetic_archive() {
        let f = Fpq::from_bytes(tiny()).unwrap();
        let i = f.find("A.ODF").unwrap();
        assert_eq!(f.path(i).unwrap(), "ships/a.odf");
        assert_eq!(f.read(i).unwrap(), b"classLabel = \"starship\"\n");
        assert_eq!(f.find("b.odf"), None);
    }

    #[test]
    fn rejects_truncated() {
        let mut b = tiny();
        b.pop();
        assert!(matches!(Fpq::from_bytes(b), Err(FpqError::Corrupt(_))));
    }

    /// Validates against a real install when STA64_TEST_FPQ points to an archive.
    #[test]
    fn real_archive_if_available() {
        let Ok(p) = std::env::var("STA64_TEST_FPQ") else {
            return;
        };
        let f = Fpq::open(Path::new(&p)).unwrap();
        for i in 0..f.len() {
            let name = f.path(i).unwrap();
            let base = name.rsplit('/').next().unwrap();
            assert_eq!(f.find(base), Some(i), "hash lookup for {name}");
            let data = f.read(i).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(data.len(), f.entry(i).size as usize);
        }
        eprintln!("validated {} entries in {p}", f.len());
    }
}
