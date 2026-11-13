//! Passive resource income (SIM-24): `resourceweapon` hardpoints that add
//! resources to their owner's stock.
//!
//! Source: Fleet Operations 3.x and later (Roots, Fleet Ops 4.0, Dominion
//! War); stock Armada I and II have no such class. A unit or station lists
//! the weapon like any other (`f_generate_supply` on `fbase`, hardpoint
//! `root`), and the engine applies `addDilithium`, `addLatinum`, `addMetal`,
//! `addBiomatter` and `addCrew` to the owner. `alwaysActivePlayer` / `alwaysActiveAI`
//! say whether the weapon works without being triggered for a human or an AI
//! seat; with neither set it is a button ability (`bor_recycle_dilithium`,
//! `buttonslot = 1`), which this module leaves alone.
//!
//! - Amounts come from the weapon's ODF ([`sta_content::ResourceWeaponDef`]),
//!   never from a constant here. They are signed; the stock never drops below
//!   zero.
//! - The time base is [`SECONDS_PER_CYCLE`]: see the measurement note there.
//! - Fractions accumulate without drift and without stored state: the gain of
//!   tick `t` is `floor(rate * (t + 1) / HZ) - floor(rate * t / HZ)` on the
//!   exact fixed-point bits, so any whole second adds exactly the ODF amount
//!   and the sim keeps no remainder to snapshot or hash.
//! - Any carrier counts, hidden `pod` research items included (`borR_connections*`
//!   add metal per second); a station still being built, or dying, produces nothing.
//!
//! Not yet: `oneTimeMode` weapons (lump sums such as `all_supply_routes_bio`
//! `addBiomatter = 250`) ride on hidden `pod` items that the sim does not
//! spawn, so there is no activation moment to grant them at; they are skipped,
//! never run as a rate. Button-activated resource weapons are skipped too.

use sta_content::{ContentDb, ResourceWeaponDef};
use sta_core::{Fixed, PlayerId, TICK_HZ};

use super::{Economy, RESOURCES};
use crate::view::UnitFlags;
use crate::world::{Entities, EntityKind};

/// Game seconds one application of the `add*` amounts takes: the amounts are
/// per second, applied continuously (evidence grade: measured on the original
/// Roots for `addbiomatter`; FO 3.2.7 and FO 4.0 not measured, the ODFs carry
/// no shot delay to say otherwise).
pub const SECONDS_PER_CYCLE: i64 = 1;

/// Whole-bit share of `rate` that falls into tick `tick`.
fn gain(rate: Fixed, tick: u64) -> Fixed {
    let (hz, cycle) = (i128::from(TICK_HZ), i128::from(SECONDS_PER_CYCLE));
    let bits = i128::from(rate.to_bits());
    let at = |t: i128| (bits * t).div_euclid(hz * cycle);
    let t = i128::from(tick);
    Fixed::from_bits((at(t + 1) - at(t)) as i64)
}

/// Whether `w` works for `owner` without being triggered.
fn active(w: &ResourceWeaponDef, ai: bool) -> bool {
    !w.one_time && if ai { w.always_ai } else { w.always_player }
}

