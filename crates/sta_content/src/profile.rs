//! Profile detection, the per-profile defaults table and feature overrides
//! (CON-05). [`Profile`] and [`Features`] are the S5 seam types in
//! `features.rs`; this file gives them behaviour.
//!
//! # Detection ([`Profile::detect`], [`Profile::detect_with_source`])
//!
//! Layers are the mounted [`Vfs`] layers, top-most (child mod) first; the
//! last one is normally `Data`. The first step that matches wins (the order
//! amended by CON-43, `docs/FO_PROFILE_DETECTION.md`; D4):
//!
//! 1. **Explicit** (D4: manifest, then `info.ini`, then the heuristics
//!    below). First the `profile` key of any layer's `manifest.toml` (loose
//!    or in a root `.sta`), then a `Profile = <name>` key in the `[Mod]`
//!    section of any layer's `info.ini`; top-most layer first. Names: `native`/`sta64`,
//!    `fo4`/`fo`, `fo3`/`fo327`/`fleetops3`, `sta2`/`sta2classic`, `sta1`/`sta1classic` (see
//!    [`Profile::from_name`]).
//! 2. **Armada I layout.** Any layer holds a `.zfs` archive. (An `addon/`
//!    directory is not a signature: Armada II and FO ship one too, and a
//!    real FO Roots install detected as Armada I on it.) This keeps its
//!    place ahead of every step below it.
//! 3. **`AssetVersion`.** The top-most layer whose own `info.ini` has a valid
//!    `[mod] AssetVersion` decides: `>= 50000` is the Roots lineage (today's
//!    [`Profile::Fo4`]); a lower value (3.2.5 is 30205) does not select it and
//!    the steps below go on. A value that is not a whole number is ignored
//!    with a warning. The field is data the mod ships; it is not read from
//!    an executable.
//! 4. **Host base `Data/info.ini`.** For a mod whose own `info.ini` has no
//!    `AssetVersion` (a stated older value skips this step): the install's base `Data/info.ini` is read, for its
//!    `[Mod] Name` only (the matcher of step 6), even when D6 does not mount
//!    `Data` under a `standalone` mod. It is never mounted or merged.
//! 5. **`Data/packages.ini`.** `[FleetOps] Version=` of the host `Data`
//!    is recorded in [`Detection::fleetops_version`]. A major number of 3
//!    (3.2.7 on Fleet Operations 3.x and Dominion War, which have no
//!    `info.ini`) selects [`Profile::Fo3`] (CON-44, D5); the Roots install's
//!    own file says 4.0.0 and decides nothing (the steps above found it).
//! 6. **Fleet Operations heuristics.** Any layer whose `info.ini` `Name`
//!    contains `fleet op`/`fleetop`/`fo4`/`fo 4`, including `Data`; a non-Data
//!    layer's folder name with that signature; or the existing
//!    `mmm_loader.dll` (FO's mission loader) hints at a mod layer or above
//!    `Data`.
//! 7. **Stock Armada II.** Any other install: a `Data` layer, or a plain game
//!    folder with an `odf/` directory, `ODF.fpq` or `Armada2.exe`, possibly
//!    with mods that match nothing above.
//! 8. **Native.** No `Data` layer and nothing matched (also the fallback for
//!    unknown or custom content).
//!
//! (The amended contract counts the heuristics as one step, its step 5; they
//! are numbered here in the order they run. `[modules]` lists are not read at
//! all: STA64 builds those extensions in natively.)
//!
//! [`Detection`] reports which step and which file decided. When the detected
//! profile is the Roots lineage and a layer's `AssetVersion` is lower than
//! 50000, a [`Warning`](crate::Warning) says so; it is never a refusal
//! (the original engine's incompatibility warning is skippable too).
//!
//! The FO and A1 signatures are heuristics from the documented layouts.
//! Content-selection checks, including the install-gated test, do not prove
//! original-game runtime compatibility.
//!
//! # Precedence ([`resolve_features`])
//!
//! profile defaults < mod overrides (`[Features]` of `info.ini`, child mod
//! over parent) < per-ODF overrides (`Feature_<name>` keys). Each source is an
//! [`Overrides`]; later sources win per feature.
//!
//! Hookup: `ContentDb::load` calls `Profile::detect_with_source(vfs)` and
//! `Overrides::from_vfs(vfs)`; db-wide features are
//! `resolve_features(profile, &[&mod_ov])`, and a def's own are
//! `resolve_features(profile, &[&mod_ov, &Overrides::from_odf_keys(..)])`,
//! stored in `UnitDef::features` / `WeaponDef::features`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sta_vfs::Vfs;

