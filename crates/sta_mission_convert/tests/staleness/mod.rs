//! Shared by the staleness tests (`core.rs`, `drl.rs`, `dsl_lift.rs`): the reference is the Python
//! tools, and the committed expected files carry CPython exception texts, which differ between
//! CPython versions. So the reference is pinned to one CPython major.minor ([`PINNED`]; the
//! converter reproduces the reference tools' output byte for byte, exception texts included, so
//! the pinned version is part of the reference), and a staleness test compares only under it:
//!
//! - under the pinned version it compares strictly;
//! - under any other version, or without `python3`, it SKIPS VISIBLY: a line starting with
//!   `SKIPPED:` on stderr that names the version found and the version required (cargo shows
//!   stderr of a passing test with `-- --nocapture`);
//! - with `STA64_REQUIRE_PINNED_PYTHON=1` in the environment a skip is a failure, for a CI job
//!   that installs the pinned CPython and must not pass without comparing.
//!
//! Each expected file names the CPython it was generated with in its first line (its generator
//! writes the line from the interpreter that runs it). On every platform, with or without
//! Python, a staleness test first checks that this line names [`PINNED`]: files regenerated
//! under another version are refused with a message that says so, before any text is compared.
//!
//! The Rust-against-expected comparisons of the same files run on every platform regardless.
//! When the expected files are regenerated with another CPython, change [`PINNED`] with them.

use std::process::Command;

/// The CPython `major.minor` the committed expected files were generated with.
pub const PINNED: &str = "3.14";

/// Set to `1` to turn a skip into a failure (the CI job with the pinned Python).
pub const REQUIRE_VAR: &str = "STA64_REQUIRE_PINNED_PYTHON";

/// The `major.minor` of the `python3` on the path, or why there is none.
fn found_version() -> Result<String, String> {
    let out = Command::new("python3")
        .args(["-I", "-c", "import sys; print(f'{sys.version_info[0]}.{sys.version_info[1]}')"])
        .output()
        .map_err(|e| format!("python3 not found ({e})"))?;
    if !out.status.success() {
        return Err("python3 did not run".to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The `major.minor` an expected file says it was generated with: its first line, a comment
/// (`# ` in a text file, `//! ` in a Rust file) reading `generated with CPython <major.minor>`.
pub fn generated_with(text: &str) -> Option<&str> {
    let line = text.lines().next()?.trim_end();
    let line = line.strip_prefix("# ").or_else(|| line.strip_prefix("//! "))?;
    line.strip_prefix("generated with CPython ")
}

/// What a staleness test does, decided from the interpreter found and the require flag.
#[derive(Debug, PartialEq)]
enum Decision {
    /// The interpreter is the pinned one: compare strictly.
    Compare,
    /// Another interpreter or none, and nothing requires the comparison: skip with this message.
    Skip(String),
    /// Another interpreter or none, but the comparison is required: fail with this message.
    Fail(String),
}

/// Decides from a found version (or the reason there is none) whether the comparison runs.
/// `required` is the [`REQUIRE_VAR`] flag: it turns a skip into a failure.
fn decide(found: Result<String, String>, pinned: &str, required: bool) -> Decision {
    let reason = match found {
        Ok(v) if v == pinned => return Decision::Compare,
        Ok(v) => format!("this Python is CPython {v}, the reference files were made with CPython {pinned}"),
        Err(why) => format!("{why}; the reference files were made with CPython {pinned}"),
    };
    if required { Decision::Fail(reason) } else { Decision::Skip(reason) }
}

/// `true` when the staleness comparison of `test` may run (the interpreter is the pinned one).
/// Otherwise prints the visible skip message and returns `false`, or panics when
/// [`REQUIRE_VAR`] is `1`. Before that, on every machine, panics when `committed` (the text of
/// the expected file `file`) does not name [`PINNED`] in its first line.
pub fn pinned_python_available(test: &str, file: &str, committed: &str) -> bool {
    match generated_with(committed) {
        Some(v) if v == PINNED => {}
        Some(v) => panic!("{file} says it was generated with CPython {v}, the pinned reference is CPython {PINNED}: regenerate it with CPython {PINNED}, or change PINNED with all expected files"),
        None => panic!("{file} does not start with the line naming the CPython it was generated with: regenerate it with CPython {PINNED}"),
    }
    let required = std::env::var(REQUIRE_VAR).as_deref() == Ok("1");
    match decide(found_version(), PINNED, required) {
        Decision::Compare => true,
        Decision::Skip(reason) => {
            eprintln!("SKIPPED: {test}: {reason}; the Rust output is still compared with the committed files by the other tests");
            false
        }
        Decision::Fail(reason) => panic!("{test} skipped ({reason}) but {REQUIRE_VAR}=1 requires the comparison"),
    }
}

#[test]
fn the_decision_follows_the_pinned_version() {
    for required in [false, true] {
        assert_eq!(decide(Ok(PINNED.to_owned()), PINNED, required), Decision::Compare);
    }
    let Decision::Skip(other) = decide(Ok("3.9".to_owned()), PINNED, false) else { panic!("another version skips") };
    assert!(other.contains("CPython 3.9") && other.contains(&format!("CPython {PINNED}")), "{other}");
    let Decision::Skip(none) = decide(Err("python3 not found (x)".to_owned()), PINNED, false) else { panic!("no python skips") };
    assert!(none.contains("not found") && none.contains(PINNED), "{none}");
}

#[test]
fn a_required_comparison_fails_instead_of_skipping() {
    // the same reasons, as failures: the job that sets the variable must not pass without comparing
    assert_eq!(decide(Ok("3.9".to_owned()), PINNED, true), Decision::Fail(format!("this Python is CPython 3.9, the reference files were made with CPython {PINNED}")));
    let Decision::Fail(none) = decide(Err("python3 not found (x)".to_owned()), PINNED, true) else { panic!("no python fails when required") };
    assert!(none.contains("not found") && none.contains(PINNED), "{none}");
}

#[test]
fn the_generating_version_is_read_from_the_first_line_only() {
    assert_eq!(generated_with("# generated with CPython 3.14\n== case\n"), Some("3.14"));
    assert_eq!(generated_with("//! generated with CPython 3.14\r\n//!\r\n"), Some("3.14"));
    assert_eq!(generated_with("== case\n# generated with CPython 3.14\n"), None);
    assert_eq!(generated_with(""), None);
}
