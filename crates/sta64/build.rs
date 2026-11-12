//! Embeds the STA64 icon and version info into the Windows executable.

/// `git <args>` in the repository, trimmed; `None` outside a checkout or without git.
fn git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|s| !s.is_empty())
}

fn main() {
    // The commit a playtest capture says it was built from (TOOL-14). Best effort:
    // the reflog grows on every commit, so a rebuild after one picks it up.
    if let Some(commit) = git(&["rev-parse", "--short=12", "HEAD"]) {
        println!("cargo:rustc-env=STA64_GIT_COMMIT={commit}");
    }
    if let Some(log) = git(&["rev-parse", "--git-path", "logs/HEAD"]) {
        println!("cargo:rerun-if-changed={log}");
    }
    println!("cargo:rerun-if-changed=../../assets/icons/STA64.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icons/STA64.ico");
        res.set("ProductName", "STA64");
        res.set("FileDescription", "STA64");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows icon: {e}");
        }
    }
}
