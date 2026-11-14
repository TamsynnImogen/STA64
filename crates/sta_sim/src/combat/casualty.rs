//! Crew casualties as the original computes them (SIM-39).
//!
//! Only for weapons whose resolved features have `crew_loss_on_hit` (stock
//! Armada II rules, every Armada II family profile). Evidence grades: **S** read
//! from the executable (static), **M** measured on stock Armada II, **D** read
//! from data, **N** not measured. The Armada II routines are the same code in
//! every Armada II family executable (S), and `FleetOpsHook.dll` (FO 3.2.7,
//! Roots, FO 4.0, Classic inside Roots) patches nothing on this path (S), so
//! the profiles differ only by the data keys read here: FO and Roots data
//! carry no `shieldProtection` and set `SHIELD_DAMAGE_PROTECTION` to 1, so no
//! damage bleeds through the shields and the shield crew law never fires (D);
//! `hullCrewModifier` is 0 on almost all their ordnance, but a crew-location
//! roll still kills `min(H, 1)` crew, and system hit points, system-death crew
//! loss, `damageVariance` and the life-support drain act as in every profile.
//! Not run: that Classic inside Roots hits like stock (N). Armada I's
//! executable was not analysed (N), its profile is off.
//!
//! One hit, in the order the engine does it (S, with M where noted):
//!
//! 1. `D = damageBase +- damageVariance` (uniform; one draw from the sim's
//!    stream when the ordnance has a variance).
//! 2. `abs = shieldProtection` (unit key, else `SHIELD_DAMAGE_PROTECTION`,
//!    else 1); the team shield upgrade (`abs = 1 - (1 - p) / upgrade`) is not
//!    modelled (no upgrade state), so it reads 1. `S = D x abs` is the shield
//!    part, `H = D - S` bleeds through to the location roll (M: 20 percent).
//! 3. Shield crew law, only while the struck pool holds shields, the ordnance
//!    has `damageThreshold > 0` and `abs < 1`, with `e = S - threshold > 0`:
//!    `loss = shieldCrewModifier x crew x e / 100 + min(e, 1)` on the unit's
//!    **current** crew, or with `saneCrewLossMethod` and crew above 100
//!    `shieldCrewModifier x e + min(e, 1)`; capped at `e` (M, both branches).
//! 4. Shields lose `S`; what they could not hold is added to `H` (M).
//! 5. `H` goes to one rolled location (S): hull, crew or one of five systems.
//!    Hull: damage. Crew: `hullCrewModifier x crew x H / 100 + min(H, 1)` and
//!    no hull damage (M). A system: its hit points fall by `H` and the hull
//!    takes nothing until it dies; on death the overflow goes to the hull and
//!    the crew loses `<system>CrewLoss x crew / 100 + 1` (M).
//!
//! Location roll (S for the order, weights from the unit's `*HitPercent`):
//! engines, life support, weapons, shield generator, sensors, crew, else hull.
//! A dead or switched-off system, and crew on a unit with no crew pool, weighs
//! 0 and its weight is spread over the live ones in proportion (the engine
//! renormalises to 100; the exact spreading rule was not re-checked). A unit
//! with no `*HitPercent` at all has only hull weight and draws nothing. A
//! system with no `*HitPoints` is not a target. The roll is a uniform value in
//! 0 to 99.99 (the engine's resolution is not measured).
//!
//! Known deviation: with `cfgFACING_WEIGHT` above 0 the engine takes 20
//! percent of the location weight from the distance of the hit point to each
//! system's `*TargetHardpoints` (S). The sim has no hardpoint positions of the
//! target and hits carry the struck point only as `at`, so the blend is
//! omitted: the weights are the ODF percentages alone. The keys are read and
//! digested (`CfgRules`).
//!
//! The model is on per weapon (`apply`) and per unit (`step`) by the same
//! feature bit; per-definition overrides that disagree are not supported.
//! Systems killed in combat stay off until the unit is derelict-and-recrewed
//! (a bench `Heal` mends the points, not the switches).
//!
//! Per tick, while life support is switched off or dead: `lifeSupportLoss x
//! crew x dt / 100 + dt` crew (percent of current crew per second plus one
//! per second, S, M); the team upgrade 3 modifier is not modelled (reads 1).
//!
//! Derelict latch (S): after every crew change a unit with max crew above 0
//! and crew under 1 either gains 10 (flagged immortal by script and not
//! `ButItCanBeDerelicted`) or latches: crew 0, every system off, shields 0,
//! and no further crew loss. Crew at 1 or more clears the latch. Timed
//! takeover (destroyed) and the derelict owner/boarding rules are not here.

