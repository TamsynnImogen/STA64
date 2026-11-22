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

pub(crate) type ResearchRec = Vec<(EntityId, EntityId, u32)>;

pub(super) fn external_pod(station: &UnitDef, product: &UnitDef) -> bool {
    !station.research_slots.is_empty()
        && product.class.0 == "pod"
        && !product.research_station_upgrade
}

fn slot_accepts(station: &UnitDef, slot: usize, product: &UnitDef) -> bool {
    let slot = &station.research_slots[slot];
    slot.pose.is_some()
        && slot
            .reserved_for
            .as_deref()
            .is_none_or(|name| name == product.odf)
}

fn free_slot(station: &UnitDef, slots: &[Option<ContentKey>], product: &UnitDef) -> Option<usize> {
    slots
        .iter()
        .enumerate()
        .position(|(i, pod)| pod.is_none() && slot_accepts(station, i, product))
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

    pub(super) fn completed_slot(
        &self,
        units: &Entities,
        station: EntityId,
        def: &UnitDef,
        product: &UnitDef,
    ) -> Option<usize> {
        let slots = self.research_slots(units, station, def);
        if slots.contains(&Some(product.key)) {
            return None;
        }
        free_slot(def, &slots, product)
    }

    pub(super) fn research_block(
        &self,
        db: &ContentDb,
        units: &Entities,
        station: EntityId,
        def: &UnitDef,
        product: &UnitDef,
    ) -> Option<ResearchBlock> {
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
                && let Some(slot) = free_slot(def, &slots, queued)
            {
                slots[slot] = Some(queued.key);
            }
        }
        free_slot(def, &slots, product)
            .is_none()
            .then_some(ResearchBlock::NoSlot)
    }

    pub(super) fn research_view(
        &self,
        db: &ContentDb,
        units: &Entities,
        unit: &Entity,
        def: &UnitDef,
    ) -> ResearchView {
        let unavailable = def
            .build
            .list
            .iter()
            .filter_map(|item| {
                Some((
                    item.key,
                    self.research_block(db, units, unit.id, def, db.unit(item.key)?)?,
                ))
            })
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
            unavailable,
            pods,
            active_axes: None,
        }
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
                    && slot_accepts(def, slot, product)
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
