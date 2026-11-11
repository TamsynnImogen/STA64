use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::run;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sta64-odf-references-{}-{now}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn args(root: &Fixture, odf: &str, tail: &[&str]) -> Vec<String> {
    std::iter::once(root.path())
        .chain(std::iter::once(odf.into()))
        .chain(tail.iter().map(|s| (*s).into()))
        .collect()
}

fn lines<'a>(report: &'a str, prefix: &str) -> Vec<&'a str> {
    report.lines().filter(|line| line.starts_with(prefix)).collect()
}

#[test]
fn layered_self_include_uses_effective_values_and_index_winners_without_rewriting_bytes() {
    let fixture = Fixture::new();
    let base = b"weapon1 = \"OldGun\"\nweapon3 = \"BaseGun.ODF\"\nbuilditem4 = \"BaseHull\"\nordName = \"NULL\"\n";
    let parent = b"#include \"ship.odf\"\nweapon3 = \"ParentGun\"\n";
    let child = b"#include \"SHIP.odf\"\nweapon1 = \"NewGun.ODF\"\nbuilditem1 = \"ODF\\ships\\ChildHull.ODF\"\n";
    fixture.write("Data/odf/ships/ship.odf", base);
    fixture.write("Data/odf/weapons/basegun.odf", b"classLabel = \"cannon\"\n");
    fixture.write("Data/odf/ships/basehull.odf", b"classLabel = \"craft\"\n");
    fixture.write("Data/Mods/Parent/info.ini", b"[Mod]\nName = Parent\n");
    fixture.write("Data/Mods/Parent/odf/ships/ship.odf", parent);
    fixture.write(
        "Data/Mods/Parent/odf/weapons/parentgun.odf",
        b"classLabel = \"cannon\"\n",
    );
    fixture.write("Data/Mods/Child/info.ini", b"[Mod]\nName = Child\nParentMod = Parent\n");
    fixture.write("Data/Mods/Child/odf/ships/SHIP.odf", child);
    fixture.write("Data/Mods/Child/odf/weapons/newgun.odf", b"#include \"absent.odf\"\n");
    fixture.write("Data/Mods/Child/odf/ships/childhull.odf", b"classLabel = \"craft\"\n");

    let report = run(&args(&fixture, "ODF/SHIPS/ship.ODF", &["--mod", "Child"])).unwrap();
    assert_eq!(
        lines(&report, "source\t"),
        ["source\todf=ODF/SHIPS/ship.ODF\tlayer=Child\tpath=odf/ships/ship.odf"]
    );
    assert_eq!(
        lines(&report, "link\t"),
        [
            "link\tkey=weapon1\tref=NewGun.ODF\tstatus=indexed\tlayer=Child\tpath=odf/weapons/newgun.odf",
            "link\tkey=weapon3\tref=ParentGun\tstatus=indexed\tlayer=Parent\tpath=odf/weapons/parentgun.odf",
            "link\tkey=builditem1\tref=ODF\\\\ships\\\\ChildHull.ODF\tstatus=indexed\tlayer=Child\tpath=odf/ships/childhull.odf",
            "link\tkey=builditem4\tref=BaseHull\tstatus=indexed\tlayer=Data\tpath=odf/ships/basehull.odf",
            "link\tkey=ordname\tref=NULL\tstatus=missing",
        ]
    );
    assert_eq!(fs::read(fixture.0.join("Data/odf/ships/ship.odf")).unwrap(), base);
    assert_eq!(
        fs::read(fixture.0.join("Data/Mods/Child/odf/ships/SHIP.odf")).unwrap(),
        child
    );
}

