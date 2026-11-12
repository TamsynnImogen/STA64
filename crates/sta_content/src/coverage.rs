//! Coverage report and `why` (CON-34, MODDING_DECISIONS D7).
//!
//! [`check`] reports, for the mod mounted on top of a [`Vfs`] (layer 0, the
//! files it brings):
//!
//! - **ODF key coverage**, weighted by use: every key of every effective ODF
//!   the layer supplies counts once per use, and counts as understood when a
//!   STA64 loader reads it ([`READ_ODF_KEYS`]).
//! - **Changed-but-ignored cfg keys**: `RTS_CFG.h` / `ART_CFG.h` entries whose
//!   value differs from the version the file hides in a lower layer and that
//!   no STA64 code reads ([`READ_CFG_KEYS`]); until the typed `Rules` of
//!   CON-35 reads them, a mod's changes to them have no effect.
//! - **Unresolved map references**: objects, start-field and tech-tree files
//!   a visible map names that no ODF or tech tree file provides.
//!
//! [`why_file`] and [`why_key`] answer which layer (and, for a key, which
//! file and line) supplied what a loader sees. `.h` and `.tt` files are
//! answered through [`crate::layered`], so the provenance of inherited
//! (self-included) entries is the one the loaders use.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use sta_odf::Odf;
use sta_vfs::{Source, Vfs};

use crate::OdfIndex;
use crate::TechTree;
use crate::layered;

/// ODF keys (lower case) some STA64 loader reads. Seed list taken from the
/// getters of `sta_content`, `sta_sim`, `sta_mission` and `sta64`; keys read
/// by rendering and audio helpers through wrappers may be missing, which
/// shows as "not understood" rather than hiding a gap.
pub const READ_ODF_KEYS: &[&str] = &[
    "addon", "atmospheretexturename", "avoidme", "buildhardpoint", "canattack", "classlabel",
    "constructionchunks", "curhealth", "dont_load_sod", "effectradius", "effecttimer",
    "energydrainrate", "fieldradius", "firearc", "fire_arcs", "forwardaccel", "groundtexturename",
    "impulsespeed", "is_starbase", "lightcolor", "maxasteroids", "maxcrewgain", "maxhealth",
    "maxspecialenergy", "minasteroids", "minimappanelarea", "nolatinum", "numberofraces",
    "numberofworkerbees", "orbitingspeed", "ordname", "physicsfile", "possiblecraftnames",
    "preferredposition", "queueinstantdeduct", "radius", "rotationspeed", "scalesod", "shotdelay",
    "specialenergycost", "specialenergyrate", "turnalpha", "turnomega", "turnomegafractionatrest",
    "unitname", "weldingbeamsprite", "workerbeehardpoints", "wreckage", "wpnname", "sta64_mapconfinementinitial",
    "sta64_queuelimit", "turrets", "weapon_damage_controls",
    // Found by intersecting the keys of the installed ODFs with the string
    // literals of the loaders (`check_pack <root> --keys` lists the rest).
    "tooltip", "basename", "race", "fireball", "officercost", "curshields", "maxshields",
    "shieldrate", "eventacknowledge", "eventselect", "buildtime", "rangescan", "crewcost", "range",
    "lifespan", "shotspeed", "damagebase", "sprite", "firesound", "invincible", "shotgeometry",
    "restrictfirearc", "coloractive", "colordisabled", "colormouseover", "colornormal",
    "colorpressed", "buttonname", "commandname", "combatspeed", "forwardcontroldistance",
    "pathleaddistance", "tooclosetoturn", "turnalphafractionatrest", "turncontrolangle",
    "turncontrolsquared", "crewredstatus", "crewyellowstatus", "displaykey", "disableweapons",
    "interfaceconfiguration", "interfacesprites", "maxdilithium", "multiplayermusic",
    "singleplayermusic", "miningrate", "dilithiumtransferrate", "timeuntilclosure",
    "timeuntiltakeoff", "timewelding", "weldingbeamwidth", "workerbeename", "instantactionslot",
    "commandnames",
];

/// Prefixes of numbered ODF keys (`weapon1`, `buildItem3`, ...) that are read.
pub const READ_ODF_PREFIXES: &[&str] = &["weapon", "builditem", "feature_", "sta64_"];

