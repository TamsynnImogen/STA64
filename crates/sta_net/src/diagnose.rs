//! Offline desync diagnosis (NET-06; NET_DECISIONS N6).
//!
//! Two peers that desynced each wrote a replay (`<stem>.star`, a
//! `sta_sim::replay` log of the commands that peer's sim was given) and a
//! sidecar. [`diagnose`] re-simulates both replays from tick 0, side by side,
//! and answers the questions a developer has:
//!
//! - **which tick**: the first tick whose state differs, which is before the
//!   turn boundary the router noticed it on ([`Divergence::tick`] and
//!   [`Divergence::detected_at`]);
//! - **which part of the state**: the section digests that differ there
//!   ([`SectionDiff`]);
//! - **which entity and field**: the itemised rows of that section that
//!   differ ([`RowDiff`]; `sta_sim::digest::Sim::section_rows`);
//! - **which command preceded it**: the commands applied in the last turns
//!   before the tick on each side, and the commands only one side applied
//!   ([`Commands`]).
//!
//! Light crates only (no Bevy, no game install): the caller supplies each
//! side's sim at tick 0, built from its replay and the content it ran on.
//! Output is stable: sections, rows and commands are sorted, and
//! [`Report::to_json`] / [`Report::to_markdown`] are pure functions of the
//! report. A replay that does not reproduce its own recorded digests (another
//! sim build or content, or state edited outside the command log) is an
//! error, not a verdict: nothing is concluded from a re-simulation that is not
//! the peer's game.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use sta_core::{EntityId, Tick};
use sta_sim::command::Command;
use sta_sim::digest::SectionDigests;
use sta_sim::replay::{ControlRec, Replay};
use sta_sim::{Sim, WorldSource};

/// Turns of commands shown before the first differing tick unless told otherwise.
pub const DEFAULT_HISTORY_TURNS: u32 = 3;
/// Differing rows listed per section unless told otherwise.
pub const DEFAULT_MAX_ROWS: usize = 64;
/// Longest text kept of one command or value, so one report stays readable.
const MAX_TEXT: usize = 240;

/// Why two replays could not be compared (as opposed to: they differ).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnoseError {
    /// A file is not a readable replay (wrong magic, another format or seam
    /// revision, truncated or corrupt body).
    Replay { label: String, why: String },
    /// A side's sim is not at tick 0.
    NotAtStart { label: String, tick: Tick },
    /// Re-simulating a replay did not give the digest it recorded.
    NotReproducible {
        label: String,
        tick: Tick,
        recorded: u64,
        found: u64,
    },
    /// A replay with no recorded digests: nothing shows it is that peer's game.
    NoDigests { label: String },
    /// A sidecar is not a desync report.
    Sidecar { label: String, why: String },
}

impl std::fmt::Display for DiagnoseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiagnoseError::Replay { label, why } => write!(f, "{label}: not a readable replay: {why}"),
            DiagnoseError::NotAtStart { label, tick } => {
                write!(f, "{label}: the sim to replay starts at tick {tick}, not 0")
            }
            DiagnoseError::NotReproducible { label, tick, recorded, found } => write!(
                f,
                "{label}: re-running the replay gives digest {found:#018x} at tick {tick}, but it recorded \
                 {recorded:#018x}: this is not the game that peer ran (another sim build or content, or its \
                 state was changed outside its command log)"
            ),
            DiagnoseError::NoDigests { label } => write!(f, "{label}: the replay records no digests, so it cannot be checked against the peer's game"),
            DiagnoseError::Sidecar { label, why } => write!(f, "{label}: not a desync sidecar: {why}"),
        }
    }
}

impl std::error::Error for DiagnoseError {}

/// Reads a replay file, naming `label` in the error.
pub fn read_replay(label: &str, bytes: &[u8]) -> Result<Replay, DiagnoseError> {
    Replay::decode(bytes).map_err(|e| DiagnoseError::Replay {
        label: label.to_owned(),
        why: format!("{e:?}"),
    })
}

/// One peer's artefacts: a name for the report, its replay, and its sim at
/// tick 0 (`Sim::from_setup` of the replay's setup, or what the peer started
/// from).
pub struct Side {
    pub label: String,
    pub replay: Replay,
    pub sim: Sim,
    /// Its sidecar text, when there is one: the report cross-checks it.
    pub sidecar: Option<String>,
}

