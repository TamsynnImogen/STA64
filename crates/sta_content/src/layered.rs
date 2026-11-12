//! Layered `.h` cfg files: include inheritance with provenance (CON-34, CON-36,
//! MODDING_DECISIONS D11).
//!
//! A header in an upper layer may `#include` its own name: that means "the
//! copy of this file in the next layer down", the rule ODFs already follow
//! (`sta_odf::resolve_layered`). The upper file then states only what it
//! changes. Each self-include steps one copy further down, and a self-include
//! with no copy left is dropped (stock files include themselves). An include
//! of any other name takes that file's top copy. Includes of other names that
//! lead back to a file being read are a true cycle and an [`IncludeError`].
//!
//! An entry is a `#define NAME value` line or a one-line `[type] NAME = value;`
//! assignment, recorded with the file, layer copy and line that wins. Array
//! initialisers over several lines are not entries here: the typed readers
//! (`CfgH`) read those from the expanded text, this scan exists to say where a
//! value came from.

use std::collections::BTreeMap;

use sta_vfs::Vfs;

use crate::Warning;

/// One cfg assignment and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub value: String,
    /// 1-based line in `file`.
    pub line: usize,
    /// The file name as included.
    pub file: String,
    /// The layer the file was read from.
    pub layer: String,
    /// Which copy of the file: 0 is the one the game sees, 1 the one it hides.
    pub version: usize,
}

/// A header's entries by name; a later assignment replaces an earlier one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    pub entries: BTreeMap<String, Entry>,
}

/// A header include that leads back to a file being read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("include cycle: {0}")]
pub struct IncludeError(pub String);

/// A resolved header: the expanded text the typed readers parse, the
/// entries with provenance, and what could not be read.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub text: String,
    pub header: Header,
    pub warnings: Vec<Warning>,
}

/// The `(name, value)` of a one-line entry, if `raw` is one.
pub fn scan_line(raw: &str) -> Option<(String, String)> {
    let line = strip_comment(raw).trim();
    if let Some(rest) = line.strip_prefix('#') {
        let def = rest.trim_start().strip_prefix("define").filter(|r| r.starts_with(char::is_whitespace))?;
        let mut p = def.trim().splitn(2, char::is_whitespace);
        let (name, value) = (p.next()?, p.next()?);
        return Some((name.to_owned(), value.trim().trim_matches('"').to_owned()));
    }
    let (lhs, rhs) = line.split_once('=')?;
    let rhs = rhs.trim().trim_end_matches(';').trim();
    let name = lhs.split_whitespace().last().map(|n| n.split('[').next().unwrap_or(n))?;
    (!name.is_empty() && !rhs.is_empty() && !rhs.starts_with('{') && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then(|| (name.to_owned(), rhs.trim_matches('"').to_owned()))
}

/// Scan one header's text: `(name, value, line)` in file order, includes not followed.
pub fn scan(text: &str) -> Vec<(String, String, usize)> {
    text.lines().enumerate().filter_map(|(i, l)| scan_line(l).map(|(k, v)| (k, v, i + 1))).collect()
}

fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    let b = line.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b'"' => quoted = !quoted,
            b'/' if !quoted && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
    }
    line
}

struct Walk<'a> {
    vfs: &'a Vfs,
    out: Resolved,
    stack: Vec<(String, usize)>,
}

impl Walk<'_> {
    /// Apply the `version`-th copy of `name`; `Ok(false)` when there is none.
    fn apply(&mut self, name: &str, version: usize) -> Result<bool, IncludeError> {
        let key = sta_vfs::normalize(name);
        if self.stack.contains(&(key.clone(), version)) || self.stack.len() >= 128 {
            let mut chain: Vec<String> = self.stack.iter().map(|(k, v)| if *v == 0 { k.clone() } else { format!("{k}@{v}") }).collect();
            chain.push(key);
            return Err(IncludeError(chain.join(" -> ")));
        }
        let versions = self.vfs.resolve_all(name);
        let Some(&(layer, source)) = versions.get(version) else { return Ok(false) };
        let Some(index) = self.vfs.layer_index(layer) else { return Ok(false) };
        let bytes = match self.vfs.read_from(index, source) {
            Ok(b) => b,
            Err(e) => {
                self.out.warnings.push(Warning::new(name, e.to_string()));
                return Ok(false);
            }
        };
        let layer = layer.to_owned();
        let text = String::from_utf8_lossy(&bytes);
        self.stack.push((key.clone(), version));
        for (i, line) in text.lines().enumerate() {
            if let Some(rest) = line.trim().strip_prefix("#include") {
                let Some(include) = rest.trim().split('"').nth(1) else {
                    self.out.warnings.push(Warning::new(name, "unsupported header include syntax"));
                    continue;
                };
                if sta_vfs::normalize(include) == key {
                    // Inherit: the copy of this file the layer above hides.
                    self.apply(include, version + 1)?;
                } else if !self.apply(include, 0)? {
                    self.out.warnings.push(Warning::new(name, format!("header include unavailable: {include}")));
                }
                continue;
            }
            self.out.text.push_str(line);
            self.out.text.push('\n');
            if let Some((k, value)) = scan_line(line) {
                self.out.header.entries.insert(k, Entry { value, line: i + 1, file: name.to_owned(), layer: layer.clone(), version });
            }
        }
        self.stack.pop();
        Ok(true)
    }
}

