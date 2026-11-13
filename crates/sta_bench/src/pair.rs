//! Paired scenes: one scene description, run on the original game (Refit's
//! bench) and in STA64, compared on one sim-seconds axis (TOOL-15;
//! docs/TESTBENCH.md, "Paired scenes").
//!
//! A pair file (`bench/pairs/<name>.ini`) is a scene file in Refit's format
//! with a `[Pair]` section (which Refit's `Scene.asi` never reads) and keys
//! prefixed by a game (`roots.Odf=`, `a2.Race=`, `fake.Odf=`): for the game
//! being run the prefixed key replaces the plain one in its section, other
//! games' prefixed keys are dropped ([`materialize`]). The same text, so
//! materialised, goes to both sides.
//!
//! The two sides leave one directory each, in the same file formats:
//!
//! - `refit/` (by `tools/pair/refit_side.py`): `refit.json` (when each command
//!   landed and each frame was taken, in sim seconds after the `ready` mark),
//!   `Scene.marks` with `t=` on every mark (`[Marks] SimTime=1`), `state`
//!   lines every `State=` ms, `Scene.objects.jsonl`, `frames/f<i>.png`.
//! - `sta64/` ([`run_sta64`]): the same `Scene.marks` format (first column
//!   the tick), `state` lines every tick, `run.json`, `frames/tick-N.png`.
//!
//! [`compare`] reads the two directories and nothing else, so a report is
//! reproducible from them: events paired by kind, object and weapon (Refit's
//! vocabulary, [`crate::parity::NAME_MAP`]), `state` series resampled on
//! Refit's sample times, motion figures (start, top speed, arrival), ordnance
//! counts, and frames side by side with a difference image.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_core::{TICK_HZ, Tick};

use crate::pair_metrics::{self, MetricTol, frame_metrics};
use crate::parity::{self, PMark, Run, Support};
use crate::parse::{SceneFile, parse_command};
use crate::run::{Bench, RunConfig, json, to_f64};

/// Games the original-game bench runs. Armada I is not one: Refit's bench
/// has no Armada I profile (PLAN-ARMADA1), so A1 keeps install-reading checks.
pub const GAMES: &[&str] = &["a2", "roots"];

// ---- a scene file as lines ---------------------------------------------

#[derive(Debug, Clone)]
enum Line {
    /// A comment, a blank line, or anything that is not `key=value`.
    Other(String),
    Kv(String, String),
}

#[derive(Debug, Clone)]
struct Section {
    /// `None` for the lines before the first header.
    name: Option<String>,
    lines: Vec<Line>,
}

fn sections(text: &str) -> Vec<Section> {
    let mut out = vec![Section { name: None, lines: Vec::new() }];
    for raw in text.lines() {
        let l = raw.trim();
        if let Some(n) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            out.push(Section { name: Some(n.trim().to_owned()), lines: Vec::new() });
            continue;
        }
        let line = match l.split_once('=') {
            Some((k, v)) if !l.starts_with([';', '#']) && !k.trim().is_empty() => {
                Line::Kv(k.trim().to_owned(), v.trim().to_owned())
            }
            _ => Line::Other(raw.trim_end().to_owned()),
        };
        out.last_mut().expect("one section").lines.push(line);
    }
    out
}

fn is_pair(s: &Section) -> bool {
    s.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case("pair"))
}

/// The scene text one side runs: `[Pair]` removed, and in every section a
/// key prefixed by one of `games` (`roots.Odf=`) put in place of the plain
/// one (a later game in the list wins), other prefixed keys dropped.
pub fn materialize(text: &str, games: &[&str]) -> String {
    let mut out = String::new();
    for s in sections(text).iter().filter(|s| !is_pair(s)) {
        // key (lower case) -> (value, rank of the game)
        let mut over: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for l in &s.lines {
            if let Line::Kv(k, v) = l
                && let Some((g, key)) = k.split_once('.')
                && let Some(rank) = games.iter().position(|x| x.eq_ignore_ascii_case(g))
                && over.get(&key.to_ascii_lowercase()).is_none_or(|o| o.1 <= rank)
            {
                over.insert(key.to_ascii_lowercase(), (v.clone(), rank));
            }
        }
        if let Some(n) = &s.name {
            let _ = writeln!(out, "[{n}]");
        }
        let mut placed: Vec<String> = Vec::new();
        for l in &s.lines {
            match l {
                Line::Other(t) => {
                    let _ = writeln!(out, "{t}");
                }
                Line::Kv(k, _) if k.contains('.') => {}
                Line::Kv(k, v) => {
                    let lk = k.to_ascii_lowercase();
                    match over.get(&lk) {
                        Some((ov, _)) if !placed.contains(&lk) => {
                            let _ = writeln!(out, "{k}={ov}");
                            placed.push(lk);
                        }
                        Some(_) => {}
                        None => {
                            let _ = writeln!(out, "{k}={v}");
                        }
                    }
                }
            }
        }
        for (k, (v, _)) in over.iter().filter(|(k, _)| !placed.contains(k)) {
            // The plain key was absent: written in the section's own spelling style.
            let mut c = k.chars();
            let key = c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect());
            let _ = writeln!(out, "{key}={v}");
        }
    }
    out
}

// ---- the description ---------------------------------------------------

/// How close counts as the same in a paired report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairTol {
    /// Seconds (events, motion start and arrival).
    pub time: f64,
    /// World units (position along the `state` series).
    pub pos: f64,
    /// Hull, shield and special-energy points.
    pub hull: f64,
    /// Mean absolute difference (0..255) a frame pair may have; `None`: frames
    /// are reported, not judged (the default while STA64 draws stand-ins).
    pub frame: Option<f64>,
    /// The frame metrics robust to a small misalignment, each opt-in
    /// ([`MetricTol`]).
    pub metrics: MetricTol,
}

impl Default for PairTol {
    fn default() -> Self {
        Self { time: 0.25, pos: 5.0, hull: 1.0, frame: None, metrics: MetricTol::default() }
    }
}

impl PairTol {
    /// `time=0.25,pos=5,hull=1,frame=8`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut t = Self::default();
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (k, v) = part.split_once('=').ok_or_else(|| format!("`{part}`: expected key=value"))?;
            let v: f64 = v.trim().parse().map_err(|_| format!("`{part}`: not a number"))?;
            match k.trim() {
                "time" => t.time = v,
                "pos" => t.pos = v,
                "hull" => t.hull = v,
                "frame" => t.frame = Some(v),
                k if t.metrics.set(k, v) => {}
                k => return Err(format!("unknown tolerance `{k}` (time, pos, hull, frame, {})", MetricTol::KEYS)),
            }
        }
        Ok(t)
    }

    fn text(&self) -> String {
        let mut s = format!("time={},pos={},hull={}", self.time, self.pos, self.hull);
        if let Some(f) = self.frame {
            let _ = write!(s, ",frame={f}");
        }
        s += &self.metrics.text();
        s
    }
}

/// A pair file, read for one game.
#[derive(Debug, Clone, PartialEq)]
pub struct PairDesc {
    pub name: String,
    pub game: String,
    /// `Games=`: the games the description is written for.
    pub games: Vec<String>,
    /// `Duration=`: sim seconds after the scene is built.
    pub duration: f64,
    /// `Frames=`: sim seconds to take a frame at.
    pub frames: Vec<f64>,
    /// `Size=WxH`, default 1280x720.
    pub size: (u32, u32),
    pub tol: PairTol,
    /// `At=<s> <command>`, in file order.
    pub at: Vec<(f64, String)>,
    /// `Mod=`: the mod STA64 mounts for this game (Refit's launch names it too).
    pub mod_name: Option<String>,
    /// The map the original loads and STA64 loads with it: `Map=` of `[Pair]`, else the
    /// map `Launch=` names (`-nointro a2_borg01`, `... fulltech.bzn`), lower case, no
    /// extension. `None`: STA64's empty map (a unit-level scene).
    pub map: Option<String>,
    /// The pair file as written.
    pub text: String,
}

fn seconds(v: &str) -> Option<f64> {
    let v = v.trim();
    v.strip_suffix('s').unwrap_or(v).trim().parse().ok().filter(|x: &f64| x.is_finite() && *x >= 0.0)
}

