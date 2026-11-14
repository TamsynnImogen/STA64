//! Resources and mining (SIM-07): player stock, resource nodes and freighters
//! that loop between a node and the nearest friendly station.
//!
//! - Resource ids are the content ones: 0 dilithium, 1 latinum, 2 metal,
//!   3 biomatter ([`RESOURCES`]). A player's stock is per id, in the view
//!   ([`PlayerView::resources`](crate::view::PlayerView)).
//! - `Order::Mine { units, node }` sends freighters (`UnitDef::mines`: class `freighter` or an
//!   ODF with `miner 1`, unless it says `miner 0`) to a [`EntityKind::Resource`] entity. A freighter
//!   cycles `ToNode -> Mining -> ToDock -> Unloading`: it fills its hold at
//!   `mining rate` per second (ODF `miningRate`, default 15), carries it to
//!   the nearest finished station of its owner and unloads at the transfer
//!   rate (ODF `dilithiumTransferRate`, default 15 per second), crediting the
//!   stock as it goes. [`SimEvent::Delivered`] is emitted once per trip.
//!   Hold size: ODF `maxdilithium`, default 150.
//! - A `Move` (not queued) or `Stop` ends mining, but a loaded freighter keeps
//!   its cargo while idle. A later Mine order delivers that cargo first.
//! - Which resource a node gives comes from its class (`latinum`) or its
//!   ODF `resource` key, which content resolves to a slot when it loads: the
//!   `amt<slot>` of the resource ODF (Fleet Operations 4.0's tritanium moon
//!   holds `amtlatinum`, so it fills the slot the game calls Tritanium), else
//!   the game's word for the slot in the text, else dilithium. Nodes hold [`RICHNESS`] and run dry.
//!   Typed Background planets reject mining; typed Producer planets need an
//!   explicit resource slot. This does not implement orbital/multi-resource mining.
//! - A freighter whose ODF has a mining beam (ordnance class `minebm`) works once the surface gap
//!   to the node is under the beam's `range - 15`, then circles the node while it mines: it steers
//!   at the point 30 degrees ahead on the circle of radius `range` (the original's
//!   `ANCHOR_PURSUIT_POINT_ANGLE`, read from the executable, static), on its own height layer
//!   (`id & 15`); the harvest numbers are unchanged. Any other freighter, and a station, count as
//!   there within 150 units of the centre; the freighter
//!   stops and works that same tick, so a trip with no travel takes exactly
//!   `capacity / rate + capacity / unload` ticks.
//! - Passive income from `resourceweapon` hardpoints is in [`passive`] (SIM-24).
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

mod passive;
mod crew;
mod personnel;

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
    Idle,
}

