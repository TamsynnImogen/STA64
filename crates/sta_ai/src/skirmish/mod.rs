//! The skirmish AI (AI-02): a computer player that mines, builds from the tech
//! tree and its AIP, defends its base and sends attack waves.
//!
//! It is an ordinary [`Controller`]: it reads a [`WorldView`] and the events
//! of the last tick, and answers with the same orders a player gives. It is
//! registered as [`SKIRMISH`] (`"skirmish"`).
//!
//! # What it knows
//!
//! - Units: only what `WorldView::visible_to(seat)` shows. Enemy ships and
//!   stations are seen when a sensor of its team reaches them, and not before.
//! - The map: start positions and where resource objects are placed. That is
//!   static data every player has in front of them in the lobby and on the
//!   minimap, so it comes in through [`SkirmishConfig`] rather than the view.
//!   Mining nodes are neutral entities, so a node's id is learned only when
//!   one of its units has seen it; until then a miner is sent to the place
//!   the map puts it.
//! - Its own build queues. The view does not show them, so the controller
//!   keeps its own account from the orders it sent, `Built` and `Rejected`.
//!
//! # What it does, every [`PERIOD`] ticks
//!
//! 1. **Economy.** Keeps [`Params::miners`] freighters mining the node that
//!    suits its stock, building them first.
//! 2. **Build order.** For each finished yard, picks from what the tech tree
//!    (or, with none, the yard's build list) allows: miners, then the AIP's
//!    `the_build_list` rows in order, then warships, the strongest it can
//!    pay for. Construction rigs deploy the stations the AIP asks for.
//! 3. **Defence.** [`Params::defence`] warships stay at the base; enemies
//!    near a station bring them and the waiting reinforcements out.
//! 4. **Attack waves.** When enough warships wait at the rally point they go
//!    out as a wave, to the nearest visible enemy, else to the enemy starts
//!    in turn. Each wave is a little larger than the last, up to the AIP's
//!    maximum.
//!
//! # Determinism
//!
//! Everything it computes is [`Fixed`] or integer. All collections are
//! ordered (`BTreeMap`, sorted `Vec`), the view arrives in ascending id, and
//! its one random choice draws from a splitmix64 stream seeded from the game
//! seed and the seat. The same views and events give the same commands.
//!
//! # Using it
//!
//! `PlayerSetup { control: Control::Ai("skirmish".into()), .. }`, then build
//! the controller for that seat with
//! [`Registry::create_for_seat`](crate::Registry::create_for_seat). A game
//! mode that wants an AIP calls [`SkirmishController::new`] with
//! [`SkirmishConfig::with_aip`] instead.

mod params;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sta_content::{Aip, ContentDb, Owned, WeaponKind};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Tick, Vec3};
use sta_sim::SimEvent;
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Controller, GameSetup};
use sta_sim::view::{PlayerView, Relation, UnitFlags, UnitView, WorldView};
use sta_sim::world::EntityKind;

pub use params::Params;

/// Registry name of the skirmish AI.
pub const SKIRMISH: &str = "skirmish";

/// Ticks between plans (half a second). Events are read every call.
pub const PERIOD: Tick = 15;
/// Orders in flight a yard may hold: one building and one waiting.
const MAX_QUEUED: usize = 2;
/// Enemies this close to home or a station are a threat.
const DEFEND_RADIUS: i32 = 1800;
/// Resources a miner is assumed to carry, for balancing nodes.
const MINER_LOAD: i32 = 150;
/// Most miners on one node.
const MINERS_PER_NODE: usize = 4;
/// Ticks after which an unanswered rig order is forgotten.
const RIG_TIMEOUT: Tick = 2700;
/// Ticks after which a sent order is no longer waiting for a rejection.
const SENT_KEEP: Tick = 300;
/// Distance from the base at which the garrison, then the rally point, sit.
const GUARD_DISTANCE: i32 = 400;
const RALLY_DISTANCE: i32 = 900;
/// A wave is within this of a goal when it has reached it.
const GOAL_REACHED: i32 = 600;
/// Engage range when a ship has no real weapon to measure.
const FALLBACK_RANGE: i32 = 300;

