//! The footage log's file: the stamps, one JSON line per record, flushed per line.

#[cfg(test)]
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// The format tag of the first record.
pub const FORMAT: &str = "sta64-footage/1";

/// Wall-clock stamps that never go backwards: the Unix time is read once, at the start, and the
/// monotonic time since then is added to it.
#[derive(Debug, Clone, Copy)]
pub struct Stamps {
    started_ms: u64,
    started: Instant,
}

impl Stamps {
    pub fn now() -> Stamps {
        let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        Stamps { started_ms: ms, started: Instant::now() }
    }

    pub fn started_ms(&self) -> u64 {
        self.started_ms
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// `(ms, t)`: Unix epoch milliseconds and seconds since the start, for `elapsed` after it.
    pub fn at(&self, elapsed: Duration) -> (u64, f64) {
        (self.started_ms + elapsed.as_millis() as u64, round3(elapsed.as_secs_f64()))
    }
}

/// `x` rounded to three decimals, as the file writes numbers.
pub fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// The record's line (no newline): `ms`, `t`, `tick`, `type`, then the fields by name.
pub fn line(ms: u64, t: f64, tick: Option<u64>, ty: &str, fields: Value) -> String {
    let mut s = format!("{{\"ms\":{ms},\"t\":{},\"tick\":{},\"type\":{}", Value::from(t), tick.map_or("null".to_owned(), |t| t.to_string()), Value::from(ty));
    if let Value::Object(map) = fields {
        for (k, v) in map {
            s.push(',');
            s.push_str(&Value::from(k).to_string());
            s.push(':');
            s.push_str(&v.to_string());
        }
    }
    s.push('}');
    s
}

/// A shared in-memory sink, for tests.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct MemSink(pub Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl Write for MemSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
impl MemSink {
    pub fn lines(&self) -> Vec<Value> {
        String::from_utf8_lossy(&self.0.lock().unwrap()).lines().map(|l| serde_json::from_str(l).expect("a JSON line")).collect()
    }
}

/// An RFC 3339 UTC time for Unix `ms`, from `sta_bench`'s `YYYYMMDD-HHMMSS` stamp.
pub fn rfc3339(ms: u64) -> String {
    let s = sta_bench::capture::stamp(ms / 1000);
    // `20261112-143015`
    format!("{}-{}-{}T{}:{}:{}.{:03}Z", &s[0..4], &s[4..6], &s[6..8], &s[9..11], &s[11..13], &s[13..15], ms % 1000)
}

/// Where the log goes for the `--footage-log` / `STA64_FOOTAGE_LOG` value `spec`.
///
/// Empty or `1`: `footage/<stamp>.jsonl` next to the captures folder. An existing directory or a
/// value ending in a separator: `<stamp>.jsonl` in it. Anything else is the file itself.
pub fn target_path(spec: &str, started_ms: u64, default_dir: &Path) -> PathBuf {
    let file = format!("{}.jsonl", sta_bench::capture::stamp(started_ms / 1000));
    let spec = spec.trim();
    if spec.is_empty() || spec == "1" {
        return default_dir.join(file);
    }
    let p = PathBuf::from(spec);
    if p.is_dir() || spec.ends_with('/') || spec.ends_with(std::path::MAIN_SEPARATOR) { p.join(file) } else { p }
}

/// `<captures folder's parent>/footage`: the per-user data directory next to the captures.
pub fn default_dir() -> PathBuf {
    let captures = sta_bench::capture::default_capture_dir();
    captures.parent().map_or_else(|| captures.join("footage"), |p| p.join("footage"))
}

/// Opens the file, with its folders, for appending lines.
pub fn open(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stamps_add_the_monotonic_time_to_the_start() {
        let s = Stamps { started_ms: 1_700_000_000_000, started: Instant::now() };
        assert_eq!(s.at(Duration::ZERO), (1_700_000_000_000, 0.0));
        assert_eq!(s.at(Duration::from_millis(1234)), (1_700_000_001_234, 1.234));
        let mut last = s.at(Duration::ZERO);
        for ms in [1, 1, 5, 1000, 1000, 86_400_000] {
            let next = s.at(Duration::from_millis(ms));
            assert!(next.0 >= last.0 && next.1 >= last.1);
            last = next;
        }
        // The real clock never steps back either.
        let real = Stamps::now();
        let a = real.at(real.elapsed());
        let b = real.at(real.elapsed());
        assert!(b.0 >= a.0 && b.1 >= a.1 && a.0 >= real.started_ms());
    }

    #[test]
    fn rfc3339_from_unix_ms() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    }

    #[test]
    fn target_paths() {
        let d = Path::new("/data/sta64/footage");
        let ms = 1_700_000_000_000;
        let default = d.join("20231114-221320.jsonl");
        assert_eq!(target_path("", ms, d), default);
        assert_eq!(target_path("1", ms, d), default);
        assert_eq!(target_path("/tmp/x/run.jsonl", ms, d), Path::new("/tmp/x/run.jsonl"));
        assert_eq!(target_path("/tmp/x/", ms, d), Path::new("/tmp/x/20231114-221320.jsonl"));
        let tmp = std::env::temp_dir();
        assert_eq!(target_path(tmp.to_str().unwrap(), ms, d), tmp.join("20231114-221320.jsonl"));
    }

    #[test]
    fn golden_lines() {
        let l = line(1_700_000_000_123, 0.5, None, "start", json!({"format": FORMAT, "pid": 7}));
        assert_eq!(l, r#"{"ms":1700000000123,"t":0.5,"tick":null,"type":"start","format":"sta64-footage/1","pid":7}"#);
        let l = line(1_700_000_001_000, 1.0, Some(42), "pause", json!({"on": true}));
        assert_eq!(l, r#"{"ms":1700000001000,"t":1.0,"tick":42,"type":"pause","on":true}"#);
        let l = line(5, 0.001, Some(0), "cast", json!({"units": [{"id": 3, "x": 0.25}], "total": 1}));
        assert_eq!(l, r#"{"ms":5,"t":0.001,"tick":0,"type":"cast","total":1,"units":[{"id":3,"x":0.25}]}"#);
        // Every line parses back.
        assert!(serde_json::from_str::<Value>(&l).is_ok());
    }
}
