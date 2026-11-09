//! `.sta` signatures (`docs/STA_ARCHIVE.md`, "Signature block"; CON-19).
//!
//! A signature is Ed25519 over the archive's 64-byte header (with
//! `signature_offset` as written) followed by its `content_hash`. The block
//! after the index holds the public key, the signature and the signer's name:
//!
//! | size | field |
//! |---:|---|
//! | 4 | block size (this table, name included) |
//! | 1 | algorithm: 1 = Ed25519 |
//! | 32 | public key |
//! | 64 | signature |
//! | 2 | signer name length, then the name (UTF-8) |
//!
//! [`verify`] gives a [`Status`]: trusted, unknown key, unsigned, or tampered.
//! Tampering anywhere is caught: a changed file fails its SHA-256 (checked
//! when every file is read), a changed index fails `content_hash` at open, and
//! a changed header or `content_hash` fails the signature. The signer's name
//! is not signed; a trusted key is shown under the name in the trust store.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::sta::{HEADER_SIZE, Kind, MAGIC, StaArchive, StaError};

pub const ALG_ED25519: u8 = 1;
const FIXED: usize = 4 + 1 + 32 + 64 + 2;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.trim();
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// A publisher's signing key.
pub struct KeyPair {
    key: SigningKey,
}

impl KeyPair {
    /// A new key from the operating system's randomness.
    pub fn generate() -> Result<KeyPair, StaError> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| StaError::Io(format!("no randomness for a key: {e}")))?;
        Ok(KeyPair::from_seed(seed))
    }

    pub fn from_seed(seed: [u8; 32]) -> KeyPair {
        KeyPair { key: SigningKey::from_bytes(&seed) }
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// Key file text. Keep it secret: whoever has it can sign as you.
    pub fn to_text(&self) -> String {
        format!(
            "# STA64 .sta signing key: keep this file secret.\nsecret-ed25519 {}\npublic-ed25519 {}\n",
            hex(&self.key.to_bytes()),
            hex(&self.public_key())
        )
    }

    pub fn from_text(text: &str) -> Result<KeyPair, StaError> {
        let bad = || StaError::Corrupt("not an STA64 signing key file".into());
        let seed = text.lines().find_map(|l| l.trim().strip_prefix("secret-ed25519 ")).and_then(unhex::<32>).ok_or_else(bad)?;
        let k = KeyPair::from_seed(seed);
        if let Some(p) = text.lines().find_map(|l| l.trim().strip_prefix("public-ed25519 "))
            && unhex::<32>(p) != Some(k.public_key())
        {
            return Err(StaError::Corrupt("key file: public key does not belong to the secret key".into()));
        }
        Ok(k)
    }
}

/// Public keys the player trusts, with the names to show for them.
/// Text form: one `<64 hex digits> <name>` per line, `#` comments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    keys: BTreeMap<[u8; 32], String>,
}

impl TrustStore {
    pub fn parse(text: &str) -> Result<TrustStore, StaError> {
        let mut t = TrustStore::default();
        for (n, line) in text.lines().enumerate() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let (k, name) = l.split_once(char::is_whitespace).unwrap_or((l, ""));
            let key = unhex::<32>(k).ok_or_else(|| StaError::Corrupt(format!("trust store line {}: not a public key", n + 1)))?;
            t.keys.insert(key, name.trim().to_owned());
        }
        Ok(t)
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("# STA64 trusted .sta publishers: <public key> <name>\n");
        for (k, name) in &self.keys {
            s += &format!("{} {name}\n", hex(k));
        }
        s
    }

    pub fn add(&mut self, key: [u8; 32], name: &str) {
        self.keys.insert(key, name.to_owned());
    }

    pub fn remove(&mut self, key: &[u8; 32]) -> bool {
        self.keys.remove(key).is_some()
    }

    pub fn name(&self, key: &[u8; 32]) -> Option<&str> {
        self.keys.get(key).map(String::as_str)
    }
}

/// What a signature check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Signed by a key in the trust store (shown under the store's name).
    Trusted { name: String, key: [u8; 32] },
    /// A valid signature by a key the player hasn't trusted; `signer` is the
    /// name the archive claims (unverified).
    UnknownKey { signer: String, key: [u8; 32] },
    /// No signature block. Normal; the archive still loads.
    Unsigned,
    /// Something doesn't match: damaged or modified.
    Tampered(String),
}

