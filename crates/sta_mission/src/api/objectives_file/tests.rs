use super::{parse, ParsedObjectives};

#[test]
fn parses_heading_briefing_goals_and_trailing_notes() {
    let parsed = parse(b"\xef\xbb\xbfMission\r\n\r\nBriefing one.\r\nBriefing two.\r\n* First goal\r\n* Second goal\r\n\r\nHint text.\r\n").unwrap();

    assert_eq!(
        parsed,
        ParsedObjectives {
            title: "Mission".into(),
            briefing: "Briefing one.\nBriefing two.".into(),
            notes: "Hint text.".into(),
            goals: vec!["First goal".into(), "Second goal".into()],
        }
    );
}

#[test]
fn accepts_heading_without_goals_and_normalizes_cr_only() {
    let parsed = parse(b"Mission\rNo Briefing Available.").unwrap();

    assert_eq!(parsed.title, "Mission");
    assert_eq!(parsed.briefing, "No Briefing Available.");
    assert!(parsed.notes.is_empty());
    assert!(parsed.goals.is_empty());
}

#[test]
fn rejects_empty_input_invalid_utf8_and_empty_goal() {
    assert_eq!(parse(b"").unwrap_err(), "objectives file is empty");
    assert_eq!(
        parse(b"\xff").unwrap_err(),
        "objectives file is not valid UTF-8"
    );
    assert!(parse(b"Mission\n*   ").unwrap_err().contains("empty goal"));
    assert!(parse(b"* No heading").unwrap_err().contains("heading"));
}