/// `RTS_CFG.h` / `ART_CFG.h` names some STA64 code reads.
pub const READ_CFG_KEYS: &[&str] = &[
    "MP_OFFICER_LIMIT",
    "MP_OFFICER_LIST_SIZE",
    "SINGLE_PLAYER_OFFICER_LIMIT",
    "SCROLL_BORDER_WIDTH",
    "cfgSOUND_BUTTON_SAMPLE",
    "cfgSOUND_SHELL_BUTTON_HOVER",
    "cfgSOUND_SHELL_BUTTON_CLICK",
    "cfgSOUND_INGAME_CHAT_SEND",
    "cfgSOUND_INGAME_CHAT_RECEIVE",
    "DETAIL_2_STARFIELD_BACKGROUND_STAR_MAX_SIZE",
    "DETAIL_2_STARFIELD_BACKGROUND_STAR_SIZE_VARIATION",
    "DETAIL_2_STARFIELD_BACKGROUND_STAR_MAX_DISTANCE",
    "DETAIL_2_STARFIELD_BACKGROUND_STAR_DISTANCE_VARIATION",
];

/// The cfg files checked for changed keys.
pub const CFG_FILES: &[&str] = &["RTS_CFG.h", "ART_CFG.h"];

fn odf_key_read(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    READ_ODF_KEYS.contains(&k.as_str())
        || READ_ODF_PREFIXES.iter().any(|p| {
            let numbered = *p == "weapon" || *p == "builditem";
            k.strip_prefix(p).is_some_and(|rest| !numbered || (!rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())))
        })
}

fn cfg_key_read(key: &str) -> bool {
    READ_CFG_KEYS.contains(&key)
}

/// ODF key coverage of one layer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OdfCoverage {
    /// Effective ODFs whose winning file is in the layer.
    pub files: usize,
    /// Key uses across those files, and how many were understood.
    pub uses: usize,
    pub understood: usize,
    /// Keys not understood with their use counts, most used first.
    pub unknown: Vec<(String, usize)>,
    /// ODFs that failed to resolve (include missing or a cycle), with the reason.
    pub errors: Vec<(String, String)>,
}

impl OdfCoverage {
    /// Share of key uses understood, 0 to 100.
    pub fn percent(&self) -> f64 {
        if self.uses == 0 { 100.0 } else { 100.0 * self.understood as f64 / self.uses as f64 }
    }
}

/// A cfg entry a mod changes relative to the file it hides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfgChange {
    pub file: String,
    pub key: String,
    /// Value in the hidden version; `None` when the mod adds the key.
    pub was: Option<String>,
    pub now: String,
    pub layer: String,
    pub line: usize,
    /// STA64 reads this key.
    pub read: bool,
}

/// A map reference nothing provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRef {
    pub map: String,
    /// `object`, `starfield` or `techtree`.
    pub kind: &'static str,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Layers, top first; the report covers the first.
    pub layers: Vec<String>,
    pub odf: OdfCoverage,
    pub cfg: Vec<CfgChange>,
    pub maps: Vec<MapRef>,
    pub maps_checked: usize,
    /// Maps that did not parse.
    pub map_errors: Vec<(String, String)>,
}

/// Check the layer on top of `vfs` (the mod, or `Data` with none).
pub fn check(vfs: &Vfs) -> Report {
    let mut report = Report { layers: vfs.layers().iter().map(|l| l.name().to_owned()).collect(), ..Report::default() };
    if vfs.layers().is_empty() {
        return report;
    }
    let (index, _) = OdfIndex::build(vfs);
    odf_coverage(vfs, &index, &mut report);
    for file in CFG_FILES {
        report.cfg.extend(cfg_changes(vfs, file));
    }
    map_refs(vfs, &index, &mut report);
    report
}

fn odf_coverage(vfs: &Vfs, index: &OdfIndex, report: &mut Report) {
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    let cov = &mut report.odf;
    for (stem, entry) in index.iter() {
        if entry.layer != 0 {
            continue;
        }
        let file = entry.path.rsplit('/').next().unwrap_or(&entry.path);
        match sta_odf::resolve_layered(file, &mut |name, version| index.load(vfs, name, version)) {
            Ok(odf) => {
                cov.files += 1;
                for key in odf.keys() {
                    cov.uses += 1;
                    if odf_key_read(key) {
                        cov.understood += 1;
                    } else {
                        *unknown.entry(key.to_ascii_lowercase()).or_default() += 1;
                    }
                }
            }
            Err(e) => cov.errors.push((stem.to_owned(), e.to_string())),
        }
    }
    let mut unknown: Vec<_> = unknown.into_iter().collect();
    unknown.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    cov.unknown = unknown;
}