impl PairDesc {
    /// Read the `[Pair]` section of `text` for `game` (`<game>.<Key>` wins).
    pub fn parse(name: &str, text: &str, game: &str) -> Result<Self, String> {
        let game = game.to_ascii_lowercase();
        if game == "a1" || game == "armada1" {
            return Err("Armada I is not supported: Refit's bench has no Armada I profile (docs/TESTBENCH.md, \"Paired scenes\")".into());
        }
        let secs = sections(text);
        let pair = secs.iter().find(|s| is_pair(s)).ok_or("no [Pair] section")?;
        let mut keys: Vec<(String, String)> = Vec::new();
        let mut over: BTreeMap<String, String> = BTreeMap::new();
        for l in &pair.lines {
            if let Line::Kv(k, v) = l {
                match k.split_once('.') {
                    Some((g, key)) if g.eq_ignore_ascii_case(&game) => {
                        over.insert(key.to_ascii_lowercase(), v.clone());
                    }
                    Some(_) => {}
                    None => keys.push((k.to_ascii_lowercase(), v.clone())),
                }
            }
        }
        let one = |k: &str| over.get(k).cloned().or_else(|| keys.iter().find(|x| x.0 == k).map(|x| x.1.clone()));
        let games: Vec<String> = one("games")
            .ok_or("[Pair] needs Games= (a2, roots)")?
            .split(',')
            .map(|g| g.trim().to_ascii_lowercase())
            .filter(|g| !g.is_empty())
            .collect();
        if let Some(bad) = games.iter().find(|g| !GAMES.contains(&g.as_str())) {
            return Err(format!("Games={bad}: the bench runs {}", GAMES.join(", ")));
        }
        if !games.contains(&game) {
            return Err(format!("this pair is written for {}, not {game}", games.join(", ")));
        }
        let duration = one("duration").and_then(|v| seconds(&v)).ok_or("[Pair] needs Duration=<sim seconds>")?;
        let frames = match one("frames") {
            Some(v) => v
                .split(',')
                .filter(|x| !x.trim().is_empty())
                .map(|x| seconds(x).ok_or_else(|| format!("Frames: `{x}` is not seconds")))
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };
        if let Some(f) = frames.iter().find(|f| **f > duration) {
            return Err(format!("Frames: {f} s is after Duration={duration}"));
        }
        let size = match one("size") {
            Some(v) => v
                .split_once('x')
                .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
                .ok_or_else(|| format!("Size={v}: expected WxH"))?,
            None => (1280, 720),
        };
        let tol = match one("tol") {
            Some(v) => PairTol::parse(&v).map_err(|e| format!("Tol: {e}"))?,
            None => PairTol::default(),
        };
        let mut at = Vec::new();
        for (k, v) in keys.iter().filter(|k| k.0 == "at") {
            let _ = k;
            let (t, cmd) = v.split_once(char::is_whitespace).ok_or_else(|| format!("At={v}: expected `<seconds> <command>`"))?;
            let t = seconds(t).ok_or_else(|| format!("At={v}: `{t}` is not seconds"))?;
            parse_command(cmd.trim()).map_err(|e| format!("At={v}: {e}"))?;
            at.push((t, cmd.trim().to_owned()));
        }
        Ok(PairDesc {
            name: name.to_owned(),
            game,
            games,
            duration,
            frames,
            size,
            tol,
            at,
            mod_name: one("mod").filter(|m| !m.is_empty()),
            map: one("map").filter(|m| !m.is_empty()).map(|m| map_stem(&m)).or_else(|| one("launch").and_then(|l| launch_map(&l))),
            text: text.to_owned(),
        })
    }

    pub fn load(path: &Path, game: &str) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path.file_stem().map_or_else(|| "pair".into(), |s| s.to_string_lossy().into_owned());
        Self::parse(&name, &text, game).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The scene STA64 runs: this game's keys, then `extra` (`fake` for the
    /// bench's fake content) on top.
    ///
    /// With `map` in `extra`, a pair that names a map (`PairDesc::map`) gets `Map=` and
    /// `MapObjects=keep` in `[Scene]`: the original always has the launch map's own
    /// objects, so the frames STA64 draws have them too. Keys the file sets itself stay.
    /// The measuring run (marks, states) leaves `map` out: the map's own units fight and
    /// would change what a scene measures, and the original's marks name only the scene's
    /// objects.
    pub fn scene_text(&self, extra: &[&str]) -> String {
        let mut g = vec![self.game.as_str()];
        g.extend_from_slice(extra);
        let text = materialize(&self.text, &g);
        match &self.map {
            Some(m) if extra.contains(&"map") => with_map(&text, m),
            _ => text,
        }
    }
}

/// `a2_borg01.bzn` -> `a2_borg01`.
fn map_stem(s: &str) -> String {
    let s = s.trim().trim_matches('"');
    let l = s.to_ascii_lowercase();
    l.strip_suffix(".bzn").or_else(|| l.strip_suffix(".sol")).unwrap_or(&l).to_owned()
}

/// The map a game command line names: its last argument that is neither a
/// flag nor a flag's value (`-mod "STA2 Classic Mod"`).
pub fn launch_map(launch: &str) -> Option<String> {
    let mut args: Vec<String> = Vec::new();
    let (mut cur, mut quoted, mut any) = (String::new(), false, false);
    for c in launch.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        args.push(cur);
    }
    let mut map = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if matches!(a.to_ascii_lowercase().as_str(), "-mod" | "--mod") {
            it.next();
        } else if !a.is_empty() && !a.starts_with('-') && !a.chars().all(|c| c.is_ascii_digit()) {
            // A bare number is a flag's value (`-res 1280`), not a map.
            map = Some(map_stem(&a));
        }
    }
    map
}

/// `text` with `Map=<map>` and `MapObjects=keep` added to `[Scene]` unless it sets them.
fn with_map(text: &str, map: &str) -> String {
    let secs = sections(text);
    let scene = secs.iter().find(|s| s.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case("scene")));
    let has = |k: &str| scene.is_some_and(|s| s.lines.iter().any(|l| matches!(l, Line::Kv(x, _) if x.eq_ignore_ascii_case(k))));
    let mut add = String::new();
    if !has("map") {
        let _ = writeln!(add, "Map={map}");
    }
    if !has("mapobjects") {
        add += "MapObjects=keep\n";
    }
    if add.is_empty() {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut done = false;
    for line in text.lines() {
        out += line;
        out.push('\n');
        if !done && line.trim().eq_ignore_ascii_case("[scene]") {
            out += &add;
            done = true;
        }
    }
    if !done {
        out = format!("[Scene]\n{add}{out}");
    }
    out
}

// ---- a small JSON reader (refit.json, run.json) -------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Jv {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Jv>),
    Obj(Vec<(String, Jv)>),
}

impl Jv {
    pub fn get(&self, k: &str) -> &Jv {
        match self {
            Jv::Obj(m) => m.iter().find(|x| x.0 == k).map_or(&Jv::Null, |x| &x.1),
            _ => &Jv::Null,
        }
    }
    pub fn num(&self) -> Option<f64> {
        if let Jv::Num(n) = self { Some(*n) } else { None }
    }
    pub fn str(&self) -> Option<&str> {
        if let Jv::Str(s) = self { Some(s) } else { None }
    }
    pub fn arr(&self) -> &[Jv] {
        if let Jv::Arr(a) = self { a } else { &[] }
    }
}

struct Jp<'a> {
    s: &'a [u8],
    i: usize,
}

impl Jp<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn string(&mut self) -> Option<String> {
        if self.peek()? != b'"' {
            return None;
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return Some(out),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let h = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            self.i += 4;
                            out.push(char::from_u32(u32::from_str_radix(h, 16).ok()?).unwrap_or('\u{fffd}'));
                        }
                        o => out.push(o as char),
                    }
                }
                _ => {
                    // Copy a run of UTF-8 bytes as they are.
                    let st = self.i - 1;
                    while self.s.get(self.i).is_some_and(|b| *b != b'"' && *b != b'\\') {
                        self.i += 1;
                    }
                    out.push_str(std::str::from_utf8(&self.s[st..self.i]).ok()?);
                }
            }
        }
    }
    fn value(&mut self) -> Option<Jv> {
        match self.peek()? {
            b'"' => self.string().map(Jv::Str),
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                if self.peek()? == b']' {
                    self.i += 1;
                    return Some(Jv::Arr(v));
                }
                loop {
                    v.push(self.value()?);
                    match self.peek()? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(Jv::Arr(v));
                        }
                        _ => return None,
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut m = Vec::new();
                if self.peek()? == b'}' {
                    self.i += 1;
                    return Some(Jv::Obj(m));
                }
                loop {
                    let k = self.string()?;
                    if self.peek()? != b':' {
                        return None;
                    }
                    self.i += 1;
                    m.push((k, self.value()?));
                    match self.peek()? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(Jv::Obj(m));
                        }
                        _ => return None,
                    }
                }
            }
            _ => {
                let st = self.i;
                while self.s.get(self.i).is_some_and(|c| !matches!(c, b',' | b'}' | b']') && !c.is_ascii_whitespace()) {
                    self.i += 1;
                }
                match std::str::from_utf8(&self.s[st..self.i]).ok()? {
                    "null" => Some(Jv::Null),
                    "true" => Some(Jv::Bool(true)),
                    "false" => Some(Jv::Bool(false)),
                    n => n.parse().ok().map(Jv::Num),
                }
            }
        }
    }
}

