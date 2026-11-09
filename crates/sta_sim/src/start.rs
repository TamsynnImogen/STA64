//! Starting forces (INT-M3): what each seat has at tick 0 besides the map's
//! own objects.
//!
//! [`Sim::from_setup`] places the map's objects only. [`Sim::from_setup_with_starts`]
//! then gives every seat that has a start position its race's [`Loadout`]
//! (`sta_content::Races`): the units on the start's spawn grid
//! ([`START_SPACING`] apart, the place nearest the start first, so the yard
//! stands on it) and the starting stock. A seat with no start position on the
//! map gets nothing, as before. Both the game and the map corpus test build
//! their skirmishes through this one function.

use sta_content::{Races, StartPos, Warning};
use sta_core::{Fixed, PlayerId};

use crate::setup::GameSetup;
use crate::Sim;

/// Distance between starting units, map units.
pub const START_SPACING: i32 = 250;

impl Sim {
    /// [`Sim::from_setup_with_warnings`], then each seat's starting forces on
    /// its start position (`MapDef::start_for(seat id)`).
    pub fn from_setup_with_starts(s: &GameSetup, races: &Races) -> (Sim, Vec<Warning>) {
        let (mut sim, warnings) = Sim::from_setup_with_warnings(s);
        for p in &s.players {
            let Some(start) = s.map.start_for(p.id.0) else { continue };
            let loadout = races.loadout(&s.content, &p.race);
            sim.place_start(p.id, start, &loadout);
        }
        (sim, warnings)
    }

    /// Place `loadout` for `player` on `start` and set its stock.
    pub fn place_start(&mut self, player: PlayerId, start: &StartPos, loadout: &sta_content::Loadout) {
        let places = start.spawn_points(loadout.units.len(), Fixed::from_int(START_SPACING));
        for (&key, (pos, heading)) in loadout.units.iter().zip(places) {
            self.spawn_def(key, player, pos, heading);
        }
        self.economy.set_stock(player, loadout.stock);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::{fake, Races};
    use sta_core::TeamId;

    use super::*;
    use crate::setup::{Control, GameOptions, PlayerSetup};
    use crate::WorldSource;

    fn setup() -> GameSetup {
        let player = |i: u8| PlayerSetup { id: PlayerId(i), team: TeamId(i), race: "fx".into(), control: Control::Idle };
        GameSetup {
            seed: 3,
            map: Arc::new(fake::two_bases()),
            content: Arc::new(fake::db()),
            players: vec![player(0), player(1), player(2)],
            options: GameOptions::default(),
        }
    }

    #[test]
    fn each_seat_with_a_start_gets_its_race_loadout() {
        let s = setup();
        let (sim, _) = Sim::from_setup_with_starts(&s, &Races::default());
        let view = sim.view();
        for seat in [0u8, 1] {
            let mine: Vec<_> = view.units.iter().filter(|u| u.owner == PlayerId(seat)).map(|u| s.content.name(u.def).unwrap()).collect();
            assert_eq!(mine, ["fx_yard", "fx_miner"], "seat {seat}");
            assert_eq!(view.players[seat as usize].resources[0], Fixed::from_int(4000));
        }
        // Slot 2 has no start position on the map: nothing.
        assert!(view.units.iter().all(|u| u.owner != PlayerId(2)));
        // The yard stands within a grid step of the start.
        let yard = view.units.iter().find(|u| u.owner == PlayerId(0)).unwrap();
        assert!((yard.pos.x - Fixed::from_int(-4000)).abs() <= Fixed::from_int(START_SPACING) && yard.pos.z.abs() <= Fixed::from_int(START_SPACING));
    }

    #[test]
    fn starts_are_deterministic() {
        let s = setup();
        let r = Races::default();
        assert_eq!(Sim::from_setup_with_starts(&s, &r).0.digest(), Sim::from_setup_with_starts(&s, &r).0.digest());
        assert_ne!(Sim::from_setup_with_starts(&s, &r).0.digest(), Sim::from_setup(&s).digest());
    }
}
