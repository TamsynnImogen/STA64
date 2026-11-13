//! Fixture tests (EDT-06) with ODF-like files written for this repository:
//! the checks `odf_fmt --check` makes, and the real binary end to end.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use sta_odf::{Cst, GroupTable};
use sta_tools::odf_fmt::{check_bytes, Layout};

fn fixtures() -> Vec<(&'static str, Vec<u8>)> {
    let ship = "// fixture ship\r\n#include \"base.odf\"\r\n\r\n[Ship]\r\nweapon1 = \"phaser\"\r\nweaponHardpoints1 = \"hp01\"\r\nmaxHealth=900  // hull\r\nunitName = \"Test Ship\"\r\nclassLabel = \"starship\"\r\nmetalCost = 120\r\nnewKey = 7\r\npossibleCraftNames = \"A\" \"B\"\r\n    \"C\"\r\n\r\n\r\n// footer\r\n";
    vec![
        ("crlf ship", ship.as_bytes().to_vec()),
        ("lf ship", ship.replace("\r\n", "\n").into_bytes()),
        ("mixed endings", b"a = 1\r\nb = 2\nc = 3\r\n".to_vec()),
        ("bom", b"\xef\xbb\xbfclassLabel = \"x\"\nmaxHealth = 5\n".to_vec()),
        ("latin-1", b"unitName = \"Caf\xe9\"\nmaxHealth = 5\nclassLabel = 1\n".to_vec()),
        ("duplicate keys", b"maxHealth = 1\nclassLabel = \"a\"\nmaxHealth = 2\n".to_vec()),
        ("no final newline", b"weapon1 = 1\nclassLabel = 2".to_vec()),
        ("only comments", b"// nothing\n\n// here\n".to_vec()),
        ("empty", Vec::new()),
        ("orphans", b"\"orphan\"\nstray text\n= nokey\nrace = 1\nclassLabel = 2\n".to_vec()),
    ]
}

#[test]
fn every_fixture_passes_both_layouts() {
    for (name, bytes) in fixtures() {
        for layout in [Layout::Plain, Layout::Grouped(GroupTable::fo())] {
            assert_eq!(check_bytes(&bytes, layout), Vec::<String>::new(), "{name}");
        }
    }
}

#[test]
fn grouped_fixture_comes_out_in_group_order_with_its_own_line_endings() {
    let (_, ship) = fixtures().remove(0);
    let out = Cst::parse(&ship).format_grouped(GroupTable::fo());
    let s = String::from_utf8(out).unwrap();
    let at = |k: &str| s.find(k).unwrap();
    assert!(at("unitName") < at("classLabel") || at("classLabel") < at("unitName"));
    assert!(at("classLabel") < at("metalCost"));
    assert!(at("metalCost") < at("maxHealth"));
    assert!(at("maxHealth") < at("weapon1"));
    assert!(at("weapon1") < at("newKey"));
    assert!(at("#include") < at("[Ship]") && at("[Ship]") < at("classLabel"));
    assert!(s.contains("possibleCraftNames = \"A\" \"B\"\r\n    \"C\"") || s.contains("possibleCraftNames = \"A\" \"B\"\r\n\"C\""));
    assert!(s.ends_with("// footer\r\n"));
    assert!(!s.replace("\r\n", "").contains('\n'), "no bare LF appeared");
}

fn scratch(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("odf_fmt-bin-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn the_binary_checks_formats_and_reports() {
    let d = scratch("bin");
    for (i, (_, bytes)) in fixtures().iter().enumerate() {
        fs::write(d.join(format!("f{i}.odf")), bytes).unwrap();
    }
    let bin = env!("CARGO_BIN_EXE_odf_fmt");
    let report = d.join("report.json");
    let st = Command::new(bin).args(["--check", "--grouped", "--report"]).arg(&report).arg(&d).output().unwrap();
    assert_eq!(st.status.code(), Some(0), "{}", String::from_utf8_lossy(&st.stderr));
    let json = fs::read_to_string(&report).unwrap();
    assert!(json.contains("\"files\":10") && json.contains("\"failed\":[]"), "{json}");

    // Dry run exits 1 and writes nothing; --write then --grouped dry run exits 0.
    let before = fs::read(d.join("f0.odf")).unwrap();
    assert_eq!(Command::new(bin).arg("--grouped").arg(&d).status().unwrap().code(), Some(1));
    assert_eq!(fs::read(d.join("f0.odf")).unwrap(), before);
    assert_eq!(Command::new(bin).args(["--grouped", "--write"]).arg(&d).status().unwrap().code(), Some(0));
    assert_eq!(Command::new(bin).arg("--grouped").arg(&d).status().unwrap().code(), Some(0));
    assert_eq!(Command::new(bin).arg("--bogus").status().unwrap().code(), Some(2));
    fs::remove_dir_all(&d).unwrap();
}