pub fn parse_json(text: &str) -> Option<Jv> {
    let mut p = Jp { s: text.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    (p.i == p.s.len()).then_some(v)
}

// ---- traces ------------------------------------------------------------

/// One `state` line: where a craft was and what it had, at sim time `t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub t: f64,
    pub pos: Option<[f64; 3]>,
    pub hull: Option<f64>,
    pub sh: Option<f64>,
    pub sp: Option<f64>,
}

/// A `Scene.marks` file of either side: `state` series by object, the rest
/// as events, all in sim seconds after `ready`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trace {
    pub states: BTreeMap<String, Vec<Sample>>,
    pub events: Vec<PMark>,
}

fn f(m: &PMark, k: &str) -> Option<f64> {
    m.fields.get(k).and_then(|v| v.parse().ok())
}

pub fn read_trace(text: &str) -> Trace {
    let mut run = Run::default();
    parity::read_refit_marks(text, &mut run);
    let mut tr = Trace::default();
    for m in run.marks {
        if m.kind == "state" {
            let Some(obj) = m.obj.clone() else { continue };
            let pos = m.fields.get("pos").and_then(|p| {
                let v: Vec<f64> = p.split(',').filter_map(|x| x.parse().ok()).collect();
                <[f64; 3]>::try_from(v).ok()
            });
            let s = Sample { t: m.t, pos, hull: f(&m, "hull"), sh: f(&m, "sh"), sp: f(&m, "sp") };
            tr.states.entry(obj).or_default().push(s);
        } else {
            tr.events.push(m);
        }
    }
    tr
}

/// The series' value at `t`, linear between samples; `None` outside it.
fn at_time(s: &[Sample], t: f64, get: impl Fn(&Sample) -> Option<[f64; 3]>) -> Option<[f64; 3]> {
    let i = s.partition_point(|x| x.t < t);
    if i < s.len() && (s[i].t - t).abs() < 1e-9 {
        return get(&s[i]);
    }
    if i == 0 || i == s.len() {
        return None;
    }
    let (a, b) = (&s[i - 1], &s[i]);
    let (va, vb) = (get(a)?, get(b)?);
    let w = if b.t > a.t { (t - a.t) / (b.t - a.t) } else { 0.0 };
    Some([0, 1, 2].map(|k| va[k] + (vb[k] - va[k]) * w))
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Motion figures of one series: when it started to move (more than 1 unit
/// from where it began), its top speed (between samples), when it arrived
/// (first within 5 units of where it ended, after starting) and the path length.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Motion {
    pub start: Option<f64>,
    pub top_speed: f64,
    pub arrive: Option<f64>,
    pub path: f64,
}

pub fn motion(s: &[Sample]) -> Motion {
    let pts: Vec<(f64, [f64; 3])> = s.iter().filter_map(|x| Some((x.t, x.pos?))).collect();
    let mut m = Motion::default();
    let (Some(first), Some(last)) = (pts.first(), pts.last()) else { return m };
    m.start = pts.iter().find(|p| dist(p.1, first.1) > 1.0).map(|p| p.0);
    for w in pts.windows(2) {
        let d = dist(w[0].1, w[1].1);
        m.path += d;
        if w[1].0 > w[0].0 {
            m.top_speed = m.top_speed.max(d / (w[1].0 - w[0].0));
        }
    }
    if let Some(st) = m.start
        && dist(first.1, last.1) > 5.0
    {
        m.arrive = pts.iter().find(|p| p.0 >= st && dist(p.1, last.1) <= 5.0).map(|p| p.0);
    }
    m
}

// ---- the STA64 side ----------------------------------------------------

pub fn tick_of(t: f64) -> Tick {
    (t * f64::from(TICK_HZ)).round().max(0.0) as Tick
}

fn secs_of(tick: Tick) -> f64 {
    tick as f64 / f64::from(TICK_HZ)
}

/// A command for the STA64 side and when it lands (sim seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct Timed {
    pub t: f64,
    pub text: String,
}

/// What [`run_sta64`] wrote.
#[derive(Debug, Clone)]
pub struct Sta64Run {
    pub ticks: Tick,
    pub digest: u64,
    /// `(tick, command, answer)`.
    pub answers: Vec<(Tick, String, String)>,
}

