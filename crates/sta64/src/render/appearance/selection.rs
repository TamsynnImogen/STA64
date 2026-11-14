//! RND-35: select presentation from the existing immutable race attributes.
//!
//! A2FO rendering reads the craft's current race (source, static). STA64's
//! explicit precedence is per-unit mission override, current owner's mission
//! override, then its setup faction. This changes no ownership or gameplay.

use sta_sim::{UnitView, WorldView};

/// Return the authored identifier before catalogue lookup. An explicit override
/// that the mounted catalogue cannot resolve stays unknown; it must not fall
/// through and draw another faction. The view tables are sorted by their keys.
pub(super) fn faction_identifier<'a>(view: &'a WorldView, unit: &UnitView) -> Option<&'a str> {
    if let Ok(i) = view.attrs.races.binary_search_by_key(&unit.id, |r| r.0) {
        return Some(view.attrs.races[i].1.as_str());
    }
    if let Ok(i) = view
        .attrs
        .player_races
        .binary_search_by_key(&unit.owner, |r| r.0)
    {
        return Some(view.attrs.player_races[i].1.as_str());
    }
    view.appearance.faction(unit.owner)
}