impl Economy {
    /// One tick of every passive resource weapon, ascending entity id.
    pub(crate) fn step_passive(&mut self, units: &Entities, db: &ContentDb, players: &[(PlayerId, sta_core::TeamId)], ai_seats: &[PlayerId], tick: u64) {
        for u in units.iter() {
            if u.kind == EntityKind::Projectile
                || u.flags.contains(UnitFlags::DYING)
                || u.progress.is_some_and(|p| p < Fixed::ONE)
                || !players.iter().any(|(p, _)| *p == u.owner)
            {
                continue;
            }
            let Some(def) = db.unit(u.def) else { continue };
            let ai = ai_seats.contains(&u.owner);
            for hp in &def.hardpoints {
                let Some(r) = db.weapon(hp.weapon.key).and_then(|w| w.resource.as_ref()) else { continue };
                if !active(r, ai) {
                    continue;
                }
                // One application is all or nothing: a weapon that converts
                // (positive and negative adders) does not pay out what it
                // cannot afford. (Our choice; not measured on the original.)
                let mut g = [Fixed::ZERO; RESOURCES];
                for (slot, add) in r.add.iter().enumerate() {
                    if *add != Fixed::ZERO {
                        g[slot] = gain(*add, tick);
                    }
                }
                let crew_gain = if r.crew != Fixed::ZERO { gain(r.crew, tick) } else { Fixed::ZERO };
                let have = self.stock_of(u.owner);
                let held = |slot: usize| have.get(slot).copied().unwrap_or(Fixed::ZERO);
                let crew_have = self.crew_of(u.owner);
                if (0..RESOURCES).any(|s| g[s] < Fixed::ZERO && held(s) + g[s] < Fixed::ZERO)
                    || (crew_gain < Fixed::ZERO && crew_have.is_some_and(|c| c + crew_gain < Fixed::ZERO))
                {
                    continue;
                }
                for (slot, add) in g.iter().enumerate() {
                    if *add != Fixed::ZERO {
                        let s = &mut self.slot(u.owner)[slot];
                        *s = Fixed::from_bits(s.to_bits().saturating_add(add.to_bits())).max(Fixed::ZERO);
                    }
                }
                if crew_gain != Fixed::ZERO
                    && let Some(c) = crew_have
                {
                    let limit = self.officer_limit(u.owner);
                    self.set_personnel(u.owner, Some(Fixed::from_bits(c.to_bits().saturating_add(crew_gain.to_bits())).max(Fixed::ZERO)), limit);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, CommandKind, Edit};
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use crate::{Sim, WorldSource};
    use sta_content::def::{DefRef, Hardpoint};
    use sta_content::{MapDef, UnitDef, WeaponDef};
    use sta_core::{TeamId, Vec3};
    use std::sync::Arc;

    // Own fixtures: only the keys and values the rule reads, named after the
    // installed weapons whose shape they follow.
    const F_SUPPLY: &str = "addbiomatter = 0.2\nalwaysactiveplayer = 1\nalwaysactiveai = 1\nclasslabel = \"resourceweapon\"\n";
    const GENERATE: &str = "addDilithium = 2.6\naddMetal = 1.255\naddBiomatter = 0.2\nalwaysactiveplayer = 1\nalwaysactiveai = 1\nclasslabel = \"resourceweapon\"\n";
    const AI_SUPPLY: &str = "addbiomatter = 5\nalwaysactiveai = 1\nclasslabel = \"resourceweapon\"\n";
    const ONE_TIME: &str = "addmetal = 20\nalwaysactiveplayer = 1\nalwaysactiveai = 1\nonetimemode = 1\nclasslabel = \"resourceweapon\"\n";
    const DRAIN: &str = "oneTimeMode = 0\naddBiomatter = -200\nalwaysactiveplayer = 1\nalwaysactiveai = 1\nclasslabel = \"resourceweapon\"\n";
    const BUTTON: &str = "buttonslot = 1\nadddilithium = 5\naddbiomatter = -0.33\nclasslabel = \"resourceweapon\"\n";

    fn load(name: &str, text: &str) -> WeaponDef {
        let odf = sta_odf::resolve(name, &mut |_| Some(text.to_owned())).unwrap();
        let mut warnings = Vec::new();
        let (w, _) = sta_content::weapon::weapon_def(name, &odf, None, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        w
    }

    /// `fake::db()` plus one station per weapon, `fx_<weapon>`.
    fn db(weapons: &[(&str, &str)]) -> Arc<sta_content::ContentDb> {
        let fake = sta_content::fake::db();
        let mut b = sta_content::ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons().cloned() {
            b = b.weapon(w);
        }
        for u in fake.units().cloned() {
            b = b.unit(u);
        }
        for (name, text) in weapons {
            b = b.weapon(load(name, text));
            let mut u = UnitDef::new(&format!("fx_{name}"), "starbase");
            u.race = "fx".into();
            u.hull = Fixed::from_int(500);
            u.hardpoints = vec![Hardpoint { weapon: DefRef::new(*name), node: "root".into(), arc: None }];
            b = b.unit(u);
        }
        Arc::new(b.build().unwrap())
    }

    fn game(db: Arc<sta_content::ContentDb>) -> Sim {
        let seat = |id, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
        Sim::from_setup(&GameSetup {
            seed: 1,
            map: Arc::new(MapDef::empty()),
            content: db,
            players: vec![seat(0, Control::Human), seat(1, Control::Ai("idle".into())), seat(2, Control::Idle)],
            options: GameOptions::default(),
        })
    }

    fn spawn(sim: &mut Sim, odf: &str, player: u8) {
        let key = sim.content().unwrap().key(odf).unwrap();
        let at = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };
        let e = Edit::Spawn { odf: key, player: PlayerId(player), pos: at, heading: (Fixed::ZERO, Fixed::ONE) };
        let c = Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(e) };
        sim.step(&[c]);
    }

    fn stock(sim: &Sim, p: u8) -> [Fixed; RESOURCES] {
        let v = sim.economy.stock_of(PlayerId(p));
        let mut out = [Fixed::ZERO; RESOURCES];
        for (o, s) in out.iter_mut().zip(v) {
            *o = s;
        }
        out
    }

    fn run(sim: &mut Sim, ticks: u32) {
        for _ in 0..ticks {
            sim.step(&[]);
        }
    }

    const SECOND: u32 = TICK_HZ;

    /// An ODF amount as the loader reads it (through `f32`).
    fn odf(v: f32) -> Fixed {
        Fixed::from_f64(f64::from(v))
    }

    /// Equal to within a tick's gain: a carrier spawned a tick later has another phase.
    fn near(a: Fixed, b: Fixed) {
        let tolerance = over(Fixed::from_int(5), 2).to_bits().max(2);
        assert!((a.to_bits() - b.to_bits()).abs() <= tolerance, "{a:?} vs {b:?}");
    }

    /// Exact gain of `rate` per second over `ticks` ticks from tick 0 (the
    /// carrier is spawned by the first step, which already counts one tick).
    fn over(rate: Fixed, ticks: u32) -> Fixed {
        Fixed::from_bits((i128::from(rate.to_bits()) * i128::from(ticks) / i128::from(TICK_HZ)) as i64)
    }

    #[test]
    fn keys_are_read_from_the_odf() {
        let w = load("f_generate_supply", F_SUPPLY);
        let r = w.resource.unwrap();
        assert_eq!(r.add, [Fixed::ZERO, Fixed::ZERO, Fixed::ZERO, odf(0.2)]);
        assert!(r.always_player && r.always_ai && !r.one_time);
        let r = load("generate_resources", GENERATE).resource.unwrap();
        assert_eq!(r.add, [odf(2.6), Fixed::ZERO, odf(1.255), odf(0.2)]);
        let r = load("rsteal_2", DRAIN).resource.unwrap();
        assert_eq!(r.add[3], Fixed::from_int(-200));
        assert!(load("bor_20priority", ONE_TIME).resource.unwrap().one_time);
        assert!(load("phaser", "classlabel = \"cannon\"\n").resource.is_none());
    }

    #[test]
    fn supply_station_raises_its_owners_biomatter_at_the_odf_rate_and_nothing_else() {
        let mut sim = game(db(&[("f_generate_supply", F_SUPPLY)]));
        spawn(&mut sim, "fx_f_generate_supply", 0);
        let before = stock(&sim, 0);
        run(&mut sim, 60 * SECOND);
        let after = stock(&sim, 0);
        let rate = odf(0.2);
        // A whole number of seconds adds exactly rate * seconds; no drift.
        assert_eq!(after[3], over(rate, 60 * SECOND + 1));
        assert_eq!(&after[..3], &before[..3]);
        assert_eq!(stock(&sim, 1), [Fixed::ZERO; RESOURCES]);
        assert_eq!(stock(&sim, 2), [Fixed::ZERO; RESOURCES]);
    }

    #[test]
    fn fractions_accumulate_without_drift_over_any_window() {
        let mut sim = game(db(&[("f_generate_supply", F_SUPPLY)]));
        spawn(&mut sim, "fx_f_generate_supply", 0);
        let start = stock(&sim, 0)[3];
        let t0 = sim.tick;
        let bits = odf(0.2).to_bits() as i128;
        for n in 1..=200u32 {
            sim.step(&[]);
            let want = (bits * i128::from(sim.tick - t0)).div_euclid(i128::from(TICK_HZ));
            let got = (stock(&sim, 0)[3] - start).to_bits() as i128;
            // The window starts on an arbitrary tick: at most one bit away.
            assert!((got - want).abs() <= 1, "tick {n}: {got} vs {want}");
        }
        assert!(stock(&sim, 0)[3] > start);
    }

    #[test]
    fn several_resources_from_one_weapon() {
        let mut sim = game(db(&[("generate_resources", GENERATE)]));
        spawn(&mut sim, "fx_generate_resources", 1);
        run(&mut sim, 10 * SECOND);
        let s = stock(&sim, 1);
        let n = 10 * SECOND + 1;
        assert_eq!(s[0], over(odf(2.6), n));
        assert_eq!(s[1], Fixed::ZERO);
        assert_eq!(s[2], over(odf(1.255), n));
        assert_eq!(s[3], over(odf(0.2), n));
    }

    #[test]
    fn alwaysactive_gates_split_human_and_ai_seats() {
        let mut sim = game(db(&[("all_ai_supply", AI_SUPPLY)]));
        spawn(&mut sim, "fx_all_ai_supply", 0);
        spawn(&mut sim, "fx_all_ai_supply", 1);
        run(&mut sim, 4 * SECOND);
        assert_eq!(stock(&sim, 0), [Fixed::ZERO; RESOURCES], "alwaysactiveai only: a human gets nothing");
        near(stock(&sim, 1)[3], over(Fixed::from_int(5), 4 * SECOND + 1));
        // The player-only mirror image.
        let player_only = AI_SUPPLY.replace("alwaysactiveai", "alwaysactiveplayer");
        let mut sim = game(db(&[("p_supply", &player_only)]));
        spawn(&mut sim, "fx_p_supply", 0);
        spawn(&mut sim, "fx_p_supply", 1);
        run(&mut sim, 4 * SECOND);
        near(stock(&sim, 0)[3], over(Fixed::from_int(5), 4 * SECOND + 1));
        assert_eq!(stock(&sim, 1), [Fixed::ZERO; RESOURCES]);
    }

    #[test]
    fn one_time_and_button_weapons_are_not_run_as_a_rate() {
        let mut sim = game(db(&[("bor_20priority", ONE_TIME), ("bor_recycle_dilithium", BUTTON)]));
        spawn(&mut sim, "fx_bor_20priority", 0);
        spawn(&mut sim, "fx_bor_recycle_dilithium", 0);
        run(&mut sim, 5 * SECOND);
        assert_eq!(stock(&sim, 0), [Fixed::ZERO; RESOURCES]);
    }

    #[test]
    fn a_drain_never_takes_the_stock_below_zero() {
        let mut sim = game(db(&[("rsteal_2", DRAIN)]));
        spawn(&mut sim, "fx_rsteal_2", 0);
        run(&mut sim, 3 * SECOND);
        assert_eq!(stock(&sim, 0)[3], Fixed::ZERO);
        let c = Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 3, Fixed::from_int(500))) };
        sim.step(&[c]);
        run(&mut sim, SECOND);
        near(stock(&sim, 0)[3], Fixed::from_int(500) - over(Fixed::from_int(200), SECOND + 1));
    }

    #[test]
    fn two_carriers_add_up_and_the_digest_is_reproducible() {
        let make = || {
            let mut sim = game(db(&[("f_generate_supply", F_SUPPLY)]));
            spawn(&mut sim, "fx_f_generate_supply", 0);
            spawn(&mut sim, "fx_f_generate_supply", 0);
            run(&mut sim, 20 * SECOND + 7);
            sim
        };
        let (a, b) = (make(), make());
        assert_eq!(a.digest(), b.digest());
        near(stock(&a, 0)[3], over(odf(0.2), 20 * SECOND + 7) + over(odf(0.2), 20 * SECOND + 7));
    }

    #[test]
    fn a_converter_pays_nothing_it_cannot_afford_and_add_crew_reaches_the_crew_stock() {
        let convert = "adddilithium = 5\naddbiomatter = -0.33\nalwaysactiveplayer = 1\nclasslabel = \"resourceweapon\"\n";
        let mut sim = game(db(&[("convert", convert)]));
        spawn(&mut sim, "fx_convert", 0);
        run(&mut sim, 3 * SECOND);
        assert_eq!(stock(&sim, 0), [Fixed::ZERO; RESOURCES], "no biomatter to pay with: nothing is made");
        let c = Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 3, Fixed::from_int(10))) };
        sim.step(&[c]);
        run(&mut sim, SECOND);
        assert!(stock(&sim, 0)[0] > Fixed::from_int(4));
        assert!(stock(&sim, 0)[3] < Fixed::from_int(10));
        let crew = "addCrew = 2\nalwaysactiveplayer = 1\nclasslabel = \"resourceweapon\"\n";
        let mut sim = game(db(&[("crewgen", crew)]));
        sim.economy.set_personnel(PlayerId(0), Some(Fixed::from_int(100)), None);
        spawn(&mut sim, "fx_crewgen", 0);
        run(&mut sim, 10 * SECOND);
        near(sim.economy.crew_of(PlayerId(0)).unwrap() - Fixed::from_int(100), over(Fixed::from_int(2), 10 * SECOND + 1));
    }

    #[test]
    fn snapshot_round_trip_continues_identically() {
        let db = db(&[("generate_resources", GENERATE)]);
        let mut live = game(Arc::clone(&db));
        spawn(&mut live, "fx_generate_resources", 0);
        run(&mut live, 3 * SECOND + 11);
        let mut copy = Sim::from_snapshot(&live.snapshot(), Some(db)).unwrap();
        assert_eq!(copy.digest(), live.digest());
        for _ in 0..200 {
            live.step(&[]);
            copy.step(&[]);
            assert_eq!(copy.digest(), live.digest());
        }
    }

    #[test]
    fn content_without_resource_weapons_keeps_its_hash_and_economy_is_untouched() {
        let mut sim = game(Arc::new(sta_content::fake::db()));
        run(&mut sim, SECOND);
        assert!(sim.economy.is_empty() || stock(&sim, 0) == [Fixed::ZERO; RESOURCES]);
    }
}
