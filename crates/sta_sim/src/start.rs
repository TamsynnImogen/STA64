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
//!
//! Fleet Operations and Dominion War races list engine helpers among their
//! `standardUnits` (Roots' Federation: `all_ai_pod`, `all_start`,
//! `fed_effectsS`): `hidden = 1` pods with an empty model and minimap icon.
//! They are placed like any entry, and the view shows them to no player
//! ([`crate::world::EntityKind::hidden`]). What they do after tick 0 (expiry,
//! the replace weapons that swap them for AI and unit-cap pods) is SIM-28's;
//! see docs/START_HELPERS.md.

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

    /// `fake::db()` plus `fx_pod`, a start helper shaped like Roots'
    /// `all_ai_pod` / `fed_effectsS` with its `hidden` key as given.
    fn helper_setup(hidden: Option<&str>) -> (GameSetup, Races) {
        let fake = fake::db();
        let mut b = sta_content::ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            b = b.unit(u.clone());
        }
        let mut pod = sta_content::UnitDef::new("fx_pod", "pod");
        pod.race = "fx".into();
        pod.hull = Fixed::from_int(100);
        pod.model = "effect_empty".into();
        // As the ODF loader types `hidden` (whole number, nonzero is on).
        pod.hidden = hidden.is_some_and(|h| h.parse::<i64>().is_ok_and(|v| v != 0));
        b = b.unit(pod);
        let mut s = setup();
        s.content = Arc::new(b.build().unwrap());
        s.players[1].control = Control::Ai("test".into());
        let races = Races::from_defs(vec![RaceDef {
            stem: "fx".into(), name: "fx".into(), display_key: None, slot: Some(0),
            interface: None, interface_sprites: None,
            single_player_music: None, multiplayer_music: None,
            standard_units: vec!["fx_pod".into(), "fx_yard".into(), "fx_pod".into(), "fx_miner".into()],
            minimal_units: Vec::new(), stock: [7, 0, 0, 0],
            lots_stock: [0; 4], normal_crew: None, lots_crew: None,
        }]);
        (s, races)
    }

    fn names<'a>(s: &GameSetup, units: impl IntoIterator<Item = &'a crate::UnitView>) -> Vec<String> {
        units.into_iter().map(|u| s.content.name(u.def).unwrap().to_owned()).collect()
    }

    #[test]
    fn hidden_start_entries_exist_but_no_player_sees_or_picks_them() {
        let (s, races) = helper_setup(Some("1"));
        let (mut sim, warnings) = Sim::from_setup_with_starts(&s, &races);
        assert!(warnings.is_empty(), "{warnings:?}");
        for _ in 0..90 {
            sim.step(&[]);
        }
        let view = sim.view();
        for seat in [0u8, 1] {
            let p = PlayerId(seat);
            // Placed in authored order, so scripts and later lifecycle rules find them...
            let all: Vec<_> = view.units.iter().filter(|u| u.owner == p).collect();
            assert_eq!(names(&s, all.iter().copied()), ["fx_pod", "fx_yard", "fx_pod", "fx_miner"], "seat {seat}");
            for u in &all {
                let pod = s.content.name(u.def) == Some("fx_pod");
                assert_eq!(u.flags.contains(crate::view::UnitFlags::SELECTABLE), !pod, "seat {seat} {:?}", u.id);
            }
            // ...but neither their owner nor anyone else sees them.
            for viewer in [0u8, 1, 2] {
                let seen = view.visible_to(PlayerId(viewer));
                assert!(seen.iter().all(|u| s.content.name(u.def) != Some("fx_pod")), "viewer {viewer}");
            }
            let mine = view.visible_to(p).into_iter().filter(|u| u.owner == p);
            assert_eq!(names(&s, mine), ["fx_yard", "fx_miner"], "seat {seat}");
        }
    }

    #[test]
    fn the_hidden_key_decides_not_the_class() {
        // The same pod with `hidden = 0`, or no key, is an ordinary visible unit.
        for hidden in [Some("0"), None] {
            let (s, races) = helper_setup(hidden);
            let (sim, _) = Sim::from_setup_with_starts(&s, &races);
            let view = sim.view();
            let mine: Vec<_> = view.visible_to(PlayerId(0)).into_iter().filter(|u| u.owner == PlayerId(0)).collect();
            assert_eq!(names(&s, mine.iter().copied()), ["fx_pod", "fx_yard", "fx_pod", "fx_miner"], "{hidden:?}");
            assert!(mine.iter().all(|u| u.flags.contains(crate::view::UnitFlags::SELECTABLE)), "{hidden:?}");
        }
    }

    #[test]
    fn hiding_changes_the_view_only() {
        // The same start with `hidden = 1` and `hidden = 0` runs the same world:
        // after 30 ticks every unit's state is equal apart from SELECTABLE, and
        // only the visible sets differ. (The digests differ by the content hash
        // alone, since the two ODFs differ.)
        let run = |hidden| {
            let (s, races) = helper_setup(Some(hidden));
            let (mut sim, _) = Sim::from_setup_with_starts(&s, &races);
            for _ in 0..30 {
                sim.step(&[]);
            }
            let restored = Sim::from_snapshot(&sim.snapshot(), Some(s.content.clone())).unwrap();
            assert_eq!(restored.view(), sim.view(), "hidden = {hidden}");
            sim.view()
        };
        let (hid, shown) = (run("1"), run("0"));
        let strip = |v: &crate::WorldView| -> Vec<_> {
            v.units.iter().cloned().map(|mut u| {
                u.flags = u.flags.with(crate::view::UnitFlags::SELECTABLE, false);
                u
            }).collect()
        };
        assert_eq!(strip(&hid), strip(&shown));
        assert_eq!(hid.players, shown.players);
        assert_ne!(hid.visibility, shown.visibility);
    }
}