/// Knobs; [`Options::default`] is what the command line uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Turns of commands to list before the first differing tick.
    pub history_turns: u32,
    /// Ticks per turn; `None` reads it from the spacing of the replays'
    /// recorded digests (the peer logs one per turn).
    pub ticks_per_turn: Option<u32>,
    /// Differing rows listed per section.
    pub max_rows: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { history_turns: DEFAULT_HISTORY_TURNS, ticks_per_turn: None, max_rows: DEFAULT_MAX_ROWS }
    }
}

/// A command as the report shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRow {
    /// The tick the sim was at when it was given the command; its effect is
    /// first in the state at `tick + 1`.
    pub tick: Tick,
    pub player: u8,
    pub seq: u32,
    pub text: String,
}

/// The commands around the first differing tick.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Commands {
    /// First tick of the window: the last `history_turns` turns before the
    /// differing tick, both ends inclusive.
    pub from: Tick,
    pub to: Tick,
    /// What each side applied in the window, in the order applied.
    pub window: [Vec<CommandRow>; 2],
    /// Commands one side applied before the differing tick and the other did
    /// not (same tick, player, seq and content); earliest first. The first
    /// entry is the likeliest cause of a divergence that is not a state edit.
    pub only_in: [Vec<CommandRow>; 2],
}

/// One entity field (or subsystem block) of a section that differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowDiff {
    /// `None`: not per entity (generator, per-player stock, a subsystem hash).
    pub entity: Option<EntityId>,
    pub field: String,
    /// The value on each side; `None`: the row does not exist there.
    pub values: [Option<String>; 2],
}

/// A section whose digest differs at the first differing tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionDiff {
    pub section: &'static str,
    pub digests: [u64; 2],
    /// How many rows differ (the list may be shorter: [`Options::max_rows`]).
    pub differing_rows: usize,
    pub rows: Vec<RowDiff>,
}

/// What a sidecar said about the desync, read back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reported {
    pub tick: Option<Tick>,
    pub sections: Vec<String>,
}

/// Where the two games first part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// The first tick whose state differs (the sim at `tick`, before that
    /// tick's commands apply).
    pub tick: Tick,
    /// The turn boundary the recorded digests first differ on, which is where
    /// a router notices; `None` when the replays recorded no differing digest.
    pub detected_at: Option<Tick>,
    /// Section digests that differ at `tick`, in section order.
    pub sections: Vec<SectionDiff>,
    /// The sections that differ at `detected_at`: what the routers' reports
    /// can name. Empty when `detected_at` is `None`.
    pub sections_at_detection: Vec<&'static str>,
    pub commands: Commands,
}

/// The result of comparing two replays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub labels: [String; 2],
    pub end_ticks: [Tick; 2],
    pub command_counts: [usize; 2],
    pub ticks_per_turn: u32,
    /// Ticks re-simulated: both replays agree on every tick through this one
    /// when there is no divergence, and stop agreeing at `divergence.tick`.
    pub compared_through: Tick,
    /// Parts of the setups (seed, map, players, options, content, starts)
    /// that differ; a divergence at tick 0 follows from them.
    pub setup_differences: Vec<&'static str>,
    pub divergence: Option<Divergence>,
    /// Each side's sidecar, read back, when it had one.
    pub reported: [Option<Reported>; 2],
}

impl Report {
    /// Whether the two games diverged.
    pub fn diverged(&self) -> bool {
        self.divergence.is_some()
    }
}

// ---------------------------------------------------------------------------
// Diagnosis
// ---------------------------------------------------------------------------

/// A replay's commands in the order the sim applies them, and a cursor.
struct Feed<'a> {
    replay: &'a Replay,
    next_cmd: usize,
    next_digest: usize,
}

impl Feed<'_> {
    /// The commands the sim at `tick` is given.
    fn batch(&mut self, tick: Tick) -> Vec<Command> {
        let mut out = Vec::new();
        while let Some(l) = self.replay.commands.get(self.next_cmd).filter(|l| l.tick <= tick) {
            out.push(l.command.clone());
            self.next_cmd += 1;
        }
        out
    }

    /// The digest the replay recorded for `tick`, if any.
    fn recorded(&mut self, tick: Tick) -> Option<u64> {
        while let Some(&(t, d)) = self.replay.digests.get(self.next_digest) {
            if t < tick {
                self.next_digest += 1;
            } else {
                return (t == tick).then_some(d);
            }
        }
        None
    }
}