/// Entries of `name` that the top version changes against what it hides.
pub fn cfg_changes(vfs: &Vfs, name: &str) -> Vec<CfgChange> {
    let Some(top) = layered::resolve_header(vfs, name, 0) else { return Vec::new() };
    // Nothing hidden below: nothing to be changed against.
    let Some(below) = layered::resolve_header(vfs, name, 1) else { return Vec::new() };
    let below = Some(below);
    let mut out = Vec::new();
    for (key, e) in &top.entries {
        // Only what the top layer's own file supplies, not what it inherits.
        if e.version != 0 {
            continue;
        }
        let was = below.as_ref().and_then(|b| b.entries.get(key)).map(|b| b.value.clone());
        if was.as_deref() == Some(e.value.as_str()) {
            continue;
        }
        out.push(CfgChange { file: e.file.clone(), key: key.clone(), was, now: e.value.clone(), layer: e.layer.clone(), line: e.line, read: cfg_key_read(key) });
    }
    out.sort_by_key(|c| c.line);
    out
}

fn map_refs(vfs: &Vfs, index: &OdfIndex, report: &mut Report) {
    for path in vfs.list("bzn") {
        if !path.ends_with(".bzn") {
            continue;
        }
        let Some(Ok(bytes)) = vfs.read(&path) else { continue };
        let map = match sta_bzn::parse(&bytes) {
            Ok(m) => m,
            Err(e) => {
                report.map_errors.push((path.clone(), e.to_string()));
                continue;
            }
        };
        report.maps_checked += 1;
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let mut seen = std::collections::BTreeSet::new();
        for o in &map.objects {
            let stem = o.odf.trim();
            if !stem.is_empty() && index.get(stem).is_none() && seen.insert(stem.to_ascii_lowercase()) {
                report.maps.push(MapRef { map: name.clone(), kind: "object", name: stem.to_owned() });
            }
        }
        let star = map.starfield_odf.trim();
        if !star.is_empty() && index.get(star).is_none() {
            report.maps.push(MapRef { map: name.clone(), kind: "starfield", name: star.to_owned() });
        }
        let tt = map.tech_tree_file.trim();
        if !tt.is_empty() && !tt.eq_ignore_ascii_case("none") && vfs.resolve(&format!("techtree/{tt}")).is_none() && vfs.resolve(tt).is_none() {
            report.maps.push(MapRef { map: name, kind: "techtree", name: tt.to_owned() });
        }
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "layers: {}", self.layers.join(" > "))?;
        let o = &self.odf;
        writeln!(f, "odf: {} effective files from {}, {}/{} key uses understood ({:.1}%)", o.files, self.layers.first().map_or("-", String::as_str), o.understood, o.uses, o.percent())?;
        for (k, n) in o.unknown.iter().take(15) {
            writeln!(f, "  not understood: {k} x{n}")?;
        }
        if o.unknown.len() > 15 {
            writeln!(f, "  ... {} more keys", o.unknown.len() - 15)?;
        }
        for (s, e) in &o.errors {
            writeln!(f, "  unresolved odf {s}: {e}")?;
        }
        let ignored: Vec<_> = self.cfg.iter().filter(|c| !c.read).collect();
        writeln!(f, "cfg: {} changed entries, {} ignored by STA64", self.cfg.len(), ignored.len())?;
        for c in &self.cfg {
            let was = c.was.as_deref().map_or("new".to_owned(), |w| format!("was {w}"));
            writeln!(f, "  {} {}: {} = {} ({was}; {}:{})", if c.read { "read   " } else { "IGNORED" }, c.file, c.key, c.now, c.layer, c.line)?;
        }
        writeln!(f, "maps: {} checked, {} unresolved references", self.maps_checked, self.maps.len())?;
        for m in self.maps.iter().take(30) {
            writeln!(f, "  {}: {} {}", m.map, m.kind, m.name)?;
        }
        if self.maps.len() > 30 {
            writeln!(f, "  ... {} more", self.maps.len() - 30)?;
        }
        for (m, e) in &self.map_errors {
            writeln!(f, "  unreadable map {m}: {e}")?;
        }
        Ok(())
    }
}

/// One layer's copy of a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    pub layer: String,
    /// Where in the layer: a loose path, or the archive file name and kind.
    pub from: String,
    /// The copy the game sees.
    pub winner: bool,
}

/// Every layer providing `path`, winner first, as [`Vfs::resolve_all`] orders them.
pub fn why_file(vfs: &Vfs, path: &str) -> Vec<FileHit> {
    vfs.resolve_all(path)
        .into_iter()
        .enumerate()
        .map(|(i, (layer, source))| FileHit { layer: layer.to_owned(), from: describe(vfs, layer, source), winner: i == 0 })
        .collect()
}

