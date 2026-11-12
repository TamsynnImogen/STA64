//! Mod discovery (CON-13): every mod under `Data/Mods`, its `info.ini`
//! metadata, and its parent chain.
//!
//! A mod is a folder under `Data/Mods` with an `info.ini` (any case), or
//! whose root `.sta` archive carries one (`docs/STA_ARCHIVE.md`). Its
//! `[mod]` section gives the metadata the installed mods use:
//! `Name`, `Description`, `Author` (comma-separated), `Version`,
//! `ParentMod`, `standalone`, `ignoreParentMaps`, `AssetVersion`,
//! `Screenshot`, `Icon`, `SettingsDirectory`, `DefaultGameSpeed`, `Website`,
//! `Email`. Fleet Ops Roots also has a `[modules]` section (`activeN` /
//! `requiredN`: hook modules); it is not read, because STA64 builds those
//! extensions in natively (Tammy, 2026-11-12), and a mod that has one loads
//! without a warning. Every section stays readable through [`ModInfo::ini`].
//!
//! The chain follows `ParentMod` as [`crate::mod_chain`] does (child first,
//! names case-insensitive); here it is computed over the discovered list,
//! with a missing parent or a loop reported instead of failing the listing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::VfsError;
use crate::sta::StaArchive;

/// An `.ini` file: sections of `key = value`, names compared case-insensitively.
/// `;`, `#` and `//` start comment lines; values lose one pair of surrounding
/// quotes. The first assignment of a key in a section counts, as Windows'
/// `GetPrivateProfileString` reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ini {
    /// Lower-case section name -> entries in file order (key as written).
    sections: BTreeMap<String, Vec<(String, String)>>,
}

impl Ini {
    pub fn parse(text: &str) -> Ini {
        let mut ini = Ini::default();
        let mut section = String::new();
        for line in text.lines() {
            let l = line.trim().trim_start_matches('\u{feff}');
            if l.is_empty() || l.starts_with(';') || l.starts_with('#') || l.starts_with("//") {
                continue;
            }
            if let Some(name) = l.strip_prefix('[').and_then(|r| r.split_once(']')).map(|(n, _)| n) {
                section = name.trim().to_ascii_lowercase();
                ini.sections.entry(section.clone()).or_default();
                continue;
            }
            if let Some((k, v)) = l.split_once('=') {
                let k = k.trim();
                if k.is_empty() {
                    continue;
                }
                let v = v.trim();
                let v = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v);
                ini.sections.entry(section.clone()).or_default().push((k.to_owned(), v.to_owned()));
            }
        }
        ini
    }

    /// The first value of `key` in `section`.
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections.get(&section.to_ascii_lowercase())?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    /// A section's entries in file order.
    pub fn section(&self, section: &str) -> &[(String, String)] {
        self.sections.get(&section.to_ascii_lowercase()).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Lower-case section names.
    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.sections.keys().map(String::as_str)
    }

    pub(crate) fn flag(&self, key: &str) -> bool {
        matches!(self.get("mod", key).map(str::trim), Some(v) if v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
    }

    /// `AssetVersion` of the `[mod]` section: the Fleet Operations version the
    /// mod was designed for (major x 10000 + minor x 100 + patch, so 3.2.5 is
    /// 30205). `None` when the key is absent or empty; `Some(Err(raw))` with
    /// the text as written when it is not a whole number in `0..=u32::MAX`
    /// (negative, fractional, huge, words). Whitespace, one pair of single or
    /// double quotes and a trailing `;` / `#` comment are tolerated.
    pub fn asset_version(&self) -> Option<Result<u32, String>> {
        let raw = self.get("mod", "AssetVersion")?;
        let v = raw.split([';', '#']).next().unwrap_or("").trim().trim_matches(['"', '\'']).trim();
        if v.is_empty() {
            return None;
        }
        Some(v.parse::<u32>().map_err(|_| raw.trim().to_owned()))
    }
}

/// One mod and what its `info.ini` says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModInfo {
    /// Folder name under `Data/Mods` (what `ParentMod` and `--mod` name).
    pub folder: String,
    pub path: PathBuf,
    /// `Name`, else the folder name.
    pub name: String,
    pub description: String,
    pub authors: Vec<String>,
    pub version: Option<String>,
    pub website: Option<String>,
    pub email: Option<String>,
    /// File names, relative to the mod folder.
    pub screenshot: Option<String>,
    pub icon: Option<String>,
    pub parent: Option<String>,
    pub standalone: bool,
    pub ignore_parent_maps: bool,
    pub asset_version: Option<u32>,
    pub default_game_speed: Option<i32>,
    pub settings_directory: Option<String>,
    /// Archives at the mod's root (`.sta`, `.fpq`, `.zfs`), sorted.
    pub archives: Vec<String>,
    /// Where `info.ini` came from: `None` for the folder, else the `.sta`.
    pub info_from_archive: Option<String>,
    pub ini: Ini,
}