fn setup_differences(a: &Replay, b: &Replay) -> Vec<&'static str> {
    let mut d = Vec::new();
    if a.content_hash != b.content_hash {
        d.push("content");
    }
    if a.seed != b.seed {
        d.push("seed");
    }
    if a.map != b.map {
        d.push("map");
    }
    // Each peer's own seat is `Human` and the others' `Remote`: not a difference.
    let seats = |r: &Replay| -> Vec<_> {
        r.players
            .iter()
            .map(|p| (p.id, p.team, p.race.clone(), if let ControlRec::Ai(n) = &p.control { Some(n.clone()) } else { None }, p.control == ControlRec::Idle))
            .collect()
    };
    if seats(a) != seats(b) {
        d.push("players");
    }
    if a.options != b.options {
        d.push("options");
    }
    if a.starts != b.starts {
        d.push("starts");
    }
    d
}

/// Ticks per turn as the replay's recorded digests spaced it, or the default.
fn infer_ticks_per_turn(a: &Replay, b: &Replay) -> u32 {
    [a, b]
        .iter()
        .find_map(|r| match r.digests.as_slice() {
            [first, second, ..] => u32::try_from(second.0.saturating_sub(first.0)).ok().filter(|&g| g > 0),
            _ => None,
        })
        .unwrap_or(sta_sim::replay::DEFAULT_DIGEST_EVERY as u32)
}

fn first_recorded_difference(a: &Replay, b: &Replay) -> Option<Tick> {
    let map: BTreeMap<Tick, u64> = b.digests.iter().copied().collect();
    a.digests.iter().find(|(t, d)| map.get(t).is_some_and(|e| e != d)).map(|&(t, _)| t)
}

fn clip(mut s: String) -> String {
    if s.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push_str("...");
    }
    s
}

fn command_row(tick: Tick, c: &Command) -> CommandRow {
    CommandRow { tick, player: c.player.0, seq: c.seq, text: clip(format!("{:?}", c.kind)) }
}

/// Re-simulates both sides and reports where they first differ.
///
/// Errors when a side's replay does not reproduce its own recorded digests
/// (see the module docs). Compares through the end of the shorter replay.
pub fn diagnose(a: Side, b: Side, opts: &Options) -> Result<Report, DiagnoseError> {
    for s in [&a, &b] {
        let tick = s.sim.view().tick;
        if tick != 0 {
            return Err(DiagnoseError::NotAtStart { label: s.label.clone(), tick });
        }
    }
    for s in [&a, &b] {
        if s.replay.digests.is_empty() {
            return Err(DiagnoseError::NoDigests { label: s.label.clone() });
        }
    }
    let reported = [a.sidecar.as_deref().map(parse_sidecar), b.sidecar.as_deref().map(parse_sidecar)];
    let tpt = opts.ticks_per_turn.filter(|&t| t > 0).unwrap_or_else(|| infer_ticks_per_turn(&a.replay, &b.replay));
    let setup_differences = setup_differences(&a.replay, &b.replay);
    let detected_at = first_recorded_difference(&a.replay, &b.replay);
    let end = a.replay.end_tick.min(b.replay.end_tick);
    let labels = [a.label.clone(), b.label.clone()];
    let mut report = Report {
        labels: labels.clone(),
        end_ticks: [a.replay.end_tick, b.replay.end_tick],
        command_counts: [a.replay.commands.len(), b.replay.commands.len()],
        ticks_per_turn: tpt,
        compared_through: 0,
        setup_differences,
        divergence: None,
        reported,
    };

    let (mut sim_a, mut sim_b) = (a.sim, b.sim);
    let mut feeds = [Feed { replay: &a.replay, next_cmd: 0, next_digest: 0 }, Feed { replay: &b.replay, next_cmd: 0, next_digest: 0 }];
    let mut at_detection: Option<Vec<&'static str>> = None;
    let mut first: Option<(Tick, [SectionDigests; 2], [Sim; 2])> = None;
    let mut tick = 0;
    loop {
        // Is each side still the game it recorded?
        for (i, sim) in [&sim_a, &sim_b].into_iter().enumerate() {
            if let Some(recorded) = feeds[i].recorded(tick) {
                let found = sim.digest();
                if found != recorded {
                    return Err(DiagnoseError::NotReproducible { label: labels[i].clone(), tick, recorded, found });
                }
            }
        }
        let (da, db) = (sim_a.section_digests(), sim_b.section_digests());
        if first.is_none() && da != db {
            first = Some((tick, [da, db], [sim_a.clone(), sim_b.clone()]));
        }
        if detected_at == Some(tick) {
            at_detection = Some(da.differing(&db));
            // The peers' digests differ here: never report "identical", even if
            // no section shows why (state the full digest covers but no section does).
            if first.is_none() {
                first = Some((tick, [da, db], [sim_a.clone(), sim_b.clone()]));
            }
        }
        // Past the detection turn there is nothing more to learn.
        if tick >= end || (first.is_some() && (detected_at.is_none() || at_detection.is_some())) {
            break;
        }
        let batch_a = feeds[0].batch(tick);
        let batch_b = feeds[1].batch(tick);
        sim_a.step(&batch_a);
        sim_b.step(&batch_b);
        tick += 1;
    }
    report.compared_through = tick;
    if let Some((t, digests, sims)) = first {
        report.divergence = Some(Divergence {
            tick: t,
            detected_at,
            sections: section_diffs(&digests, &sims, opts.max_rows),
            sections_at_detection: at_detection.unwrap_or_default(),
            commands: commands(&a.replay, &b.replay, t, tpt, opts.history_turns),
        });
        report.compared_through = report.compared_through.max(t);
    }
    Ok(report)
}