use crate::Warning;
use crate::features::{Features, Profile};

/// Profile names as written in `info.ini` (`Profile=`) and `.sta` manifests.
const PROFILE_NAMES: [(&str, Profile); 11] = [
    ("native", Profile::Native),
    ("sta64", Profile::Native),
    ("fo4", Profile::Fo4),
    ("fo", Profile::Fo4),
    ("fo3", Profile::Fo3),
    ("fo327", Profile::Fo3),
    ("fleetops3", Profile::Fo3),
    ("sta2", Profile::Sta2Classic),
    ("sta2classic", Profile::Sta2Classic),
    ("sta1", Profile::Sta1Classic),
    ("sta1classic", Profile::Sta1Classic),
];

impl Profile {
    /// Every profile, for tables and tests.
    pub const ALL: [Profile; 5] = [Profile::Native, Profile::Fo4, Profile::Fo3, Profile::Sta2Classic, Profile::Sta1Classic];

    /// Canonical lower-case name.
    pub fn name(self) -> &'static str {
        match self {
            Profile::Native => "native",
            Profile::Fo4 => "fo4",
            Profile::Fo3 => "fo3",
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

    /// Total station build jobs, including the active head (SIM-20).
    /// Original A1 uses five; stock A2 and FO use ten. Native retains
    /// STA64's five-job policy. A producer ODF can override every default.
    pub fn build_queue_limit(self) -> u32 {
        match self {
            Profile::Native | Profile::Sta1Classic => 5,
            Profile::Fo4 | Profile::Fo3 | Profile::Sta2Classic => 10,
        }
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
                ..off
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
                weapon_shot_timing: true,
                start_fire_hold: true,
                // SIM-39: stock casualty code; the hook never replaces it
                // (read from the executable, static). Roots data zeroes the
                // shield and hull crew modifiers, so those laws do nothing.
                crew_loss_on_hit: true,
                // SIM-36: the Roots engine (also running STA2 Classic, CON-43).
                attack_buffer_roots: true,
                attack_opportunity_fire: true,
                target_score: true,
                idle_assist: true,
                assist_high_autonomy_only: true,
                ..off
            },
            // FO 3.2.7 (CON-44, D5). Each bit from the 3.2.7 and Dominion War
            // installs, with its evidence grade:
            // - Engine rules measured on stock A2 (`turrets`, `weapon_shot_timing`,
            //   `start_fire_hold`, `crew_loss_on_hit`, and SIM-36's
            //   `attack_combat_speed`, `builder_hold_fire`, `target_score`,
            //   `idle_assist`, `assist_on_ally_fire`) keep the stock value: the
            //   engine the 3.x launcher loads (`Data/Armada2.ldl`) is the stock
            //   executable with small differences (read from the executable,
            //   static). `FleetOpsHook.dll` patches nothing on the casualty path
            //   (read from the executable, static); no Roots rule (`pd_energy`,
            //   `pd_debt_timer`, the Roots attack and assist bits) is assumed.
            // - `point_defense_cycles` on: the 3.2.7 PD weapons are FO's own
            //   `ordnancedefenseweapon` class (read from data); their timing is
            //   not measured on 3.2.7.
            // - `extra_resources` on: ODFs of both installs charge latinum, metal
            //   and biomatter slots in thousands of units, stock in tens (read
            //   from data). `multi_yard_builds` on: `buildyard` class ODFs and
            //   `builditem*` availability keys, which stock lacks (read from data).
            // - `officer_capacity`, `construction_rigs` stay at the stock value
            //   (off): the keys the data offers (`MP_OFFICER_LIMIT`, the
            //   `constructionrig` class) are stock A2's too, so they do not tell
            //   3.2.7 apart; not measured.
            // - `weapon_damage_controls`: no key of the 3.2.7 data names it; not
            //   measured, off. `pay_on_queue`: its ODF key is absent, off.
            Profile::Fo3 => Features {
                turrets: true,
                point_defense_cycles: true,
                weapon_shot_timing: true,
                start_fire_hold: true,
                crew_loss_on_hit: true,
                extra_resources: true,
                multi_yard_builds: true,
                attack_combat_speed: true,
                builder_hold_fire: true,
                target_score: true,
                idle_assist: true,
                assist_on_ally_fire: true,
                ..off
            },
            // Stock A2: plain rules, turrets, point-defence cycles and the
            // original shot timing. `pd_energy` stays off until a HUD button
            // can toggle point defence (UI-31); then SIM-37's one-line flip:
            // `Sta2Classic` `pd_energy`, `Fo4` `pd_energy` + `pd_debt_timer`.
            Profile::Sta2Classic => Features {
                turrets: true,
                point_defense_cycles: true,
                weapon_shot_timing: true,
                start_fire_hold: true,
                crew_loss_on_hit: true,
                // SIM-36: stock A2's attack approach and construction ships.
                attack_combat_speed: true,
                builder_hold_fire: true,
                target_score: true,
                idle_assist: true,
                assist_on_ally_fire: true,
                ..off
            },
            // A1: its own class labels and health model, officers.
            Profile::Sta1Classic => Features { officer_capacity: true, a1_compat: true, ..off },
        }
    }

    /// Detect the profile of a mounted install; see the module docs for the
    /// rule list. [`detect_with_source`](Profile::detect_with_source) also
    /// says which step and file decided.
    pub fn detect(vfs: &Vfs) -> Profile {
        Profile::detect_with_source(vfs).profile
    }

    /// [`detect`](Profile::detect) with the deciding step and file, the
    /// `packages.ini` version and the warnings.
    pub fn detect_with_source(vfs: &Vfs) -> Detection {
        let layers = vfs.layers();
        let infos: Vec<Option<Ini>> = layers.iter().map(|l| read_info(&l.root)).collect();
        let mut warnings = Vec::new();

        // Each layer's own `AssetVersion`, read through sta_vfs's info.ini
        // reader. A malformed value is dropped with a warning, whichever step
        // ends up deciding.
        let mut versions: Vec<Option<(u32, PathBuf)>> = Vec::new();
        for l in layers {
            let found = find_ci(&l.root, "info.ini").and_then(|p| {
                let ini = sta_vfs::mods::Ini::parse(&String::from_utf8_lossy(&std::fs::read(&p).ok()?));
                Some((ini.asset_version()?, p))
            });
            versions.push(match found {
                Some((Ok(v), p)) => Some((v, p)),
                Some((Err(raw), p)) => {
                    warnings.push(Warning::new(p.display().to_string(), format!("AssetVersion {raw:?} is not a whole number; ignored")));
                    None
                }
                None => None,
            });
        }

        // The host install's base Data folder, mounted or not (D6 decides
        // what is mounted; this only reads two files from it).
        let host_data = layers.iter().find_map(|l| {
            if l.name.eq_ignore_ascii_case("data") {
                Some(l.root.clone())
            } else {
                let mods = l.root.parent().filter(|m| m.file_name().is_some_and(|n| n.eq_ignore_ascii_case("mods")))?;
                mods.parent().filter(|d| d.file_name().is_some_and(|n| n.eq_ignore_ascii_case("data"))).map(Path::to_path_buf)
            }
        });
        let fleetops_version = host_data.as_deref().and_then(read_fleetops_version);

        let (profile, step, file, detail) = Profile::decide(layers, &infos, &versions, host_data.as_deref());

        if profile == Profile::Fo4 {
            for (v, p) in versions.iter().flatten().filter(|(v, _)| *v < ROOTS_ASSET_VERSION) {
                warnings.push(Warning::new(
                    p.display().to_string(),
                    format!("AssetVersion {v} is older than the Roots lineage ({ROOTS_ASSET_VERSION}) this mod loads under; it may not work as designed"),
                ));
            }
        }
        Detection { profile, step, file, detail, fleetops_version, warnings }
    }

    fn decide(
        layers: &[sta_vfs::Layer],
        infos: &[Option<Ini>],
        versions: &[Option<(u32, PathBuf)>],
        host_data: Option<&Path>,
    ) -> (Profile, DetectStep, Option<PathBuf>, String) {
        let info_path = |l: &sta_vfs::Layer| find_ci(&l.root, "info.ini");

        // 1a. Explicit, manifest first (D4): `profile` of a `manifest.toml`.
        for l in layers {
            if let Some(Ok(m)) = sta_vfs::manifest::manifest_text(&l.root).map(|t| sta_vfs::manifest::Manifest::parse(&t))
                && let Some(p) = m.profile.as_deref().and_then(Profile::from_name)
            {
                return (p, DetectStep::Explicit, find_ci(&l.root, "manifest.toml"), format!("manifest profile = {}", p.name()));
            }
        }
        // 1b. Explicit, `info.ini`.
        for (l, ini) in layers.iter().zip(infos) {
            if let Some(p) = ini.as_ref().and_then(|i| i.get("mod", "profile")).and_then(Profile::from_name) {
                return (p, DetectStep::Explicit, info_path(l), format!("[mod] Profile = {}", p.name()));
            }
        }
        // 2. Armada I layout.
        for l in layers {
            if !l.zfs.is_empty() || has_zfs_file(&l.root) {
                return (Profile::Sta1Classic, DetectStep::Zfs, Some(l.root.clone()), "a .zfs archive".into());
            }
        }
        // 3. AssetVersion: the top-most layer that states one.
        if let Some((v, p)) = versions.iter().flatten().next()
            && *v >= ROOTS_ASSET_VERSION
        {
            return (Profile::Fo4, DetectStep::AssetVersion, Some(p.clone()), format!("AssetVersion = {v}"));
        }
        // 4. The host's base Data/info.ini, for its name only: a fallback for a
        // mod that states no AssetVersion. A stated older value (a mod designed
        // for FO 3.x) is not a Roots mod and skips this step.
        if versions.iter().all(Option::is_none)
            && let Some(data) = host_data
            && let Some(p) = find_ci(data, "info.ini")
            && let Some(ini) = read_info(data)
            && let Some(name) = ini.get("mod", "name").filter(|n| looks_like_fo(n))
        {
            return (Profile::Fo4, DetectStep::HostBaseInfo, Some(p), format!("host base [mod] Name = {name}"));
        }
        // 5. `Data/packages.ini` `[FleetOps] Version=3.x`: the FO 3.x engine
        // (CON-44). The Roots host says 4.x and decides nothing here.
        if let Some(data) = host_data
            && let Some(v) = read_fleetops_version(data).filter(|v| fleetops_major(v) == Some(3))
            && let Some(p) = find_ci(data, "packages.ini")
        {
            return (Profile::Fo3, DetectStep::PackagesIni, Some(p), format!("[FleetOps] Version = {v}"));
        }
        // 6. Fleet Operations heuristics, then 7. / 8.
        let mut has_data = false;
        for (l, ini) in layers.iter().zip(infos) {
            let name = ini.as_ref().and_then(|i| i.get("mod", "name")).unwrap_or("");
            if looks_like_fo(name) {
                return (Profile::Fo4, DetectStep::Heuristic, info_path(l), format!("[mod] Name = {name}"));
            }
            if l.name.eq_ignore_ascii_case("data") {
                has_data = true;
                if l.root.parent().is_some_and(|r| has_child(r, "mmm_loader.dll", false)) {
                    return (Profile::Fo4, DetectStep::Heuristic, l.root.parent().map(|r| r.join("mmm_loader.dll")), "mmm_loader.dll beside Data".into());
                }
                continue;
            }
            if has_child(&l.root, "odf", true) || has_child(&l.root, "odf.fpq", false) || has_child(&l.root, "armada2.exe", false) {
                has_data = true;
            }
            if looks_like_fo(&l.name) {
                return (Profile::Fo4, DetectStep::Heuristic, Some(l.root.clone()), format!("layer folder name {:?}", l.name));
            }
            if has_child(&l.root, "mmm_loader.dll", false) {
                return (Profile::Fo4, DetectStep::Heuristic, Some(l.root.join("mmm_loader.dll")), "mmm_loader.dll in the layer".into());
            }
        }
        if has_data {
            (Profile::Sta2Classic, DetectStep::Heuristic, None, "Armada II layout".into())
        } else {
            (Profile::Native, DetectStep::Heuristic, None, "nothing matched".into())
        }
    }
}