use sta_content::{ContentDb, UnitDef, WeaponDef};
use sta_core::{EntityId, Fixed, TICK_HZ};

use super::damage::{SubsystemRow, Subsystems};
use crate::switches::SwitchTable;
use crate::view::UnitFlags;
use crate::world::{Entities, Entity};
use crate::Rng;

/// What a hit may change besides the entities.
pub(crate) struct Ctx<'a> {
    pub subsystems: &'a mut Subsystems,
    pub rng: &'a mut Rng,
    pub switches: &'a SwitchTable,
}

/// System indices follow `UnitDef::system_hit_points`.
const ENGINES: usize = 0;
const WEAPONS: usize = 1;
const LIFE_SUPPORT: usize = 3;
/// The engine's roll order: engines, life support, weapons, shield generator, sensors.
const ROLL_ORDER: [usize; 5] = [ENGINES, LIFE_SUPPORT, WEAPONS, 2, 4];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spot {
    System(usize),
    Crew,
    Hull,
}

/// Saturating `a x b` for non-negative quantities.
fn mul(a: Fixed, b: Fixed) -> Fixed {
    Fixed::from_bits(((i128::from(a.to_bits()) * i128::from(b.to_bits())) >> 32).clamp(0, i128::from(i64::MAX)) as i64)
}

/// `rate x base / 100`.
fn percent_of(rate: Fixed, base: Fixed) -> Fixed {
    Fixed::from_bits(mul(rate, base).to_bits() / 100)
}

fn system_dead(def: &UnitDef, row: &SubsystemRow, n: usize) -> bool {
    let hp = def.system_hit_points[n];
    hp > Fixed::ZERO && row.damage[n] >= hp.to_bits()
}

fn system_off(t: &Entity, row: &SubsystemRow, n: usize) -> bool {
    match n {
        ENGINES => t.flags.contains(UnitFlags::ENGINES_OFF),
        WEAPONS => t.flags.contains(UnitFlags::WEAPONS_OFF),
        _ => row.off & (1 << n) != 0,
    }
}

fn set_off(t: &mut Entity, ctx: &mut Ctx, n: usize) {
    match n {
        ENGINES => t.flags = t.flags.with(UnitFlags::ENGINES_OFF, true),
        WEAPONS => t.flags = t.flags.with(UnitFlags::WEAPONS_OFF, true),
        _ => ctx.subsystems.edit(t.id, |r| r.off |= 1 << n),
    }
}

/// `D +- variance`, uniform, never below 0. No draw without a variance.
fn varied(damage: Fixed, variance: Fixed, rng: &mut Rng) -> Fixed {
    if variance <= Fixed::ZERO {
        return damage;
    }
    let r = (rng.next_u64() % 2001) as i64 - 1000;
    let frac = Fixed::from_bits((r << 32) / 1000);
    (damage + variance * frac).max(Fixed::ZERO)
}