#[test]
fn numbered_order_gaps_missing_links_lookalikes_and_escaped_rows_are_stable() {
    let fixture = Fixture::new();
    let source = b"weapon10 = MissingTen\nweapon2 = \"MissingTwo\"\nweapon02 = \"MissingZeroTwo\"\nweapon3 = NULL\nweapon4 = \" \"\nweaponHardPoints8 = Fake\nweapon4294967296 = Overflow\nbuildItem11 = Later\nbuildItem2 = NULL\nbuildItem1 = \"Missing\tTab\"\nbuildItem3availability = Fake\nordName = NULL\n";
    fixture.write("odf/ships/source.odf", source);
    let report = run(&args(&fixture, "source", &["--loose-root"])).unwrap();
    assert_eq!(report, run(&args(&fixture, "source", &["--loose-root"])).unwrap());
    assert_eq!(
        lines(&report, "link\t"),
        [
            "link\tkey=weapon02\tref=MissingZeroTwo\tstatus=missing",
            "link\tkey=weapon2\tref=MissingTwo\tstatus=missing",
            "link\tkey=weapon10\tref=MissingTen\tstatus=missing",
            "link\tkey=builditem1\tref=Missing\\tTab\tstatus=missing",
            "link\tkey=builditem11\tref=Later\tstatus=missing",
            "link\tkey=ordname\tref=NULL\tstatus=missing",
        ]
    );
    assert_eq!(fs::read(fixture.0.join("odf/ships/source.odf")).unwrap(), source);
}

#[test]
fn missing_source_include_cycle_and_same_root_duplicate_diagnostic() {
    let fixture = Fixture::new();
    fixture.write("odf/ships/source.odf", b"#include \"missing.odf\"\nweapon1 = kept\n");
    assert_eq!(run(&args(&fixture, "absent", &["--loose-root"])).unwrap_err().code, 1);
    let error = run(&args(&fixture, "source", &["--loose-root"])).unwrap_err();
    assert_eq!(error.code, 1);
    assert!(error.message.contains("include not found"), "{error:?}");

    fixture.write("odf/other/missing.odf", b"#include \"third.odf\"\n");
    fixture.write("odf/other/third.odf", b"#include \"missing.odf\"\n");
    let error = run(&args(&fixture, "source", &["--loose-root"])).unwrap_err();
    assert_eq!(error.code, 1);
    assert!(error.message.contains("include cycle"), "{error:?}");

    fixture.write("odf/other/missing.odf", b"weapon1 = ordinary\n");
    fixture.write("odf/other/source.odf", b"weapon1 = lower_priority\n");
    let report = run(&args(&fixture, "source", &["--loose-root"])).unwrap();
    assert_eq!(lines(&report, "warning\t").len(), 1);
    assert!(report.contains("found 2 times"), "{report}");
    assert!(report.contains("ref=lower_priority"));
    assert!(!report.contains("ref=kept"));
}

#[test]
fn usage_and_mount_errors_never_fall_back_to_loose_root() {
    let fixture = Fixture::new();
    fixture.write("odf/ships/source.odf", b"weapon1 = Missing\n");
    assert_eq!(run(&["--help".into()]).unwrap(), format!("{}\n", super::USAGE));
    for bad in [
        vec![],
        vec![fixture.path()],
        args(&fixture, "source", &["--mod"]),
        args(&fixture, "source", &["--loose-root", "--mod", "Child"]),
        args(&fixture, "source", &["--unknown"]),
        args(&fixture, "source", &["--mod", ""]),
        vec!["--help".into(), "source".into()],
        vec![fixture.path(), "--unknown".into()],
    ] {
        assert_eq!(run(&bad).unwrap_err().code, 2, "{bad:?}");
    }
    assert_eq!(run(&args(&fixture, "source", &[])).unwrap_err().code, 1);
    assert_eq!(
        run(&args(&fixture, "source", &["--mod", "Missing"])).unwrap_err().code,
        1
    );
    assert!(
        run(&args(&fixture, "source", &["--loose-root"]))
            .unwrap()
            .contains("ref=Missing")
    );
    assert!(!fixture.0.join("Data").exists());
}

#[test]
fn indexed_source_filename_is_not_trimmed_into_another_odf() {
    let fixture = Fixture::new();
    fixture.write("odf/ships/ source.odf", b"weapon1 = intended\n");
    fixture.write("odf/ships/source.odf", b"weapon1 = decoy\n");
    let report = run(&args(&fixture, " source", &["--loose-root"])).unwrap();
    assert!(report.contains("path=odf/ships/ source.odf"), "{report}");
    assert!(report.contains("ref=intended"), "{report}");
    assert!(!report.contains("ref=decoy"), "{report}");
}
