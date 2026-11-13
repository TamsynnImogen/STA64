//! The keyed content digest (CON-35, docs/MODDING_DECISIONS.md D9, D10).
//!
//! Peers, replays and snapshots compare one digest of the resolved
//! sim-visible rules. A plain hash can only say "different", so the digest
//! is built from a sorted list of [`DigestEntry`]s, each a key and the
//! canonical text of its value, and two lists can name the first key on
//! which they differ ([`ContentDigest::first_difference`]).
//!
//! Keys (bytewise ascending, unique, at most [`MAX_KEY_BYTES`]):
//!
//! | key | value |
//! |---|---|
//! | `RTS_CFG.h:<NAME>` | a typed `RTS_CFG.h` rule ([`crate::Rules`]) |
//! | `content:features`, `content:profile`, `content:tech-tree` | database-wide switches, lineage rules, whether a tech tree gates building |
//! | `geometry:<unit>` | the unit's collision geometry as the sim uses it |
//! | `race:<name>` | a race's starting stock, crew and units |
//! | `resources:slots`, `resources:aliases` | the resource model's slots and the words that mean them |
//! | `techcap:<name>` | a tech tree capacity list |
//! | `unit:<odf>`, `weapon:<odf>` | every sim-visible value of a definition, its tech tree entry included (`tech=`) |
//!
//! Definition keys (lower-cased ODF names) are part of the comparison: the
//! sim's `ContentKey`s index the name-sorted table, so two databases with
//! the same values under different names would read each other's commands
//! as other units. Display strings (tooltips, slot names, sounds, effects,
//! model and node names) are not. Geometry has its own entry per unit so a
//! refusal names the unit and the value; SIM-32 widens that value (ellipsoid
//! axes, radius, centre offset) without changing the key.
//!
//! Values are text so a refusal can show them: numbers are exact (a
//! [`Fixed`] in exact decimal, [`fixed_text`]), strings are quoted, fields
//! are `name=value` joined by `;`. The overall digest is SHA-256 over the
//! entries ([`ContentDigest::sha256`]); `ContentDb::hash` is its first eight
//! bytes. Nothing here depends on the platform, a `HashMap` order or float
//! formatting.

use std::fmt;

use sha2::{Digest as _, Sha256};
use sta_core::Fixed;

/// The longest key; NET-01's `MAX_RULE_KEY_BYTES`.
pub const MAX_KEY_BYTES: usize = 96;

/// Domain separation for the overall digest; a change to the entry
/// encoding bumps it.
const DOMAIN: &[u8] = b"sta64.rules-digest.v1\0";

/// One resolved rule: its key and the canonical text of its value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DigestEntry {
    pub key: String,
    pub value: String,
}

impl DigestEntry {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> DigestEntry {
        DigestEntry { key: key.into(), value: value.into() }
    }
}

/// Where two digests part: the key and both values (`None`: no such key on
/// that side).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryDiff {
    pub key: String,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

impl EntryDiff {
    /// The first `name=value` field of a structured value that differs, for
    /// a message finer than the key (`hull`, `radius`). `None` when one side
    /// is missing or the values are not field lists.
    pub fn field(&self) -> Option<(String, Option<String>, Option<String>)> {
        let (a, b) = (fields(self.ours.as_deref()?), fields(self.theirs.as_deref()?));
        let names: Vec<&str> = a.iter().chain(&b).map(|f| f.0).collect();
        names.into_iter().find_map(|n| {
            let get = |s: &[(&str, &str)]| s.iter().find(|f| f.0 == n).map(|f| f.1.to_owned());
            let (x, y) = (get(&a), get(&b));
            (x != y).then(|| (n.to_owned(), x, y))
        })
    }
}

impl fmt::Display for EntryDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "absent".into());
        match self.field() {
            Some((name, a, b)) => write!(f, "{} {name}: {} here, {} there", self.key, show(&a), show(&b)),
            None => write!(f, "{}: {} here, {} there", self.key, show(&self.ours), show(&self.theirs)),
        }
    }
}