/// Resolve the `version`-th copy of `name` (0 = the one the game sees) with
/// its includes. `Ok(None)` when no such copy exists; a true include cycle is
/// an error.
pub fn resolve(vfs: &Vfs, name: &str, version: usize) -> Result<Option<Resolved>, IncludeError> {
    let mut w = Walk { vfs, out: Resolved::default(), stack: Vec::new() };
    Ok(w.apply(name, version)?.then_some(w.out))
}

/// The typed reader over the inherited text of `name`: what the game sees
/// of a cfg header once self-includes are expanded. `None` when no layer has
/// the file or it cycles ([`crate::Rules::load`] reports the cycle).
pub fn cfg_h(vfs: &Vfs, name: &str) -> Option<sta_assets::cfg_h::CfgH> {
    resolve(vfs, name, 0).ok().flatten().map(|r| sta_assets::cfg_h::CfgH::parse(&r.text))
}

/// [`resolve`]'s entries; `None` when there is no such copy or it cycles.
pub fn resolve_header(vfs: &Vfs, name: &str, version: usize) -> Option<Header> {
    resolve(vfs, name, version).ok().flatten().map(|r| r.header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scans_defines_and_assignments() {
        let t = "#include \"A.h\"\n#define NAME \"x.bik\" // c\nfloat F = 0.5; // c\nint ARR[2] = {1, 2};\nint  N=3;\n";
        assert_eq!(
            scan(t),
            [("NAME".to_owned(), "x.bik".to_owned(), 2), ("F".to_owned(), "0.5".to_owned(), 3), ("N".to_owned(), "3".to_owned(), 5)]
        );
    }

    struct Tmp(std::path::PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn mount(name: &str, files: &[(&str, &str)]) -> (Tmp, Vfs) {
        let d = std::env::temp_dir().join(format!("sta_layered_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for (p, text) in files {
            let p = d.join(p);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        let vfs = Vfs::mount_install(&d, Some("Top")).unwrap();
        (Tmp(d), vfs)
    }

    #[test]
    fn self_include_inherits_the_layer_below_with_provenance() {
        let (_t, vfs) = mount(
            "inherit",
            &[
                ("Data/RTS_CFG.h", "int A = 1;\nint B = 2;\n#define C 3\n"),
                ("Data/Mods/Mid/info.ini", "[mod]\n"),
                ("Data/Mods/Mid/RTS_CFG.h", "#include \"RTS_CFG.h\"\nint B = 20;\n"),
                ("Data/Mods/Top/info.ini", "[mod]\nParentMod=Mid\n"),
                ("Data/Mods/Top/RTS_CFG.h", "#include \"RTS_CFG.h\"\nint A = 100;\n"),
            ],
        );
        let r = resolve(&vfs, "RTS_CFG.h", 0).unwrap().unwrap();
        let e = &r.header.entries;
        assert_eq!((e["A"].value.as_str(), e["A"].layer.as_str(), e["A"].line, e["A"].version), ("100", "Top", 2, 0));
        assert_eq!((e["B"].value.as_str(), e["B"].layer.as_str(), e["B"].line, e["B"].version), ("20", "Mid", 2, 1));
        assert_eq!((e["C"].value.as_str(), e["C"].layer.as_str(), e["C"].line, e["C"].version), ("3", "Data", 3, 2));
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(r.text.contains("int B = 2;") && r.text.contains("int B = 20;"), "the expanded text keeps the order: {}", r.text);
        assert!(r.text.find("int B = 2;").unwrap() < r.text.find("int B = 20;").unwrap());
    }

    #[test]
    fn a_self_include_with_nothing_below_is_dropped() {
        let (_t, vfs) = mount("cut", &[("Data/Mods/Top/info.ini", "[mod]\nstandalone=1\n"), ("Data/Mods/Top/RTS_CFG.h", "#include \"rts_cfg.h\"\nint A = 1;\n")]);
        let r = resolve(&vfs, "RTS_CFG.h", 0).unwrap().unwrap();
        assert_eq!(r.header.entries["A"].value, "1");
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn a_true_cycle_is_an_error() {
        let (_t, vfs) = mount(
            "cycle",
            &[
                ("Data/Mods/Top/info.ini", "[mod]\nstandalone=1\n"),
                ("Data/Mods/Top/RTS_CFG.h", "#include \"ART_CFG.h\"\n"),
                ("Data/Mods/Top/ART_CFG.h", "#include \"RTS_CFG.h\"\n"),
            ],
        );
        let e = resolve(&vfs, "RTS_CFG.h", 0).unwrap_err();
        assert_eq!(e.0, "rts_cfg.h -> art_cfg.h -> rts_cfg.h");
    }

    #[test]
    fn missing_include_is_a_warning() {
        let (_t, vfs) = mount("missing", &[("Data/Mods/Top/info.ini", "[mod]\nstandalone=1\n"), ("Data/Mods/Top/RTS_CFG.h", "#include \"Gone.h\"\nint A = 1;\n")]);
        let r = resolve(&vfs, "RTS_CFG.h", 0).unwrap().unwrap();
        assert!(r.warnings[0].message.contains("Gone.h"));
    }
}