/// A parsed signature block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
    pub signer: String,
}

fn block_bytes(b: &Block) -> Result<Vec<u8>, StaError> {
    let name = b.signer.as_bytes();
    let len = u16::try_from(name.len()).map_err(|_| StaError::Corrupt("signer name over 65535 bytes".into()))?;
    let size = (FIXED + name.len()) as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&size.to_le_bytes());
    out.push(ALG_ED25519);
    out.extend_from_slice(&b.public_key);
    out.extend_from_slice(&b.signature);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(name);
    Ok(out)
}

/// The signature block of `a`, if it has one, checked for shape.
pub fn read_block(a: &StaArchive) -> Result<Option<Block>, StaError> {
    if !a.is_signed() {
        return Ok(None);
    }
    let at = a.signature_offset();
    let bad = |s: &str| StaError::Corrupt(format!("signature block: {s}"));
    let size = u32::from_le_bytes(a.raw(at, 4)?.try_into().unwrap()) as u64;
    if size < FIXED as u64 || at + size != a.file_len() {
        return Err(bad("its size does not reach exactly to the end of the file"));
    }
    let b = a.raw(at, size)?;
    if b[4] != ALG_ED25519 {
        return Err(bad("unknown algorithm"));
    }
    let name_len = u16::from_le_bytes([b[101], b[102]]) as usize;
    if FIXED + name_len != size as usize {
        return Err(bad("signer name length does not fit the block"));
    }
    let signer = String::from_utf8(b[FIXED..].to_vec()).map_err(|_| bad("signer name is not UTF-8"))?;
    Ok(Some(Block { public_key: b[5..37].try_into().unwrap(), signature: b[37..101].try_into().unwrap(), signer }))
}

fn message(header: &[u8; 64], content_hash: &[u8; 32]) -> Vec<u8> {
    let mut m = Vec::with_capacity(96);
    m.extend_from_slice(header);
    m.extend_from_slice(content_hash);
    m
}

/// Signature-only check (no file reads): is the block a valid signature of
/// this header and identity, and by whom.
pub fn check_signature(a: &StaArchive, trust: &TrustStore) -> Status {
    let block = match read_block(a) {
        Ok(None) => return Status::Unsigned,
        Ok(Some(b)) => b,
        Err(e) => return Status::Tampered(e.to_string()),
    };
    let header = match a.header_bytes() {
        Ok(h) => h,
        Err(e) => return Status::Tampered(e.to_string()),
    };
    let Ok(vk) = VerifyingKey::from_bytes(&block.public_key) else {
        return Status::Tampered("signature block: invalid public key".into());
    };
    let sig = Signature::from_bytes(&block.signature);
    if vk.verify_strict(&message(&header, &a.content_hash()), &sig).is_err() {
        return Status::Tampered("signature does not match the header and content".into());
    }
    match trust.name(&block.public_key) {
        Some(name) => Status::Trusted { name: name.to_owned(), key: block.public_key },
        None => Status::UnknownKey { signer: block.signer, key: block.public_key },
    }
}

/// Full check: every file against its SHA-256, then the signature.
pub fn verify(a: &StaArchive, trust: &TrustStore) -> Status {
    for (i, e) in a.entries().iter().enumerate() {
        if e.kind == Kind::File
            && let Err(err) = a.read(i)
        {
            return Status::Tampered(err.to_string());
        }
    }
    check_signature(a, trust)
}

