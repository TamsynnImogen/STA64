//! LUA-32: alert status, the last enemy hit and the ally attack counters.
//!
//! Rules (read from the executable, static; nothing measured in a running
//! game; "ne" marks what is not established):
//!
//! - **Alert** is one level per craft or station: red (default), yellow,
//!   green. `Edit::Alert` writes it directly: no voice, no command reset, no
//!   target clear (the HUD's alert order, which does reset the command, is a
//!   UI package). A craft's row is dropped when it is set to red. The default
//!   is red; the engine's class key `defaultGreenAlert` (stock `csuper` only)
//!   is not in the content crate yet (TODO: CON lane). A unit built by a yard
//!   copies its producer's level (the engine copies it at `FinishBuild`); a
//!   script spawn starts at the default.
//! - **Gates** (the engine's fire and idle code, stock A2):
//!   the ordered attack target is fired at under every level. Opportunity
//!   fire (any other target): green never, yellow only within
//!   [`YELLOW_WINDOW_TICKS`] (15 s, strict) of an enemy hit on the unit, red
//!   always ([`Sim::opportunity_fire_allowed`], read by `weapons::pick`; a
//!   starbase runs the same gate). Idle assist ([`Sim::assist_gate`], read by
//!   `targeting::assist_idle`): green skips the unit; yellow assists as red
//!   does (assist includes the unit's own hit and fire). The separate pursue
//!   branch is AI-team and red only (TODO: not split out; assist already covers it). Alert has
//!   no effect on shields, recharge, energy or point defence.
//! - **Last enemy hit**: the tick of the last hit the unit took from an enemy
//!   (the victim's own view of the attacker's owner is Enemy; any damage;
//!   never a collision). A change of owner and actually cloaking reset it. The
//!   engine also resets it on a queue command; STA64 has no counterpart yet
//!   (TODO). The binding `UnderAttack(h, s)` is `now - stamp < s` (strict).
//! - **Ally attack counters** `[team][byTeam]` (`GetAllyShipAttacks`,
//!   `GetAllyStationAttacks`): one count per craft that takes an accepted
//!   `Order::Attack` against a live craft or station whose owner's view of the
//!   attacker's owner is not Enemy; ship more than 1.0 s, station more than
//!   0.05 s after the last record of that attacker seat (one shared time per
//!   attacker). Cumulative, no reset. Orders only: the assist's own attack
//!   orders and the automatic pickup do not count (whether the engine's
//!   automatic pickup goes through its ATTACK command is not established).
//!   Both teams must be seats here.
//!
//! State and digest: alert rows, the counters and their last-record times are
//! hashed only when present; the enemy-hit table is always saved but hashed
//! only while some unit is not at red (its only in-sim reader is the yellow
//! gate; hits already show in hull and shields). A run that never leaves red
//! and never orders an attack on a non-enemy hashes nothing new.

use serde::{Deserialize, Serialize};
use sta_core::{EntityId, PlayerId, TICK_HZ};

use crate::Sim;
use crate::command::AlertState;
use crate::event::SimEvent;
use crate::view::{Relation, UnitFlags};
use crate::world::{EntityKind, Fnv, Table};

/// Yellow alert fires at other targets for 15 s after an enemy hit.
pub const YELLOW_WINDOW_TICKS: u64 = 15 * TICK_HZ as u64;
/// A ship attack order counts only more than this many ticks (1.0 s) after the last record.
const SHIP_GAP_TICKS: u64 = TICK_HZ as u64;

/// The stored form of a non-red level.
const YELLOW: u8 = 1;
const GREEN: u8 = 2;

fn code(s: AlertState) -> Option<u8> {
    match s {
        AlertState::Yellow => Some(YELLOW),
        AlertState::Green => Some(GREEN),
        _ => None,
    }
}

fn state(c: u8) -> AlertState {
    match c {
        YELLOW => AlertState::Yellow,
        GREEN => AlertState::Green,
        _ => AlertState::Red,
    }
}

