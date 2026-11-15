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
//!   with `survivors = (kills - crew) x D / A` (M, two pairings); the survivors
//!   become the crew.
//! - Capture changes owner and race on one tick, the order becomes Stop and the
//!   perceived owner follows (M); the carrier goes on transporting to it.
//! - The carrier stops when `(crew - max) / maxCrew <= crewRedStatus` after a
//!   pulse (M); its shields go to 0 at the first pulse and return in full in one
//!   step a fixed time after the last (M); the target's recharge delay is set to 3.0 s on every
//!   pulse (M).
//!
//! **Content seam (SIM-69).** The numbers come from content: the header's
//! `TRANSPORTER_*` ([`TransportRules`]), the unit's `transporter`,
//! `boardingPartyStrength`, `disableShieldsOnTransport` and `shieldDelay`, the
//! race's `boardingStrength`, `canCommandeer` and `canBeCommandeered` (through
//! `Rules::race_boarding`). Absent keys take the documented defaults of
//! `sta_content`. All of them are in the content digest when set. The
//! executable constants (crew floor 9, the 1.5 defence factor, the carrier's
//! 3.3 s shield return) and the race `crewRedStatus` stay as before.

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
pub(crate) const DEFENCE_FACTOR: Fixed = Fixed::from_bits(6_442_450_944);
/// 0.25, the executable's default `crewRedStatus`.
const DEFAULT_CREW_RED: Fixed = Fixed::from_bits(1 << 30);

/// The transporter numbers of the content: `RTS_CFG.h`'s `TRANSPORTER_*`
/// (the engine's defaults for a key the header leaves out) and the executable's
/// constants (SIM-69).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransportRules {
    /// `TRANSPORTER_RANGE`, surface to surface.
    pub range: Fixed,
    /// `TRANSPORTER_MAX`: the party size and the carrier's loss per pulse.
    pub party: Fixed,
    /// `TRANSPORTER_DELAY` in ticks.
    pub delay: Tick,
    /// `TRANSPORTER_ACTION_DELAY` in ticks: an enemy party's flight.
    pub action_delay: Tick,
    /// The carrier needs more crew than this to start (9, exe constant, S).
    pub min_crew: Fixed,
    /// The target's shield recharge delay set per pulse when its ODF has no
    /// `shieldDelay` (3.0 s, M); [`shield_hold`] reads the key.
    pub target_shield_hold: Tick,
    /// How long after a pulse the carrier's shields stay off (3.3 s, M; not a data key).
    pub carrier_shield_return: Tick,
}

/// Seconds as ticks, rounded to the nearest; negative reads as zero.
fn ticks(seconds: Fixed) -> Tick {
    ((i128::from(seconds.to_bits().max(0)) * i128::from(TICK_HZ) + (1 << 31)) >> 32) as Tick
}

impl TransportRules {
    pub(crate) fn of(db: &ContentDb) -> TransportRules {
        let t = db.rules().transporter();
        TransportRules {
            range: t.range,
            party: Fixed::from_int(t.max.clamp(0, 1_000_000) as i32),
            delay: ticks(t.delay),
            action_delay: ticks(t.action_delay),
            min_crew: Fixed::from_int(9),
            target_shield_hold: 3 * TICK_HZ as Tick,
            carrier_shield_return: 99,
        }
    }
}

/// Whether the unit's game family and its own keys allow `Order::Transport` at all, whatever its
/// crew: the profile family (`crew_loss_on_hit`, see the module docs), the unit's `transporter` key
/// (absent: a crewed ship or station, as the stub did) and no planets. Read-only; a client shows
/// its button from this and from [`can_transport`] and does not repeat the rule.
pub fn transport_allowed(db: &ContentDb, def: &UnitDef, crew_max: Fixed) -> bool {
    def.planet_role.is_none() && db.unit_features(def).crew_loss_on_hit && def.transport.transporter.unwrap_or(crew_max > Fixed::ZERO)
}

