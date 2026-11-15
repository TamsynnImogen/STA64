//! The transporter order, crew transfer, boarding and capture (SIM-46 P1).
//!
//! Armada II family only: a transport is accepted when the carrier's resolved
//! features have `crew_loss_on_hit` (stock A2, Classic, FO 3.x, Roots), because
//! the derelict latch the capture rules lean on exists only there (Native and
//! Armada I are refused; the A1 rule is not read yet). Planets, boarding
//! specials, Roots/FO deltas, hold fire and the assault attack mode are not
//! here: `Order::Transport { attack: true }` is refused.
//!
//! Rules, evidence **M** = measured on stock Armada II (bench, 2026-11-15),
//! **S** = read from the executable (static), **D** = read from data:
//! - The first pulse is on the order tick; one pulse per `TRANSPORTER_DELAY`;
//!   each takes `TRANSPORTER_MAX` crew from the carrier (M).
//! - In range when the centre distance is at most `TRANSPORTER_RANGE` plus both
//!   radii (M). Out of range, or an enemy whose shields are not down, the order
//!   is taken and nothing happens: no approach, no beam (M).
//! - Friend (anyone who is not an enemy seat): the party joins at once; crew
//!   above the target's maximum is lost; stops when the target is full (M).
//! - Enemy (a derelict too): shields must be at most 0.01 (M); each party arrives
//!   `TRANSPORTER_ACTION_DELAY` after its pulse (M). Arrival on a derelict adds
//!   the party and captures it (M). Otherwise the boarding fight:
//!   `kills = size x A / D`, `D = defender multiplier x race strength x 1.5`;
//!   capture when `kills > crew` (a tie leaves a derelict that keeps its owner)
//!   with `survivors = (kills - crew) x 0.3` (M); the survivors become the crew.
//! - Capture changes owner and race on one tick, the order becomes Stop and the
//!   perceived owner follows (M); the carrier goes on transporting to it.
//! - The carrier stops when `(crew - max) / maxCrew <= crewRedStatus` after a
//!   pulse (M); its shields go to 0 at the first pulse and come back 3.3 s
//!   after the last (M); the target's recharge delay is set to 3.0 s on every
//!   pulse (M).
//!
//! **Content seam.** The transporter and boarding keys (`TRANSPORTER_*`,
//! `boardingPartyStrength`, race `boardingStrength`, `shieldDelay`,
//! `disableShieldsOnTransport`, `transporter`, `canCommandeer`...) are not
//! typed in `sta_content` yet (SIM-46 P0). [`TransportRules`] and the four
//! small functions below hold the stock values until they are; the only race
//! key read from content is `crewRedStatus` (with its exe default).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, Tick, Vec3};

use super::casualty::{Ctx, kill_crew};
use crate::relations::Diplomacy;
use crate::view::UnitFlags;
use crate::world::{Entity, EntityKind, Fnv};
use crate::Sim;

/// 0.01: an enemy with at most this much shield is "shields down" (S, M).
const SHIELDS_DOWN: Fixed = Fixed::from_bits(42_949_673);
/// 1.5, the defender's factor of the boarding fight (S, M).
const DEFENCE_FACTOR: Fixed = Fixed::from_bits(6_442_450_944);
/// 0.3, the share of the surplus kills that lives on as crew (M).
const SURVIVOR_FACTOR: Fixed = Fixed::from_bits(1_288_490_189);
/// 0.25, the executable's default `crewRedStatus`.
const DEFAULT_CREW_RED: Fixed = Fixed::from_bits(1 << 30);

/// The stock transporter numbers, until P0's content keys carry them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransportRules {
    /// `TRANSPORTER_RANGE`, surface to surface (stock A2 500).
    pub range: Fixed,
    /// `TRANSPORTER_MAX`: the party size and the carrier's loss per pulse (5).
    pub party: Fixed,
    /// `TRANSPORTER_DELAY` in ticks (1.0 s).
    pub delay: Tick,
    /// `TRANSPORTER_ACTION_DELAY` in ticks: an enemy party's flight (1.0 s).
    pub action_delay: Tick,
    /// The carrier needs more crew than this to start (9, exe constant, S).
    pub min_crew: Fixed,
    /// The target's shield recharge delay set per pulse: `shieldDelay` (3.0 s, M).
    pub target_shield_hold: Tick,
    /// How long after a pulse the carrier's shields stay off (3.3 s, M).
    pub carrier_shield_return: Tick,
}