/// `[team][byTeam]` ship and station counters: `(team, by, ships, stations)`.
pub type AllyRow = (u8, u8, u32, u32);

#[derive(Debug, Clone, Default)]
pub struct Alerts {
    /// Units not at red, ascending id.
    level: Table<u8>,
    /// The tick of the last enemy hit on each unit.
    enemy_hit: Table<u64>,
    /// Non-zero counters, ascending `(team, by)`.
    ally: Vec<AllyRow>,
    /// The tick of the last counted order of each attacker seat, ascending.
    ally_last: Vec<(u8, u64)>,
}

/// [`Alerts`] as the snapshot stores it (format 30).
#[derive(Serialize, Deserialize)]
pub(crate) struct AlertRec {
    pub(crate) version: u8,
    pub(crate) level: Vec<(EntityId, u8)>,
    pub(crate) enemy_hit: Vec<(EntityId, u64)>,
    pub(crate) ally: Vec<AllyRow>,
    pub(crate) ally_last: Vec<(u8, u64)>,
}

impl Alerts {
    pub(crate) fn remove(&mut self, id: EntityId) {
        self.level.remove(id);
        self.enemy_hit.remove(id);
    }

    pub fn state(&self, id: EntityId) -> AlertState {
        self.level.get(id).map_or(AlertState::Red, |c| state(*c))
    }

    fn set(&mut self, id: EntityId, s: AlertState) {
        match code(s) {
            Some(c) => {
                self.level.insert(id, c);
            }
            None => {
                self.level.remove(id);
            }
        }
    }

    /// Nothing that is hashed: no level off red and no counter.
    pub(crate) fn is_default(&self) -> bool {
        self.level.is_empty() && self.ally.is_empty()
    }

    /// Ticks since the last enemy hit on `id` at tick `now`, `None` if never hit.
    pub fn since_hit(&self, id: EntityId, now: u64) -> Option<u64> {
        self.enemy_hit.get(id).map(|t| now.saturating_sub(*t))
    }

    /// May `id` fire at a target other than its ordered one at tick `now`?
    pub(crate) fn opportunity_allowed(&self, id: EntityId, now: u64) -> bool {
        match self.level.get(id).copied() {
            None => true,
            Some(YELLOW) => self.since_hit(id, now).is_some_and(|age| age < YELLOW_WINDOW_TICKS),
            Some(_) => false,
        }
    }

    pub(crate) fn levels(&self) -> Vec<(EntityId, AlertState)> {
        self.level.iter().map(|(id, c)| (id, state(*c))).collect()
    }

    pub(crate) fn hits(&self) -> Vec<(EntityId, u64)> {
        self.enemy_hit.iter().map(|(id, t)| (id, *t)).collect()
    }

    pub(crate) fn ally_rows(&self) -> &[AllyRow] {
        &self.ally
    }

    pub(crate) fn stamp(&mut self, id: EntityId, tick: u64) {
        self.enemy_hit.insert(id, tick);
    }

    pub(crate) fn clear_hit(&mut self, id: EntityId) {
        self.enemy_hit.remove(id);
    }

