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
}

/// An omitted or empty ODF filter matches every unit.
pub(super) fn odf_ok(c: &Ctx, unit: &UnitView, filter: Option<&str>) -> bool {
    match filter {
        Some(s) if !s.is_empty() => stem(&c.odf_name(unit)).eq_ignore_ascii_case(stem(s)),
        _ => true,
    }
}

/// Live units inside the query, nearest first, then by id.
pub(super) fn near<'c>(c: &'c Ctx, q: &Query) -> Vec<&'c UnitView> {
    let visible: Option<BTreeSet<EntityId>> =
        q.seen_by.map(|p| c.view.visible_to(p).iter().map(|u| u.id).collect());
    let mut found: Vec<(f64, &UnitView)> = c
        .view
        .units
        .iter()
        .filter(|u| u.id != q.exclude && c.alive(u.id).is_some())
        .filter(|u| q.owner.is_none_or(|o| u.owner == o))
        .filter(|u| visible.as_ref().is_none_or(|v| v.contains(&u.id)))
        .filter(|u| odf_ok(c, u, q.odf))
        .map(|u| (dist(q.center, pos_f64(u.pos)), u))
        .filter(|(d, _)| *d <= q.radius)
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
    found.into_iter().map(|(_, u)| u).collect()
}
