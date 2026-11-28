//! Which install an install-gated test runs on (issue #1352).
//!
//! An install-gated test reads a game the repository cannot ship. Each test names the *kind* of install it needs
//! ([`Kind`]); the kind has one environment variable, and a test whose variable is unset, or whose install is not of
//! that kind, skips with one line, `SKIP: set <NAME> (needs <kind>)`, instead of failing or returning silently.
//!
//! Transition: the old general variables `STA64_TEST_INSTALL` and `STA64_TEST_MOD` are still accepted, for a kind
//! that has a content profile, when and only when the install they name is of that kind (read-only profile
//! detection through the same mount the game uses). They go once every crate's tests are migrated.
//!
//! Nothing here writes to disk or starts a game.

use std::path::{Path, PathBuf};

use sta_content::Profile;
use sta_vfs::Vfs;

/// The general variable of the transition and its mod companion.
const GENERAL_INSTALL: &str = "STA64_TEST_INSTALL";
const GENERAL_MOD: &str = "STA64_TEST_MOD";

/// The kind of install a test needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Armada I, the first game.
    FirstGame,
    /// Stock Armada II, the second game, without mods.
    StockSecond,
    /// The mod family (the Roots lineage), with or without a named mod.
    ModFamily,
    /// Fleet Operations 3.x.
    Fo327,
    /// The Dominion War mod of the second game (Fleet Operations 3.x engine).
    DominionWar,
    /// The Evolution mod of the second game.
    Evolution,
    /// A test that reads any game's data and asserts only structure: the first of the specific variables that is set,
    /// in the order of [`Kind::ANY`].
    Any,
}

impl Kind {
    /// The kinds [`Kind::Any`] tries, in this order.
    pub const ANY: [Kind; 6] = [Kind::ModFamily, Kind::StockSecond, Kind::FirstGame, Kind::Fo327, Kind::DominionWar, Kind::Evolution];

    /// The variable of the kind's install root.
    pub fn variable(self) -> &'static str {
        match self {
            Kind::FirstGame => "STA64_TEST_A1_INSTALL",
            Kind::StockSecond => "STA64_TEST_A2_STOCK_INSTALL",
            Kind::ModFamily | Kind::Any => "STA64_TEST_ROOTS_INSTALL",
            Kind::Fo327 => "STA64_TEST_FO327_INSTALL",
            Kind::DominionWar => "STA64_TEST_DW_INSTALL",
            Kind::Evolution => "STA64_TEST_EVOLUTION_INSTALL",
        }
    }

    /// The variable of the mod to activate, for the kinds that take one.
    pub fn mod_variable(self) -> Option<&'static str> {
        match self {
            Kind::ModFamily | Kind::Any => Some("STA64_TEST_ROOTS_MOD"),
            _ => None,
        }
    }

    /// What the kind is, for the skip line.
    pub fn needs(self) -> &'static str {
        match self {
            Kind::FirstGame => "an Armada I install",
            Kind::StockSecond => "a stock Armada II install",
            Kind::ModFamily => "a mod-family (Roots) install",
            Kind::Fo327 => "a Fleet Operations 3.2.7 install",
            Kind::DominionWar => "a Dominion War install",
            Kind::Evolution => "an Evolution install",
            Kind::Any => "an install of any game",
        }
    }

    /// The content profile of an install of this kind, when it has one the general variable can be checked against.
    fn profile(self) -> Option<Profile> {
        match self {
            Kind::FirstGame => Some(Profile::Sta1Classic),
            Kind::StockSecond => Some(Profile::Sta2Classic),
            Kind::ModFamily => Some(Profile::Fo4),
            Kind::Fo327 | Kind::DominionWar => Some(Profile::Fo3),
            Kind::Evolution | Kind::Any => None,
        }
    }
}

/// An install a test runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub root: PathBuf,
    /// The mod to activate, when the kind's mod variable (or, in the transition, the general one) names one.
    pub active_mod: Option<String>,
}

