//! The Armada I front end (LUA-15e): the port of `tools/a1_to_lua.py`.
//!
//! The Python tool is a subclass of the Armada II lifter (`A1Dll(dsl.Dsl)`, `A1Lifter(dsl.Lifter)`)
//! with its own control-flow graph and stack model, an `esp`-relative frame, calls by import
//! signature, and its own mission driver. Here:
//!
//! - [`dll`]: `A1Dll` (import stubs, entries, the entry point, `DllMain`, `resolve`).
//! - [`lift`]: `A1Lifter.cfg`, `lift`, `linear`, `check_helpers`, `reads_params`, the compare
//!   facts (`note_compare`, `edge_value`) and `entry_state`.
//! - [`ops`]: the stack model and the overrides of `read`, `lvalue`, `write`, `glob`, `load`,
//!   `store`, `snap`, `step`, `arith`. They hang off the lifter core's `Game::A1` flag: the core's
//!   methods in `lift.rs` call the `a1_*` methods here, and the Python's `super()` calls are the
//!   core's `*_base` methods.
//! - [`calls`]: `ret_slots`, `call`, `internal`, `tail_import`, `arg_at`, `engine`, `undecoded`,
//!   `this_var`, `special`, `typed`, `ref_lv`, `lv_plus`, `deref_value`, `arg`.
//! - [`render`]: `render_expr`'s Armada I part, `render_failed`, `a1_prelude` use.
//! - [`mission`]: `imports_in`, `call_name`, `classify`, `norm`, `lift_mission`, `CondNamer`,
//!   `emit_function`, `convert`, `mission_report`, `find_missions`.
//!
//! # What this row takes from the Armada II tool
//!
//! The Armada I tool imports from `dsl_to_lua.py` (see the PR body for the sorted list): the lifter
//! core, which is on `main` (LUA-15d), the shared core (LUA-15c), and the A2 emission half, which is
//! LUA-15h's (`Namer`, `render_function` with `render_expr`/`render_lv`/`render_stmt`/
//! `inline_temps`). The last is reached through the seams of [`mission::Render`] and
//! [`render::intercept`]; nothing of it is ported here.
//!
//! # Python behaviour not matched
//!
//! Listed in the PR body; each place carries a `Python:` note where it is.

pub mod calls;
pub mod dll;
pub mod lift;
pub mod mission;
pub mod ops;
pub mod render;

use std::collections::{BTreeSet, HashSet};

use indexmap::IndexMap;

use crate::pyval::Py;
use crate::tup;
use crate::Result;

pub use dll::A1Dll;
pub use mission::{convert_dlls, convert_install, find_missions, lift_mission, mission_report, MissionLift, Report};

/// `SAVED` of `a1_to_lua.py`: the registers a function keeps for its caller.
pub const SAVED: [&str; 4] = ["ebx", "esi", "edi", "ebp"];

/// `WORLD_IIIE` of `a1_to_lua.py`: `WORLD`'s rule engine is the member at this offset.
pub const WORLD_IIIE: i64 = 0x38;

/// `VAR_CLASSES` of `a1_to_lua.py`: `drl.VAR_KINDS + ("hot_type",)`.
pub const VAR_CLASSES: [&str; 5] = ["hot_int", "hot_float", "dirty_int", "timer_int", "hot_type"];

/// `VAR_METHODS` of `a1_to_lua.py`: variable methods by the decorated method name (operator
/// codes `??4` is `=`, `??B` is the conversion).
pub fn var_method(name: &str) -> Option<&'static str> {
    Some(match name {
        "op4" => "set",
        "opB" => "get",
        "start" => "start",
        "stop" => "stop",
        "update" => "update",
        "mark_dirty" => "mark_dirty",
        "set_clean" => "set_clean",
        "is_dirty" => "is_dirty",
        "time" => "time",
        _ => return None,
    })
}

/// `MEM`: a stack slot whose value is in the function's slot table.
pub fn mem() -> Py {
    tup!["mem"]
}

/// `UNINIT`: reserved by `sub esp` / `push reg`, not yet written.
pub fn uninit() -> Py {
    tup!["stackslot"]
}