impl Phase {
    fn code(self) -> u8 {
        self as u8
    }
    fn announced(self) -> crate::event::MinerPhase {
        use crate::event::MinerPhase;
        match self {
            Phase::ToNode => MinerPhase::ToNode,
            Phase::Mining => MinerPhase::Mining,
            Phase::ToDock => MinerPhase::ToDock,
            Phase::Unloading => MinerPhase::Unloading,
            Phase::Idle => MinerPhase::Idle,
        }
    }
    #[allow(dead_code)] // used by from_rec, which the snapshot calls
    fn from_code(c: u8) -> Phase {
        match c {
            1 => Phase::Mining,
            2 => Phase::ToDock,
            3 => Phase::Unloading,
            4 => Phase::Idle,
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
    if !d.mines() {
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

/// Ordnance class of a mining beam (`classLabel = "minebm"`).
const MINING_BEAM_CLASS: &str = "minebm";
/// RTS_CFG `ANCHOR_PURSUIT_POINT_ANGLE` (pi/6 in every install): the pursuit point on the orbit is
/// this far ahead of the freighter. cos and sin of it.
const PURSUIT_COS: Fixed = Fixed::from_bits(3_719_550_787);
const PURSUIT_SIN: Fixed = Fixed::from_bits(2_147_483_648);
/// Height layer constants: 7.5, 2/15, k 5/6 (moons, planets) and 1/2 (latinum nebula).
const LAYER_MID: Fixed = Fixed::from_bits(32_212_254_720);
const LAYER_STEP: Fixed = Fixed::from_bits(572_662_306);
const K_MOON: Fixed = Fixed::from_bits(3_579_139_413);
const K_NEBULA: Fixed = Fixed::from_bits(2_147_483_648);
/// How far inside the weapon range a freighter starts working (the original's `approachRange - 15`).
const APPROACH_SLACK: i32 = 15;
/// Distance from a station's origin at which a beam-armed freighter starts to unload: the last
/// short stretch is the dock approach. The dock node itself (`dockingHardpoint`) is not read yet;
/// a fallback, not measured.
const DOCK_ARRIVE: i32 = 40;
/// A loaded freighter on a moon keeps circling while it is within this many ranges of the node.
const ORBIT_BAND: Fixed = Fixed::from_bits(6_442_450_944); // 1.5

/// Range of the freighter's mining weapon: the first hardpoint whose ordnance is a mining beam.
pub fn mining_range(db: &ContentDb, def: ContentKey) -> Option<Fixed> {
    db.unit(def)?.hardpoints.iter().find_map(|hp| {
        let w = db.weapon(hp.weapon.key)?;
        (w.class.0 == MINING_BEAM_CLASS && w.range > Fixed::ZERO).then_some(w.range)
    })
}

/// Distance from the node's centre at which a freighter starts to work: the surface gap under
/// `range - 15` when it has a mining beam, else 150 from the centre.
fn work_distance(db: &ContentDb, miner: ContentKey, node: ContentKey) -> Fixed {
    match mining_range(db, miner) {
        Some(r) => {
            let radii = db.unit(miner).map_or(Fixed::ZERO, |d| d.radius) + db.unit(node).map_or(Fixed::ZERO, |d| d.radius);
            (r - Fixed::from_int(APPROACH_SLACK) + radii).max(Fixed::ONE)
        }
        None => reach(),
    }
}

/// Where a mining freighter heads while it works: a point on the circle of the weapon's range
/// round the resource, `ANCHOR_PURSUIT_POINT_ANGLE` ahead of it (clockwise seen from above), on the
/// freighter's own height layer (`handle & 15`; `k` 5/6, a latinum nebula 1/2). Read from the
/// executable, static; the on-screen direction and speed are not measured.
fn orbit_point(db: &ContentDb, id: EntityId, pos: Vec3, node_def: ContentKey, node: Vec3, range: Fixed) -> Option<Vec3> {
    let (dx, dz) = (pos.x - node.x, pos.z - node.z);
    let len = (dx * dx + dz * dz).sqrt();
    if len < Fixed::ONE {
        return None;
    }
    let (c, s) = (PURSUIT_COS, PURSUIT_SIN);
    let (rx, rz) = (dx * c + dz * s, dz * c - dx * s);
    let k = range / len;
    let nd = db.unit(node_def);
    let radius = nd.map_or(Fixed::ZERO, |d| d.radius);
    let kk = if nd.is_some_and(|d| d.class.0 == "latinum") { K_NEBULA } else { K_MOON };
    let layer = Fixed::from_int((id.0 & 15) as i32) - LAYER_MID;
    let y = node.y + layer * LAYER_STEP * radius * kk;
    Some(Vec3 { x: node.x + rx * k, y, z: node.z + rz * k })
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
    personnel: Vec<(PlayerId, Option<Fixed>, Option<Fixed>)>,
    /// What is left in each node that has been mined.
    nodes: Table<Fixed>,
    miners: Table<Miner>,
    /// Miners whose work was ended or redirected by an order since the last step: announced
    /// there. Empty between steps, not part of the state.
    announce: Vec<EntityId>,
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
        self.stock.is_empty() && self.nodes.is_empty() && self.miners.is_empty() && self.personnel.is_empty()
    }

    /// A player's stock per resource id; empty until they have any.
    pub fn stock_of(&self, p: PlayerId) -> Vec<Fixed> {
        self.stock
            .iter()
            .find(|s| s.0 == p)
            .map(|s| s.1.to_vec())
            .unwrap_or_default()
    }

    /// Whether `id` has an active mining order.
    pub fn is_mining(&self, id: EntityId) -> bool {
        self.miners.get(id).is_some_and(|m| m.phase != Phase::Idle)
    }

    /// What `id` carries, including while idle.
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
        let Some(n) = units.get(node).filter(|n| n.kind == EntityKind::Resource
            && db.unit(n.def).is_none_or(EntityKind::mineable_def)) else {
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
            let cargo = old.map_or(Fixed::ZERO, |m| m.cargo);
            let phase = if cargo > Fixed::ZERO {
                Phase::ToDock
            } else {
                Phase::ToNode
            };
            if old.is_some_and(|m| m.phase != phase) {
                self.announce.push(id);
            }
            self.miners.insert(
                id,
                Miner {
                    node,
                    res: if cargo > Fixed::ZERO { old.unwrap().res } else { res },
                    cargo,
                    unloaded: Fixed::ZERO,
                    phase,
                },
            );
        }
        !ids.is_empty()
    }

    /// A move or stop suspends mining; carried cargo remains physical state.
    pub(crate) fn cancel(&mut self, units: &Entities, player: PlayerId, ids: &[EntityId]) {
        for &id in ids {
            if units
                .get(id)
                .is_some_and(|u| !player.is_player() || u.owner == player)
                && let Some(mut m) = self.miners.get(id).copied()
            {
                self.announce.push(id);
                if m.cargo > Fixed::ZERO {
                    m.phase = Phase::Idle;
                    m.unloaded = Fixed::ZERO;
                    self.miners.insert(id, m);
                } else {
                    self.miners.remove(id);
                }
            }
        }
    }

    /// The stations a freighter is unloading at right now (RND-21): the station each
    /// `Unloading` freighter hands its hold to, as [`Self::step`] chooses it (the nearest
    /// finished station of its owner, recomputed from the freighter's position, which does not
    /// change while it unloads).
    /// Derived from the miner table, so it is the same after a load; not state of its own.
    pub(crate) fn docked_stations(&self, units: &Entities) -> std::collections::BTreeSet<EntityId> {
        self.miners
            .iter()
            .filter(|(_, m)| m.phase == Phase::Unloading)
            .filter_map(|(id, _)| {
                let u = units.get(id)?;
                nearest_station_id(units, u.owner, u.pos)
            })
            .collect()
    }

    /// One tick of every freighter, ascending id.
    pub(crate) fn step(
        &mut self,
        units: &mut Entities,
        db: &ContentDb,
        events: &mut Vec<SimEvent>,
    ) {
        // Orders that took a freighter off its work (a stop, a move, a new order) say so.
        for id in std::mem::take(&mut self.announce) {
            let (phase, node) = self.miners.get(id).map_or((Phase::Idle, id), |m| (m.phase, m.node));
            events.push(SimEvent::MinerPhase { id, phase: phase.announced(), node, dock: None });
        }
        let ids: Vec<EntityId> = self.miners.iter().map(|(id, _)| id).collect();
        for id in ids {
            let Some(mut m) = self.miners.get(id).copied() else {
                continue;
            };
            let was = m.phase;
            let mut dock = None;
            let Some((def, owner)) = units.get(id).map(|u| (u.def, u.owner)) else {
                self.miners.remove(id);
                continue;
            };
            let Some(cfg) = cfg_of(db, def) else {
                self.miners.remove(id);
                continue;
            };
            let node_def = units.get(m.node).map_or(def, |n| n.def);
            let node_pos = units.get(m.node).filter(|n| n.kind == EntityKind::Resource
                && db.unit(n.def).is_none_or(EntityKind::mineable_def)).map(|n| n.pos);
            if matches!(m.phase, Phase::ToNode | Phase::Mining) && node_pos.is_none() {
                if m.cargo == Fixed::ZERO {
                    set_target(units, id, None);
                    self.miners.remove(id);
                    events.push(SimEvent::MinerPhase { id, phase: crate::event::MinerPhase::Idle, node: m.node, dock: None });
                    continue;
                }
                m.phase = Phase::ToDock;
            }
            // A transition that lands in reach acts the same tick.
            for _ in 0..4 {
                let Some(pos) = units.get(id).map(|u| u.pos) else {
                    break;
                };
                match m.phase {
                    Phase::Idle => break,
                    Phase::ToNode => {
                        if dist(pos, node_pos.unwrap()) <= work_distance(db, def, node_def) {
                            set_target(units, id, None);
                            m.phase = Phase::Mining;
                        } else {
                            steer(units, id, node_pos.unwrap());
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
                        // Round the resource, for as long as the beam works.
                        if let (Some(range), Some(np)) = (mining_range(db, def), node_pos) {
                            set_target(units, id, orbit_point(db, id, pos, node_def, np, range));
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
                            // No station: the orbit point of a beam-armed freighter must not outlive the
                            // work. A freighter without a beam keeps its target (as on main; no rule measured).
                            if mining_range(db, def).is_some() {
                                set_target(units, id, None);
                            }
                            break;
                        };
                        // Off a moon the freighter keeps circling until the station lies on its
                        // outer side, then peels off (read from the executable, static; the
                        // latinum nebula and planets leave at once).
                        // (Not when the station lies inside the orbit band, or the freighter sits on the
                        // node's centre: there is no outer side to wait for.)
                        if let (Some(range), Some(np)) = (mining_range(db, def), node_pos)
                            && db.unit(node_def).is_none_or(|d| d.class.0 != "latinum")
                            && dist(pos, np) <= range * ORBIT_BAND
                            && dist(at, np) > range * ORBIT_BAND
                            && (at.x - pos.x) * (pos.x - np.x) + (at.z - pos.z) * (pos.z - np.z) <= Fixed::ZERO
                            && let Some(p) = orbit_point(db, id, pos, node_def, np, range)
                        {
                            set_target(units, id, Some(p));
                            break;
                        }
                        // Arrival must lie beyond the smooth model's own "too close to turn" stop.
                        let arrive = if mining_range(db, def).is_some() {
                            Fixed::from_int(DOCK_ARRIVE).max(db.unit(def).map_or(Fixed::ZERO, |d| d.movement.too_close_to_turn) + Fixed::from_int(10))
                        } else {
                            reach()
                        };
                        if dist(pos, at) <= arrive {
                            set_target(units, id, None);
                            dock = Some(at);
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
                            if let Some(n) = units.get(m.node).filter(|n| n.kind == EntityKind::Resource
                                && db.unit(n.def).is_none_or(EntityKind::mineable_def)) {
                                m.res = node_resource(db, n.def);
                                m.phase = Phase::ToNode;
                            } else {
                                m.phase = Phase::Idle;
                            }
                        }
                        break;
                    }
                }
            }
            if m.phase != was {
                events.push(SimEvent::MinerPhase { id, phase: m.phase.announced(), node: m.node, dock });
            }
            if m.phase == Phase::Idle && m.cargo == Fixed::ZERO {
                self.miners.remove(id);
            } else {
                self.miners.insert(id, m);
            }
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
        self.hash_personnel(h);
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
    nearest_station_id(units, owner, from).and_then(|id| units.get(id)).map(|s| s.pos)
}

/// The finished station of `owner` nearest to `from` (ties: lowest id): where a freighter unloads.
fn nearest_station_id(units: &Entities, owner: PlayerId, from: Vec3) -> Option<EntityId> {
    units
        .iter()
        .filter(|s| {
            s.kind == EntityKind::Station
                && s.owner == owner
                && s.progress.is_none_or(|p| p >= Fixed::ONE)
        })
        .map(|s| (dist(from, s.pos), s.id, s.pos))
        .min_by_key(|&(d, id, _)| (d, id))
        .map(|(_, id, _)| id)
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod orbit_tests;
