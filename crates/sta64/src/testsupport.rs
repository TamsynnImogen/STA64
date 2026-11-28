//! Which install an install-gated test of the client runs on: `sta_testkit`, plus the transition rule for the old
//! general variables (`STA64_TEST_INSTALL`, `STA64_TEST_MOD`), accepted for a kind that has a content profile when
//! and only when the install they name is of that kind (read-only profile detection through the mount the game uses).
//! Removed when every crate's tests have moved (issue #1352).

pub(crate) use sta_testkit::{Found, Kind, a_map, skip};

fn profile_of(kind: Kind) -> Option<sta_content::Profile> {
    use sta_content::Profile;
    match kind {
        Kind::FirstGame => Some(Profile::Sta1Classic),
        Kind::StockSecond => Some(Profile::Sta2Classic),
        Kind::ModFamily => Some(Profile::Fo4),
        Kind::Fo327 | Kind::DominionWar => Some(Profile::Fo3),
        Kind::Evolution | Kind::Any => None,
    }
}

/// The install of `kind`: its variable, else the general one when its install is of this kind.
pub(crate) fn find(kind: Kind) -> Option<Found> {
    if let Some(found) = sta_testkit::find(kind) {
        return Some(found);
    }
    let root = std::path::PathBuf::from(sta_testkit::var("STA64_TEST_INSTALL")?);
    let active_mod = sta_testkit::var("STA64_TEST_MOD");
    let found = Found { root, active_mod };
    match profile_of(kind) {
        _ if kind == Kind::Any => Some(found),
        Some(wanted) => found.mount().is_some_and(|vfs| sta_content::Profile::detect(&vfs) == wanted).then_some(found),
        None => None,
    }
}

/// The install of `kind` with its mod, or `None` after the one-line skip message.
pub(crate) fn install_with_mod(kind: Kind) -> Option<Found> {
    let found = find(kind);
    if found.is_none() {
        skip(&format!("set {} (needs {})", kind.variable(), kind.needs()));
    }
    found
}

/// The root of the install of `kind`, or `None` after the skip message.
pub(crate) fn install(kind: Kind) -> Option<std::path::PathBuf> {
    install_with_mod(kind).map(|f| f.root)
}
