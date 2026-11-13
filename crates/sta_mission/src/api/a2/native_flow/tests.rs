//! Script-taking A2 presentation calls: payloads, ids, argument strictness and
//! step waits. Fixtures are authored here; no game text.

use sta_core::{Fixed, PlayerId};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::{Arg, Presentation};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

fn runtime() -> MissionRuntime {
    let rt = MissionRuntime::from_source_with("flow.lua", "script = {}", 1, MissionConfig::default());
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn events(rt: &mut MissionRuntime) -> Vec<Presentation> {
    rt.take_commands()
        .into_iter()
        .map(|c| {
            assert_eq!(c.player, PlayerId::SCRIPT);
            match c.kind {
                CommandKind::Edit(Edit::Present(p)) => p,
                other => panic!("{other:?}"),
            }
        })
        .collect()
}

fn waits(rt: &MissionRuntime) -> i64 {
    rt.eval::<i64>("return A2.ScriptWaits(script)").unwrap()
}

#[test]
fn fades_map_duration_colour_and_delay() {
    let mut rt = runtime();
    rt.eval::<()>(
        "A2.FadeIn(script, 3, 0, 0, true); A2.FadeOut(script, 1.5, 0, 1, true); A2.FadeOut(script, 5, 9, 0, true)",
    )
    .unwrap();
    assert_eq!(
        events(&mut rt),
        vec![
            Presentation::Fade { out: false, seconds: Fixed::from_f64(3.0), rgb: [0; 3] },
            Presentation::Fade { out: true, seconds: Fixed::from_f64(1.5), rgb: [255; 3] },
            Presentation::Call {
                name: "A2.FadeOut".into(),
                args: vec![
                    Arg::Num(Fixed::from_f64(5.0)),
                    Arg::Num(Fixed::from_f64(9.0)),
                    Arg::Bool(false),
                ],
            },
        ]
    );
    assert_eq!(waits(&rt), 0, "dont-wait takes no reference");
}

#[test]
fn a_waiting_fade_holds_the_step_until_delay_plus_duration() {
    let mut rt = runtime();
    rt.eval::<()>("A2.FadeIn(script, 2, 1, 0, false); other = {}; A2.FadeOut(other, 1, 0, 0, false)").unwrap();
    assert_eq!(waits(&rt), 1);
    assert_eq!(rt.eval::<i64>("return A2.ScriptWaits(other)").unwrap(), 1);
    rt.step(29); // just under 1 s: both still waiting
    assert_eq!(waits(&rt), 1);
    assert_eq!(rt.eval::<i64>("return A2.ScriptWaits(other)").unwrap(), 1);
    rt.step(30); // 1 s: other (1 s) is done, script (3 s) is not
    assert_eq!(waits(&rt), 1);
    assert_eq!(rt.eval::<i64>("return A2.ScriptWaits(other)").unwrap(), 0);
    rt.step(90);
    assert_eq!(waits(&rt), 0);
}

#[test]
fn audio_ids_are_shared_from_one_and_waits_follow_the_client_report() {
    let mut rt = runtime();
    let a: i64 = rt.eval("return A2.AudioMessage(script, 'one', true)").unwrap();
    let m: i64 = rt.eval("return A2.PlayMusic(script, 'two', false)").unwrap();
    let b: i64 = rt.eval("return A2.AudioMessage(script, 'three', false)").unwrap();
    assert_eq!((a, m, b), (1, 2, 3));
    assert_eq!(
        events(&mut rt),
        vec![
            Presentation::Sound { id: 1, name: "one".into(), at: None },
            Presentation::Call {
                name: "A2.PlayMusic".into(),
                args: vec![Arg::Int(2), Arg::Text("two".into())],
            },
            Presentation::Sound { id: 3, name: "three".into(), at: None },
        ]
    );
    assert_eq!(waits(&rt), 2);
    assert!(!rt.eval::<bool>("return A2.IsAudioMessageDone(1)").unwrap());
    rt.eval::<()>("A2.AudioMessageDone(2)").unwrap();
    assert_eq!(waits(&rt), 1);
    rt.eval::<()>("A2.StopAudioMessage(3)").unwrap();
    assert_eq!(waits(&rt), 0);
    assert_eq!(events(&mut rt), vec![Presentation::StopSound { id: 3 }]);
    // Stopping a finished, unknown or failed id is a no-op as in the engine.
    rt.eval::<()>("A2.StopAudioMessage(3); A2.StopAudioMessage(99); A2.StopAudioMessage(-1)").unwrap();
    assert!(events(&mut rt).is_empty());
}

#[test]
fn a_waiting_message_after_mission_end_returns_minus_one_and_plays_nothing() {
    let mut rt = runtime();
    rt.eval::<()>("Mission:finish(true, 0)").unwrap();
    assert_eq!(rt.eval::<i64>("return A2.AudioMessage(script, 'x', false)").unwrap(), -1);
    assert_eq!(rt.eval::<i64>("return A2.PlayMusic(script, 'x', false)").unwrap(), -1);
    assert!(events(&mut rt).is_empty());
    assert_eq!(waits(&rt), 0);
    // Without a wait it still plays.
    assert_eq!(rt.eval::<i64>("return A2.AudioMessage(script, 'x', true)").unwrap(), 1);
}

#[test]
fn movie_cineractive_and_enter_key_waits() {
    let mut rt = runtime();
    rt.eval::<()>("A2.PlayCinematicMovie(script, 'clip', 1, false)").unwrap();
    rt.eval::<()>("A2.CineractiveStart(script, false); A2.CineractiveStart(script, true)").unwrap();
    rt.eval::<()>("A2.WaitForEnterKey(script); A2.WaitForEnterKey(script)").unwrap();
    let got = events(&mut rt);
    assert_eq!(got[0], Presentation::Movie { name: "clip".into(), in_panel: false, skippable: true });
    assert!(matches!(got[1], Presentation::Cineractive { on: true, .. }));
    assert!(matches!(got[2], Presentation::Cineractive { on: true, .. }));
    assert_eq!(got[3], Presentation::WaitForKey { id: 1 });
    assert_eq!(got[4], Presentation::WaitForKey { id: 2 });
    assert!(rt.eval::<bool>("return A2.IsMoviePlaying()").unwrap());
    assert_eq!(waits(&rt), 4);
    rt.eval::<()>("A2.MovieDone()").unwrap();
    assert_eq!(waits(&rt), 3);
    rt.eval::<()>("A2.CineractiveStartDone()").unwrap();
    assert_eq!(waits(&rt), 2);
    rt.eval::<()>("A2.EnterKeyDone(2)").unwrap();
    assert_eq!(waits(&rt), 1);
    rt.eval::<()>("A2.EnterKeyDone(1)").unwrap();
    assert_eq!(waits(&rt), 0);
}

#[test]
fn invalid_calls_queue_nothing_and_take_no_wait() {
    let mut rt = runtime();
    for bad in [
        "A2.FadeIn(script, 3, 0, 0)",
        "A2.FadeIn(nil, 3, 0, 0, false)",
        "A2.FadeIn(1, 3, 0, 0, false)",
        "A2.FadeIn(script, -1, 0, 0, false)",
        "A2.FadeOut(script, 1, -1, 0, false)",
        "A2.FadeOut(script, 1/0, 0, 0, false)",
        "A2.FadeOut(script, 1, 0, 'x', false)",
        "A2.FadeOut(script, 1, 0, 0, 1)",
        "A2.AudioMessage(script, 'x')",
        "A2.AudioMessage(script, 5, false)",
        "A2.AudioMessage(script, 'x', 0)",
        "A2.AudioMessage(script, string.char(0), false)",
        "A2.PlayMusic('x', false)",
        "A2.StopAudioMessage()",
        "A2.StopAudioMessage(1.5)",
        "A2.StopAudioMessage('1')",
        "A2.CineractiveStart(true)",
        "A2.CineractiveStart(script, 'no')",
        "A2.PlayCinematicMovie(script, '', 1, false)",
        "A2.PlayCinematicMovie(script, 'c', 1.5, false)",
        "A2.PlayCinematicMovie(script, 'c', 1)",
        "A2.WaitForEnterKey()",
        "A2.WaitForEnterKey({}, 1)",
        "A2.ScriptWaits(1)",
        "A2.EnterKeyDone('1')",
        "A2.CineractiveStartDone(1)",
    ] {
        assert!(rt.eval::<()>(bad).is_err(), "{bad}");
    }
    assert!(events(&mut rt).is_empty());
    assert_eq!(waits(&rt), 0);
    assert!(rt.eval::<()>("local a = A2.AudioMessage(script, 'ok', true); assert(a == 1)").is_ok(), "failed calls take no id");
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}