/// `AssetVersion` from which a mod belongs to the Roots lineage: Fleet
/// Operations 5.0 is 50000 (major x 10000 + minor x 100 + patch).
pub const ROOTS_ASSET_VERSION: u32 = 50_000;

/// Which detection step decided; see the module docs for the order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectStep {
    /// A `Profile` in a `manifest.toml` or `info.ini` (step 1).
    Explicit,
    /// An Armada I `.zfs` archive (step 2).
    Zfs,
    /// A layer's own `AssetVersion` (step 3).
    AssetVersion,
    /// The host install's base `Data/info.ini` name, not mounted (step 4).
    HostBaseInfo,
    /// `[FleetOps] Version=3.x` in the host `Data/packages.ini` (step 5).
    PackagesIni,
    /// The existing layout, name and folder heuristics, and the fallbacks.
    Heuristic,
}

/// What [`Profile::detect_with_source`] found and how.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub profile: Profile,
    pub step: DetectStep,
    /// The file (or archive/folder) that decided; `None` for a fallback.
    pub file: Option<PathBuf>,
    /// What was read there, e.g. `AssetVersion = 50000`.
    pub detail: String,
    /// `[FleetOps] Version=` of the host `Data/packages.ini` (3.2.7 on the
    /// FO 3.x and Dominion War installs); a 3.x value selects `Fo3`.
    pub fleetops_version: Option<String>,
    /// Load warnings: malformed `AssetVersion`, older-than-lineage mods.
    pub warnings: Vec<Warning>,
}

