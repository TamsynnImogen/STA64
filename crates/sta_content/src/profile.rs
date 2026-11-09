//! Profile detection, the per-profile defaults table and feature overrides
//! (CON-05). [`Profile`] and [`Features`] are the S5 seam types in
//! `features.rs`; this file gives them behaviour.
//!
//! # Detection ([`Profile::detect`])
//!
//! Layers are the mounted [`Vfs`] layers, top-most (child mod) first; the
//! last one is normally `Data`. The first rule that matches wins:
//!
//! 1. **Explicit.** A `Profile = <name>` key in the `[Mod]` section of any
//!    layer's `info.ini`, top-most layer first. Names: `native`/`sta64`,
//!    `fo4`/`fo`, `sta2`/`sta2classic`, `sta1`/`sta1classic` (see
//!    [`Profile::from_name`]).
//! 2. **Armada I layout.** Any layer holds a `.zfs` archive. (An `addon/`
//!    directory is not a signature: Armada II and FO ship one too, and a
//!    real FO Roots install detected as Armada I on it.)
//! 3. **Fleet Operations.** Any layer other than `Data` whose folder name or
//!    `info.ini` `Name` contains `fleet op`/`fleetop`/`fo4`/`fo 4`, or an
//!    `mmm_loader.dll` (FO's mission loader) at a layer root or the install
//!    root above `Data`.
//! 4. **Stock Armada II.** Any other install: a `Data` layer, or a plain game
//!    folder with an `odf/` directory, `ODF.fpq` or `Armada2.exe`, possibly
//!    with mods that match nothing above.
//! 5. **Native.** No `Data` layer and nothing matched (also the fallback for
//!    unknown or custom content).
//!
//! The FO and A1 signatures are heuristics from the documented layouts; they
//! are unverified against a real install here (see the install-gated test).
//!
//! # Precedence ([`resolve_features`])
//!
//! profile defaults < mod overrides (`[Features]` of `info.ini`, child mod
//! over parent) < per-ODF overrides (`Feature_<name>` keys). Each source is an
//! [`Overrides`]; later sources win per feature.
//!
//! Hookup: `ContentDb::load` calls `Profile::detect(vfs)` and
//! `Overrides::from_vfs(vfs)`; db-wide features are
//! `resolve_features(profile, &[&mod_ov])`, and a def's own are
//! `resolve_features(profile, &[&mod_ov, &Overrides::from_odf_keys(..)])`,
//! stored in `UnitDef::features` / `WeaponDef::features`.

use std::collections::BTreeMap;
use std::path::Path;

use sta_vfs::Vfs;

use crate::features::{Features, Profile};

/// Profile names as written in `info.ini` (`Profile=`) and `.sta` manifests.
const PROFILE_NAMES: [(&str, Profile); 8] = [
    ("native", Profile::Native),
    ("sta64", Profile::Native),
    ("fo4", Profile::Fo4),
    ("fo", Profile::Fo4),
    ("sta2", Profile::Sta2Classic),
    ("sta2classic", Profile::Sta2Classic),
    ("sta1", Profile::Sta1Classic),
    ("sta1classic", Profile::Sta1Classic),
];

impl Profile {
    /// Every profile, for tables and tests.
    pub const ALL: [Profile; 4] = [Profile::Native, Profile::Fo4, Profile::Sta2Classic, Profile::Sta1Classic];