fn describe(vfs: &Vfs, layer: &str, source: &Source) -> String {
    let Some(l) = vfs.layer_index(layer).and_then(|i| vfs.layers().get(i)) else { return String::new() };
    let name = |p: &std::path::Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match source {
        Source::Loose(p) => p.display().to_string(),
        Source::Packed { archive, .. } => l.archives.get(*archive).map_or(String::new(), |(p, _)| format!("{} (fpq)", name(p))),
        Source::Zfs { archive, .. } => l.zfs.get(*archive).map_or(String::new(), |(p, _)| format!("{} (zfs)", name(p))),
        Source::Sta { archive, .. } => l.sta.get(*archive).map_or(String::new(), |(p, _)| format!("{} (sta)", name(p))),
        Source::Whiteout => "whiteout".into(),
    }
}

/// A tech tree entry as its line reads: the prerequisites, or why it is withheld.
fn tt_value(tree: &TechTree, key: &str) -> String {
    let Some(e) = tree.entry(key) else { return String::new() };
    let needs = tree.prerequisites(key).join(" ");
    match (e.withheld, needs.is_empty()) {
        (true, _) => "withheld".to_owned(),
        (false, true) => "no prerequisites".to_owned(),
        (false, false) => format!("needs {needs}"),
    }
}

/// The assignment of a key that a loader sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyWhy {
    /// The file the assignment is in (an ODF may include others).
    pub file: String,
    pub layer: String,
    pub line: usize,
    pub value: String,
}

/// Which layer, file and line supplied `key` of `file` (an ODF stem or file
/// name, `RTS_CFG.h`, `ART_CFG.h` or another `.h`).
pub fn why_key(vfs: &Vfs, file: &str, key: &str) -> Option<KeyWhy> {
    if file.to_ascii_lowercase().ends_with(".h") {
        let h = layered::resolve_header(vfs, file, 0)?;
        let e = h.entries.get(key)?;
        return Some(KeyWhy { file: e.file.clone(), layer: e.layer.clone(), line: e.line, value: e.value.clone() });
    }
    let lower = file.to_ascii_lowercase();
    if lower.ends_with(".tt") || lower.ends_with(".xml") {
        // A tech tree: the key is an ODF name; its line may sit in an included file or a lower layer.
        let stem = lower.rsplit(['/', '\\']).next().unwrap_or(&lower).rsplit_once('.').map_or(lower.as_str(), |(s, _)| s).to_owned();
        let path = TechTree::find(vfs, &stem)?;
        let (dir, _) = path.rsplit_once('/').map_or((String::new(), path.as_str()), |(d, n)| (format!("{d}/"), n));
        let (tree, _) = TechTree::from_vfs(vfs, &stem).ok()??;
        let src = tree.source(key)?;
        let (layer, from) = TechTree::locate(vfs, &dir, &src.file, src.version)?;
        return Some(KeyWhy { file: from, layer, line: src.line, value: tt_value(&tree, key) });
    }
    let (index, _) = OdfIndex::build(vfs);
    let entry = index.get(file)?;
    let selected = entry.path.rsplit('/').next().unwrap_or(&entry.path).to_owned();
    let odf: Odf = sta_odf::resolve_layered(&selected, &mut |name, version| index.load(vfs, name, version)).ok()?;
    let src = odf.source(key)?;
    let hit = index.versions(&src.file).get(src.version).map(|e| (e.layer, e.path.clone()));
    let layer = hit.as_ref().and_then(|(l, _)| vfs.layers().get(*l)).map_or_else(String::new, |l| l.name().to_owned());
    Some(KeyWhy { file: hit.map_or_else(|| src.file.to_string(), |(_, p)| p), layer, line: src.line, value: odf.raw(key).unwrap_or_default().to_owned() })
}

/// `why_file` and (with a key) `why_key` as text.
pub fn why_text(vfs: &Vfs, file: &str, key: Option<&str>) -> String {
    let mut out = String::new();
    let norm = if file.contains('/') || file.contains('\\') || file.contains('.') { file.to_owned() } else { format!("{file}.odf") };
    let lower = norm.to_ascii_lowercase();
    let mut hits = why_file(vfs, &norm);
    if hits.is_empty() && (lower.ends_with(".tt") || lower.ends_with(".xml")) {
        // Tech trees sit under techtree/ (or the root, as Armada I's ZFS names them).
        hits = why_file(vfs, &format!("techtree/{norm}"));
    }
    for h in hits.iter().chain(odf_hits(vfs, file).iter()) {
        let _ = writeln!(out, "{} {}  {}", if h.winner { "wins   " } else { "hidden " }, h.layer, h.from);
    }
    if out.is_empty() {
        let _ = writeln!(out, "{file}: no layer provides it");
    }
    if let Some(k) = key {
        match why_key(vfs, file, k) {
            Some(w) => {
                let _ = writeln!(out, "key {k} = {} from {} (layer {}) line {}", w.value, w.file, w.layer, w.line);
            }
            None => {
                let _ = writeln!(out, "key {k}: not set");
            }
        }
    }
    out
}

