//! LUA-32: reads shared by the A1 and A2 alert, `UnderAttack` and ally-attack
//! bindings. Rules (read from the executable, static; see `sta_sim::alert`):
//! script alert calls set the level directly (no voice, no command reset, no
//! target clear); `UnderAttack(h, s)` is `now - last enemy hit < s` (strict,
//! game seconds, false for a handle that is gone); the ally counters are the
//! sim's `[team][byTeam]` order counts.

use sta_core::{EntityId, TICK_HZ};
use sta_sim::command::{AlertState, CommandKind, Edit};

use super::Ctx;

/// Does the unit hold an alert level: a live craft or station (the sim rejects the
/// edit for anything else, so the bindings neither queue nor answer for it).
pub(crate) fn holds_alert(c: &Ctx, id: EntityId) -> bool {
    use sta_sim::world::EntityKind;
    c.alive(id).and_then(|u| c.def(u)).is_some_and(|d| matches!(EntityKind::of_def(d), EntityKind::Craft | EntityKind::Station))
}

/// The alert of a unit: the latest alert edit queued this tick, else one the
/// host took but the sim has not settled, else the view's row (red when absent).
pub(crate) fn state(c: &Ctx, id: EntityId) -> AlertState {
    let queued = c.commands.iter().rev().find_map(|cmd| match &cmd.kind {
        CommandKind::Edit(Edit::Alert(who, s)) if *who == id => Some(*s),
        _ => None,
    });
    queued
        .or_else(|| c.edit_feedback.pending_alert(id))
        .or_else(|| c.view.attrs.alerts.binary_search_by_key(&id, |r| r.0).ok().map(|i| c.view.attrs.alerts[i].1))
        .unwrap_or(AlertState::Red)
}

/// `UnderAttack(h, seconds)`: false for a handle that is not alive or never hit.
pub(crate) fn under_attack(c: &Ctx, id: EntityId, seconds: f64) -> bool {
    if c.alive(id).is_none() {
        return false;
    }
    let Ok(i) = c.view.attrs.enemy_hit.binary_search_by_key(&id, |r| r.0) else { return false };
    let age = c.view.tick.saturating_sub(c.view.attrs.enemy_hit[i].1);
    (age as f64) / f64::from(TICK_HZ) < seconds
}

/// `GetAllyShipAttacks` / `GetAllyStationAttacks` of the team numbers `team`
/// and `by`; 0 for a number no seat has (the engine does no range check).
pub(crate) fn ally_attacks(c: &Ctx, team: i64, by: i64, station: bool) -> i64 {
    let (Some(t), Some(b)) = (super::ai_switch::seat(c, team), super::ai_switch::seat(c, by)) else { return 0 };
    c.view
        .attrs
        .ally_attacks
        .iter()
        .find(|r| (r.0, r.1) == (t.0, b.0))
        .map_or(0, |r| i64::from(if station { r.3 } else { r.2 }))
}
