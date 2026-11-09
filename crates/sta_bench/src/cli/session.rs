//! `session start|stop|list` and `drive` (docs/TESTBENCH.md, "Sessions and
//! reports").
//!
//! A session is a headless `sta64 --control stdio` that outlives the
//! `sta-bench` call that started it. Its stdin is a named pipe held open by a
//! sleeping writer, its stdout a file; `drive` writes a command line to the
//! pipe and prints the answer line that appears in the file. State lives in
//! `<results parent>/sessions/<id>/` (`$STA64_BENCH_SESSIONS` overrides).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::tier2::{materialize_scene, sta64_path};
use super::{Fail, Opts, Outcome, config, report_root};
use crate::parse::parse_command;

fn root(o: &Opts) -> PathBuf {
    std::env::var_os("STA64_BENCH_SESSIONS").map(PathBuf::from).unwrap_or_else(|| {
        report_root(o).parent().map_or_else(|| PathBuf::from("sessions"), |p| p.join("sessions"))
    })
}

/// `20261108-142501-48213` from the clock and the process id.
fn new_id() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil date from days since 1970-01-01 (Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}-{}", rem / 3600, rem % 3600 / 60, rem % 60, std::process::id())
}

fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

fn read_pid(dir: &Path, file: &str) -> Option<u32> {
    fs::read_to_string(dir.join(file)).ok()?.trim().parse().ok()
}

fn kill(pid: u32) {
    let _ = Command::new("kill").arg(pid.to_string()).stderr(Stdio::null()).status();
}

pub(crate) fn session(o: &Opts) -> Outcome {
    match o.rest.first().map(String::as_str) {
        Some("start") => start(o),
        Some("stop") => stop(o),
        Some("list") => list(o),
        _ => Err(Fail::Usage("session: start --scene <name> | stop --session ID | list".into())),
    }
}

fn start(o: &Opts) -> Outcome {
    if !cfg!(unix) {
        return Err(Fail::Error("sessions need a Unix system (named pipes)".into()));
    }
    let name = o.scene.clone().ok_or_else(|| Fail::Usage("session start: --scene <name>".into()))?;
    let overrides: Vec<(String, String)> =
        o.rest[1..].iter().filter_map(|w| w.split_once('=')).map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
    let cfg = config(o);
    let scene = materialize_scene(&cfg.scene_dir, &name, &overrides).map_err(Fail::Error)?;
    let id = new_id();
    let dir = root(o).join(&id);
    fs::create_dir_all(&dir).map_err(|e| Fail::Error(format!("{}: {e}", dir.display())))?;
    let (scene_path, fifo) = (dir.join("scene.ini"), dir.join("in"));
    fs::write(&scene_path, scene).map_err(|e| Fail::Error(e.to_string()))?;
    let made = Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        return Err(Fail::Error("mkfifo failed".into()));
    }
    let err = |e: std::io::Error| Fail::Error(e.to_string());

    // The write end stays open in a process of its own, so sta64 never sees
    // end of input between two `drive` calls.
    let holder = Command::new("sh")
        .args(["-c", "exec sleep 2147483647 > \"$0\"", fifo.to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(err)?;
    let input = File::open(&fifo).map_err(err)?;
    let mut cmd = Command::new(sta64_path(o));
    cmd.args(["--headless", "--clock", "bench", "--control", "stdio", "--scene"])
        .arg(&scene_path)
        .stdin(Stdio::from(input))
        .stdout(File::create(dir.join("out.log")).map_err(err)?)
        .stderr(File::create(dir.join("err.log")).map_err(err)?);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let child = cmd.spawn().map_err(|e| Fail::Error(format!("{}: {e}", sta64_path(o).display())))?;
    fs::write(dir.join("pid"), child.id().to_string()).map_err(err)?;
    fs::write(dir.join("holder.pid"), holder.id().to_string()).map_err(err)?;

    // Hold the clock until the first drive, then prove it is listening.
    match send(&dir, "pause", o.timeout) {
        Ok(a) => println!("{id}\n  started: {a}"),
        Err(e) => {
            kill(child.id());
            kill(holder.id());
            let tail = |f: &str| {
                let t = fs::read_to_string(dir.join(f)).unwrap_or_default();
                let lines: Vec<&str> = t.lines().collect();
                lines[lines.len().saturating_sub(15)..].join("\n")
            };
            return Err(Fail::Error(format!(
                "session did not start: {e} (see {}/err.log)\n--- err.log ---\n{}\n--- out.log ---\n{}",
                dir.display(),
                tail("err.log"),
                tail("out.log")
            )));
        }
    }
    println!("  drive: sta-bench drive --session {id} \"<command>\"");
    println!("  stop:  sta-bench session stop --session {id}");
    Ok(0)
}

fn pick(o: &Opts) -> Result<(String, PathBuf), Fail> {
    let id = o
        .session
        .clone()
        .or_else(|| std::env::var("STA64_BENCH_SESSION").ok())
        .ok_or_else(|| Fail::Usage("name a session: --session ID or STA64_BENCH_SESSION".into()))?;
    let dir = root(o).join(&id);
    if !dir.is_dir() {
        return Err(Fail::Error(format!("no session `{id}` (ended, or never started)")));
    }
    Ok((id, dir))
}

fn stop(o: &Opts) -> Outcome {
    let (id, dir) = pick(o)?;
    for f in ["pid", "holder.pid"] {
        if let Some(p) = read_pid(&dir, f) {
            kill(p);
        }
    }
    let _ = fs::remove_dir_all(&dir);
    println!("stopped {id}");
    Ok(0)
}

fn list(o: &Opts) -> Outcome {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root(o)).into_iter().flatten().flatten().map(|e| e.path()).collect();
    dirs.sort();
    for d in dirs.iter().filter(|d| d.is_dir()) {
        let alive = read_pid(d, "pid").is_some_and(pid_alive);
        println!("{}  {}", d.file_name().unwrap().to_string_lossy(), if alive { "running" } else { "ended" });
    }
    Ok(0)
}

/// Write one command line and wait for its answer line.
fn send(dir: &Path, line: &str, secs: u64) -> Result<String, String> {
    if !read_pid(dir, "pid").is_some_and(pid_alive) {
        return Err("the session has ended".into());
    }
    let log = dir.join("out.log");
    let before = fs::metadata(&log).map_or(0, |m| m.len());
    let mut pipe = OpenOptions::new().write(true).open(dir.join("in")).map_err(|e| e.to_string())?;
    writeln!(pipe, "{line}").map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        if let Ok(text) = fs::read_to_string(&log) {
            let new = text.get(before as usize..).unwrap_or("");
            if let Some(end) = new.find('\n') {
                return Ok(new[..end].to_owned());
            }
        }
        if !read_pid(dir, "pid").is_some_and(pid_alive) {
            return Err("the session ended before it answered".into());
        }
        if start.elapsed() > Duration::from_secs(secs) {
            return Err(format!("no answer in {secs} s"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn drive(o: &Opts) -> Outcome {
    if o.rest.is_empty() {
        return Err(Fail::Usage("drive --session ID \"<command>\"...".into()));
    }
    let (_, dir) = pick(o)?;
    let mut code = 0;
    for line in &o.rest {
        match parse_command(line) {
            Ok(None) => continue,
            Err(e) => {
                println!("! {e}");
                code = 1;
                continue;
            }
            Ok(Some(_)) => {}
        }
        let answer = send(&dir, line, o.timeout).map_err(Fail::Error)?;
        println!("{answer}");
        if answer.starts_with('!') {
            code = 1;
        }
    }
    Ok(code)
}