/// [`verify`] a file; one that won't even open as an archive (a changed index,
/// header or block layout) is `Tampered` too.
pub fn verify_path(path: &Path, trust: &TrustStore) -> Status {
    match StaArchive::open(path) {
        Ok(a) => verify(&a, trust),
        Err(StaError::Io(e)) => Status::Tampered(format!("cannot read: {e}")),
        Err(e) => Status::Tampered(e.to_string()),
    }
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// The signed header and block for an archive whose index ends at `index_end`.
fn signed_parts(header: &[u8; 64], index_end: u64, content_hash: &[u8; 32], key: &KeyPair, signer: &str) -> Result<([u8; 64], Vec<u8>), StaError> {
    let mut h = *header;
    h[40..48].copy_from_slice(&index_end.to_le_bytes());
    let sig = key.key.sign(&message(&h, content_hash));
    let block = block_bytes(&Block { public_key: key.public_key(), signature: sig.to_bytes(), signer: signer.to_owned() })?;
    Ok((h, block))
}

/// Sign an archive held in memory (replacing any earlier signature).
pub fn sign_bytes(archive: Vec<u8>, key: &KeyPair, signer: &str) -> Result<Vec<u8>, StaError> {
    let a = StaArchive::from_bytes(archive.clone())?;
    let header = a.header_bytes()?;
    let index_end = u64_at(&header, 16) + u64_at(&header, 24);
    let (h, block) = signed_parts(&header, index_end, &a.content_hash(), key, signer)?;
    let mut out = archive;
    out.truncate(index_end as usize);
    out[..HEADER_SIZE as usize].copy_from_slice(&h);
    out.extend_from_slice(&block);
    Ok(out)
}

/// Sign an archive file in place (replacing any earlier signature), without
/// reading its data: cut any old block, append the new one, rewrite the header.
pub fn sign_file(path: &Path, key: &KeyPair, signer: &str) -> Result<(), StaError> {
    let io = |e: std::io::Error| StaError::Io(e.to_string());
    let a = StaArchive::open(path)?;
    let header = a.header_bytes()?;
    let index_end = u64_at(&header, 16) + u64_at(&header, 24);
    let (h, block) = signed_parts(&header, index_end, &a.content_hash(), key, signer)?;
    drop(a);
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(path).map_err(io)?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic).map_err(io)?;
    if &magic != MAGIC {
        return Err(StaError::BadMagic);
    }
    f.set_len(index_end).map_err(io)?;
    f.seek(SeekFrom::Start(index_end)).map_err(io)?;
    f.write_all(&block).map_err(io)?;
    f.seek(SeekFrom::Start(0)).map_err(io)?;
    f.write_all(&h).map_err(io)?;
    f.sync_all().map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sta_write::{Compression, Content, pack};

    fn archive() -> Vec<u8> {
        pack(
            vec![
                ("manifest.toml".into(), Content::Data(b"[mod]\nid = \"signed\"\n".to_vec())),
                ("odf/ships/fakira.odf".into(), Content::Data(b"classLabel = \"craft\"\r\n".repeat(50))),
                ("assets/raw.bin".into(), Content::Data((0..=255u8).collect())),
                ("odf/ships/fdefiant.odf".into(), Content::Whiteout),
            ],
            Compression::Auto,
        )
        .unwrap()
    }

    fn key(n: u8) -> KeyPair {
        KeyPair::from_seed([n; 32])
    }

    fn status(bytes: Vec<u8>, trust: &TrustStore) -> Status {
        match StaArchive::from_bytes(bytes) {
            Ok(a) => verify(&a, trust),
            Err(e) => Status::Tampered(e.to_string()),
        }
    }

    #[test]
    fn sign_then_verify() {
        let k = key(7);
        let signed = sign_bytes(archive(), &k, "Immy").unwrap();
        let none = TrustStore::default();
        assert_eq!(status(signed.clone(), &none), Status::UnknownKey { signer: "Immy".into(), key: k.public_key() });
        let mut trust = TrustStore::default();
        trust.add(k.public_key(), "Immy (STA64)");
        assert_eq!(status(signed.clone(), &trust), Status::Trusted { name: "Immy (STA64)".into(), key: k.public_key() });
        assert_eq!(status(archive(), &trust), Status::Unsigned);
        // Content and identity are untouched by signing.
        let a = StaArchive::from_bytes(signed).unwrap();
        assert_eq!(a.content_hash(), StaArchive::from_bytes(archive()).unwrap().content_hash());
        assert_eq!(a.read(a.find("odf/ships/fakira.odf").unwrap()).unwrap(), b"classLabel = \"craft\"\r\n".repeat(50));
    }

    #[test]
    fn resigning_replaces_the_signature() {
        let once = sign_bytes(archive(), &key(1), "first").unwrap();
        let twice = sign_bytes(once.clone(), &key(2), "second").unwrap();
        assert_eq!(once.len() - "first".len(), twice.len() - "second".len());
        assert!(matches!(status(twice, &TrustStore::default()), Status::UnknownKey { signer, .. } if signer == "second"));
    }

    /// The done-when: a flipped byte anywhere reports tampered, unless every
    /// file still reads back the same (some bits of a Zstandard frame, such
    /// as its window size, change the encoding but not the content: a
    /// signature protects content, not encoding). Those are counted, and
    /// proven to change no file.
    #[test]
    fn every_flipped_byte_is_tampered() {
        let signed = sign_bytes(archive(), &key(3), "x").unwrap();
        let a = StaArchive::from_bytes(signed.clone()).unwrap();
        let files: Vec<Option<Vec<u8>>> = (0..a.len()).map(|i| a.read(i).ok()).collect();
        let index_at = u64_at(&signed, 16) as usize;
        let sig_at = a.signature_offset() as usize;
        let mut tampered = BTreeMap::new();
        let mut same_content = 0;
        for at in 0..signed.len() {
            let region = match at {
                0..64 => "header",
                _ if at < index_at => "data",
                _ if at < sig_at => "index",
                _ => "signature block",
            };
            // Padding between files belongs to no file; the signer's name is unsigned.
            let padding = region == "data" && a.entries().iter().all(|e| (at as u64) < e.offset || (at as u64) >= e.offset + e.stored_size);
            if padding || at >= sig_at + FIXED {
                continue;
            }
            let mut b = signed.clone();
            b[at] ^= 0x01;
            match status(b.clone(), &TrustStore::default()) {
                Status::Tampered(_) => *tampered.entry(region).or_insert(0) += 1,
                s => {
                    // Only inside a Zstandard frame (a file's, or the index's).
                    assert!(region == "data" || region == "index", "byte {at} ({region}) gave {s:?}");
                    let again = StaArchive::from_bytes(b).unwrap();
                    let same_index = again.entries().iter().zip(a.entries()).all(|(x, y)| (&x.path, x.kind, x.size, x.sha256) == (&y.path, y.kind, y.size, y.sha256));
                    assert!(same_index && again.len() == a.len(), "byte {at}: index changed but {s:?}");
                    let now: Vec<Option<Vec<u8>>> = (0..again.len()).map(|i| again.read(i).ok()).collect();
                    assert_eq!(now, files, "byte {at}: content changed but {s:?}");
                    same_content += 1;
                }
            }
        }
        assert_eq!(tampered.len(), 4, "{tampered:?}");
        eprintln!("tampered by region: {tampered:?}; {same_content} encoding-only flips (content unchanged)");
    }

    #[test]
    fn signing_a_file_in_place() {
        let p = std::env::temp_dir().join(format!("sta_sign_{}.sta", std::process::id()));
        std::fs::write(&p, archive()).unwrap();
        let k = key(9);
        sign_file(&p, &k, "file signer").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), sign_bytes(archive(), &k, "file signer").unwrap());
        sign_file(&p, &key(8), "again").unwrap();
        assert!(matches!(verify_path(&p, &TrustStore::default()), Status::UnknownKey { signer, .. } if signer == "again"));
        let mut b = std::fs::read(&p).unwrap();
        let mid = b.len() / 2;
        b[mid] ^= 0x80;
        std::fs::write(&p, b).unwrap();
        assert!(matches!(verify_path(&p, &TrustStore::default()), Status::Tampered(_)));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn key_and_trust_files_round_trip() {
        let k = KeyPair::generate().unwrap();
        let back = KeyPair::from_text(&k.to_text()).unwrap();
        assert_eq!(back.public_key(), k.public_key());
        let mut wrong = k.to_text();
        wrong = wrong.replace(&hex(&k.public_key()), &hex(&[0u8; 32]));
        assert!(KeyPair::from_text(&wrong).is_err());
        assert!(KeyPair::from_text("hello").is_err());

        let mut t = TrustStore::default();
        t.add(k.public_key(), "Publisher One");
        t.add(key(1).public_key(), "Two");
        let parsed = TrustStore::parse(&t.to_text()).unwrap();
        assert_eq!(parsed, t);
        assert!(TrustStore::parse("zz not-a-key").is_err());
        assert!(parsed.clone().remove(&key(1).public_key()));
    }
}