/// Everything a skirmish controller needs besides the view. The sim hands a
/// [`Controller`] no seat, content or map, so they come in here.
#[derive(Debug, Clone)]
pub struct SkirmishConfig {
    pub player: PlayerId,
    /// The seat's race, as the tech tree asks it (`unitedfederation`, ...).
    pub race: String,
    pub content: Arc<ContentDb>,
    /// The seat's start position.
    pub home: Vec3,
    /// Start positions of the seats on other teams, where waves look first.
    pub enemy_starts: Vec<Vec3>,
    /// Where the map places resource objects.
    pub nodes: Vec<Vec3>,
    pub seed: u64,
    pub params: Params,
}

impl SkirmishConfig {
    /// The config for `player` in `setup`: its race, start, the enemies'
    /// starts and the map's resource objects. Slot `i` of the map belongs to
    /// the `i`-th seat by id. `None` when the seat is not in the setup.
    pub fn from_setup(setup: &GameSetup, player: PlayerId) -> Option<SkirmishConfig> {
        let mut seats: Vec<_> = setup.players.iter().collect();
        seats.sort_by_key(|p| p.id);
        let index = seats.iter().position(|p| p.id == player)?;
        let me = seats[index];
        let start = |slot: usize| setup.map.starts.iter().find(|s| usize::from(s.slot) == slot).map(|s| s.pos);
        let home = start(index).or_else(|| setup.map.starts.get(index).map(|s| s.pos)).unwrap_or(Vec3::ZERO);
        let enemy_starts = seats
            .iter()
            .enumerate()
            .filter(|(_, p)| p.team != me.team)
            .filter_map(|(i, _)| start(i))
            .collect();
        let db = &setup.content;
        let nodes = setup
            .map
            .objects
            .iter()
            .filter(|o| {
                db.key(&o.odf)
                    .and_then(|k| db.unit(k))
                    .is_some_and(|d| EntityKind::of(&d.class, &d.movement) == EntityKind::Resource)
            })
            .map(|o| o.pos)
            .collect();
        Some(SkirmishConfig {
            player,
            race: me.race.clone(),
            content: Arc::clone(&setup.content),
            home,
            enemy_starts,
            nodes,
            seed: setup.seed,
            params: Params::default(),
        })
    }

    /// Tuned by an AIP (CON-14); see [`Params::from_aip`].
    pub fn with_aip(mut self, aip: &Aip) -> SkirmishConfig {
        self.params = Params::from_aip(aip);
        self
    }
}

/// What a unit is to the AI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// A station that builds ships or other units.
    Yard,
    /// A construction rig: builds stations where told.
    Rig,
    Miner,
    Warship,
    /// A station that builds nothing (turrets, sensors).
    Fort,
    Other,
}

fn role_of(db: &ContentDb, key: ContentKey) -> Role {
    let Some(d) = db.unit(key) else { return Role::Other };
    match EntityKind::of(&d.class, &d.movement) {
        EntityKind::Station if d.build.list.is_empty() => Role::Fort,
        EntityKind::Station => Role::Yard,
        EntityKind::Craft if d.class.0 == "constructionrig" => Role::Rig,
        EntityKind::Craft if d.class.0 == "freighter" || d.miner => Role::Miner,
        EntityKind::Craft if !d.hardpoints.is_empty() => Role::Warship,
        _ => Role::Other,
    }
}

fn is_station(db: &ContentDb, key: ContentKey) -> bool {
    db.unit(key).is_some_and(|d| EntityKind::of(&d.class, &d.movement) == EntityKind::Station)
}

fn cost_sum(db: &ContentDb, key: ContentKey) -> Fixed {
    db.unit(key).map_or(Fixed::ZERO, |d| d.build.cost.iter().fold(Fixed::ZERO, |a, &c| a + c))
}