    /// Canonical lower-case name.
    pub fn name(self) -> &'static str {
        match self {
            Profile::Native => "native",
            Profile::Fo4 => "fo4",
            Profile::Sta2Classic => "sta2classic",
            Profile::Sta1Classic => "sta1classic",
        }
    }

    /// Parse a profile name; ignores case, spaces, `_` and `-`
    /// (`"STA2 Classic"`, `"sta2_classic"`).
    pub fn from_name(s: &str) -> Option<Profile> {
        let k: String = s.chars().filter(|c| !matches!(c, ' ' | '_' | '-')).collect::<String>().to_ascii_lowercase();
        PROFILE_NAMES.iter().find(|(n, _)| *n == k).map(|(_, p)| *p)
    }

    /// The defaults table: which rule switches this profile turns on.
    ///
    /// First-pass values from docs/NATIVE_FEATURES.md (Source and Profile
    /// columns); tune per row as the sims land. Every row is pinned by a test.
    pub fn features(self) -> Features {
        let off = Features::default();
        match self {
            // Everything STA64 implements, except A1 compatibility.
            Profile::Native => Features {
                weapon_damage_controls: true,
                point_defense_cycles: true,
                fire_arcs: true,
                directional_shields: true,
                energy_systems: true,
                turrets: true,
                team_change_weapons: true,
                extra_resources: true,
                officer_capacity: true,
                multi_yard_builds: true,
                construction_rigs: true,
                upgrade_pods: true,
                hybrid_yards: true,
                breeders: true,
                collective_xp: true,
                squadrons: true,
                station_rotation: true,
                wreckage: true,
                craft_identity: true,
                cheats: true,
                a1_compat: false,
                pay_on_queue: true,
            },
            // FOH rows: damage controls, PD cycles, resources, officers, rigs.
            Profile::Fo4 => Features {
                weapon_damage_controls: true,
                point_defense_cycles: true,
                turrets: true,
                extra_resources: true,
                officer_capacity: true,
                multi_yard_builds: true,
                construction_rigs: true,
                ..off
            },
            // Stock A2: plain rules, turrets only.
            Profile::Sta2Classic => Features { turrets: true, ..off },
            // A1: its own class labels and health model, officers.
            Profile::Sta1Classic => Features { officer_capacity: true, a1_compat: true, ..off },
        }
    }

    /// Detect the profile of a mounted install; see the module docs for the
    /// rule list.
    pub fn detect(vfs: &Vfs) -> Profile {
        let layers = vfs.layers();
        let infos: Vec<Option<Ini>> = layers.iter().map(|l| read_info(&l.root)).collect();

        // 1. Explicit.
        for ini in infos.iter().flatten() {
            if let Some(p) = ini.get("mod", "profile").and_then(Profile::from_name) {
                return p;
            }
        }
        // 2. Armada I layout.
        for l in layers {
            if !l.zfs.is_empty() || has_zfs_file(&l.root) {
                return Profile::Sta1Classic;
            }
        }
        // 3. Fleet Operations.
        let mut has_data = false;
        for (l, ini) in layers.iter().zip(&infos) {
            if l.name.eq_ignore_ascii_case("data") {
                has_data = true;
                if l.root.parent().is_some_and(|r| has_child(r, "mmm_loader.dll", false)) {
                    return Profile::Fo4;
                }
                continue;
            }
            if has_child(&l.root, "odf", true) || has_child(&l.root, "odf.fpq", false) || has_child(&l.root, "armada2.exe", false) {
                has_data = true;
            }
            let name = ini.as_ref().and_then(|i| i.get("mod", "name")).unwrap_or("");
            if looks_like_fo(&l.name) || looks_like_fo(name) || has_child(&l.root, "mmm_loader.dll", false) {
                return Profile::Fo4;
            }
        }
        // 4. / 5.
        if has_data { Profile::Sta2Classic } else { Profile::Native }
    }
}

fn looks_like_fo(s: &str) -> bool {
    let k = s.to_ascii_lowercase();
    k.contains("fleet op") || k.contains("fleetop") || k.contains("fo4") || k.contains("fo 4")
}

fn has_child(dir: &Path, name: &str, is_dir: bool) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_dir() == is_dir)
}

fn has_zfs_file(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("zfs")))
}

/// A parsed `info.ini`: lower-cased `(section, key) -> value`.
struct Ini(BTreeMap<(String, String), String>);

impl Ini {
    fn parse(text: &str) -> Ini {
        let mut m = BTreeMap::new();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = s.trim().to_ascii_lowercase();
            } else if let Some((k, v)) = line.split_once('=') {
                let v = v.split([';', '#']).next().unwrap_or("").trim().trim_matches('"').trim();
                m.insert((section.clone(), k.trim().to_ascii_lowercase()), v.to_owned());
            }
        }
        Ini(m)
    }

    fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.0.get(&(section.to_owned(), key.to_owned())).map(String::as_str)
    }
}

fn read_info(root: &Path) -> Option<Ini> {
    let p = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("info.ini"))?
        .path();
    Some(Ini::parse(&String::from_utf8_lossy(&std::fs::read(p).ok()?)))
}