fn section_diffs(digests: &[SectionDigests; 2], sims: &[Sim; 2], max_rows: usize) -> Vec<SectionDiff> {
    let mut out = Vec::new();
    for (i, name) in SectionDigests::NAMES.iter().enumerate() {
        let (x, y) = (digests[0].values()[i], digests[1].values()[i]);
        if x == y {
            continue;
        }
        let rows = |s: &Sim| s.section_rows(name).unwrap_or_default();
        // Rows sort by entity, then by where the section lists the field.
        let mut rank: BTreeMap<String, usize> = BTreeMap::new();
        let mut merged: BTreeMap<(Option<EntityId>, usize, String), [Option<String>; 2]> = BTreeMap::new();
        for (side, sim) in sims.iter().enumerate() {
            for r in rows(sim) {
                let next = rank.len();
                let k = *rank.entry(r.field.clone()).or_insert(next);
                merged.entry((r.entity, k, r.field)).or_default()[side] = Some(r.value);
            }
        }
        let differing: Vec<RowDiff> = merged
            .into_iter()
            .filter(|(_, v)| v[0] != v[1])
            .map(|((entity, _, field), values)| RowDiff { entity, field, values: values.map(|v| v.map(clip)) })
            .collect();
        out.push(SectionDiff { section: name, digests: [x, y], differing_rows: differing.len(), rows: differing.into_iter().take(max_rows).collect() });
    }
    out
}

fn commands(a: &Replay, b: &Replay, at: Tick, tpt: u32, history: u32) -> Commands {
    let from = at.saturating_sub(Tick::from(tpt) * Tick::from(history));
    let before = |r: &Replay| -> Vec<CommandRow> {
        r.commands.iter().take_while(|l| l.tick < at).map(|l| command_row(l.tick, &l.command)).collect()
    };
    let (all_a, all_b) = (before(a), before(b));
    let window = |all: &[CommandRow]| all.iter().filter(|c| c.tick >= from).cloned().collect::<Vec<_>>();
    // Multiset difference in O(n log n): a long game must not make this quadratic.
    let only = |x: &[CommandRow], y: &[CommandRow]| -> Vec<CommandRow> {
        let mut left: BTreeMap<(Tick, u8, u32, &str), usize> = BTreeMap::new();
        for d in y {
            *left.entry((d.tick, d.player, d.seq, d.text.as_str())).or_default() += 1;
        }
        let mut out = Vec::new();
        for c in x {
            match left.get_mut(&(c.tick, c.player, c.seq, c.text.as_str())) {
                Some(n) if *n > 0 => *n -= 1,
                _ => out.push(c.clone()),
            }
        }
        out
    };
    Commands {
        from,
        to: at.saturating_sub(1),
        window: [window(&all_a), window(&all_b)],
        only_in: [only(&all_a, &all_b), only(&all_b, &all_a)],
    }
}

// ---------------------------------------------------------------------------
// Sidecars
// ---------------------------------------------------------------------------