/// Where the part of a hit that reached the hull goes.
fn locate(t: &Entity, def: &UnitDef, row: &SubsystemRow, rng: &mut Rng) -> Spot {
    let k = &def.casualty;
    let has_crew = t.crew.is_some_and(|[_, max]| max > Fixed::ZERO) && !row.derelict;
    let mut w = [Fixed::ZERO; 6];
    let (mut total, mut freed) = (Fixed::ZERO, Fixed::ZERO);
    #[allow(clippy::needless_range_loop)] // `n` also indexes the definition and the row
    for n in 0..5 {
        total = total + k.system_hit_percent[n];
        let usable = def.system_hit_points[n] > Fixed::ZERO && !system_dead(def, row, n) && !system_off(t, row, n);
        if usable {
            w[n] = k.system_hit_percent[n];
        } else {
            freed = freed + k.system_hit_percent[n];
        }
    }
    total = total + k.crew_hit_percent;
    if has_crew {
        w[5] = k.crew_hit_percent;
    } else {
        freed = freed + k.crew_hit_percent;
    }
    let hull = k.hull_hit_percent.unwrap_or_else(|| (Fixed::from_int(100) - total).max(Fixed::ZERO));
    let live = total - freed + hull;
    if total - freed <= Fixed::ZERO {
        return Spot::Hull;
    }
    // Always scaled so the live weights and the hull make 100 (the roll is over 0..100).
    if live > Fixed::ZERO && live != Fixed::from_int(100) {
        let factor = Fixed::from_int(100) / live;
        for x in &mut w {
            *x = mul(*x, factor);
        }
    }
    let r = Fixed::from_bits((((rng.next_u64() % 10_000) as i64) << 32) / 100);
    let mut cum = Fixed::ZERO;
    for n in ROLL_ORDER {
        cum = cum + w[n];
        if r < cum {
            return Spot::System(n);
        }
    }
    cum = cum + w[5];
    if r < cum { Spot::Crew } else { Spot::Hull }
}

/// `KillCrew`: ignored on a derelict and on a unit with no crew capacity;
/// then the derelict check.
pub(crate) fn kill_crew(t: &mut Entity, ctx: &mut Ctx, loss: Fixed) {
    let Some([now, max]) = t.crew else { return };
    if max <= Fixed::ZERO || loss <= Fixed::ZERO || ctx.subsystems.get(t.id).is_some_and(|r| r.derelict) {
        return;
    }
    t.crew = Some([now - loss, max]);
    check_derelict(t, ctx);
}

/// `CheckDerelict`: crew under 1 with a crew capacity latches the unit
/// derelict (or, when script-immortal and not derelictable, gives 10 crew).
pub(crate) fn check_derelict(t: &mut Entity, ctx: &mut Ctx) {
    let Some([now, max]) = t.crew else { return };
    if max <= Fixed::ZERO || now >= Fixed::ONE {
        return;
    }
    if t.flags.contains(UnitFlags::IMMORTAL) && !ctx.switches.get(t.id).derelict_ok {
        t.crew = Some([(now.max(Fixed::ZERO) + Fixed::from_int(10)).min(max), max]);
        return;
    }
    t.crew = Some([Fixed::ZERO, max]);
    t.shields.now = [Fixed::ZERO; 4];
    t.flags = t.flags.with(UnitFlags::ENGINES_OFF, true).with(UnitFlags::WEAPONS_OFF, true);
    ctx.subsystems.edit(t.id, |r| {
        r.derelict = true;
        r.off |= 0b1_1100;
    });
}

/// Crew back to 1 or more clears the latch and switches the systems on again,
/// except those that are dead.
fn clear_derelict(t: &mut Entity, def: &UnitDef, ctx: &mut Ctx) {
    let row = ctx.subsystems.get(t.id).unwrap_or_default();
    let dead = |n| system_dead(def, &row, n);
    t.flags = t.flags.with(UnitFlags::ENGINES_OFF, dead(ENGINES)).with(UnitFlags::WEAPONS_OFF, dead(WEAPONS));
    ctx.subsystems.edit(t.id, |r| {
        r.derelict = false;
        r.off = (2..5).filter(|n| dead(*n)).fold(0, |bits, n| bits | 1 << n);
    });
}

