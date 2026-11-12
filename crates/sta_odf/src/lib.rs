//! ODF ("object definition file") parsing.
//!
//! Format, as used by Armada I/II and Fleet Operations:
//! - `key = value` lines; keys are case-insensitive.
//! - `//` starts a comment (outside quoted strings).
//! - `#include "other.odf"` pulls in another ODF. Includes are resolved by file
//!   name across all known ODF folders, so the folder is organisational only.
//! - Later assignments override earlier ones, so an ODF's own keys override
//!   anything it includes.
//! - Quoted list rows may continue an assignment on following lines.
//! - Fleet Operations layers its files (a mod's over its parent's over the
//!   game's). An include of the root file's own name is its **next version
//!   down** ([`resolve_layered`]): a mod's `x.odf` that includes `x.odf`
//!   inherits the parent's `x.odf` and changes only what it writes. Each such
//!   include, in the order the tree is read, takes the next version below the
//!   last one taken (the rule of `a2fmt::Odf::load_layered`). With no version
//!   left the include is cut, as the game does, and listed in [`Odf::cut`].
//!   Any other loop is an [`OdfError::IncludeCycle`].
//!
//! Values are kept as raw text with typed accessors; interpretation belongs to
//! the systems that consume them.

use std::collections::HashMap;

pub mod cst;
pub use cst::{Cst, LineKind};
pub mod diagnostics;
pub use diagnostics::DuplicateEntry;

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
    let mut continuation: Option<usize> = None;
    for (i, line) in text.lines().enumerate() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            continuation = None;
            let rest = rest.trim_start();
            if rest.len() >= 7 && rest[..7].eq_ignore_ascii_case("include") {
                let name = unquote(rest[7..].trim());
                if !name.is_empty() {
                    out.includes.push(name.to_owned());
                }
            }
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            continuation = None;
            continue;
        }
        if is_quoted_continuation(line) {
            if let Some(entry) = continuation.and_then(|index| out.entries.get_mut(index)) {
                if !entry.value.is_empty() {
                    entry.value.push(' ');
                }
                entry.value.push_str(line);
            }
            continue;
        }
        continuation = None;
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            if !key.is_empty() {
                out.entries.push(Entry {
                    key: key.to_owned(),
                    value: v.trim().to_owned(),
                    line: i + 1,
                });
                continuation = Some(out.entries.len() - 1);
            }
        }
    }
    out
}