/// Reads the lines of a sidecar (`Peer::desync_sidecar`) the report cross-checks:
/// `first differing tick: N` and `first differing sections, seat A against
/// seat B: x, y` (sections of every pair, merged, sorted).
pub fn parse_sidecar(text: &str) -> Reported {
    let mut r = Reported::default();
    for line in text.lines() {
        if let Some(t) = line.strip_prefix("first differing tick: ") {
            r.tick = t.trim().parse().ok();
        } else if line.starts_with("first differing sections, ")
            && let Some((_, list)) = line.split_once(": ")
        {
            for s in list.split(", ").map(str::trim).filter(|s| !s.is_empty() && *s != "none") {
                if !r.sections.iter().any(|x| x == s) {
                    r.sections.push(s.to_owned());
                }
            }
        }
    }
    r.sections.sort_by_key(|s| SectionDigests::NAMES.iter().position(|n| n == s).unwrap_or(usize::MAX));
    r
}

/// Whether a sidecar agrees with the re-simulation: the router's turn and the
/// sections it named are what the replays show at that turn. `None` when the
/// sidecar names no tick (the peer never found a desync) or nothing differs.
pub fn sidecar_agrees(rep: &Reported, d: &Divergence) -> Option<bool> {
    let tick = rep.tick?;
    if d.detected_at != Some(tick) {
        return Some(false);
    }
    if rep.sections.is_empty() {
        // The peer never received the other's sections: only the tick is checked.
        return None;
    }
    Some(d.sections_at_detection.iter().map(|s| s.to_string()).eq(rep.sections.iter().cloned()))
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_list<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    format!("[{}]", items.iter().map(f).collect::<Vec<_>>().join(","))
}

fn json_opt<T>(v: Option<T>, f: impl Fn(T) -> String) -> String {
    v.map_or_else(|| "null".into(), f)
}

fn command_json(c: &CommandRow) -> String {
    format!("{{\"tick\":{},\"player\":{},\"seq\":{},\"command\":{}}}", c.tick, c.player, c.seq, json_str(&c.text))
}

impl Report {
    /// Machine-readable form: one JSON object, fields in a fixed order.
    pub fn to_json(&self) -> String {
        let mut s = String::from("{");
        let _ = write!(s, "\"format\":\"sta-desync\",\"version\":1,");
        let _ = write!(s, "\"verdict\":{},", json_str(if self.diverged() { "diverged" } else { "identical" }));
        let sides: Vec<String> = (0..2)
            .map(|i| {
                format!(
                    "{{\"label\":{},\"end_tick\":{},\"commands\":{}}}",
                    json_str(&self.labels[i]),
                    self.end_ticks[i],
                    self.command_counts[i]
                )
            })
            .collect();
        let _ = write!(s, "\"sides\":[{}],", sides.join(","));
        let _ = write!(s, "\"ticks_per_turn\":{},\"compared_through_tick\":{},", self.ticks_per_turn, self.compared_through);
        let _ = write!(s, "\"setup_differences\":{},", json_list(&self.setup_differences, |d| json_str(d)));
        match &self.divergence {
            None => {
                let _ = write!(s, "\"first_differing_tick\":null,\"detected_at_tick\":null,\"sections\":[],\"commands\":null");
            }
            Some(d) => {
                let _ = write!(
                    s,
                    "\"first_differing_tick\":{},\"detected_at_tick\":{},\"sections_at_detection\":{},",
                    d.tick,
                    json_opt(d.detected_at, |t| t.to_string()),
                    json_list(&d.sections_at_detection, |n| json_str(n))
                );
                let sections = json_list(&d.sections, |x| {
                    let rows = json_list(&x.rows, |r| {
                        format!(
                            "{{\"entity\":{},\"field\":{},\"a\":{},\"b\":{}}}",
                            json_opt(r.entity, |e| e.0.to_string()),
                            json_str(&r.field),
                            json_opt(r.values[0].as_deref(), json_str),
                            json_opt(r.values[1].as_deref(), json_str)
                        )
                    });
                    format!(
                        "{{\"section\":{},\"digest_a\":\"{:#018x}\",\"digest_b\":\"{:#018x}\",\"differing_rows\":{},\"rows\":{}}}",
                        json_str(x.section),
                        x.digests[0],
                        x.digests[1],
                        x.differing_rows,
                        rows
                    )
                });
                let _ = write!(s, "\"sections\":{sections},");
                let c = &d.commands;
                let _ = write!(
                    s,
                    "\"commands\":{{\"window_from_tick\":{},\"window_to_tick\":{},\"a\":{},\"b\":{},\"only_in_a\":{},\"only_in_b\":{}}}",
                    c.from,
                    c.to,
                    json_list(&c.window[0], command_json),
                    json_list(&c.window[1], command_json),
                    json_list(&c.only_in[0], command_json),
                    json_list(&c.only_in[1], command_json)
                );
            }
        }
        let sidecars: Vec<String> = (0..2)
            .map(|i| match &self.reported[i] {
                None => "null".to_owned(),
                Some(r) => format!(
                    "{{\"tick\":{},\"sections\":{},\"agrees\":{}}}",
                    json_opt(r.tick, |t| t.to_string()),
                    json_list(&r.sections, |n| json_str(n)),
                    json_opt(self.divergence.as_ref().and_then(|d| sidecar_agrees(r, d)), |b| b.to_string())
                ),
            })
            .collect();
        let _ = write!(s, ",\"sidecars\":[{}]", sidecars.join(","));
        s.push('}');
        s
    }

