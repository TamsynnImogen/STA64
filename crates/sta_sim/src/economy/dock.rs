//! One unloading berth per transfer station. Both the chosen receiver and
//! the time its approach queue was joined survive save and lockstep replay.

use std::collections::BTreeSet;

use sta_content::ContentDb;
use sta_core::{ContentKey, EntityId, Fixed, Vec3};

use super::{DOCK_ARRIVE, Economy, Phase, dist, mining_range, per_tick, reach};
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, Table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Ticket {
    pub station: EntityId,
    pub joined: Option<u64>,
}

pub(crate) type DockRec = Vec<(EntityId, EntityId, Option<u64>)>;

pub(super) struct Berth {
    pub pos: Vec3,
    pub heading: (Fixed, Fixed),
    pub extent: Fixed,
    pub authored: bool,
}

impl Berth {
    pub(super) fn of(db: &ContentDb, station: &Entity) -> Self {
        let def = db.unit(station.def);
        let extent = def.map_or(Fixed::ZERO, |d| d.radius);
        let Some(point) = def
            .and_then(|d| d.dock.as_ref())
            .filter(|p| p.node.is_some())
        else {
            return Self {
                pos: station.pos,
                heading: station.heading,
                extent,
                authored: false,
            };
        };
        let (x, z) = turn(station.heading, point.offset);
        Self {
            pos: Vec3 {
                x: station.pos.x + x,
                y: station.pos.y + point.height,
                z: station.pos.z + z,
            },
            heading: turn(station.heading, point.dir),
            extent,
            authored: true,
        }
    }

    pub(super) fn waiting_point(&self, waiting_span: Fixed) -> Vec3 {
        let distance = self.extent + waiting_span;
        Vec3 {
            x: self.pos.x - self.heading.0 * distance,
            y: self.pos.y,
            z: self.pos.z - self.heading.1 * distance,
        }
    }
}

/// The approach must reach beyond the smooth model's minimum turn distance.
pub(super) fn arrival(db: &ContentDb, def: ContentKey, authored: bool) -> Fixed {
    if authored || mining_range(db, def).is_some() {
        let turn_gap = db
            .unit(def)
            .map_or(Fixed::ZERO, |d| d.movement.too_close_to_turn);
        Fixed::from_int(DOCK_ARRIVE).max(turn_gap + Fixed::from_int(10))
    } else {
        reach()
    }
}

pub(super) fn span(db: &ContentDb, def: ContentKey, authored: bool) -> Fixed {
    let radius = db.unit(def).map_or(Fixed::ZERO, |d| d.radius);
    // Native queue positions accumulate ship diameters. Keep a wider gap
    // when a smooth mover would otherwise stall or enter the next slot.
    (radius + Fixed::from_int(10)).max(arrival(db, def, authored)) * Fixed::from_int(2)
}

fn turn(h: (Fixed, Fixed), v: (Fixed, Fixed)) -> (Fixed, Fixed) {
    (h.1 * v.0 + h.0 * v.1, h.1 * v.1 - h.0 * v.0)
}

/// Stock transfer interfaces expose positive rates for the cargo they take.
pub(super) fn rate(
    db: &ContentDb,
    station: &Entity,
    miner: &Entity,
    cargo: u8,
    fallback: Fixed,
) -> Option<Fixed> {
    if station.id == miner.id
        || station.owner != miner.owner
        || station.hull[0] <= Fixed::ZERO
        || station.flags.contains(UnitFlags::DYING)
        || station.progress.is_some_and(|p| p < Fixed::ONE)
    {
        return None;
    }
    let def = db.unit(station.def)?;
    // Orbital processors remain compatible with Native economy fixtures.
    // Armada II's PlanetMiningBase is a separate class without a transfer interface.
    if !def.receives_freight()
        && !(db.profile() == sta_content::Profile::Native && def.class.0 == "p_mining")
    {
        return None;
    }
    let Some(rates) = def.freight_transfer else {
        // Programmatically constructed stations retain their synthetic transfer rate.
        return Some(fallback);
    };
    let authored = *rates.get(usize::from(cargo))?;
    (authored > Fixed::ZERO).then(|| per_tick(authored).max(Fixed::from_bits(1)))
}

