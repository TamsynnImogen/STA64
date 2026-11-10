//! Language packs: `Dynamic_Localized_Strings.h` and `label.map` (CON-11).
//!
//! # `Dynamic_Localized_Strings.h`
//!
//! A C source file whose table is a flat list of `"KEY", "text"` pairs inside
//! `String_Table_Entry translations[MAX_STRINGS] = { ... };`. Every install
//! (Armada I, II, Fleet Operations, Dominion War, the FO mods) ships one;
//! none ships per-language variants, a translation replaces the English file.
//! What the real files contain, and what [`StringTable::parse`] accepts:
//!
//! * `//` and `/* */` comments, `#define` lines and the struct declaration
//!   (none contain string literals, so they are skipped);
//! * pairs split across lines or several to a line, the last pair with no
//!   trailing comma;
//! * literals that span lines (the FO tooltips put `(Hotkey: A)` on a second
//!   line inside the quotes): the line break stays in the text, as `\n`;
//! * the usual C escapes (`\n \t \" \\ ...`), though no shipped file uses one;
//! * Windows-1252 bytes (FO's file has curly quotes and ellipses). A file that
//!   is valid UTF-8 is read as such, anything else as Windows-1252.
//!
//! A duplicate key keeps its first text, as the engine's linear scan would.
//! A dangling key with no text is dropped and noted in
//! [`StringTable::warnings`].
//!
//! # Layering
//!
//! [`StringTable::load`] reads the file from **every** VFS layer and merges them
//! lowest first, so a mod that lists only a few keys extends the base strings
//! and a mod that restates a key overrides it.
//!
//! # `label.map`
//!
//! `section { + key {Text} }`, `#` comments. [`LabelMap`] keeps every
//! `+ key {text}` entry (keys case-insensitive, several texts per key); the
//! mission resolver's `sta_mission::Labels` is a thin wrapper over it.

use std::collections::HashMap;

use sta_vfs::Vfs;

/// The file name of the string table, as the VFS knows it.
pub const STRINGS_FILE: &str = "Dynamic_Localized_Strings.h";
/// The file name of the label map.
pub const LABEL_FILE: &str = "label.map";

/// Windows-1252 code points for bytes 0x80..=0x9F (the rest equal Latin-1).
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}', '\u{017D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{9D}', '\u{017E}', '\u{0178}',
];

/// Decodes a game text file: UTF-8 when it is valid, else Windows-1252.
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes
            .iter()
            .map(|&b| match b {
                0x80..=0x9F => CP1252_HIGH[(b - 0x80) as usize],
                _ => b as char,
            })
            .collect(),
    }
}

/// Every layer's copy of `path`, lowest priority first, decoded.
fn layers_low_to_high(vfs: &Vfs, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (name, source) in vfs.resolve_all(path).into_iter().rev() {
        let Some(layer) = vfs.layer_index(name) else {
            continue;
        };
        if let Ok(bytes) = vfs.read_from(layer, source) {
            out.push(decode_text(&bytes));
        }
    }
    out
}

/// `Dynamic_Localized_Strings.h`: game key -> text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StringTable {
    map: HashMap<String, String>,
    warnings: Vec<String>,
}

impl StringTable {
    /// Parses the text of a strings header (already decoded).
    pub fn parse(text: &str) -> StringTable {
        let mut literals = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '/' if chars.peek() == Some(&'/') => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    let mut prev = ' ';
                    for c in chars.by_ref() {
                        if prev == '*' && c == '/' {
                            break;
                        }
                        prev = c;
                    }
                }
                '"' => literals.push(read_literal(&mut chars)),
                _ => {}
            }
        }
        let mut table = StringTable::default();
        let mut it = literals.into_iter();
        while let Some(key) = it.next() {
            match it.next() {
                Some(value) => {
                    table.map.entry(key).or_insert(value);
                }
                None => table.warnings.push(format!("key {key:?} has no text")),
            }
        }
        table
    }

    /// Parses raw file bytes (UTF-8 or Windows-1252).
    pub fn parse_bytes(bytes: &[u8]) -> StringTable {
        StringTable::parse(&decode_text(bytes))
    }

    /// The table of every layer of `vfs` that has the file, higher layers
    /// overriding lower ones; empty when no layer has it.
    pub fn load(vfs: &Vfs) -> StringTable {
        let mut table = StringTable::default();
        for text in layers_low_to_high(vfs, STRINGS_FILE) {
            table.overlay(StringTable::parse(&text));
        }
        table
    }

    /// Adds `top`'s entries over this table's.
    pub fn overlay(&mut self, top: StringTable) {
        self.map.extend(top.map);
        self.warnings.extend(top.warnings);
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }

    /// The text for `key`, or `key` itself (the engine's `ENGLISH 1` fallback
    /// shows the English key when there is no translation).
    pub fn get_or_key<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).unwrap_or(key)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Problems found while parsing (a dangling key, ...).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Reads a string literal whose opening quote is consumed. Line breaks are
