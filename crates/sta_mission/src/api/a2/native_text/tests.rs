//! Title calls: ids, typed event plus placement call, removal and the
//! raising command-VO calls. Fixtures are authored here; no game text.

use sta_core::PlayerId;
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::{Arg, Presentation};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

fn runtime() -> MissionRuntime {
    let rt = MissionRuntime::from_source_with("text.lua", "", 1, MissionConfig::default());
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

fn call(name: &str, args: Vec<Arg>) -> Presentation {
    Presentation::Call { name: format!("A2.{name}"), args }
}

#[test]
fn titles_and_subtitles_share_a_counter_from_zero() {
    let mut rt = runtime();
    let ids: (i64, i64, i64) = rt
        .eval(
            "return A2.ShowTitle(100, 100, 'KEY_A', true), A2.ShowSubtitle(-1, 1100, 'KEY_B', false), \
             A2.ShowTitle(0, 0, 'KEY_C', false)",
        )
        .unwrap();
    assert_eq!(ids, (0, 1, 2));
    assert_eq!(
        &events(&mut rt)[..4],
        &[
            Presentation::Title { id: 0, text: "KEY_A".into(), subtitle: false, seconds: None },
            call("ShowTitle", vec![Arg::Int(0), Arg::Int(100), Arg::Int(100), Arg::Bool(true)]),
            Presentation::Title { id: 1, text: "KEY_B".into(), subtitle: true, seconds: None },
            call("ShowSubtitle", vec![Arg::Int(1), Arg::Int(-1), Arg::Int(1100), Arg::Bool(false)]),
        ]
    );
}

#[test]
fn removal_queues_once_and_ignores_ids_that_are_gone() {
    let mut rt = runtime();
    rt.eval::<()>("t = A2.ShowTitle(1, 2, 'K', false); s = A2.ShowSubtitle(3, 4, 'L', false)").unwrap();
    rt.take_commands();
    rt.eval::<()>("A2.RemoveTitle(t, true); A2.RemoveSubtitle(s, false)").unwrap();
    assert_eq!(
        events(&mut rt),
        vec![
            Presentation::RemoveTitle { id: 0 },
            call("RemoveTitle", vec![Arg::Int(0), Arg::Bool(true)]),
            Presentation::RemoveTitle { id: 1 },
            call("RemoveSubtitle", vec![Arg::Int(1), Arg::Bool(false)]),
        ]
    );
    rt.eval::<()>("A2.RemoveTitle(t, true); A2.RemoveTitle(77, false); A2.RemoveSubtitle(-1, true)").unwrap();
    assert!(events(&mut rt).is_empty());
}

#[test]
fn invalid_calls_queue_nothing_and_take_no_id() {
    let mut rt = runtime();
    for bad in [
        "A2.ShowTitle(1, 2, 'k')",
        "A2.ShowTitle(1.5, 2, 'k', true)",
        "A2.ShowTitle(1, 2147483648, 'k', true)",
        "A2.ShowTitle(1, 2, 5, true)",
        "A2.ShowTitle(1, 2, 'k', 1)",
        "A2.ShowSubtitle(1, 2, string.char(0), true)",
        "A2.ShowSubtitle(1, 2, 'k', true, 3)",
        "A2.RemoveTitle(1)",
        "A2.RemoveTitle('1', true)",
        "A2.RemoveSubtitle(1, 'x')",
        "A2.RemoveSubtitle(1.5, true)",
    ] {
        assert!(rt.eval::<()>(bad).is_err(), "{bad}");
    }
    assert!(events(&mut rt).is_empty());
    assert_eq!(rt.eval::<i64>("return A2.ShowTitle(1, 2, 'k', true)").unwrap(), 0);
}

#[test]
fn command_vo_calls_raise_and_are_counted() {
    let mut rt = runtime();
    for call in [
        "A2.SetCommandWhatWhoVO(1, 2, 3, true)",
        "A2.SetCommandWhatWhoParamVO(1, 2, 3, 4, true)",
        "A2.SetCommandWhatWhoWhereVO(1, 2, 'x', Vector(1, 2, 3), true)",
    ] {
        assert!(rt.eval::<()>(call).is_err(), "{call}");
    }
    assert!(events(&mut rt).is_empty());
    let counted = rt.unsupported();
    assert_eq!(counted.len(), 3, "{counted:?}");
    assert!(counted.iter().all(|(k, n)| k.starts_with("A2.SetCommandWhatWho") && *n == 1));
}
