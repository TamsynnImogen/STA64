//! Writing FPQ archives (v1 and v2), laid out as FPQEdit lays them out.
//!
//! - Folders in first-appearance order, each after its parent; the name blob
//!   holds every folder name, then every file name, with no separators.
//! - Files and their data in the order given, data contiguous.
//! - Hash table: the next power of two at or above the file count; empty
//!   slots all zero; a name goes in its slot hash's slot or the next free one.
//! - Each entry is LZO1X-compressed (via `lzokay-native`) and stored raw when
//!   that is not smaller, as the originals store tiny files raw.
//! - v1 entries get the size prefix ([`crate::v1_size_prefix_bytes`]) and no
//!   end-of-stream marker, as FPQEdit 3.2.7 writes them; v2 entries are plain
//!   LZO1X streams with the marker.
//!
//! Lookups go by basename alone, so two files with the same basename (any
//! case) would hide one another: that is an error here.

use std::collections::BTreeMap;

use crate::{FpqError, slot_and_hashes, v1_size_prefix_bytes};

/// How entries are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// LZO1X when smaller, else raw (FPQEdit's behaviour).
    Auto,
    /// Everything raw.
    None,
}

fn latin1(s: &str) -> Result<Vec<u8>, FpqError> {
    s.chars()
        .map(|c| u8::try_from(c as u32).map_err(|_| FpqError::Corrupt(format!("{s}: name is not Latin-1"))))
        .collect()
}