impl std::fmt::Display for Detection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?}: {}", self.profile.name(), self.step, self.detail)?;
        if let Some(p) = &self.file {
            write!(f, "; {}", p.display())?;
        }
        if let Some(v) = &self.fleetops_version {
            write!(f, "; packages.ini [FleetOps] Version={v}")?;
        }
        write!(f, ")")
    }
}

/// `[FleetOps] Version=` of `<data>/packages.ini`, if it reads and looks like
/// a version (starts with a digit).
fn read_fleetops_version(data: &Path) -> Option<String> {
    let ini = read_ini_named(data, "packages.ini")?;
    ini.get("fleetops", "version").filter(|v| v.starts_with(|c: char| c.is_ascii_digit())).map(str::to_owned)
}

/// The major number of a `[FleetOps] Version=` text (`3.2.7` gives 3).
fn fleetops_major(v: &str) -> Option<u32> {
    v.split('.').next()?.trim().parse().ok()
}

fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path())
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
            let line = line.trim().trim_start_matches('\u{feff}');
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
    read_ini_named(root, "info.ini")
}

fn read_ini_named(root: &Path, name: &str) -> Option<Ini> {
    Some(Ini::parse(&String::from_utf8_lossy(&std::fs::read(find_ci(root, name)?).ok()?)))
}