/// normalised to `\n`; C escapes are resolved; an unterminated literal ends at EOF.
fn read_literal(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    s.push('\n');
                }
            }
            '\\' => match chars.next() {
                Some('n') => s.push('\n'),
                Some('t') => s.push('\t'),
                Some('r' | '0') => {}
                Some('\r') => {
                    chars.next_if_eq(&'\n');
                }
                Some('\n') => {}
                Some(o) => s.push(o),
                None => break,
            },
            _ => s.push(c),
        }
    }
    s
}

/// `label.map`: lower-cased key -> texts, in file order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LabelMap {
    by_key: HashMap<String, Vec<String>>,
}

impl LabelMap {
    /// Only `+ key {text}` lines matter; sections and `#` comments are skipped.
    pub fn parse(text: &str) -> LabelMap {
        let mut map = LabelMap::default();
        for line in text.lines() {
            let t = line.trim();
            let Some(rest) = t.strip_prefix('+') else {
                continue;
            };
            let Some(open) = rest.find('{') else { continue };
            let Some(close) = rest.rfind('}') else {
                continue;
            };
            if close <= open {
                continue;
            }
            let key = rest[..open].trim();
            let value = rest[open + 1..close].trim();
            if !key.is_empty() && !value.is_empty() {
                let v = map.by_key.entry(key.to_ascii_lowercase()).or_default();
                if !v.iter().any(|x| x == value) {
                    v.push(value.to_owned());
                }
            }
        }
        map
    }

    pub fn parse_bytes(bytes: &[u8]) -> LabelMap {
        LabelMap::parse(&decode_text(bytes))
    }

    /// Every layer's `label.map`; a higher layer's texts for a key come first.
    pub fn load(vfs: &Vfs) -> LabelMap {
        let mut out = LabelMap::default();
        for text in layers_low_to_high(vfs, LABEL_FILE) {
            for (key, mut texts) in LabelMap::parse(&text).by_key {
                let v = out.by_key.entry(key).or_default();
                v.retain(|x| !texts.contains(x));
                texts.append(v);
                *v = texts;
            }
        }
        out
    }

