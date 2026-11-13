//! SIM-19: guarded ownership edits through the shared simulation.
//!
//! Station queues have checked cleanup, including a paid active head.
//! Rig cleanup and typed planet roles share the same transaction. Direct
//! unfinished-child transfer remains unsupported; see OWNERSHIP_TRANSFER.md.

use sta_core::{EntityId, Fixed, PlayerId};

use crate::Sim;
use crate::view::UnitFlags;
use crate::world::{EntityKind, NEUTRAL};

fn unit_team_change(from: Option<sta_core::TeamId>, to: Option<sta_core::TeamId>) -> bool {
    // A seat unknown to the setup (neutral, script) counts as a team of its own.
    from.is_none() || to.is_none() || from != to
}

impl Sim {
    pub(crate) fn transfer_owner(&mut self, from: PlayerId, id: EntityId, to: PlayerId) -> bool {
        if from != PlayerId::SCRIPT && from != PlayerId::BENCH {
            return false;
        }
        let destination = to == NEUTRAL
            || (to.is_player() && (self.players.is_empty() || self.players.iter().any(|&(p, _)| p == to)));
        if !destination || self.victory.is_eliminated(to) {
            return false;
        }
        let Some(unit) = self.units.get(id) else {
            return false;
        };
        let previous_owner = unit.owner;
        let perceived = self.perceived_owner(id).expect("validated live unit");
        if !crate::perception::representable(perceived) {
            return false;
        }
        let def = self.content.as_deref().and_then(|db| db.unit(unit.def));
        let planet = def.is_some_and(|d| d.planet_role.is_some());
        let background = def.is_some_and(|d| d.planet_role == Some(sta_content::PlanetRole::Background));
        if (!planet && !matches!(unit.kind, EntityKind::Craft | EntityKind::Station))
            || (unit.hull[0] <= Fixed::ZERO && !(background && unit.hull == [Fixed::ZERO; 2]))
            || unit.flags.contains(UnitFlags::DYING)
            || unit.progress.is_some()
            || (!planet && def.is_some_and(|def| def.resource.is_some()))
        {
            return false;
        }
        let Some(production) =
            self.production
                .prepare_transfer(self.content.as_deref(), &self.economy, &self.units, id, unit)
        else {
            return false;
        };

        // All rejection checks precede cleanup, including same-owner requests.
        // Consume the checked production plan; ordinary Stop/cancel has a separate
        // refund contract, particularly for deferred active production.
        let children = self.production.finish_transfer(&mut self.economy, production);
        for child in children {
            self.despawn(child, None);
        }
        self.weapons.stop_attack(&self.units, from, &[id]);
        self.economy.cancel(&self.units, from, &[id]);
        self.movement.clear(id);
        self.production.clear_builder_rally(id);
        if perceived == to {
            self.perception.remove(id);
        } else {
            self.perception.insert(id, perceived);
        }
        let team_of = |p: PlayerId| self.players.iter().find(|(q, _)| *q == p).map(|(_, t)| *t);
        let new_team = unit_team_change(team_of(previous_owner), team_of(to));
        let unit = self.units.get_mut(id).expect("validated live unit");
        unit.target = None;
        unit.owner = to;
        // `Craft::SetTeam` clears the special-forces flag (SIM-58), and the
        // craft's scripted order goes with its old owner's.
        self.scripted.remove(id);
        self.switches.edit(id, |s| s.special_forces = false);
        if new_team && !self.team_changes.contains(&id) {
            self.team_changes.push(id);
        }
        true
    }

    /// SIM-29: after a completed change of team, every special of the unit
    /// whose weapon says `activateOnTeamChange = 1` is engaged for the final
    /// owner on this tick's weapon pass, under the weapon's own rules (energy
    /// cost for a cloak; point defence also its reload under `weapon_shot_timing`; a refused one is dropped).
    /// Needs `Features::team_change_weapons`. A cloak is engaged (never
    /// toggled off), a point-defence weapon switched on if it is off. Same-team
    /// handovers and units handed to the neutral owner activate nothing (a unit going from the neutral owner to a player counts as a team change); the engine's alliance-only
    /// and race-only changes have no counterpart here.
    pub(crate) fn activate_team_change_weapons(&mut self, db: &sta_content::ContentDb) {
        for id in std::mem::take(&mut self.team_changes) {
            let Some(unit) = self.units.get(id).filter(|u| u.owner.is_player() && u.hull[0] > Fixed::ZERO) else { continue };
            let owner = unit.owner;
            let Some(def) = db.unit(unit.def) else { continue };
            let wanted: Vec<(usize, usize, crate::specials::SpecialKind)> = crate::specials::specials_indexed(db, def)
                .enumerate()
                .filter(|(_, (_, _, w))| w.activate_on_team_change && db.weapon_features(w).team_change_weapons)
                .map(|(slot, (i, kind, _))| (slot, i, kind))
                .collect();
            for (slot, i, kind) in wanted {
                match kind {
                    crate::specials::SpecialKind::Cloak => {
                        self.specials.order_cloak(db, &self.units, owner, &[id], true);
                    }
                    crate::specials::SpecialKind::PointDefence if !self.weapons.pd_is_on(id, i) => {
                        let (tick, units) = (self.tick, &self.units);
                        self.weapons.order_special(tick, db, units, &mut self.specials, &mut self.rng, &mut self.events, owner, &[id], slot as u8);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Planet contribution only, not total team crew capacity or crew stock.
    /// Derived from immutable definitions and current owners, so repeated swaps
    /// cannot double-credit it and snapshots need no mutable accumulator.
    pub fn planet_crew_capacity(&self, player: PlayerId) -> i64 {
        if !player.is_player() {
            return 0;
        }
        let Some(db) = self.content.as_deref() else { return 0 };
        self.units
            .iter()
            .filter(|u| u.owner == player)
            .filter_map(|u| db.unit(u.def))
            .filter(|d| d.planet_role == Some(sta_content::PlanetRole::Producer))
            .map(|d| i64::from(d.planet_crew_gain))
            .sum()
    }
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ownership_queue_tests.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "ownership_completion_tests.rs"]
mod completion_tests;

#[cfg(test)]
#[path = "ownership_team_change_tests.rs"]
mod team_change_tests;
