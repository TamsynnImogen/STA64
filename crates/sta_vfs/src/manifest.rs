//! Mod manifests (`manifest.toml`, `docs/STA_ARCHIVE.md` "Manifest"; CON-18).
//!
//! ```toml
//! [mod]
//! id = "federation-expansion"     # a-z 0-9 - _
//! name = "Federation Expansion"
//! version = "1.2.0"               # semantic versioning
//! description = "..."
//! authors = ["Immy"]
//! licence = "CC-BY-4.0"
//! engine = ">=0.4"
//! profile = "fo4"
//! kind = "addon"                  # or "standalone"
//!
//! [dependencies]
//! "fleet-ops-4-0" = "^4.0"        # id = version requirement
//!
//! [load]
//! after = ["better-textures"]
//! before = []
//! ```
//!
//! A mod without a manifest (every mod folder today) gets one from its
//! `info.ini` ([`Manifest::from_info`]): id from the folder name, `ParentMod`
//! as a dependency on any version (none when `standalone`), the `standalone`
//! flag as the kind (a mod with neither is an addon over the game's data),
//! `Profile` as the profile. Legacy versions
//! (`4.0Z`, `v1.2Z (Original Gameplay)`) are kept as text; a requirement
//! other than `*` on a mod with such a version cannot be checked and fails.
//! Unknown keys are kept in [`Manifest::extra`] so a later STA64 can add
//! fields.

use std::collections::BTreeMap;

use semver::{Version, VersionReq};

use crate::convert::mod_id;
use crate::mods::ModInfo;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest.toml: {0}")]
    Toml(String),
    #[error("manifest.toml: [mod] {0} is missing")]
    Missing(&'static str),
    #[error("manifest.toml: id {0:?} may use only a-z, 0-9, '-' and '_'")]
    BadId(String),
    #[error("manifest.toml: {field} {value:?} is not a version requirement: {why}")]
    BadRequirement { field: String, value: String, why: String },
    #[error("manifest.toml: kind {0:?} is neither \"addon\" nor \"standalone\"")]
    BadKind(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKind {
    /// Needs other mods (has dependencies).
    Addon,
    /// Runs on its own (over the game's data only).
    Standalone,
}

/// A mod's version: semantic when it parses as one, else the text as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModVersion {
    Semver(Version),
    Legacy(String),
    Unknown,
}

impl ModVersion {
    pub fn parse(text: &str) -> ModVersion {
        let t = text.trim();
        if t.is_empty() {
            return ModVersion::Unknown;
        }
        match Version::parse(t.trim_start_matches(['v', 'V'])) {
            Ok(v) => ModVersion::Semver(v),
            Err(_) => ModVersion::Legacy(t.to_owned()),
        }
    }

    /// Does this version satisfy `req`? `*` is satisfied by anything;
    /// anything else needs a semantic version.
    pub fn satisfies(&self, req: &VersionReq) -> bool {
        if *req == VersionReq::STAR {
            return true;
        }
        match self {
            ModVersion::Semver(v) => req.matches(v),
            _ => false,
        }
    }
}

impl std::fmt::Display for ModVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModVersion::Semver(v) => write!(f, "{v}"),
            ModVersion::Legacy(s) => f.write_str(s),
            ModVersion::Unknown => f.write_str("(no version)"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: ModVersion,
    pub description: String,
    pub authors: Vec<String>,
    pub licence: Option<String>,
    /// STA64 versions it works with.
    pub engine: Option<VersionReq>,
    pub profile: Option<String>,
    pub kind: ModKind,
    /// Dependency id -> version requirement, in id order.
    pub dependencies: BTreeMap<String, VersionReq>,
    pub load_after: Vec<String>,
    pub load_before: Vec<String>,
    /// Keys this STA64 does not know, kept as written.
    pub extra: toml::Table,
    /// Where it came from: `manifest.toml`, or generated from `info.ini`.
    pub generated: bool,
    /// `kind` was written in the file (a missing one is guessed from the dependencies).
    pub kind_explicit: bool,
}

/// The manifest text a mod folder at `dir` carries: its loose `manifest.toml`,
/// else one inside a root `.sta` archive.
pub fn manifest_text(dir: &std::path::Path) -> Option<String> {
    let entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
    if let Some(e) = entries.iter().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("manifest.toml"))
        && let Ok(t) = std::fs::read_to_string(e.path())
    {
        return Some(t);
    }
    let mut stas: Vec<_> = entries.iter().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sta"))).collect();
    stas.sort();
    for p in stas {
        let Ok(a) = crate::sta::StaArchive::open(&p) else { continue };
        if let Some(i) = a.find("manifest.toml")
            && let Ok(b) = a.read(i)
        {
            return Some(String::from_utf8_lossy(&b).into_owned());
        }
    }
    None
}