/// ODFs are found by file name anywhere under `odf/`; list their copies.
fn odf_hits(vfs: &Vfs, name: &str) -> Vec<FileHit> {
    if name.contains('/') || name.contains('\\') || name.contains('.') && !name.to_ascii_lowercase().ends_with(".odf") {
        return Vec::new();
    }
    let (index, _) = OdfIndex::build(vfs);
    index
        .versions(name)
        .into_iter()
        .enumerate()
        .map(|(i, e)| FileHit { layer: vfs.layers()[e.layer].name().to_owned(), from: e.path.clone(), winner: i == 0 })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(name: &str) -> Tmp {
            let d = std::env::temp_dir().join(format!("sta_cov_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&d);
            Tmp(d)
        }
        fn write(&self, rel: &str, text: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Data plus a child mod that changes cfg lines and extends an ODF.
    fn install(name: &str) -> (Tmp, Vfs) {
        let t = Tmp::new(name);
        t.write("Data/RTS_CFG.h", "float SHIELD_DAMAGE_PROTECTION = 0.8;\nint SCROLL_BORDER_WIDTH = 2;\nint cfgFREIGHTERS_PER_NEBULA = 1;\n");
        t.write("Data/odf/ships/base.odf", "classLabel = \"starship\"\nmaxHealth = 100\nmystery = 1\n");
        t.write("Data/Mods/Child/info.ini", "[mod]\nName=Child\n");
        t.write("Data/Mods/Child/RTS_CFG.h", "float SHIELD_DAMAGE_PROTECTION = 0.9; // up\nint SCROLL_BORDER_WIDTH = 2;\nint cfgFREIGHTERS_PER_NEBULA = 1;\nint NEW_KEY = 5;\n");
        t.write("Data/Mods/Child/odf/ships/child.odf", "#include \"base.odf\"\nmaxHealth = 200\nweapon1 = \"x\"\nodd = 2\n");
        let vfs = Vfs::mount_install(&t.0, Some("Child")).unwrap();
        (t, vfs)
    }

    #[test]
    fn reports_changed_ignored_cfg_keys_and_odf_coverage() {
        let (_t, vfs) = install("report");
        let r = check(&vfs);
        let keys: Vec<(&str, bool, Option<&str>, usize)> = r.cfg.iter().map(|c| (c.key.as_str(), c.read, c.was.as_deref(), c.line)).collect();
        assert_eq!(keys, [("SHIELD_DAMAGE_PROTECTION", false, Some("0.8"), 1), ("NEW_KEY", false, None, 4)]);
        assert_eq!(r.odf.files, 1);
        // classLabel, maxHealth (200 wins), mystery, weapon1, odd: 5 uses, 3 understood.
        assert_eq!((r.odf.uses, r.odf.understood), (5, 3));
        assert_eq!(r.odf.unknown, [("mystery".to_owned(), 1), ("odd".to_owned(), 1)]);
        assert!(r.to_string().contains("IGNORED"));
    }

    #[test]
    fn why_names_layer_file_and_line() {
        let (_t, vfs) = install("why");
        let hits = why_file(&vfs, "RTS_CFG.h");
        assert_eq!(hits.iter().map(|h| (h.layer.as_str(), h.winner)).collect::<Vec<_>>(), [("Child", true), ("Data", false)]);
        let w = why_key(&vfs, "child", "maxHealth").unwrap();
        assert_eq!((w.layer.as_str(), w.line, w.value.as_str()), ("Child", 2, "200"));
        let w = why_key(&vfs, "child", "classLabel").unwrap();
        assert_eq!((w.layer.as_str(), w.line, w.file.ends_with("base.odf")), ("Data", 1, true));
        let w = why_key(&vfs, "RTS_CFG.h", "SHIELD_DAMAGE_PROTECTION").unwrap();
        assert_eq!((w.layer.as_str(), w.line, w.value.as_str()), ("Child", 1, "0.9"));
        assert!(why_text(&vfs, "child", Some("odd")).contains("line 4"));
    }
}

