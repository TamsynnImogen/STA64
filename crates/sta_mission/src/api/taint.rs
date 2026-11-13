//! Which answers may rest on a gap (LUA-13).
//!
//! STA64 can run a mission and still not do something the original does: an
//! order with no seam, an edit the sim rejects, a spawn that fails. A mission
//! end that follows from such a deviation is neither a win nor a loss. This
//! module follows the deviation, per object, instead of treating every
//! unsupported call as poison.
//!
//! Rule (STA64's own, not read from the game):
//!
//! * A call to the `A2` table is a *gap call* when it raises or the call
//!   counted something unsupported (a call that is not bound at all reaches
//!   [`Taint::gap`] through `A2.NoteGap`, which the converted scripts' guard
//!   makes). Its handle arguments are *tainted*: argument 1 when it is a
//!   positive integer, later arguments only above 8 (small integers there
//!   are team numbers), and no small integer of a call that takes a team
//!   first (`Count...`, `SetRelation`, resource calls). The owner each had at that time is kept.
//! * A gap call that should have returned a handle (`BuildObject`,
//!   `GetHandle`, `GetNearest...`, ...) records that handles may be missing
//!   because of a gap.
//! * A *read* is a condition query (`IsAlive`, `Exists`, `DoesNotExist`,
//!   `IsCompromised`, the `Count...` searches). It is *tainted* when it names
//!   a tainted handle, or a handle that never existed (0, never in a view)
//!   while a handle-producing call has been a gap, or, for a team-wide count,
//!   when a tainted handle belongs to a team the count covers.
//! * `SucceedMission` / `FailMission` are blocked when a tainted read came
//!   before the call. Reads are not tied to the rule that made the call, so
//!   this over-blocks: a tainted read in an unrelated rule blocks a real end.
//!   It never reports a win or a loss that rests on a tainted read it saw.
//!
//! * A gap call that changes the world with no object to name (`SetRelation`,
//!   `DisableAllAITeams`, `EnableAllAITeams`, `ResetStrategyCycle`,
//!   `SetPerceivedTeam`) taints every later read.
//!
//! Limits: any other influence that never passes through an argument (the
//! other global switches, an AI that is not seated) taints nothing; and the
//! rule over-blocks: an end is held back by any tainted read before it, not
//! only the ones its own rule depended on, and a tainted read never clears.

use std::cell::RefCell;
use std::collections::BTreeMap;

use mlua::{Function, Lua, MultiValue, Table, Value};
use sta_core::{EntityId, PlayerId};

use super::{Ctx, Shared};

#[derive(Default)]
pub(crate) struct Taint {
    /// Tainted handles and the owner each had when it was tainted.
    handles: BTreeMap<EntityId, Option<PlayerId>>,
    /// The first gap call that changes the world with no object to name.
    world_gap: Option<String>,
    /// The first gap call that should have produced a handle.
    handle_gap: Option<String>,
    /// The first tainted read: what was asked and why it is tainted.
    read: RefCell<Option<String>>,
}

/// Calls whose result is an object handle.
fn yields_handle(name: &str) -> bool {
    matches!(name, "BuildObject" | "GetHandle" | "GetFirstCraft" | "GetPlayerHandle")
        || name.starts_with("GetNearest")
        || name.starts_with("Spawn")
}

/// Calls that change who fights whom or whether the AI acts, with no object
/// to name: when one is a gap every later read is tainted.
fn world_effect(name: &str) -> bool {
    matches!(
        name,
        "SetRelation" | "DisableAllAITeams" | "EnableAllAITeams" | "ResetStrategyCycle" | "SetPerceivedTeam"
    )
}

/// Calls whose first argument is a team number, not a handle.
fn team_first(name: &str) -> bool {
    name.starts_with("Count")
        || name.starts_with("AI_")
        || matches!(
            name,
            "SetRelation" | "AddMetal" | "SetMetal" | "AddDilithium" | "SetDilithium" | "AddLatinum" | "SetLatinum"
                | "GetMetal" | "GetDilithium" | "GetLatinum" | "CreateNewSiegeFleet" | "CreateNewMoveFleet"
        )
}

