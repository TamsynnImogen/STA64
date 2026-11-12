//! Layered `.h` cfg files with provenance (CON-34 reads one version's own
//! entries; CON-36 adds include inheritance, MODDING_DECISIONS D11).
//!
//! An entry is a `#define NAME value` line or a one-line `[type] NAME = value;`
//! assignment. Array initialisers over several lines are not entries here:
//! the typed readers (`CfgH`) still read those, this scanner exists to say
//! which layer and line a value came from.

use std::collections::BTreeMap;

use sta_vfs::Vfs;

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

/// Scan one header's text: `(name, value, line)` in file order.
pub fn scan(text: &str) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if let Some(rest) = line.strip_prefix('#') {
            if let Some(def) = rest.trim_start().strip_prefix("define").filter(|r| r.starts_with(char::is_whitespace)) {
                let mut p = def.trim().splitn(2, char::is_whitespace);
                if let (Some(name), Some(value)) = (p.next(), p.next()) {
                    out.push((name.to_owned(), value.trim().trim_matches('"').to_owned(), i + 1));
                }
            }
            continue;
        }
        if let Some((lhs, rhs)) = line.split_once('=') {
            let rhs = rhs.trim().trim_end_matches(';').trim();
            let name = lhs.split_whitespace().last().map(|n| n.split('[').next().unwrap_or(n));
            if let Some(name) = name.filter(|n| !n.is_empty() && !rhs.is_empty() && !rhs.starts_with('{') && n.chars().all(|c| c.is_alphanumeric() || c == '_')) {
                out.push((name.to_owned(), rhs.trim_matches('"').to_owned(), i + 1));
            }
        }
    }
    out
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

/// The entries of the `version`-th copy of `name` (0 = the one the game
/// sees), counting down the layers.
pub fn resolve_header(vfs: &Vfs, name: &str, version: usize) -> Option<Header> {
    let all = vfs.resolve_all(name);
    let (layer, source) = *all.get(version)?;
    let bytes = vfs.read_from(vfs.layer_index(layer)?, source).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut h = Header::default();
    for (key, value, line) in scan(&text) {
        h.entries.insert(key, Entry { value, line, file: name.to_owned(), layer: layer.to_owned(), version });
    }
    Some(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_defines_and_assignments() {
        let t = "#include \"A.h\"\n#define NAME \"x.bik\" // c\nfloat F = 0.5; // c\nint ARR[2] = {1, 2};\nint  N=3;\n";
        assert_eq!(
            scan(t),
            [("NAME".to_owned(), "x.bik".to_owned(), 2), ("F".to_owned(), "0.5".to_owned(), 3), ("N".to_owned(), "3".to_owned(), 5)]
        );
    }
}
