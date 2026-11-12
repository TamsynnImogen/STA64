//! Parity diff against Refit's bench (docs/TESTBENCH.md, "Parity with stock").
//!
//! Refit's bench (`armada2-refit/testbench/`) leaves two files per game run:
//! `Scene.objects.jsonl` (a header with the simulation's `time` in seconds,
//! then one JSON line per object) and `Scene.marks` (lines of
//! `GETTICKCOUNT kind k=v ...` plus `epoch` lines, in wall milliseconds).
//! `sta-bench` leaves `objects.jsonl` (header with a `tick`) and `marks`
//! (`TICK kind k=v ...`). Both are read into one [`Run`]: objects keyed by
//! name, marks in simulation seconds, kinds in Refit's vocabulary
//! ([`NAME_MAP`]). [`diff`] then compares two runs within [`Tolerances`].
//!
//! Sim seconds: ours are `tick / TICK_HZ`. Refit's marks are placed on the
//! sim clock by taking the `ready` mark (written as the scene is built, tick
//! 0) as time 0, or the first mark when there is none; under `Clock: bench`
//! the game's tick count and sim time advance together.
//!
//! Marks are paired by `(kind, obj)` and then by order (the n-th `fire.start`
//! of `warbird` in A against the n-th in B); the pair's times are compared.
//! Kinds only one engine emits are counted, not reported as differences
//! (see [`Support`]).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use sta_core::TICK_HZ;

// ---- the model ---------------------------------------------------------

/// One object, as both engines list it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Object {
    pub odf: Option<String>,
    pub pos: Option<[f64; 3]>,
    /// `[now, max]`.
    pub hull: Option<[f64; 2]>,
    pub shields: Option<[f64; 2]>,
    pub dying: Option<bool>,
}

/// One mark, normalised.
#[derive(Debug, Clone, PartialEq)]
pub struct PMark {
    /// Simulation seconds.
    pub t: f64,
    /// Refit's vocabulary (after [`NAME_MAP`]).
    pub kind: String,
    /// The object it names (`obj=`), lower-cased.
    pub obj: Option<String>,
    pub fields: BTreeMap<String, String>,
}

/// A run, normalised: objects by name, marks in sim seconds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Run {
    /// Sim seconds of the objects snapshot, when the file says.
    pub time: Option<f64>,
    pub objects: BTreeMap<String, Object>,
    /// Objects with neither a name nor a label (not comparable).
    pub unnamed: usize,
    pub marks: Vec<PMark>,
}

// ---- mark names --------------------------------------------------------

/// Which engine emits a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Both,
    RefitOnly,
    OursOnly,
    /// Not in [`NAME_MAP`]: compared by kind and object when both runs have
    /// it, and named in the diff's notes, never dropped silently.
    Unknown,
}

/// The measuring kinds (`[Marks] Ordnance=1`, `State=<ms>`, `bank`; TOOL-16).
/// `state` is compared as a series per object, the others one by one.
pub const MEASURING: &[&str] = &["state", "ord.new", "ord.gone", "bank"];

/// One row of the mark-name table. `ours` is the kind (and fields) STA64's
/// `marks.rs` writes; `refit` is the kind Refit's `Scene.asi` writes; the
/// canonical name used by [`Run`] is `refit`. Where the two differ in name
/// or shape, `note` says how.
pub struct NameRow {
    pub refit: &'static str,
    pub ours: &'static str,
    /// A field value of ours that selects this row (`cloak on=1`).
    pub ours_when: Option<(&'static str, &'static str)>,
    pub support: Support,
    pub note: &'static str,
}

const fn row(
    refit: &'static str,
    ours: &'static str,
    ours_when: Option<(&'static str, &'static str)>,
    support: Support,
    note: &'static str,
) -> NameRow {
    NameRow { refit, ours, ours_when, support, note }
}

use Support::{Both, OursOnly, RefitOnly};

/// Refit's mark kinds (testbench/scene/README.md "Marks", films/README.md
/// "Marks"; the emitting calls in `scene.c`) against STA64's TOOL-07 kinds.
pub const NAME_MAP: &[NameRow] = &[
    row("fire.start", "fire.start", None, Both, "fields differ: Refit obj,weapon,target,shots,secs; ours obj,weapon,hp. Compared on obj and weapon"),
    row("fire.end", "fire.end", None, Both, "as fire.start; Refit's is FireGap (1 s) after the last ordnance, ours last shot + weapon duration"),
    row("destroyed", "death", None, Both, "name differs: Refit `destroyed` (how=removed for `kill`), ours `death` (by=)"),
    row("cloak.start", "cloak", Some(("on", "1")), Both, "shape differs: ours is one `cloak on=1|0` mark"),
    row("decloak.start", "cloak", Some(("on", "0")), Both, "ours `cloak on=0`"),
    row("cloak.done", "", None, RefitOnly, "ours has no done step; Refit marks it 3 s after start"),
    row("decloak.done", "", None, RefitOnly, "as cloak.done"),
    row("removed", "", None, RefitOnly, "an object is gone; ours folds it into `death`"),
    row("ready", "", None, RefitOnly, "scene built; used as time 0"),
    row("cmd", "", None, RefitOnly, "a scene command was taken"),
    row("select", "", None, RefitOnly, "selection changed"),
    row("special", "", None, RefitOnly, "a `special` command's answer"),
    row("explosion", "", None, RefitOnly, "a new explosion; ours reports it as `death`"),
    row("camera.move", "", None, RefitOnly, "camera is a renderer matter"),
    row("camera.still", "", None, RefitOnly, ""),
    row("glide.start", "", None, RefitOnly, ""),
    row("glide.end", "", None, RefitOnly, ""),
    row("spin.start", "", None, RefitOnly, ""),
    row("spin.stop", "", None, RefitOnly, ""),
    row("hit", "hit", None, OursOnly, "Refit has no per-hit mark (only explosions)"),
    row("shields.down", "shields.down", None, OursOnly, ""),
    row("spawn", "spawn", None, OursOnly, ""),
    row("build.start", "build.start", None, OursOnly, ""),
    row("built", "built", None, OursOnly, ""),
    row("delivered", "delivered", None, OursOnly, ""),
    row("rejected", "rejected", None, OursOnly, ""),
    row("eliminated", "eliminated", None, OursOnly, ""),
    // Measuring marks (`[Marks] SimTime=1`, `Ordnance=1`, `State=<ms>`; Refit's
    // `bench/measure-marks`, ours `run/marks.rs`), `t=` in sim seconds.
    row("state", "state", None, Both, "pos hull sh sp every State ms (ours: no q/ma/swa); compared as a series per object: position, hull, shields, special-energy slope"),
    row("ord.new", "ord.new", None, Both, "one shot: id obj weapon target at; weapon is the ordnance ODF (ordName) on both"),
    row("ord.gone", "ord.gone", None, Both, "a shot's end: at life tgt_at d; ours only when it hits (the sim has no expiry event)"),
    row("bank", "bank", None, Both, "a team's resources by name (the `bank` command); compared within the hull tolerance"),
];

