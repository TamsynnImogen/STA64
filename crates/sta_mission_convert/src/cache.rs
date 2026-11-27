//! The conversion cache (LUA-15f, plan step S9a): what one mission's conversion produced, kept in
//! a folder (the user data directory's, chosen by the caller) so that the next run of the same
//! input does not convert again.
//!
//! # Key
//!
//! [`KeyBuilder`] hashes (SHA-256) the converter version ([`crate::CONVERTER_VERSION`]), a kind
//! label, and every input the output depends on, each part length-prefixed so that parts cannot
//! run into each other: for a mission, the bytes of its DLL files, the engine files that name the
//! engine calls (Armada II) and the names that reach the header. A change of the version or of
//! any input is another key, so the old entry is simply never asked for again (it stays in the
//! folder until the caller clears it).
//!
//! # Entry
//!
//! An [`Entry`] is either a converted mission (the Lua bytes and the console lines the converter
//! printed for it, so that a hit replays the run) or a failure (the message), recorded so that a
//! mission that does not convert is not tried again and does not stop the rest. The file holds a
//! magic, the key, the payload and a SHA-256 over everything before it: a file that is truncated,
//! changed, renamed to another key or not ours is a miss, never an error, and the next
//! [`Cache::put`] replaces it. Entries are written to a temporary name and renamed into place.
//!
//! Entries hold converted game text: the cache folder is as private as the converted output.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"STA64CC1";
const TAG_CONVERTED: u8 = 1;
const TAG_FAILED: u8 = 2;

/// Builds the key of an entry from the converter version and the inputs.
pub struct KeyBuilder(Sha256);

impl KeyBuilder {
    /// `kind` says what is converted (`"a2"`, `"a1"`); the converter version is part of every key.
    pub fn new(kind: &str) -> Self {
        let mut h = Sha256::new();
        h.update(b"sta64-convert-cache-key");
        h.update(crate::CONVERTER_VERSION.to_le_bytes());
        Self(h).part(kind.as_bytes())
    }

    /// One input: its bytes, or a name.
    pub fn part(mut self, bytes: &[u8]) -> Self {
        self.0.update((bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
        self
    }

    /// The key: 64 lower-case hex digits.
    pub fn finish(self) -> String {
        hex(&self.0.finalize())
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// What the cache keeps of one mission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// The Lua file and the console lines of the run that made it.
    Converted { source: Vec<u8>, console: Vec<String> },
    /// The converter's message for a mission that does not convert.
    Failed { message: String },
}

/// A folder of entries, one file per key.
#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u64).to_le_bytes());
    out.extend_from_slice(b);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = usize::try_from(self.u64()?).ok()?;
        self.take(n)
    }

    fn string(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?.to_vec()).ok()
    }
}

fn encode(key: &str, e: &Entry) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(MAGIC);
    put_bytes(&mut o, key.as_bytes());
    match e {
        Entry::Converted { source, console } => {
            o.push(TAG_CONVERTED);
            put_bytes(&mut o, source);
            o.extend_from_slice(&(console.len() as u64).to_le_bytes());
            for l in console {
                put_bytes(&mut o, l.as_bytes());
            }
        }
        Entry::Failed { message } => {
            o.push(TAG_FAILED);
            put_bytes(&mut o, message.as_bytes());
        }
    }
    let sum = Sha256::digest(&o);
    o.extend_from_slice(&sum);
    o
}

fn decode(key: &str, data: &[u8]) -> Option<Entry> {
    if data.len() < MAGIC.len() + 32 {
        return None;
    }
    let (body, sum) = data.split_at(data.len() - 32);
    if Sha256::digest(body).as_slice() != sum {
        return None;
    }
    let mut r = Reader(body);
    if r.take(MAGIC.len())? != MAGIC || r.bytes()? != key.as_bytes() {
        return None;
    }
    let e = match r.take(1)?[0] {
        TAG_CONVERTED => {
            let source = r.bytes()?.to_vec();
            let n = usize::try_from(r.u64()?).ok()?;
            let mut console = Vec::new();
            for _ in 0..n {
                console.push(r.string()?);
            }
            Entry::Converted { source, console }
        }
        TAG_FAILED => Entry::Failed { message: r.string()? },
        _ => return None,
    };
    r.0.is_empty().then_some(e)
}

impl Cache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.entry"))
    }

    /// The entry for `key`, or `None` when there is none or it is damaged, renamed, truncated or
    /// not ours.
    pub fn get(&self, key: &str) -> Option<Entry> {
        if !valid_key(key) {
            return None;
        }
        decode(key, &fs::read(self.path(key)).ok()?)
    }

    /// Stores `e` under `key`, replacing what is there (a damaged entry included).
    pub fn put(&self, key: &str, e: &Entry) -> io::Result<()> {
        if !valid_key(key) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a cache key"));
        }
        fs::create_dir_all(&self.dir)?;
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let tmp = self.dir.join(format!(
            "{key}.tmp-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::write(&tmp, encode(key, e))?;
        fs::rename(&tmp, self.path(key)).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }
}
