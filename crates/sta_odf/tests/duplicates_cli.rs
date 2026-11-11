use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sta-odf-duplicates-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sta-odf-duplicates"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn files_report_source_order_and_case_without_mutating_bytes() {
    let scratch = Scratch::new();
    let bytes = b"#include \"base.odf\"\nSpeed = 1\nother = 9\nspeed = 0 // explicit zero\nSPEED = \"a // b\"\n";
    let first = scratch.file("first.odf", bytes);
    let second = scratch.file("second.odf", b"x = 1\nX = 2\n");
    let advisory = bin().arg(&first).arg(&second).output().unwrap();
    assert_eq!(advisory.status.code(), Some(0));
    let lines: Vec<_> = stdout(&advisory).lines().map(str::to_owned).collect();
    assert_eq!(
        lines,
        [
            format!("{}:4: speed overrides 2:Speed", first.display()),
            format!("{}:5: SPEED overrides 4:speed", first.display()),
            format!("{}:2: X overrides 1:x", second.display()),
        ]
    );
    assert!(
        !stdout(&advisory).contains("a // b"),
        "values must not be printed"
    );
    let strict = bin()
        .arg("--deny-duplicates")
        .arg(&first)
        .arg(&second)
        .output()
        .unwrap();
    assert_eq!(strict.status.code(), Some(1));
    assert_eq!(stdout(&strict), stdout(&advisory));
    assert_eq!(fs::read(&first).unwrap(), bytes);
    assert_eq!(fs::read(&second).unwrap(), b"x = 1\nX = 2\n");
}

#[test]
fn stdin_is_advisory_by_default_and_strict_only_on_request() {
    for (args, status) in [(vec!["-"], 0), (vec!["--deny-duplicates", "-"], 1)] {
        let mut child = bin()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"name = 1\nNAME = 0\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(status));
        assert_eq!(stdout(&output), "-:2: NAME overrides 1:name\n");
    }
    let output = bin()
        .arg("--deny-duplicates")
        .arg("-")
        .stdin(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

#[test]
fn read_and_utf8_errors_take_precedence_over_earlier_duplicates() {
    let scratch = Scratch::new();
    let duplicate = scratch.file("duplicate.odf", b"a=1\nA=0\n");
    let missing = scratch.0.join("missing.odf");
    let bad_utf8 = scratch.file("bad.odf", b"a = \xff\n");
    for path in [&missing, &bad_utf8] {
        let output = bin()
            .arg("--deny-duplicates")
            .arg(&duplicate)
            .arg(path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn help_usage_and_end_of_options_are_explicit() {
    let scratch = Scratch::new();
    scratch.file("-option-like.odf", b"a=1\nA=0\n");
    let output = bin()
        .current_dir(&scratch.0)
        .arg("--")
        .arg("-option-like.odf")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("-option-like.odf:2: A overrides 1:a"));
    let help = bin().arg("--help").output().unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(stdout(&help).contains("--deny-duplicates"));
    for args in [vec![], vec!["--bogus"], vec!["--deny-duplicates"]] {
        let output = bin().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!output.stderr.is_empty());
    }
}