/// Switch names, in `Features` declaration order.
pub const FEATURE_NAMES: [&str; 22] = [
    "weapon_damage_controls",
    "point_defense_cycles",
    "fire_arcs",
    "directional_shields",
    "energy_systems",
    "turrets",
    "team_change_weapons",
    "extra_resources",
    "officer_capacity",
    "multi_yard_builds",
    "construction_rigs",
    "upgrade_pods",
    "hybrid_yards",
    "breeders",
    "collective_xp",
    "squadrons",
    "station_rotation",
    "wreckage",
    "craft_identity",
    "cheats",
    "a1_compat",
    "pay_on_queue",
];

impl Features {
    fn slot(&mut self, name: &str) -> Option<&mut bool> {
        Some(match name {
            "weapon_damage_controls" => &mut self.weapon_damage_controls,
            "point_defense_cycles" => &mut self.point_defense_cycles,
            "fire_arcs" => &mut self.fire_arcs,
            "directional_shields" => &mut self.directional_shields,
            "energy_systems" => &mut self.energy_systems,
            "turrets" => &mut self.turrets,
            "team_change_weapons" => &mut self.team_change_weapons,
            "extra_resources" => &mut self.extra_resources,
            "officer_capacity" => &mut self.officer_capacity,
            "multi_yard_builds" => &mut self.multi_yard_builds,
            "construction_rigs" => &mut self.construction_rigs,
            "upgrade_pods" => &mut self.upgrade_pods,
            "hybrid_yards" => &mut self.hybrid_yards,
            "breeders" => &mut self.breeders,
            "collective_xp" => &mut self.collective_xp,
            "squadrons" => &mut self.squadrons,
            "station_rotation" => &mut self.station_rotation,
            "wreckage" => &mut self.wreckage,
            "craft_identity" => &mut self.craft_identity,
            "cheats" => &mut self.cheats,
            "a1_compat" => &mut self.a1_compat,
            "pay_on_queue" => &mut self.pay_on_queue,
            _ => return None,
        })
    }

    /// Set a switch by its [`FEATURE_NAMES`] name; false if unknown.
    pub fn set(&mut self, name: &str, on: bool) -> bool {
        match self.slot(&name.to_ascii_lowercase()) {
            Some(s) => {
                *s = on;
                true
            }
            None => false,
        }
    }

    /// Read a switch by name.
    pub fn get(&self, name: &str) -> Option<bool> {
        let i = FEATURE_NAMES.iter().position(|n| n.eq_ignore_ascii_case(name))?;
        Some(self.bits()[i])
    }
}

/// One source of feature overrides: switch name -> forced value. Unknown
/// names are kept out and listed in [`Overrides::unknown`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    set: BTreeMap<&'static str, bool>,
    /// Names that matched no switch (for a warning).
    pub unknown: Vec<String>,
}

/// The ODF key prefix: `Feature_point_defense_cycles = 0`.
pub const ODF_KEY_PREFIX: &str = "feature_";

impl Overrides {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Force `name` on or off. Returns false (and records it) if unknown.
    pub fn insert(&mut self, name: &str, on: bool) -> bool {
        match FEATURE_NAMES.iter().find(|n| n.eq_ignore_ascii_case(name)) {
            Some(n) => {
                self.set.insert(n, on);
                true
            }
            None => {
                self.unknown.push(name.to_owned());
                false
            }
        }
    }

