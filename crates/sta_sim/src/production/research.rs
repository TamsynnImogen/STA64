//! Stock Armada II mounts research pods in the first available authored slot.
//! Completed pods are ordinary tech-tree holdings with a station/slot link.

use sta_content::{ContentDb, UnitDef};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Vec3};

use super::{
    Production, launch,
    view::{ResearchBlock, ResearchPodView, ResearchView},
};
use crate::world::{Entities, Entity, Table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Attachment {
    pub station: EntityId,
    pub slot: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PodSlot {
    pub slot: usize,
    pub replacing: Option<EntityId>,
}

pub(crate) type ResearchRec = Vec<(EntityId, EntityId, u32)>;

pub(super) fn external_pod(station: &UnitDef, product: &UnitDef) -> bool {
    (!station.research_slots.is_empty() || (station.class.0 == "research" && product.system_upgrade.is_some()))
        && product.class.0 == "pod"
        && !product.research_station_upgrade
}

fn slot_accepts(db: &ContentDb, station: &UnitDef, slot: usize, product: &UnitDef) -> bool {
    let index = slot;
    let slot = &station.research_slots[index];
    slot.pose.is_some() && slot.reserved_for.as_deref().is_none_or(|name| {
        name == product.odf || station.research_build.iter().any(|line| {
            let primary = line.primary.and_then(|key| db.unit(key));
            (line.index == index || primary.is_some_and(|p| p.odf == name))
                && (line.primary == Some(product.key) || line.successors.contains(&product.key))
        })
    })
}

fn free_slot(db: &ContentDb, station: &UnitDef, slots: &[Option<ContentKey>], product: &UnitDef) -> Option<usize> {
    slots
        .iter()
        .enumerate()
        .position(|(i, pod)| pod.is_none() && slot_accepts(db, station, i, product))
}

impl Production {
    fn research_slots(
        &self,
        units: &Entities,
        station: EntityId,
        def: &UnitDef,
    ) -> Vec<Option<ContentKey>> {
        let mut slots = vec![None; def.research_slots.len()];
        for (id, link) in self
            .research
            .iter()
            .filter(|(_, link)| link.station == station)
        {
            if let Some(pod) = units.get(id) {
                slots[link.slot] = Some(pod.def);
            }
        }
        slots
    }

    /// Advance each authored button using the pod attached to this station.
    /// The chain index is a button index; the attachment itself owns the
    /// physical slot. Construction order therefore cannot cross two systems.
    fn upgrade_line(
        &self,
        db: &ContentDb,
        units: &Entities,
        station: EntityId,
        line: &sta_content::ResearchBuildSlot,
    ) -> Option<(ContentKey, Option<PodSlot>)> {
        let first = line.primary?;
        let Some(upgrade) = db.unit(first).and_then(|def| def.system_upgrade) else {
            return Some((first, None));
        };
        let held = self.research.iter()
            .filter(|(_, link)| link.station == station)
            .filter_map(|(id, link)| {
                let pod = units.get(id)?;
                let info = db.unit(pod.def)?.system_upgrade?;
                (info.system == upgrade.system && (pod.def == first || line.successors.contains(&pod.def)))
                    .then_some((info.tier, pod.def, id, link.slot))
            })
            .max_by_key(|(tier, _, _, _)| *tier);
        let Some((tier, key, id, slot)) = held else {
            return Some((first, None));
        };
        let max = db.rules().upgrade_pod_maximum_tier();
        let next = line.successors.iter().copied().find(|&key| db.unit(key)
            .and_then(|pod| pod.system_upgrade)
            .is_some_and(|next| next.tier == tier + 1 && next.tier <= max));
        Some((next.unwrap_or(key), next.map(|_| PodSlot { slot, replacing: Some(id) })))
    }

    pub(super) fn completed_slot(
        &self,
        db: &ContentDb,
        units: &Entities,
        station: EntityId,
        def: &UnitDef,
        product: &UnitDef,
    ) -> Option<PodSlot> {
        let slots = self.research_slots(units, station, def);
        if slots.contains(&Some(product.key)) {
            return None;
        }
        if !def.research_build.is_empty() && product.system_upgrade.is_some() {
            let (_, replacing) = def.research_build.iter()
                .filter_map(|line| self.upgrade_line(db, units, station, line))
                .find(|(key, _)| *key == product.key)?;
            if replacing.is_some() {
                return replacing;
            }
        }
        free_slot(db, def, &slots, product).map(|slot| PodSlot { slot, replacing: None })
    }

    pub(super) fn research_block(
        &self,
        db: &ContentDb,
        units: &Entities,
        station: EntityId,
        def: &UnitDef,
        product: &UnitDef,
        check_list: bool,
    ) -> Option<ResearchBlock> {
        let mut replacement = false;
        if !def.research_build.is_empty() && (check_list || product.system_upgrade.is_some()) {
            let row = def.research_build.iter()
                .filter_map(|line| self.upgrade_line(db, units, station, line))
                .find(|(key, _)| *key == product.key);
            let Some((_, replacing)) = row else { return Some(ResearchBlock::Prerequisite) };
            replacement = replacing.is_some();
        }
        if !external_pod(def, product) {
            return None;
        }
        let mut slots = self.research_slots(units, station, def);
        if slots.contains(&Some(product.key)) {
            return Some(ResearchBlock::AlreadyResearched);
        }
        for item in self.queues.get(station).into_iter().flatten() {
            if item.def == product.key {
                return Some(ResearchBlock::Queued);
            }
            if let Some(queued) = db.unit(item.def).filter(|d| external_pod(def, d))
                && self.completed_slot(db, units, station, def, queued).is_none_or(|slot| slot.replacing.is_none())
                && let Some(slot) = free_slot(db, def, &slots, queued)
            {
                slots[slot] = Some(queued.key);
            }
        }
        (!replacement && free_slot(db, def, &slots, product).is_none()).then_some(ResearchBlock::NoSlot)
    }

    pub(super) fn research_view(
        &self,
        db: &ContentDb,
        units: &Entities,
        unit: &Entity,
        def: &UnitDef,
    ) -> ResearchView {
        let build_slots = (!def.research_build.is_empty()).then(|| def.research_build.iter()
            .filter_map(|line| Some((line.index, self.upgrade_line(db, units, unit.id, line)?.0)))
            .collect::<Vec<_>>());
        let build_items = match &build_slots {
            Some(slots) => slots.iter().map(|(_, key)| *key).collect::<Vec<_>>(),
            None => def.build.list.iter().map(|item| item.key).collect(),
        };
        let unavailable = build_items.into_iter()
            .filter_map(|key| Some((key, self.research_block(db, units, unit.id, def, db.unit(key)?, true)?)))
            .collect();
        let mut pods: Vec<_> = self
            .research
            .iter()
            .filter(|(_, link)| link.station == unit.id)
            .filter_map(|(entity, link)| {
                let (_, _, axes) = pose(unit, def, link.slot)?;
                units.get(entity)?;
                Some(ResearchPodView {
                    entity,
                    slot: link.slot,
                    axes,
                })
            })
            .collect();
        pods.sort_by_key(|pod| pod.slot);
        ResearchView {
            build_slots,
            unavailable,
            pods,
            active_axes: None,
            replacing: None,
        }
    }

    pub(crate) fn has_upgrade_pods(&self, station: EntityId, units: &Entities, db: Option<&ContentDb>) -> bool {
        self.research.iter().filter(|(_, link)| link.station == station)
            .any(|(id, _)| units.get(id).and_then(|pod| db?.unit(pod.def)).is_some_and(|def| def.system_upgrade.is_some()))
    }

    pub(crate) fn remove_research_entity(&mut self, id: EntityId) -> Vec<EntityId> {
        self.research.remove(id);
        let children: Vec<_> = self
            .research
            .iter()
            .filter(|(_, link)| link.station == id)
            .map(|(child, _)| child)
            .collect();
        for child in &children {
            self.research.remove(*child);
        }
        children
    }

    pub(crate) fn transfer_research_pods(
        &self,
        units: &mut Entities,
        station: EntityId,
        to: PlayerId,
    ) {
        for (id, _) in self
            .research
            .iter()
            .filter(|(_, link)| link.station == station)
        {
            if let Some(pod) = units.get_mut(id) {
                pod.owner = to;
            }
        }
    }

    pub(crate) fn update_research_poses(&self, db: &ContentDb, units: &mut Entities) {
        for (id, link) in self.research.iter() {
            let Some(host) = units.get(link.station) else {
                continue;
            };
            let Some((pos, heading, _)) = db.unit(host.def).and_then(|d| pose(host, d, link.slot))
            else {
                continue;
            };
            if let Some(pod) = units.get_mut(id) {
                pod.pos = pos;
                pod.heading = heading;
            }
        }
    }

    pub(crate) fn research_rows(&self) -> ResearchRec {
        self.research
            .iter()
            .map(|(id, link)| (id, link.station, link.slot as u32))
            .collect()
    }

    pub(crate) fn restore_research(
        &mut self,
        rows: ResearchRec,
        units: &Entities,
        db: Option<&ContentDb>,
    ) -> Option<()> {
        let mut occupied = std::collections::BTreeSet::new();
        let links = rows
            .into_iter()
            .map(|(id, station, slot)| {
                let db = db?;
                let host = units.get(station)?;
                let pod = units.get(id)?;
                let def = db.unit(host.def)?;
                let product = db.unit(pod.def)?;
                let slot = slot as usize;
                (pod.owner == host.owner
                    && external_pod(def, product)
                    && slot < def.research_slots.len()
                    && slot_accepts(db, def, slot, product)
                    && occupied.insert((station, slot)))
                .then_some((id, Attachment { station, slot }))
            })
            .collect::<Option<Vec<_>>>()?;
        self.research = Table::from_rows(links)?;
        Some(())
    }
}

/// Model-space origin (scaled by the station) and basis rotated into world
/// space. The sim uses horizontal heading, while the renderer uses all axes.
pub(super) fn pose(
    station: &Entity,
    def: &UnitDef,
    slot: usize,
) -> Option<(Vec3, (Fixed, Fixed), [Vec3; 3])> {
    let pose = def.research_slots.get(slot)?.pose?;
    let scale = if def.scale > Fixed::ZERO {
        def.scale
    } else {
        Fixed::ONE
    };
    let turn = |v: Vec3| {
        let (x, z) = launch::turn(station.heading, (v.x, v.z));
        Vec3 { x, y: v.y, z }
    };
    let offset = turn(Vec3 {
        x: pose.origin.x * scale,
        y: pose.origin.y * scale,
        z: pose.origin.z * scale,
    });
    let pos = Vec3 {
        x: station.pos.x + offset.x,
        y: station.pos.y + offset.y,
        z: station.pos.z + offset.z,
    };
    let axes = pose.axes.map(turn);
    let ahead = axes[2];
    let length = (ahead.x * ahead.x + ahead.z * ahead.z).sqrt();
    let heading = if length > Fixed::from_f64(0.001) {
        (ahead.x / length, ahead.z / length)
    } else {
        station.heading
    };
    Some((pos, heading, axes))
}