    /// Count one accepted attack order of seat `by` on a unit of seat `team`, if
    /// the throttle allows; true when it was counted.
    pub(crate) fn count(&mut self, now: u64, team: u8, by: u8, station: bool) -> bool {
        let last = self.ally_last.iter().find(|r| r.0 == by).map(|r| r.1);
        // Ship: more than 1.0 s; station: more than 0.05 s (age * 20 > one second of ticks).
        let gap = |age: u64| if station { age * 20 > u64::from(TICK_HZ) } else { age > SHIP_GAP_TICKS };
        if last.is_some_and(|t| !gap(now.saturating_sub(t))) {
            return false;
        }
        match self.ally.binary_search_by_key(&(team, by), |r| (r.0, r.1)) {
            Ok(i) => {
                let r = &mut self.ally[i];
                if station { r.3 = r.3.saturating_add(1) } else { r.2 = r.2.saturating_add(1) }
            }
            Err(i) => self.ally.insert(i, (team, by, u32::from(!station), u32::from(station))),
        }
        match self.ally_last.binary_search_by_key(&by, |r| r.0) {
            Ok(i) => self.ally_last[i].1 = now,
            Err(i) => self.ally_last.insert(i, (by, now)),
        }
        true
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        if !self.level.is_empty() {
            h.str("alert-v1");
            h.u64(self.level.len() as u64);
            for (id, c) in self.level.iter() {
                h.u64(id.0 as u64);
                h.u64(u64::from(*c));
            }
            // Only read by the yellow gate, so only hashed while a gate exists.
            h.str("enemy-hit-v1");
            h.u64(self.enemy_hit.len() as u64);
            for (id, t) in self.enemy_hit.iter() {
                h.u64(id.0 as u64);
                h.u64(*t);
            }
        }
        if !self.ally.is_empty() {
            h.str("ally-attacks-v1");
            h.u64(self.ally.len() as u64);
            for r in &self.ally {
                h.u64(u64::from(r.0));
                h.u64(u64::from(r.1));
                h.u64(u64::from(r.2));
                h.u64(u64::from(r.3));
            }
            for r in &self.ally_last {
                h.u64(u64::from(r.0));
                h.u64(r.1);
            }
        }
    }

    pub(crate) fn to_rec(&self) -> AlertRec {
        AlertRec {
            version: 1,
            level: self.level.iter().map(|(id, c)| (id, *c)).collect(),
            enemy_hit: self.hits(),
            ally: self.ally.clone(),
            ally_last: self.ally_last.clone(),
        }
    }

    /// The canonical form only: ascending ids of live craft or stations at yellow
    /// or green, hit ticks not after `now` on existing units, non-zero counters of
    /// seats ascending, one last-record time per attacker with a counter.
    pub(crate) fn from_rec(
        r: AlertRec,
        now: u64,
        kind: impl Fn(EntityId) -> Option<EntityKind>,
        seat: impl Fn(u8) -> bool,
    ) -> Result<Alerts, &'static str> {
        if r.version != 1 {
            return Err("unknown alert record version");
        }
        if r.level.iter().any(|(id, c)| !matches!(kind(*id), Some(EntityKind::Craft | EntityKind::Station)) || !matches!(*c, YELLOW | GREEN)) {
            return Err("alert row of a unit that cannot hold one, or at red");
        }
        if r.enemy_hit.iter().any(|(id, t)| kind(*id).is_none() || *t > now) {
            return Err("enemy-hit row of an unknown unit or from the future");
        }
        if r.ally.windows(2).any(|w| (w[0].0, w[0].1) >= (w[1].0, w[1].1))
            || r.ally.iter().any(|a| a.2 == 0 && a.3 == 0 || !seat(a.0) || !seat(a.1))
        {
            return Err("ally attack rows");
        }
        let mut want: Vec<u8> = r.ally.iter().map(|a| a.1).collect();
        want.sort_unstable();
        want.dedup();
        if r.ally_last.iter().map(|l| l.0).ne(want.iter().copied()) || r.ally_last.iter().any(|l| l.1 > now) {
            return Err("ally attack times");
        }
        Ok(Alerts {
            level: Table::from_rows(r.level).ok_or("alert rows out of order")?,
            enemy_hit: Table::from_rows(r.enemy_hit).ok_or("enemy-hit rows out of order")?,
            ally: r.ally,
            ally_last: r.ally_last,
        })
    }
}

impl Sim {
    /// The alert level of a unit (red for anything without a row).
    pub fn alert_of(&self, id: EntityId) -> AlertState {
        self.alerts.state(id)
    }