impl ModInfo {
    fn from_ini(folder: String, path: PathBuf, ini: Ini, archives: Vec<String>, info_from_archive: Option<String>) -> ModInfo {
        let s = |k: &str| ini.get("mod", k).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
        ModInfo {
            name: s("Name").unwrap_or_else(|| folder.clone()),
            description: s("Description").unwrap_or_default(),
            authors: s("Author").map(|a| a.split(',').map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect()).unwrap_or_default(),
            version: s("Version"),
            website: s("Website"),
            email: s("Email"),
            screenshot: s("Screenshot"),
            icon: s("Icon"),
            parent: s("ParentMod"),
            standalone: ini.flag("standalone"),
            ignore_parent_maps: ini.flag("ignoreParentMaps"),
            asset_version: ini.asset_version().and_then(Result::ok),
            default_game_speed: s("DefaultGameSpeed").and_then(|v| v.parse().ok()),
            settings_directory: s("SettingsDirectory"),
            folder,
            path,
            archives,
            info_from_archive,
            ini,
        }
    }
}

/// Why a chain could not be followed to its root.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error("mod {0:?} not found")]
    NotFound(String),
    #[error("{child:?} names parent {parent:?}, which is not installed")]
    MissingParent { child: String, parent: String },
    #[error("ParentMod loop: {0}")]
    Cycle(String),
}

/// Every mod in a `Data` folder, sorted by folder name (case-insensitive),
/// and a note for each folder skipped (no `info.ini`, unreadable archive).
pub fn list_mods(data: &Path) -> Result<(Vec<ModInfo>, Vec<String>), VfsError> {
    let Some(mods_dir) = child_ci(data, "Mods") else { return Ok((Vec::new(), Vec::new())) };
    let rd = std::fs::read_dir(&mods_dir).map_err(|source| VfsError::Io { path: mods_dir.clone(), source })?;
    let mut out = Vec::new();
    let mut notes = Vec::new();
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let path = e.path();
        let folder = e.file_name().to_string_lossy().into_owned();
        let mut archives: Vec<String> = std::fs::read_dir(&path)
            .map(|rd| {
                rd.flatten()
                    .filter(|f| f.file_type().is_ok_and(|t| t.is_file()))
                    .map(|f| f.file_name().to_string_lossy().into_owned())
                    .filter(|n| {
                        let l = n.to_ascii_lowercase();
                        l.ends_with(".sta") || l.ends_with(".fpq") || l.ends_with(".zfs")
                    })
                    .collect()
            })
            .unwrap_or_default();
        archives.sort_by_key(|n| n.to_ascii_lowercase());
        if let Some(ini_path) = child_ci(&path, "info.ini") {
            match std::fs::read(&ini_path) {
                Ok(b) => out.push(ModInfo::from_ini(folder, path, Ini::parse(&String::from_utf8_lossy(&b)), archives, None)),
                Err(err) => notes.push(format!("{folder}: info.ini unreadable: {err}")),
            }
            continue;
        }
        // A packed mod: its info.ini inside a root .sta.
        let mut found = None;
        for a in archives.iter().filter(|a| a.to_ascii_lowercase().ends_with(".sta")) {
            match StaArchive::open(&path.join(a)) {
                Ok(sta) => {
                    if let Some(i) = sta.find("info.ini") {
                        match sta.read(i) {
                            Ok(b) => {
                                found = Some((a.clone(), Ini::parse(&String::from_utf8_lossy(&b))));
                                break;
                            }
                            Err(err) => notes.push(format!("{folder}: {a}: info.ini: {err}")),
                        }
                    }
                }
                Err(err) => notes.push(format!("{folder}: {a}: {err}")),
            }
        }
        match found {
            Some((a, ini)) => out.push(ModInfo::from_ini(folder, path, ini, archives, Some(a))),
            None => notes.push(format!("{folder}: no info.ini, not a mod")),
        }
    }
    out.sort_by_key(|m| m.folder.to_ascii_lowercase());
    notes.sort();
    Ok((out, notes))
}

/// The mod named `name` (folder, case-insensitive).
pub fn find_mod<'a>(mods: &'a [ModInfo], name: &str) -> Option<&'a ModInfo> {
    mods.iter().find(|m| m.folder.eq_ignore_ascii_case(name))
}

