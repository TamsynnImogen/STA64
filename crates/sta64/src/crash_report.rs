//! Local, human-readable reports for Rust panics and fatal app exits.
//!
//! These are best-effort diagnostics. The original panic hook still runs, and
//! this module never catches a panic or changes the app's exit behavior.

use std::backtrace::Backtrace;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::path::PathBuf;
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

static INSTALLED: Once = Once::new();

/// Install once, before building or running the Bevy app.
pub(crate) fn install() {
    INSTALLED.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let message = panic_message(info);
            let location = info
                .location()
                .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()));
            let backtrace = Backtrace::force_capture();
            report("panic", &message, location.as_deref(), Some(&backtrace));
            previous(info);
        }));
    });
}

/// Record an unrecoverable Bevy `AppExit::Error` before the caller exits.
pub(crate) fn fatal_exit(code: u8) {
    report(
        "fatal-exit",
        &format!("Bevy returned AppExit::Error({code})"),
        None,
        None,
    );
}

fn panic_message(info: &PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

fn report(kind: &str, message: &str, location: Option<&str>, backtrace: Option<&Backtrace>) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let stamp = sta_bench::capture::stamp(now.as_secs());
    let timestamp = format!(
        "{}-{}-{} {}:{}:{}.{:03} UTC",
        &stamp[0..4],
        &stamp[4..6],
        &stamp[6..8],
        &stamp[9..11],
        &stamp[11..13],
        &stamp[13..15],
        now.subsec_millis(),
    );
    let mut body = format!(
        "STA64 diagnostic report\n\
         Kind: {kind}\n\
         Time: {timestamp}\n\
         Version: {}\n\
         Process: {}\n\
         Thread: {}\n\
         Message: {message}\n",
        crate::platform::diagnostic_build(),
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed"),
    );
    if let Some(location) = location {
        body.push_str(&format!("Location: {location}\n"));
    }
    if let Some(backtrace) = backtrace {
        body.push_str(&format!("\nBacktrace:\n{backtrace}\n"));
    }
    let filename = format!(
        "{kind}-{stamp}-{:03}-{}.txt",
        now.subsec_millis(),
        std::process::id()
    );
    match write_report(&filename, body.as_bytes()) {
        Ok(path) => {
            let _ = writeln!(io::stderr(), "STA64 {kind} report: {}", path.display());
        }
        Err(err) => {
            let _ = writeln!(
                io::stderr(),
                "STA64 could not save {kind} report: {err}\n{body}"
            );
        }
    }
}

fn write_report(filename: &str, body: &[u8]) -> io::Result<PathBuf> {
    let dir = crate::platform::diagnostics_dir();
    std::fs::create_dir_all(&dir)?;
    for n in 1..=1000 {
        let name = if n == 1 {
            filename.to_owned()
        } else {
            filename.replace(".txt", &format!("-{n}.txt"))
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(body)?;
                return Ok(path);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free diagnostic report name",
    ))
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "launched in a subprocess by panic_writes_a_readable_report"]
    fn write_panic_in_child() {
        if std::env::var_os("STA64_TEST_PANIC_REPORT").is_some() {
            install();
            panic!("diagnostic test panic");
        }
    }

    #[test]
    fn panic_writes_a_readable_report() {
        let dir = std::env::temp_dir().join(format!("sta64-panic-test-{}", std::process::id()));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crash_report::tests::write_panic_in_child",
                "--ignored",
                "--nocapture",
            ])
            .env("STA64_TEST_PANIC_REPORT", "1")
            .env("STA64_DATA_DIR", &dir)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("STA64 panic report"));
        let path = std::fs::read_dir(dir.join("logs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let report = std::fs::read_to_string(path).unwrap();
        assert!(report.contains("Message: diagnostic test panic"));
        assert!(report.contains("Location: "));
        assert!(report.contains("Backtrace:"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
