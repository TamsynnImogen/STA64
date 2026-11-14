//! Team rules that follow the seat's race, and the script's edits of the team
//! pool (SIM-54). Rules are read from the executables, static (Armada II 1.1
//! `Team::AddCrew` / `AddCrewCapacity` / `GetMaxOfficers`, byte-identical in
//! the Roots and Fleet Operations 3.2.7 engines; Armada I's team calls),
//! nothing measured in game.

use sta_content::SeatRaceRule;

use super::*;
use crate::command::TeamOp;
use crate::team_pool::{crew_pool_after, officer_count_after};

/// Latinum's resource id.
const LATINUM: usize = 1;
const LIMIT: i64 = 1_000_000_000;

fn mul(a: Fixed, b: Fixed) -> Fixed {
    Fixed::from_bits(((i128::from(a.to_bits()) * i128::from(b.to_bits())) >> 32).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
}

fn add(a: Fixed, b: Fixed) -> Fixed {
    Fixed::from_bits(a.to_bits().saturating_add(b.to_bits()))
}

fn bound(v: Fixed) -> Fixed {
    Fixed::from_bits(v.to_bits().clamp(-(LIMIT << 32), LIMIT << 32))
}

impl Economy {
    pub(crate) fn seat_rule(&self, player: PlayerId) -> SeatRaceRule {
        self.seat.binary_search_by_key(&player, |r| r.0).map_or_else(|_| SeatRaceRule::default(), |i| self.seat[i].1)
    }

    pub(crate) fn set_seat_rule(&mut self, player: PlayerId, rule: SeatRaceRule) {
        match (self.seat.binary_search_by_key(&player, |r| r.0), rule == SeatRaceRule::default()) {
            (Ok(i), true) => {
                self.seat.remove(i);
            }
            (Ok(i), false) => self.seat[i].1 = rule,
            (Err(i), false) => self.seat.insert(i, (player, rule)),
            (Err(_), true) => {}
        }
    }

    /// The factor on the latinum part of what `player` pays for a build: 0 for
    /// a team of a race the data frees of latinum (the original's Borg), else 1.
    pub fn latinum_scale(&self, player: PlayerId) -> Fixed {
        self.seat_rule(player).latinum_scale
    }

    /// `cost` per resource id with the seat race's latinum factor applied.
    pub fn scaled_cost(&self, player: PlayerId, cost: &[Fixed]) -> Vec<Fixed> {
        let scale = self.latinum_scale(player);
        cost.iter().enumerate().map(|(i, &c)| if i == LATINUM && scale != Fixed::ONE { mul(c, scale) } else { c }).collect()
    }

    fn adjust_of(&self, player: PlayerId) -> (Fixed, Fixed) {
        self.adjust.binary_search_by_key(&player, |r| r.0).map_or((Fixed::ZERO, Fixed::ZERO), |i| (self.adjust[i].1, self.adjust[i].2))
    }

    /// Roots `Player.maxCrew` writes the capacity raw: what it added to the
    /// derived capacity, and what later capacity changes then shift.
    pub fn crew_capacity_adjust(&self, player: PlayerId) -> Fixed {
        self.adjust_of(player).0
    }

    /// Armada I `AddOfficers` / `SetOfficers` write the officer count: the
    /// offset from the count of live and reserved actors.
    pub fn officer_adjust(&self, player: PlayerId) -> Fixed {
        self.adjust_of(player).1
    }

    fn set_adjust(&mut self, player: PlayerId, capacity: Option<Fixed>, officers: Option<Fixed>) {
        let (c, o) = self.adjust_of(player);
        let row = (player, capacity.map_or(c, bound), officers.map_or(o, bound));
        let empty = row.1 == Fixed::ZERO && row.2 == Fixed::ZERO;
        match (self.adjust.binary_search_by_key(&player, |r| r.0), empty) {
            (Ok(i), true) => {
                self.adjust.remove(i);
            }
            (Ok(i), false) => self.adjust[i] = row,
            (Err(i), false) => self.adjust.insert(i, row),
            (Err(_), true) => {}
        }
    }

    /// One scripted team-pool edit. `capacity` is the derived crew capacity
    /// without the script offset (None: unknown, the edit is refused), `census`
    /// the live and reserved officers.
    pub(crate) fn team_op(&mut self, player: PlayerId, op: TeamOp, x: Fixed, derived_capacity: Option<Fixed>, census: Fixed) -> bool {
        let offset = self.crew_capacity_adjust(player);
        let capacity = derived_capacity.map(|c| add(c, offset));
        match op {
            TeamOp::CrewAddA2 | TeamOp::CrewSetA2 | TeamOp::CrewAddA1 | TeamOp::CrewSetA1 => {
                let Some(capacity) = capacity else { return false };
                let pool = self.crew_of(player).unwrap_or(Fixed::ZERO);
                let to = crew_pool_after(op, pool, x, capacity).expect("a crew op");
                let limit = self.officer_base(player);
                self.set_personnel(player, Some(bound(to)), limit);
                true
            }
            TeamOp::CapacityRaw => {
                let Some(derived) = derived_capacity else { return false };
                self.set_adjust(player, Some(Fixed::from_bits(x.to_bits().saturating_sub(derived.to_bits()))), None);
                true
            }
            TeamOp::OfficersAddA1 | TeamOp::OfficersSetA1 => {
                let Some(cap) = self.officer_limit(player) else { return false };
                let used = add(census, self.officer_adjust(player));
                let to = officer_count_after(op, used, x, cap).expect("an officer op");
                self.set_adjust(player, None, Some(Fixed::from_bits(to.to_bits().saturating_sub(census.to_bits()))));
                true
            }
        }
    }

    pub(super) fn hash_adjust(&self, h: &mut Fnv) {
        if self.adjust.is_empty() {
            return;
        }
        h.u64(0x6164_6a75_7374_7631);
        h.u64(self.adjust.len() as u64);
        for &(player, capacity, officers) in &self.adjust {
            h.u64(u64::from(player.0));
            h.fixed(capacity);
            h.fixed(officers);
        }
    }

    pub(crate) fn adjust_rec(&self) -> Vec<(PlayerId, i64, i64)> {
        self.adjust.iter().map(|&(p, c, o)| (p, c.to_bits(), o.to_bits())).collect()
    }

    pub(crate) fn restore_adjust(&mut self, rows: Vec<(PlayerId, i64, i64)>, players: &[(PlayerId, sta_core::TeamId)]) -> Result<(), &'static str> {
        let limit = LIMIT << 32;
        if rows.windows(2).any(|w| w[0].0 >= w[1].0)
            || rows.iter().any(|&(p, c, o)| !players.iter().any(|q| q.0 == p) || (c == 0 && o == 0) || c.unsigned_abs() > limit as u64 || o.unsigned_abs() > limit as u64)
        {
            return Err("invalid team adjustment rows");
        }
        self.adjust = rows.into_iter().map(|(p, c, o)| (p, Fixed::from_bits(c), Fixed::from_bits(o))).collect();
        Ok(())
    }
}
