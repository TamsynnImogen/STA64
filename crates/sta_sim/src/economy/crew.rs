//! Team crew regeneration (SIM-25): the player's crew stock grows by
//! `crewAccumulationRate` per second for each finished starbase it owns,
//! up to its crew capacity.
//!
//! - Rate: the owning race's `crewAccumulationRate` (race ODF, crew per second
//!   per starbase, `sta_content::CrewRules`), looked up by the starbase's own
//!   `race`. A race without the key, or with 0 (Fleet Ops Roots' playable
//!   races), adds nothing. A starbase is a unit of class `starbase`.
//! - Capacity: the larger of `STARTING_CREW_CAPACITY` (`RTS_CFG.h`) and the sum
//!   of the owned finished stations' `maxCrewGain` plus owned Producer planets'
//!   `planet_crew_gain` (capped at 1e9). A stock above the capacity
//!   (capacity shrank) is left as it is, never clamped down.
//! - Fractions use the same exact scheme as [`super::passive`]: no state.
//! - Station `maxCrewGain` is read non-negative (`Reader::fixed`) and floored to whole
//!   crew. Capacity as the larger of the two terms (not their sum) fits the one
//!   measurement taken (cap below 3005 with a 3000 starbase and 250 base capacity);
//!   the exact cap is not measured.
//! - Not implemented, with the reason: planets' own `crewAccumulationRate` (Classic
//!   Enhanced planets carry the key; not measured); `planetCrewAccumulationModifier`
//!   (what counts as "close to a planet" is not measured); `crewRetreatRatio`,
//!   the yard refill (`CfgRules::yard_crew_per_sec`, `BASE_CREW_PER_SEC`, default 20). Per-unit crew: loss from hits, system deaths, the
//!   life-support drain and the derelict latch is `combat/casualty.rs`
//!   (SIM-39); boarding losses are not implemented (SIM-46), and recovery
//!   (station RECREW at `recrewRate`, yard recrew at `yard_crew_per_sec` from the team
//!   pool) is not either; a ship does not regain crew by itself (measured).

use sta_content::ContentDb;
use sta_core::{Fixed, PlayerId, TICK_HZ};

use super::Economy;
use crate::view::UnitFlags;
use crate::world::{Entities, EntityKind};

fn tick_gain(rate_bits: i128, tick: u64) -> i128 {
    let hz = i128::from(TICK_HZ);
    let at = |t: i128| (rate_bits * t).div_euclid(hz);
    let t = i128::from(tick);
    at(t + 1) - at(t)
}

impl Economy {
    /// One tick of every initialized player's crew stock.
    pub(crate) fn step_crew(&mut self, units: &Entities, db: &ContentDb, attrs: &crate::attrs::Attrs, tick: u64) {
        let Some(rules) = db.rules().crew() else { return };
        if self.personnel.is_empty() {
            return;
        }
        for i in 0..self.personnel.len() {
            let (player, Some(crew), limit) = self.personnel[i] else { continue };
            let (rate, gain_cap) = team_numbers(units, db, attrs, rules, player);
            let cap = capacity_of(rules, gain_cap, self.crew_capacity_adjust(player));
            if rate <= 0 || crew >= cap {
                continue;
            }
            let add = Fixed::from_bits(tick_gain(rate, tick).min(i128::from(i64::MAX)) as i64);
            self.personnel[i] = (player, Some(Fixed::from_bits(crew.to_bits().saturating_add(add.to_bits())).min(cap)), limit);
        }
    }
}

/// The crew capacity: the larger of the starting and the station and planet
/// capacity, at most `MAX_CREW` (`AddCrewCapacity` clamps its sum to it), plus
/// what a script wrote with Roots `Player.maxCrew` (a raw write that later
/// capacity changes shift by their delta; SIM-54, read from the executable, static).
fn capacity_of(rules: &sta_content::CrewRules, gain_cap: Fixed, offset: Fixed) -> Fixed {
    const LIMIT: i64 = 1_000_000_000;
    let mut cap = Fixed::from_int(rules.starting_capacity.unwrap_or(0).clamp(0, LIMIT) as i32).max(gain_cap);
    if let Some(max) = rules.max_crew {
        cap = cap.min(Fixed::from_int(max.clamp(0, LIMIT) as i32));
    }
    Fixed::from_bits(cap.to_bits().saturating_add(offset.to_bits()))
}

