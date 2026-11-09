//! Resources and mining (SIM-07): player stock, resource nodes and freighters
//! that loop between a node and the nearest friendly station.
//!
//! - Resource ids are the content ones: 0 dilithium, 1 latinum, 2 metal,
//!   3 biomatter ([`RESOURCES`]). A player's stock is per id, in the view
//!   ([`PlayerView::resources`](crate::view::PlayerView)).
//! - `Order::Mine { units, node }` sends freighters (class `freighter`, or an
//!   ODF with `miner 1`) to a [`EntityKind::Resource`] entity. A freighter
//!   cycles `ToNode -> Mining -> ToDock -> Unloading`: it fills its hold at
//!   `mining rate` per second (ODF `miningRate`, default 15), carries it to
//!   the nearest finished station of its owner and unloads at the transfer
//!   rate (ODF `dilithiumTransferRate`, default 15 per second), crediting the
//!   stock as it goes. [`SimEvent::Delivered`] is emitted once per trip.
//!   Hold size: ODF `maxdilithium`, default 150.
//! - A `Move` (not queued) or `Stop` of a freighter ends its mining.
//! - Which resource a node gives comes from its class (`latinum`) or its
//!   ODF `resource` key (names containing latinum, metal/tritanium,
//!   biomatter), else dilithium. Nodes hold [`RICHNESS`] and run dry.
//! - Within 150 units of node or station counts as there; the freighter
//!   stops and works that same tick, so a trip with no travel takes exactly
//!   `capacity / rate + capacity / unload` ticks.
//! - Not yet: mining stations (`mining` class) producing by themselves,
//!   trade, `miningratemodifier`, `trickleMiningRateFraction`, per-resource
//!   transfer rates (all resources use `dilithiumTransferRate`).
//! - Persistent state (stock, nodes, miners) is [`Economy`]; the snapshot
//!   (`to_rec`/`from_rec`) must include it. It is in the digest once any
//!   exists.

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Vec3, TICK_HZ};

use crate::command::Edit;
use crate::event::SimEvent;
use crate::world::{Entities, EntityKind, Fnv, Table};

/// Resource ids: dilithium, latinum, metal, biomatter.
pub const RESOURCES: usize = 4;

/// What a fresh node holds.
pub const RICHNESS: i32 = 1_000_000;

const DEFAULT_CAPACITY: i32 = 150;
const DEFAULT_RATE: i32 = 15;

fn reach() -> Fixed {
    Fixed::from_int(150)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    ToNode,
    Mining,
    ToDock,
    Unloading,
}