impl Found {
    /// The root as text, for the tests that pass it on as a string.
    pub fn root_string(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    /// The mod as text, empty for none (the shape the older tests used).
    pub fn mod_string(&self) -> String {
        self.active_mod.clone().unwrap_or_default()
    }

    /// The mounted virtual file system of the install with its mod, read-only; `None` when it does not mount.
    pub fn mount(&self) -> Option<Vfs> {
        Vfs::mount_install(&self.root, self.active_mod.as_deref()).ok()
    }
}

/// The text of a variable: set and not empty.
fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The profile of the install at `root` with `active_mod`, read-only; `None` when it does not mount.
fn detect(root: &Path, active_mod: Option<&str>) -> Option<Profile> {
    Vfs::mount_install(root, active_mod).ok().map(|v| Profile::detect(&v))
}

/// The install of `kind`, if the machine has one: the kind's variable, else (transition) the general one when its
/// install is of this kind. Prints nothing; [`install_with_mod`] and [`install`] print the skip line.
pub fn find(kind: Kind) -> Option<Found> {
    if kind == Kind::Any {
        return Kind::ANY.iter().find_map(|k| find(*k)).or_else(|| general(None));
    }
    if let Some(root) = var(kind.variable()) {
        let active_mod = kind.mod_variable().and_then(var);
        return Some(Found { root: PathBuf::from(root), active_mod });
    }
    general(Some(kind))
}

/// The general variables of the transition, accepted for `kind` only when the install they name is of that kind
/// (`None`: any kind, for [`Kind::Any`]).
fn general(kind: Option<Kind>) -> Option<Found> {
    let root = PathBuf::from(var(GENERAL_INSTALL)?);
    let active_mod = var(GENERAL_MOD);
    match kind {
        None => Some(Found { root, active_mod }),
        Some(kind) => match kind.profile() {
            Some(wanted) if detect(&root, active_mod.as_deref()) == Some(wanted) => Some(Found { root, active_mod }),
            _ => None,
        },
    }
}

/// The root of the install of `kind`, or `None` after the one-line skip message.
pub fn install(kind: Kind) -> Option<PathBuf> {
    install_with_mod(kind).map(|f| f.root)
}

/// The install of `kind` with its mod, or `None` after the one-line skip message.
pub fn install_with_mod(kind: Kind) -> Option<Found> {
    let found = find(kind);
    if found.is_none() {
        skip(&format!("set {} (needs {})", kind.variable(), kind.needs()));
    }
    found
}

/// The one-line skip message of a test, on the standard error stream.
pub fn skip(why: &str) {
    eprintln!("SKIP: {why}");
}

/// A skirmish map `vfs` has, by its file stem: `STA64_TEST_MAP` when the install has it, else one of the usual
/// skirmish maps when present, else the first map file in name order; `None` (after the skip line) when the install
/// has no map file at all.
pub fn a_map(vfs: &Vfs) -> Option<String> {
    let stems: Vec<String> = vfs
        .list("bzn")
        .iter()
        .filter_map(|p| p.rsplit('/').next())
        .filter_map(|f| f.rsplit_once('.').filter(|(_, ext)| ext.eq_ignore_ascii_case("bzn")).map(|(stem, _)| stem.to_owned()))
        .collect();
    let has = |name: &str| stems.iter().find(|s| s.eq_ignore_ascii_case(name)).cloned();
    let chosen = var("STA64_TEST_MAP")
        .and_then(|m| has(&m))
        .or_else(|| ["blueamber", "adapt", "boobytrap", "4alliance", "fed1"].iter().find_map(|n| has(n)))
        .or_else(|| stems.first().cloned());
    if chosen.is_none() {
        skip("the install has no map file (needs a map under bzn/)");
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_has_its_own_variable_and_only_the_mod_family_takes_a_mod() {
        let all = [Kind::FirstGame, Kind::StockSecond, Kind::ModFamily, Kind::Fo327, Kind::DominionWar, Kind::Evolution];
        let names: std::collections::BTreeSet<_> = all.iter().map(|k| k.variable()).collect();
        assert_eq!(names.len(), all.len());
        assert!(all.iter().all(|k| k.mod_variable().is_some() == (*k == Kind::ModFamily)));
        assert!(all.iter().all(|k| k.needs().contains("install")));
    }

    #[test]
    fn a_kind_with_no_variable_finds_nothing_and_a_set_variable_is_taken_as_it_is() {
        // Evolution has no content profile, so the general variable never stands in for it.
        if var(Kind::Evolution.variable()).is_none() {
            assert_eq!(find(Kind::Evolution), None);
            assert_eq!(install(Kind::Evolution), None);
        }
    }

    #[test]
    fn a_map_stem_comes_from_the_listing_and_an_install_without_maps_has_none() {
        let dir = std::env::temp_dir().join(format!("sta_testkit_{}", std::process::id()));
        let data = dir.join("Data");
        std::fs::create_dir_all(data.join("bzn")).unwrap();
        std::fs::write(data.join("bzn").join("Zeta.bzn"), b"x").unwrap();
        std::fs::write(data.join("bzn").join("alpha.bzn"), b"x").unwrap();
        let vfs = Vfs::mount_install(&dir, None).unwrap();
        let first = a_map(&vfs).expect("a map");
        assert!(["Zeta", "alpha"].contains(&first.as_str()), "{first}");
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::create_dir_all(data.join("bzn")).unwrap();
        let empty = Vfs::mount_install(&dir, None).unwrap();
        assert_eq!(a_map(&empty), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
