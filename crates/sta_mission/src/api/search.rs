//! Unit searches over the latest mission view, shared by the `A1` and `A2`
//! DLL-named tables: live units inside a radius, filtered by owner, ODF and
//! observer, nearest first then by id.

use std::collections::BTreeSet;

use sta_core::{EntityId, PlayerId};
use sta_sim::view::UnitView;

use super::{Ctx, dist, pos_f64};

/// `name` without a trailing `.odf`.
pub(super) fn stem(name: &str) -> &str {
    match name.get(name.len().saturating_sub(4)..) {
        Some(suffix) if suffix.eq_ignore_ascii_case(".odf") => &name[..name.len() - 4],
        _ => name,
    }
}

pub(super) struct Query<'a> {
    pub center: (f64, f64, f64),
    pub radius: f64,
    pub owner: Option<PlayerId>,
    /// `None` (or empty) matches every unit.
    pub odf: Option<&'a str>,
    pub exclude: EntityId,
    /// Only units this player can see.
    pub seen_by: Option<PlayerId>,
    /// A building still under construction matches no named ODF (see [`unfinished_named`]).
    pub unfinished_named: bool,
}

/// An omitted or empty ODF filter matches every unit.
pub(super) fn odf_ok(c: &Ctx, unit: &UnitView, filter: Option<&str>) -> bool {
    match filter {
        Some(s) if !s.is_empty() => stem(&c.odf_name(unit)).eq_ignore_ascii_case(stem(s)),
        _ => true,
    }
}

/// An unfinished building is not an object of its ODF yet: a search for a named ODF does not
/// count it (stock A2: stated by a player, and read from the executable, static); a search with
/// no ODF still does. Armada I, Fleet Operations and Roots: not looked at.
pub(super) fn unfinished_named(unit: &UnitView, filter: Option<&str>) -> bool {
    unit.progress.is_some_and(|p| p < sta_core::Fixed::ONE) && filter.is_some_and(|s| !s.is_empty())
}

/// Live units inside the query, nearest first, then by id. A seen-by query reads
/// [`Ctx::visible_ids`]: another team's unit is not counted in the tick after it first
/// appears (STA64's own one-tick rule, not measured in the original).
pub(super) fn near<'c>(c: &'c Ctx, q: &Query) -> Vec<&'c UnitView> {
    let visible: Option<BTreeSet<EntityId>> =
        q.seen_by.map(|p| c.visible_ids(p));
    let mut found: Vec<(f64, &UnitView)> = c
        .view
        .units
        .iter()
        .filter(|u| u.id != q.exclude && c.alive(u.id).is_some())
        .filter(|u| q.owner.is_none_or(|o| u.owner == o))
        .filter(|u| visible.as_ref().is_none_or(|v| v.contains(&u.id)))
        .filter(|u| odf_ok(c, u, q.odf))
        .filter(|u| !(q.unfinished_named && unfinished_named(u, q.odf)))
        .map(|u| (dist(q.center, pos_f64(u.pos)), u))
        .filter(|(d, _)| *d <= q.radius)
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
    found.into_iter().map(|(_, u)| u).collect()
}