/// Support of a canonical kind (`action.*` and `menu.*` are the film
/// runner's and `UI.log`'s, never the sim's).
pub fn support(kind: &str) -> Support {
    if kind.starts_with("action.") || kind.starts_with("menu.") {
        return RefitOnly;
    }
    NAME_MAP.iter().find(|r| r.refit == kind).map_or(Support::Unknown, |r| r.support)
}

/// Our kind and fields into Refit's kind.
pub(crate) fn canonical_ours(kind: &str, fields: &BTreeMap<String, String>) -> String {
    for r in NAME_MAP.iter().filter(|r| r.ours == kind && !r.ours.is_empty()) {
        match r.ours_when {
            Some((k, v)) if fields.get(k).map(String::as_str) == Some(v) => return r.refit.into(),
            Some(_) => {}
            None => return r.refit.into(),
        }
    }
    kind.into()
}

// ---- a small JSON reader (flat objects; the crate has no serde) --------

#[derive(Debug, Clone, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Option<()> {
        self.ws();
        (self.s.get(self.i) == Some(&c)).then(|| self.i += 1)
    }
    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'u' => {
                            let h = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            self.i += 4;
                            let ch = char::from_u32(u32::from_str_radix(h, 16).ok()?)?;
                            out.extend(ch.to_string().bytes());
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
    }
    fn value(&mut self) -> Option<J> {
        self.ws();
        match *self.s.get(self.i)? {
            b'"' => self.string().map(J::Str),
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(J::Arr(v));
                }
                loop {
                    v.push(self.value()?);
                    self.ws();
                    match *self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(J::Arr(v));
                        }
                        _ => return None,
                    }
                }
            }
            _ => {
                let st = self.i;
                while self.s.get(self.i).is_some_and(|c| !matches!(c, b',' | b'}' | b']')) {
                    self.i += 1;
                }
                match std::str::from_utf8(&self.s[st..self.i]).ok()?.trim() {
                    "null" => Some(J::Null),
                    "true" => Some(J::Bool(true)),
                    "false" => Some(J::Bool(false)),
                    n => n.parse().ok().map(J::Num),
                }
            }
        }
    }
}

/// A line holding one flat JSON object.
fn json_line(line: &str) -> Option<BTreeMap<String, J>> {
    let mut p = P { s: line.as_bytes(), i: 0 };
    p.eat(b'{')?;
    let mut m = BTreeMap::new();
    p.ws();
    if p.s.get(p.i) == Some(&b'}') {
        return Some(m);
    }
    loop {
        let k = p.string()?;
        p.eat(b':')?;
        m.insert(k, p.value()?);
        p.ws();
        match *p.s.get(p.i)? {
            b',' => p.i += 1,
            b'}' => return Some(m),
            _ => return None,
        }
    }
}

fn num(m: &BTreeMap<String, J>, k: &str) -> Option<f64> {
    match m.get(k)? {
        J::Num(n) => Some(*n),
        _ => None,
    }
}

fn nums<const N: usize>(m: &BTreeMap<String, J>, k: &str) -> Option<[f64; N]> {
    let J::Arr(a) = m.get(k)? else { return None };
    let v: Vec<f64> = a
        .iter()
        .filter_map(|x| if let J::Num(n) = x { Some(*n) } else { None })
        .collect();
    v.try_into().ok()
}

