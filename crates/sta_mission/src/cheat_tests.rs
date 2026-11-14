//! Asset-free tests of the cheat rule (LUA-29): the matcher, the session refusals and the forced end.

use super::*;
use crate::campaign::scan_outcome;
use crate::runtime::MissionEndState;

const A1: CheatGame = CheatGame::A1;
const A2: CheatGame = CheatGame::A2;

fn rt() -> MissionRuntime {
    MissionRuntime::from_source("cheat", "-- idle\n", 1)
}

#[test]
fn match_is_exact_and_case_sensitive() {
    assert_eq!(match_cheat(A2, "kobayashimaru"), Some(Cheat::Win));
    assert_eq!(match_cheat(A1, "kobayashimaru"), Some(Cheat::Win));
    assert_eq!(match_cheat(A2, "kobayashimaru_lost"), Some(Cheat::Lose));
    for bad in [
        "Kobayashimaru", "KOBAYASHIMARU", "kobayashiMaru", "KOBAYASHIMARU_LOST", "Kobayashimaru_lost",
        " kobayashimaru", "kobayashimaru ", "kobayashimaru\n", "\tkobayashimaru", "kobayashimaru\0",
        "kobayashimar", "kobayashimaruu", "kobayashimaru_", "kobayashimaru_los", "kobayashimaru_lost ",
        "say kobayashimaru", "kobayashimaru please", "xkobayashimaru", "kobayashimaru kobayashimaru",
        "", " ", "showmethemoney", "canofwhoopass", "kobayashi maru", "kobayashimaru_won",
    ] {
        assert_eq!(match_cheat(A2, bad), None, "{bad:?}");
        assert_eq!(match_cheat(A1, bad), None, "{bad:?}");
    }
}

#[test]
fn armada_one_has_no_loss_cheat() {
    assert_eq!(match_cheat(A1, "kobayashimaru_lost"), None);
    let r = rt();
    assert_eq!(try_chat_cheat(&r, A1, CheatSession::SinglePlayer, 10, "kobayashimaru_lost"), ChatCheat::NotACheat);
    assert_eq!(r.mission_end(1000), MissionEndState::None, "nothing was forced");
}

#[test]
fn only_single_player_accepts() {
    for (session, why) in [
        (CheatSession::InstantAction, Refusal::InstantAction),
        (CheatSession::Network, Refusal::Network),
        (CheatSession::Replay, Refusal::Replay),
    ] {
        for game in [A1, A2] {
            let r = rt();
            assert_eq!(try_chat_cheat(&r, game, session, 10, "kobayashimaru"), ChatCheat::Refused(why));
            assert_eq!(r.mission_end(10_000), MissionEndState::None, "{session:?}: nothing forced");
        }
        let r = rt();
        assert_eq!(try_chat_cheat(&r, A2, session, 10, "kobayashimaru_lost"), ChatCheat::Refused(why));
        assert_eq!(r.mission_end(10_000), MissionEndState::None);
    }
    // Ordinary chat is never touched, whatever the session.
    let r = rt();
    assert_eq!(try_chat_cheat(&r, A2, CheatSession::Network, 10, "hello"), ChatCheat::NotACheat);
}

#[test]
fn forced_win_and_loss_resolve_on_the_next_update() {
    for (text, won) in [("kobayashimaru", true), ("kobayashimaru_lost", false)] {
        let r = rt();
        let cheat = if won { Cheat::Win } else { Cheat::Lose };
        assert_eq!(try_chat_cheat(&r, A2, CheatSession::SinglePlayer, 90, text), ChatCheat::Forced(cheat));
        assert_eq!(r.mission_end(90), MissionEndState::Pending, "ends once mission time is past the end time");
        assert_eq!(scan_outcome(&r, 90), None);
        let o = scan_outcome(&r, 91).expect("ended");
        assert_eq!((o.won, o.tick), (won, 91));
        // It stays ended, and a second cheat does nothing.
        assert_eq!(try_chat_cheat(&r, A2, CheatSession::SinglePlayer, 95, "kobayashimaru"), ChatCheat::Refused(Refusal::AlreadyEnded));
        assert_eq!(scan_outcome(&r, 500).map(|o| o.won), Some(won));
    }
}

#[test]
fn forced_end_overrides_a_pending_scripted_end() {
    // The script asks for a loss 100 seconds out; the cheat wins now.
    let r = MissionRuntime::from_source("cheat", "A2.FailMission(100.0, nil)\n", 1);
    assert_eq!(r.mission_end(60), MissionEndState::Pending);
    assert!(force_end(&r, 60, true));
    assert_eq!(scan_outcome(&r, 61).map(|o| o.won), Some(true));
}

#[test]
fn a_mission_that_ended_by_itself_cannot_be_forced() {
    let r = MissionRuntime::from_source("cheat", "A2.SucceedMission(0.0, nil)\n", 1);
    assert_eq!(r.mission_end(5), MissionEndState::Ended { won: true });
    assert!(!force_end(&r, 5, false));
    assert_eq!(r.mission_end(6), MissionEndState::Ended { won: true });
}
