//! The folder key of an install (LUA-15j): the one rule for the per-install folders the mission
//! conversion uses below the user data folder (`converted/<key>` and `convert-cache/<key>`). The
//! converter's command line and library (`sta_mission_convert`), the game's mount
//! (`sta_mission::converted`) and the client (`platform::converted_missions_dir`) all call
//! [`key`]; none of them has a copy of the rule.
//!
//! The key is `<slug>-<hash>`, then `+<slug of the mod>` for a mod (ADR 0004 D6 keeps the mod part):
//!
//! * `slug` is the install folder's own name, lower-cased, every run of characters other than
//!   `a-z` and `0-9` one `-`, trimmed; `install` when nothing is left.
//! * `hash` is the first [`HASH_LEN`] lower-case hex digits of the FNV-1a 64-bit hash of the
//!   *normal form* of the resolved install path ([`normal_form`]), so two installs with one folder
//!   name in two places get two folders. FNV-1a is fixed by its definition (no seed, no std
//!   hasher), so the value is the same on every platform and Rust version.
//!
//! The normal form of the path: the path resolved ([`resolve`]: links followed, made absolute;
//! a path that cannot be canonicalised is made absolute and its `.` and `..` removed
//! lexically), as UTF-8 text (invalid bytes replaced), without a trailing separator; on Windows
//! also without the `\\?\` prefix, with `/` for `\` and in lower case (the file systems there do
//! not tell cases apart). Other systems keep the case (macOS keeps the case the file system
//! reports, which is the case on disk).
//!
//! The Python tools in `tools/` keep the old rule (the slug alone) and are superseded.

use std::path::{Component, Path, PathBuf};

/// How many hex digits of the hash the key carries (32 bits: two installs on one machine
/// colliding is about one in four billion).
pub const HASH_LEN: usize = 8;

/// FNV-1a, 64 bit (the published offset basis and prime).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `slug`: lower-case, every run of other characters one `-`, trimmed; `install` when nothing is
/// left.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            out.push(c);
            gap = false;
        } else {
            gap = true;
        }
    }
    if out.is_empty() {
        "install".to_owned()
    } else {
        out
    }
}

/// The path as the key sees it before the text rules of [`normal_form`]: links followed and made
/// absolute; a path that cannot be canonicalised (it does not exist) is made absolute, then
/// cleaned of `.` and `..` lexically; if even that fails the path as given.
pub fn resolve(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The text of a resolved path the hash is taken over. `windows` selects the Windows rules
/// (prefix, separators, case); the real key passes `cfg!(windows)`, tests pass both.
pub fn normal_form(path: &str, windows: bool) -> String {
    let mut s = path.to_owned();
    if windows {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            s = format!(r"\\{rest}");
        } else if let Some(rest) = s.strip_prefix(r"\\?\") {
            s = rest.to_owned();
        }
        s = s.replace('\\', "/").to_lowercase();
    }
    // no trailing separator, but a root stays a root (`/`, `c:/`)
    let keep = if windows && s.len() >= 2 && s.as_bytes()[1] == b':' { 3 } else { 1 };
    while s.len() > keep && s.ends_with('/') {
        s.pop();
    }
    s
}

/// The hash part of a key for a normal form: [`HASH_LEN`] hex digits.
pub fn hash_of(normal: &str) -> String {
    let full = format!("{:016x}", fnv1a64(normal.as_bytes()));
    full[..HASH_LEN].to_owned()
}

/// The hash part of the key of an install folder: [`resolve`], [`normal_form`], [`hash_of`].
pub fn path_hash(install_root: &Path) -> String {
    hash_of(&normal_form(&resolve(install_root).to_string_lossy(), cfg!(windows)))
}

/// The longest slug a key carries (the folder name part and the mod part each): a longer name is
/// cut (the slug is ASCII, so on a character boundary), a `-` left at the cut is trimmed. The
/// hash keeps two cut names apart. A key is at most `2 * MAX_SLUG + 1 + HASH_LEN + 1` bytes.
pub const MAX_SLUG: usize = 48;

/// [`slug`] cut to [`MAX_SLUG`] characters.
pub fn capped_slug(s: &str) -> String {
    let mut k = slug(s);
    k.truncate(MAX_SLUG);
    while k.ends_with('-') && k.len() > 1 {
        k.pop();
    }
    k
}