impl Economy {
    pub(super) fn destination(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        id: EntityId,
        cargo: u8,
        fallback: Fixed,
    ) -> Option<Ticket> {
        let miner = units.get(id)?;
        if let Some(ticket) = self.docks.get(id).copied() {
            if units
                .get(ticket.station)
                .and_then(|s| rate(db, s, miner, cargo, fallback))
                .is_some()
            {
                return Some(ticket);
            }
            self.docks.remove(id);
        }
        let speed = miner.stats.max_speed.max(Fixed::ONE);
        let radius = db.unit(miner.def).map_or(Fixed::ZERO, |d| d.radius);
        let station = units
            .iter()
            .filter(|s| rate(db, s, miner, cargo, fallback).is_some())
            .min_by_key(|s| {
                let radius_station = db.unit(s.def).map_or(Fixed::ZERO, |d| d.radius);
                let waiting = self
                    .docks
                    .iter()
                    .filter(|&(other, ticket)| {
                        ticket.station == s.id
                            && ticket.joined.is_some()
                            && self
                                .miners
                                .get(other)
                                .is_some_and(|m| m.phase == Phase::ToDock)
                    })
                    .count();
                // Stock estimates travel between the bounding spheres and
                // adds five seconds per queued freighter.
                let cost = (dist(miner.pos, s.pos) - radius_station - radius) / speed
                    + Fixed::from_int(5) * Fixed::from_int(waiting.min(32767) as i32);
                (cost, s.id)
            })?;
        let ticket = Ticket {
            station: station.id,
            joined: None,
        };
        self.docks.insert(id, ticket);
        Some(ticket)
    }

    pub(super) fn queue_ahead(
        &self,
        id: EntityId,
        at: Ticket,
    ) -> impl Iterator<Item = EntityId> + '_ {
        self.docks
            .iter()
            .filter(move |&(other, ticket)| {
                other != id
                    && ticket.station == at.station
                    && self
                        .miners
                        .get(other)
                        .is_some_and(|m| m.phase == Phase::ToDock)
                    && ticket.joined.is_some_and(|joined| {
                        at.joined.is_none_or(|ours| (joined, other) < (ours, id))
                    })
            })
            .map(|(other, _)| other)
    }

    pub(crate) fn docked_stations(&self, units: &Entities) -> BTreeSet<EntityId> {
        self.docks
            .iter()
            .filter(|&(id, ticket)| {
                self.miners
                    .get(id)
                    .is_some_and(|m| m.phase == Phase::Unloading)
                    && units.contains(id)
                    && units.contains(ticket.station)
            })
            .map(|(_, ticket)| ticket.station)
            .collect()
    }

    pub(crate) fn remove_docks(&mut self, entity: EntityId) {
        self.docks.remove(entity);
        let gone: Vec<_> = self
            .docks
            .iter()
            .filter(|(_, ticket)| ticket.station == entity)
            .map(|(id, _)| id)
            .collect();
        for id in gone {
            self.docks.remove(id);
            if let Some(miner) = self
                .miners
                .get_mut(id)
                .filter(|m| m.phase == Phase::Unloading)
            {
                miner.phase = Phase::ToDock;
                miner.unloaded = Fixed::ZERO;
                self.announce.push(id);
            }
        }
    }

    pub(crate) fn dock_rows(&self) -> DockRec {
        self.docks
            .iter()
            .map(|(id, ticket)| (id, ticket.station, ticket.joined))
            .collect()
    }

    pub(crate) fn restore_docks(
        &mut self,
        rows: DockRec,
        units: &Entities,
        tick: u64,
    ) -> Option<()> {
        let mut occupied = BTreeSet::new();
        let rows = rows
            .into_iter()
            .map(|(id, station, joined)| {
                let phase = self.miners.get(id)?.phase;
                let miner = units.get(id)?;
                let home = units.get(station)?;
                (id != station
                    && miner.owner == home.owner
                    && matches!(phase, Phase::ToDock | Phase::Unloading)
                    && joined.is_none_or(|arrival| arrival <= tick)
                    && (phase != Phase::Unloading
                        || (joined.is_some() && occupied.insert(station))))
                .then_some((id, Ticket { station, joined }))
            })
            .collect::<Option<Vec<_>>>()?;
        let docks = Table::from_rows(rows)?;
        if self
            .miners
            .iter()
            .any(|(id, m)| m.phase == Phase::Unloading && docks.get(id).is_none())
        {
            return None;
        }
        self.docks = docks;
        Some(())
    }
}