fn is_quoted_continuation(line: &str) -> bool {
    let mut rest = line.trim();
    if !rest.starts_with('"') {
        return false;
    }
    while let Some(quoted) = rest.strip_prefix('"') {
        let Some(end) = quoted.find('"') else { return false };
        rest = quoted[end + 1..].trim_start();
        if rest.is_empty() {
            return true;
        }
    }
    false
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

/// Where the winning assignment of an ODF key came from (CON-34 `why`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySource {
    /// File name with `.odf`, as it was included (shared between the keys of one file).
    pub file: std::sync::Arc<str>,
    /// Which copy of that file: 0 is the highest-priority layer's, 1 the one
    /// it hides (a self-include), and so on, as `resolve_layered` counts.
    pub version: usize,
    /// 1-based line of the assignment.
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Odf {
    pub name: String,
    values: HashMap<String, String>,
    sources: HashMap<String, KeySource>,
    /// Keys in first-definition order, original case.
    order: Vec<String>,
    /// Includes of the root's own name that had no lower version left, so
    /// were skipped (stock files that include themselves).
    pub cut: Vec<String>,
}

impl Odf {
    fn set(&mut self, key: &str, value: &str, source: KeySource) {
        let k = key.to_ascii_lowercase();
        self.sources.insert(k.clone(), source);
        if self.values.insert(k, value.to_owned()).is_none() {
            self.order.push(key.to_owned());
        }
    }

    /// The file, layer copy and line whose assignment of `key` won.
    pub fn source(&self, key: &str) -> Option<&KeySource> {
        self.sources.get(&key.to_ascii_lowercase())
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
/// Only one version of each file exists, so a self-include is cut.
pub fn resolve(name: &str, load: &mut dyn FnMut(&str) -> Option<String>) -> Result<Odf, OdfError> {
    resolve_layered(name, &mut |n, version| if version == 0 { load(n) } else { None })
}

/// Resolve `name` over layered files. `load(name, version)` returns the text of
/// `name`'s `version`-th copy counting down from the highest-priority layer
/// (0 = the copy normally seen, 1 = the one it hides, ...), or `None`.
pub fn resolve_layered(name: &str, load: &mut dyn FnMut(&str, usize) -> Option<String>) -> Result<Odf, OdfError> {
    let mut odf = Odf {
        name: name.to_owned(),
        ..Default::default()
    };
    let mut walk = Walk {
        root: with_odf_ext(name).to_ascii_lowercase(),
        version: 0,
        stack: Vec::new(),
    };
    apply(name, 0, load, &mut odf, &mut walk)?;
    Ok(odf)
}

struct Walk {
    /// Root file name, lower case, with `.odf`.
    root: String,
    /// Last version of the root taken.
    version: usize,
    /// Files being applied, outermost first, as `name@version`.
    stack: Vec<(String, usize)>,
}

fn apply(
    name: &str,
    version: usize,
    load: &mut dyn FnMut(&str, usize) -> Option<String>,
    odf: &mut Odf,
    w: &mut Walk,
) -> Result<(), OdfError> {
    let file = with_odf_ext(name);
    let key = file.to_ascii_lowercase();
    let text = load(&file, version).ok_or_else(|| OdfError::IncludeNotFound(name.to_owned()))?;
    let raw = parse(&text);
    let file_arc: std::sync::Arc<str> = file.as_str().into();
    w.stack.push((key, version));
    for inc in &raw.includes {
        let inc_key = with_odf_ext(inc).to_ascii_lowercase();
        if inc_key == w.root {
            // The root's own name: its next version down, if there is one.
            let next = w.version + 1;
            if load(&with_odf_ext(inc), next).is_none() {
                odf.cut.push(inc.clone());
                continue;
            }
            w.version = next;
            apply(inc, next, load, odf, w)?;
        } else if w.stack.iter().any(|(k, _)| *k == inc_key) {
            let mut chain: Vec<String> = w.stack.iter().map(|(k, _)| k.clone()).collect();
            chain.push(inc_key);
            return Err(OdfError::IncludeCycle(chain.join(" -> ")));
        } else {
            apply(inc, 0, load, odf, w)?;
        }
    }
    w.stack.pop();
    for e in &raw.entries {
        odf.set(&e.key, &e.value, KeySource { file: std::sync::Arc::clone(&file_arc), version, line: e.line });
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
    fn quoted_lists_continue_across_blank_and_comment_rows() {
        let raw = parse(
            "possibleCraftNames = \"USS A\" \"USS B\" // inline\n\
             // between rows\n\n\
             \"USS C\" \"A=B // C\" \"https://fleet.example/ship\" // trailing\n\
             maxHealth = 12\n",
        );
        assert_eq!(raw.entries.len(), 2);
        assert_eq!(raw.entries[0].line, 1);
        assert_eq!(raw.entries[0].value, "\"USS A\" \"USS B\" \"USS C\" \"A=B // C\" \"https://fleet.example/ship\"");
        assert_eq!(raw.entries[1].value, "12");

        let empty_first = parse("possibleCraftNames =\n\"USS A\" \"USS B\"\n\"USS C\"\n");
        assert_eq!(empty_first.entries[0].value, "\"USS A\" \"USS B\" \"USS C\"");
    }

    #[test]
    fn continuation_stops_at_content_boundaries_and_new_assignments() {
        let raw = parse(
            "names = \"first\"\n\
             stray content\n\"not added\"\n\
             #include \"parent.odf\"\n\"not added either\"\n\
             names =\n\
             [section]\n\"still not added\"\n\
             names = \"last\"\n\"A=B\"\n",
        );
        assert_eq!(raw.includes, ["parent.odf"]);
        assert_eq!(raw.entries.iter().map(|entry| entry.value.as_str()).collect::<Vec<_>>(), ["\"first\"", "", "\"last\" \"A=B\""]);
        assert_eq!(raw.entries.iter().map(|entry| entry.line).collect::<Vec<_>>(), [1, 6, 9]);
        let duplicates = raw.duplicates();
        assert_eq!(duplicates.len(), 2);
        assert_eq!(duplicates[1].replacement.line, 9);
    }

    #[test]
    fn quoted_list_includes_and_layered_override_keep_precedence() {
        let mut load = |name: &str, version: usize| match (name, version) {
            ("ship.odf", 0) => Some("#include \"ship.odf\"\npossibleCraftNames = \"Mod Ship\"\n".to_owned()),
            ("ship.odf", 1) => Some("#include \"shared.odf\"\npossibleCraftNames =\n\"Parent Ship\"\n".to_owned()),
            ("shared.odf", 0) => Some("possibleCraftNames = \"Shared Ship\"\n".to_owned()),
            _ => None,
        };
        let odf = resolve_layered("ship", &mut load).unwrap();
        assert_eq!(odf.raw("possibleCraftNames"), Some("\"Mod Ship\""));

        let mut load = |name: &str| match name {
            "child.odf" => Some("#include \"parent.odf\"\n".to_owned()),
            "parent.odf" => Some("possibleCraftNames =\n\"Parent Ship\"\n".to_owned()),
            _ => None,
        };
        assert_eq!(resolve("child", &mut load).unwrap().raw("possibleCraftNames"), Some("\"Parent Ship\""));

        let raw = parse("possibleCraftNames = \"Old\"\npossibleCraftNames =\n");
        assert_eq!(raw.entries[1].value, "");
        let mut load = |_: &str| Some("possibleCraftNames = \"Old\"\npossibleCraftNames =\n".to_owned());
        assert_eq!(resolve("ship", &mut load).unwrap().raw("possibleCraftNames"), Some(""));
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
        // a -> b -> c -> b: a loop not through the root's own name.
        let next = |n: &str| match n {
            "a.odf" => "b",
            "b.odf" => "c",
            _ => "b",
        };
        let mut load = |n: &str| Some(format!("#include \"{}\"\n", next(n)));
        let err = resolve("a", &mut load).unwrap_err();
        assert_eq!(err, OdfError::IncludeCycle("a.odf -> b.odf -> c.odf -> b.odf".into()));
    }

    #[test]
    fn self_include_without_lower_version_is_cut() {
        // Stock: one version only. The game skips the include; so do we.
        let mut load = |_: &str| Some("#include \"a.odf\"\nmaxHealth = 5\n".to_string());
        let odf = resolve("a", &mut load).unwrap();
        assert_eq!(odf.i64("maxHealth"), Some(5));
        assert_eq!(odf.cut, ["a.odf"]);
    }

    /// Layered fixture loader: `fixtures/odf/<layer>/<name>`, top layer first.
    fn fixture_loader(layers: &'static [&'static str]) -> impl FnMut(&str, usize) -> Option<String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/odf");
        move |name, version| {
            layers
                .iter()
                .filter_map(|l| std::fs::read_to_string(root.join(l).join(name.to_ascii_lowercase())).ok())
                .nth(version)
        }
    }

    #[test]
    fn self_include_takes_next_layer_down() {
        let mut load = fixture_loader(&["mod", "parent", "base"]);
        let odf = resolve_layered("fcruiser", &mut load).unwrap();
        // mod overrides, parent's change kept, base fills the rest.
        assert_eq!(odf.i64("maxHealth"), Some(1500));
        assert_eq!(odf.f32("maxSpeed"), Some(4.0));
        assert_eq!(odf.class_label(), Some("craft"));
        assert_eq!(odf.str("unitName"), Some("Cruiser (mod)"));
        // base's own include of a shared file still applies.
        assert_eq!(odf.str("race"), Some("federation"));
        assert!(odf.cut.is_empty(), "{:?}", odf.cut);
    }

    #[test]
    fn self_include_through_another_file_takes_next_version() {
        // mod/fdestroy.odf includes fdestroy_tweak.odf, which includes fdestroy.odf:
        // that reaches the next version down too (a2fmt counts any include of the root).
        let mut load = fixture_loader(&["mod", "parent", "base"]);
        let odf = resolve_layered("fdestroy", &mut load).unwrap();
        assert_eq!(odf.i64("maxHealth"), Some(800));
        assert_eq!(odf.f32("shotDelay"), Some(0.5));
        assert_eq!(odf.class_label(), Some("craft"));
    }

    #[test]
    fn true_cycle_in_layers_is_still_an_error() {
        let mut load = fixture_loader(&["cycle"]);
        assert!(matches!(resolve_layered("loop_a", &mut load), Err(OdfError::IncludeCycle(_))));
    }

    #[test]
    fn layered_self_include_runs_out_of_versions() {
        // Only two layers hold fcruiser.odf here; parent's self-include is cut.
        let mut load = fixture_loader(&["mod", "parent"]);
        let odf = resolve_layered("fcruiser", &mut load).unwrap();
        assert_eq!(odf.i64("maxHealth"), Some(1500));
        assert_eq!(odf.class_label(), None);
        assert_eq!(odf.cut, ["fcruiser.odf"]);
    }
}