    pub fn get(&self, key: &str) -> &[String] {
        self.by_key
            .get(&key.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE: &str = "// head\r\nstruct String_Table_Entry\r\n{\r\n\tchar key_string[1500];\t// c\r\n};\r\n#define MAX_STRINGS 7500\r\n/* block \"not a string\" */\r\nString_Table_Entry translations[MAX_STRINGS] =\r\n{\r\n\t\"GUI_A\",\t\t\"Hello there\",\r\n\t// \"GUI_X\", \"commented\"\r\n\t\"GUI_B\", \"Line one.\r\n\t(Hotkey: A)\", \"GUI_C\", \"say \\\"hi\\\"\\n\",\r\n\t\"GUI_A\", \"dup\",\r\n\t\"GUI_LAST\", \"no comma\"\r\n};\r\n";

    #[test]
    fn parses_pairs_comments_multiline_and_escapes() {
        let t = StringTable::parse(SAMPLE);
        assert_eq!(t.get("GUI_A"), Some("Hello there"), "first duplicate wins");
        assert_eq!(t.get("GUI_B"), Some("Line one.\n\t(Hotkey: A)"));
        assert_eq!(t.get("GUI_C"), Some("say \"hi\"\n"));
        assert_eq!(t.get("GUI_LAST"), Some("no comma"));
        assert_eq!(t.get("GUI_X"), None);
        assert_eq!(t.len(), 4);
        assert_eq!(t.get_or_key("Nope"), "Nope");
        assert!(t.warnings().is_empty());
        assert_eq!(StringTable::parse("\"K\"").warnings().len(), 1);
    }

    #[test]
    fn decodes_windows_1252() {
        let t = StringTable::parse_bytes(b"{ \"K\", \"It\x92s \x93fine\x94\x85 caf\xe9\" }");
        assert_eq!(
            t.get("K"),
            Some("It\u{2019}s \u{201C}fine\u{201D}\u{2026} caf\u{e9}")
        );
        let u = StringTable::parse_bytes("{ \"K\", \"caf\u{e9}\" }".as_bytes());
        assert_eq!(u.get("K"), Some("caf\u{e9}"));
    }

    #[test]
    fn parses_label_map() {
        let l = LabelMap::parse("# c\nmulti {\n\t+ title\t{ }\n    + GoodDayToDie.bzn {A Good Day to Die}\n\t+ kling4.bzn\t{A Good Day to Die}\n}\n");
        assert_eq!(l.get("GOODDAYTODIE.bzn"), ["A Good Day to Die"]);
        assert!(l.get("title").is_empty());
    }

    fn tmp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("sta_content_strings_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn mod_layers_override_and_extend() {
        let (base, top) = (tmp("base"), tmp("top"));
        std::fs::write(
            base.join(STRINGS_FILE),
            "{ \"A\", \"base a\", \"B\", \"base b\" };",
        )
        .unwrap();
        std::fs::write(
            top.join(STRINGS_FILE),
            "{ \"B\", \"mod b\", \"C\", \"mod c\" };",
        )
        .unwrap();
        std::fs::write(base.join(LABEL_FILE), "s {\n+ m.bzn {Base}\n}\n").unwrap();
        std::fs::write(top.join(LABEL_FILE), "s {\n+ m.bzn {Mod}\n}\n").unwrap();
        let mut vfs = Vfs::default();
        vfs.push_dir("mod".into(), top).unwrap();
        vfs.push_dir("Data".into(), base).unwrap();
        let t = StringTable::load(&vfs);
        assert_eq!(
            (t.get("A"), t.get("B"), t.get("C")),
            (Some("base a"), Some("mod b"), Some("mod c"))
        );
        assert_eq!(LabelMap::load(&vfs).get("m.bzn"), ["Mod", "Base"]);
        assert!(StringTable::load(&Vfs::default()).is_empty());
    }

    /// Every language pack of every installed game parses: `STA64_TEST_INSTALL`
    /// (+ `STA64_TEST_MOD`) as in the other install tests, plus the standard
    /// Heroic installs when present. Skips cleanly with none.
    #[test]
    fn installed_packs_parse() {
        let mut roots: Vec<(PathBuf, Option<String>)> = Vec::new();
        if let Ok(r) = std::env::var("STA64_TEST_INSTALL") {
            roots.push((r.into(), std::env::var("STA64_TEST_MOD").ok()));
        }
        if let Some(home) = std::env::var_os("HOME") {
            let heroic = PathBuf::from(home).join("Games/Heroic");
            for n in [
                "Star Trek Armada",
                "Star Trek Armada II",
                "Star Trek Armada II (Stock)",
                "Star Trek Armada II Dominion War",
                "Star Trek Armada II Fleet Operations",
                "Star Trek Armada II Fleet Operations Roots",
            ] {
                roots.push((heroic.join(n), None));
            }
            for m in ["Fleet Ops 4.0", "STA2 Classic Mod"] {
                roots.push((
                    heroic.join("Star Trek Armada II Fleet Operations Roots"),
                    Some(m.into()),
                ));
            }
        }
        let mut seen = 0;
        for (root, m) in roots {
            // Armada I and the stock install have no `Data` folder: their files sit in the root.
            let vfs = match Vfs::mount_install(&root, m.as_deref()) {
                Ok(v) => v,
                Err(_) if root.join(STRINGS_FILE).is_file() => {
                    let mut v = Vfs::default();
                    v.push_dir("root".into(), root.clone()).unwrap();
                    v
                }
                Err(_) => continue,
            };
            if vfs.resolve(STRINGS_FILE).is_none() {
                continue;
            }
            seen += 1;
            let t = StringTable::load(&vfs);
            assert!(
                t.len() > 100,
                "{}: only {} strings",
                root.display(),
                t.len()
            );
            assert!(
                t.warnings().is_empty(),
                "{}: {:?}",
                root.display(),
                t.warnings()
            );
            assert!(t
                .iter()
                .all(|(k, v)| !k.is_empty() && !v.contains('\u{FFFD}')));
            assert_eq!(t.get("NO_SUCH_KEY_X"), None);
            let l = LabelMap::load(&vfs);
            assert!(!l.is_empty(), "{}: empty label.map", root.display());
            if let Some(v) = t.get("GUI_CV_MIN_TOOLTIP") {
                assert_eq!(v, "Double click to return to strategic mode.");
            }
            eprintln!(
                "{} {:?}: {} strings, {} labels",
                root.display(),
                m,
                t.len(),
                l.len()
            );
        }
        if seen == 0 {
            eprintln!("no install found; skipped");
        }
    }
}
