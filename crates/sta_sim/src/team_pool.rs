//! What a race changes at run time (SIM-54), and the scripted edits of a
//! team's crew pool and officer count.
//!
//! Rules, read from the executables (static; nothing measured in game):
//!
//! - A unit's race is its definition's unless a script set it
//!   ([`Sim::unit_race`]). What a unit builds takes its race: a producer, a research
//!   station, a mining station building a freighter and a construction rig starting a
//!   build all copy the producer's race (read from the executable, static). A build whose
//!   definition already carries the builder's race needs no row, so only a
//!   difference is recorded in [`crate::attrs::Attrs`]. A construction rig's
//!   station takes the rig's race when the station completes (the original
//!   copies it when the build starts).
//! - A change of team never re-races a unit. A planet does take the new
//!   team's race (read from the executable, static); the sim stores no planet race: a colony's
//!   growth and crew income read its owner's [`Sim::seat_race`] every tick
//!   (SIM-30, `crate::colony`), so they follow a change of owner or seat race.
//! - A seat's race ([`Sim::seat_race`]: the script's, else the setup's)
//!   gives the officer cap multiplier and the latinum factor of the team
//!   (`sta_content::SeatRaceRule`), re-read whenever the seat race changes.
//!   Starting resources and the AI choice are read once at the start and do
//!   not follow a re-race.

use sta_core::{EntityId, Fixed, PlayerId};

use crate::command::TeamOp;
use crate::economy::Economy;
use crate::event::SimEvent;
use crate::Sim;

impl Sim {
    /// A unit's race: the script's, else its definition's.
    pub fn unit_race(&self, id: EntityId) -> Option<&str> {
        if let Some(race) = self.attrs.race(id) {
            return Some(race);
        }
        let def = self.units.get(id)?.def;
        self.content.as_deref()?.unit(def).map(|d| d.race.as_str())
    }

    /// A seat's race: the script's (Roots `Player.race`), else the setup's.
    pub fn seat_race(&self, player: PlayerId) -> Option<&str> {
        self.attrs
            .player_race(player)
            .or_else(|| self.appearance_factions.iter().find(|(p, _)| *p == player).map(|(_, r)| r.as_str()))
    }

    /// Re-derive what each seat's race changes. Called after a setup, a snapshot
    /// restore and a seat re-race.
    pub(crate) fn refresh_seat_rules(&mut self) {
        let Some(db) = self.content.clone() else { return };
        let seats: Vec<PlayerId> = self.players.iter().map(|p| p.0).collect();
        for p in seats {
            let rule = db.rules().seat_race(self.seat_race(p).unwrap_or(""));
            self.economy.set_seat_rule(p, rule);
        }
    }

    /// The units the tick's production finished take their builder's race.
    pub(crate) fn inherit_builder_races(&mut self, from: usize) {
        let Some(db) = self.content.clone() else { return };
        let built: Vec<(EntityId, EntityId)> = self.events[from..]
            .iter()
            .filter_map(|e| match e {
                SimEvent::Built { builder, id } => Some((*builder, *id)),
                _ => None,
            })
            .collect();
        for (builder, id) in built {
            let (Some(race), Some(def)) = (self.unit_race(builder), self.units.get(id).and_then(|u| db.unit(u.def))) else { continue };
            if !race.trim().is_empty() && !race.trim().eq_ignore_ascii_case(def.race.trim()) {
                let race = race.to_owned();
                self.attrs.set_race(id, &race);
            }
        }
    }

    /// [`crate::command::Edit::TeamPool`].
    pub(crate) fn edit_team_pool(&mut self, player: PlayerId, op: TeamOp, x: Fixed) -> bool {
        if let TeamOp::ResourceAdd(slot) = op {
            if !self.players.iter().any(|(id, _)| *id == player)
                || usize::from(slot) >= crate::economy::RESOURCES
                || x < Fixed::ZERO
            {
                return false;
            }
            let mut stock: [Fixed; crate::economy::RESOURCES] = self.economy.stock_of(player).try_into().unwrap_or([Fixed::ZERO; crate::economy::RESOURCES]);
            let index = usize::from(slot);
            stock[index] = Fixed::from_bits(stock[index].to_bits().saturating_add(x.to_bits()));
            self.economy.set_stock(player, stock);
            return true;
        }
        let Some(db) = self.content.clone() else { return false };
        if !self.players.iter().any(|(id, _)| *id == player) {
            return false;
        }
        let capacity = Economy::derived_crew_capacity(&self.units, &db, &self.attrs, player);
        let census = self.production.officer_census(&self.units, &db, player);
        let officers = self.officer_limit(player).map(|cap| (census, cap, db.a1_officers()));
        self.economy.team_op(player, op, x, capacity, officers)
    }
}

fn add(a: Fixed, b: Fixed) -> Fixed {
    Fixed::from_bits(a.to_bits().saturating_add(b.to_bits()))
}

/// A float to an integer count as the original's `ftol` does (toward zero).
fn trunc(v: Fixed) -> Fixed {
    Fixed::from_bits((v.to_bits() / (1 << 32)) << 32)
}

/// The A2 rule for adding crew to a team (read from the executable, static): over
/// capacity a positive add is dropped and a negative one applied; otherwise
/// the sum, clamped to capacity. No floor at 0.
fn a2_add(pool: Fixed, x: Fixed, capacity: Fixed) -> Fixed {
    if pool > capacity {
        if x > Fixed::ZERO { pool } else { add(pool, x) }
    } else {
        add(pool, x).min(capacity)
    }
}

/// The crew pool after a crew [`TeamOp`] (`None` for another kind of op).
/// A2 `SetTeamCrew(v)` is `AddCrew(v - pool)`; A1's calls clamp to the team
/// maximum with no floor.
pub fn crew_pool_after(op: TeamOp, pool: Fixed, x: Fixed, capacity: Fixed) -> Option<Fixed> {
    Some(match op {
        TeamOp::CrewAddA2 => a2_add(pool, x, capacity),
        TeamOp::CrewSetA2 => a2_add(pool, Fixed::from_bits(x.to_bits().saturating_sub(pool.to_bits())), capacity),
        TeamOp::CrewAddA1 => add(pool, x).min(capacity),
        TeamOp::CrewSetA1 => x.min(capacity),
        _ => return None,
    })
}

/// The officer count after an A1 officer [`TeamOp`] (`None` for another kind of op):
/// `AddOfficers` is `min(used + ftol(x), max)`, `SetOfficers` `min(ftol(v), max)`;
/// no floor (Armada I `Team` calls, read from the executable, static).
pub fn officer_count_after(op: TeamOp, used: Fixed, x: Fixed, cap: Fixed) -> Option<Fixed> {
    Some(match op {
        TeamOp::OfficersAddA1 => add(used, trunc(x)).min(cap),
        TeamOp::OfficersSetA1 => trunc(x).min(cap),
        _ => return None,
    })
}
