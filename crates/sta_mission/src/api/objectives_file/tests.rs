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
fn rejects_empty_input_and_empty_goal() {
    assert_eq!(parse(b"").unwrap_err(), "objectives file is empty");
    assert!(parse(b"Mission\n*   ").unwrap_err().contains("empty goal"));
    assert!(parse(b"* No heading").unwrap_err().contains("heading"));
}

/// A file in the code page of a localised install (authored text, Windows-1252 bytes) is read with
/// its accented letters, as the game's other text files are; the same text in UTF-8 gives the same
/// result, with or without a BOM.
#[test]
fn a_file_that_is_not_utf8_is_read_as_windows_1252() {
    // an authored heading, briefing and goal with accented letters, and a curly apostrophe
    // (0x92) in the notes
    let legacy: &[u8] = b"Pr\xfcfung\r\nGr\xf6\xdfe \xe4ndern\r\n* \xd6l f\xf6rdern\r\nit\x92s done\r\n";
    let parsed = parse(legacy).unwrap();
    assert_eq!(parsed.title, "Pr\u{fc}fung");
    assert_eq!(parsed.briefing, "Gr\u{f6}\u{df}e \u{e4}ndern");
    assert_eq!(parsed.goals, ["\u{d6}l f\u{f6}rdern"]);
    assert_eq!(parsed.notes, "it\u{2019}s done");

    let utf8 = "Pr\u{fc}fung\r\nGr\u{f6}\u{df}e \u{e4}ndern\r\n* \u{d6}l f\u{f6}rdern\r\nit\u{2019}s done\r\n";
    assert_eq!(parse(utf8.as_bytes()).unwrap(), parsed);
    let with_bom = [b"\xEF\xBB\xBF".as_slice(), utf8.as_bytes()].concat();
    assert_eq!(parse(&with_bom).unwrap(), parsed);
}