/// Merge bursts per shooter and weapon, as Refit marks them (its
/// `fire.start`/`fire.end` follow a shooter's ordnance of one weapon ODF;
/// STA64's one hardpoint). `marks`: `(tick, kind, obj, weapon, line)`.
fn merge_bursts(fire: &[(Tick, &'static str, String, String)], gap: Tick) -> Vec<(Tick, &'static str, String, String)> {
    // (obj, weapon) -> intervals
    type Intervals = Vec<(Tick, Option<Tick>)>;
    let mut open: BTreeMap<(String, String), Intervals> = BTreeMap::new();
    for (t, kind, obj, w) in fire {
        let v = open.entry((obj.clone(), w.clone())).or_default();
        match *kind {
            "fire.start" => v.push((*t, None)),
            _ => {
                if let Some(iv) = v.iter_mut().rev().find(|iv| iv.1.is_none()) {
                    iv.1 = Some(*t);
                }
            }
        }
    }
    let mut out = Vec::new();
    for ((obj, w), mut ivs) in open {
        ivs.sort_by_key(|iv| iv.0);
        let mut merged: Vec<(Tick, Option<Tick>)> = Vec::new();
        for iv in ivs {
            match merged.last_mut() {
                Some(last) if last.1.is_none_or(|e| iv.0 <= e + gap) => {
                    last.1 = match (last.1, iv.1) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        _ => None,
                    };
                }
                _ => merged.push(iv),
            }
        }
        for (s, e) in merged {
            out.push((s, "fire.start", obj.clone(), w.clone()));
            if let Some(e) = e {
                out.push((e, "fire.end", obj.clone(), w.clone()));
            }
        }
    }
    out
}

fn ordnance_label(db: &sta_content::ContentDb, weapon_odf: &str) -> String {
    db.key(weapon_odf).map_or_else(|| weapon_odf.to_owned(), |k| crate::marks::ordnance_label(db, k))
}

/// Run the description on STA64's sim in-process and write `dir`
/// (`Scene.ini`, `Scene.marks`, `Scene.objects.jsonl`, `bench.log`).
/// `schedule`: the commands and when they land; `end_t`: sim seconds to run.
pub fn run_sta64(scene_text: &str, cfg: Arc<RunConfig>, schedule: &[Timed], end_t: f64, dir: &Path) -> Result<Sta64Run, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    fs::write(dir.join("Scene.ini"), scene_text).map_err(|e| e.to_string())?;
    let scene = SceneFile::parse(scene_text).map_err(|e| format!("scene: {e}"))?;
    let gap = tick_of(scene.marks.fire_gap);
    let mut b = Bench::build(cfg.clone(), &scene)?;
    let mut sched: Vec<(Tick, &str)> = schedule.iter().map(|c| (tick_of(c.t), c.text.as_str())).collect();
    sched.sort_by_key(|c| c.0);
    let end = tick_of(end_t);
    let mut answers = Vec::new();
    let mut next = 0;
    loop {
        let now = b.tick();
        while next < sched.len() && sched[next].0 <= now {
            let text = sched[next].1;
            let ans = match parse_command(text) {
                Ok(Some(c)) => b.exec_timed(&c),
                Ok(None) => "! empty".into(),
                Err(e) => format!("! {e}"),
            };
            answers.push((now, text.to_owned(), ans));
            next += 1;
        }
        if now >= end || !b.advance() {
            break;
        }
    }
    let ticks = b.tick();
    let digest = b.digest_at(ticks)?;

    // Scene.marks, Refit's format: `TICK kind t=S k=v ...`.
    let mut lines: Vec<(Tick, u8, String)> = Vec::new();
    let named = |id| {
        let n = b.name_of(id);
        (!n.starts_with('#')).then_some(n)
    };
    let v0 = b.view_at(0)?;
    lines.push((0, 0, format!("ready t=0.000 objects={}", v0.units.len())));
    for (tick, text, _) in &answers {
        lines.push((*tick, 1, format!("cmd t={:.3} text={text}", secs_of(*tick))));
    }
    // Events, Refit's names; fire bursts merged per shooter and weapon.
    let mut fire = Vec::new();
    // The bench's own marks (`ord.new`/`ord.gone` with `Ordnance=1`, `bank`);
    // its `state` lines are left out for the per-tick ones below.
    for m in b.marks().into_iter().filter(|m| m.kind != "state") {
        let mut fields: BTreeMap<String, String> = BTreeMap::new();
        for (k, val) in &m.fields {
            let s = match val {
                crate::marks::Val::Obj(id) => b.name_of(*id),
                crate::marks::Val::Text(t) => t.clone(),
            };
            if *k != "t" {
                fields.insert((*k).to_owned(), s);
            }
        }
        let kind = parity::canonical_ours(m.kind, &fields);
        if kind == "fire.start" || kind == "fire.end" {
            let o = fields.get("obj").cloned().unwrap_or_default();
            let w = ordnance_label(&cfg.content, fields.get("weapon").map_or("", String::as_str));
            fire.push((m.tick, if kind == "fire.start" { "fire.start" } else { "fire.end" }, o, w));
            continue;
        }
        let mut s = format!("{kind} t={:.3}", secs_of(m.tick));
        for (k, v) in &fields {
            let _ = write!(s, " {k}={v}");
        }
        lines.push((m.tick, 2, s));
    }
    for (tick, kind, o, w) in merge_bursts(&fire, gap) {
        lines.push((tick, 2, format!("{kind} t={:.3} obj={o} weapon={w}", secs_of(tick))));
    }
    // A `state` line per named object every tick (Refit: every `State=` ms).
    for k in 0..=ticks {
        let v = b.view_at(k)?;
        for u in &v.units {
            let Some(n) = named(u.id) else { continue };
            // No special-energy pool: no `sp=` (never a made-up 0).
            let sp = u.energy.map_or(String::new(), |e| format!(" sp={:.3}", to_f64(e[0])));
            lines.push((
                k,
                4,
                format!(
                    "state t={:.3} obj={n} pos={:.3},{:.3},{:.3} hull={:.3} sh={:.3}{sp}",
                    secs_of(k),
                    to_f64(u.pos.x),
                    to_f64(u.pos.y),
                    to_f64(u.pos.z),
                    to_f64(u.hull[0]),
                    to_f64(u.shields[0][0]),
                ),
            ));
        }
    }
    lines.sort_by_key(|a| (a.0, a.1));
    let mut marks = String::new();
    for (t, _, l) in &lines {
        let _ = writeln!(marks, "{t} {l}");
    }
    fs::write(dir.join("Scene.marks"), marks).map_err(|e| e.to_string())?;
    fs::write(dir.join("Scene.objects.jsonl"), b.objects_jsonl()).map_err(|e| e.to_string())?;
    fs::write(dir.join("bench.log"), b.log.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(Sta64Run { ticks, digest, answers })
}

/// `run.json` of the STA64 side.
pub fn sta64_run_json(content: &str, run: &Sta64Run, frames: &[(f64, Tick, Option<String>)], frames_note: &str) -> String {
    let mut s = format!(
        "{{\"schema\":1,\"content\":{},\"ticks\":{},\"tick_hz\":{},\"digest\":\"{:016x}\",\"commands\":[",
        json::string(content),
        run.ticks,
        TICK_HZ,
        run.digest
    );
    for (i, (t, c, a)) in run.answers.iter().enumerate() {
        let _ = write!(
            s,
            "{}{{\"t\":{},\"tick\":{t},\"text\":{},\"answer\":{}}}",
            if i > 0 { "," } else { "" },
            json::num(secs_of(*t)),
            json::string(c),
            json::string(a)
        );
    }
    s += "],\"frames\":[";
    for (i, (t, tick, file)) in frames.iter().enumerate() {
        let _ = write!(
            s,
            "{}{{\"t\":{},\"tick\":{tick},\"file\":{}}}",
            if i > 0 { "," } else { "" },
            json::num(*t),
            file.as_deref().map_or("null".into(), json::string)
        );
    }
    let _ = writeln!(s, "],\"frames_note\":{}}}", json::string(frames_note));
    s
}

// ---- frames ------------------------------------------------------------

/// An 8-bit RGB image.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}

pub fn read_png(path: &Path) -> Result<Image, String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("png: image too large")?];
    let info = r.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = (info.width, info.height);
    let n = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(format!("{}: palette not expanded", path.display())),
    };
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h as usize {
        let row = &buf[y * info.line_size..];
        for x in 0..w as usize {
            let p = &row[x * n..x * n + n];
            match n {
                1 | 2 => rgb.extend([p[0]; 3]),
                _ => rgb.extend(&p[..3]),
            }
        }
    }
    Ok(Image { w, h, rgb })
}

pub fn write_png(path: &Path, img: &Image) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.w, img.h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(&img.rgb).map_err(|e| e.to_string())
}

/// Nearest-neighbour resize (only to bring two frames to one size).
pub fn resize(img: &Image, w: u32, h: u32) -> Image {
    if (img.w, img.h) == (w, h) {
        return img.clone();
    }
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        let sy = (u64::from(y) * u64::from(img.h) / u64::from(h.max(1))) as u32;
        for x in 0..w {
            let sx = (u64::from(x) * u64::from(img.w) / u64::from(w.max(1))) as u32;
            let i = ((sy * img.w + sx) * 3) as usize;
            rgb.extend(&img.rgb[i..i + 3]);
        }
    }
    Image { w, h, rgb }
}

/// Refit's frame-comparison figures (`Expect shots ... to match within`): mean
/// absolute difference over every channel (0..255), the largest, the share of
/// pixels that differ at all and by more than 16 in some channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameDiff {
    pub mean: f64,
    pub max: u8,
    pub share: f64,
    pub share16: f64,
}

/// `b` is brought to `a`'s size first.
pub fn frame_diff(a: &Image, b: &Image) -> FrameDiff {
    let b = resize(b, a.w, a.h);
    let (mut sum, mut max, mut diff, mut diff16) = (0u64, 0u8, 0usize, 0usize);
    for (pa, pb) in a.rgb.chunks_exact(3).zip(b.rgb.chunks_exact(3)) {
        let d = [0, 1, 2].map(|k| pa[k].abs_diff(pb[k]));
        sum += d.iter().map(|x| u64::from(*x)).sum::<u64>();
        let m = *d.iter().max().expect("3");
        max = max.max(m);
        diff += usize::from(m > 0);
        diff16 += usize::from(m > 16);
    }
    let n = (a.w as usize * a.h as usize).max(1);
    FrameDiff { mean: sum as f64 / (n * 3) as f64, max, share: diff as f64 / n as f64, share16: diff16 as f64 / n as f64 }
}

/// `a | b | difference` (b brought to a's size; the difference is the largest
/// channel difference, times four, in red).
pub fn side_by_side(a: &Image, b: &Image) -> Image {
    let b = resize(b, a.w, a.h);
    let w = a.w * 3;
    let mut rgb = Vec::with_capacity((w * a.h * 3) as usize);
    for y in 0..a.h as usize {
        let r = y * a.w as usize * 3..(y + 1) * a.w as usize * 3;
        rgb.extend(&a.rgb[r.clone()]);
        rgb.extend(&b.rgb[r.clone()]);
        for (pa, pb) in a.rgb[r.clone()].chunks_exact(3).zip(b.rgb[r].chunks_exact(3)) {
            let d = (0..3).map(|k| pa[k].abs_diff(pb[k])).max().unwrap_or(0);
            rgb.extend([d.saturating_mul(4), 0, 0]);
        }
    }
    Image { w, h: a.h, rgb }
}

// ---- the comparison ----------------------------------------------------

/// One side's directory, read.
struct Side {
    meta: Jv,
    trace: Trace,
}

fn read_side(dir: &Path, meta: &str) -> Result<Side, String> {
    let mp = dir.join(meta);
    let meta = fs::read_to_string(&mp).map_err(|e| format!("{}: {e}", mp.display()))?;
    let meta = parse_json(&meta).ok_or_else(|| format!("{}: not JSON", mp.display()))?;
    let marks = dir.join("Scene.marks");
    let text = fs::read_to_string(&marks).map_err(|e| format!("{}: {e}", marks.display()))?;
    Ok(Side { meta, trace: read_trace(&text) })
}