/// Switch names, in `Features` declaration order.
pub const FEATURE_NAMES: [&str; 35] = [
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
    "pd_energy",
    "pd_debt_timer",
    "weapon_shot_timing",
    "start_fire_hold",
    "crew_loss_on_hit",
    "attack_buffer_roots",
    "attack_combat_speed",
    "attack_opportunity_fire",
    "builder_hold_fire",
    "target_score",
    "idle_assist",
    "assist_high_autonomy_only",
    "assist_on_ally_fire",
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
            "pd_energy" => &mut self.pd_energy,
            "pd_debt_timer" => &mut self.pd_debt_timer,
            "weapon_shot_timing" => &mut self.weapon_shot_timing,
            "start_fire_hold" => &mut self.start_fire_hold,
            "crew_loss_on_hit" => &mut self.crew_loss_on_hit,
            "attack_buffer_roots" => &mut self.attack_buffer_roots,
            "attack_combat_speed" => &mut self.attack_combat_speed,
            "attack_opportunity_fire" => &mut self.attack_opportunity_fire,
            "builder_hold_fire" => &mut self.builder_hold_fire,
            "target_score" => &mut self.target_score,
            "idle_assist" => &mut self.idle_assist,
            "assist_high_autonomy_only" => &mut self.assist_high_autonomy_only,
            "assist_on_ally_fire" => &mut self.assist_on_ally_fire,
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
        // Left off: it would move every golden digest; Native has no original to match.
        assert!(!f.crew_loss_on_hit);
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
                "construction_rigs",
                "weapon_shot_timing",
                "start_fire_hold",
                "crew_loss_on_hit",
                "attack_buffer_roots",
                "attack_opportunity_fire",
                "target_score",
                "idle_assist",
                "assist_high_autonomy_only"
            ]
        );
        assert!(!Profile::Fo4.features().pay_on_queue);
    }

    #[test]
    fn defaults_table_fo3() {
        assert_eq!(
            on(&Profile::Fo3.features()),
            [
                "point_defense_cycles",
                "turrets",
                "extra_resources",
                "multi_yard_builds",
                "weapon_shot_timing",
                "start_fire_hold",
                "crew_loss_on_hit",
                "attack_combat_speed",
                "builder_hold_fire",
                "target_score",
                "idle_assist",
                "assist_on_ally_fire"
            ]
        );
        let f = Profile::Fo3.features();
        assert!(!f.weapon_damage_controls && !f.pd_energy && !f.pd_debt_timer && !f.pay_on_queue, "no Roots rule is assumed for 3.2.7");
        assert_eq!(Profile::from_name("FO 3.2.7"), None, "only the listed names parse");
        assert_eq!([Profile::from_name("fo3"), Profile::from_name("FO327"), Profile::from_name("fleetops-3")], [Some(Profile::Fo3); 3]);
    }

    #[test]
    fn defaults_table_sta2_classic() {
        assert_eq!(on(&Profile::Sta2Classic.features()), [
            "point_defense_cycles",
            "turrets",
            "weapon_shot_timing",
            "start_fire_hold",
            "crew_loss_on_hit",
            "attack_combat_speed",
            "builder_hold_fire",
            "target_score",
            "idle_assist",
            "assist_on_ally_fire"
        ]);
    }

    #[test]
    fn defaults_table_sta1_classic() {
        assert_eq!(on(&Profile::Sta1Classic.features()), ["officer_capacity", "a1_compat"]);
        assert!(!Profile::Sta1Classic.features().crew_loss_on_hit, "A1's executable was not analysed");
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
