//! The special-forces flag of a craft as the mission bindings use it (LUA-28).
//!
//! **Rule (read from the executable, static, both games; not measured in a running
//! game):** the flag is per craft, default off, cleared by a team change. The script
//! order calls (`Goto`, `Patrol`, `Attack`, `AI_Attack`, `Special_Attack`,
//! `AttackMainEnemyBase`, `Mine`, `Colonize`, `Build`, `Stop`, `Cloak`,
//! `ImmediateCloak`, `Decloak`, `GetRepair`, `Scout`, `SearchAndDestroy`) do nothing
//! on a craft whose flag is off. `CommandAllShips` selects only flagged craft when its
//! fourth argument is true. `RecruitSpecialForces(odf, team, &out)` sets `out = 0`,
//! then takes the first craft in the unit list of that team and class that is not in
//! production and not yet flagged, flags it and returns its handle. The player's own
//! orders, selection and targeting do not read the flag.
//!
//! A2 `Patrol(h, path)` is gated too: the executable's `Patrol` is a path type plus a call of the
//! `Goto(h, path, false, 1)` overload, which tests the flag (read from the executable, static).
//!
//! The state is the sim's `Switches.special_forces` (`Edit::SpecialForces`, tag 37,
//! already in the snapshot and the digest). The mission reads it from the view
//! (`AttrsView::special_forces`) plus the flag edits it queued itself this tick, so a
//! script that flags a craft and orders it in the same tick sees its own write.
//!
//! The gate applies to a live craft that has a content definition; an object with no
//! definition (the sim's bare test units) and a station are never gated. A gated call
//! is counted under [`Ctx::absent_paths`] (key `order refused ... (Name)`), which the
//! readiness and playthrough output list, so a flag the sim lost shows up there. It is
//! not an `unsupported` call: the engine does exactly this, nothing is missing.
//!
//! Deviation: the engine's recruit has no dying check; here a dying craft is skipped,
//! because the sim rejects a flag edit on it. For the same reason a flag call (and a recruit) on a
//! craft the script removed earlier in the same tick is treated as no object (the view still shows
//! it, the sim has dropped it by the time the edit applies).

use mlua::{Function, Lua, MultiValue, Value};
use sta_core::EntityId;
use sta_sim::command::{CommandKind, Edit};
use sta_sim::view::{UnitFlags, UnitView};
use sta_sim::world::EntityKind;

use super::{Ctx, Shared, edit_feedback};

/// The script calls that do nothing on an unflagged craft (their first argument is the
/// craft). Both games; a name a game does not bind is simply never called.
pub(crate) const GATED: &[&str] = &[
    "AI_Attack",
    "Attack",
    "AttackMainEnemyBase",
    "Build",
    "Cloak",
    "Colonize",
    "Decloak",
    "GetRepair",
    "Goto",
    "ImmediateCloak",
    "Mine",
    "Patrol",
    "Scout",
    "SearchAndDestroy",
    "Special_Attack",
    "Stop",
];

/// Whether the craft carries the flag now: the latest flag or team edit queued this tick, else the
/// view. The sim applies the edits in order and a team change clears the flag, so a team change
/// queued after the last flag edit leaves the flag off.
pub(crate) fn flagged(c: &Ctx, id: EntityId) -> bool {
    c.commands
        .iter()
        .rev()
        .find_map(|cmd| match &cmd.kind {
            CommandKind::Edit(Edit::SpecialForces(who, on)) if *who == id => Some(*on),
            CommandKind::Edit(Edit::Team(who, _)) if *who == id => Some(false),
            _ => None,
        })
        .unwrap_or_else(|| c.view.is_special_forces(id))
}

/// Whether the script already removed `id` this tick (`RemoveObject` queues `Edit::Kill`, the
/// view still shows the unit): the sim rejects a flag edit on a unit that is gone by then, so the
/// bindings treat it as no object, as for a dying craft.
pub(crate) fn removed_this_tick(c: &Ctx, id: EntityId) -> bool {
    c.commands.iter().any(|cmd| matches!(&cmd.kind, CommandKind::Edit(Edit::Kill(who)) if *who == id))
}

/// A unit with a content definition of the craft kind.
pub(crate) fn is_craft(c: &Ctx, u: &UnitView) -> bool {
    c.def(u).is_some_and(|d| EntityKind::of_def(d) == EntityKind::Craft)
}