/// The sim's own answer to "may this unit be ordered to transport now?": [`transport_allowed`] and
/// the crew floors (`crew` is current and maximum crew: the carrier needs more than the
/// executable's 9 and more than one party). Read-only; the order validation uses it.
pub fn can_transport(db: &ContentDb, def: &UnitDef, crew: [Fixed; 2]) -> bool {
    let rules = TransportRules::of(db);
    transport_allowed(db, def, crew[1]) && crew[0] > rules.min_crew && crew[0] > rules.party
}

/// `boardingPartyStrength` (1.0 when absent).
fn boarding_multiplier(def: &UnitDef) -> Fixed {
    def.transport.boarding_party_strength
}

/// The race's `boardingStrength` (1.0 when absent).
fn race_strength(db: &ContentDb, race: &str) -> Fixed {
    db.rules().race_boarding(race).boarding_strength
}

/// Race `canBeCommandeered` (true when absent).
fn can_be_commandeered(db: &ContentDb, race: &str) -> bool {
    db.rules().race_boarding(race).can_be_commandeered
}

/// Race `canCommandeer` (true when absent).
fn can_commandeer(db: &ContentDb, race: &str) -> bool {
    db.rules().race_boarding(race).can_commandeer
}

/// `disableShieldsOnTransport` (true when absent).
fn drops_shields(def: &UnitDef) -> bool {
    def.transport.disable_shields_on_transport
}

/// The recharge hold set on a boarded target: its `shieldDelay`, else the default.
fn shield_hold(def: &UnitDef, rules: &TransportRules) -> Tick {
    def.transport.shield_delay.map_or(rules.target_shield_hold, ticks)
}

/// The carrier race's `crewRedStatus` (data), the executable's 0.25 when absent.
fn crew_red_status(db: &ContentDb, race: &str) -> Fixed {
    db.rules().race_repair_of(race).and_then(|r| r.crew_red_status).unwrap_or(DEFAULT_CREW_RED)
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
    /// Keyed by entity and kind ([`TARGET_HOLD`], [`CARRIER_HOLD`]), so a unit that is a
    /// carrier and an enemy's target at once keeps both; each kind keeps its later end.
    pub(crate) holds: BTreeMap<(EntityId, u8), Tick>,
}

/// Hold kind: the unit is a boarded enemy whose recharge delay was set.
pub(crate) const TARGET_HOLD: u8 = 0;
/// Hold kind: the unit is a carrier whose shields are off; they return in full when it ends.
pub(crate) const CARRIER_HOLD: u8 = 1;

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
    /// `(entity, kind, held until)`, ascending `(entity, kind)`.
    pub holds: Vec<(EntityId, u8, Tick)>,
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
        self.holds.contains_key(&(id, TARGET_HOLD)) || self.holds.contains_key(&(id, CARRIER_HOLD))
    }

    /// Holds that ended by `tick` are dropped (before the recharge pass); returns the
    /// carriers whose shields-off hold ended, ascending.
    pub(crate) fn expire(&mut self, tick: Tick) -> Vec<EntityId> {
        let ended: Vec<EntityId> = self.holds.iter().filter(|((_, kind), until)| *kind == CARRIER_HOLD && **until <= tick).map(|((id, _), _)| *id).collect();
        self.holds.retain(|_, until| *until > tick);
        ended
    }

    /// Hold `id` of `kind` until `until`, keeping a later end if there is one.
    fn hold(&mut self, id: EntityId, kind: u8, until: Tick) {
        let held = self.holds.entry((id, kind)).or_insert(until);
        *held = (*held).max(until);
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
        self.holds.remove(&(id, TARGET_HOLD));
        self.holds.remove(&(id, CARRIER_HOLD));
        self.reset_seq();
    }

    fn reset_seq(&mut self) {
        if self.parties.is_empty() {
            self.next_seq = 0;
        }
    }

    fn send(&mut self, mut party: Party) {
        party.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
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
            holds: self.holds.iter().map(|((id, kind), t)| (*id, *kind, *t)).collect(),
        }
    }

    /// Rebuild from a snapshot record: ascending ids and order keys, entities that exist, sane sizes.
    pub(crate) fn from_rec(rec: TransportsRec, exists: impl Fn(EntityId) -> bool) -> Option<Transports> {
        if rec.version != 1
            || !rec.rows.windows(2).all(|w| w[0].0 < w[1].0)
            || !rec.holds.windows(2).all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1))
            || !rec.parties.windows(2).all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1))
        {
            return None;
        }
        const LIMIT: i64 = 1_000_000i64 << 32;
        let rows_ok = rec.rows.iter().all(|r| exists(r.0) && exists(r.1) && r.0 != r.1);
        let parties_ok = rec.parties.iter().all(|p| p.1 < rec.next_seq && exists(p.2) && (1..=LIMIT).contains(&p.3) && (0..=LIMIT).contains(&p.4));
        let holds_ok = rec.holds.iter().all(|h| exists(h.0) && h.1 <= CARRIER_HOLD);
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
            holds: rec.holds.into_iter().map(|(id, kind, until)| ((id, kind), until)).collect(),
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
        for ((id, kind), until) in &self.holds {
            h.u64(u64::from(id.0));
            h.u64(u64::from(*kind));
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
        t.holds.insert((carrier, CARRIER_HOLD), 219);
        t.holds.insert((carrier, TARGET_HOLD), 205);
        t.holds.insert((target, TARGET_HOLD), 210);
        t
    }
}