fn stats(v: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
    // (max, mean, t of max)
    let (mut max, mut tmax, mut sum) = (f64::MIN, 0.0, 0.0);
    for (t, d) in v {
        sum += d;
        if *d > max {
            max = *d;
            tmax = *t;
        }
    }
    (!v.is_empty()).then(|| (max, sum / v.len() as f64, tmax))
}

fn jnum(v: Option<f64>) -> String {
    v.map_or("null".into(), |x| json::num((x * 1000.0).round() / 1000.0))
}

/// The written report.
#[derive(Debug, Clone)]
pub struct Report {
    pub json: String,
    pub md: String,
    /// Differences beyond the tolerances.
    pub out_of_tolerance: usize,
}

type EvKey = (String, String, String);

/// Kinds paired one by one in the events table; the measuring kinds have
/// tables of their own (series, ordnance counts).
fn one_by_one(kind: &str) -> bool {
    parity::support(kind) == Support::Both && !parity::MEASURING.contains(&kind)
}

fn event_key(m: &PMark) -> EvKey {
    let w = if m.kind.starts_with("fire.") { m.fields.get("weapon").cloned().unwrap_or_default().to_ascii_lowercase() } else { String::new() };
    (m.kind.clone(), m.obj.clone().unwrap_or_default(), w)
}