fn str_of(m: &BTreeMap<String, J>, k: &str) -> Option<String> {
    match m.get(k)? {
        J::Str(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

// ---- readers -----------------------------------------------------------

/// Which engine's files a text is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Refit,
    Ours,
}

/// `objects.jsonl` of either engine. The header has `time` (Refit, sim
/// seconds) or `tick` (ours). An object is keyed by `name`, else by its
/// script `label` (Refit), else it is counted in `unnamed`; names compare
/// case-insensitively.
pub fn read_objects(text: &str, source: Source, run: &mut Run) {
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Some(m) = json_line(line) else { continue };
        if m.contains_key("header") || m.contains_key("schema") {
            run.time = num(&m, "time").or_else(|| num(&m, "tick").map(|t| t / f64::from(TICK_HZ)));
            continue;
        }
        let Some(name) = str_of(&m, "name").or_else(|| str_of(&m, "label")) else {
            run.unnamed += 1;
            continue;
        };
        // Ours names an unnamed object `#id`: not comparable across engines.
        if source == Source::Ours && name.starts_with('#') {
            run.unnamed += 1;
            continue;
        }
        let o = Object {
            odf: str_of(&m, "odf"),
            pos: nums::<3>(&m, "pos"),
            hull: nums::<2>(&m, "hull"),
            shields: nums::<2>(&m, "shields"),
            dying: match m.get("dying") {
                Some(J::Bool(b)) => Some(*b),
                _ => None,
            },
        };
        run.objects.insert(name.to_lowercase(), o);
    }
}

/// A `key=value` field list; Refit's `text` runs to the end of the line.
fn fields_of(rest: &str) -> BTreeMap<String, String> {
    let mut f = BTreeMap::new();
    let mut rest = rest.trim_start();
    while !rest.is_empty() {
        if let Some(t) = rest.strip_prefix("text=") {
            f.insert("text".into(), t.to_owned());
            break;
        }
        let (word, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        if let Some((k, v)) = word.split_once('=') {
            f.insert(k.to_owned(), v.to_owned());
        }
        rest = tail.trim_start();
    }
    f
}

fn mark(t: f64, kind: String, fields: BTreeMap<String, String>) -> PMark {
    let obj = fields.get("obj").map(|o| o.to_lowercase());
    PMark { t, kind, obj, fields }
}

fn split_mark_line(line: &str) -> Option<(u64, &str, &str)> {
    let (tick, rest) = line.trim().split_once(char::is_whitespace)?;
    // `epoch 0x.. 0x.. N` and comments do not start with a number.
    let tick = tick.parse::<u64>().ok()?;
    let rest = rest.trim_start();
    let (kind, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    Some((tick, kind, tail))
}

/// Refit's `Scene.marks`: `TICK kind k=v ...` in `GetTickCount()` ms, `epoch`
/// lines skipped. Time 0 is the `ready` mark, else the first mark.
///
/// With `[Marks] SimTime=1` every mark carries `t=`, the simulation's seconds;
/// then the marks are placed by `t=` (minus the `ready` mark's), never by the
/// wall clock, which runs at another rate than the sim on Roots and whenever
/// the game speed is not 1.
pub fn read_refit_marks(text: &str, run: &mut Run) {
    let raw: Vec<(u64, &str, BTreeMap<String, String>)> = text
        .lines()
        .filter_map(split_mark_line)
        .map(|(t, k, tail)| (t, k, fields_of(tail)))
        .collect();
    let sim_t = |f: &BTreeMap<String, String>| f.get("t").and_then(|t| t.parse::<f64>().ok());
    if raw.iter().any(|r| sim_t(&r.2).is_some()) {
        let origin = raw
            .iter()
            .find(|r| r.1 == "ready")
            .and_then(|r| sim_t(&r.2))
            .or_else(|| raw.iter().find_map(|r| sim_t(&r.2)))
            .unwrap_or(0.0);
        // `fire.end` is dated back to the burst's last ordnance in the wall
        // column, but its `t=` is the time it was written (FireGap later): its
        // sim time is read off the wall column, between the marks around it.
        let w0 = raw.first().map_or(0, |r| r.0 as u32);
        let wall = |w: u64| f64::from((w as u32).wrapping_sub(w0) as i32);
        let mut anchors: Vec<(f64, f64)> = raw
            .iter()
            .filter(|r| r.1 != "fire.end")
            .filter_map(|r| Some((wall(r.0), sim_t(&r.2)?)))
            .collect();
        anchors.sort_by(|a, b| a.0.total_cmp(&b.0));
        let at_wall = |w: f64| -> Option<f64> {
            let i = anchors.partition_point(|a| a.0 < w);
            match (i.checked_sub(1).and_then(|j| anchors.get(j)), anchors.get(i)) {
                (Some(a), Some(b)) if b.0 > a.0 => Some(a.1 + (b.1 - a.1) * (w - a.0) / (b.0 - a.0)),
                (_, Some(b)) => Some(b.1),
                (Some(a), None) => Some(a.1),
                (None, None) => None,
            }
        };
        for (w, kind, fields) in raw {
            let t = if kind == "fire.end" { at_wall(wall(w)).or_else(|| sim_t(&fields)) } else { sim_t(&fields) };
            // A line without `t=` (none are written so) has no place on the axis.
            if let Some(t) = t {
                run.marks.push(mark(t - origin, kind.to_owned(), fields));
            }
        }
    } else {
        let origin = raw.iter().find(|r| r.1 == "ready").or(raw.first()).map_or(0, |r| r.0);
        for (tick, kind, fields) in raw {
            // GetTickCount wraps at 2^32 ms; the difference as i32 survives it.
            let ms = f64::from((tick as u32).wrapping_sub(origin as u32) as i32);
            run.marks.push(mark(ms / 1000.0, kind.to_owned(), fields));
        }
    }
    run.marks.sort_by(|a, b| a.t.total_cmp(&b.t));
}

/// STA64's `marks`: `TICK kind k=v ...`, `TICK_HZ` ticks a second.
pub fn read_our_marks(text: &str, run: &mut Run) {
    for (tick, kind, tail) in text.lines().filter_map(split_mark_line) {
        let fields = fields_of(tail);
        let kind = canonical_ours(kind, &fields);
        run.marks.push(mark(tick as f64 / f64::from(TICK_HZ), kind, fields));
    }
    run.marks.sort_by(|a, b| a.t.total_cmp(&b.t));
}

/// A run directory: `<dir>` or `<dir>/logs` holding Refit's `Scene.marks` and
/// `Scene.objects.jsonl`, or our `marks` and `objects.jsonl`. Either file may
/// be missing; both missing is an error.
pub fn load(dir: &Path, source: Source) -> Result<Run, String> {
    let (m, o) = match source {
        Source::Refit => ("Scene.marks", "Scene.objects.jsonl"),
        Source::Ours => ("marks", "objects.jsonl"),
    };
    let find = |name: &str| -> Option<PathBuf> {
        [dir.join(name), dir.join("logs").join(name)]
            .into_iter()
            .find(|p| p.is_file())
    };
    let (mp, op) = (find(m), find(o));
    if mp.is_none() && op.is_none() {
        return Err(format!("{}: neither {m} nor {o} (also looked in logs/)", dir.display()));
    }
    let read = |p: &Path| fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let mut run = Run::default();
    if let Some(p) = op {
        read_objects(&read(&p)?, source, &mut run);
    }
    if let Some(p) = mp {
        let t = read(&p)?;
        match source {
            Source::Refit => read_refit_marks(&t, &mut run),
            Source::Ours => read_our_marks(&t, &mut run),
        }
    }
    Ok(run)
}

// ---- the diff ----------------------------------------------------------

/// How close counts as the same. The default time tolerance is two stock
/// ticks (0.1 s): a mark also leads the first frame showing it by 0.05 s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerances {
    /// Seconds.
    pub time: f64,
    /// World units (distance).
    pub pos: f64,
    /// Hull and shield points (and resources in `bank`).
    pub hull: f64,
    /// Special energy per second: the slope of `sp` over a `state` series.
    pub slope: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self { time: 0.1, pos: 1.0, hull: 1.0, slope: 0.5 }
    }
}

impl Tolerances {
    /// `time=0.2,pos=5,hull=2`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut t = Self::default();
        for part in s.split(',').filter(|p| !p.is_empty()) {
            let (k, v) = part
                .split_once('=')
                .ok_or_else(|| format!("`{part}`: expected key=value"))?;
            let v: f64 = v.parse().map_err(|_| format!("`{part}`: not a number"))?;
            match k {
                "time" => t.time = v,
                "pos" => t.pos = v,
                "hull" => t.hull = v,
                "slope" => t.slope = v,
                _ => return Err(format!("unknown tolerance `{k}` (time, pos, hull, slope)")),
            }
        }
        Ok(t)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Finding {
    ObjectOnlyIn { side: char, name: String },
    ObjectField { name: String, field: &'static str, a: String, b: String },
    MarkOnlyIn { side: char, kind: String, obj: Option<String>, t: f64 },
    MarkTime { kind: String, obj: Option<String>, n: usize, a: f64, b: f64 },
    MarkField { kind: String, obj: Option<String>, n: usize, field: &'static str, a: String, b: String },
    /// A `state` series: the first sample where `field` (pos distance, hull,
    /// shields) is beyond tolerance; `a`/`b` the two values (pos: distance in b).
    Series { obj: String, field: &'static str, t: f64, a: f64, b: f64 },
    /// A `state` series: special energy per second (least squares) differs.
    Slope { obj: String, a: f64, b: f64 },
    /// A `state` series only one run has.
    SeriesOnlyIn { side: char, obj: String },
}

#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub findings: Vec<Finding>,
    /// Facts that are not differences (kinds skipped, snapshot times, ...).
    pub notes: Vec<String>,
    pub objects_compared: usize,
    pub marks_compared: usize,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    /// A readable report. A is the first run (Refit in the CLI), B the second.
    pub fn report(&self, a: &str, b: &str) -> String {
        let mut s = format!("parity: A = {a}, B = {b}\n");
        let side = |c: char| if c == 'A' { a } else { b };
        let o = |o: &Option<String>| o.as_deref().map_or(String::new(), |o| format!(" obj={o}"));
        for f in &self.findings {
            let _ = match f {
                Finding::ObjectOnlyIn { side: c, name } => {
                    writeln!(s, "  object `{name}` only in {}", side(*c))
                }
                Finding::ObjectField { name, field, a, b } => {
                    writeln!(s, "  object `{name}` {field}: {a} vs {b}")
                }
                Finding::MarkOnlyIn { side: c, kind, obj, t } => {
                    writeln!(s, "  mark {kind}{} at {t:.3}s only in {}", o(obj), side(*c))
                }
                Finding::MarkTime { kind, obj, n, a, b } => writeln!(
                    s,
                    "  mark {kind}{} #{n}: {a:.3}s vs {b:.3}s (delta {:+.3}s)",
                    o(obj),
                    b - a
                ),
                Finding::MarkField { kind, obj, n, field, a, b } => {
                    writeln!(s, "  mark {kind}{} #{n} {field}: {a} vs {b}", o(obj))
                }
                Finding::Series { obj, field, t, a, b } => {
                    writeln!(s, "  state obj={obj} {field} at {t:.3}s: {a:.3} vs {b:.3}")
                }
                Finding::Slope { obj, a, b } => {
                    writeln!(s, "  state obj={obj} special energy slope: {a:.3}/s vs {b:.3}/s")
                }
                Finding::SeriesOnlyIn { side: c, obj } => {
                    writeln!(s, "  state obj={obj} only in {}", side(*c))
                }
            };
        }
        for n in &self.notes {
            let _ = writeln!(s, "  note: {n}");
        }
        let _ = writeln!(
            s,
            "{} differences ({} objects, {} marks compared)",
            self.findings.len(),
            self.objects_compared,
            self.marks_compared
        );
        s
    }
}

fn f3(v: f64) -> String {
    format!("{v:.3}")
}

fn list<const N: usize>(v: [f64; N]) -> String {
    format!("[{}]", v.iter().map(|x| f3(*x)).collect::<Vec<_>>().join(", "))
}

fn object_diff(name: &str, a: &Object, b: &Object, tol: &Tolerances, out: &mut Vec<Finding>) {
    let mut add = |field, a: String, b: String| {
        out.push(Finding::ObjectField { name: name.to_owned(), field, a, b });
    };
    if let (Some(x), Some(y)) = (&a.odf, &b.odf)
        && !x.eq_ignore_ascii_case(y)
    {
        add("odf", x.clone(), y.clone());
    }
    if let (Some(x), Some(y)) = (a.pos, b.pos) {
        let d = ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt();
        if d > tol.pos {
            add("pos", list(x), format!("{} (distance {})", list(y), f3(d)));
        }
    }
    for (field, x, y) in [("hull", a.hull, b.hull), ("shields", a.shields, b.shields)] {
        if let (Some(x), Some(y)) = (x, y)
            && ((x[0] - y[0]).abs() > tol.hull || (x[1] - y[1]).abs() > tol.hull)
        {
            add(field, list(x), list(y));
        }
    }
    if let (Some(x), Some(y)) = (a.dying, b.dying)
        && x != y
    {
        add("dying", x.to_string(), y.to_string());
    }
}

/// Fields compared when both marks have them, by kind.
fn compared_fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "fire.start" | "fire.end" => &["weapon"],
        "destroyed" => &["by"],
        "ord.new" | "ord.gone" => &["weapon"],
        _ => &[],
    }
}

type Key = (String, Option<String>);

/// Fields compared as numbers within a tolerance, by kind.
fn numeric_fields(kind: &str, tol: &Tolerances) -> &'static [(&'static str, ToleranceOf)] {
    let _ = tol;
    match kind {
        "bank" => &[
            ("dilithium", ToleranceOf::Hull),
            ("latinum", ToleranceOf::Hull),
            ("metal", ToleranceOf::Hull),
            ("biomatter", ToleranceOf::Hull),
            ("crew", ToleranceOf::Hull),
            ("officers", ToleranceOf::Hull),
        ],
        "ord.gone" => &[("d", ToleranceOf::Pos), ("life", ToleranceOf::Time)],
        _ => &[],
    }
}

