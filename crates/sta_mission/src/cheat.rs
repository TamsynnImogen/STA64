//! Mission-ending cheat codes (LUA-29): the host-side rule, with no UI in it.
//!
//! **Rule.** The chat box of a single-player game takes two exact strings that end the mission at
//! once: `kobayashimaru` (a win; Armada I and II) and `kobayashimaru_lost` (a loss; Armada II
//! only). The text is matched whole and case-sensitively (no trim, no prefix, no substring) and,
//! when it matches, is *not* sent as chat. It is accepted only when the session is single player:
//! the campaign or a command-line mission. Instant Action and every network transport refuse it
//! (the text then goes on as ordinary chat), and STA64 also refuses it during replay playback.
//! Armada I has no loss cheat; [`force_end`] can still lose an Armada I mission for tests, but no
//! chat text reaches it. `showmethemoney` and `canofwhoopass` are not mission ends and are left
//! out. Evidence grade: **read from the executable, static; not measured; the strings are
//! compiled into the executables and are in no data file** (so they are STA64 constants, not
//! data keys). STA64 deviates in one place on purpose: replay playback refuses the cheats (the
//! original asks a per-replay flag in Armada II and always allows it in Armada I).
//!
//! **What a forced end is.** The same end record a script's `SucceedMission` / `FailMission`
//! writes ([`crate::runtime::MissionRuntime::force_mission_end`]: `MissionEnd { time: now, failed,
//! blocked: None }`), so the mission ends on the next update and [`crate::campaign::scan_outcome`]
//! and [`crate::campaign::CampaignRunner::finish`] take it from there exactly as for a scripted
//! end. It **overrides a pending scripted end** (and a blocked one) and **skips the script's own
//! win or lose steps**: no victory cineractive, voice line or objective update the script would
//! have run before calling its end. It touches no sim state and no command, so it needs no
//! snapshot, protocol or replay number; it is a host input of a single-player game and must never
//! be applied in a replay or a network session (that is the [`CheatSession`] check).
//!
//! **Left open** (not modelled by STA64, so not honoured here): the Armada II conditions under
//! which the chat box does not open (a script's `DisableChat(true)`, a script step waiting for
//! Enter, the objectives display up). The caller that owns the chat box decides whether it opens;
//! this module only sees the text it sends. STA64 has no single-player in-game chat input yet
//! (the chat log is display only), so nothing calls [`try_chat_cheat`] in the binary today.

use crate::runtime::MissionRuntime;

/// The win cheat, Armada I and II.
pub const CHEAT_WIN: &str = "kobayashimaru";
/// The loss cheat, Armada II only.
pub const CHEAT_LOSE: &str = "kobayashimaru_lost";

/// Which game's rule set applies (the loss cheat exists in Armada II only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheatGame {
    A1,
    A2,
}

/// What the game session is, as far as the cheat is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheatSession {
    /// The campaign or a single mission started without a network transport.
    SinglePlayer,
    /// The Instant Action menu's game.
    InstantAction,
    /// Any network transport (LAN, internet, relay).
    Network,
    /// Replay playback.
    Replay,
}

/// A cheat that ends the mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cheat {
    Win,
    Lose,
}

/// Why a cheat text was not accepted. The caller sends the text on as ordinary chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    InstantAction,
    Network,
    Replay,
    /// The mission has already ended.
    AlreadyEnded,
}

/// What the chat box does with a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatCheat {
    /// Not a cheat: send it as chat.
    NotACheat,
    /// A cheat in a session that refuses it: send it as chat.
    Refused(Refusal),
    /// A cheat that ended the mission: do not send it as chat.
    Forced(Cheat),
}

/// The cheat `text` names in `game`, by exact case-sensitive whole-string match.
pub fn match_cheat(game: CheatGame, text: &str) -> Option<Cheat> {
    match text {
        CHEAT_WIN => Some(Cheat::Win),
        CHEAT_LOSE if game == CheatGame::A2 => Some(Cheat::Lose),
        _ => None,
    }
}

/// Whether `session` accepts the cheats.
pub fn session_accepts(session: CheatSession) -> Result<(), Refusal> {
    match session {
        CheatSession::SinglePlayer => Ok(()),
        CheatSession::InstantAction => Err(Refusal::InstantAction),
        CheatSession::Network => Err(Refusal::Network),
        CheatSession::Replay => Err(Refusal::Replay),
    }
}

/// Force the end of the mission `runtime` runs, at `tick`: `won` or lost. No chat text and no
/// session check: this is the test-only entry (an Armada I loss is reachable only here) and the
/// last step of [`try_chat_cheat`]. False when the mission has already ended.
pub fn force_end(runtime: &MissionRuntime, tick: u64, won: bool) -> bool {
    runtime.force_mission_end(tick, won)
}

/// The in-game chat box's cheat check for one sent line (single-player rule above): an exact
/// cheat text in a session that accepts it forces the end and is reported `Forced` (do not send
/// it as chat); anything else is for the normal chat path.
pub fn try_chat_cheat(runtime: &MissionRuntime, game: CheatGame, session: CheatSession, tick: u64, text: &str) -> ChatCheat {
    let Some(cheat) = match_cheat(game, text) else {
        return ChatCheat::NotACheat;
    };
    if let Err(why) = session_accepts(session) {
        return ChatCheat::Refused(why);
    }
    if !force_end(runtime, tick, cheat == Cheat::Win) {
        return ChatCheat::Refused(Refusal::AlreadyEnded);
    }
    ChatCheat::Forced(cheat)
}

#[cfg(test)]
#[path = "cheat_tests.rs"]
mod tests;