/// Compare the two sides' directories and write `report.json`, `report.md`
/// and `frames/f<i>.png` (side by side) under `out`. Reads nothing else, so
/// the same two directories give the same report.
pub fn compare(desc: &PairDesc, refit_dir: &Path, sta_dir: &Path, out: &Path) -> Result<Report, String> {
    let r = read_side(refit_dir, "refit.json")?;
    let s = read_side(sta_dir, "run.json")?;
    let tol = desc.tol;
    let mut over = 0usize;
    let mut md = String::new();
    let mut js = String::new();
    let _ = write!(
        js,
        "{{\"schema\":1,\"pair\":{},\"game\":{},\"tolerances\":{},",
        json::string(&desc.name),
        json::string(&desc.game),
        json::string(&tol.text())
    );
    let _ = write!(
        js,
        "\"refit\":{{\"dir\":{},\"session\":{},\"commit\":{},\"launch\":{},\"ready_t\":{},\"end_t\":{}}},",
        json::string(&refit_dir.display().to_string()),
        json::string(r.meta.get("session").str().unwrap_or("")),
        json::string(r.meta.get("refit_commit").str().unwrap_or("")),
        json::string(&r.meta.get("launch").arr().iter().filter_map(Jv::str).collect::<Vec<_>>().join(" ")),
        jnum(r.meta.get("ready_t").num()),
        jnum(r.meta.get("end_t").num()),
    );
    let _ = write!(
        js,
        "\"sta64\":{{\"dir\":{},\"content\":{},\"ticks\":{},\"digest\":{},\"frames_note\":{}}},",
        json::string(&sta_dir.display().to_string()),
        json::string(s.meta.get("content").str().unwrap_or("")),
        jnum(s.meta.get("ticks").num()),
        json::string(s.meta.get("digest").str().unwrap_or("")),
        json::string(s.meta.get("frames_note").str().unwrap_or("")),
    );
    let _ = writeln!(md, "# Paired scene `{}` on {} against STA64\n", desc.name, desc.game);
    let _ = writeln!(
        md,
        "Original: Refit bench session `{}` (Refit {}), launch `{}`. STA64: {} ({} ticks). Times are sim seconds after the scene was built. Tolerances: {}.\n",
        r.meta.get("session").str().unwrap_or("?"),
        r.meta.get("refit_commit").str().unwrap_or("?").get(..8).unwrap_or("?"),
        r.meta.get("launch").arr().iter().filter_map(Jv::str).collect::<Vec<_>>().join(" "),
        s.meta.get("content").str().unwrap_or("?"),
        s.meta.get("ticks").num().unwrap_or(0.0),
        tol.text()
    );

    // Commands: when each landed on either side.
    js += "\"commands\":[";
    let _ = writeln!(md, "## Commands\n\n| command | asked | original | STA64 |\n|---|---|---|---|");
    let sc = s.meta.get("commands").arr();
    let mut used_cmd = vec![false; sc.len()];
    for (i, c) in r.meta.get("commands").arr().iter().enumerate() {
        let text = c.get("text").str().unwrap_or("");
        let hit = sc.iter().enumerate().find(|(j, x)| !used_cmd[*j] && x.get("text").str() == Some(text));
        let ours = hit.and_then(|(j, x)| {
            used_cmd[j] = true;
            x.get("t").num()
        });
        let _ = write!(
            js,
            "{}{{\"text\":{},\"at\":{},\"refit_t\":{},\"sta64_t\":{}}}",
            if i > 0 { "," } else { "" },
            json::string(text),
            jnum(c.get("at").num()),
            jnum(c.get("landed_t").num()),
            jnum(ours)
        );
        let fm = |v: Option<f64>| v.map_or("-".into(), |x| format!("{x:.3}"));
        let _ = writeln!(md, "| `{text}` | {} | {} | {} |", fm(c.get("at").num()), fm(c.get("landed_t").num()), fm(ours));
    }
    js += "],";

    // Events.
    let both = |m: &&PMark| one_by_one(&m.kind);
    let group = |tr: &Trace| {
        let mut g: BTreeMap<EvKey, Vec<f64>> = BTreeMap::new();
        for m in tr.events.iter().filter(both) {
            g.entry(event_key(m)).or_default().push(m.t);
        }
        g
    };
    let (gr, gs) = (group(&r.trace), group(&s.trace));
    let mut keys: Vec<&EvKey> = gr.keys().chain(gs.keys()).collect();
    keys.sort();
    keys.dedup();
    js += "\"events\":[";
    let _ = writeln!(md, "\n## Events\n\nPaired in order within kind, object and weapon: one burst missing early shifts the pairs after it.\n\n| kind | obj | weapon | n | original | STA64 | delta |\n|---|---|---|---|---|---|---|");
    let mut first = true;
    let mut ev_compared = 0;
    let empty = Vec::new();
    for k in keys {
        let (a, b) = (gr.get(k).unwrap_or(&empty), gs.get(k).unwrap_or(&empty));
        for i in 0..a.len().max(b.len()) {
            let (x, y) = (a.get(i).copied(), b.get(i).copied());
            let bad = match (x, y) {
                (Some(x), Some(y)) => {
                    ev_compared += 1;
                    (x - y).abs() > tol.time
                }
                _ => true,
            };
            over += usize::from(bad);
            let _ = write!(
                js,
                "{}{{\"kind\":{},\"obj\":{},\"weapon\":{},\"n\":{},\"refit_t\":{},\"sta64_t\":{},\"within\":{}}}",
                if first { "" } else { "," },
                json::string(&k.0),
                json::string(&k.1),
                json::string(&k.2),
                i + 1,
                jnum(x),
                jnum(y),
                !bad
            );
            first = false;
            // The table lists the first five of a kind, and every one beyond tolerance.
            if i < 5 || bad {
                let fm = |v: Option<f64>| v.map_or("-".into(), |v| format!("{v:.3}"));
                let d = match (x, y) {
                    (Some(x), Some(y)) => format!("{:+.3}{}", y - x, if bad { " **over**" } else { "" }),
                    _ => "**only one side**".into(),
                };
                let _ = writeln!(md, "| {} | {} | {} | {} | {} | {} | {d} |", k.0, k.1, k.2, i + 1, fm(x), fm(y));
            }
        }
    }
    js += "],";
    let skipped = |tr: &Trace| {
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        for e in tr.events.iter().filter(|m| !one_by_one(&m.kind)) {
            *m.entry(e.kind.clone()).or_default() += 1;
        }
        m
    };
    let sk = |m: BTreeMap<String, usize>| m.iter().map(|(k, n)| format!("{k} x{n}")).collect::<Vec<_>>().join(", ");
    let _ = writeln!(md, "\nNot compared one by one (one engine's kinds, or series below): original {}; STA64 {}.", sk(skipped(&r.trace)), sk(skipped(&s.trace)));

    // Ordnance counts per shooter and weapon.
    let ord = |tr: &Trace| {
        let mut m: BTreeMap<(String, String), (usize, f64, f64)> = BTreeMap::new();
        for e in tr.events.iter().filter(|m| m.kind == "ord.new") {
            let k = (e.obj.clone().unwrap_or_default(), e.fields.get("weapon").cloned().unwrap_or_default().to_ascii_lowercase());
            let x = m.entry(k).or_insert((0, e.t, e.t));
            x.0 += 1;
            x.2 = e.t;
        }
        m
    };
    let (or, os) = (ord(&r.trace), ord(&s.trace));
    let mut okeys: Vec<&(String, String)> = or.keys().chain(os.keys()).collect();
    okeys.sort();
    okeys.dedup();
    js += "\"ordnance\":[";
    if !okeys.is_empty() {
        let _ = writeln!(md, "\n## Ordnance (shots per shooter and weapon)\n\n| obj | weapon | original: n, first, last | STA64: n, first, last |\n|---|---|---|---|");
    }
    for (i, k) in okeys.iter().enumerate() {
        let (a, b) = (or.get(*k), os.get(*k));
        let part = |x: Option<&(usize, f64, f64)>| x.map_or("null".into(), |x| format!("{{\"n\":{},\"first\":{},\"last\":{}}}", x.0, jnum(Some(x.1)), jnum(Some(x.2))));
        let _ = write!(js, "{}{{\"obj\":{},\"weapon\":{},\"refit\":{},\"sta64\":{}}}", if i > 0 { "," } else { "" }, json::string(&k.0), json::string(&k.1), part(a), part(b));
        let fm = |x: Option<&(usize, f64, f64)>| x.map_or("-".into(), |x| format!("{}, {:.2}, {:.2}", x.0, x.1, x.2));
        let _ = writeln!(md, "| {} | {} | {} | {} |", k.0, k.1, fm(a), fm(b));
    }
    js += "],";

    // State series, on the original's sample times.
    js += "\"states\":[";
    let _ = writeln!(md, "\n## State series (sampled at the original's `state` times)\n\n| obj | samples | pos: max, mean (at) | hull: max | shields: max | special: max | motion original: start, top speed, arrive | motion STA64 |\n|---|---|---|---|---|---|---|---|");
    let mut objs: Vec<&String> = r.trace.states.keys().chain(s.trace.states.keys()).collect();
    objs.sort();
    objs.dedup();
    let mut sfirst = true;
    for o in objs {
        let (Some(ra), Some(sa)) = (r.trace.states.get(o), s.trace.states.get(o)) else {
            over += 1;
            let side = if r.trace.states.contains_key(o) { "original" } else { "STA64" };
            let _ = write!(js, "{}{{\"obj\":{},\"only_in\":{}}}", if sfirst { "" } else { "," }, json::string(o), json::string(side));
            sfirst = false;
            let _ = writeln!(md, "| {o} | **only in {side}** | | | | | | |");
            continue;
        };
        let (mut dp, mut dh, mut ds, mut dsp) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        // Resample STA64 at the original's times, and the original on those same
        // times, so both motion figures come from one sampling.
        let mut rs: Vec<Sample> = Vec::new();
        let mut ss: Vec<Sample> = Vec::new();
        for x in ra.iter().filter(|x| x.t >= 0.0) {
            let y = |g: &dyn Fn(&Sample) -> Option<f64>| at_time(sa, x.t, |s| g(s).map(|v| [v, 0.0, 0.0])).map(|v| v[0]);
            let ypos = at_time(sa, x.t, |s| s.pos);
            if ypos.is_none() && y(&|s| s.hull).is_none() {
                continue;
            }
            rs.push(*x);
            ss.push(Sample { t: x.t, pos: ypos, hull: y(&|s| s.hull), sh: y(&|s| s.sh), sp: y(&|s| s.sp) });
            if let (Some(a), Some(b)) = (x.pos, ypos) {
                dp.push((x.t, dist(a, b)));
            }
            for (dv, a, b) in [(&mut dh, x.hull, y(&|s| s.hull)), (&mut ds, x.sh, y(&|s| s.sh)), (&mut dsp, x.sp, y(&|s| s.sp))] {
                if let (Some(a), Some(b)) = (a, b) {
                    dv.push((x.t, (a - b).abs()));
                }
            }
        }
        let (mr, ms) = (motion(&rs), motion(&ss));
        let mut bad = 0;
        let part = |v: &[(f64, f64)], lim: f64, bad: &mut usize| match stats(v) {
            Some((max, mean, t)) => {
                let first = v.iter().find(|x| x.1 > lim).map(|x| x.0);
                *bad += usize::from(max > lim);
                (format!("{{\"max\":{},\"mean\":{},\"t_max\":{},\"first_over\":{}}}", jnum(Some(max)), jnum(Some(mean)), jnum(Some(t)), jnum(first)), Some((max, mean, t)))
            }
            None => ("null".into(), None),
        };
        let (jp, sp_) = part(&dp, tol.pos, &mut bad);
        let (jh, sh_) = part(&dh, tol.hull, &mut bad);
        let (jsh, ss_) = part(&ds, tol.hull, &mut bad);
        let (jsp, ssp_) = part(&dsp, tol.hull, &mut bad);
        let mo = |m: &Motion| format!("{{\"start\":{},\"top_speed\":{},\"arrive\":{},\"path\":{}}}", jnum(m.start), jnum(Some(m.top_speed)), jnum(m.arrive), jnum(Some(m.path)));
        for (a, b) in [(mr.start, ms.start), (mr.arrive, ms.arrive)] {
            bad += usize::from(match (a, b) {
                (Some(a), Some(b)) => (a - b).abs() > tol.time,
                (None, None) => false,
                _ => true,
            });
        }
        over += bad;
        let _ = write!(
            js,
            "{}{{\"obj\":{},\"samples\":{},\"pos\":{jp},\"hull\":{jh},\"shields\":{jsh},\"special\":{jsp},\"motion\":{{\"refit\":{},\"sta64\":{}}},\"beyond_tolerance\":{bad}}}",
            if sfirst { "" } else { "," },
            json::string(o),
            rs.len(),
            mo(&mr),
            mo(&ms)
        );
        sfirst = false;
        let fm1 = |x: Option<(f64, f64, f64)>| x.map_or("-".into(), |x| format!("{:.1}", x.0));
        let fmp = |x: Option<(f64, f64, f64)>| x.map_or("-".into(), |x| format!("{:.1}, {:.1} ({:.2} s)", x.0, x.1, x.2));
        let fmm = |m: &Motion| {
            let o = |v: Option<f64>| v.map_or("-".into(), |v| format!("{v:.2}"));
            format!("{}, {:.1}/s, {}", o(m.start), m.top_speed, o(m.arrive))
        };
        let _ = writeln!(
            md,
            "| {o} | {} | {}{} | {} | {} | {} | {} | {} |",
            rs.len(),
            fmp(sp_),
            if bad > 0 { " **over**" } else { "" },
            fm1(sh_),
            fm1(ss_),
            fm1(ssp_),
            fmm(&mr),
            fmm(&ms)
        );
    }
    js += "],";

    // Frames.
    js += "\"frames\":[";
    let _ = writeln!(md, "\n## Frames\n");
    let sf = s.meta.get("frames").arr();
    let rf = r.meta.get("frames").arr();
    if rf.is_empty() {
        let _ = writeln!(md, "No frames were asked for.");
    } else {
        let _ = writeln!(
            md,
            "STA64 side: {}.\n\nEach pair is written as `frames/f<i>.png`: original | STA64 | difference (x4, red) | blurred difference (x4, red). Frames are game-derived and stay local. `mean`, `max`, `differing` and `>16` are per pixel; the metrics below are robust to a small misalignment.\n\n| frame | t original | STA64 tick | mean | max | differing | >16 | judged |\n|---|---|---|---|---|---|---|---|",
            s.meta.get("frames_note").str().unwrap_or("not taken")
        );
    }
    let _ = fs::create_dir_all(out.join("frames"));
    let mut measured: Vec<(String, pair_metrics::FrameMetrics)> = Vec::new();
    for (i, fr) in rf.iter().enumerate() {
        let t = fr.get("t").num().or(fr.get("at").num());
        let sfr = sf.get(i);
        let (a, b) = (fr.get("file").str().map(|p| refit_dir.join(p)), sfr.and_then(|x| x.get("file").str()).map(|p| sta_dir.join(p)));
        let tick = sfr.and_then(|x| x.get("tick").num());
        let res = match (a, b) {
            (Some(a), Some(b)) => read_png(&a).and_then(|ia| read_png(&b).map(|ib| (ia, ib))),
            (None, _) => Err("no original frame".into()),
            (_, None) => Err(format!("no STA64 frame ({})", s.meta.get("frames_note").str().unwrap_or("not taken"))),
        };
        match res {
            Ok((ia, ib)) => {
                let m = frame_metrics(&ia, &ib);
                let d = m.plain;
                // Judged: the plain mean when `frame=` is set, and each metric with a limit.
                let mut breaches: Vec<String> = Vec::new();
                if let Some(lim) = tol.frame.filter(|lim| d.mean > *lim) {
                    breaches.push(format!("frame {:.2} > {lim}", d.mean));
                }
                for (k, v, lim) in tol.metrics.exceeded(&m) {
                    breaches.push(format!("{k} {v:.2} > {lim}"));
                }
                let judged = (tol.frame.is_some() || !tol.metrics.text().is_empty()).then_some(breaches.is_empty());
                over += breaches.len();
                let name = format!("frames/f{i}.png");
                write_png(&out.join(&name), &pair_metrics::strip(&ia, &ib))?;
                let _ = write!(
                    js,
                    "{}{{\"i\":{i},\"t\":{},\"sta64_tick\":{},\"pair\":{},\"size\":[{},{}],\"sta64_size\":[{},{}],\"mean\":{},\"max\":{},\"share\":{},\"share16\":{},\"within\":{},\"breaches\":[{}],\"metrics\":{}}}",
                    if i > 0 { "," } else { "" },
                    jnum(t),
                    jnum(tick),
                    json::string(&name),
                    ia.w,
                    ia.h,
                    ib.w,
                    ib.h,
                    jnum(Some(d.mean)),
                    d.max,
                    jnum(Some(d.share)),
                    jnum(Some(d.share16)),
                    judged.map_or("null".into(), |b| b.to_string()),
                    breaches.iter().map(|b| json::string(b)).collect::<Vec<_>>().join(","),
                    pair_metrics::metrics_json(&m)
                );
                let _ = writeln!(
                    md,
                    "| f{i} | {} | {} | {:.2} | {} | {:.1}% | {:.1}% | {} |",
                    t.map_or("-".into(), |t| format!("{t:.3}")),
                    tick.map_or("-".into(), |t| format!("{t}")),
                    d.mean,
                    d.max,
                    d.share * 100.0,
                    d.share16 * 100.0,
                    match judged {
                        None => "no (reported only)".to_owned(),
                        Some(true) => "within".to_owned(),
                        Some(false) => format!("**over**: {}", breaches.join("; ")),
                    }
                );
                measured.push((format!("f{i}"), m));
            }
            Err(e) => {
                let _ = write!(js, "{}{{\"i\":{i},\"t\":{},\"error\":{}}}", if i > 0 { "," } else { "" }, jnum(t), json::string(&e));
                let _ = writeln!(md, "| f{i} | {} | - | - | - | - | - | {e} |", t.map_or("-".into(), |t| format!("{t:.3}")));
            }
        }
    }
    if !measured.is_empty() {
        let rows: Vec<(String, &pair_metrics::FrameMetrics)> = measured.iter().map(|(n, m)| (n.clone(), m)).collect();
        md += &pair_metrics::metrics_md(&rows);
    }
    js += "],";
    let notes: Vec<String> = r.meta.get("notes").arr().iter().filter_map(Jv::str).map(str::to_owned).collect();
    let _ = write!(js, "\"refit_notes\":[{}],", notes.iter().map(|n| json::string(n)).collect::<Vec<_>>().join(","));
    let _ = writeln!(js, "\"events_compared\":{ev_compared},\"out_of_tolerance\":{over}}}");
    if !notes.is_empty() {
        let _ = writeln!(md, "\n## Notes from the original side\n");
        for n in &notes {
            let _ = writeln!(md, "- {n}");
        }
    }
    let _ = writeln!(md, "\n**{over} differences beyond tolerance** ({ev_compared} event pairs compared).");
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    fs::write(out.join("report.json"), &js).map_err(|e| e.to_string())?;
    fs::write(out.join("report.md"), &md).map_err(|e| e.to_string())?;
    Ok(Report { json: js, md, out_of_tolerance: over })
}