#[derive(Clone, Copy)]
enum ToleranceOf {
    Time,
    Pos,
    Hull,
}

impl ToleranceOf {
    fn of(self, t: &Tolerances) -> f64 {
        match self {
            ToleranceOf::Time => t.time,
            ToleranceOf::Pos => t.pos,
            ToleranceOf::Hull => t.hull,
        }
    }
}

fn group<'a>(r: &'a Run, skipped: &mut BTreeMap<String, usize>) -> BTreeMap<Key, Vec<&'a PMark>> {
    let mut g: BTreeMap<Key, Vec<&PMark>> = BTreeMap::new();
    for m in &r.marks {
        if m.kind == "state" {
            continue;
        }
        if matches!(support(&m.kind), Both | Support::Unknown) {
            g.entry((m.kind.clone(), m.obj.clone())).or_default().push(m);
        } else {
            *skipped.entry(m.kind.clone()).or_default() += 1;
        }
    }
    g
}

/// Compare two runs. Identical runs give an empty diff.
pub fn diff(a: &Run, b: &Run, tol: &Tolerances) -> Diff {
    let mut d = Diff::default();
    // Objects.
    for (n, oa) in &a.objects {
        match b.objects.get(n) {
            None => d.findings.push(Finding::ObjectOnlyIn { side: 'A', name: n.clone() }),
            Some(ob) => {
                d.objects_compared += 1;
                object_diff(n, oa, ob, tol, &mut d.findings);
            }
        }
    }
    for n in b.objects.keys().filter(|n| !a.objects.contains_key(*n)) {
        d.findings.push(Finding::ObjectOnlyIn { side: 'B', name: n.clone() });
    }
    if let (Some(x), Some(y)) = (a.time, b.time)
        && (x - y).abs() > tol.time
    {
        d.notes.push(format!(
            "objects are snapshots at {x:.3}s (A) and {y:.3}s (B): positions and hull are not like for like"
        ));
    }
    for (s, r) in [('A', a), ('B', b)] {
        if r.unnamed > 0 {
            d.notes.push(format!("{} unnamed objects in {s} not compared", r.unnamed));
        }
    }
    // Marks: group by (kind, obj), pair in order.
    let (mut sa, mut sb) = (BTreeMap::new(), BTreeMap::new());
    let (ga, gb) = (group(a, &mut sa), group(b, &mut sb));
    let mut keys: Vec<&Key> = ga.keys().chain(gb.keys()).collect();
    keys.sort();
    keys.dedup();
    let empty = Vec::new();
    for key in keys {
        let (va, vb) = (ga.get(key).unwrap_or(&empty), gb.get(key).unwrap_or(&empty));
        let (kind, obj) = key;
        for (i, (x, y)) in va.iter().zip(vb.iter()).enumerate() {
            d.marks_compared += 1;
            if (x.t - y.t).abs() > tol.time {
                d.findings.push(Finding::MarkTime {
                    kind: kind.clone(),
                    obj: obj.clone(),
                    n: i + 1,
                    a: x.t,
                    b: y.t,
                });
            }
            for (f, which) in numeric_fields(kind, tol) {
                let num = |m: &PMark| m.fields.get(*f).and_then(|v| v.parse::<f64>().ok());
                if let (Some(p), Some(q)) = (num(x), num(y))
                    && (p - q).abs() > which.of(tol)
                {
                    d.findings.push(Finding::MarkField {
                        kind: kind.clone(),
                        obj: obj.clone(),
                        n: i + 1,
                        field: f,
                        a: p.to_string(),
                        b: q.to_string(),
                    });
                }
            }
            for f in compared_fields(kind) {
                if let (Some(p), Some(q)) = (x.fields.get(*f), y.fields.get(*f))
                    && !p.eq_ignore_ascii_case(q)
                {
                    d.findings.push(Finding::MarkField {
                        kind: kind.clone(),
                        obj: obj.clone(),
                        n: i + 1,
                        field: f,
                        a: p.clone(),
                        b: q.clone(),
                    });
                }
            }
        }
        for (side, v, other) in [('A', va, vb.len()), ('B', vb, va.len())] {
            for m in v.iter().skip(other) {
                d.findings.push(Finding::MarkOnlyIn {
                    side,
                    kind: kind.clone(),
                    obj: obj.clone(),
                    t: m.t,
                });
            }
        }
    }
    series(a, b, tol, &mut d);
    let unknown: Vec<String> = {
        let mut u: Vec<String> = a
            .marks
            .iter()
            .chain(&b.marks)
            .filter(|m| support(&m.kind) == Support::Unknown)
            .map(|m| m.kind.clone())
            .collect();
        u.sort();
        u.dedup();
        u
    };
    if !unknown.is_empty() {
        d.notes.push(format!(
            "kinds not in the name table, compared by kind and object: {}",
            unknown.join(", ")
        ));
    }
    for (s, sk) in [('A', &sa), ('B', &sb)] {
        if !sk.is_empty() {
            let l = sk.iter().map(|(k, n)| format!("{k} x{n}")).collect::<Vec<_>>().join(", ");
            d.notes.push(format!("marks in {s} with no counterpart in the other engine, skipped: {l}"));
        }
    }
    d
}