    /// The forced value for `name`, if any.
    pub fn get(&self, name: &str) -> Option<bool> {
        self.set.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| *v)
    }

    /// Per-ODF overrides from raw `(key, value)` pairs: keys named
    /// `Feature_<switch>` with a value of `1`/`true`/`yes`/`on` or
    /// `0`/`false`/`no`/`off`. Other keys are ignored.
    pub fn from_odf_keys<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Overrides {
        let mut o = Overrides::default();
        for (k, v) in pairs {
            let k = k.to_ascii_lowercase();
            if let (Some(name), Some(on)) = (k.strip_prefix(ODF_KEY_PREFIX), parse_bool(v)) {
                o.insert(name, on);
            }
        }
        o
    }

    /// Overrides from the `[Features]` section of one `info.ini` text.
    pub fn from_info_ini(text: &str) -> Overrides {
        let mut o = Overrides::default();
        o.add_ini(&Ini::parse(text));
        o
    }

    /// Mod-level overrides of a mounted chain: every layer's `[Features]`,
    /// child mod over parent (layers are top-most first).
    pub fn from_vfs(vfs: &Vfs) -> Overrides {
        let mut out = Overrides::default();
        for l in vfs.layers().iter().rev() {
            if let Some(ini) = read_info(&l.root) {
                out.add_ini(&ini);
            }
        }
        out
    }

    fn add_ini(&mut self, ini: &Ini) {
        for ((section, key), v) in &ini.0 {
            if section == "features"
                && let Some(on) = parse_bool(v)
            {
                self.insert(key, on);
            }
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// The profile's defaults with `sources` applied in order; later sources win.
/// Pass `[mod, odf]` for a per-def result: defaults < mod < ODF.
pub fn resolve_features(profile: Profile, sources: &[&Overrides]) -> Features {
    let mut f = profile.features();
    for o in sources {
        for (name, on) in &o.set {
            f.set(name, *on);
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Tmp {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!("sta64-profile-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn file(&self, rel: &str, body: &str) -> &Tmp {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
            self
        }
        /// A real (one-file) Armada I archive at `rel`.
        fn zfs(&self, rel: &str) -> &Tmp {
            let f = sta_vfs::zfs::ZfsFile { name: "a.odf".into(), data: b"x".to_vec(), time: 0 };
            let bytes = sta_vfs::zfs::write(&[f], 0x9466_279b).unwrap();
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, bytes).unwrap();
            self
        }
        fn detect(&self, active: Option<&str>) -> Profile {
            Profile::detect(&Vfs::mount_install(&self.0, active).unwrap())
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn on(f: &Features) -> Vec<&'static str> {
        FEATURE_NAMES.iter().copied().filter(|n| f.get(n) == Some(true)).collect()
    }

    #[test]
    fn names_cover_every_bit() {
        let mut f = Features::default();
        for n in FEATURE_NAMES {
            assert!(f.set(n, true), "{n}");
        }
        assert!(f.bits().iter().all(|b| *b));
        assert!(!f.set("nope", true));
    }

    #[test]
    fn name_round_trip() {
        for p in Profile::ALL {
            assert_eq!(Profile::from_name(p.name()), Some(p));
        }
        assert_eq!(Profile::from_name("STA2 Classic"), Some(Profile::Sta2Classic));
        assert_eq!(Profile::from_name("sta1-classic"), Some(Profile::Sta1Classic));
        assert_eq!(Profile::from_name("bogus"), None);
    }

    #[test]
    fn defaults_table_native() {
        let f = Profile::Native.features();
        assert_eq!(on(&f).len(), 21);
        assert!(!f.a1_compat && f.pay_on_queue && f.cheats);
    }

    #[test]
    fn defaults_table_fo4() {
        assert_eq!(
            on(&Profile::Fo4.features()),
            [
                "weapon_damage_controls",
                "point_defense_cycles",
                "turrets",
                "extra_resources",
                "officer_capacity",
                "multi_yard_builds",
                "construction_rigs"
            ]
        );
        assert!(!Profile::Fo4.features().pay_on_queue);
    }

    #[test]
    fn defaults_table_sta2_classic() {
        assert_eq!(on(&Profile::Sta2Classic.features()), ["turrets"]);
    }

    #[test]
    fn defaults_table_sta1_classic() {
        assert_eq!(on(&Profile::Sta1Classic.features()), ["officer_capacity", "a1_compat"]);
    }

    #[test]
    fn override_precedence() {
        // FO4: point_defense_cycles on by default.
        let mut modov = Overrides::default();
        modov.insert("point_defense_cycles", false);
        modov.insert("fire_arcs", true);
        let mut odf = Overrides::from_odf_keys([("Feature_Fire_Arcs", "0"), ("Name", "x"), ("feature_wreckage", "yes")]);
        assert_eq!(odf.get("fire_arcs"), Some(false));

        let base = resolve_features(Profile::Fo4, &[]);
        assert!(base.point_defense_cycles && !base.fire_arcs);
        let m = resolve_features(Profile::Fo4, &[&modov]);
        assert!(!m.point_defense_cycles && m.fire_arcs);
        let d = resolve_features(Profile::Fo4, &[&modov, &odf]);
        assert!(!d.point_defense_cycles && !d.fire_arcs && d.wreckage, "ODF beats mod beats default");
        assert!(!odf.insert("bogus", true));
        assert_eq!(odf.unknown, ["bogus"]);
    }

    #[test]
    fn info_ini_features_section() {
        let o = Overrides::from_info_ini("[Mod]\nName=X\n[Features]\nTurrets = 0 ; no\nwreckage=true\nbad=maybe\n");
        assert_eq!(o.get("turrets"), Some(false));
        assert_eq!(o.get("wreckage"), Some(true));
        assert!(o.get("bad").is_none() && o.unknown.is_empty());
    }

    #[test]
    fn child_mod_overrides_parent() {
        let t = Tmp::new();
        t.file("Data/Mods/Parent/info.ini", "[Mod]\nName=Parent\n[Features]\nwreckage=1\nturrets=1\n")
            .file("Data/Mods/Child/info.ini", "[Mod]\nParentMod=Parent\n[Features]\nwreckage=0\n");
        let vfs = Vfs::mount_install(&t.0, Some("Child")).unwrap();
        let o = Overrides::from_vfs(&vfs);
        assert_eq!((o.get("wreckage"), o.get("turrets")), (Some(false), Some(true)));
    }

    #[test]
    fn detects_stock_a2() {
        let t = Tmp::new();
        t.file("Data/odf/a.odf", "");
        assert_eq!(t.detect(None), Profile::Sta2Classic);
    }

    #[test]
    fn detects_a2_mod_with_no_signature() {
        let t = Tmp::new();
        t.file("Data/Mods/Plain/info.ini", "[Mod]\nName=Plain Mod\n");
        assert_eq!(t.detect(Some("Plain")), Profile::Sta2Classic);
    }

    #[test]
    fn detects_a1_layout() {
        let t = Tmp::new();
        t.zfs("Data/assets.zfs");
        assert_eq!(t.detect(None), Profile::Sta1Classic);
    }

    /// FO and A2 `Data` folders carry an `addon/` directory too.
    #[test]
    fn an_addon_folder_alone_is_not_armada_one() {
        let t = Tmp::new();
        t.file("Data/addon/ship.odf", "");
        assert_eq!(t.detect(None), Profile::Sta2Classic);
    }

    /// A plain game folder (stock A2 as GOG installs it) has no `Data`.
    #[test]
    fn detects_a_plain_a2_folder() {
        let t = Tmp::new();
        t.file("odf/a.odf", "");
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), t.0.clone()).unwrap();
        assert_eq!(Profile::detect(&vfs), Profile::Sta2Classic);
    }

    #[test]
    fn detects_fo4_chain() {
        let t = Tmp::new();
        t.file("Data/Mods/FleetOps/info.ini", "[Mod]\nName=Fleet Operations 4.0\n")
            .file("Data/Mods/Addon/info.ini", "[Mod]\nName=Some Addon\nParentMod=FleetOps\n");
        assert_eq!(t.detect(Some("Addon")), Profile::Fo4);
        // Same chain, by info.ini name only.
        let t = Tmp::new();
        t.file("Data/Mods/xyz/info.ini", "[Mod]\nName=Fleet Operations\n");
        assert_eq!(t.detect(Some("xyz")), Profile::Fo4);
    }

    #[test]
    fn detects_fo4_by_loader() {
        let t = Tmp::new();
        t.file("Data/odf/a.odf", "").file("mmm_loader.dll", "");
        assert_eq!(t.detect(None), Profile::Fo4);
    }

    #[test]
    fn explicit_profile_wins() {
        let t = Tmp::new();
        t.zfs("Data/assets.zfs").file("Data/Mods/M/info.ini", "[Mod]\nName=Fleet Ops\nProfile=STA2 Classic\n");
        assert_eq!(t.detect(Some("M")), Profile::Sta2Classic);
    }

    #[test]
    fn empty_vfs_is_native() {
        assert_eq!(Profile::detect(&Vfs::default()), Profile::Native);
    }

    /// Install-gated: `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`); optionally
    /// `STA64_TEST_EXPECT` names the profile it must detect. Not run here.
    #[test]
    fn detects_on_real_install() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let m = std::env::var("STA64_TEST_MOD").ok();
        let vfs = Vfs::mount_install(Path::new(&root), m.as_deref()).unwrap();
        let p = Profile::detect(&vfs);
        eprintln!("detected {} for {root} mod {m:?}", p.name());
        assert_ne!(p, Profile::Native, "an install is never native");
        if let Some(e) = std::env::var("STA64_TEST_EXPECT").ok().and_then(|e| Profile::from_name(&e)) {
            assert_eq!(p, e);
        }
    }
}