/// The schedule for the STA64 side: each `At=` command when it landed on
/// the original (`refit.json`), else when it was asked for.
pub fn schedule(desc: &PairDesc, refit_meta: Option<&Jv>) -> Vec<Timed> {
    let landed = refit_meta.map(|m| m.get("commands").arr()).unwrap_or(&[]);
    let mut used = vec![false; landed.len()];
    desc.at
        .iter()
        .map(|(t, text)| {
            let hit = landed.iter().enumerate().find(|(i, c)| !used[*i] && c.get("text").str() == Some(text.as_str()));
            let t = match hit {
                Some((i, c)) => {
                    used[i] = true;
                    c.get("landed_t").num().unwrap_or(*t)
                }
                None => *t,
            };
            Timed { t, text: text.clone() }
        })
        .collect()
}

/// The frames for the STA64 side: each at the sim time the original took its own.
pub fn frame_times(desc: &PairDesc, refit_meta: Option<&Jv>) -> Vec<f64> {
    let taken = refit_meta.map(|m| m.get("frames").arr()).unwrap_or(&[]);
    desc.frames
        .iter()
        .enumerate()
        .map(|(i, at)| taken.get(i).and_then(|f| f.get("t").num()).unwrap_or(*at))
        .collect()
}

pub fn read_meta(dir: &Path, file: &str) -> Result<Jv, String> {
    let p = dir.join(file);
    let t = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    parse_json(&t).ok_or_else(|| format!("{}: not JSON", p.display()))
}

/// Content from an install (and mod), for the STA64 side of a real pair, with
/// the install's maps `maps` (stems under `bzn/`) for `Map=`. The maps are
/// converted as the map view converts them (`MapDef::convert_bzn_named_in`),
/// so the same scene builds the same sim in the bench and in the client.
pub fn install_config(root: &Path, mod_name: Option<&str>, maps: &[String]) -> Result<RunConfig, String> {
    let vfs = sta_vfs::Vfs::mount_install(root, mod_name).map_err(|e| format!("{}: {e:?}", root.display()))?;
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = sta_content::ContentDb::load(&vfs, &index).map_err(|e| format!("content: {e:?}"))?;
    let mut cfg = RunConfig::fake();
    cfg.aliases.clear();
    cfg.maps.clear();
    for name in maps {
        let stem = map_stem(name);
        let data = [format!("bzn/{stem}.bzn"), format!("bzn/{stem}.sol")]
            .iter()
            .find_map(|p| vfs.read(p))
            .ok_or_else(|| format!("Map={stem}: no bzn/{stem}.bzn in the install"))?
            .map_err(|e| format!("Map={stem}: {e}"))?;
        let bzn = sta_bzn::parse(&data).map_err(|e| format!("Map={stem}: {e}"))?;
        let conv = sta_content::MapDef::convert_bzn_named_in(&bzn, &stem, &db, &vfs, &index);
        cfg.maps.insert(stem, conv.map);
    }
    cfg.content = Arc::new(db);
    Ok(cfg)
}

/// True when `p` lies inside a checkout of this repository (frames of stock
/// art never go there).
pub fn inside_checkout(p: &Path) -> bool {
    let abs = if p.is_absolute() { p.to_owned() } else { std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_owned()) };
    // Resolve `..` and symlinks through the nearest part that exists.
    let abs = abs
        .ancestors()
        .find_map(|a| {
            let c = a.canonicalize().ok()?;
            Some(c.join(abs.strip_prefix(a).ok()?))
        })
        .unwrap_or(abs);
    abs.ancestors().any(|a| a.join(".git").exists() && a.join("bench").join("pairs").is_dir())
}