impl TransportRules {
    /// STUB: stock Armada II values; P0 replaces this with the rules read from the content.
    pub(crate) fn of(_db: &ContentDb) -> TransportRules {
        TransportRules {
            range: Fixed::from_int(500),
            party: Fixed::from_int(5),
            delay: TICK_HZ as Tick,
            action_delay: TICK_HZ as Tick,
            min_crew: Fixed::from_int(9),
            target_shield_hold: 3 * TICK_HZ as Tick,
            carrier_shield_return: 99,
        }
    }
}

/// STUB (`transporter`, default true on `craft.odf`): crewed ships and stations.
fn can_transport(def: &UnitDef, crew_max: Fixed) -> bool {
    def.planet_role.is_none() && crew_max > Fixed::ZERO
}

/// STUB (`boardingPartyStrength`, default 1.0).
fn boarding_multiplier(_def: &UnitDef) -> Fixed {
    Fixed::ONE
}

/// STUB (race `boardingStrength`, default 1.0).
fn race_strength(_db: &ContentDb, _race: &str) -> Fixed {
    Fixed::ONE
}

/// STUB (race `canBeCommandeered`, default true; stock A2 sets 0 on one race).
fn can_be_commandeered(_db: &ContentDb, _race: &str) -> bool {
    true
}

/// STUB (race `canCommandeer`, default true; stock A2 sets 0 on one race).
fn can_commandeer(_db: &ContentDb, _race: &str) -> bool {
    true
}

/// STUB (`disableShieldsOnTransport`, default true).
fn drops_shields(_def: &UnitDef) -> bool {
    true
}

/// The carrier race's `crewRedStatus` (data), the executable's 0.25 when absent.
fn crew_red_status(db: &ContentDb, race: &str) -> Fixed {
    db.rules().race_repair(race).and_then(|r| r.crew_red_status).unwrap_or(DEFAULT_CREW_RED)
}

/// One carrier's transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportRow {
    pub target: EntityId,
    pub next_pulse: Tick,
}

/// A boarding party in flight to an enemy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Party {
    pub arrive: Tick,
    pub seq: u32,
    pub target: EntityId,
    pub size: Fixed,
    /// `boardingStrength` of the race times the carrier's `boardingPartyStrength`.
    pub strength: Fixed,
    /// The boarders' race (the carrier's at the pulse); empty when it had none.
    pub race: String,
    pub team: PlayerId,
    /// The carrier; a party whose carrier is gone is lost (S).
    pub source: EntityId,
}

/// The transporter state; in the digest and the snapshot once any exists.
#[derive(Debug, Clone, Default)]
pub struct Transports {
    rows: BTreeMap<EntityId, TransportRow>,
    /// Ascending `(arrive, seq)`.
    parties: Vec<Party>,
    next_seq: u32,
    /// Shield recharge held until this tick (exclusive): targets and carriers.
    pub(crate) holds: BTreeMap<EntityId, Tick>,
}

/// A party as a snapshot row: arrive, seq, target, size bits, strength bits, race, team, source.
pub type PartyRow = (Tick, u32, EntityId, i64, i64, String, PlayerId, EntityId);

/// The snapshot form of [`Transports`] (format 29).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportsRec {
    pub version: u8,
    /// `(carrier, target, next pulse)`, ascending carrier.
    pub rows: Vec<(EntityId, EntityId, Tick)>,
    /// `(arrive, seq, target, size bits, strength bits, race, team, source)`, ascending `(arrive, seq)`.
    pub parties: Vec<PartyRow>,
    pub next_seq: u32,
    /// `(entity, held until)`, ascending entity.
    pub holds: Vec<(EntityId, Tick)>,
}

fn alive(e: &Entity) -> bool {
    e.hull[0] > Fixed::ZERO && !e.flags.contains(UnitFlags::DYING)
}