/// How close a ship must get to use its guns: three quarters of its shortest
/// real weapon range, so every mount reaches.
fn engage_range(db: &ContentDb, key: ContentKey) -> Fixed {
    let shortest = db
        .unit(key)
        .into_iter()
        .flat_map(|d| d.hardpoints.iter())
        .filter_map(|h| db.weapon(h.weapon.key))
        .filter(|w| w.kind != WeaponKind::PointDefence && w.range > Fixed::ZERO)
        .map(|w| w.range)
        .min();
    shortest.map_or(Fixed::from_int(FALLBACK_RANGE), |r| r * Fixed::from_f64(0.75))
}

fn dist(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

/// The point `d` from `from` toward `to` (`from` itself when they coincide).
fn toward(from: Vec3, to: Vec3, d: Fixed) -> Vec3 {
    let len = dist(from, to);
    if len == Fixed::ZERO {
        return from;
    }
    Vec3 { x: from.x + (to.x - from.x) / len * d, y: from.y, z: from.z + (to.z - from.z) / len * d }
}

fn finished(u: &UnitView) -> bool {
    u.progress.is_none_or(|p| p >= Fixed::ONE)
}

/// Orders a unit may already be following, to avoid sending the same again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrderKey {
    Move(i64, i64),
    Attack(EntityId),
    Mine(EntityId),
}

/// An order sent and not yet known to have been accepted.
#[derive(Debug, Clone)]
struct Sent {
    tick: Tick,
    units: Vec<EntityId>,
    /// The yard or rig of a build order.
    builder: Option<EntityId>,
}

#[derive(Debug, Clone)]
struct Wave {
    members: BTreeSet<EntityId>,
    /// Index into the enemy starts of the place to look next.
    goal: usize,
}

#[derive(Debug, Clone, Copy)]
struct Own {
    id: EntityId,
    def: ContentKey,
    pos: Vec3,
    role: Role,
    finished: bool,
}

#[derive(Debug, Clone, Copy)]
struct Foe {
    id: EntityId,
    pos: Vec3,
    armed: bool,
}

/// The skirmish AI of one seat.
#[derive(Debug)]
pub struct SkirmishController {
    cfg: SkirmishConfig,
    rng: u64,
    seq: u32,
    /// Orders sent, by seq, until a rejection could no longer come.
    sent: BTreeMap<u32, Sent>,
    /// Build orders queued per yard, oldest first: `(seq, def)`.
    queued: BTreeMap<EntityId, Vec<(u32, ContentKey)>>,
    /// Rigs on their way to a site, with the def and when they were sent.
    rig_jobs: BTreeMap<EntityId, (ContentKey, Tick)>,
    sites_used: usize,
    /// What each unit is doing, as far as this controller ordered it.
    orders: BTreeMap<EntityId, OrderKey>,
    /// Mining nodes seen: position and resource id.
    nodes: BTreeMap<EntityId, (Vec3, u8)>,
    /// Which node each miner works.
    mining: BTreeMap<EntityId, EntityId>,
    waves: Vec<Wave>,
    wave_size: usize,
}