/// Build an archive of `version` (1 or 2) from `(path, data)` pairs, paths
/// with `/` between folders (`odf/system/fed.odf`).
pub fn write(version: u32, files: &[(String, Vec<u8>)], compression: Compression) -> Result<Vec<u8>, FpqError> {
    if !(1..=2).contains(&version) {
        return Err(FpqError::Version(version));
    }
    // Folders: path -> index, in first-appearance order.
    let mut folder_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut folders: Vec<(i32, String)> = Vec::new(); // (parent, name)
    let mut file_folder = Vec::with_capacity(files.len());
    let mut seen_base: BTreeMap<String, &str> = BTreeMap::new();
    for (path, _) in files {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let Some((base, dirs)) = parts.split_last() else {
            return Err(FpqError::Corrupt(format!("empty path {path:?}")));
        };
        if let Some(prev) = seen_base.insert(base.to_ascii_lowercase(), path) {
            return Err(FpqError::Corrupt(format!("{path} and {prev} share a basename; lookups would find only one")));
        }
        let mut parent: i32 = -1;
        let mut so_far = String::new();
        for d in dirs {
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(&d.to_ascii_lowercase());
            let i = *folder_index.entry(so_far.clone()).or_insert_with(|| {
                folders.push((parent, (*d).to_owned()));
                folders.len() - 1
            });
            parent = i as i32;
        }
        file_folder.push((parent, (*base).to_owned()));
    }

    // Name blob.
    let mut names = Vec::new();
    let mut folder_names = Vec::with_capacity(folders.len());
    for (_, name) in &folders {
        let b = latin1(name)?;
        folder_names.push((names.len() as u32, b.len() as u32));
        names.extend_from_slice(&b);
    }
    let mut file_names = Vec::with_capacity(files.len());
    for (_, name) in &file_folder {
        let b = latin1(name)?;
        file_names.push((names.len() as u32, b.len() as u32));
        names.extend_from_slice(&b);
    }

    // Data.
    let mut data = Vec::new();
    let mut records = Vec::with_capacity(files.len());
    for (path, bytes) in files {
        let size = u32::try_from(bytes.len()).map_err(|_| FpqError::Corrupt(format!("{path}: over 4 GiB")))?;
        let packed = match compression {
            Compression::None => None,
            Compression::Auto if bytes.is_empty() => None,
            Compression::Auto => {
                let mut z = lzokay_native::compress(bytes).map_err(|e| FpqError::Corrupt(format!("{path}: LZO: {e}")))?;
                if version == 1 {
                    // FPQEdit 3.2.7 writes no end-of-stream marker.
                    if z.ends_with(&[0x11, 0, 0]) {
                        z.truncate(z.len() - 3);
                    }
                    let mut p = v1_size_prefix_bytes(size);
                    p.extend_from_slice(&z);
                    z = p;
                }
                (z.len() < bytes.len()).then_some(z)
            }
        };
        let offset = data.len() as u32;
        let (stored, comp) = match &packed {
            Some(z) => (z.as_slice(), 1u8),
            None => (bytes.as_slice(), 0u8),
        };
        data.extend_from_slice(stored);
        records.push((offset, size, stored.len() as u32, comp));
    }

    // Hash table.
    let n_hash = files.len().max(1).next_power_of_two();
    let mask = n_hash - 1;
    let mut table = vec![(0u32, 0u32, 0u32); n_hash];
    for (i, (_, base)) in file_folder.iter().enumerate() {
        let (slot, a, b) = slot_and_hashes(version, base);
        let mut s = slot as usize & mask;
        while table[s] != (0, 0, 0) {
            s = (s + 1) & mask;
        }
        table[s] = (a, b, i as u32);
    }

    let mut out = Vec::with_capacity(28 + n_hash * 12 + folders.len() * 12 + files.len() * 25 + names.len() + data.len());
    out.extend_from_slice(b"FPQ\0");
    for v in [version, folders.len() as u32, files.len() as u32, n_hash as u32, data.len() as u32, names.len() as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for (a, b, i) in table {
        for v in [a, b, i] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    for ((parent, _), (no, nl)) in folders.iter().zip(&folder_names) {
        out.extend_from_slice(&parent.to_le_bytes());
        out.extend_from_slice(&no.to_le_bytes());
        out.extend_from_slice(&nl.to_le_bytes());
    }
    for (((offset, size, stored, comp), (no, nl)), (folder, _)) in records.iter().zip(&file_names).zip(&file_folder) {
        for v in [*offset, *size, *stored, *no, *nl] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(*comp);
        out.extend_from_slice(&folder.to_le_bytes());
    }
    out.extend_from_slice(&names);
    out.extend_from_slice(&data);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Fpq;

    fn sample() -> Vec<(String, Vec<u8>)> {
        let big: Vec<u8> = (0..5000).flat_map(|i| format!("weapon{i} = \"phaser\"\r\n").into_bytes()).collect();
        vec![
            ("odf/system/fed_akira.odf".into(), b"classlabel = \"craft\"\r\nmaxhealth = 900\r\n".to_vec()),
            ("odf/system/big.odf".into(), big),
            ("odf/other/tiny.odf".into(), b"x".to_vec()),
            ("empty.odf".into(), Vec::new()),
            ("ODF/System/Caps.ODF".into(), b"a = 1\r\n".repeat(50)),
        ]
    }

    #[test]
    fn round_trips_both_versions() {
        for version in [1, 2] {
            for comp in [Compression::Auto, Compression::None] {
                let files = sample();
                let bytes = write(version, &files, comp).unwrap();
                let f = Fpq::from_bytes(bytes).unwrap();
                assert_eq!((f.version(), f.len()), (version, files.len()));
                for (path, data) in &files {
                    let base = path.rsplit('/').next().unwrap();
                    let i = f.find(&base.to_ascii_uppercase()).unwrap_or_else(|| panic!("v{version} find {base}"));
                    assert_eq!(f.path(i).unwrap().to_ascii_lowercase(), path.to_ascii_lowercase());
                    assert_eq!(&f.read(i).unwrap(), data, "v{version} {path}");
                }
                assert_eq!(f.find("missing.odf"), None);
                if comp == Compression::Auto {
                    let big = f.find("big.odf").unwrap();
                    assert_eq!(f.entry(big).compression, 1);
                    assert_eq!(f.entry(f.find("tiny.odf").unwrap()).compression, 0, "not smaller: raw");
                }
            }
        }
    }

    #[test]
    fn layout_matches_fpqedit() {
        let f = Fpq::from_bytes(write(2, &sample(), Compression::Auto).unwrap()).unwrap();
        // odf, odf/system, odf/other; 5 files -> 8 slots.
        assert_eq!((f.folder_count(), f.hash_slots()), (3, 8));
        let offsets: Vec<u32> = (0..f.len()).map(|i| f.entry(i).data_offset).collect();
        assert!(offsets.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn duplicate_basenames_are_refused() {
        let files = vec![("a/x.odf".to_string(), vec![1]), ("b/X.ODF".to_string(), vec![2])];
        assert!(matches!(write(2, &files, Compression::Auto), Err(FpqError::Corrupt(m)) if m.contains("share a basename")));
    }

    #[test]
    fn size_prefix_round_trips() {
        for n in [0u32, 1, 396, 32767, 32768, 522_404, 65_535, 1 << 30] {
            let b = crate::v1_size_prefix_bytes(n);
            assert_eq!(crate::v1_size_prefix(&b), Some((n, b.len())), "{n}");
        }
        assert_eq!(crate::v1_size_prefix_bytes(396), [0x8c, 0x01]);
        assert_eq!(crate::v1_size_prefix_bytes(522_404), [0xa4, 0xf8, 0x0f, 0x00]);
    }
}
