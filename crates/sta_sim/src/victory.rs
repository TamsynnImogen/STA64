//! Victory and elimination (SIM-13).
//!
//! Rules:
//! - A seat is *alive* while it owns at least one craft or station. Resources,
//!   projectiles and props never count.
//! - Elimination is checked for a seat on the tick it loses a unit to damage,
//!   or after a command batch transfers away one of its craft/stations.
//!   Capture candidates require a configured seat; same-batch recovery survives
//!   (so a seat that starts with nothing is never eliminated). When the seat
//!   has no craft or station left, it gets one [`SimEvent::PlayerEliminated`]
//!   and joins the eliminated set for good.
//! - The game is over when at most one team has seats left that are not
//!   eliminated, and at least one seat has been eliminated ([`Victory::over`]).
//!   The seam has no game-over event, so [`Sim::winners`] is the query.
//! - Commands from eliminated seats are rejected.
//!
//! `GameOptions` has no victory-condition field yet, so this is the only rule.

use crate::event::SimEvent;
use crate::world::{Entities, EntityKind, Fnv};
use serde::{Deserialize, Serialize};
use sta_core::{PlayerId, TeamId};

/// The eliminated seats, ascending id. In the digest once non-empty.
#[derive(Debug, Clone, Default)]
pub struct Victory {
    eliminated: Vec<PlayerId>,
}

/// Snapshot row (SIM-10 must carry it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VictoryRec {
    pub eliminated: Vec<u8>,
}

impl Victory {
    pub fn is_empty(&self) -> bool {
        self.eliminated.is_empty()
    }

    pub fn is_eliminated(&self, p: PlayerId) -> bool {
        self.eliminated.binary_search(&p).is_ok()
    }

    pub fn eliminated(&self) -> &[PlayerId] {
        &self.eliminated
    }

    /// Whether `p` still owns a craft or a station.
    fn has_assets(units: &Entities, p: PlayerId) -> bool {
        units
            .iter()
            .any(|u| u.owner == p && matches!(u.kind, EntityKind::Craft | EntityKind::Station))
    }

    /// Eliminate those of `lost` (seats that lost a unit this tick) that have
    /// nothing left, ascending id, once each.
    pub(crate) fn step(
        &mut self,
        units: &Entities,
        lost: &mut Vec<PlayerId>,
        events: &mut Vec<SimEvent>,
    ) {
        lost.sort();
        lost.dedup();
        for &p in lost.iter() {
            if self.is_eliminated(p) || Self::has_assets(units, p) {
                continue;
            }
            let at = self.eliminated.binary_search(&p).unwrap_err();
            self.eliminated.insert(at, p);
            events.push(SimEvent::PlayerEliminated { player: p });
        }
    }

    /// The seats of the one team left, once the game is over.
    pub fn winners(&self, players: &[(PlayerId, TeamId)]) -> Option<Vec<PlayerId>> {
        if self.eliminated.is_empty() {
            return None;
        }
        let mut teams: Vec<TeamId> = players
            .iter()
            .filter(|(p, _)| !self.is_eliminated(*p))
            .map(|&(_, t)| t)
            .collect();
        teams.sort();
        teams.dedup();
        if teams.len() > 1 {
            return None;
        }
        Some(
            players
                .iter()
                .filter(|(p, t)| !self.is_eliminated(*p) && teams.contains(t))
                .map(|&(p, _)| p)
                .collect(),
        )
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.u64(self.eliminated.len() as u64);
        for p in &self.eliminated {
            h.u64(p.0 as u64);
        }
    }

    pub fn to_rec(&self) -> VictoryRec {
        VictoryRec {
            eliminated: self.eliminated.iter().map(|p| p.0).collect(),
        }
    }

    pub fn from_rec(r: &VictoryRec) -> Victory {
        let mut eliminated: Vec<PlayerId> = r.eliminated.iter().map(|&p| PlayerId(p)).collect();
        eliminated.sort();
        eliminated.dedup();
        Victory { eliminated }
    }
}

impl crate::Sim {
    /// Whether `p` has been eliminated.
    pub fn is_eliminated(&self, p: PlayerId) -> bool {
        self.victory.is_eliminated(p)
    }

    /// The winning seats once only one team is left, else `None`.
    pub fn winners(&self) -> Option<Vec<PlayerId>> {
        self.victory.winners(&self.players)
    }
}

#[cfg(test)]
#[path = "victory_tests.rs"]
mod tests;