/// `v == MEM`.
pub(crate) fn is_mem(v: &Py) -> bool {
    *v == mem()
}

/// `v == UNINIT`.
pub(crate) fn is_uninit(v: &Py) -> bool {
    *v == uninit()
}

/// `v and v[0]` as an f-string prints it: the head of a truthy tuple, else the value itself
/// (`None`, `()`).
pub(crate) fn and0(v: &Py) -> Result<String> {
    if v.truthy() {
        Ok(v.item(0)?.to_str())
    } else {
        Ok(v.to_str())
    }
}

/// `v and v[0] == name` (false for a falsy `v`).
pub(crate) fn is_head(v: &Py, name: &str) -> Result<bool> {
    Ok(v.truthy() && v.item(0)?.as_str() == Some(name))
}

/// What `test r,r` / `cmp r,imm` proved (`st.cfact` of `A1Lifter.note_compare`):
/// `(register, constant, the register's value, the flags value it was proved on)`. The Python
/// tests the flags by identity (`st.flags is f[3]`): here it is the number of assignments the
/// flags had ([`crate::lift::State::flags_gen`]), which is unchanged exactly while the same
/// object is held.
#[derive(Debug, Clone)]
pub struct CFact {
    pub reg: String,
    pub imm: i64,
    pub value: Py,
    pub flags_gen: u64,
}

/// `st.edge` of `A1Lifter.note_compare`: `(register, constant, equal on the taken edge, target)`.
#[derive(Debug, Clone)]
pub struct Edge {
    pub reg: String,
    pub imm: i64,
    pub eq_on_taken: bool,
    pub tgt: i64,
}

/// The fields the Armada I `State` adds to the core's (`State` of `a1_to_lua.py`, with the
/// attributes `note_compare` and `lift` set dynamically: `cfact`, `edge`, `out_r`, `out_s`).
#[derive(Debug, Clone, Default)]
pub struct StateExtra {
    /// Dwords pushed since the function's entry.
    pub depth: i64,
    /// Stack position (1 is just below the return address) to its value; a missing position is
    /// `MEM`.
    pub slots: IndexMap<i64, Py>,
    /// Positions whose value is in the slot table at block entry.
    pub entry_mem: BTreeSet<i64>,
    /// Of those, the ones this block reads.
    pub loc_in: BTreeSet<i64>,
    /// The depth `ebp` was set at (`mov ebp,esp`), if any.
    pub frame: Option<i64>,
    /// Slots holding a one-byte temporary.
    pub byte_slots: BTreeSet<i64>,
    pub cfact: Option<CFact>,
    pub edge: Option<Edge>,
    pub out_r: BTreeSet<String>,
    pub out_s: BTreeSet<i64>,
}

/// The fields the Armada I `Lifted` adds to the core's.
#[derive(Debug, Clone, Default)]
pub struct LiftedExtra {
    /// `(call, why)` written as `UD(...)`.
    pub undecoded: Vec<(String, String)>,
    /// Rule indexes this function created.
    pub rules: Vec<i64>,
    /// Every import the function calls, in address order.
    pub imports_called: Vec<String>,
}

/// One rule the rule DLL's registration code creates: `[name, condition addr, action target,
/// deps]` of `A1Lifter.rules`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub cond: i64,
    pub act: i64,
    pub deps: Vec<String>,
}

/// The fields `A1Lifter` adds to the core's `Lifter`, and the tables of its `A1Dll` the lifter
/// reads (`self.d.stubs`, `self.d.entries`).
#[derive(Debug, Clone, Default)]
pub struct LifterExtra {
    /// `jmp [IAT]` stub address to its import symbol.
    pub stubs: IndexMap<i64, String>,
    /// Thunk targets and exported entries.
    pub entries: HashSet<i64>,
    /// Helper functions already checked to read no parameter.
    pub no_params: HashSet<i64>,
    /// Rule-pointer global to the rule index (registration).
    pub rule_globals: IndexMap<i64, i64>,
    pub rules: Vec<Rule>,
}
