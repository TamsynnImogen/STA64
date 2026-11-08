//! ODF ("object definition file") parsing.
//!
//! Format, as used by Armada I/II and Fleet Operations:
//! - `key = value` lines; keys are case-insensitive.
//! - `//` starts a comment (outside quoted strings).
//! - `#include "other.odf"` pulls in another ODF. Includes are resolved by file
//!   name across all known ODF folders, so the folder is organisational only.
//! - Later assignments override earlier ones, so an ODF's own keys override
//!   anything it includes.
//!
//! Values are kept as raw text with typed accessors; interpretation belongs to
//! the systems that consume them.

use std::collections::HashMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum OdfError {
    #[error("include not found: {0}")]
    IncludeNotFound(String),
    #[error("include cycle: {0}")]
    IncludeCycle(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Key as written (original case).
    pub key: String,
    pub value: String,
    pub line: usize,
}

/// One parsed file, before include resolution.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawOdf {
    pub includes: Vec<String>,
    pub entries: Vec<Entry>,
}

pub fn parse(text: &str) -> RawOdf {
    let mut out = RawOdf::default();
    for (i, line) in text.lines().enumerate() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let rest = rest.trim_start();
            if rest.len() >= 7 && rest[..7].eq_ignore_ascii_case("include") {
                let name = unquote(rest[7..].trim());
                if !name.is_empty() {
                    out.includes.push(name.to_owned());
                }
            }
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            if !key.is_empty() {
                out.entries.push(Entry {
                    key: key.to_owned(),
                    value: v.trim().to_owned(),
                    line: i + 1,
                });
            }
        }
    }
    out
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let b = line.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b'"' => in_quote = !in_quote,
            b'/' if !in_quote && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

/// A fully resolved ODF: includes flattened, last assignment wins.
fn number(value: &str) -> Option<&str> {
    let first = value.split_whitespace().next()?;
    Some(first.trim_end_matches(';').trim_end_matches(['f', 'F']).trim_end_matches(';'))
}

#[derive(Debug, Clone, Default)]
pub struct Odf {
    pub name: String,
    values: HashMap<String, String>,
    /// Keys in first-definition order, original case.
    order: Vec<String>,
}

impl Odf {
    fn set(&mut self, key: &str, value: &str) {
        let k = key.to_ascii_lowercase();
        if self.values.insert(k, value.to_owned()).is_none() {
            self.order.push(key.to_owned());
        }
    }

    /// Raw value text (quotes retained).
    pub fn raw(&self, key: &str) -> Option<&str> {
        self.values.get(&key.to_ascii_lowercase()).map(String::as_str)
    }

    /// String value with surrounding quotes removed.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.raw(key).map(unquote)
    }

    /// First number of the value. Accepts the suffixes stock files use: `1.0f`,
    /// a trailing `;` (physics ODFs: `forwardAccel = 1;`), and both.
    pub fn f32(&self, key: &str) -> Option<f32> {
        number(self.str(key)?)?.parse().ok()
    }

    pub fn i64(&self, key: &str) -> Option<i64> {
        let first = number(self.str(key)?)?;
        first.parse().ok().or_else(|| first.parse::<f64>().ok().map(|f| f as i64))
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.str(key)?.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        }
    }

    /// Whitespace-separated list values (e.g. vectors, build lists).
    pub fn list(&self, key: &str) -> Vec<&str> {
        self.str(key).map(|s| s.split_whitespace().collect()).unwrap_or_default()
    }

    pub fn class_label(&self) -> Option<&str> {
        self.str("classLabel")
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }
}

/// Resolve `name` and all of its includes. `load` maps an ODF file name (e.g.
/// `"fdefiant.odf"`) to its text, typically by searching the ODF folders of a VFS.
pub fn resolve(name: &str, load: &mut dyn FnMut(&str) -> Option<String>) -> Result<Odf, OdfError> {
    let mut odf = Odf {
        name: name.to_owned(),
        ..Default::default()
    };
    let mut stack = Vec::new();
    apply(name, load, &mut odf, &mut stack)?;
    Ok(odf)
}

fn apply(
    name: &str,
    load: &mut dyn FnMut(&str) -> Option<String>,
    odf: &mut Odf,
    stack: &mut Vec<String>,
) -> Result<(), OdfError> {
    let key = with_odf_ext(name).to_ascii_lowercase();
    if stack.contains(&key) {
        stack.push(key);
        return Err(OdfError::IncludeCycle(stack.join(" -> ")));
    }
    let text = load(&with_odf_ext(name)).ok_or_else(|| OdfError::IncludeNotFound(name.to_owned()))?;
    let raw = parse(&text);
    stack.push(key);
    for inc in &raw.includes {
        apply(inc, load, odf, stack)?;
    }
    stack.pop();
    for e in &raw.entries {
        odf.set(&e.key, &e.value);
    }
    Ok(())
}

fn with_odf_ext(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".odf") {
        name.to_owned()
    } else {
        format!("{name}.odf")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entries_comments_and_includes() {
        let raw = parse(
            "// header\n#include \"base.odf\"\nclassLabel = \"starship\" // trailing\n\
             tooltip = \"Uses // inside\"\n\nmaxHealth=1200\n",
        );
        assert_eq!(raw.includes, ["base.odf"]);
        assert_eq!(raw.entries.len(), 3);
        assert_eq!(raw.entries[1].value, "\"Uses // inside\"");
        assert_eq!(raw.entries[2].line, 6);
    }

    #[test]
    fn resolves_includes_with_override() {
        let files: HashMap<&str, &str> = HashMap::from([
            ("base.odf", "classLabel = \"starship\"\nmaxHealth = 500\nspeed = 1.5f\n"),
            ("ship.odf", "#include \"base\"\nMAXHEALTH = 900\nbuildList = \"a b c\"\n"),
        ]);
        let mut load = |n: &str| files.get(n.to_ascii_lowercase().as_str()).map(|s| s.to_string());
        let odf = resolve("ship", &mut load).unwrap();
        assert_eq!(odf.class_label(), Some("starship"));
        assert_eq!(odf.i64("maxHealth"), Some(900));
        assert_eq!(odf.f32("speed"), Some(1.5));
        assert_eq!(odf.list("buildList"), ["a", "b", "c"]);
        let phys = resolve("p", &mut |_| Some("forwardAccel = 1;\nturnOmega = .5f;\nmaxCount = 3;\n".into())).unwrap();
        assert_eq!(phys.f32("forwardAccel"), Some(1.0));
        assert_eq!(phys.f32("turnOmega"), Some(0.5));
        assert_eq!(phys.i64("maxCount"), Some(3));
    }

    #[test]
    fn detects_include_cycle() {
        let mut load = |n: &str| Some(format!("#include \"{}\"\n", if n == "a.odf" { "b" } else { "a" }));
        assert!(matches!(resolve("a", &mut load), Err(OdfError::IncludeCycle(_))));
    }
}