    /// `Edit::Alert`: a live craft or station (a prop, a shot, a dying unit and
    /// a gone one are rejected). Red drops the row.
    pub(crate) fn edit_alert(&mut self, id: EntityId, to: AlertState) -> bool {
        let live = self.units.get(id).is_some_and(|u| {
            matches!(u.kind, EntityKind::Craft | EntityKind::Station) && u.hull[0] > sta_core::Fixed::ZERO && !u.flags.contains(UnitFlags::DYING)
        });
        if live {
            self.alerts.set(id, to);
        }
        live
    }

    /// Weapon gate: may `id` fire at something other than its ordered target now?
    pub(crate) fn opportunity_fire_allowed(&self, id: EntityId) -> bool {
        self.alerts.opportunity_allowed(id, self.tick)
    }

    /// Idle-assist gate: true for a green unit (red and yellow assist alike).
    pub(crate) fn assist_gate(&self, id: EntityId) -> bool {
        self.alerts.level.get(id).copied() == Some(GREEN)
    }

    /// Stamp the enemy hits among `events[from..]` (a hit by an enemy of the
    /// victim's owner; own, allied and neutral hits do not count).
    pub(crate) fn record_enemy_hits(&mut self, from: usize) {
        let dip = crate::relations::Diplomacy::new(&self.players, &self.relations);
        let tick = self.tick;
        let mut stamps = Vec::new();
        for e in &self.events[from..] {
            if let SimEvent::Hit { target, by, .. } = *e
                && target != by
                && let (Some(v), Some(a)) = (self.units.get(target), self.units.get(by))
                && dip.is_enemy(v.owner, a.owner)
            {
                stamps.push(target);
            }
        }
        for id in stamps {
            self.alerts.stamp(id, tick);
        }
    }

    /// A unit that actually cloaked forgets its last hit.
    pub(crate) fn forget_hits_of_cloakers(&mut self, from: usize) {
        let ids: Vec<EntityId> = self.events[from..]
            .iter()
            .filter_map(|e| match e {
                SimEvent::CloakChanged { id, on: true } => Some(*id),
                _ => None,
            })
            .collect();
        for id in ids {
            self.alerts.clear_hit(id);
        }
    }

    /// A unit a yard finished starts at its producer's alert level.
    pub(crate) fn inherit_builder_alert(&mut self, from: usize) {
        let built: Vec<(EntityId, EntityId)> = self.events[from..]
            .iter()
            .filter_map(|e| match e {
                SimEvent::Built { builder, id } => Some((*builder, *id)),
                _ => None,
            })
            .collect();
        for (builder, id) in built {
            let level = self.alerts.state(builder);
            if level != AlertState::Red && self.units.get(id).is_some() {
                self.alerts.set(id, level);
            }
        }
    }

    /// Count the attack orders `ids` (the units that took it) gave against
    /// `target` for `GetAllyShipAttacks` / `GetAllyStationAttacks`.
    pub(crate) fn count_ally_attacks(&mut self, ids: &[EntityId], target: EntityId) {
        let Some(t) = self.units.get(target).filter(|t| t.hull[0] > sta_core::Fixed::ZERO) else { return };
        let station = match t.kind {
            EntityKind::Craft => false,
            EntityKind::Station => true,
            _ => return,
        };
        let (team, tick) = (t.owner, self.tick);
        let mut counted: Vec<(u8, u8)> = Vec::new();
        {
            let dip = crate::relations::Diplomacy::new(&self.players, &self.relations);
            for &id in ids {
                let Some(a) = self.units.get(id) else { continue };
                if dip.is_seat(a.owner) && dip.is_seat(team) && dip.view(team, a.owner) != Relation::Enemy {
                    counted.push((team.0, a.owner.0));
                }
            }
        }
        for (team, by) in counted {
            self.alerts.count(tick, team, by, station);
        }
    }

    /// `GetAllyShipAttacks(team, by)` and `GetAllyStationAttacks(team, by)`.
    pub fn ally_attacks(&self, team: PlayerId, by: PlayerId) -> (u32, u32) {
        self.alerts.ally.iter().find(|r| (r.0, r.1) == (team.0, by.0)).map_or((0, 0), |r| (r.2, r.3))
    }
}
