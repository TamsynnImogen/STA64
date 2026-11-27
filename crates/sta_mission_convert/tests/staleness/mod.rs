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

/// Decides from a found version (or the reason there is none) whether the comparison may run;
/// `Err` carries the skip message.
fn decide(found: Result<String, String>, pinned: &str) -> Result<(), String> {
    match found {
        Ok(v) if v == pinned => Ok(()),
        Ok(v) => Err(format!("this Python is CPython {v}, the reference files were made with CPython {pinned}")),
        Err(why) => Err(format!("{why}; the reference files were made with CPython {pinned}")),
    }
}

/// `true` when the staleness comparison of `test` may run (the interpreter is the pinned one).
/// Otherwise prints the visible skip message and returns `false`, or panics when
/// [`REQUIRE_VAR`] is `1`.
pub fn pinned_python_available(test: &str) -> bool {
    match decide(found_version(), PINNED) {
        Ok(()) => true,
        Err(reason) => {
            eprintln!("SKIPPED: {test}: {reason}; the Rust output is still compared with the committed files by the other tests");
            assert!(std::env::var(REQUIRE_VAR).as_deref() != Ok("1"), "{test} skipped ({reason}) but {REQUIRE_VAR}=1 requires the comparison");
            false
        }
    }
}

#[test]
fn the_decision_follows_the_pinned_version() {
    assert_eq!(decide(Ok(PINNED.to_owned()), PINNED), Ok(()));
    let other = decide(Ok("3.9".to_owned()), PINNED).unwrap_err();
    assert!(other.contains("CPython 3.9") && other.contains(&format!("CPython {PINNED}")), "{other}");
    let none = decide(Err("python3 not found (x)".to_owned()), PINNED).unwrap_err();
    assert!(none.contains("not found") && none.contains(PINNED), "{none}");
}