/// The folder key of an install (and mod): `<slug>-<hash>` and `+<slug of the mod>` for a mod,
/// always one safe path component.
///
/// One install normally has one key. If the path cannot be canonicalised on one run (an
/// unreadable parent folder) the lexical fallback may give it a second key: a duplicate
/// conversion, never two installs sharing a key.
pub fn key(install_root: &Path, active_mod: Option<&str>) -> String {
    let real = resolve(install_root);
    let name = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let hash = hash_of(&normal_form(&real.to_string_lossy(), cfg!(windows)));
    let mut k = format!("{}-{hash}", capped_slug(&name));
    if let Some(m) = active_mod.filter(|m| !m.is_empty()) {
        k.push('+');
        k.push_str(&capped_slug(m));
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_has_the_published_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn slug_is_the_old_rule() {
        assert_eq!(slug("Star Trek Armada II (Stock)"), "star-trek-armada-ii-stock");
        assert_eq!(slug("@@@"), "install");
        assert_eq!(slug("--Fleet  Ops 4.0--"), "fleet-ops-4-0");
    }

    #[test]
    fn normal_form_unix_keeps_case_and_drops_a_trailing_separator() {
        assert_eq!(normal_form("/Games/Star Trek Armada II/", false), "/Games/Star Trek Armada II");
        assert_eq!(normal_form("/", false), "/");
        assert_eq!(normal_form(r"/odd\name", false), r"/odd\name");
    }

    #[test]
    fn normal_form_windows_folds_prefix_separators_and_case() {
        let want = "d:/games/star trek armada ii";
        assert_eq!(normal_form(r"D:\Games\Star Trek Armada II", true), want);
        assert_eq!(normal_form(r"\\?\D:\Games\Star Trek Armada II\", true), want);
        assert_eq!(normal_form("d:/GAMES/star trek armada ii/", true), want);
        assert_eq!(normal_form(r"\\?\UNC\Host\Share\Armada", true), "//host/share/armada");
        assert_eq!(normal_form(r"C:\", true), "c:/");
        assert_eq!(hash_of(&normal_form(r"D:\Games\Star Trek Armada II", true)), hash_of(want));
    }

    #[test]
    fn hash_golden_for_an_authored_path() {
        // The same text gives the same digits on every platform and Rust version (FNV-1a 64).
        assert_eq!(hash_of("/authored/Star Trek Armada II"), GOLDEN_UNIX);
        assert_eq!(hash_of("d:/games/star trek armada ii"), GOLDEN_WINDOWS);
        assert_eq!(hash_of("/authored/Star Trek Armada II").len(), HASH_LEN);
        // through the normal form: every spelling of the Windows path is the one golden
        for spelling in [r"D:\Games\Star Trek Armada II", r"\\?\d:\GAMES\STAR TREK ARMADA II\", "d:/games/star trek armada ii/"] {
            assert_eq!(hash_of(&normal_form(spelling, true)), GOLDEN_WINDOWS, "{spelling}");
        }
        assert_eq!(hash_of(&normal_form("/authored/Star Trek Armada II/", false)), GOLDEN_UNIX);
    }

    const GOLDEN_UNIX: &str = "e3e919c9";
    const GOLDEN_WINDOWS: &str = "dfebe654";

    #[test]
    fn one_folder_name_in_two_places_gives_two_keys() {
        let tmp = std::env::temp_dir().join(format!("sta64-install-key-{}", std::process::id()));
        let (a, b) = (tmp.join("one").join("Armada II"), tmp.join("two").join("Armada II"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let (ka, kb) = (key(&a, None), key(&b, None));
        assert_ne!(ka, kb);
        assert!(ka.starts_with("armada-ii-") && kb.starts_with("armada-ii-"));
        assert_eq!(ka, key(&a, None), "the same path, the same key");
        // a mod adds its slug after the hash, the hash does not change
        assert_eq!(key(&a, Some("Fleet Ops 4.0")), format!("{ka}+fleet-ops-4-0"));
        assert_eq!(key(&a, Some("")), ka);
        // `.` and a path through `..` are the same install
        assert_eq!(key(&a.join("..").join("Armada II"), None), ka);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn an_over_long_name_is_cut_and_the_key_stays_one_short_component() {
        let long = "Star Trek Armada II ".repeat(20);
        let cut = capped_slug(&long);
        assert_eq!(cut, "star-trek-armada-ii-star-trek-armada-ii-star-tre");
        assert!(cut.len() <= MAX_SLUG);
        // a cut that lands on a separator loses it
        assert_eq!(capped_slug(&format!("{}-x", "a".repeat(MAX_SLUG - 1))), "a".repeat(MAX_SLUG - 1));
        let p = format!("/nonexistent/sta64-key/{long}");
        let k = key(Path::new(&p), Some(&long));
        assert!(k.len() <= 2 * MAX_SLUG + 1 + HASH_LEN + 1, "{k}");
        assert!(k.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'+'), "{k}");
        // two long names that share their first characters stay apart by the hash
        let q = format!("/nonexistent/sta64-key/{long}x");
        assert_ne!(key(Path::new(&q), None), key(Path::new(&p), None));
        // the golden of the over-long case, through the normal form (the key itself would differ
        // on Windows, where the path is made absolute first)
        assert_eq!(format!("{cut}-{}", hash_of(&normal_form(&p, false))), format!("{cut}-{GOLDEN_LONG}"));
        assert!(key(Path::new(&p), None).starts_with(&format!("{cut}-")));
    }

    const GOLDEN_LONG: &str = "d97451f7";

    #[test]
    fn a_path_that_does_not_exist_still_has_a_key() {
        let p = Path::new("/nonexistent/sta64-key/Armada II");
        let k = key(p, None);
        assert!(k.starts_with("armada-ii-"), "{k}");
        assert_eq!(k.len(), "armada-ii-".len() + HASH_LEN);
        assert_eq!(key(Path::new("/nonexistent/sta64-key/x/../Armada II"), None), k);
    }
}