/// Splits `a=1;b="x;y";c=[..]` at top-level `;` (outside quotes and brackets).
fn fields(v: &str) -> Vec<(&str, &str)> {
    let mut parts: Vec<&str> = Vec::new();
    let (mut depth, mut quoted, mut escaped, mut start) = (0i32, false, false, 0);
    for (i, c) in v.char_indices() {
        if quoted {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => quoted = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            ';' if depth == 0 => {
                parts.push(&v[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&v[start..]);
    parts.into_iter().filter_map(|p| p.split_once('=')).collect()
}

/// The sorted entries of a database's resolved rules and their SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentDigest {
    entries: Vec<DigestEntry>,
    sha: [u8; 32],
}

impl Default for ContentDigest {
    fn default() -> Self {
        ContentDigest::from_entries(Vec::new())
    }
}

impl ContentDigest {
    /// Sorts `entries` bytewise by key and digests them.
    ///
    /// # Panics
    ///
    /// On a duplicate key or a key over [`MAX_KEY_BYTES`]: both are bugs in
    /// whatever produced the entries, and either would let two different
    /// rule sets compare equal or fail on the wire.
    pub fn from_entries(mut entries: Vec<DigestEntry>) -> ContentDigest {
        entries.sort_by(|a, b| a.key.as_bytes().cmp(b.key.as_bytes()));
        if let Some(w) = entries.windows(2).find(|w| w[0].key == w[1].key) {
            panic!("duplicate digest key {:?}", w[0].key);
        }
        if let Some(e) = entries.iter().find(|e| e.key.len() > MAX_KEY_BYTES) {
            panic!("digest key over {MAX_KEY_BYTES} bytes: {:?}", e.key);
        }
        let mut h = Sha256::new();
        h.update(DOMAIN);
        h.update((entries.len() as u64).to_le_bytes());
        for e in &entries {
            for part in [&e.key, &e.value] {
                h.update((part.len() as u64).to_le_bytes());
                h.update(part.as_bytes());
            }
        }
        ContentDigest { entries, sha: h.finalize().into() }
    }

    /// Every entry, sorted bytewise by key.
    pub fn entries(&self) -> &[DigestEntry] {
        &self.entries
    }

    /// The value of `key`, if there is such an entry.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .binary_search_by(|e| e.key.as_bytes().cmp(key.as_bytes()))
            .ok()
            .map(|i| self.entries[i].value.as_str())
    }

    /// SHA-256 over the domain tag, the entry count and each key and value
    /// with its length (all lengths `u64` little-endian).
    pub fn sha256(&self) -> [u8; 32] {
        self.sha
    }

    /// The first eight bytes of [`sha256`](ContentDigest::sha256), little-endian:
    /// `ContentDb::hash`, which the sim digest, replays and snapshots carry.
    pub fn hash64(&self) -> u64 {
        u64::from_le_bytes(self.sha[..8].try_into().expect("eight bytes"))
    }

    /// The first key (bytewise) whose value differs or that only one side has.
    pub fn first_difference(&self, theirs: &ContentDigest) -> Option<EntryDiff> {
        diff_entries(&self.entries, &theirs.entries, true).into_iter().next()
    }

    /// Every differing key, ascending.
    pub fn differences(&self, theirs: &ContentDigest) -> Vec<EntryDiff> {
        diff_entries(&self.entries, &theirs.entries, false)
    }
}

/// Walks two sorted lists.
fn diff_entries(a: &[DigestEntry], b: &[DigestEntry], first: bool) -> Vec<EntryDiff> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        let order = match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) => x.key.as_bytes().cmp(y.key.as_bytes()),
            (Some(_), None) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Greater,
        };
        let d = match order {
            std::cmp::Ordering::Less => {
                i += 1;
                Some(EntryDiff { key: a[i - 1].key.clone(), ours: Some(a[i - 1].value.clone()), theirs: None })
            }
            std::cmp::Ordering::Greater => {
                j += 1;
                Some(EntryDiff { key: b[j - 1].key.clone(), ours: None, theirs: Some(b[j - 1].value.clone()) })
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
                (a[i - 1].value != b[j - 1].value).then(|| EntryDiff {
                    key: a[i - 1].key.clone(),
                    ours: Some(a[i - 1].value.clone()),
                    theirs: Some(b[j - 1].value.clone()),
                })
            }
        };
        if let Some(d) = d {
            out.push(d);
            if first {
                break;
            }
        }
    }
    out
}

/// `prefix` and `name` as one key. A name that would make the key longer
/// than [`MAX_KEY_BYTES`] is cut and ends in `#` and 16 hex digits of its
/// SHA-256, so keys stay unique and within the bound.
pub fn key(prefix: &str, name: &str) -> String {
    if prefix.len() + name.len() <= MAX_KEY_BYTES {
        return format!("{prefix}{name}");
    }
    let tail: String = Sha256::digest(name.as_bytes())[..8].iter().map(|b| format!("{b:02x}")).collect();
    let mut cut = MAX_KEY_BYTES - prefix.len() - 1 - tail.len();
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{prefix}{}#{tail}", &name[..cut])
}

