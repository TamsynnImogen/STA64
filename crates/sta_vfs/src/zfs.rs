//! Armada I `.zfs` archives (`assets.zfs`, `objtxt.zfs`).
//!
//! Layout, little-endian:
//!
//! - Header, 28 bytes: `"ZFSF"`, version (1), `max_name` (16),
//!   `files_per_block` (100), `file_count`, `key`, `first_block` (offset).
//! - Blocks, chained: a `next_block` offset (0 ends the chain), then
//!   `files_per_block` entries of `name[max_name]` (NUL-padded), `offset`,
//!   `index`, `size`, `time`, `flags`. Unused slots have an empty name.
//! - Data: `size` bytes at `offset`. When `key` is not 0 every byte is XOR'd
//!   with the key's bytes in turn (`key.to_le_bytes()[i % 4]`, `i` counted
//!   from the start of the file's data). `assets.zfs` has a key;
//!   `objtxt.zfs` has 0.
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
    #[error("{name}: flags {flags:#x} (compressed?) are not supported")]
    Unsupported { name: String, flags: u32 },
    #[error("io: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZfsEntry {
    /// As stored (original case).
    pub name: String,
    pub offset: u32,
    pub size: u32,
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
                let flags = u32_at(&data, at + max_name + 16)?;
                if (offset as usize).checked_add(size as usize).is_none_or(|e| e > data.len()) {
                    return Err(ZfsError::Truncated(offset as usize));
                }
                entries.push(ZfsEntry { name, offset, size, flags });
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
        Ok(raw.iter().enumerate().map(|(i, &c)| c ^ self.key[i & 3]).collect())
    }
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
                    data.extend(bytes.iter().enumerate().map(|(i, &c)| if key == 0 { c } else { c ^ k[i & 3] }));
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
            (0..7).map(|i| (format!("f{i}.odf"), format!("classLabel = \"x{i}\"\r\n").into_bytes())).collect();
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
    }
}
