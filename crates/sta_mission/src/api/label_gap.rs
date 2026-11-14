//! `GetHandle` on a label the map has and the load did not create (LUA-25).
//!
//! The host hands the runtime the map's unresolved labels
//! ([`crate::map_labels`]). A `GetHandle` that finds no object and names one of
//! them is a handle gap: it is listed in `unsupported()` under the binding's
//! name with the label and what the object is, and `Ctx::gap("GetHandle")` records
//! that handles may be missing because of a gap ([`super::taint`]), so a read
//! that later names a handle that never existed is tainted and a mission end after
//! such a read is held back (Armada II; Armada I has no end hold). A label in no
//! map object is no gap. The entries are kept apart from `Ctx::unsupported` so the
//! `A2` call wrapper does not also taint every read through the call.

use std::collections::BTreeMap;

use crate::map_labels::{LabelGap, LabelKind};

use super::Ctx;

#[derive(Default)]
pub(crate) struct LabelGaps {
    /// Unresolved map labels by name.
    pub(super) by_label: BTreeMap<String, LabelGap>,
    /// Counted `unsupported()` entries made by lookups.
    pub(super) hits: BTreeMap<String, u32>,
}

impl LabelGaps {
    pub(crate) fn set(&mut self, gaps: Vec<LabelGap>) {
        self.by_label = gaps.into_iter().map(|g| (g.label.clone(), g)).collect();
    }

    pub(crate) fn hits(&self) -> impl Iterator<Item = (&String, &u32)> {
        self.hits.iter()
    }
}

impl Ctx {
    /// A `GetHandle(label)` of binding `api` (`A1`, `A2`) found no object: record the
    /// gap when the map has the label. `ignore_case` is set for A1, whose lookup also matches without case.
    pub fn note_unresolved_label(&mut self, api: &str, label: &str, ignore_case: bool) {
        let found = self.label_gaps.by_label.get(label).or_else(|| {
            ignore_case.then(|| self.label_gaps.by_label.values().find(|g| g.label.eq_ignore_ascii_case(label))).flatten()
        });
        let Some(g) = found else { return };
        let why = match g.kind {
            LabelKind::Wormhole => "wormhole object, not an entity yet (LUA-24)".to_owned(),
            k => format!("{} `{}` is in the map, the load creates no labelled object", k.name(), g.odf),
        };
        let key = format!("{api}.GetHandle: label '{}': {why}", g.label);
        *self.label_gaps.hits.entry(key).or_default() += 1;
        self.gap("GetHandle", &[]);
    }
}

#[cfg(test)]
mod tests;