/// One party's boarding fight: `kills = size x A / D`; when `kills > crew`,
/// `survivors = (kills - crew) x D / A` (measured), else none.
pub(crate) fn outcome(size: Fixed, attack: Fixed, defence: Fixed, crew: Fixed) -> (Fixed, Fixed) {
    let kills = size * attack / defence;
    if kills > crew && attack > Fixed::ZERO {
        (kills, (kills - crew) * defence / attack)
    } else {
        (kills, Fixed::ZERO)
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
                            && def.is_some_and(|d| u.crew.is_some_and(|c| can_transport(&db, d, c)))
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
            self.transports.hold(target, TARGET_HOLD, tick + shield_hold(tdef, rules));
        }
        let Some(c) = self.units.get_mut(carrier) else { return false };
        c.crew = Some([cnow - size, cmax]);
        if drops {
            c.shields.now = [Fixed::ZERO; 4];
            self.transports.hold(carrier, CARRIER_HOLD, tick + rules.carrier_shield_return);
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
        let (kills, survivors) = outcome(p.size, p.strength, defence, now);
        let takes = kills > now;
        let mut ctx = Ctx { subsystems: &mut self.subsystems, rng: &mut self.rng, switches: &self.switches, colonies: &mut self.colonies };
        let Some(t) = self.units.get_mut(p.target) else { return };
        kill_crew(t, &mut ctx, kills);
        let emptied = t.crew.is_some_and(|c| c[0] <= Fixed::ZERO);
        if takes && survivors > Fixed::ZERO && emptied && self.capture(p.target, &p) {
            if let Some(t) = self.units.get_mut(p.target) {
                t.crew = Some([survivors.min(max), max]);
            }
        } else if emptied {
            // A tie (or a refused capture) leaves a derelict that keeps its owner; its order is Stop.
            self.stop_order(p.target);
        }
    }

    /// The unit's order becomes Stop (as a capture leaves it).
    fn stop_order(&mut self, id: EntityId) {
        self.weapons.stop_attack(&self.units, PlayerId::SCRIPT, &[id]);
        self.clear_scripted(PlayerId::SCRIPT, &[id]);
        self.movement.clear_waypoints(id);
        self.set_target(id, None);
    }

    /// Carriers whose shields-off hold ended get their shields back in full in one step (measured).
    pub(crate) fn restore_carrier_shields(&mut self, carriers: &[EntityId]) {
        for &id in carriers {
            let derelict = self.subsystems.get(id).is_some_and(|r| r.derelict);
            if let Some(u) = self.units.get_mut(id).filter(|u| alive(u) && !derelict) {
                let pools = if u.shields.directional { 4 } else { 1 };
                for now in &mut u.shields.now[..pools] {
                    *now = u.shields.max;
                }
            }
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