    /// A short Markdown summary for people.
    pub fn to_markdown(&self) -> String {
        let mut s = String::from("# Desync diagnosis\n\n");
        let [la, lb] = &self.labels;
        let _ = writeln!(s, "- a: `{la}` (ends at tick {}, {} commands)", self.end_ticks[0], self.command_counts[0]);
        let _ = writeln!(s, "- b: `{lb}` (ends at tick {}, {} commands)", self.end_ticks[1], self.command_counts[1]);
        if !self.setup_differences.is_empty() {
            let _ = writeln!(s, "- the setups differ in: {}", self.setup_differences.join(", "));
        }
        let Some(d) = &self.divergence else {
            let _ = writeln!(s, "\n**No divergence**: both replays give the same state on every tick through {}.", self.compared_through);
            return s;
        };
        let _ = writeln!(s, "\n**Diverged at tick {}** (the state before that tick's commands).", d.tick);
        if let Some(t) = d.detected_at {
            let _ = writeln!(
                s,
                "A router sees it at the turn boundary, tick {t}; the sections that differ there: {}.",
                if d.sections_at_detection.is_empty() { "none".to_owned() } else { d.sections_at_detection.join(", ") }
            );
        }
        for x in &d.sections {
            let _ = writeln!(s, "\n## Section `{}` ({} differing rows)\n", x.section, x.differing_rows);
            let _ = writeln!(s, "| entity | field | a | b |\n|---|---|---|---|");
            for r in &x.rows {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} |",
                    r.entity.map_or_else(|| "-".into(), |e| e.0.to_string()),
                    r.field,
                    r.values[0].as_deref().unwrap_or("(absent)"),
                    r.values[1].as_deref().unwrap_or("(absent)")
                );
            }
            if x.rows.len() < x.differing_rows {
                let _ = writeln!(s, "\n... {} more rows (see the JSON, or raise --max-rows)", x.differing_rows - x.rows.len());
            }
        }
        let c = &d.commands;
        let _ = writeln!(s, "\n## Commands, ticks {} to {}\n", c.from, c.to);
        for (i, label) in ["a", "b"].iter().enumerate() {
            let _ = writeln!(s, "{label}:");
            if c.window[i].is_empty() {
                let _ = writeln!(s, "- (none)");
            }
            for r in &c.window[i] {
                let _ = writeln!(s, "- tick {} player {} seq {}: `{}`", r.tick, r.player, r.seq, r.text);
            }
        }
        for (i, label) in ["a", "b"].iter().enumerate() {
            if let Some(r) = c.only_in[i].first() {
                let _ = writeln!(
                    s,
                    "\nOnly {label} applied (earliest of {}): tick {} player {} seq {}: `{}`",
                    c.only_in[i].len(),
                    r.tick,
                    r.player,
                    r.seq,
                    r.text
                );
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_lines_are_read_back() {
        let text = "STA64 desync report\nfirst differing tick: 93\n\
                    first differing sections, seat 0 against seat 1: combat, movement\n\
                    first differing sections, seat 0 against seat 2: movement\n";
        let r = parse_sidecar(text);
        assert_eq!(r.tick, Some(93));
        assert_eq!(r.sections, ["movement", "combat"]);
        assert_eq!(parse_sidecar("nothing here"), Reported::default());
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_str("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
    }

    #[test]
    fn long_text_is_clipped_on_a_char_boundary() {
        let s = clip("é".repeat(MAX_TEXT));
        assert!(s.ends_with("...") && s.len() < MAX_TEXT + 4);
    }
}