/// Centre distance at most `reach`, in 128-bit arithmetic (no square root).
fn within(a: Vec3, b: Vec3, reach: Fixed) -> bool {
    let d = |p: Fixed, q: Fixed| i128::from(p.to_bits()) - i128::from(q.to_bits());
    let (dx, dy, dz) = (d(a.x, b.x), d(a.y, b.y), d(a.z, b.z));
    let r = i128::from(reach.to_bits().max(0));
    dx * dx + dy * dy + dz * dz <= r * r
}

/// Total shield strength of a unit.
fn shield_total(e: &Entity) -> Fixed {
    e.shields.now.iter().fold(Fixed::ZERO, |a, &s| a + s)
}

impl Transports {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.parties.is_empty() && self.holds.is_empty()
    }

    /// The target a carrier is transporting to.
    pub fn target_of(&self, carrier: EntityId) -> Option<EntityId> {
        self.rows.get(&carrier).map(|r| r.target)
    }

    /// Parties on their way to a target.
    pub fn incoming(&self, target: EntityId) -> usize {
        self.parties.iter().filter(|p| p.target == target).count()
    }

    /// Whether a unit's shields may not recharge now.
    pub(crate) fn shields_held(&self, id: EntityId) -> bool {
        self.holds.contains_key(&id)
    }

    /// Holds that ended by `tick` are dropped (before the recharge pass).
    pub(crate) fn expire(&mut self, tick: Tick) {
        self.holds.retain(|_, until| *until > tick);
    }

    /// A carrier's transport ends (a new order, a change of owner).
    pub(crate) fn end(&mut self, carrier: EntityId) {
        self.rows.remove(&carrier);
    }

    /// An entity left the world.
    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        self.rows.remove(&id);
        self.rows.retain(|_, r| r.target != id);
        self.parties.retain(|p| p.target != id);
        self.holds.remove(&id);
        self.reset_seq();
    }

    fn reset_seq(&mut self) {
        if self.parties.is_empty() {
            self.next_seq = 0;
        }
    }

    fn send(&mut self, mut party: Party) {
        party.seq = self.next_seq;
        self.next_seq += 1;
        let at = self.parties.partition_point(|p| (p.arrive, p.seq) <= (party.arrive, party.seq));
        self.parties.insert(at, party);
    }

    pub(crate) fn to_rec(&self) -> TransportsRec {
        TransportsRec {
            version: 1,
            rows: self.rows.iter().map(|(c, r)| (*c, r.target, r.next_pulse)).collect(),
            parties: self
                .parties
                .iter()
                .map(|p| (p.arrive, p.seq, p.target, p.size.to_bits(), p.strength.to_bits(), p.race.clone(), p.team, p.source))
                .collect(),
            next_seq: self.next_seq,
            holds: self.holds.iter().map(|(id, t)| (*id, *t)).collect(),
        }
    }

    /// Rebuild from a snapshot record: ascending ids and order keys, entities that exist, sane sizes.
    pub(crate) fn from_rec(rec: TransportsRec, exists: impl Fn(EntityId) -> bool) -> Option<Transports> {
        if rec.version != 1
            || !rec.rows.windows(2).all(|w| w[0].0 < w[1].0)
            || !rec.holds.windows(2).all(|w| w[0].0 < w[1].0)
            || !rec.parties.windows(2).all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1))
        {
            return None;
        }
        const LIMIT: i64 = 1_000_000i64 << 32;
        let rows_ok = rec.rows.iter().all(|r| exists(r.0) && exists(r.1) && r.0 != r.1);
        let parties_ok = rec.parties.iter().all(|p| p.1 < rec.next_seq && exists(p.2) && (1..=LIMIT).contains(&p.3) && (0..=LIMIT).contains(&p.4));
        let holds_ok = rec.holds.iter().all(|h| exists(h.0));
        if !(rows_ok && parties_ok && holds_ok) || (rec.parties.is_empty() && rec.next_seq != 0) {
            return None;
        }
        Some(Transports {
            rows: rec.rows.into_iter().map(|(c, target, next_pulse)| (c, TransportRow { target, next_pulse })).collect(),
            parties: rec
                .parties
                .into_iter()
                .map(|(arrive, seq, target, size, strength, race, team, source)| Party {
                    arrive,
                    seq,
                    target,
                    size: Fixed::from_bits(size),
                    strength: Fixed::from_bits(strength),
                    race,
                    team,
                    source,
                })
                .collect(),
            next_seq: rec.next_seq,
            holds: rec.holds.into_iter().collect(),
        })
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("transports-v1");
        h.u64(self.rows.len() as u64);
        for (c, r) in &self.rows {
            h.u64(u64::from(c.0));
            h.u64(u64::from(r.target.0));
            h.u64(r.next_pulse);
        }
        h.u64(self.parties.len() as u64);
        for p in &self.parties {
            h.u64(p.arrive);
            h.u64(u64::from(p.seq));
            h.u64(u64::from(p.target.0));
            h.fixed(p.size);
            h.fixed(p.strength);
            h.str(&p.race);
            h.u64(u64::from(p.team.0));
            h.u64(u64::from(p.source.0));
        }
        h.u64(u64::from(self.next_seq));
        h.u64(self.holds.len() as u64);
        for (id, until) in &self.holds {
            h.u64(u64::from(id.0));
            h.u64(*until);
        }
    }

    /// A sample with a transport, a party in flight and two shield holds (digest pin).
    #[cfg(test)]
    pub(crate) fn sample_for_test() -> Transports {
        let (carrier, target) = (EntityId(2), EntityId(1));
        let mut t = Transports::default();
        t.rows.insert(carrier, TransportRow { target, next_pulse: 120 });
        t.send(Party { arrive: 135, seq: 0, target, size: Fixed::from_int(5), strength: Fixed::ONE, race: "federation".into(), team: PlayerId(0), source: carrier });
        t.holds.insert(carrier, 219);
        t.holds.insert(target, 210);
        t
    }
}

