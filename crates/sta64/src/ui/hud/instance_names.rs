//! The display names a map gives its objects (host-only, never in the sim).
//!
//! Armada II binary maps carry a name buffer per object beside the script label
//! (`sta_bzn::MapObject::instance_name`, read from data). The sim does not keep it: a
//! `MapDef` field would change the replay and the snapshot formats. The host joins it to
//! the running sim's entity ids once, when the map is loaded, through the label the sim
//! already holds for every labelled object, and the HUD reads the result.
//!
//! Limits: only the map file this machine loaded is read, so a saved game or a network
//! client shows the names only when it has the same map file (a client that loads another
//! copy of the map joins by label, so a different file gives different or no names; the
//! lobby already refuses a different map). Text-format maps carry no name here.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::Resource;
use sta_sim::EntityId;

/// Entity id to the map's display name, for the objects that have one.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct InstanceNames(Arc<HashMap<EntityId, String>>);

impl InstanceNames {
    /// The names of `map`'s objects joined to the sim's `labels` (entity id, label). A label
    /// is matched trimmed; a label shared by objects that disagree (a different name, or one
    /// of them unnamed) gives nothing; no name, no row.
    pub fn of<'a>(map: &sta_bzn::Map, labels: impl IntoIterator<Item = (EntityId, &'a str)>) -> InstanceNames {
        // A label several objects share names an entity only when every sharer gives the same
        // name; otherwise the label is dropped (the map is ambiguous, no name is guessed).
        let mut by_label: HashMap<&str, Option<&str>> = HashMap::new();
        for o in &map.objects {
            let (label, name) = (o.label.trim(), o.instance_name.trim());
            if label.is_empty() {
                continue;
            }
            let name = Some(name).filter(|n| !n.is_empty());
            by_label.entry(label).and_modify(|seen| if *seen != name { *seen = None }).or_insert(name);
        }
        by_label.retain(|_, name| name.is_some());
        if by_label.is_empty() {
            return InstanceNames::default();
        }
        let rows = labels.into_iter().filter_map(|(id, label)| by_label.get(label.trim()).and_then(|n| *n).map(|n| (id, n.to_owned())));
        InstanceNames(Arc::new(rows.collect()))
    }

    pub fn get(&self, id: EntityId) -> Option<&str> {
        self.0.get(&id).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(label: &str, name: &str) -> sta_bzn::MapObject {
        sta_bzn::MapObject { odf: "fx_cruiser".into(), label: label.into(), instance_name: name.into(), ..Default::default() }
    }

    #[test]
    fn names_join_by_trimmed_label_and_blank_names_give_no_row() {
        let map = sta_bzn::Map {
            objects: vec![object("gate_a", "Aurelia Test"), object("gate_b", "  "), object("gate_c", ""), object(" gate_d ", "Orsino Station-B"), object("", "Nameless Label")],
            ..Default::default()
        };
        let labels = [(EntityId(3), "gate_a"), (EntityId(4), "gate_b"), (EntityId(5), "gate_c"), (EntityId(6), "gate_d"), (EntityId(7), ""), (EntityId(8), "unknown")];
        let names = InstanceNames::of(&map, labels);
        assert_eq!(names.get(EntityId(3)), Some("Aurelia Test"));
        assert_eq!(names.get(EntityId(6)), Some("Orsino Station-B"));
        for id in [4, 5, 7, 8] {
            assert_eq!(names.get(EntityId(id)), None);
        }
        assert!(InstanceNames::of(&sta_bzn::Map::default(), labels).is_empty());
    }

    #[test]
    fn a_label_shared_by_objects_that_disagree_gives_no_name() {
        let map = sta_bzn::Map {
            objects: vec![object("dup", "Same Name"), object("dup", "Same Name"), object("mixed", "Named"), object("mixed", ""), object("clash", "One"), object("clash", "Two")],
            ..Default::default()
        };
        let labels = [(EntityId(1), "dup"), (EntityId(2), "dup"), (EntityId(3), "mixed"), (EntityId(4), "clash")];
        let names = InstanceNames::of(&map, labels);
        assert_eq!((names.get(EntityId(1)), names.get(EntityId(2))), (Some("Same Name"), Some("Same Name")));
        assert_eq!((names.get(EntityId(3)), names.get(EntityId(4))), (None, None));
    }
}