/// One hit under the casualty model; returns the shield and hull damage it
/// really did and whether it emptied the pool.
pub(crate) fn hit(t: &mut Entity, def: &UnitDef, w: &WeaponDef, db: &ContentDb, ctx: &mut Ctx, pool: usize, damage: Fixed) -> (Fixed, Fixed, bool) {
    let k = &def.casualty;
    let d = varied(damage, w.damage_variance, ctx.rng);
    let protection = k.shield_protection.or(db.rules().cfg.shield_damage_protection).unwrap_or(Fixed::ONE).max(Fixed::ZERO).min(Fixed::ONE);
    let s_part = mul(d, protection);
    let mut h = d - s_part;
    let held = t.shields.now[pool];
    let lc = w.crew_loss.unwrap_or_default();
    if held > Fixed::ZERO && lc.threshold > Fixed::ZERO && protection < Fixed::ONE {
        let e = s_part - lc.threshold;
        if e > Fixed::ZERO {
            let crew = t.crew.map_or(Fixed::ZERO, |c| c[0]);
            let by_points = def.sane_crew_loss && crew > Fixed::from_int(100);
            let law = if by_points { mul(lc.shield, e) } else { percent_of(mul(lc.shield, crew), e) };
            kill_crew(t, ctx, (law + e.min(Fixed::ONE)).min(e));
        }
    }
    // Re-read: the crew law may have latched the unit derelict and zeroed them.
    let held = t.shields.now[pool];
    let taken = s_part.min(held);
    t.shields.now[pool] = held - taken;
    h = h + (s_part - taken);
    let down = taken > Fixed::ZERO && t.shields.now[pool] == Fixed::ZERO;
    let mut hull = Fixed::ZERO;
    if h > Fixed::ZERO {
        let row = ctx.subsystems.get(t.id).unwrap_or_default();
        let crew = t.crew.map_or(Fixed::ZERO, |c| c[0]);
        match locate(t, def, &row, ctx.rng) {
            Spot::Hull => hull = h,
            Spot::Crew => kill_crew(t, ctx, percent_of(mul(lc.hull, crew), h) + h.min(Fixed::ONE)),
            Spot::System(n) => {
                let hp = def.system_hit_points[n];
                let now = Fixed::from_bits(row.damage[n]) + h;
                if now < hp {
                    ctx.subsystems.edit(t.id, |r| r.damage[n] = now.to_bits());
                } else {
                    ctx.subsystems.edit(t.id, |r| r.damage[n] = hp.to_bits());
                    set_off(t, ctx, n);
                    hull = now - hp;
                    kill_crew(t, ctx, percent_of(k.system_crew_loss[n], crew) + Fixed::ONE);
                }
            }
        }
    }
    (taken, hull, down)
}

/// Once per tick: the life-support drain and the derelict latch's upkeep.
pub(crate) fn step(units: &mut Entities, db: &ContentDb, ctx: &mut Ctx) {
    let dt = Fixed::ONE / Fixed::from_int(TICK_HZ as i32);
    let ids: Vec<(EntityId, SubsystemRow)> = ctx.subsystems.to_rows();
    for (id, row) in ids {
        let Some(t) = units.get_mut(id).filter(|t| t.hull[0] > Fixed::ZERO) else { continue };
        let Some(def) = db.unit(t.def).filter(|d| db.unit_features(d).crew_loss_on_hit) else { continue };
        let Some([now, max]) = t.crew else { continue };
        if row.derelict {
            t.shields.now = [Fixed::ZERO; 4];
            if max > Fixed::ZERO && now >= Fixed::ONE {
                clear_derelict(t, def, ctx);
            }
        } else if system_off(t, &row, LIFE_SUPPORT) || system_dead(def, &row, LIFE_SUPPORT) {
            let loss = percent_of(mul(def.casualty.life_support_loss, dt), now) + dt;
            kill_crew(t, ctx, loss);
        }
    }
}