/// The STA64 version this build is, as a manifest `engine` requirement sees it.
pub fn engine_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0))
}

impl Manifest {
    /// Does this STA64 build meet the manifest's `engine` requirement (D4)?
    /// No requirement always does. Pre-release builds are compared by their
    /// release number.
    pub fn engine_ok(&self) -> bool {
        let mut v = engine_version();
        v.pre = semver::Prerelease::EMPTY;
        self.engine.as_ref().is_none_or(|r| r.matches(&v))
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn req(field: &str, value: &str) -> Result<VersionReq, ManifestError> {
    VersionReq::parse(value.trim()).map_err(|e| ManifestError::BadRequirement { field: field.to_owned(), value: value.to_owned(), why: e.to_string() })
}

fn strings(v: Option<&toml::Value>) -> Vec<String> {
    match v {
        Some(toml::Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
        Some(toml::Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let mut root: toml::Table = text.parse().map_err(|e: toml::de::Error| ManifestError::Toml(e.to_string()))?;
        let mut m = match root.remove("mod") {
            Some(toml::Value::Table(t)) => t,
            _ => return Err(ManifestError::Missing("section")),
        };
        let mut take = |k: &str| m.remove(k);
        let id = take("id").and_then(|v| v.as_str().map(str::to_owned)).ok_or(ManifestError::Missing("id"))?;
        if !valid_id(&id) {
            return Err(ManifestError::BadId(id));
        }
        let s = |v: Option<toml::Value>| v.and_then(|v| v.as_str().map(str::to_owned));
        let name = s(take("name")).unwrap_or_else(|| id.clone());
        let version = s(take("version")).map(|v| ModVersion::parse(&v)).unwrap_or(ModVersion::Unknown);
        let description = s(take("description")).unwrap_or_default();
        let authors = strings(take("authors").as_ref());
        let licence = s(take("licence")).or_else(|| s(take("license")));
        let engine = s(take("engine")).map(|e| req("engine", &e)).transpose()?;
        let profile = s(take("profile"));
        let kind_text = s(take("kind"));
        let mut dependencies = BTreeMap::new();
        if let Some(toml::Value::Table(d)) = root.remove("dependencies") {
            for (k, v) in d {
                let r = v.as_str().map(|r| req(&format!("dependency {k}"), r)).transpose()?.unwrap_or(VersionReq::STAR);
                dependencies.insert(k.to_ascii_lowercase(), r);
            }
        }
        let kind_explicit = kind_text.is_some();
        let kind = match kind_text.as_deref() {
            Some("addon") => ModKind::Addon,
            Some("standalone") => ModKind::Standalone,
            None if dependencies.is_empty() => ModKind::Standalone,
            None => ModKind::Addon,
            Some(k) => return Err(ManifestError::BadKind(k.to_owned())),
        };
        let (mut load_after, mut load_before) = (Vec::new(), Vec::new());
        if let Some(toml::Value::Table(l)) = root.remove("load") {
            load_after = strings(l.get("after")).into_iter().map(|x| x.to_ascii_lowercase()).collect();
            load_before = strings(l.get("before")).into_iter().map(|x| x.to_ascii_lowercase()).collect();
        }
        // Whatever is left, in [mod] or at the top, is kept.
        let mut extra = root;
        if !m.is_empty() {
            extra.insert("mod".into(), toml::Value::Table(m));
        }
        Ok(Manifest { id, name, version, description, authors, licence, engine, profile, kind, dependencies, load_after, load_before, extra, generated: false, kind_explicit })
    }

    /// The manifest a legacy mod would have, from its `info.ini`.
    pub fn from_info(info: &ModInfo) -> Manifest {
        let mut dependencies = BTreeMap::new();
        // A standalone mod runs over nothing but the game: its ParentMod is not followed (D6).
        if let Some(p) = info.parent.as_ref().filter(|_| !info.standalone) {
            dependencies.insert(mod_id(p), VersionReq::STAR);
        }
        Manifest {
            id: mod_id(&info.folder),
            name: info.name.clone(),
            version: info.version.as_deref().map(ModVersion::parse).unwrap_or(ModVersion::Unknown),
            description: info.description.clone(),
            authors: info.authors.clone(),
            licence: None,
            engine: None,
            profile: info.ini.get("mod", "profile").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned),
            kind: if info.standalone { ModKind::Standalone } else { ModKind::Addon },
            dependencies,
            load_after: Vec::new(),
            load_before: Vec::new(),
            extra: toml::Table::new(),
            generated: true,
            kind_explicit: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
[mod]
id = "federation-expansion"
name = "Federation Expansion"
version = "1.2.0"
description = "Twelve new ships."
authors = ["Immy", "Blade"]
licence = "CC-BY-4.0"
engine = ">=0.4"
profile = "fo4"
kind = "addon"
future_key = 7

[dependencies]
"fleet-ops-4-0" = "^4.0"
Better-Textures = "*"

[load]
after = ["better-textures"]
before = ["Other"]

[later]
x = 1
"#;

    #[test]
    fn parses_every_field_and_keeps_unknown_ones() {
        let m = Manifest::parse(FULL).unwrap();
        assert_eq!((m.id.as_str(), m.name.as_str()), ("federation-expansion", "Federation Expansion"));
        assert_eq!(m.version, ModVersion::Semver(Version::new(1, 2, 0)));
        assert_eq!(m.authors, ["Immy", "Blade"]);
        assert_eq!((m.licence.as_deref(), m.profile.as_deref(), m.kind), (Some("CC-BY-4.0"), Some("fo4"), ModKind::Addon));
        assert!(m.engine.as_ref().unwrap().matches(&Version::new(0, 5, 0)));
        assert_eq!(m.dependencies.keys().collect::<Vec<_>>(), ["better-textures", "fleet-ops-4-0"]);
        assert_eq!((m.load_after.clone(), m.load_before.clone()), (vec!["better-textures".to_string()], vec!["other".to_string()]));
        assert_eq!(m.extra["mod"]["future_key"].as_integer(), Some(7));
        assert!(m.extra.contains_key("later"));
        assert!(!m.generated);
    }

    #[test]
    fn refuses_bad_manifests() {
        assert!(matches!(Manifest::parse("[mod]\nname='x'\n"), Err(ManifestError::Missing("id"))));
        assert!(matches!(Manifest::parse("id = 'x'\n"), Err(ManifestError::Missing("section"))));
        assert!(matches!(Manifest::parse("[mod]\nid='Has Space'\n"), Err(ManifestError::BadId(_))));
        assert!(matches!(Manifest::parse("[mod]\nid='a'\n[dependencies]\nb='not a req'\n"), Err(ManifestError::BadRequirement { .. })));
        assert!(matches!(Manifest::parse("[mod]\nid='a'\nkind='other'\n"), Err(ManifestError::BadKind(_))));
        assert!(matches!(Manifest::parse("[mod\n"), Err(ManifestError::Toml(_))));
        // Kind defaults from dependencies.
        assert_eq!(Manifest::parse("[mod]\nid='a'\n").unwrap().kind, ModKind::Standalone);
        assert_eq!(Manifest::parse("[mod]\nid='a'\n[dependencies]\nb='*'\n").unwrap().kind, ModKind::Addon);
    }

    #[test]
    fn versions() {
        assert_eq!(ModVersion::parse("v1.2.3"), ModVersion::Semver(Version::new(1, 2, 3)));
        assert_eq!(ModVersion::parse("4.0Z"), ModVersion::Legacy("4.0Z".into()));
        assert_eq!(ModVersion::parse(" "), ModVersion::Unknown);
        let r = VersionReq::parse("^4.0").unwrap();
        assert!(ModVersion::parse("4.1.0").satisfies(&r));
        assert!(!ModVersion::parse("5.0.0").satisfies(&r));
        assert!(!ModVersion::parse("4.0Z").satisfies(&r), "a legacy version can't be checked against a range");
        assert!(ModVersion::parse("4.0Z").satisfies(&VersionReq::STAR));
        assert!(ModVersion::Unknown.satisfies(&VersionReq::STAR));
    }
}