impl Economy {
    /// The capacity of `player`'s crew pool without a script offset (None when
    /// the content has no crew rules).
    pub(crate) fn derived_crew_capacity(units: &Entities, db: &ContentDb, attrs: &crate::attrs::Attrs, player: PlayerId) -> Option<Fixed> {
        let rules = db.rules().crew()?;
        let (_, gain_cap) = team_numbers(units, db, attrs, rules, player);
        Some(capacity_of(rules, gain_cap, Fixed::ZERO))
    }
}

/// `(accumulation bits per second, capacity from stations and planets)` of one team.
/// A starbase's rate is that of its own race, which a script may have changed.
fn team_numbers(units: &Entities, db: &ContentDb, attrs: &crate::attrs::Attrs, rules: &sta_content::CrewRules, player: PlayerId) -> (i128, Fixed) {
    const LIMIT: i128 = 1_000_000_000;
    let (mut rate, mut capacity) = (0i128, 0i128);
    for u in units.iter() {
        if u.owner != player || u.flags.contains(UnitFlags::DYING) || u.progress.is_some_and(|p| p < Fixed::ONE) {
            continue;
        }
        let Some(def) = db.unit(u.def) else { continue };
        if def.planet_role == Some(sta_content::PlanetRole::Producer) {
            capacity += i128::from(def.planet_crew_gain.max(0));
            continue;
        }
        if u.kind != EntityKind::Station {
            continue;
        }
        capacity += i128::from(def.crew_gain.to_bits()) >> 32;
        if def.class.0 == "starbase"
            && let Some((_, r)) = rules.races.iter().find(|(k, _)| k.eq_ignore_ascii_case(attrs.race(u.id).unwrap_or(def.race.as_str()).trim()))
        {
            rate += i128::from(r.to_bits());
        }
    }
    (rate, Fixed::from_int(capacity.clamp(0, LIMIT) as i32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, CommandKind, Edit};
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use crate::{Sim, WorldSource};
    use sta_content::{CrewRules, MapDef, Rules, UnitDef};
    use sta_core::{TeamId, Vec3};
    use std::sync::Arc;

    fn db(rate: f64, starting: Option<i64>, gain: &str) -> Arc<ContentDb> {
        let fake = sta_content::fake::db();
        let mut b = sta_content::ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons().cloned() {
            b = b.weapon(w);
        }
        for u in fake.units().cloned() {
            b = b.unit(u);
        }
        let mut base = UnitDef::new("fx_starbase", "starbase");
        base.race = "fx".into();
        base.hull = Fixed::from_int(500);
        base.crew_gain = Fixed::from_f64(gain.split('=').nth(1).and_then(|v| v.trim().parse().ok()).unwrap_or(0.0));
        let rules = Rules::default().with_crew(CrewRules { starting_capacity: starting, max_crew: None, races: vec![("fx".into(), Fixed::from_f64(rate))] });
        Arc::new(b.unit(base).build().unwrap().with_rules(rules))
    }

    fn game(db: Arc<ContentDb>, crew: i32) -> Sim {
        let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human };
        let mut sim = Sim::from_setup(&GameSetup {
            seed: 1,
            map: Arc::new(MapDef::empty()),
            content: db,
            players: vec![seat(0), seat(1)],
            options: GameOptions::default(),
        });
        sim.economy.set_personnel(PlayerId(0), Some(Fixed::from_int(crew)), None);
        sim.economy.set_personnel(PlayerId(1), Some(Fixed::from_int(crew)), None);
        sim
    }

    fn spawn(sim: &mut Sim, odf: &str, player: u8) {
        let key = sim.content().unwrap().key(odf).unwrap();
        let at = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };
        let e = Edit::Spawn { odf: key, player: PlayerId(player), pos: at, heading: (Fixed::ZERO, Fixed::ONE) };
        sim.step(&[Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(e) }]);
    }

    fn run(sim: &mut Sim, ticks: u32) {
        for _ in 0..ticks {
            sim.step(&[]);
        }
    }

    fn crew(sim: &Sim, p: u8) -> Fixed {
        sim.economy.crew_of(PlayerId(p)).unwrap()
    }

    fn close(a: Fixed, b: Fixed) {
        assert!((a.to_bits() - b.to_bits()).abs() <= Fixed::from_f64(0.5).to_bits(), "{a:?} vs {b:?}");
    }

    #[test]
    fn each_starbase_adds_the_race_rate_per_second() {
        let mut sim = game(db(4.0, Some(250), "maxCrewGain = 3000\n"), 750);
        spawn(&mut sim, "fx_starbase", 0);
        run(&mut sim, 30 * 10);
        close(crew(&sim, 0), Fixed::from_int(750 + 40));
        assert_eq!(crew(&sim, 1), Fixed::from_int(750), "another seat gains nothing");
        spawn(&mut sim, "fx_starbase", 0);
        let before = crew(&sim, 0);
        run(&mut sim, 30 * 10);
        close(crew(&sim, 0) - before, Fixed::from_int(80));
    }

    #[test]
    fn crew_stops_at_the_capacity_and_is_never_clamped_down() {
        let mut sim = game(db(4.0, Some(50), "maxCrewGain = 100\n"), 90);
        spawn(&mut sim, "fx_starbase", 0);
        run(&mut sim, 30 * 20);
        assert_eq!(crew(&sim, 0), Fixed::from_int(100));
        let mut sim = game(db(4.0, Some(50), "maxCrewGain = 100\n"), 5000);
        spawn(&mut sim, "fx_starbase", 0);
        run(&mut sim, 30 * 5);
        assert_eq!(crew(&sim, 0), Fixed::from_int(5000));
    }

    #[test]
    fn the_starting_capacity_is_the_floor() {
        let mut sim = game(db(4.0, Some(250), ""), 240);
        spawn(&mut sim, "fx_starbase", 0);
        run(&mut sim, 30 * 20);
        assert_eq!(crew(&sim, 0), Fixed::from_int(250));
    }

    #[test]
    fn zero_rate_races_and_missing_keys_add_nothing() {
        let mut sim = game(db(0.0, Some(250), "maxCrewGain = 3000\n"), 750);
        spawn(&mut sim, "fx_starbase", 0);
        run(&mut sim, 30 * 10);
        assert_eq!(crew(&sim, 0), Fixed::from_int(750));
        let mut sim = game(Arc::new(sta_content::fake::db()), 750);
        run(&mut sim, 30 * 10);
        assert_eq!(crew(&sim, 0), Fixed::from_int(750));
    }

    #[test]
    fn station_capacity_is_part_of_the_content_hash_and_a_foreign_base_gives_nothing() {
        assert_ne!(db(4.0, Some(250), "maxCrewGain = 3000\n").hash(), db(4.0, Some(250), "maxCrewGain = 3001\n").hash());
        assert_ne!(db(4.0, Some(250), "").hash(), db(4.0, Some(250), "maxCrewGain = 5\n").hash());
        let mut sim = game(db(4.0, Some(250), "maxCrewGain = 3000\n"), 750);
        spawn(&mut sim, "fx_starbase", 1);
        run(&mut sim, 30 * 10);
        assert_eq!(crew(&sim, 0), Fixed::from_int(750), "an enemy starbase feeds its own owner only");
        assert!(crew(&sim, 1) > Fixed::from_int(750));
    }

    #[test]
    fn crew_rules_reach_the_content_hash() {
        let base = db(4.0, Some(250), "maxCrewGain = 3000\n").hash();
        assert_ne!(base, db(5.0, Some(250), "maxCrewGain = 3000\n").hash(), "race rate");
        assert_ne!(base, db(4.0, Some(251), "maxCrewGain = 3000\n").hash(), "starting capacity");
    }

    #[test]
    fn an_unfinished_starbase_adds_no_crew() {
        let mut sim = game(db(4.0, Some(250), "maxCrewGain = 3000\n"), 750);
        let key = sim.content().unwrap().key("fx_starbase").unwrap();
        let cmd = Edit::Spawn { odf: key, player: PlayerId(0), pos: Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO }, heading: (Fixed::ZERO, Fixed::ONE) };
        sim.step(&[Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(cmd) }]);
        let id = sim.units.iter().next().unwrap().id;
        sim.units.get_mut(id).unwrap().progress = Some(Fixed::from_f64(0.5));
        let before = crew(&sim, 0);
        run(&mut sim, 30 * 5);
        assert_eq!(crew(&sim, 0), before, "unfinished starbase");
    }

    #[test]
    fn snapshot_round_trip_and_replay_digest_are_stable() {
        let db = db(4.0, Some(250), "maxCrewGain = 3000\n");
        let mut live = game(Arc::clone(&db), 750);
        spawn(&mut live, "fx_starbase", 0);
        run(&mut live, 77);
        let mut copy = Sim::from_snapshot(&live.snapshot(), Some(db)).unwrap();
        assert_eq!(copy.digest(), live.digest());
        for _ in 0..120 {
            live.step(&[]);
            copy.step(&[]);
            assert_eq!(copy.digest(), live.digest());
        }
        assert!(crew(&live, 0) > Fixed::from_int(750));
    }
}