/// `name` and its `ParentMod` ancestors, child first, as [`crate::mod_chain`]
/// mounts them.
pub fn chain<'a>(mods: &'a [ModInfo], name: &str) -> Result<Vec<&'a ModInfo>, ChainError> {
    let mut out: Vec<&ModInfo> = vec![find_mod(mods, name).ok_or_else(|| ChainError::NotFound(name.to_owned()))?];
    while let Some(parent) = out.last().unwrap().parent.clone() {
        let Some(p) = find_mod(mods, &parent) else {
            return Err(ChainError::MissingParent { child: out.last().unwrap().folder.clone(), parent });
        };
        if out.iter().any(|m| m.folder.eq_ignore_ascii_case(&p.folder)) {
            let mut names: Vec<&str> = out.iter().map(|m| m.folder.as_str()).collect();
            names.push(&p.folder);
            return Err(ChainError::Cycle(names.join(" -> ")));
        }
        out.push(p);
    }
    Ok(out)
}

/// Mods whose `ParentMod` is `name`, in listing order.
pub fn children<'a>(mods: &'a [ModInfo], name: &str) -> Vec<&'a ModInfo> {
    mods.iter().filter(|m| m.parent.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(name))).collect()
}

fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    std::fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp(name: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("sta_vfs_mods_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Data/Mods")).unwrap();
        Tmp(d)
    }

    fn write(p: PathBuf, s: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    #[test]
    fn ini_parsing() {
        let ini = Ini::parse(
            "\u{feff}[Mod]\r\nName = Fleet Operations\r\n; comment\r\n//ParentMod=Old\r\nParentMod=\"STA2 Classic Mod\"\r\nName=Second\r\n\
             [modules]\r\nactive0 = \"A\"\r\n",
        );
        assert_eq!(ini.get("mod", "name"), Some("Fleet Operations"), "first assignment counts");
        assert_eq!(ini.get("MOD", "parentmod"), Some("STA2 Classic Mod"));
        assert_eq!(ini.sections().collect::<Vec<_>>(), ["mod", "modules"]);
    }

    #[test]
    fn asset_version_reads_tolerantly_and_reports_malformed_values() {
        let v = |body: &str| Ini::parse(&format!("\u{feff}[Mod]\r\n{body}\r\n")).asset_version();
        assert_eq!(v("AssetVersion=50000"), Some(Ok(50000)));
        assert_eq!(v("assetversion = \" 30205 \" ; Fleet Ops 3.2.5"), Some(Ok(30205)));
        assert_eq!(v("AssetVersion='49999'"), Some(Ok(49999)));
        assert_eq!(v("AssetVersion=50000\r\nAssetVersion=1"), Some(Ok(50000)), "first assignment counts");
        assert_eq!(v("AssetVersion="), None);
        assert_eq!(v("Name=x"), None);
        for bad in ["-5", "5.0", "4294967296", "abc", "0x1234", "50 000"] {
            assert_eq!(v(&format!("AssetVersion={bad}")), Some(Err(bad.to_owned())), "{bad}");
        }
        assert_eq!(Ini::parse("[other]\nAssetVersion=50000").asset_version(), None, "only the [mod] section");
        let modules = Ini::parse("[mod]\nName=x\n[modules]\nactive0=A2FOFeaturePack\nrequired0=Z\n");
        assert_eq!(modules.get("mod", "name"), Some("x"), "a [modules] section is just another section");
    }

    #[test]
    fn lists_mods_with_metadata_and_chains() {
        let t = tmp("list");
        let m = t.0.join("Data/Mods");
        write(m.join("Fleet Ops 4.0/info.ini"), b"[mod]\nName=Fleet Operations\nAuthor=DOCa Cola, Optec\nVersion=4.0Z\nAssetVersion=50000\nstandalone=1\nignoreParentMaps=1\nScreenshot=splash.png\nDefaultGameSpeed=7\n[modules]\nactive0=\"A2FOFeaturePack\"\n");
        write(m.join("Noxter/INFO.INI"), b"[mod]\nName=Noxter\nParentMod=Fleet Ops 4.0\nstandalone=0\n");
        write(m.join("Sub/info.ini"), b"[mod]\nParentMod=noxter\n");
        write(m.join("Broken/info.ini"), b"[mod]\nParentMod=Gone\n");
        write(m.join("Loop A/info.ini"), b"[mod]\nParentMod=Loop B\n");
        write(m.join("Loop B/info.ini"), b"[mod]\nParentMod=Loop A\n");
        std::fs::create_dir_all(m.join("Not a mod")).unwrap();
        write(m.join("Noxter/ODF.fpq"), b"x");
        let (mods, notes) = list_mods(&t.0.join("Data")).unwrap();
        let names: Vec<&str> = mods.iter().map(|m| m.folder.as_str()).collect();
        assert_eq!(names, ["Broken", "Fleet Ops 4.0", "Loop A", "Loop B", "Noxter", "Sub"]);
        assert_eq!(notes, ["Not a mod: no info.ini, not a mod"]);

        let fo = find_mod(&mods, "fleet ops 4.0").unwrap();
        assert_eq!(fo.name, "Fleet Operations");
        assert_eq!(fo.authors, ["DOCa Cola", "Optec"]);
        assert_eq!((fo.version.as_deref(), fo.asset_version, fo.default_game_speed), (Some("4.0Z"), Some(50000), Some(7)));
        assert!(fo.standalone && fo.ignore_parent_maps && fo.parent.is_none());
        assert_eq!(find_mod(&mods, "Sub").unwrap().name, "Sub", "no Name: the folder");
        assert_eq!(find_mod(&mods, "Noxter").unwrap().archives, ["ODF.fpq"]);

        let c: Vec<&str> = chain(&mods, "sub").unwrap().iter().map(|m| m.folder.as_str()).collect();
        assert_eq!(c, ["Sub", "Noxter", "Fleet Ops 4.0"]);
        assert_eq!(chain(&mods, "Broken").unwrap_err(), ChainError::MissingParent { child: "Broken".into(), parent: "Gone".into() });
        assert!(matches!(chain(&mods, "Loop A"), Err(ChainError::Cycle(_))));
        assert_eq!(chain(&mods, "nope").unwrap_err(), ChainError::NotFound("nope".into()));
        let kids: Vec<&str> = children(&mods, "FLEET OPS 4.0").iter().map(|m| m.folder.as_str()).collect();
        assert_eq!(kids, ["Noxter"]);
    }

    #[test]
    fn packed_mod_reads_info_ini_from_its_sta() {
        let t = tmp("sta");
        let sta = crate::sta_write::pack(
            vec![("info.ini".into(), crate::sta_write::Content::Data(b"[mod]\nName=Packed\nParentMod=Fleet Ops 4.0\n".to_vec()))],
            crate::sta_write::Compression::Auto,
        )
        .unwrap();
        write(t.0.join("Data/Mods/Packed/packed.sta"), &sta);
        let (mods, notes) = list_mods(&t.0.join("Data")).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let p = find_mod(&mods, "packed").unwrap();
        assert_eq!((p.name.as_str(), p.parent.as_deref(), p.info_from_archive.as_deref()), ("Packed", Some("Fleet Ops 4.0"), Some("packed.sta")));
    }

    #[test]
    fn no_mods_folder_is_empty() {
        let d = std::env::temp_dir().join(format!("sta_vfs_mods_none_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(list_mods(&d).unwrap(), (Vec::new(), Vec::new()));
        let _ = std::fs::remove_dir_all(d);
    }

    /// With STA64_TEST_INSTALL: list the install's mods, and check every
    /// chain agrees with `mod_chain`, which mounting uses.
    #[test]
    fn real_install_if_available() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let data = child_ci(Path::new(&root), "Data").expect("Data folder");
        let (mods, notes) = list_mods(&data).unwrap();
        for n in &notes {
            eprintln!("  note: {n}");
        }
        for m in &mods {
            let ours = chain(&mods, &m.folder).map(|c| c.iter().map(|x| x.folder.to_ascii_lowercase()).collect::<Vec<_>>());
            let theirs = crate::mod_chain(&data, &m.folder).map(|c| c.iter().map(|(n, _)| n.to_ascii_lowercase()).collect::<Vec<_>>());
            match (&ours, &theirs) {
                (Ok(a), Ok(b)) => assert_eq!(a, b, "{}", m.folder),
                (Err(_), Err(_)) => {}
                _ => panic!("{}: list says {ours:?}, mod_chain says {theirs:?}", m.folder),
            }
            eprintln!(
                "{:<36} {:<34} v{:<8} parent {:<20} {}{}",
                m.folder,
                m.name,
                m.version.as_deref().unwrap_or("-"),
                m.parent.as_deref().unwrap_or("-"),
                if m.standalone { "standalone " } else { "" },
                if m.archives.is_empty() { String::new() } else { format!("[{}]", m.archives.join(", ")) }
            );
        }
        eprintln!("{} mods, {} notes", mods.len(), notes.len());
        assert!(!mods.is_empty());
    }
}
