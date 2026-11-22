//! Map labels the load leaves unresolved (LUA-25).
//!
//! `GetHandle(label)` answers a handle only for an object the map load created
//! with that label. The load creates no labelled object for some objects a map
//! does carry: nebula and asteroid regions, scenery props (suns, moons,
//! wrecks), scenery lights, wormholes. A script that fetches one of them gets
//! 0 and nothing records that this is STA64's gap and not the original's own
//! unresolvable reference. This module lists those labels per mission, host
//! side, from the map file itself (the BZN) and the labels the built sim
//! carries; it adds nothing to the map definition, the snapshot or the replay.
//!
//! Rule (STA64's own, not read from the game): a label that exists in the
//! mission's own map file and is carried by no object of the built sim is a
//! *handle gap*; a label in no map object (the script's own sentinel, or an
//! object the script builds) is not, and stays the original's unresolvable
//! reference (LUA-21 answers). Evidence: the label data is the map's (read
//! from data); which kinds the load skips is `MapDef`'s conversion (read from
//! code).

use std::collections::BTreeSet;

use sta_content::OdfIndex;
use sta_vfs::Vfs;

/// What the unlabelled object is, from its ODF class label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelKind {
    /// `asteroidfield`, `nebula_obj`: the load keeps a region with no label.
    Region,
    /// `background_obj` (sun, moon, wreck): the load keeps no labelled object.
    Prop,
    /// A `background_obj` whose ODF name holds `light`: usually a scenery light (cosmetic; the load keeps position and colour only when it is hidden), named by the ODF name alone, so not proven to be one.
    Light,
    /// `wormhole_obj`: skipped today; LUA-24 makes these entities.
    Wormhole,
    /// Anything else the load did not label.
    Other,
}

impl LabelKind {
    pub fn name(self) -> &'static str {
        match self {
            LabelKind::Region => "region",
            LabelKind::Prop => "prop",
            LabelKind::Light => "prop (light name)",
            LabelKind::Wormhole => "wormhole",
            LabelKind::Other => "object",
        }
    }

    fn of_class(class: &str) -> LabelKind {
        match class {
            "asteroidfield" | "nebula_obj" => LabelKind::Region,
            "background_obj" => LabelKind::Prop,
            "wormhole_obj" => LabelKind::Wormhole,
            _ => LabelKind::Other,
        }
    }
}

/// A map label no object of the built sim carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelGap {
    pub label: String,
    pub odf: String,
    pub kind: LabelKind,
}

/// The labelled objects of `map` whose label is not in `carried` (the labels the
/// built sim carries), first object per label, in file order. `class_of` gives
/// an ODF's class label (lower case; empty when unknown).
pub fn find_label_gaps(map: &sta_bzn::Map, carried: &BTreeSet<String>, class_of: &mut dyn FnMut(&str) -> String) -> Vec<LabelGap> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for o in &map.objects {
        // `MapDef` trims a label the same way.
        let label = o.label.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        if label.is_empty() || carried.contains(label) || !seen.insert(label.to_owned()) {
            continue;
        }
        let mut kind = LabelKind::of_class(&class_of(&o.odf));
        if kind == LabelKind::Prop && o.odf.to_ascii_lowercase().contains("light") {
            kind = LabelKind::Light;
        }
        out.push(LabelGap { label: label.to_owned(), odf: o.odf.to_ascii_lowercase(), kind });
    }
    out
}

/// [`find_label_gaps`] over the map `bzn` (`a1_fed5.bzn`) of `vfs`; empty when the
/// map cannot be read.
pub fn read_map_label_gaps(vfs: &Vfs, index: &OdfIndex, bzn: &str, carried: &BTreeSet<String>) -> Vec<LabelGap> {
    let Some(Ok(bytes)) = vfs.read(&format!("bzn/{bzn}")) else { return Vec::new() };
    let Ok(map) = sta_bzn::parse(&bytes) else { return Vec::new() };
    let profile = sta_content::Profile::detect(vfs);
    let mut class_of = |odf: &str| {
        let k = odf.to_ascii_lowercase();
        sta_content::odf::resolve_exact_for_profile(&k, profile, &mut |n, v| index.load(vfs, n, v))
            .ok()
            .and_then(|d| d.class_label().map(|c| c.trim().to_ascii_lowercase()))
            .unwrap_or_default()
    };
    find_label_gaps(&map, carried, &mut class_of)
}

#[cfg(test)]
mod tests;