impl Phase {
    fn code(self) -> u8 {
        self as u8
    }
    #[allow(dead_code)] // used by from_rec, which the snapshot calls
    fn from_code(c: u8) -> Phase {
        match c {
            1 => Phase::Mining,
            2 => Phase::ToDock,
            3 => Phase::Unloading,
            _ => Phase::ToNode,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Miner {
    node: EntityId,
    res: u8,
    cargo: Fixed,
    /// Delivered so far on this unloading, for the event.
    unloaded: Fixed,
    phase: Phase,
}

/// Mining numbers of one freighter, from its definition.
#[derive(Debug, Clone, Copy)]
struct Cfg {
    capacity: Fixed,
    rate: Fixed,
    unload: Fixed,
}

fn per_tick(per_second: Fixed) -> Fixed {
    per_second / Fixed::from_int(TICK_HZ as i32)
}

fn cfg_of(db: &ContentDb, def: ContentKey) -> Option<Cfg> {
    let d = db.unit(def)?;
    let miner = d.class.0 == "freighter" || d.miner;
    if !miner {
        return None;
    }
    // Zero means the definition does not say: the defaults apply.
    let or = |v: Fixed, default: i32| if v > Fixed::ZERO { v } else { Fixed::from_int(default) };
    Some(Cfg {
        capacity: or(d.max_dilithium, DEFAULT_CAPACITY),
        rate: per_tick(or(d.mining_rate, DEFAULT_RATE)),
        unload: per_tick(or(d.transfer_rate, DEFAULT_RATE)),
    })
}

/// Resource id a node gives.
fn node_resource(db: &ContentDb, def: ContentKey) -> u8 {
    let Some(d) = db.unit(def) else { return 0 };
    if d.class.0 == "latinum" {
        return 1;
    }
    d.resource.unwrap_or(0)
}

fn dist(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

/// Stock, nodes and freighters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Economy {
    /// Ascending player id.
    stock: Vec<(PlayerId, [Fixed; RESOURCES])>,
    /// What is left in each node that has been mined.
    nodes: Table<Fixed>,
    miners: Table<Miner>,
}

/// Plain form of [`Economy`] for snapshots (bits of every [`Fixed`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomyRec {
    pub stock: Vec<(u8, [i64; RESOURCES])>,
    pub nodes: Vec<(u32, i64)>,
    /// `(id, node, resource, cargo, unloaded, phase)`.
    pub miners: Vec<(u32, u32, u8, i64, i64, u8)>,
}

impl Economy {
    pub fn is_empty(&self) -> bool {
        self.stock.is_empty() && self.nodes.is_empty() && self.miners.is_empty()
    }

    /// A player's stock per resource id; empty until they have any.
    pub fn stock_of(&self, p: PlayerId) -> Vec<Fixed> {
        self.stock
            .iter()
            .find(|s| s.0 == p)
            .map(|s| s.1.to_vec())
            .unwrap_or_default()
    }

    /// Whether `id` is on a mining order.
    pub fn is_mining(&self, id: EntityId) -> bool {
        self.miners.get(id).is_some()
    }

    /// What `id` carries, if it is a freighter on a mining order.
    pub fn cargo(&self, id: EntityId) -> Option<Fixed> {
        self.miners.get(id).map(|m| m.cargo)
    }

    fn slot(&mut self, p: PlayerId) -> &mut [Fixed; RESOURCES] {
        let i = match self.stock.binary_search_by_key(&p, |s| s.0) {
            Ok(i) => i,
            Err(i) => {
                self.stock.insert(i, (p, [Fixed::ZERO; RESOURCES]));
                i
            }
        };
        &mut self.stock[i].1
    }

    /// `Edit::Cargo`; `None` when `e` is another edit.
    pub(crate) fn apply_edit(&mut self, from: PlayerId, e: &Edit) -> Option<bool> {
        let Edit::Cargo(p, res, amount) = *e else {
            return None;
        };
        if (from != PlayerId::SCRIPT && from != PlayerId::BENCH) || res as usize >= RESOURCES {
            return Some(false);
        }
        self.slot(p)[res as usize] = amount;
        Some(true)
    }

    /// Set a player's whole stock (a game's starting resources).
    pub(crate) fn set_stock(&mut self, p: PlayerId, stock: [Fixed; RESOURCES]) {
        *self.slot(p) = stock;
    }

    /// Take `amount` of `res` from `p` if there is that much (SIM-08 builds).
    pub fn spend(&mut self, p: PlayerId, res: u8, amount: Fixed) -> bool {
        if res as usize >= RESOURCES
            || self
                .stock_of(p)
                .get(res as usize)
                .is_none_or(|&s| s < amount)
        {
            return false;
        }
        let s = &mut self.slot(p)[res as usize];
        *s = *s - amount;
        true
    }

    /// An `Order::Mine`: the freighters among `ids` (own units for players)
    /// head for `node`. `false`: none could.
    pub(crate) fn order_mine(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        player: PlayerId,
        ids: &[EntityId],
        node: EntityId,
    ) -> bool {
        let Some(n) = units.get(node).filter(|n| n.kind == EntityKind::Resource) else {
            return false;
        };
        let res = node_resource(db, n.def);
        let mut ids: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|&id| {
                units.get(id).is_some_and(|u| {
                    u.kind == EntityKind::Craft
                        && (!player.is_player() || u.owner == player)
                        && cfg_of(db, u.def).is_some()
                })
            })
            .collect();
        ids.sort();
        ids.dedup();
        for &id in &ids {
            let old = self.miners.get(id).copied();
            let cargo = old
                .filter(|m| m.res == res)
                .map_or(Fixed::ZERO, |m| m.cargo);
            let phase = if cargo > Fixed::ZERO {
                Phase::ToDock
            } else {
                Phase::ToNode
            };
            self.miners.insert(
                id,
                Miner {
                    node,
                    res,
                    cargo,
                    unloaded: Fixed::ZERO,
                    phase,
                },
            );
        }
        !ids.is_empty()
    }

    /// A move or stop of these units ends their mining (their cargo is lost).
    pub(crate) fn cancel(&mut self, units: &Entities, player: PlayerId, ids: &[EntityId]) {
        for &id in ids {
            if units
                .get(id)
                .is_some_and(|u| !player.is_player() || u.owner == player)
            {
                self.miners.remove(id);
            }
        }
    }

    /// One tick of every freighter, ascending id.
    pub(crate) fn step(
        &mut self,
        units: &mut Entities,
        db: &ContentDb,
        events: &mut Vec<SimEvent>,
    ) {
        let ids: Vec<EntityId> = self.miners.iter().map(|(id, _)| id).collect();
        for id in ids {
            let Some(mut m) = self.miners.get(id).copied() else {
                continue;
            };
            let alive = units
                .get(id)
                .zip(units.get(m.node))
                .map(|(u, n)| (u.def, u.owner, n.pos));
            let Some((def, owner, node_pos)) = alive else {
                self.miners.remove(id);
                continue;
            };
            let Some(cfg) = cfg_of(db, def) else {
                self.miners.remove(id);
                continue;
            };
            // A transition that lands in reach acts the same tick.
            for _ in 0..4 {
                let Some(pos) = units.get(id).map(|u| u.pos) else {
                    break;
                };
                match m.phase {
                    Phase::ToNode => {
                        if dist(pos, node_pos) <= reach() {
                            set_target(units, id, None);
                            m.phase = Phase::Mining;
                        } else {
                            steer(units, id, node_pos);
                            break;
                        }
                    }
                    Phase::Mining => {
                        let left = self
                            .nodes
                            .get(m.node)
                            .copied()
                            .unwrap_or(Fixed::from_int(RICHNESS));
                        if m.cargo >= cfg.capacity || (left == Fixed::ZERO && m.cargo > Fixed::ZERO)
                        {
                            m.phase = Phase::ToDock;
                            break;
                        }
                        let take = cfg.rate.min(cfg.capacity - m.cargo).min(left);
                        m.cargo = m.cargo + take;
                        self.nodes.insert(m.node, left - take);
                        if m.cargo >= cfg.capacity {
                            m.phase = Phase::ToDock;
                        }
                        break;
                    }
                    Phase::ToDock => {
                        let Some(at) = nearest_station(units, owner, pos) else {
                            break;
                        };
                        if dist(pos, at) <= reach() {
                            set_target(units, id, None);
                            m.phase = Phase::Unloading;
                        } else {
                            steer(units, id, at);
                            break;
                        }
                    }
                    Phase::Unloading => {
                        let give = cfg.unload.min(m.cargo);
                        m.cargo = m.cargo - give;
                        m.unloaded = m.unloaded + give;
                        let s = &mut self.slot(owner)[m.res as usize];
                        *s = *s + give;
                        if m.cargo == Fixed::ZERO {
                            events.push(SimEvent::Delivered {
                                player: owner,
                                resource: m.res,
                                amount: m.unloaded,
                            });
                            m.unloaded = Fixed::ZERO;
                            m.phase = Phase::ToNode;
                        }
                        break;
                    }
                }
            }
            self.miners.insert(id, m);
        }
        // Rows of nodes that are gone go with them.
        let gone: Vec<EntityId> = self
            .nodes
            .iter()
            .map(|(id, _)| id)
            .filter(|&id| !units.contains(id))
            .collect();
        for id in gone {
            self.nodes.remove(id);
        }
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.u64(self.stock.len() as u64);
        for (p, s) in &self.stock {
            h.u64(p.0 as u64);
            for &f in s {
                h.fixed(f);
            }
        }
        h.u64(self.nodes.len() as u64);
        for (id, left) in self.nodes.iter() {
            h.u64(id.0 as u64);
            h.fixed(*left);
        }
        h.u64(self.miners.len() as u64);
        for (id, m) in self.miners.iter() {
            h.u64(id.0 as u64);
            h.u64(m.node.0 as u64);
            h.u64(m.res as u64);
            h.fixed(m.cargo);
            h.fixed(m.unloaded);
            h.u64(m.phase.code() as u64);
        }
    }

    /// The state in plain form, for the snapshot.
    #[allow(dead_code)] // the snapshot (separate package) calls it
    pub(crate) fn to_rec(&self) -> EconomyRec {
        EconomyRec {
            stock: self
                .stock
                .iter()
                .map(|(p, s)| (p.0, s.map(Fixed::to_bits)))
                .collect(),
            nodes: self
                .nodes
                .iter()
                .map(|(id, f)| (id.0, f.to_bits()))
                .collect(),
            miners: self
                .miners
                .iter()
                .map(|(id, m)| {
                    (
                        id.0,
                        m.node.0,
                        m.res,
                        m.cargo.to_bits(),
                        m.unloaded.to_bits(),
                        m.phase.code(),
                    )
                })
                .collect(),
        }
    }

    #[allow(dead_code)] // the snapshot (separate package) calls it
    pub(crate) fn from_rec(r: &EconomyRec) -> Economy {
        let mut stock: Vec<_> = r.stock.iter().map(|&(p, s)| (PlayerId(p), s.map(Fixed::from_bits))).collect();
        stock.sort_by_key(|s| s.0);
        let mut e = Economy { stock, ..Economy::default() };
        for &(id, f) in &r.nodes {
            e.nodes.insert(EntityId(id), Fixed::from_bits(f));
        }
        for &(id, node, res, cargo, unloaded, phase) in &r.miners {
            e.miners.insert(
                EntityId(id),
                Miner {
                    node: EntityId(node),
                    res,
                    cargo: Fixed::from_bits(cargo),
                    unloaded: Fixed::from_bits(unloaded),
                    phase: Phase::from_code(phase),
                },
            );
        }
        e
    }
}

fn set_target(units: &mut Entities, id: EntityId, to: Option<Vec3>) {
    if let Some(u) = units.get_mut(id) {
        u.target = to;
    }
}

/// Head for `to`.
fn steer(units: &mut Entities, id: EntityId, to: Vec3) {
    set_target(units, id, Some(to));
}

/// The owner's nearest finished station by distance (ties: lowest id).
fn nearest_station(units: &Entities, owner: PlayerId, from: Vec3) -> Option<Vec3> {
    units
        .iter()
        .filter(|s| {
            s.kind == EntityKind::Station
                && s.owner == owner
                && s.progress.is_none_or(|p| p >= Fixed::ONE)
        })
        .map(|s| (dist(from, s.pos), s.id, s.pos))
        .min_by_key(|&(d, id, _)| (d, id))
        .map(|(_, _, p)| p)
}

#[cfg(test)]
pub(crate) mod tests;