impl SkirmishController {
    pub fn new(cfg: SkirmishConfig) -> SkirmishController {
        // splitmix64 wants a nonzero-ish start; mix the seat in.
        let rng = cfg.seed ^ (u64::from(cfg.player.0) + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let wave_size = cfg.params.min_attack.max(1);
        SkirmishController {
            cfg,
            rng,
            seq: 0,
            sent: BTreeMap::new(),
            queued: BTreeMap::new(),
            rig_jobs: BTreeMap::new(),
            sites_used: 0,
            orders: BTreeMap::new(),
            nodes: BTreeMap::new(),
            mining: BTreeMap::new(),
            waves: Vec::new(),
            wave_size,
        }
    }

    /// For [`Registry::register_seat`](crate::Registry::register_seat).
    pub(crate) fn for_seat(setup: &GameSetup, player: PlayerId) -> Box<dyn Controller> {
        match SkirmishConfig::from_setup(setup, player) {
            Some(cfg) => Box::new(SkirmishController::new(cfg)),
            // A seat that is not in the setup has nothing to play.
            None => Box::new(crate::IdleController),
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Learn from the last tick's events (every call, whatever the period).
    fn absorb(&mut self, tick: Tick, events: &[SimEvent]) {
        let me = self.cfg.player;
        for e in events {
            match e {
                SimEvent::Rejected { player, seq } if *player == me => {
                    if let Some(s) = self.sent.remove(seq) {
                        for u in &s.units {
                            self.orders.remove(u);
                            self.mining.remove(u);
                        }
                        if let Some(b) = s.builder {
                            self.rig_jobs.remove(&b);
                            if let Some(q) = self.queued.get_mut(&b) {
                                q.retain(|(s, _)| s != seq);
                            }
                        }
                    }
                }
                SimEvent::Built { builder, .. } => {
                    if let Some(q) = self.queued.get_mut(builder).filter(|q| !q.is_empty()) {
                        q.remove(0);
                    }
                    self.rig_jobs.remove(builder);
                }
                // A rig that deployed is free again, though its weld goes on.
                SimEvent::BuildStarted { builder, .. } => {
                    self.rig_jobs.remove(builder);
                }
                _ => {}
            }
        }
        self.sent.retain(|_, s| tick.saturating_sub(s.tick) < SENT_KEEP);
        self.rig_jobs.retain(|_, (_, t)| tick.saturating_sub(*t) < RIG_TIMEOUT);
    }

    fn issue(&mut self, tick: Tick, out: &mut Vec<Command>, units: Vec<EntityId>, order: Order, key: Option<OrderKey>, builder: Option<EntityId>) -> u32 {
        let seq = self.seq;
        self.seq += 1;
        if let Some(k) = key {
            for &u in &units {
                self.orders.insert(u, k);
            }
        }
        self.sent.insert(seq, Sent { tick, units, builder });
        out.push(Command { player: self.cfg.player, seq, kind: CommandKind::Order(order) });
        seq
    }

    /// Send `unit` somewhere, unless it already has that order.
    fn go(&mut self, tick: Tick, out: &mut Vec<Command>, unit: EntityId, to: Vec3) {
        let key = OrderKey::Move(to.x.to_bits(), to.z.to_bits());
        if self.orders.get(&unit) != Some(&key) {
            self.issue(tick, out, vec![unit], Order::Move { units: vec![unit], to, queue: false }, Some(key), None);
        }
    }

    /// Bring `unit` to bear on a target: close in until its guns reach, then
    /// attack.
    fn engage(&mut self, tick: Tick, out: &mut Vec<Command>, unit: &Own, target: &Foe) {
        let range = engage_range(&self.cfg.content, unit.def);
        if dist(unit.pos, target.pos) <= range {
            let key = OrderKey::Attack(target.id);
            if self.orders.get(&unit.id) != Some(&key) {
                let o = Order::Attack { units: vec![unit.id], target: target.id };
                self.issue(tick, out, vec![unit.id], o, Some(key), None);
            }
        } else {
            self.go(tick, out, unit.id, toward(target.pos, unit.pos, range * Fixed::from_f64(0.6)));
        }
    }

    fn plan(&mut self, view: &WorldView, me: &PlayerView, out: &mut Vec<Command>) {
        let db = Arc::clone(&self.cfg.content);
        let seat = self.cfg.player;
        let mut own: Vec<Own> = Vec::new();
        let mut foes: Vec<Foe> = Vec::new();
        for u in view.visible_to(seat) {
            if u.flags.contains(UnitFlags::DYING) {
                continue;
            }
            if u.owner == seat {
                own.push(Own { id: u.id, def: u.def, pos: u.pos, role: role_of(&db, u.def), finished: finished(u) });
            } else if me.relations.iter().any(|&(p, r)| p == u.owner && r == Relation::Enemy) {
                let role = role_of(&db, u.def);
                if role != Role::Other || is_station(&db, u.def) {
                    foes.push(Foe { id: u.id, pos: u.pos, armed: role == Role::Warship });
                }
            } else if view.player(u.owner).is_none() {
                // Neutral: a mining node, once somebody's sensors reach it.
                if let Some(d) = db.unit(u.def).filter(|d| EntityKind::of(&d.class, &d.movement) == EntityKind::Resource) {
                    let res = if d.class.0 == "latinum" { 1 } else { d.resource.unwrap_or(0) };
                    self.nodes.insert(u.id, (u.pos, res));
                }
            }
        }
        let ids: BTreeSet<EntityId> = own.iter().map(|o| o.id).collect();
        self.orders.retain(|id, _| ids.contains(id));
        let known = &self.nodes;
        self.mining.retain(|id, node| ids.contains(id) && known.contains_key(node));
        self.queued.retain(|id, _| ids.contains(id));
        self.rig_jobs.retain(|id, _| ids.contains(id));
        self.waves.iter_mut().for_each(|w| w.members.retain(|m| ids.contains(m)));
        self.waves.retain(|w| !w.members.is_empty());

        let stations: Vec<Vec3> = own.iter().filter(|o| matches!(o.role, Role::Yard | Role::Fort)).map(|o| o.pos).collect();
        let home = if stations.is_empty() {
            self.cfg.home
        } else {
            let n = Fixed::from_int(stations.len() as i32);
            let sum = stations.iter().fold(Vec3::ZERO, |a, p| Vec3 { x: a.x + p.x, y: a.y + p.y, z: a.z + p.z });
            Vec3 { x: sum.x / n, y: sum.y / n, z: sum.z / n }
        };

        self.economy(view.tick, me, &own, out);
        self.mine(view.tick, me, &own, home, out);
        self.military(view.tick, &own, &foes, &stations, home, out);
    }

    /// Orders for the builders: yards queue units, rigs deploy stations.
    fn economy(&mut self, tick: Tick, me: &PlayerView, own: &[Own], out: &mut Vec<Command>) {
        let db = Arc::clone(&self.cfg.content);
        let rows = self.cfg.params.build_list.clone();
        let mut total: BTreeMap<ContentKey, usize> = BTreeMap::new();
        let mut state = Owned::new(&self.cfg.race);
        let mut done: BTreeMap<ContentKey, u32> = BTreeMap::new();
        for o in own {
            *total.entry(o.def).or_default() += 1;
            if o.finished {
                *done.entry(o.def).or_default() += 1;
            }
        }
        for (&k, &n) in &done {
            if let Some(name) = db.name(k) {
                state = state.with(name, n);
            }
        }
        for q in self.queued.values() {
            for &(_, k) in q {
                *total.entry(k).or_default() += 1;
            }
        }
        for &(k, _) in self.rig_jobs.values() {
            *total.entry(k).or_default() += 1;
        }
        let count_role = |total: &BTreeMap<ContentKey, usize>, role: Role| -> usize {
            total.iter().filter(|(k, _)| role_of(&db, **k) == role).map(|(_, n)| *n).sum()
        };

        let mut reserve = vec![Fixed::ZERO; me.resources.len().max(4)];
        // Unless the game charges on queueing, what waits behind a yard's head
        // item is not paid for yet: keep that money out of reach.
        if !db.features().pay_on_queue {
            for q in self.queued.values() {
                for &(_, k) in q.iter().skip(1) {
                    if let Some(d) = db.unit(k) {
                        for (i, &c) in d.build.cost.iter().enumerate() {
                            if i >= reserve.len() {
                                reserve.resize(i + 1, Fixed::ZERO);
                            }
                            reserve[i] = reserve[i] + c;
                        }
                    }
                }
            }
        }
        let affordable = |def: ContentKey, reserve: &[Fixed]| -> bool {
            db.unit(def).is_some_and(|d| {
                d.build.cost.iter().enumerate().all(|(i, &c)| {
                    c <= Fixed::ZERO
                        || me.resources.get(i).copied().unwrap_or(Fixed::ZERO) - reserve.get(i).copied().unwrap_or(Fixed::ZERO) >= c
                })
            })
        };
        let spend = |def: ContentKey, reserve: &mut Vec<Fixed>| {
            if let Some(d) = db.unit(def) {
                for (i, &c) in d.build.cost.iter().enumerate() {
                    if i >= reserve.len() {
                        reserve.resize(i + 1, Fixed::ZERO);
                    }
                    reserve[i] = reserve[i] + c;
                }
            }
        };
        let buildable = |builder: ContentKey| -> Vec<ContentKey> {
            let Some(u) = db.unit(builder) else { return Vec::new() };
            match db.tech_tree() {
                Some(tree) => tree
                    .build_options(&db, &u.odf, &state)
                    .into_iter()
                    .filter(|o| o.availability.is_available())
                    .map(|o| o.def.key)
                    .filter(|&k| k != ContentKey::NONE)
                    .collect(),
                None => u.build.list.iter().map(|d| d.key).filter(|&k| k != ContentKey::NONE).collect(),
            }
        };
        // How many of `key` the AIP's list wants in all, if it lists it.
        let list_limit = |key: ContentKey| -> Option<usize> {
            let name = db.name(key)?;
            rows.iter().find(|r| sta_content::techtree::stem(&r.unit) == name).map(|r| match r.max.filter(|&m| m > 0) {
                Some(m) => m as usize,
                None => ((r.count.to_bits() >> 32).max(1)) as usize,
            })
        };

        // Short of miners, and one can be built: save for them, spend nothing
        // on ships. Income is what wins long games.
        let saving = count_role(&total, Role::Miner) < self.cfg.params.miners
            && own.iter().any(|o| o.role == Role::Yard && o.finished && buildable(o.def).iter().any(|&k| role_of(&db, k) == Role::Miner));

        // Yards, ascending id.
        for y in own.iter().filter(|o| o.role == Role::Yard && o.finished) {
            if self.queued.get(&y.id).map_or(0, Vec::len) >= MAX_QUEUED {
                continue;
            }
            let opts = buildable(y.def);
            let miners = count_role(&total, Role::Miner);
            let armies = count_role(&total, Role::Warship);
            let mut want: Option<ContentKey> = None;
            if miners < self.cfg.params.miners {
                want = opts.iter().copied().filter(|&k| role_of(&db, k) == Role::Miner).min_by_key(|&k| (cost_sum(&db, k), k));
            }
            if want.is_none() {
                // The AIP's rows, in order; stations are the rigs' business.
                want = rows
                    .iter()
                    .filter_map(|r| db.key(&sta_content::techtree::stem(&r.unit)))
                    .find(|&k| {
                        opts.contains(&k)
                            && !is_station(&db, k)
                            && role_of(&db, k) != Role::Miner
                            && list_limit(k).is_some_and(|lim| total.get(&k).copied().unwrap_or(0) < lim)
                    });
            }
            if want.is_none() && !saving && armies < params::ARMY_CAP {
                let mut ships: Vec<ContentKey> = opts.iter().copied().filter(|&k| role_of(&db, k) == Role::Warship).collect();
                // Strongest (dearest) first; the cheapest when nothing is affordable, to save.
                ships.sort_by_key(|&k| (std::cmp::Reverse(cost_sum(&db, k)), k));
                let pay: Vec<ContentKey> = ships.iter().copied().filter(|&k| affordable(k, &reserve)).collect();
                want = match pay.len() {
                    0 => ships.last().copied(),
                    // One time in four, anything affordable: a varied fleet.
                    n if n > 1 && self.next_u64().is_multiple_of(4) => Some(pay[(self.next_u64() % n as u64) as usize]),
                    _ => pay.first().copied(),
                };
            }
            let Some(what) = want.filter(|&k| affordable(k, &reserve)) else { continue };
            spend(what, &mut reserve);
            *total.entry(what).or_default() += 1;
            let seq = self.issue(tick, out, Vec::new(), Order::Build { builder: y.id, what, at: None }, None, Some(y.id));
            self.queued.entry(y.id).or_default().push((seq, what));
        }

        // Rigs deploy the stations the AIP lists that are not there yet.
        let idle_rigs: Vec<&Own> = own.iter().filter(|o| o.role == Role::Rig && o.finished && !self.rig_jobs.contains_key(&o.id)).collect();
        for r in idle_rigs {
            let opts = buildable(r.def);
            let want = rows
                .iter()
                .filter_map(|row| db.key(&sta_content::techtree::stem(&row.unit)))
                .find(|&k| {
                    opts.contains(&k) && is_station(&db, k) && list_limit(k).is_some_and(|lim| total.get(&k).copied().unwrap_or(0) < lim)
                });
            let Some(what) = want.filter(|&k| affordable(k, &reserve)) else { continue };
            spend(what, &mut reserve);
            *total.entry(what).or_default() += 1;
            let at = self.site(self.sites_used);
            self.sites_used += 1;
            self.issue(tick, out, vec![r.id], Order::Build { builder: r.id, what, at: Some(at) }, None, Some(r.id));
            self.rig_jobs.insert(r.id, (what, tick));
        }
    }

    /// The `n`th building site around the start: a hexagon, then wider rings.
    fn site(&self, n: usize) -> Vec3 {
        const DIRS: [(f64, f64); 6] = [(1.0, 0.0), (0.5, 0.866), (-0.5, 0.866), (-1.0, 0.0), (-0.5, -0.866), (0.5, -0.866)];
        let (dx, dz) = DIRS[n % 6];
        let r = Fixed::from_int(500 + 300 * (n / 6) as i32);
        let h = self.cfg.home;
        Vec3 { x: h.x + Fixed::from_f64(dx) * r, y: h.y, z: h.z + Fixed::from_f64(dz) * r }
    }

    /// Miners go to nodes; until a node has been seen they go where the map
    /// puts one.
    fn mine(&mut self, tick: Tick, me: &PlayerView, own: &[Own], home: Vec3, out: &mut Vec<Command>) {
        let stock = |res: u8| me.resources.get(res as usize).copied().unwrap_or(Fixed::ZERO);
        for m in own.iter().filter(|o| o.role == Role::Miner && o.finished) {
            if self.mining.contains_key(&m.id) {
                continue;
            }
            if self.nodes.is_empty() {
                let spot = self.cfg.nodes.iter().copied().min_by_key(|&p| (dist(home, p), p.x, p.z));
                if let Some(p) = spot {
                    self.go(tick, out, m.id, p);
                }
                continue;
            }
            let load = |node: EntityId| self.mining.values().filter(|&&n| n == node).count();
            let res_load = |res: u8| {
                let on_res = self.mining.values().filter(|n| self.nodes.get(n).is_some_and(|x| x.1 == res)).count();
                stock(res) + Fixed::from_int(MINER_LOAD * on_res as i32)
            };
            let best = self
                .nodes
                .iter()
                .filter(|(id, _)| load(**id) < MINERS_PER_NODE)
                .min_by_key(|(id, v)| (res_load(v.1), dist(home, v.0), **id))
                .map(|(&id, _)| id);
            if let Some(node) = best {
                self.mining.insert(m.id, node);
                self.issue(tick, out, vec![m.id], Order::Mine { units: vec![m.id], node }, Some(OrderKey::Mine(node)), None);
            }
        }
    }

    /// Defence and attack waves.
    fn military(&mut self, tick: Tick, own: &[Own], foes: &[Foe], stations: &[Vec3], home: Vec3, out: &mut Vec<Command>) {
        let ships: Vec<Own> = own.iter().filter(|o| o.role == Role::Warship).copied().collect();
        let in_wave: BTreeSet<EntityId> = self.waves.iter().flat_map(|w| w.members.iter().copied()).collect();
        let free: Vec<Own> = ships.iter().filter(|s| !in_wave.contains(&s.id)).copied().collect();
        let keep = self.cfg.params.defence.min(free.len());
        let (guard, spare) = free.split_at(keep);

        // Which way is the enemy: the nearest of its starts.
        let starts = self.cfg.enemy_starts.clone();
        let nearest = starts.iter().copied().min_by_key(|&p| (dist(home, p), p.x, p.z));
        let (guard_at, rally_at) = match nearest {
            Some(p) => (toward(home, p, Fixed::from_int(GUARD_DISTANCE)), toward(home, p, Fixed::from_int(RALLY_DISTANCE))),
            None => (home, home),
        };

        let radius = Fixed::from_int(DEFEND_RADIUS);
        let threatened = |f: &&Foe| dist(f.pos, home) <= radius || stations.iter().any(|&s| dist(f.pos, s) <= radius);
        let threats: Vec<&Foe> = foes.iter().filter(threatened).collect();
        if threats.is_empty() {
            for g in guard {
                self.go(tick, out, g.id, guard_at);
            }
            for s in spare {
                self.go(tick, out, s.id, rally_at);
            }
            let size = self.wave_size.min(self.cfg.params.max_attack).max(1);
            if spare.len() >= size && !starts.is_empty() {
                let members: BTreeSet<EntityId> = spare.iter().take(size).map(|s| s.id).collect();
                self.waves.push(Wave { members, goal: 0 });
                self.wave_size = (size + 1).min(self.cfg.params.max_attack);
            }
        } else {
            for s in guard.iter().chain(spare) {
                if let Some(t) = threats.iter().min_by_key(|f| (dist(s.pos, f.pos), f.id)) {
                    let t = **t;
                    self.engage(tick, out, s, &t);
                }
            }
        }

        let mut waves = std::mem::take(&mut self.waves);
        for w in &mut waves {
            let crew: Vec<Own> = ships.iter().filter(|s| w.members.contains(&s.id)).copied().collect();
            let n = Fixed::from_int(crew.len().max(1) as i32);
            let sum = crew.iter().fold((Fixed::ZERO, Fixed::ZERO), |a, s| (a.0 + s.pos.x, a.1 + s.pos.z));
            let centre = Vec3 { x: sum.0 / n, y: Fixed::ZERO, z: sum.1 / n };
            // Armed enemies first, then anything, nearest to the wave.
            let target = foes.iter().min_by_key(|f| (!f.armed, dist(centre, f.pos), f.id)).copied();
            match target {
                Some(t) => crew.iter().for_each(|s| self.engage(tick, out, s, &t)),
                None => {
                    if !starts.is_empty() {
                        let goal = starts[w.goal % starts.len()];
                        if dist(centre, goal) <= Fixed::from_int(GOAL_REACHED) {
                            w.goal += 1;
                        }
                        crew.iter().for_each(|s| self.go(tick, out, s.id, goal));
                    }
                }
            }
        }
        self.waves = waves;
    }
}

impl Controller for SkirmishController {
    fn think(&mut self, view: &WorldView, events: &[SimEvent], out: &mut Vec<Command>) {
        self.absorb(view.tick, events);
        let Some(me) = view.player(self.cfg.player) else { return };
        if me.eliminated || !(view.tick + Tick::from(self.cfg.player.0)).is_multiple_of(PERIOD) {
            return;
        }
        self.plan(view, me, out);
    }
}

#[cfg(test)]
mod tests;
