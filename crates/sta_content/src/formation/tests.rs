//! Authored text only: every file here is made up (ADR 0003).

use super::*;
use crate::digest::ContentDigest;

const LINE: &str = "// made up\n#include \"made_up.h\"\nchar formationName[16] = \"Line\";\nint m_formationID = 4;\nfloat m_shipRadius = .5;\n\
    Row formationSpec[64] =\n{\n\
    2,  0.0, 0.0,  0.0, 'S', 'F', '?', 'X', 'X', 'X', 'X',\n\
    0,  1.5, 0.0, -1.0, 'F', 'S', '?', 'X', 'X', 'X', 'X',\n\
    1, -1.5, 0.0, -1.0, 'F', 'S', '?', 'X', 'X', 'X', 'X',\n\
    };\n";

fn line() -> Formation {
    let (f, w) = Formation::parse("ai/formations/line.frm", LINE);
    assert!(w.is_empty(), "{w:?}");
    f.expect("parses")
}

fn fx(n: f64) -> Fixed {
    Fixed::from_f64(n)
}

#[test]
fn rows_keep_file_order_offsets_and_the_priority_list() {
    let f = line();
    assert_eq!((f.name.as_str(), f.id, f.radius), ("Line", Some(4), fx(0.5)));
    assert_eq!(f.origin, Origin::File("ai/formations/line.frm".into()));
    let ranks: Vec<i32> = f.slots.iter().map(|s| s.rank).collect();
    assert_eq!(ranks, [2, 0, 1], "file order, not rank order");
    assert_eq!(f.slots[1].offset, Vec3 { x: fx(1.5), y: fx(0.0), z: fx(-1.0) });
    assert_eq!(f.slots[0].ship_types, ['S', 'F', '?', 'X', 'X', 'X', 'X']);
    assert_eq!(f.slots[1].ship_types, ['F', 'S', '?', 'X', 'X', 'X', 'X']);
}

#[test]
fn the_leader_is_the_lowest_rank_among_the_slots_taken_and_the_first_on_a_tie() {
    let f = line(); // ranks 2, 0, 1 in file order
    assert_eq!(f.leader_slot(3), Some(1));
    assert_eq!(f.leader_slot(99), Some(1), "more craft than slots");
    assert_eq!(f.leader_slot(2), Some(1));
    assert_eq!(f.leader_slot(1), Some(0), "the rank-0 row is not taken: never an empty slot");
    assert_eq!(f.leader_slot(0), None);
    let mut f = line();
    f.slots[2].rank = 0;
    assert_eq!(f.leader_slot(3), Some(1), "first in file order on a tie");
    f.slots.clear();
    assert_eq!(f.leader_slot(3), None);
}

#[test]
fn values_beyond_the_bound_are_refused_and_the_bound_cannot_overflow() {
    let row = |x: &str| format!("0, {x}, 0, 0, 'A','B','C','D','E','F','G',\n");
    let file = |x: &str, radius: &str| format!("char formationName[4] = \"B\";\nint m_formationID = 1;\nfloat m_shipRadius = {radius};\nRow formationSpec[2] = {{\n{}{}}};\n", row("0"), row(x));
    let (f, w) = Formation::parse("b.frm", &file("1000", "1000"));
    assert!(w.is_empty(), "{w:?}");
    let f = f.unwrap();
    let max_spacing = Fixed::from_int(10_000);
    assert_eq!(f.offset(1, max_spacing).unwrap().x, Fixed::from_int(10_000_000));
    assert_eq!(f.ship_radius(max_spacing), Fixed::from_int(10_000_000));
    let (f, w) = Formation::parse("b.frm", &file("1000.5", "1"));
    assert_eq!((f.unwrap().slots.len(), w.len()), (1, 1), "the row is skipped");
    let (f, w) = Formation::parse("b.frm", &file("0", "1001"));
    assert!(f.is_none() && !w.is_empty());
}