/// A [`Fixed`] as the shortest decimal that reads back to exactly its bits
/// (`12`, `-12.5`, `0.800000012` for the header's `0.8` read as `f32`):
/// with `k` fraction digits `d`, the bits are `d * 2^32 / 10^k` rounded half
/// up. At most ten digits are ever needed (10^-10 is under half of 2^-32).
/// Integer arithmetic only, so every platform renders the same text; and as
/// the text determines the bits, different values never render alike.
pub fn fixed_text(v: Fixed) -> String {
    const ONE: u128 = 1 << 32;
    let bits = i128::from(v.to_bits());
    let (sign, mag) = if bits < 0 { ("-", bits.unsigned_abs()) } else { ("", bits as u128) };
    let int = mag >> 32;
    let frac = mag & (ONE - 1);
    if frac == 0 {
        return format!("{sign}{int}");
    }
    // `n / den` rounded half up.
    let round = |n: u128, den: u128| (2 * n + den) / (2 * den);
    for k in 1..=10u32 {
        let scale = 10u128.pow(k);
        let d = round(frac * scale, ONE);
        if d < scale && round(d * ONE, scale) == frac {
            return format!("{sign}{int}.{d:0width$}", width = k as usize);
        }
    }
    unreachable!("ten fraction digits always read back")
}

/// `s` in double quotes, with `"` and `\` escaped.
pub fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Builds a canonical `name=value;...` value.
#[derive(Debug, Default)]
pub struct Canon(String);

impl Canon {
    pub fn new() -> Canon {
        Canon::default()
    }

    /// A field whose value is already canonical text.
    pub fn raw(&mut self, name: &str, value: &str) -> &mut Canon {
        if !self.0.is_empty() {
            self.0.push(';');
        }
        self.0.push_str(name);
        self.0.push('=');
        self.0.push_str(value);
        self
    }

    pub fn fixed(&mut self, name: &str, v: Fixed) -> &mut Canon {
        self.raw(name, &fixed_text(v))
    }

    pub fn int(&mut self, name: &str, v: impl Into<i128>) -> &mut Canon {
        self.raw(name, &v.into().to_string())
    }

    pub fn flag(&mut self, name: &str, v: bool) -> &mut Canon {
        self.raw(name, if v { "1" } else { "0" })
    }

    pub fn text(&mut self, name: &str, v: &str) -> &mut Canon {
        self.raw(name, &quoted(v))
    }

    /// `[a,b,...]` of items already canonical.
    pub fn list(&mut self, name: &str, items: impl IntoIterator<Item = String>) -> &mut Canon {
        let items: Vec<String> = items.into_iter().collect();
        self.raw(name, &format!("[{}]", items.join(",")))
    }

    /// An optional value: `-` when absent.
    pub fn opt_fixed(&mut self, name: &str, v: Option<Fixed>) -> &mut Canon {
        match v {
            Some(v) => self.fixed(name, v),
            None => self.raw(name, "-"),
        }
    }

    pub fn finish(&mut self) -> String {
        std::mem::take(&mut self.0)
    }
}

