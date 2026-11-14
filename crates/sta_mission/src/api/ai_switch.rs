//! The stock AI switches shared by A1 and A2 (LUA-20): `DisableAllAITeams`,
//! `EnableAllAITeams`, `EnableTeam`, `DisableTeam`, `SetIsAITeam` and, A2 only,
//! `ResetStrategyCycle`.
//!
//! Each call appends one `sta_ai::AiRequest` to the script's ordered request list
//! (`MissionRuntime::take_ai_requests`), which the host gives to
//! `sta_ai::MissionAiHub::submit_requests` after the script has thought and before the seats
//! do. The rule per call, the flags and what a disabled seat still does are in that module's
//! docs. Grades: read from the executables, static; nothing measured on the original bench.
//!
//! * A team number is not range-checked by the original (the call does nothing to a team
//!   that is not an AI team). A number that names no seat here is therefore a no-op, not an
//!   error. A1's `team` helper keeps raising on an invalid number as before.
//! * `SetIsAITeam` on a computer seat has no lasting effect (the original forces the flag
//!   back every tick). `SetIsAITeam(<the human's seat>, true)` would hand the player's units
//!   to the AI: not supported, counted as unsupported; it is not a world gap (no stock
//!   mission does it), so it blocks no mission end.
//! * The calls change no object and no query answer: the condition queries
//!   (`IsCompromised`, `CountStations`, ...) read nothing from the AI (read from the
//!   executable, static), which is why none of them is a world effect for `taint.rs`.

use sta_ai::AiRequest;
use sta_core::PlayerId;

use super::Ctx;

#[cfg(test)]
mod tests;

/// The seat a stock team number names, if any (no range check, as in the original).
pub(crate) fn seat(c: &Ctx, number: i64) -> Option<PlayerId> {
    u8::try_from(number).ok().and_then(|n| c.player_numbering.decode(n)).filter(|p| p.is_player())
}

pub(crate) fn enable(c: &mut Ctx, team: Option<PlayerId>, on: bool) {
    if let Some(t) = team {
        c.ai_requests.push(if on { AiRequest::Enable(t) } else { AiRequest::Disable(t) });
    }
}

pub(crate) fn enable_all(c: &mut Ctx, on: bool) {
    c.ai_requests.push(if on { AiRequest::EnableAll } else { AiRequest::DisableAll });
}

pub(crate) fn set_is_ai(c: &mut Ctx, call: &str, team: Option<PlayerId>, ai: bool) {
    let Some(t) = team else { return };
    if ai && t == c.seat {
        c.unsupported(&format!("{call}: handing the human seat to the AI"));
    }
    c.ai_requests.push(AiRequest::SetAi(t, ai));
}

pub(crate) fn replan(c: &mut Ctx) {
    c.ai_requests.push(AiRequest::Replan);
}