/// One object's `state` samples: (t, pos, hull, sh, sp).
type StateRow = (f64, Option<[f64; 3]>, Option<f64>, Option<f64>, Option<f64>);

fn states_of(r: &Run) -> BTreeMap<String, Vec<StateRow>> {
    let mut m: BTreeMap<String, Vec<StateRow>> = BTreeMap::new();
    let num = |p: &PMark, k: &str| p.fields.get(k).and_then(|v| v.parse::<f64>().ok());
    for p in r.marks.iter().filter(|p| p.kind == "state") {
        let Some(o) = &p.obj else { continue };
        let pos = p.fields.get("pos").and_then(|v| {
            let x: Vec<f64> = v.split(',').filter_map(|c| c.parse().ok()).collect();
            <[f64; 3]>::try_from(x).ok()
        });
        m.entry(o.clone()).or_default().push((p.t, pos, num(p, "hull"), num(p, "sh"), num(p, "sp")));
    }
    m
}

/// Linear between samples; `None` outside the series.
fn interp(s: &[StateRow], t: f64, get: impl Fn(&StateRow) -> Option<[f64; 3]>) -> Option<[f64; 3]> {
    let i = s.partition_point(|x| x.0 < t);
    if i < s.len() && (s[i].0 - t).abs() < 1e-9 {
        return get(&s[i]);
    }
    if i == 0 || i == s.len() {
        return None;
    }
    let (x, y) = (&s[i - 1], &s[i]);
    let (p, q) = (get(x)?, get(y)?);
    let w = if y.0 > x.0 { (t - x.0) / (y.0 - x.0) } else { 0.0 };
    Some([0, 1, 2].map(|k| p[k] + (q[k] - p[k]) * w))
}