/// A canonical value that is a list of bracketed field lists: `{a=1;b=2}`.
pub fn group(c: &mut Canon) -> String {
    format!("{{{}}}", c.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(k: &str, v: &str) -> DigestEntry {
        DigestEntry::new(k, v)
    }

    /// Reads [`fixed_text`] back, by its own rule.
    fn parse(t: &str) -> i64 {
        let (neg, t) = t.strip_prefix('-').map_or((false, t), |t| (true, t));
        let (int, frac) = t.split_once('.').unwrap_or((t, ""));
        let scale = 10u128.pow(frac.len() as u32);
        let d: u128 = if frac.is_empty() { 0 } else { frac.parse().unwrap() };
        let mag = (int.parse::<u128>().unwrap() << 32) + (2 * d * (1 << 32) + scale) / (2 * scale);
        if neg { (mag as i128).wrapping_neg() as i64 } else { mag as i64 }
    }

    #[test]
    fn fixed_text_is_short_and_reads_back_exactly() {
        assert_eq!(fixed_text(Fixed::from_int(12)), "12");
        assert_eq!(fixed_text(Fixed::from_f64(-12.5)), "-12.5");
        assert_eq!(fixed_text(Fixed::from_f64(0.8f32 as f64)), "0.800000012");
        assert_eq!(fixed_text(Fixed::from_bits(1)), "0.0000000002");
        assert_eq!(fixed_text(Fixed::from_bits(-1)), "-0.0000000002");
        assert_eq!(fixed_text(Fixed::from_bits(i64::MIN)), "-2147483648");
        assert_eq!(fixed_text(Fixed::from_bits(i64::MAX)), "2147483647.9999999998");
        // Every value reads back to its own bits, so none render alike:
        // neighbours of awkward values, and a spread over the range.
        let mut probes: Vec<i64> = vec![i64::MIN, i64::MIN + 1, i64::MAX, -1, 0, 1, 2, 3];
        for base in [Fixed::from_f64(0.9f32 as f64).to_bits(), Fixed::from_f64(8.660254).to_bits(), 1 << 31, (1 << 32) - 1] {
            probes.extend((-3..=3).map(|d| base + d));
        }
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            probes.push(x as i64);
        }
        for b in probes {
            let t = fixed_text(Fixed::from_bits(b));
            assert_eq!(parse(&t), b, "{b} -> {t}");
        }
    }

    #[test]
    fn digest_ignores_input_order_and_names_the_first_difference() {
        let a = ContentDigest::from_entries(vec![e("b", "2"), e("a", "1"), e("c", "3")]);
        let b = ContentDigest::from_entries(vec![e("a", "1"), e("c", "3"), e("b", "2")]);
        assert_eq!(a, b);
        assert_eq!(a.hash64(), b.hash64());
        assert_eq!(a.get("b"), Some("2"));
        let c = ContentDigest::from_entries(vec![e("a", "1"), e("b", "9"), e("d", "4")]);
        assert_ne!(a.sha256(), c.sha256());
        let d = a.first_difference(&c).unwrap();
        assert_eq!((d.key.as_str(), d.ours.as_deref(), d.theirs.as_deref()), ("b", Some("2"), Some("9")));
        let all: Vec<_> = a.differences(&c).into_iter().map(|d| (d.key, d.ours, d.theirs)).collect();
        assert_eq!(all, [
            ("b".into(), Some("2".into()), Some("9".into())),
            ("c".into(), Some("3".into()), None),
            ("d".into(), None, Some("4".into())),
        ]);
        assert_eq!(a.first_difference(&b), None);
    }

    #[test]
    fn entry_boundaries_are_part_of_the_digest() {
        // Moving bytes between key and value, or between entries, changes it.
        let a = ContentDigest::from_entries(vec![e("ab", "c")]);
        let b = ContentDigest::from_entries(vec![e("a", "bc")]);
        assert_ne!(a.sha256(), b.sha256());
        let c = ContentDigest::from_entries(vec![e("a", ""), e("b", "")]);
        let d = ContentDigest::from_entries(vec![e("a", "b")]);
        assert_ne!(c.sha256(), d.sha256());
    }

    #[test]
    #[should_panic(expected = "duplicate digest key")]
    fn duplicate_keys_are_a_bug() {
        ContentDigest::from_entries(vec![e("a", "1"), e("a", "2")]);
    }

    #[test]
    fn long_names_stay_unique_and_bounded() {
        let long_a = "x".repeat(200);
        let long_b = format!("{}y", "x".repeat(199));
        let (ka, kb) = (key("unit:", &long_a), key("unit:", &long_b));
        assert!(ka.len() <= MAX_KEY_BYTES && kb.len() <= MAX_KEY_BYTES);
        assert_ne!(ka, kb);
        assert_eq!(key("unit:", "fdefiant"), "unit:fdefiant");
        // A cut never splits a character.
        let wide = "é".repeat(80);
        assert!(key("unit:", &wide).len() <= MAX_KEY_BYTES);
    }

    #[test]
    fn a_difference_names_the_field() {
        let d = EntryDiff {
            key: "unit:fdefiant".into(),
            ours: Some("class=\"craft;x\";hull=1;list=[{a=1;b=2}]".into()),
            theirs: Some("class=\"craft;x\";hull=2;list=[{a=1;b=2}]".into()),
        };
        assert_eq!(d.field(), Some(("hull".into(), Some("1".into()), Some("2".into()))));
        assert_eq!(d.to_string(), "unit:fdefiant hull: 1 here, 2 there");
        let gone = EntryDiff { key: "RTS_CFG.h:X".into(), ours: Some("3".into()), theirs: None };
        assert_eq!(gone.to_string(), "RTS_CFG.h:X: 3 here, absent there");
    }

    #[test]
    fn quoting_is_injective() {
        assert_eq!(quoted("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_ne!(quoted("a\",\"b"), format!("{},{}", quoted("a"), quoted("b")));
    }
}