impl Taint {
    /// The reason an end that follows now is blocked, if any.
    pub fn reason(&self) -> Option<String> {
        self.read.borrow().clone()
    }
}

impl Ctx {
    /// A call named `name` did not do what the original does; taint its handles.
    pub fn gap(&mut self, name: &str, args: &[Value]) {
        for (i, v) in args.iter().enumerate() {
            let Value::Integer(n) = v else { continue };
            if *n <= 0 || ((i > 0 || team_first(name)) && *n <= 8) {
                continue;
            }
            let Ok(n) = u32::try_from(*n) else { continue };
            let id = EntityId(n);
            let owner = self.unit(id).map(|u| u.owner);
            self.taint.handles.entry(id).or_insert(owner);
        }
        if world_effect(name) && self.taint.world_gap.is_none() {
            self.taint.world_gap = Some(name.to_owned());
        }
        if yields_handle(name) && self.taint.handle_gap.is_none() {
            self.taint.handle_gap = Some(name.to_owned());
        }
    }

    fn tainted_read(&self, call: &str, what: String) {
        let mut r = self.taint.read.borrow_mut();
        if r.is_none() {
            *r = Some(format!("{call} {what} (tick {}); gap: see unsupported()", self.view.tick));
        }
    }

    /// A condition query named `id`.
    pub fn note_query(&self, id: EntityId, call: &'static str) {
        if let Some(gap) = &self.taint.world_gap {
            self.tainted_read(call, format!("on object {} after {gap} was a gap", id.0));
        } else if self.taint.handles.contains_key(&id) {
            self.tainted_read(call, format!("on object {} that a failed call named", id.0));
        } else if self.unit(id).is_none()
            && !self.seen.contains(&id)
            && let Some(gap) = &self.taint.handle_gap
        {
            self.tainted_read(call, format!("on object {} that never existed after {gap} was a gap", id.0));
        }
    }

    /// A team-wide count over `owner` (`None`: every team).
    pub fn note_team_query(&self, owner: Option<PlayerId>, call: &'static str) {
        if let Some(gap) = &self.taint.world_gap {
            self.tainted_read(call, format!("after {gap} was a gap"));
            return;
        }
        let hit = self.taint.handles.values().any(|o| match (owner, o) {
            (None, _) | (_, None) => true,
            (Some(a), Some(b)) => a == *b,
        });
        if hit && !self.taint.handles.is_empty() {
            self.tainted_read(call, "over a team that has an object a failed call named".to_owned());
        }
    }
}

/// Wrap every `A2` function so a gap call taints its handles, and add the
/// hidden `A2.NoteGap(name, ...)` for calls the table does not bind.
/// Call after all modules are installed and before `dll_objects::finish`, so
/// the wrapper sees the real entity ids.
pub(super) fn wrap(lua: &Lua, ctx: &Shared, a2: &Table) -> mlua::Result<()> {
    let c = ctx.clone();
    a2.set(
        "NoteGap",
        lua.create_function(move |_, args: MultiValue| {
            let mut it = args.into_iter();
            let name = match it.next() {
                Some(Value::String(s)) => s.to_string_lossy().to_string(),
                _ => return Ok(()),
            };
            let rest: Vec<Value> = it.collect();
            c.borrow_mut().gap(&name, &rest);
            Ok(())
        })?,
    )?;
    let entries: Vec<(String, Function)> = a2
        .pairs::<String, Value>()
        .filter_map(Result::ok)
        .filter_map(|(k, v)| match v {
            Value::Function(f) if k != "NoteGap" => Some((k, f)),
            _ => None,
        })
        .collect();
    for (name, inner) in entries {
        let c = ctx.clone();
        let key = name.clone();
        a2.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                let before: u32 = c.borrow().unsupported.values().sum();
                let kept: Vec<Value> = args.iter().cloned().collect();
                let out = inner.call::<MultiValue>(args);
                let after: u32 = c.borrow().unsupported.values().sum();
                if out.is_err() || after > before {
                    c.borrow_mut().gap(&key, &kept);
                }
                out
            })?,
        )?;
    }
    Ok(())
}