/// Least-squares slope of `sp` over time.
fn slope(s: &[StateRow]) -> Option<f64> {
    let v: Vec<(f64, f64)> = s.iter().filter_map(|x| Some((x.0, x.4?))).collect();
    if v.len() < 2 {
        return None;
    }
    let n = v.len() as f64;
    let (mt, ms) = (v.iter().map(|x| x.0).sum::<f64>() / n, v.iter().map(|x| x.1).sum::<f64>() / n);
    let den: f64 = v.iter().map(|x| (x.0 - mt).powi(2)).sum();
    (den > 0.0).then(|| v.iter().map(|x| (x.0 - mt) * (x.1 - ms)).sum::<f64>() / den)
}

/// `state` series of both runs, B resampled on A's sample times.
fn series(a: &Run, b: &Run, tol: &Tolerances, d: &mut Diff) {
    let (sa, sb) = (states_of(a), states_of(b));
    for (o, xa) in &sa {
        let Some(xb) = sb.get(o) else {
            d.findings.push(Finding::SeriesOnlyIn { side: 'A', obj: o.clone() });
            continue;
        };
        let one = |f: fn(&StateRow) -> Option<f64>| move |r: &StateRow| f(r).map(|v| [v, 0.0, 0.0]);
        type Check = (&'static str, fn(&StateRow) -> Option<f64>, f64);
        let checks: [Check; 2] =
            [("hull", |r| r.2, tol.hull), ("shields", |r| r.3, tol.hull)];
        let mut pos_done = false;
        let mut done = [false; 2];
        for r in xa {
            d.marks_compared += 1;
            if !pos_done
                && let (Some(p), Some(q)) = (r.1, interp(xb, r.0, |x| x.1))
            {
                let dist = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
                if dist > tol.pos {
                    d.findings.push(Finding::Series { obj: o.clone(), field: "pos", t: r.0, a: 0.0, b: dist });
                    pos_done = true;
                }
            }
            for (i, (field, get, lim)) in checks.iter().enumerate() {
                if done[i] {
                    continue;
                }
                if let (Some(p), Some(q)) = (get(r), interp(xb, r.0, one(*get)).map(|v| v[0]))
                    && (p - q).abs() > *lim
                {
                    d.findings.push(Finding::Series { obj: o.clone(), field, t: r.0, a: p, b: q });
                    done[i] = true;
                }
            }
        }
        if let (Some(p), Some(q)) = (slope(xa), slope(xb))
            && (p - q).abs() > tol.slope
        {
            d.findings.push(Finding::Slope { obj: o.clone(), a: p, b: q });
        }
    }
    for o in sb.keys().filter(|o| !sa.contains_key(*o)) {
        d.findings.push(Finding::SeriesOnlyIn { side: 'B', obj: o.clone() });
    }
}

/// The mark-name table as text, for `sta-bench parity --names`.
pub fn names_table() -> String {
    let mut s = String::from("Refit kind        ours               support     note\n");
    for r in NAME_MAP {
        let ours = match r.ours_when {
            Some((k, v)) => format!("{} {k}={v}", r.ours),
            None if r.ours.is_empty() => "-".into(),
            None => r.ours.into(),
        };
        let sup = match r.support {
            Both => "both",
            RefitOnly => "refit only",
            OursOnly => "ours only",
            Support::Unknown => "unknown",
        };
        let _ = writeln!(s, "{:<17} {ours:<18} {sup:<11} {}", r.refit, r.note);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refit() -> Run {
        let mut r = Run::default();
        read_objects(
            "{\"header\":true,\"count\":2,\"tick\":431,\"time\":21.6,\"player_team\":1,\"build\":\"x\"}\n\
             {\"id\":57,\"name\":\"Warbird\",\"odf\":\"rwarbird\",\"team\":1,\"pos\":[1.0,0.0,2.0],\"hull\":[10.0,20.0],\"shields\":null,\"dying\":false}\n\
             {\"id\":58,\"name\":null,\"label\":null,\"odf\":\"fgalaxy\"}\n",
            Source::Refit,
            &mut r,
        );
        read_refit_marks(
            "epoch 0x01dd3f6a 0x8e0c1b40 1000\n1000 ready objects=2\n2500 cloak.start obj=warbird\n5500 cloak.done obj=warbird\n3000 cmd text=cloak warbird on\n",
            &mut r,
        );
        r
    }

    #[test]
    fn refit_reader_normalises() {
        let r = refit();
        assert_eq!(r.time, Some(21.6));
        assert_eq!(r.unnamed, 1);
        assert_eq!(r.objects["warbird"].hull, Some([10.0, 20.0]));
        assert_eq!(r.objects["warbird"].shields, None);
        let m: Vec<_> = r.marks.iter().map(|m| (m.kind.as_str(), m.t)).collect();
        assert_eq!(m, [("ready", 0.0), ("cloak.start", 1.5), ("cmd", 2.0), ("cloak.done", 4.5)]);
        // `text` runs to the end of the line.
        assert_eq!(r.marks[2].fields["text"], "cloak warbird on");
    }

    #[test]
    fn our_reader_maps_names_and_ticks() {
        let mut r = Run::default();
        read_our_marks(
            "    45 cloak obj=Warbird on=1\n    60 death obj=bop by=warbird\n   300 cloak obj=warbird on=0\n    10 hit obj=bop\n",
            &mut r,
        );
        let m: Vec<_> = r
            .marks
            .iter()
            .map(|m| (m.kind.as_str(), m.t, m.obj.clone().unwrap()))
            .collect();
        assert_eq!(
            m,
            [
                ("hit", 10.0 / 30.0, "bop".to_string()),
                ("cloak.start", 1.5, "warbird".to_string()),
                ("destroyed", 2.0, "bop".to_string()),
                ("decloak.start", 10.0, "warbird".to_string()),
            ]
        );
    }

    #[test]
    fn identical_runs_diff_empty() {
        let r = refit();
        let d = diff(&r, &r.clone(), &Tolerances::default());
        assert!(d.is_empty(), "{}", d.report("a", "b"));
        assert!(d.marks_compared > 0 && d.objects_compared > 0);
    }

    #[test]
    fn differences_are_found_within_tolerance() {
        let (a, mut b) = (refit(), refit());
        b.marks[1].t += 0.05; // inside 0.1
        assert!(diff(&a, &b, &Tolerances::default()).is_empty());
        b.marks[1].t += 0.5;
        b.objects.get_mut("warbird").unwrap().pos = Some([5.0, 0.0, 2.0]);
        b.objects.insert("extra".into(), Object::default());
        let d = diff(&a, &b, &Tolerances::default());
        assert_eq!(d.findings.len(), 3, "{}", d.report("a", "b"));
        let loose = Tolerances::parse("time=1,pos=10").unwrap();
        assert_eq!(
            diff(&a, &b, &loose).findings,
            [Finding::ObjectOnlyIn { side: 'B', name: "extra".into() }]
        );
    }

    #[test]
    fn missing_marks_and_unshared_kinds() {
        let (a, mut b) = (refit(), refit());
        b.marks.retain(|m| m.kind != "cloak.start");
        let d = diff(&a, &b, &Tolerances::default());
        assert!(matches!(d.findings.as_slice(), [Finding::MarkOnlyIn { side: 'A', .. }]));
        assert!(d.notes.iter().any(|n| n.contains("cloak.done") && n.contains("ready")));
    }

    #[test]
    fn sim_time_marks_use_t_not_the_wall_clock() {
        // Roots ran 1.7x faster than the wall clock here: wall says 2.04 s, sim 3.47 s.
        let mut r = Run::default();
        read_refit_marks(
            "epoch 0x01dd72e4 0x41d2ebeb 17629000\n17629080 ready t=1.297 objects=9\n\
             17631119 cmd t=4.767 text=select I0\n17631200 state t=4.800 obj=I0 pos=1,0,2 hull=10 sh=5 sp=0\n",
            &mut r,
        );
        let m: Vec<_> = r.marks.iter().map(|m| (m.kind.as_str(), (m.t * 1000.0).round())).collect();
        assert_eq!(m, [("ready", 0.0), ("cmd", 3470.0), ("state", 3503.0)]);
        assert_eq!(support("state"), Both);
        // `fire.end` is dated back in the wall column; its `t=` is FireGap later.
        let mut r = Run::default();
        read_refit_marks(
            "1000 ready t=1.000 objects=1\n2000 state t=3.000 obj=a\n3000 state t=5.000 obj=a\n\
             2500 fire.end t=6.000 obj=a weapon=w\n4000 state t=7.000 obj=a\n",
            &mut r,
        );
        let fe = r.marks.iter().find(|m| m.kind == "fire.end").unwrap();
        assert!((fe.t - 3.0).abs() < 1e-9, "{}", fe.t);
        assert_eq!(support("ord.new"), Both);
        assert_eq!(support("frobnicate"), Support::Unknown);
    }

    /// A synthetic run with every measuring kind (TOOL-16), in Refit's format
    /// with `t=`; `shift` moves `a` after 2 s, `drain` changes its energy slope,
    /// `gone` keeps or drops the `ord.gone`.
    fn measuring(shift: f64, drain: f64, gone: bool) -> Run {
        let mut t = String::from("1000 ready t=0.500 objects=2\n");
        for i in 0..=20 {
            let s = f64::from(i) * 0.25;
            let z = 2000.0 + if s > 2.0 { shift } else { 0.0 };
            let sp = 100.0 - drain * s;
            t += &format!("{} state t={:.3} obj=a pos=2000.0,0.0,{z:.1} hull=500.0 sh=400.0 sp={sp:.1}\n", 1000 + i * 250, s + 0.5);
        }
        t += "1600 ord.new t=1.100 id=1 obj=a weapon=photono target=b at=2000.0,0.0,2000.0\n";
        if gone {
            t += "2100 ord.gone t=1.600 id=1 obj=a weapon=photono at=2000.0,0.0,2290.0 life=0.500 tgt_at=2000.0,0.0,2300.0 d=10.0\n";
        }
        t += "2600 bank t=2.100 team=1 dilithium=3500.0 latinum=1000.0 metal=0.0 biomatter=0.0 crew=750.0\n";
        t += "2700 wobble t=2.200 obj=a\n";
        let mut r = Run::default();
        read_refit_marks(&t, &mut r);
        r
    }

    #[test]
    fn measuring_kinds_diff() {
        let tol = Tolerances::parse("time=0.1,pos=5,hull=1,slope=0.5").unwrap();
        let base = measuring(0.0, 2.0, true);
        let same = diff(&base, &base.clone(), &tol);
        assert!(same.is_empty(), "{}", same.report("a", "b"));
        assert!(same.marks_compared > 20);
        // Unknown kinds are named, not dropped.
        assert!(same.notes.iter().any(|n| n.contains("not in the name table") && n.contains("wobble")), "{:?}", same.notes);
        // Within tolerance: a 3-unit move, a 0.3/s slope change.
        assert!(diff(&base, &measuring(3.0, 2.3, true), &tol).is_empty());
        // Beyond: a moved position, a changed energy slope, a missing ord.gone.
        let d = diff(&base, &measuring(40.0, 4.0, false), &tol);
        let r = d.report("a", "b");
        assert!(d.findings.iter().any(|f| matches!(f, Finding::Series { field: "pos", t, .. } if (*t - 2.25).abs() < 1e-6)), "{r}");
        assert!(d.findings.iter().any(|f| matches!(f, Finding::Slope { a, b, .. } if (a + 2.0).abs() < 1e-6 && (b + 4.0).abs() < 1e-6)), "{r}");
        assert!(d.findings.iter().any(|f| matches!(f, Finding::MarkOnlyIn { side: 'A', kind, .. } if kind == "ord.gone")), "{r}");
        assert_eq!(d.findings.len(), 3, "{r}");
        // A bank that differs beyond the hull tolerance.
        let mut b = base.clone();
        b.marks.iter_mut().find(|m| m.kind == "bank").unwrap().fields.insert("dilithium".into(), "3400.0".into());
        assert!(matches!(diff(&base, &b, &tol).findings.as_slice(), [Finding::MarkField { field: "dilithium", .. }]));
    }

    #[test]
    fn tolerance_parse() {
        assert!(Tolerances::parse("time=x").is_err());
        assert!(Tolerances::parse("speed=1").is_err());
        assert_eq!(Tolerances::parse("").unwrap(), Tolerances::default());
    }
}
