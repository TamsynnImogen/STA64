//! Armada I `.zfs` archives (`assets.zfs`, `objtxt.zfs`).
//!
//! Layout, little-endian:
//!
//! - Header, 28 bytes: `"ZFSF"`, version (1), `max_name` (16),
//!   `files_per_block` (100), `file_count`, `key`, `first_block` (offset).
//! - Blocks, chained: a `next_block` offset (0 ends the chain), then
//!   `files_per_block` entries of `name[max_name]` (NUL-padded), `offset`,
//!   `index`, `size`, `time`, `flags`. Unused slots have an empty name.
//! - Data: `size` bytes at `offset`. When `key` is not 0, each whole 4-byte
//!   word is XOR'd with the key (byte `i` with `key.to_le_bytes()[i % 4]`);
//!   the last `size % 4` bytes are stored plain. (Decrypting them too turns a
//!   file's closing `"\r\n` into junk, which is how the rule was found.)
//!   `assets.zfs` has a key; `objtxt.zfs` has 0.
//!
//! Every entry in the retail archives has `flags == 0` (stored). Other flags
//! mean compression this reader does not know; such entries are listed and
//! refuse to read rather than return garbage.
//!
//! The namespace is flat: names carry no folders.

use std::path::Path;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ZfsError {
    #[error("not a ZFS archive")]
    BadMagic,
    #[error("unsupported ZFS version {0}")]
    Version(u32),
    #[error("truncated at offset {0}")]
    Truncated(usize),
    #[error("block chain loops at offset {0}")]
    Loop(usize),
    #[error("{name}: flags {flags:#x} (compressed?) are not supported, or the name is not 1-16 ASCII bytes without folders")]
    Unsupported { name: String, flags: u32 },
    #[error("io: {0}")]
    Io(String),
    #[error("two files named {0} (names compare case-insensitively)")]
    Duplicate(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZfsEntry {
    /// As stored (original case).
    pub name: String,
    pub offset: u32,
    pub size: u32,
    /// Stored timestamp, kept as is.
    pub time: u32,
    pub flags: u32,
}

/// An archive held in memory (retail archives are under 1 MB).
pub struct Zfs {
    data: Vec<u8>,
    key: [u8; 4],
    entries: Vec<ZfsEntry>,
}

impl std::fmt::Debug for Zfs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Zfs").field("entries", &self.entries.len()).finish()
    }
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, ZfsError> {
    let s = b.get(at..at + 4).ok_or(ZfsError::Truncated(at))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

impl Zfs {
    pub fn open(path: &Path) -> Result<Zfs, ZfsError> {
        Zfs::from_bytes(std::fs::read(path).map_err(|e| ZfsError::Io(e.to_string()))?)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Zfs, ZfsError> {
        if data.get(..4) != Some(b"ZFSF".as_slice()) {
            return Err(ZfsError::BadMagic);
        }
        let version = u32_at(&data, 4)?;
        if version != 1 {
            return Err(ZfsError::Version(version));
        }
        let max_name = u32_at(&data, 8)? as usize;
        let per_block = u32_at(&data, 12)? as usize;
        let count = u32_at(&data, 16)? as usize;
        let key = u32_at(&data, 20)?.to_le_bytes();
        let mut block = u32_at(&data, 24)? as usize;
        let slot = max_name + 20;
        let mut entries = Vec::with_capacity(count);
        let mut seen = std::collections::BTreeSet::new();
        while block != 0 && entries.len() < count {
            if !seen.insert(block) {
                return Err(ZfsError::Loop(block));
            }
            let next = u32_at(&data, block)? as usize;
            for i in 0..per_block {
                let at = block + 4 + i * slot;
                let raw = data.get(at..at + max_name).ok_or(ZfsError::Truncated(at))?;
                let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                if end == 0 {
                    continue;
                }
                let name: String = raw[..end].iter().map(|&c| c as char).collect();
                let offset = u32_at(&data, at + max_name)?;
                let size = u32_at(&data, at + max_name + 8)?;
                let time = u32_at(&data, at + max_name + 12)?;
                let flags = u32_at(&data, at + max_name + 16)?;
                if (offset as usize).checked_add(size as usize).is_none_or(|e| e > data.len()) {
                    return Err(ZfsError::Truncated(offset as usize));
                }
                entries.push(ZfsEntry { name, offset, size, time, flags });
            }
            block = next;
        }
        Ok(Zfs { data, key, entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entry(&self, index: usize) -> &ZfsEntry {
        &self.entries[index]
    }

    /// The header key (0 = not encrypted).
    pub fn key(&self) -> u32 {
        u32::from_le_bytes(self.key)
    }

    /// The entry's bytes, decrypted.
    pub fn read(&self, index: usize) -> Result<Vec<u8>, ZfsError> {
        let e = &self.entries[index];
        if e.flags != 0 {
            return Err(ZfsError::Unsupported { name: e.name.clone(), flags: e.flags });
        }
        let raw = &self.data[e.offset as usize..(e.offset + e.size) as usize];
        if self.key == [0; 4] {
            return Ok(raw.to_vec());
        }
        let whole = raw.len() & !3;
        let mut out: Vec<u8> = raw[..whole].iter().enumerate().map(|(i, &c)| c ^ self.key[i & 3]).collect();
        out.extend_from_slice(&raw[whole..]);
        Ok(out)
    }
}

/// One file to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZfsFile {
    /// At most 16 Latin-1 bytes, no folders.
    pub name: String,
    pub data: Vec<u8>,
    /// Timestamp field (the retail files carry one; 0 is fine).
    pub time: u32,
}

/// Files per block and name length, as in every retail archive.
pub const FILES_PER_BLOCK: usize = 100;
pub const MAX_NAME: usize = 16;

/// Write a ZFS v1 archive laid out as the retail ones are: each block's table
/// (next-block offset and 100 slots) followed by its files' data, entries
/// stored (flags 0), whole words XOR'd with `key` when it isn't 0.
pub fn write(files: &[ZfsFile], key: u32) -> Result<Vec<u8>, ZfsError> {
    let slot = MAX_NAME + 20;
    let k = key.to_le_bytes();
    let mut out = Vec::new();
    out.extend_from_slice(b"ZFSF");
    for v in [1u32, MAX_NAME as u32, FILES_PER_BLOCK as u32, files.len() as u32, key, 28] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut seen = std::collections::BTreeSet::new();
    for f in files {
        if f.name.is_empty() || f.name.len() > MAX_NAME || f.name.contains(['/', '\\']) || !f.name.is_ascii() {
            return Err(ZfsError::Unsupported { name: f.name.clone(), flags: 0 });
        }
        if !seen.insert(f.name.to_ascii_lowercase()) {
            return Err(ZfsError::Duplicate(f.name.clone()));
        }
    }
    let blocks: Vec<&[ZfsFile]> = if files.is_empty() { vec![&[][..]] } else { files.chunks(FILES_PER_BLOCK).collect() };
    for (b, chunk) in blocks.iter().enumerate() {
        let table_at = out.len();
        let mut data_at = table_at + 4 + FILES_PER_BLOCK * slot;
        let next = if b + 1 < blocks.len() { data_at + chunk.iter().map(|f| f.data.len()).sum::<usize>() } else { 0 };
        let mut table = Vec::with_capacity(4 + FILES_PER_BLOCK * slot);
        table.extend_from_slice(&(next as u32).to_le_bytes());
        for i in 0..FILES_PER_BLOCK {
            let mut rec = vec![0u8; slot];
            if let Some(f) = chunk.get(i) {
                rec[..f.name.len()].copy_from_slice(f.name.as_bytes());
                let index = (b * FILES_PER_BLOCK + i) as u32;
                for (j, v) in [data_at as u32, index, f.data.len() as u32, f.time, 0].iter().enumerate() {
                    rec[MAX_NAME + j * 4..MAX_NAME + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                data_at += f.data.len();
            }
            table.extend_from_slice(&rec);
        }
        out.extend_from_slice(&table);
        for f in *chunk {
            let whole = if key == 0 { 0 } else { f.data.len() & !3 };
            out.extend(f.data.iter().enumerate().map(|(i, &c)| if i < whole { c ^ k[i & 3] } else { c }));
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build an archive: 16-char names, `per_block` slots per block.
    pub(crate) fn build(files: &[(&str, &[u8])], key: u32, per_block: usize) -> Vec<u8> {
        let max_name = 16;
        let slot = max_name + 20;
        let blocks = files.len().div_ceil(per_block).max(1);
        let table = 28 + blocks * (4 + per_block * slot);
        let mut out = Vec::new();
        out.extend_from_slice(b"ZFSF");
        for v in [1u32, max_name as u32, per_block as u32, files.len() as u32, key, 28] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let mut data_at = table;
        let mut data = Vec::new();
        for b in 0..blocks {
            let next = if b + 1 < blocks { 28 + (b + 1) * (4 + per_block * slot) } else { 0 };
            out.extend_from_slice(&(next as u32).to_le_bytes());
            for i in 0..per_block {
                let mut rec = vec![0u8; slot];
                if let Some((name, bytes)) = files.get(b * per_block + i) {
                    rec[..name.len()].copy_from_slice(name.as_bytes());
                    let fields = [data_at as u32, (b * per_block + i) as u32, bytes.len() as u32, 0x38c6_ca24, 0];
                    for (j, v) in fields.iter().enumerate() {
                        rec[max_name + j * 4..max_name + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
                    }
                    let k = key.to_le_bytes();
                    let whole = bytes.len() & !3;
                    data.extend(bytes.iter().enumerate().map(|(i, &c)| if i < whole { c ^ k[i & 3] } else { c }));
                    data_at += bytes.len();
                }
                out.extend_from_slice(&rec);
            }
        }
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn reads_encrypted_and_plain_over_several_blocks() {
        let files: Vec<(String, Vec<u8>)> =
            // Lengths 19..=25 cover every `len % 4`.
            (0..7).map(|i| (format!("f{i}.odf"), format!("classLabel = \"{}\"\r\n", "x".repeat(i + 1)).into_bytes())).collect();
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        for key in [0, 0x9466_279b] {
            let z = Zfs::from_bytes(build(&refs, key, 3)).unwrap();
            assert_eq!(z.len(), 7);
            for (i, (name, bytes)) in files.iter().enumerate() {
                assert_eq!(&z.entry(i).name, name);
                assert_eq!(&z.read(i).unwrap(), bytes, "key {key:#x}");
            }
        }
    }

    #[test]
    fn tail_bytes_are_plain() {
        // Known plaintext from A1's mdmoon1.odf: 23 bytes, so the last 3 are not XOR'd.
        let key: u32 = 0x9466_279b;
        let plain = b"#include \"mdmoon.odf\"\r\n";
        let k = key.to_le_bytes();
        let mut raw: Vec<u8> = plain.iter().enumerate().map(|(i, &c)| c ^ k[i & 3]).collect();
        raw[20..].copy_from_slice(&plain[20..]);
        let mut b = build(&[("mdmoon1.odf", b"")], key, 4);
        let at = b.len() as u32;
        b.extend_from_slice(&raw);
        let rec = 28 + 4;
        b[rec + 16..rec + 20].copy_from_slice(&at.to_le_bytes());
        b[rec + 24..rec + 28].copy_from_slice(&(raw.len() as u32).to_le_bytes());
        assert_eq!(Zfs::from_bytes(b).unwrap().read(0).unwrap(), plain);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(Zfs::from_bytes(b"FPQ\0rest".to_vec()).unwrap_err(), ZfsError::BadMagic);
        let mut v2 = build(&[("a.tt", b"x")], 0, 4);
        v2[4] = 2;
        assert_eq!(Zfs::from_bytes(v2).unwrap_err(), ZfsError::Version(2));
        let full = build(&[("a.tt", b"hello")], 0, 4);
        assert!(matches!(Zfs::from_bytes(full[..full.len() - 2].to_vec()), Err(ZfsError::Truncated(_))));
        // A block whose next pointer is itself.
        let mut looped = build(&[("a.tt", b"x"), ("b.tt", b"y")], 0, 1);
        looped[28..32].copy_from_slice(&28u32.to_le_bytes());
        assert_eq!(Zfs::from_bytes(looped).unwrap_err(), ZfsError::Loop(28));
    }

    #[test]
    fn compressed_entry_refuses_to_read() {
        let mut b = build(&[("a.odf", b"abc")], 0, 4);
        let flags_at = 28 + 4 + 16 + 16;
        b[flags_at] = 2;
        let z = Zfs::from_bytes(b).unwrap();
        assert_eq!(z.read(0).unwrap_err(), ZfsError::Unsupported { name: "a.odf".into(), flags: 2 });
    }

    #[test]
    fn writer_round_trips_over_blocks() {
        let files: Vec<ZfsFile> =
            (0..250).map(|i| ZfsFile { name: format!("f{i}.odf"), data: format!("n = {i}\r\n").repeat(i % 7).into_bytes(), time: i as u32 }).collect();
        for key in [0, 0x9466_279b] {
            let z = Zfs::from_bytes(write(&files, key).unwrap()).unwrap();
            assert_eq!((z.len(), z.key()), (250, key));
            for (i, f) in files.iter().enumerate() {
                assert_eq!((&z.entry(i).name, z.entry(i).time), (&f.name, f.time));
                assert_eq!(z.read(i).unwrap(), f.data);
            }
        }
        let long = ZfsFile { name: "seventeen_chars.x".into(), data: vec![], time: 0 };
        assert!(write(&[long], 0).is_err());
        let dup = |n: &str| ZfsFile { name: n.into(), data: vec![], time: 0 };
        assert_eq!(write(&[dup("a.tt"), dup("A.TT")], 0).unwrap_err(), ZfsError::Duplicate("A.TT".into()));
    }

    /// With STA64_TEST_ZFS pointing at an archive: every entry reads.
    #[test]
    fn real_archive_if_available() {
        let Ok(p) = std::env::var("STA64_TEST_ZFS") else { return };
        let z = Zfs::open(Path::new(&p)).unwrap();
        let mut by_ext = std::collections::BTreeMap::<String, usize>::new();
        for i in 0..z.len() {
            let bytes = z.read(i).unwrap();
            assert_eq!(bytes.len(), z.entry(i).size as usize);
            let ext = z.entry(i).name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            *by_ext.entry(ext).or_default() += 1;
        }
        eprintln!("{p}: {} entries {by_ext:?}", z.len());
        // Repacking with the same names, times and key gives the same file.
        let files: Vec<ZfsFile> = (0..z.len())
            .map(|i| ZfsFile { name: z.entry(i).name.clone(), data: z.read(i).unwrap(), time: z.entry(i).time })
            .collect();
        let again = write(&files, z.key()).unwrap();
        assert_eq!(again, std::fs::read(&p).unwrap(), "repack is byte-identical");
        eprintln!("repack byte-identical");
    }
}
