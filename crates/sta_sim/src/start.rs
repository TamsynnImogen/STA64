//! Starting forces (INT-M3): what each seat has at tick 0 besides the map's
//! own objects.
//!
//! [`Sim::from_setup`] places the map's objects only. [`Sim::from_setup_with_starts`]
//! then gives every seat that has a start position its race's [`Loadout`]
//! (`sta_content::Races`): the units on the start's spawn grid
//! ([`START_SPACING`] apart, the first authored entry nearest the start) and
//! the starting stock. A seat with no start position on the
//! map gets nothing, as before. Both the game and the map corpus test build
//! their skirmishes through this one function.

use sta_content::{Races, StartPos, Warning};
use sta_core::{Fixed, PlayerId};
use std::collections::BTreeSet;

use crate::setup::GameSetup;
use crate::Sim;

/// Distance between starting units, map units.
pub const START_SPACING: i32 = 250;

impl Sim {
    /// [`Sim::from_setup_with_warnings`], then each seat's starting forces on
    /// its start position (`MapDef::start_for(seat id)`).
    pub fn from_setup_with_starts(s: &GameSetup, races: &Races) -> (Sim, Vec<Warning>) {
        let (mut sim, mut warnings) = Sim::from_setup_with_warnings(s);
        let mut warned = BTreeSet::new();
        for p in &s.players {
            let Some(start) = s.map.start_for(p.id.0) else { continue };
            if let Some(race) = races.get_playable(&p.race).or_else(|| races.get(&p.race)) {
                for (index, odf) in race.standard_units.iter().enumerate() {
                    if s.content.key(odf).is_some_and(|key| s.content.unit(key).is_some()) {
                        continue;
                    }
                    if warned.insert((race.stem.clone(), index)) {
                        warnings.push(Warning::new(
                            format!("{}.odf", race.stem),
                            format!("race {}: standardUnits entry for {odf}.odf is unresolved or unsupported; not spawned", race.name),
                        ));
                    }
                }
            }
            let loadout = races.loadout_with_mode(&s.content, &p.race, s.options.starting_resources);
            sim.place_start(p.id, start, &loadout);
            let limit = s.content.rules().officer_limit(s.options.officer_limit as usize).map(Fixed::from_int);
            if limit.is_none() && !s.content.rules().officer_limits.is_empty() {
                warnings.push(Warning::new("RTS_CFG.h", format!("officer choice {} is unavailable for player {}", s.options.officer_limit, p.id.0)));
            }
            sim.economy.set_personnel(p.id, loadout.crew, limit);
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
        self.economy.set_personnel(player, loadout.crew, None);
    }
}

/// A portable hash of what [`Sim::from_setup_with_starts`] would give each
/// seat of `s` from `races`: per seat that has a start on the map, the seat
/// id, the resolved units (by content name, in spawn order), the stock and the
/// crew. A replay stores it so that a replay rebuilt on other race data is
/// refused instead of desyncing; the rules themselves stay in the race ODFs.
/// FNV-1a over fixed-width little-endian fields, equal on every platform.
pub fn loadouts_hash(s: &GameSetup, races: &Races) -> u64 {
    fn eat(h: &mut u64, bytes: &[u8]) {
        for &b in bytes {
            *h ^= u64::from(b);
            *h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in &s.players {
        if s.map.start_for(p.id.0).is_none() {
            continue;
        }
        let loadout = races.loadout_with_mode(&s.content, &p.race, s.options.starting_resources);
        eat(&mut h, &[p.id.0]);
        eat(&mut h, &(loadout.units.len() as u32).to_le_bytes());
        for &key in &loadout.units {
            let name = s.content.name(key).unwrap_or("");
            eat(&mut h, &(name.len() as u32).to_le_bytes());
            eat(&mut h, name.as_bytes());
        }
        for v in loadout.stock {
            eat(&mut h, &v.to_bits().to_le_bytes());
        }
        match loadout.crew {
            Some(c) => {
                eat(&mut h, &[1]);
                eat(&mut h, &c.to_bits().to_le_bytes());
            }
            None => eat(&mut h, &[0]),
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::{fake, RaceDef, Races};
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

    #[test]
    fn unresolved_authored_entries_warn_once_across_seats() {
        // Authored placement is covered here; missing FO lifecycle effects are separate.
        let s = setup();
        let races = Races::from_defs(vec![RaceDef {
            stem: "fx".into(), name: "fx".into(), display_key: None, slot: Some(0),
            interface: None, interface_sprites: None,
            single_player_music: None, multiplayer_music: None,
            standard_units: vec!["fx_station".into(), "missing".into(), "fx_beam".into(), "fx_station".into()],
            minimal_units: vec!["fx_miner".into()], stock: [7, 0, 0, 0],
            lots_stock: [0; 4], normal_crew: None, lots_crew: None,
        }]);
        let (sim, warnings) = Sim::from_setup_with_starts(&s, &races);
        let mine: Vec<_> = sim.view().units.iter()
            .filter(|u| u.owner == PlayerId(0))
            .map(|u| s.content.name(u.def).unwrap()).collect();
        assert_eq!(mine, ["fx_station", "fx_station"]);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings.iter().all(|w| w.file == "fx.odf" && w.message.contains("race fx")));
        assert!(warnings.iter().any(|w| w.message.contains("missing.odf") && w.message.contains("unresolved or unsupported")));
        assert!(warnings.iter().any(|w| w.message.contains("fx_beam.odf") && w.message.contains("unresolved or unsupported")));
        assert_eq!(sim.view().players[0].resources[0], Fixed::from_int(7));
    }
}
