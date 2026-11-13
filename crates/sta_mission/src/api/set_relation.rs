//! The mission relation setter shared by A1 `SetRelation`, A2 `SetRelation`
//! and Roots `Player:setRelation` (SIM-63).
//!
//! Rule (read from the executables, static): the numeric argument is the
//! engine's relation code, 0 Enemy, 1 Neutral, 2 Ally, identical in Armada I
//! and II, and `SetRelation(a, b, r)` sets only a's view of b. The call goes
//! to the sim as `Edit::SetRelation`; the mission-local override only keeps
//! reads of the same tick consistent until the edit lands.

use sta_core::PlayerId;
use sta_sim::command::Edit;
use sta_sim::view::Relation;

use super::{Ctx, edit_feedback};

/// The relation for the engine's numeric code; `None` above 2.
pub(crate) fn from_code(n: i64) -> Option<Relation> {
    u8::try_from(n).ok().and_then(sta_sim::relations::from_code)
}

pub(crate) fn queue(ctx: &mut Ctx, label: &'static str, from: PlayerId, to: PlayerId, relation: Relation) -> mlua::Result<()> {
    ctx.relations.insert((from, to), relation);
    edit_feedback::queue(ctx, label, Edit::SetRelation { from, to, relation })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_the_engines() {
        assert_eq!(from_code(0), Some(Relation::Enemy));
        assert_eq!(from_code(1), Some(Relation::Neutral));
        assert_eq!(from_code(2), Some(Relation::Ally));
        assert_eq!(from_code(3), None);
        assert_eq!(from_code(-1), None);
    }
}
