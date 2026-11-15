//! The engine's per-team strength variables (`strength_team_1..8`, script names
//! `p_strength_team_N`), LUA-38.
//!
//! Rule (read from the executable, static; the 0.1 s step measured): the value is the sum of the
//! maximum hull (`maxHealth` as the object holds it now: the number `GetMaxHealth` returns) of
//! every live Craft (ships and stations) of the object's own team, teams 1..8, team 0 never
//! summed; damage, cost, crew and unit count play no part. It is recomputed every 200 sim steps,
//! 20 s of game time, the first time on the mission's first step, and no more once the mission has
//! ended; each recompute sets the variable to 0 and adds in one call, so a team with a craft is
//! marked dirty on every recompute even when the sum is unchanged. The engine's step is 0.1 s and
//! STA64's is 1/30 s, so the 20 s period is derived from game seconds with the sim's tick rate,
//! not from a step count. Armada II only: an Armada I mission (`Mission.game == 'a1'`) is left
//! alone, its writer is not established.

use sta_core::seconds_to_ticks;
use sta_sim::view::UnitView;

use mlua::{Table, Value};

use super::queries::craft_family;
use crate::api::{Ctx, Shared};

/// Game seconds between recomputes (200 engine steps of 0.1 s).
pub(crate) const PERIOD_SECONDS: f64 = 20.0;

/// Highest team number with a variable.
const TEAMS: usize = 8;

/// Sim ticks between recomputes.
pub(crate) fn period_ticks() -> u64 {
    seconds_to_ticks(PERIOD_SECONDS).max(1)
}

/// What counts as a Craft: as `GetFirstCraft` lists it (alive, not a construction proxy, not a
/// class known not to be Craft; a unit with no definition in the view counts).
fn craft(c: &Ctx, u: &UnitView) -> bool {
    c.alive(u.id).is_some() && u.progress.is_none() && c.def(u).is_none_or(|d| craft_family(&d.class.0) != Some(false))
}

/// The sums of the live craft's maximum hull per team 1..=8 (index 0 is team 1) and whether the
/// team has any craft.
pub(crate) fn sums(c: &Ctx) -> [Option<f64>; TEAMS] {
    let mut out = [None; TEAMS];
    for u in c.view.units.iter().filter(|u| craft(c, u)) {
        let team = c.player_numbering.encode(u.owner).filter(|n| (1..=TEAMS as u8).contains(n));
        // Neutral and any other slot are never summed.
        if let Some(n) = team {
            *out[usize::from(n) - 1].get_or_insert(0.0) += u.hull[1].to_f32() as f64;
        }
    }
    out
}

/// Whether a recompute is due at `tick` given the tick of the last one.
pub(crate) fn due(last: Option<u64>, tick: u64) -> bool {
    last.is_none_or(|l| tick.saturating_sub(l) >= period_ticks())
}

/// Push the strengths into a converted mission's engine variables (`Mission.engineVars`,
/// `Mission.setEngineValue`) when a recompute is due at the view's tick: per team 1..=8 set the
/// variable to 0 and then to the sum, as the engine's one call does, so the variable is marked
/// dirty (its dependent rules re-run) exactly when the engine would mark it: every recompute for a
/// team with a craft of positive hull, and when the value moved. Objects without those fields
/// (anything but a converted mission) are left alone and do not use up the due time.
pub(crate) fn engine_update(object: &Table, ctx: &Shared, last: &mut Option<u64>) -> mlua::Result<()> {
    // Armada I's strength variables have an unread writer: not bound here.
    if object.raw_get::<Option<String>>("game")?.as_deref() == Some("a1") {
        return Ok(());
    }
    let tick = {
        let c = ctx.borrow();
        // No recomputes after the mission has ended (latched, or its end time has passed).
        let secs = c.view.tick as f64 / sta_core::TICK_HZ as f64;
        if c.ended.is_some() || c.mission_end.as_ref().is_some_and(|e| e.blocked.is_none() && secs > e.time) {
            return Ok(());
        }
        c.view.tick
    };
    if !due(*last, tick) {
        return Ok(());
    }
    let (Value::Table(vars), Value::Function(set)) = (object.raw_get::<Value>("engineVars")?, object.raw_get::<Value>("setEngineValue")?) else {
        return Ok(());
    };
    let sums = sums(&ctx.borrow());
    *last = Some(tick);
    for (i, sum) in sums.iter().enumerate() {
        if let Value::Table(var) = vars.raw_get::<Value>(format!("strength_team_{}", i + 1))? {
            set.call::<()>((var.clone(), 0.0))?;
            if let Some(sum) = sum {
                set.call::<()>((var, *sum))?;
            }
        }
    }
    Ok(())
}