/// The default report root: `$XDG_DATA_HOME/sta64/bench/pairs`.
pub fn default_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("sta64/bench/pairs")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAIR: &str = "; c\n[Scene]\nAnchor=world\nSeed=7\n\n[Marks]\nState=250\n\n[Pair]\nGames=a2,roots\nDuration=10\nFrames=2,5s\nTol=time=0.2,pos=4\nAt=1 goto ship 2000 0 3000\nAt=1.5s stop ship\nroots.Size=1280x960\nroots.Mod=STA2 Classic Mod\n\n[Object.ship]\nOdf=fgalaxy\nroots.Odf=fgalaxy_r\nfake.Odf=fx_cruiser\nTeam=1\n";

    #[test]
    fn description_reads_per_game() {
        let a = PairDesc::parse("p", PAIR, "a2").unwrap();
        assert_eq!((a.duration, a.frames.clone(), a.size), (10.0, vec![2.0, 5.0], (1280, 720)));
        assert_eq!(a.at, [(1.0, "goto ship 2000 0 3000".to_owned()), (1.5, "stop ship".to_owned())]);
        assert_eq!((a.tol.time, a.tol.pos, a.tol.hull, a.tol.frame), (0.2, 4.0, 1.0, None));
        assert_eq!(a.mod_name, None);
        let r = PairDesc::parse("p", PAIR, "roots").unwrap();
        assert_eq!((r.size, r.mod_name.as_deref()), ((1280, 960), Some("STA2 Classic Mod")));
        assert!(PairDesc::parse("p", PAIR, "a1").unwrap_err().contains("Armada I is not supported"));
        assert!(PairDesc::parse("p", &PAIR.replace("Games=a2,roots", "Games=a2"), "roots").is_err());
        assert!(PairDesc::parse("p", &PAIR.replace("At=1.5s stop ship", "At=1.5s frobnicate"), "a2").is_ok_and(|d| d.at.len() == 2));
        assert!(PairDesc::parse("p", &PAIR.replace("Frames=2,5s", "Frames=20"), "a2").is_err());
    }

    #[test]
    fn materialize_applies_the_games_keys() {
        let a2 = materialize(PAIR, &["a2"]);
        assert!(!a2.contains("[Pair]") && !a2.contains("Duration") && !a2.contains("roots.") && !a2.contains("fake."));
        assert!(a2.contains("Odf=fgalaxy\n") && !a2.contains("fx_cruiser"));
        let r = materialize(PAIR, &["roots"]);
        assert!(r.contains("Odf=fgalaxy_r\n") && !r.contains("Odf=fgalaxy\n"));
        let f = materialize(PAIR, &["roots", "fake"]);
        assert!(f.contains("Odf=fx_cruiser\n") && !f.contains("fgalaxy"));
        // The result parses, and keeps the scene's own keys.
        let s = SceneFile::parse(&f).unwrap();
        assert_eq!(s.objects[0].odf, "fx_cruiser");
        assert_eq!(s.marks.state_ms, Some(250));
        assert!(s.unknown.is_empty(), "{:?}", s.unknown);
    }

    #[test]
    fn the_pairs_map_comes_from_the_launch_line_or_the_key() {
        assert_eq!(launch_map("-nointro a2_borg01").as_deref(), Some("a2_borg01"));
        assert_eq!(launch_map("-nointro -mod \"STA2 Classic Mod\" FullTech.bzn").as_deref(), Some("fulltech"));
        assert_eq!(launch_map("-nointro"), None);
        assert_eq!(launch_map("-nointro a2_x -res 1280").as_deref(), Some("a2_x"));
        assert_eq!(launch_map("-mod \"x y\""), None);
        let text = PAIR.replace("roots.Mod=STA2 Classic Mod", "roots.Mod=STA2 Classic Mod\na2.Launch=-nointro a2_borg01\nroots.Launch=-nointro -mod \"STA2 Classic Mod\" fulltech.bzn");
        let a = PairDesc::parse("p", &text, "a2").unwrap();
        let r = PairDesc::parse("p", &text, "roots").unwrap();
        assert_eq!((a.map.as_deref(), r.map.as_deref()), (Some("a2_borg01"), Some("fulltech")));
        // An explicit `Map=` wins; none at all is the empty map.
        let e = PairDesc::parse("p", &text.replace("[Pair]\n", "[Pair]\nMap=A2_Other.bzn\n"), "a2").unwrap();
        assert_eq!(e.map.as_deref(), Some("a2_other"));
        assert_eq!(PairDesc::parse("p", PAIR, "a2").unwrap().map, None);
        // The scene text gets `Map=` and `MapObjects=keep` after `[Scene]`, once, unless the file says; never on fake content.
        assert!(!a.scene_text(&[]).contains("Map="));
        let s = a.scene_text(&["map"]);
        assert!(s.contains("[Scene]\nMap=a2_borg01\nMapObjects=keep\n"), "{s}");
        let parsed = SceneFile::parse(&s).unwrap();
        assert_eq!(parsed.scene.map.as_deref(), Some("a2_borg01"));
        assert_eq!(parsed.map_objects(), crate::parse::scene::MapObjects::Keep);
        assert!(!a.scene_text(&["fake"]).contains("Map="));
        let own = text.replace("[Scene]\n", "[Scene]\nMapObjects=strip\n");
        let s = PairDesc::parse("p", &own, "a2").unwrap().scene_text(&["map"]);
        assert!(s.contains("Map=a2_borg01") && s.contains("MapObjects=strip") && !s.contains("MapObjects=keep"), "{s}");
    }

    #[test]
    fn metric_tolerances_parse_and_print() {
        let t = PairTol::parse("time=0.1,frame=9,lowfreq=2.5,lum=3,colour=4,bg=1,fg=2,tile=6,midfreq=3,shift=1,bright=0.05").unwrap();
        assert_eq!((t.time, t.frame, t.metrics.lowfreq, t.metrics.colour, t.metrics.bright), (0.1, Some(9.0), Some(2.5), Some(4.0), Some(0.05)));
        assert!(t.text().contains("lowfreq=2.5") && t.text().contains("frame=9"));
        assert!(PairTol::parse("sharpness=1").unwrap_err().contains("lowfreq"));
        assert_eq!(PairTol::default().metrics.text(), "");
    }

    #[test]
    fn json_reader() {
        let v = parse_json("{\"a\":[1,{\"b\":\"x\\\"y\"}],\"c\":null,\"d\":-2.5e1,\"e\":\"é\"}").unwrap();
        assert_eq!(v.get("a").arr()[1].get("b").str(), Some("x\"y"));
        assert_eq!(v.get("d").num(), Some(-25.0));
        assert_eq!(v.get("e").str(), Some("é"));
        assert_eq!(v.get("zz"), &Jv::Null);
        assert!(parse_json("{\"a\":1} x").is_none());
    }

    #[test]
    fn traces_and_motion() {
        let t = read_trace(
            "100 ready t=1.0 objects=1\n200 state t=1.0 obj=S pos=0,0,0 hull=10 sh=5 sp=0\n300 state t=2.0 obj=S pos=0,0,0 hull=10 sh=5 sp=0\n\
             400 state t=3.0 obj=S pos=0,0,50 hull=10 sh=5 sp=0\n500 state t=4.0 obj=S pos=0,0,150 hull=10 sh=5 sp=0\n600 state t=5.0 obj=S pos=0,0,150 hull=10 sh=5 sp=0\n\
             650 fire.start t=4.5 obj=S weapon=phaser\n",
        );
        let s = &t.states["s"];
        assert_eq!(s.len(), 5);
        assert_eq!(s[2].t, 2.0);
        let m = motion(s);
        assert_eq!((m.start, m.top_speed, m.arrive, m.path), (Some(2.0), 100.0, Some(3.0), 150.0));
        assert_eq!(at_time(s, 1.5, |x| x.pos), Some([0.0, 0.0, 25.0]));
        assert_eq!(at_time(s, 9.0, |x| x.pos), None);
        assert_eq!(t.events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["ready", "fire.start"]);
    }

    #[test]
    fn bursts_merge_per_weapon() {
        let f = vec![
            (10, "fire.start", "s".to_owned(), "phaser".to_owned()),
            (10, "fire.start", "s".to_owned(), "phaser".to_owned()),
            (20, "fire.end", "s".to_owned(), "phaser".to_owned()),
            (25, "fire.end", "s".to_owned(), "phaser".to_owned()),
            (90, "fire.start", "s".to_owned(), "phaser".to_owned()),
            (95, "fire.end", "s".to_owned(), "phaser".to_owned()),
        ];
        let m = merge_bursts(&f, 30);
        assert_eq!(m.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(), [(10, "fire.start"), (25, "fire.end"), (90, "fire.start"), (95, "fire.end")]);
        let m = merge_bursts(&f, 70);
        assert_eq!(m.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(), [(10, "fire.start"), (95, "fire.end")]);
    }

    #[test]
    fn frames_diff_and_pair() {
        let a = Image { w: 2, h: 1, rgb: vec![0, 0, 0, 100, 100, 100] };
        let b = Image { w: 2, h: 1, rgb: vec![0, 0, 0, 100, 100, 120] };
        let d = frame_diff(&a, &b);
        assert_eq!((d.max, d.share, d.share16), (20, 0.5, 0.5));
        assert!((d.mean - 20.0 / 6.0).abs() < 1e-9);
        assert_eq!(frame_diff(&a, &a).mean, 0.0);
        let p = side_by_side(&a, &b);
        assert_eq!((p.w, p.h), (6, 1));
        assert_eq!(&p.rgb[15..18], &[80, 0, 0]);
        let big = resize(&a, 4, 2);
        assert_eq!((big.w, big.h, &big.rgb[..6]), (4, 2, &[0u8, 0, 0, 0, 0, 0][..]));
    }
}