impl Sim {
    /// The transporter state (read-only).
    pub fn transports(&self) -> &Transports {
        &self.transports
    }

    /// `Order::Transport`: see the module docs. The order is taken for a unit that
    /// is a transporter with more than `min_crew`, whatever the range or the
    /// target's shields (the pulse tests those); false when no unit could take it.
    pub(crate) fn apply_transport(&mut self, player: PlayerId, carriers: &[EntityId], target: EntityId, attack: bool) -> bool {
        let Some(db) = self.content.clone() else { return false };
        if attack {
            return false; // P3: transport attack (Roots, assault ships).
        }
        let rules = TransportRules::of(&db);
        let Some(t) = self.units.get(target).filter(|t| alive(t)) else { return false };
        let target_ok = matches!(t.kind, EntityKind::Craft | EntityKind::Station)
            && t.crew.is_some_and(|c| c[1] >= Fixed::ONE)
            && db.unit(t.def).is_some_and(|d| d.planet_role.is_none());
        if !target_ok || !can_be_commandeered(&db, self.unit_race(target).unwrap_or("")) {
            return false;
        }
        let target_owner = t.owner;
        let mut ids: Vec<EntityId> = carriers
            .iter()
            .copied()
            .filter(|&id| {
                id != target
                    && !self.production.is_welding(id)
                    && self.units.get(id).filter(|u| alive(u)).is_some_and(|u| {
                        let def = db.unit(u.def);
                        matches!(u.kind, EntityKind::Craft | EntityKind::Station)
                            // The carrier's race must be allowed to take units of another team.
                            && (self.same_team(u.owner, target_owner) || can_commandeer(&db, self.unit_race(id).unwrap_or("")))
                            && (!player.is_player() || u.owner == player)
                            && def.is_some_and(|d| db.unit_features(d).crew_loss_on_hit && can_transport(d, u.crew.map_or(Fixed::ZERO, |c| c[1])))
                            && u.crew.is_some_and(|c| c[0] > rules.min_crew && c[0] > rules.party)
                    })
            })
            .collect();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return false;
        }
        // Like every order that replaces what a unit is doing; it does not move.
        self.weapons.stop_attack(&self.units, player, &ids);
        self.economy.cancel(&self.units, player, &ids);
        self.production.cancel(&db, &self.units, &mut self.economy, player, &ids);
        self.clear_scripted(player, &ids);
        for &id in &ids {
            self.watchdog.remove(id);
            self.movement.clear_waypoints(id);
            self.set_target(id, None);
            self.colonies.end_process(id, false);
            self.transports.rows.insert(id, TransportRow { target, next_pulse: self.tick });
        }
        true
    }

    /// What other orders do to a transporting carrier: they end its transport.
    pub(crate) fn transports_after_order(&mut self, player: PlayerId, order: &crate::command::Order) {
        use crate::command::Order;
        if self.transports.rows.is_empty() {
            return;
        }
        let units: &[EntityId] = match order {
            Order::Stop { units }
            | Order::Move { units, queue: false, .. }
            | Order::MoveWarp { units, .. }
            | Order::Attack { units, .. }
            | Order::Mine { units, .. }
            | Order::Patrol { units, .. }
            | Order::SearchAndDestroy { units }
            | Order::Scout { units }
            | Order::AttackAny { units }
            | Order::AttackPoint { units, .. }
            | Order::Colonize { units, .. } => units,
            Order::Build { builder, .. } => std::slice::from_ref(builder),
            _ => return,
        };
        for &id in units {
            if self.units.get(id).is_some_and(|u| !player.is_player() || u.owner == player) {
                self.transports.end(id);
            }
        }
    }

    /// One tick: every carrier's due pulse (ascending id), then the parties that arrive (in order).
    pub(crate) fn step_transports(&mut self) {
        if self.transports.is_empty() {
            return;
        }
        let Some(db) = self.content.clone() else { return };
        let rules = TransportRules::of(&db);
        let ids: Vec<EntityId> = self.transports.rows.keys().copied().collect();
        for id in ids {
            let Some(row) = self.transports.rows.get(&id).copied() else { continue };
            if self.tick >= row.next_pulse && !self.pulse(&db, &rules, id, row.target) {
                self.transports.rows.remove(&id);
            }
        }
        let due = self.transports.parties.partition_point(|p| p.arrive <= self.tick);
        let arriving: Vec<Party> = self.transports.parties.drain(..due).collect();
        self.transports.reset_seq();
        for party in arriving {
            self.resolve_party(&db, party);
        }
    }

    /// One pulse of `carrier` to `target`; false when the transport ends.
    fn pulse(&mut self, db: &ContentDb, rules: &TransportRules, carrier: EntityId, target: EntityId) -> bool {
        let Some(c) = self.units.get(carrier).filter(|u| alive(u)) else { return false };
        let Some(t) = self.units.get(target).filter(|u| alive(u)) else { return false };
        let (Some(cdef), Some(tdef)) = (db.unit(c.def), db.unit(t.def)) else { return false };
        let (Some([cnow, cmax]), Some([tnow, tmax])) = (c.crew, t.crew) else { return false };
        if tmax < Fixed::ONE || !within(c.pos, t.pos, rules.range + cdef.radius + tdef.radius) {
            return false;
        }
        let (owner, tshield, tpos_owner) = (c.owner, shield_total(t), t.owner);
        let enemy = Diplomacy::new(&self.players, &self.relations).is_enemy(owner, tpos_owner);
        if enemy {
            if tshield > SHIELDS_DOWN {
                return false;
            }
        } else if tnow >= tmax {
            return false;
        }
        let size = rules.party.min(cnow);
        if size <= Fixed::ZERO {
            return false;
        }
        let strength = race_strength(db, self.unit_race(carrier).unwrap_or("")) * boarding_multiplier(cdef);
        let race = self.unit_race(carrier).unwrap_or("").to_owned();
        let drops = drops_shields(cdef);
        let tick = self.tick;
        if enemy {
            let until = tick + rules.target_shield_hold;
            let held = self.transports.holds.entry(target).or_insert(until);
            *held = (*held).max(until);
        }
        let Some(c) = self.units.get_mut(carrier) else { return false };
        c.crew = Some([cnow - size, cmax]);
        if drops {
            c.shields.now = [Fixed::ZERO; 4];
            let until = tick + rules.carrier_shield_return;
            let held = self.transports.holds.entry(carrier).or_insert(until);
            *held = (*held).max(until);
        }
        let party = Party { arrive: tick + if enemy { rules.action_delay } else { 0 }, seq: 0, target, size, strength, race: race.clone(), team: owner, source: carrier };
        if enemy {
            self.transports.send(party);
        } else {
            self.resolve_party(db, party);
        }
        if let Some(r) = self.transports.rows.get_mut(&carrier) {
            r.next_pulse = tick + rules.delay;
        }
        // The stop level: another pulse would take the crew to the race's red status.
        let red = crew_red_status(db, &race);
        let after = cnow - size;
        // Equality stops (M); the 32.32 value of 0.2 is a hair under, so the level is
        // rounded to 1/65536 before the comparison.
        let level = Fixed::from_bits(((red * cmax).to_bits() + (1 << 15)) & !((1i64 << 16) - 1));
        after - rules.party > level
    }

    /// A party reaches its target (see the module docs).
    pub(crate) fn resolve_party(&mut self, db: &ContentDb, p: Party) {
        // A party whose carrier is gone is lost (S).
        if self.units.get(p.source).is_none() {
            return;
        }
        let Some(t) = self.units.get(p.target).filter(|t| alive(t)) else { return };
        let Some([now, max]) = t.crew.filter(|c| c[1] > Fixed::ZERO) else { return };
        let Some(def) = db.unit(t.def) else { return };
        let owner = t.owner;
        let derelict = self.subsystems.get(p.target).is_some_and(|r| r.derelict);
        let enemy = Diplomacy::new(&self.players, &self.relations).is_enemy(p.team, owner);
        if !enemy {
            // Crew above the maximum is lost (M).
            if let Some(t) = self.units.get_mut(p.target) {
                t.crew = Some([(now + p.size).min(max), max]);
            }
            if derelict && !p.race.is_empty() && self.same_team(p.team, owner) {
                self.attrs.set_race(p.target, &p.race);
            }
            return;
        }
        if derelict {
            if let Some(t) = self.units.get_mut(p.target) {
                t.crew = Some([(now + p.size).min(max), max]);
            }
            self.capture(p.target, &p);
            return;
        }
        let defence = boarding_multiplier(def) * race_strength(db, self.unit_race(p.target).unwrap_or("")) * DEFENCE_FACTOR;
        if defence <= Fixed::ZERO {
            return;
        }
        let kills = p.size * p.strength / defence;
        let takes = kills > now;
        let survivors = if takes { (kills - now) * SURVIVOR_FACTOR } else { Fixed::ZERO };
        let mut ctx = Ctx { subsystems: &mut self.subsystems, rng: &mut self.rng, switches: &self.switches, colonies: &mut self.colonies };
        let Some(t) = self.units.get_mut(p.target) else { return };
        kill_crew(t, &mut ctx, kills);
        let emptied = t.crew.is_some_and(|c| c[0] <= Fixed::ZERO);
        if takes
            && survivors > Fixed::ZERO
            && emptied
            && self.capture(p.target, &p)
            && let Some(t) = self.units.get_mut(p.target)
        {
            t.crew = Some([survivors.min(max), max]);
        }
    }

    fn same_team(&self, a: PlayerId, b: PlayerId) -> bool {
        let team_of = |p: PlayerId| self.players.iter().find(|(q, _)| *q == p).map(|(_, t)| *t);
        a == b || team_of(a).is_some() && team_of(a) == team_of(b)
    }

    /// The boarders take the unit: owner and race on this tick, the perceived
    /// owner with them, the order Stop (by the shared guarded transfer).
    fn capture(&mut self, id: EntityId, p: &Party) -> bool {
        // A captured yard's queue is lost with nothing refunded to either side (M);
        // the shared transfer would refund the old owner's paid items. Cargo and
        // special energy are not touched by the transfer (M).
        let queue = self.production.take_queue_for_capture(id);
        if !self.transfer_owner(PlayerId::SCRIPT, id, p.team) {
            self.production.restore_queue_after_refused_capture(id, queue);
            return false;
        }
        self.perception.remove(id);
        if !p.race.is_empty() {
            self.attrs.set_race(id, &p.race);
        }
        true
    }
}