/// Whether a script order on `id` is refused: a live craft without the flag.
pub(crate) fn refuses(c: &Ctx, id: EntityId) -> bool {
    c.alive(id).is_some_and(|u| is_craft(c, u) && !flagged(c, id))
}

/// Queue the flag edit for a craft (the sim rejects a dying or non-craft id).
pub(crate) fn queue_flag(ctx: &Shared, label: &'static str, id: EntityId, on: bool) -> mlua::Result<()> {
    edit_feedback::queue(&mut ctx.borrow_mut(), label, Edit::SpecialForces(id, on))
}

/// `RecruitSpecialForces`: the first live craft of the class `odf` (a name, with or without
/// `.odf`) that `owned` accepts, is finished and has no flag; flagged and returned.
/// `None` for an empty or unknown ODF or when nobody qualifies.
pub(crate) fn recruit(
    ctx: &Shared,
    label: &'static str,
    odf: &str,
    owned: impl Fn(&Ctx, &UnitView) -> bool,
) -> mlua::Result<Option<EntityId>> {
    let found = {
        let c = ctx.borrow();
        let want = super::search::stem(odf);
        if want.is_empty() {
            return Ok(None);
        }
        c.view
            .units
            .iter()
            .filter(|u| !u.flags.contains(UnitFlags::DYING) && u.progress.is_none() && !removed_this_tick(&c, u.id))
            .filter(|u| owned(&c, u))
            .filter(|u| super::search::stem(&c.odf_name(u)).eq_ignore_ascii_case(want))
            .filter(|u| c.def(u).is_none_or(|d| EntityKind::of_def(d) == EntityKind::Craft))
            .find(|u| !flagged(&c, u.id))
            .map(|u| u.id)
    };
    if let Some(id) = found {
        queue_flag(ctx, label, id, true)?;
    }
    Ok(found)
}

/// Wrap `inner` so it does nothing (and is counted) when its first argument is an unflagged
/// craft. `resolve` turns a pending-spawn token into the built object's id (`None` when it
/// is not one).
pub(crate) fn gate(
    lua: &Lua,
    ctx: &Shared,
    name: &'static str,
    resolve: impl Fn(u32) -> Option<EntityId> + 'static,
    inner: Function,
) -> mlua::Result<Function> {
    let ctx = ctx.clone();
    lua.create_function(move |_, args: MultiValue| {
        let raw = match args.iter().next() {
            Some(Value::Integer(n)) => u32::try_from(*n).ok(),
            Some(Value::Number(n)) if n.fract() == 0.0 && *n >= 0.0 && *n <= f64::from(u32::MAX) => Some(*n as u32),
            _ => None,
        };
        if let Some(raw) = raw {
            let id = resolve(raw).unwrap_or(EntityId(raw));
            let refused = refuses(&ctx.borrow(), id);
            if refused {
                ctx.borrow_mut().note_absent_path(format!("order refused, craft is not special forces ({name})"));
                return Ok(MultiValue::new());
            }
        }
        inner.call::<MultiValue>(args)
    })
}

/// Test helper: flag every craft of `sim` (bench edits), so a scene whose scripts order its
/// craft meets an open gate. Returns the view after the step.
#[cfg(test)]
pub(crate) fn flag_all(sim: &mut sta_sim::Sim, db: &sta_content::ContentDb) -> std::sync::Arc<sta_sim::view::WorldView> {
    use sta_sim::WorldSource;
    let view = sim.view();
    let edits: Vec<sta_sim::command::Command> = view
        .units
        .iter()
        .filter(|u| db.unit(u.def).is_some_and(|d| EntityKind::of_def(d) == EntityKind::Craft))
        .zip(1u32..)
        .map(|(u, seq)| sta_sim::command::Command {
            player: sta_core::PlayerId::BENCH,
            seq,
            kind: CommandKind::Edit(Edit::SpecialForces(u.id, true)),
        })
        .collect();
    sim.step(&edits).view
}

/// Test helper: a copy of `view` that lists every craft of `db` as flagged (for scenes that
/// hand the mission one fixed view).
#[cfg(test)]
pub(crate) fn flagged_view(view: &sta_sim::view::WorldView, db: &sta_content::ContentDb) -> sta_sim::view::WorldView {
    let mut view = view.clone();
    view.attrs.special_forces = view
        .units
        .iter()
        .filter(|u| db.unit(u.def).is_some_and(|d| EntityKind::of_def(d) == EntityKind::Craft))
        .map(|u| u.id)
        .collect();
    view
}

#[cfg(test)]
#[path = "special_forces/tests.rs"]
mod tests;