#[test]
fn world_units_are_multiples_of_the_spacing() {
    let f = line();
    assert_eq!(f.offset(1, Fixed::from_int(100)), Some(Vec3 { x: fx(150.0), y: fx(0.0), z: fx(-100.0) }));
    assert_eq!(f.offset(9, Fixed::from_int(100)), None);
    assert_eq!(f.ship_radius(Fixed::from_int(100)), fx(50.0));
}

#[test]
fn numbers_read_as_the_header_floats_do() {
    let text = "char formationName[4] = \"N\";\nint m_formationID = 0;\nfloat m_shipRadius = 0.4f;\n\
        Spec formationSpec[3] = {\n  0, -.5, +1e0, 2.25f, 'A', 'B', 'C', 'D', 'E', 'F', 'G',\n};\n";
    let (f, w) = Formation::parse("n.frm", text);
    assert!(w.is_empty(), "{w:?}");
    let f = f.unwrap();
    assert_eq!(f.radius, fx(f64::from(0.4f32)));
    assert_eq!(f.slots[0].offset, Vec3 { x: fx(-0.5), y: fx(1.0), z: fx(2.25) });
    assert_eq!(f.slots.len(), 1, "the list's declared size is not a limit on rows");
}

#[test]
fn comments_blank_lines_and_several_rows_on_one_line_are_fine() {
    let text = "/* block\n comment */ char formationName[4] = \"C\"; // tail\nint m_formationID = 1;\nfloat m_shipRadius = 1;\n\
        Spec formationSpec[9] = {\n\n  0, 0,0,0, 'A','B','C','D','E','F','G',  1, 1,0,0, 'A','B','C','D','E','F','G',\n // gap\n  2, 2,0,0, ',','B','C','D','E','F','G'\n};";
    let (f, w) = Formation::parse("c.frm", text);
    assert!(w.is_empty(), "{w:?}");
    let f = f.unwrap();
    assert_eq!(f.slots.iter().map(|s| s.rank).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(f.slots[2].ship_types[0], ',');
}

#[test]
fn a_malformed_row_warns_with_file_and_line_and_is_skipped() {
    let text = "char formationName[4] = \"M\";\nint m_formationID = 1;\nfloat m_shipRadius = 1;\nSpec formationSpec[9] =\n{\n\
        0, 0.0, 0.0, 0.0, 'A','B','C','D','E','F','G',\n\
        1, 1.0, 0.0, 'A','B','C','D','E','F','G',\n\
        2, 2.0, x, 0.0, 'A','B','C','D','E','F','G',\n\
        3, 3.0, 0.0, 0.0, 'A','B','C','D','E','F', G,\n\
        4.5, 4.0, 0.0, 0.0, 'A','B','C','D','E','F','G',\n\
        5, 5.0, 0.0, 0.0, 'A','B','C','D','E','F','G',\n};\n";
    let (f, w) = Formation::parse("ai/formations/m.frm", text);
    let ranks: Vec<i32> = f.unwrap().slots.iter().map(|s| s.rank).collect();
    assert_eq!(ranks, [0, 5], "only the well-formed rows stay");
    let lines: Vec<(&str, &str)> = w.iter().map(|w| (w.file.as_str(), w.message.split(':').next().unwrap())).collect();
    assert_eq!(lines, [("ai/formations/m.frm", "line 7"), ("ai/formations/m.frm", "line 8"), ("ai/formations/m.frm", "line 9"), ("ai/formations/m.frm", "line 10")]);
    assert!(w[0].message.contains("10 values, expected 11"), "{}", w[0]);
    assert!(w[1].message.contains("offset"), "{}", w[1]);
    assert!(w[2].message.contains("priority"), "{}", w[2]);
    assert!(w[3].message.contains("rank"), "{}", w[3]);
}

#[test]
fn a_file_without_a_name_an_id_a_radius_or_a_usable_row_is_skipped() {
    for (what, text) in [
        ("formationName", "int m_formationID = 1;\nfloat m_shipRadius = 1;\nSpec formationSpec[1] = { 0,0,0,0,'A','B','C','D','E','F','G' };"),
        ("m_formationID", "char formationName[4] = \"A\";\nfloat m_shipRadius = 1;\nSpec formationSpec[1] = { 0,0,0,0,'A','B','C','D','E','F','G' };"),
        ("m_shipRadius", "char formationName[4] = \"A\";\nint m_formationID = 1;\nfloat m_shipRadius = 0;\nSpec formationSpec[1] = { 0,0,0,0,'A','B','C','D','E','F','G' };"),
        ("formationSpec row", "char formationName[4] = \"A\";\nint m_formationID = 1;\nfloat m_shipRadius = 1;\nSpec formationSpec[1] = { 0,0,0 };"),
        ("formationSpec row", "char formationName[4] = \"A\";\nint m_formationID = 1;\nfloat m_shipRadius = 1;\n"),
    ] {
        let (f, w) = Formation::parse("x.frm", text);
        assert!(f.is_none(), "{what}");
        assert!(w.iter().any(|w| w.message.contains(&format!("no usable {what}"))), "{what}: {w:?}");
    }
}

#[test]
fn more_than_the_limit_of_slots_is_cut_with_one_warning() {
    let mut text = String::from("char formationName[4] = \"B\";\nint m_formationID = 1;\nfloat m_shipRadius = 1;\nSpec formationSpec[9] = {\n");
    for i in 0..MAX_SLOTS + 3 {
        text.push_str(&format!("{i}, 0,0,0, 'A','B','C','D','E','F','G',\n"));
    }
    text.push_str("};\n");
    let (f, w) = Formation::parse("big.frm", &text);
    assert_eq!(f.unwrap().slots.len(), MAX_SLOTS);
    assert_eq!(w.len(), 1, "{w:?}");
}

#[test]
fn repeated_ids_keep_the_first_file_and_the_list_is_sorted_by_id() {
    let file = |id: i32, name: &str| {
        format!("char formationName[8] = \"{name}\";\nint m_formationID = {id};\nfloat m_shipRadius = 1;\nSpec formationSpec[1] = {{ 0,0,0,0,'A','B','C','D','E','F','G' }};\n")
    };
    let (a, b, c) = (file(2, "Two"), file(0, "Zero"), file(2, "Again"));
    let (set, w) = Formations::from_texts([("ai/formations/b.frm", a.as_str()), ("ai/formations/a.frm", b.as_str()), ("ai/formations/c.frm", c.as_str())]);
    assert_eq!(set.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Zero", "Two"]);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].file, "ai/formations/c.frm");
    assert!(w[0].message.contains("already used by ai/formations/b.frm"), "{}", w[0]);
    assert_eq!((set.by_id(2).map(|f| f.name.as_str()), set.by_name("ZERO").and_then(|f| f.id), set.len(), set.is_empty()), (Some("Two"), Some(0), 2, false));
}

#[test]
fn the_digest_sees_slots_and_radius_but_not_the_file_name_or_the_display_name() {
    let digest = |text: &str, file: &str| {
        let (set, w) = Formations::from_texts([(file, text)]);
        assert!(w.is_empty(), "{w:?}");
        ContentDigest::from_entries(set.digest_entries())
    };
    let base = digest(LINE, "ai/formations/line.frm");
    assert_eq!(base.entries().len(), 1);
    assert_eq!(base, digest(LINE, "ai/formations/renamed.frm"));
    assert_eq!(base, digest(&LINE.replace("\"Line\"", "\"Renamed\""), "x.frm"), "display text, ADR 0004 D10");
    assert_ne!(base, digest(&LINE.replace("1.5, 0.0, -1.0", "1.5, 0.0, -1.1"), "x.frm"), "an offset");
    assert_ne!(base, digest(&LINE.replace("'S', 'F', '?'", "'F', 'S', '?'"), "x.frm"), "a priority list");
    assert_ne!(base, digest(&LINE.replace("= .5", "= .6"), "x.frm"), "the radius");
    assert_ne!(base, digest(&LINE.replace("2,  0.0", "3,  0.0"), "x.frm"), "a rank");
    assert!(Formations::default().digest_entries().is_empty());
}

#[test]
fn the_built_in_x_and_cross_go_outward_in_rings_of_four() {
    let spacing = Fixed::ONE;
    let x = Formation::built_in(BuiltIn::X, 6);
    let d = fx(f64::from(std::f32::consts::FRAC_1_SQRT_2));
    let pos = |f: &Formation| f.slots.iter().map(|s| (s.offset.x, s.offset.y, s.offset.z)).collect::<Vec<_>>();
    assert_eq!(pos(&x)[..3], [(fx(0.0), fx(0.0), fx(0.0)), (d, -d, fx(0.0)), (-d, d, fx(0.0))]);
    assert_eq!(pos(&x)[4], (d, d, fx(0.0)), "the fifth slot starts the first ring");
    assert_eq!(x.offset(5, spacing).unwrap().x, d * Fixed::from_int(2));
    let c = Formation::built_in(BuiltIn::Cross, 6);
    assert_eq!(pos(&c)[1..5], [(fx(-1.0), fx(0.0), fx(0.0)), (fx(0.0), fx(1.0), fx(0.0)), (fx(0.0), fx(-1.0), fx(0.0)), (fx(1.0), fx(0.0), fx(0.0))]);
    assert_eq!(pos(&c)[5], (fx(-2.0), fx(0.0), fx(0.0)));
    assert_eq!(c.slots[3].ship_types, ['B', '?', '-', '-', '-', '-', '-']);
    assert_eq!((c.slots[3].rank, c.id, c.radius, &c.origin), (3, None, fx(0.45), &Origin::BuiltIn(BuiltIn::Cross)));
    assert_eq!(Formation::built_in(BuiltIn::X, 0).slots.len(), 1, "at least the centre");
    let one = Formation::built_in(BuiltIn::Cross, 1);
    assert_eq!((one.slots.len(), one.leader_slot(1)), (1, Some(0)));
    let big = Formation::built_in(BuiltIn::Cross, 26);
    assert_eq!(big.slots.len(), 26);
    assert_eq!(big.slots[25].offset.x.to_f32().abs() + big.slots[25].offset.y.to_f32().abs(), 7.0, "ring 7");
    assert_eq!(big.leader_slot(26), Some(0));
    assert_eq!(Formation::built_in(BuiltIn::X, 26).slots[25].offset.y, -(d * Fixed::from_int(7)), "ring 7 of the X");
}

#[test]
fn the_fake_formations_are_authored_clean_and_keep_the_plain_fake_hash_apart() {
    let set = crate::fake::formations();
    assert_eq!(set.iter().map(|f| (f.id, f.slots.len())).collect::<Vec<_>>(), [(Some(0), 5), (Some(1), 4)]);
    let plain = crate::fake::db();
    assert!(plain.formations().is_empty(), "the plain fake ships none: SIM-02's kinds are its fallback");
    assert_ne!(plain.hash(), crate::fake::db_with_formations().hash());
    assert_eq!(crate::fake::db_with_formations().formations(), &set);
}

#[test]
fn the_built_in_sheet_is_a_square_spiral_in_the_horizontal_plane() {
    let s = Formation::built_in(BuiltIn::Sheet, 10);
    let xz: Vec<(i32, i32)> = s.slots.iter().map(|s| (s.offset.x.to_f32().round() as i32, s.offset.z.to_f32().round() as i32)).collect();
    assert_eq!(xz, [(0, 0), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1), (2, -1)]);
    assert!(s.slots.iter().all(|s| s.offset.y == Fixed::ZERO));
    assert_eq!(s.slots[1].ship_types, ['B', 'C', 'D', 'F', 'S', 'X', '?']);
}
