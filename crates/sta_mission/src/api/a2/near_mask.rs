//! The `mask` argument of `GetNearestObjectNearPosition` / `GetNearestObjectNearMe` (LUA-36).
//!
//! Rule, read from the executable, static (Armada II 1.1 stock): a negative mask does not
//! filter. A non-negative mask is a bit index (`1 << (mask & 31)`, the processor's shift
//! count) into the word that `IsVisibleToTeam(object, team)` tests, so the candidate must be
//! visible to team number `mask`. A mask naming no seated team (neutral, unseated) matches nothing: the bit is
//! never set for it (not established for a team with no seat). What fills the word is not read
//! further; it is taken to be what `A2.IsVisibleToTeam` already answers from the view.

use std::collections::BTreeSet;

use sta_core::EntityId;

use crate::api::Ctx;

/// `None`: no filter. `Some(set)`: only these objects may be returned.
pub(super) fn visible(c: &Ctx, mask: i64) -> Option<BTreeSet<EntityId>> {
    if mask < 0 {
        return None;
    }
    let team = u8::try_from(mask & 31).ok().and_then(|n| c.player_numbering.decode(n)).filter(|p| p.is_player());
    Some(team.map(|p| c.visible_ids(p)).unwrap_or_default())
}
