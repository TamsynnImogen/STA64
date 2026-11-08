//! Scenarios: Refit's plain-text markdown form (`testbench/scenarios/README.md`),
//! the subset TESTBENCH keeps ("Scenarios") and its new steps.
//!
//! A file is a `# Title`, `Key: value` header lines, prose as description, and
//! one step per numbered or bulleted line, with `## ` headings naming phases.
//! Quotes may be straight or curly; a leading `Then`/`And` and a trailing full
//! stop are ignored; a step may start with a player (`Host: Click "Go"`).
//!
//! Each step becomes one of three things:
//! - a typed step the runner executes ([`StepKind`] other than the two below);
//! - [`StepKind::Dropped`]: a Refit step STA64 has no use for (the runner owns
//!   the process; there is no OS-level input or OCR), which the runner skips;
//! - [`StepKind::Unrecognised`]: anything else, kept with its line so the
//!   runner can fail or skip it knowingly rather than lose it.
//!
//! A malformed `Scene "..."` command inside a recognised step is an error, as
//! it is for the command parser on its own.

use super::command::{TimedCommand, parse_command, parse_command_list};
use super::error::ParseError;
use super::value::{TimeSpec, number};

/// A parsed scenario.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scenario {
    /// The first `# ` heading; empty when there is none.
    pub title: String,
    pub headers: Headers,
    /// Prose outside headers and steps.
    pub description: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Headers {
    /// `Scene: firing Seed=7 Pause=200`, or Refit's
    /// `Setup: testbench/scene/bench-setup.sh firing Seed=7 Pause=200`.
    pub scene: Option<SceneRef>,
    /// `Mod:`; STA64 passes it as `--mod`.
    pub mod_name: Option<String>,
    /// `Timeout: 12 min`.
    pub timeout_secs: Option<u64>,
    /// `Clock: bench|wall`; the runner defaults to bench.
    pub clock: Option<Clock>,
    /// `Size: 1280x720` (tier 2 only).
    pub size: Option<(u32, u32)>,
    /// Every other header (`Launch`, `Resolutions`, `Overlay`, `Players`, a
    /// `Setup:` script that is not the scene's, ...), in file order.
    pub other: Vec<Header>,
}

/// A scene and the `Key=value` words written after its name, which go into
/// its `[Scene]` section ahead of the file's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneRef {
    pub name: String,
    pub overrides: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    Bench,
    Wall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub key: String,
    pub value: String,
    pub line: usize,
}

/// One step with where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// 1-based line of the step in its file.
    pub line: usize,
    /// The step as normalised: no list marker, player, `Then`/`And` or full stop.
    pub text: String,
    /// The latest `## ` heading above it.
    pub phase: Option<String>,
    /// `Host: ...`: the player the step is for. Refit-only until M5.
    pub player: Option<String>,
    /// Set on steps that came from an `Include`d fragment: its name.
    pub fragment: Option<String>,
    pub kind: StepKind,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum StepKind {
    /// `Scene "orbit ship 90 20 100; pause"`.
    Scene {
        commands: Vec<TimedCommand>,
    },
    /// `Expect scene "tick" answers "paused"` / `... does not answer "..."`.
    ExpectSceneAnswers {
        command: TimedCommand,
        text: String,
        negated: bool,
    },
    /// `Run 200 ticks` / `Run 3.2 seconds`: sim time, never wall clock.
    Run(TimeSpec),
    /// Refit's `Wait N seconds`. The runner decides whether that is sim
    /// seconds (the bench clock) or nothing; kept apart from `Run` so a
    /// ported Refit scenario is not silently reinterpreted.
    WaitSeconds(f64),
    /// `Wait until <condition>` / `Wait up to N seconds until <condition>`.
    WaitUntil {
        within: Option<TimeSpec>,
        condition: Condition,
    },
    /// `Expect 3 ships on team 1`, `Expect no ship is on team 2`.
    Expect(Condition),
    /// `Select the ships of class "Galaxy"`.
    SelectClass {
        noun: Noun,
        class: String,
    },
    /// `Order "shooter" to attack "target"`, `Order the selection to attack "target"`.
    OrderAttack {
        who: Who,
        target: String,
    },
    /// `Take a screenshot called "NAME"`.
    Screenshot(String),
    /// `Dump the frame called "NAME"`.
    DumpFrame(String),
    /// `Expect shots "A" and "B" to match [within X]`.
    ShotsMatch {
        a: String,
        b: String,
        within: Option<f64>,
    },
    ExpectNoCrash,
    /// `Expect digest at tick 200 is "<hex>"`.
    ExpectDigest {
        at: TimeSpec,
        hex: String,
    },
    /// `Expect digest at tick 200 to match run "<scenario>"`.
    ExpectDigestMatches {
        at: TimeSpec,
        scenario: String,
    },
    /// `Expect mark "fire.start obj=shooter" by tick 90`.
    ExpectMark {
        mark: String,
        by: TimeSpec,
    },
    /// `Expect "bench.log" contains "TEXT"` / `... not contain "TEXT"`.
    ExpectLog {
        log: String,
        text: String,
        negated: bool,
    },
    /// `Note "TEXT"`.
    Note(String),
    /// `Include "_fragment"`: expanded by [`Scenario::parse_with`], else kept.
    Include(String),
    /// A Refit step with no counterpart here. The runner ignores it.
    Dropped(DroppedKind),
    /// A step that matched nothing above.
    Unrecognised,
}

/// Why a Refit step is dropped (TESTBENCH, "Scenarios", "Dropped").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DroppedKind {
    /// `Launch`, `Quit`, `Stop the game`, `Expect the game is still running`:
    /// the runner owns the process.
    Process,
    /// `Click`, `Drag`, `Press`, `Type`, `Move the mouse`, focus and bar steps.
    Input,
    /// OCR: `Wait for "TEXT"`, `Wait for the main menu`, `Expect "TEXT" is visible`.
    Ocr,
    /// Waits on a Refit plugin's log (`... for "Scene.log" to say "..."`):
    /// sim-side logs are written synchronously, so there is nothing to wait for.
    LogWait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Noun {
    Ship,
    Station,
    Planet,
    Object,
}

/// A check on how many objects of a kind a team has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Condition {
    pub count: u32,
    pub noun: Noun,
    pub team: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Who {
    Selection,
    Name(String),
}

impl Scenario {
    /// Parse a scenario. `Include` steps are kept as they are.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        parse_file(text, None, 0, None)
    }

    /// Parse a scenario, replacing each `Include "name"` by the steps of the
    /// fragment `load("name")` returns (its title and headers are ignored).
    /// A fragment `load` does not have is an error.
    pub fn parse_with(
        text: &str,
        load: &mut dyn FnMut(&str) -> Option<String>,
    ) -> Result<Self, ParseError> {
        parse_file(text, Some(load), 0, None)
    }

    /// Steps by kind, for the sweep report: (typed, dropped, unrecognised).
    pub fn tally(&self) -> (usize, usize, usize) {
        let mut t = (0, 0, 0);
        for s in &self.steps {
            match s.kind {
                StepKind::Dropped(_) => t.1 += 1,
                StepKind::Unrecognised => t.2 += 1,
                _ => t.0 += 1,
            }
        }
        t
    }
}

const MAX_INCLUDE_DEPTH: usize = 8;

const HEADER_KEYS: [&str; 16] = [
    "Tier",
    "Scene",
    "Size",
    "Mod",
    "Timeout",
    "Clock",
    "Setup",
    "Resolutions",
    "Launch",
    "Reference",
    "Stock shell",
    "Assets",
    "Players",
    "Overlay",
    "Sheet",
    "Layers",
];

/// Fetches the text of an included fragment by name.
type Loader<'a> = &'a mut dyn FnMut(&str) -> Option<String>;

fn parse_file(
    text: &str,
    mut load: Option<Loader>,
    depth: usize,
    fragment: Option<&str>,
) -> Result<Scenario, ParseError> {
    let mut sc = Scenario::default();
    let mut prose: Vec<&str> = Vec::new();
    let mut phase: Option<String> = None;
    let mut in_steps = false;
    let mut seen_title = false;

    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        if let Some(h) = line.strip_prefix("## ") {
            phase = Some(h.trim().to_owned());
            continue;
        }
        if let Some(h) = line.strip_prefix("# ") {
            if !seen_title && !in_steps {
                sc.title = h.trim().to_owned();
                seen_title = true;
            }
            continue;
        }
        if let Some(body) = list_item(line) {
            in_steps = true;
            let step = parse_step(body, n, phase.clone(), fragment).map_err(|e| e.at_line(n))?;
            if let StepKind::Include(name) = &step.kind
                && let Some(load) = load.as_deref_mut()
            {
                if depth >= MAX_INCLUDE_DEPTH {
                    return Err(ParseError::new(
                        n,
                        format!("Include \"{name}\" nests too deep"),
                    ));
                }
                let frag = load(name).ok_or_else(|| {
                    ParseError::new(n, format!("Include \"{name}\": no such fragment"))
                })?;
                let inner = parse_file(&frag, Some(load), depth + 1, Some(name))
                    .map_err(|e| ParseError::new(n, format!("in fragment \"{name}\": {e}")))?;
                sc.steps.extend(inner.steps.into_iter().map(|mut s| {
                    s.fragment.get_or_insert_with(|| name.clone());
                    s
                }));
                continue;
            }
            sc.steps.push(step);
            continue;
        }
        if !in_steps {
            if let Some((key, value)) = header(line) {
                apply_header(&mut sc.headers, key, value, n)?;
                continue;
            }
            prose.push(raw);
        }
    }
    sc.description = prose.join("\n").trim().to_owned();
    Ok(sc)
}

fn list_item(line: &str) -> Option<&str> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let rest = if digits > 0 {
        line[digits..].strip_prefix(['.', ')'])?
    } else {
        line.strip_prefix(['-', '*'])?
    };
    rest.strip_prefix([' ', '\t'])
        .map(str::trim)
        .filter(|r| !r.is_empty())
}

fn header(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim();
    HEADER_KEYS
        .iter()
        .find(|k| k.eq_ignore_ascii_case(key))
        .map(|k| (*k, value.trim()))
}

fn apply_header(h: &mut Headers, key: &str, value: &str, line: usize) -> Result<(), ParseError> {
    let err = |m: String| ParseError::new(line, format!("`{key}: {value}`: {m}"));
    match key {
        "Scene" => {
            if h.scene.is_some() {
                return Err(err("a second scene".into()));
            }
            h.scene = Some(scene_ref(value).ok_or_else(|| err("expected a scene name".into()))?);
        }
        "Setup" => {
            let mut words = value.split_whitespace();
            let script = words.next().unwrap_or("");
            if script.rsplit('/').next() == Some("bench-setup.sh") && h.scene.is_none() {
                let rest = words.collect::<Vec<_>>().join(" ");
                h.scene = Some(
                    scene_ref(&rest)
                        .ok_or_else(|| err("bench-setup.sh needs a scene name".into()))?,
                );
            } else {
                h.other.push(Header {
                    key: key.into(),
                    value: value.into(),
                    line,
                });
            }
        }
        "Mod" => h.mod_name = Some(value.to_owned()),
        "Timeout" => {
            h.timeout_secs =
                Some(timeout(value).ok_or_else(|| err("expected e.g. `12 min` or `90 s`".into()))?)
        }
        "Clock" => {
            h.clock = Some(match value.to_ascii_lowercase().as_str() {
                "bench" => Clock::Bench,
                "wall" => Clock::Wall,
                _ => return Err(err("expected bench or wall".into())),
            });
        }
        "Size" => {
            let (w, hh) = value
                .split_once(['x', 'X'])
                .ok_or_else(|| err("expected WxH".into()))?;
            let dim = |s: &str| s.trim().parse::<u32>().ok().filter(|n| *n > 0);
            h.size = Some((
                dim(w).ok_or_else(|| err("expected WxH".into()))?,
                dim(hh).ok_or_else(|| err("expected WxH".into()))?,
            ));
        }
        _ => {
            // Strip a parenthesised comment, as Refit's own examples carry them.
            let value = value.split_once("  (").map_or(value, |(v, _)| v.trim_end());
            h.other.push(Header {
                key: key.into(),
                value: value.into(),
                line,
            });
        }
    }
    Ok(())
}

fn scene_ref(value: &str) -> Option<SceneRef> {
    let mut words = value.split_whitespace();
    let name = words.next().filter(|n| !n.contains('='))?.to_owned();
    let overrides = words
        .filter_map(|w| w.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    Some(SceneRef { name, overrides })
}

fn timeout(value: &str) -> Option<u64> {
    let value = value.split_once("  (").map_or(value, |(v, _)| v);
    let mut it = value.split_whitespace();
    let n = number(it.next()?).filter(|n| *n >= 0.0)?;
    let mult = match it.next().unwrap_or("s").to_ascii_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
        "h" | "hour" | "hours" => 3600.0,
        _ => return None,
    };
    it.next().is_none().then_some((n * mult).round() as u64)
}

// ---- steps ----------------------------------------------------------------

/// Straight quotes, no `Then`/`And`, no trailing full stop.
fn normalise(body: &str) -> String {
    let mut s: String = body
        .chars()
        .map(|c| match c {
            '\u{201c}' | '\u{201d}' | '\u{201e}' => '"',
            '\u{2018}' | '\u{2019}' => '\'',
            c => c,
        })
        .collect();
    s = s.trim().to_owned();
    for lead in ["then ", "and "] {
        if s.len() > lead.len() && s[..lead.len()].eq_ignore_ascii_case(lead) {
            s = s[lead.len()..].trim_start().to_owned();
            break;
        }
    }
    while s.ends_with('.') && !s.ends_with("..") {
        s.pop();
    }
    s
}

/// `Host: Click "Go"` -> (`Host`, `Click "Go"`).
fn split_player(s: &str) -> (Option<&str>, &str) {
    if let Some((word, rest)) = s.split_once(": ") {
        let mut chars = word.chars();
        if chars.next().is_some_and(|c| c.is_ascii_uppercase())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return (Some(word), rest.trim());
        }
    }
    (None, s)
}

fn parse_step(
    body: &str,
    line: usize,
    phase: Option<String>,
    fragment: Option<&str>,
) -> Result<Step, ParseError> {
    let norm = normalise(body);
    let (player, text) = split_player(&norm);
    let kind = parse_kind(text)?;
    Ok(Step {
        line,
        text: text.to_owned(),
        phase,
        player: player.map(str::to_owned),
        fragment: fragment.map(str::to_owned),
        kind,
    })
}

/// A cursor over a step's words, for the handful of shapes the grammar has.
struct Cur<'a>(&'a str);

impl<'a> Cur<'a> {
    fn skip(&mut self) {
        self.0 = self.0.trim_start();
    }

    /// Eat the given words, case-insensitively, if they come next.
    fn eat(&mut self, words: &str) -> bool {
        let mut rest = self.0;
        for w in words.split(' ') {
            rest = rest.trim_start();
            match rest.get(..w.len()) {
                Some(head)
                    if head.eq_ignore_ascii_case(w)
                        && rest[w.len()..]
                            .chars()
                            .next()
                            .is_none_or(|c| !c.is_alphanumeric()) =>
                {
                    rest = &rest[w.len()..];
                }
                _ => return false,
            }
        }
        self.0 = rest;
        true
    }

    fn quoted(&mut self) -> Option<String> {
        let rest = self.0.trim_start().strip_prefix('"')?;
        let (inner, after) = rest.split_once('"')?;
        self.0 = after;
        Some(inner.to_owned())
    }

    /// The next whitespace-delimited word.
    fn word(&mut self) -> Option<&'a str> {
        self.skip();
        let end = self.0.find(char::is_whitespace).unwrap_or(self.0.len());
        (end > 0).then(|| {
            let (w, rest) = self.0.split_at(end);
            self.0 = rest;
            w
        })
    }

    fn number(&mut self) -> Option<f64> {
        let save = self.0;
        let n = self.word().and_then(number);
        if n.is_none() {
            self.0 = save;
        }
        n
    }

    fn done(&self) -> bool {
        self.0.trim().is_empty()
    }

    fn starts(&self, words: &str) -> bool {
        Cur(self.0).eat(words)
    }
}

fn parse_kind(text: &str) -> Result<StepKind, ParseError> {
    Ok(typed(text)?
        .unwrap_or_else(|| dropped(text).map_or(StepKind::Unrecognised, StepKind::Dropped)))
}

fn typed(text: &str) -> Result<Option<StepKind>, ParseError> {
    let mut c = Cur(text);
    let command = |s: &str| {
        parse_command(s).and_then(|c| c.ok_or_else(|| ParseError::new(1, "empty command")))
    };

    if c.eat("scene") {
        let Some(cmds) = c.quoted().filter(|_| c.done()) else {
            return Ok(None);
        };
        return Ok(Some(StepKind::Scene {
            commands: parse_command_list(&cmds)?,
        }));
    }
    if c.eat("expect scene") {
        let Some(cmd) = c.quoted() else {
            return Ok(None);
        };
        let negated = if c.eat("answers") || c.eat("answer") {
            false
        } else if c.eat("does not answer") || c.eat("not answer") || c.eat("not answers") {
            true
        } else {
            return Ok(None);
        };
        let Some(answer) = c.quoted().filter(|_| c.done()) else {
            return Ok(None);
        };
        return Ok(Some(StepKind::ExpectSceneAnswers {
            command: command(&cmd)?,
            text: answer,
            negated,
        }));
    }
    if c.eat("run") {
        let Some(n) = c.number() else { return Ok(None) };
        let unit = c.word().unwrap_or("").to_ascii_lowercase();
        if !c.done() || n < 0.0 {
            return Ok(None);
        }
        return Ok(match unit.as_str() {
            "tick" | "ticks" if n.fract() == 0.0 => Some(StepKind::Run(TimeSpec::Ticks(n as u64))),
            "second" | "seconds" | "s" => Some(StepKind::Run(TimeSpec::Seconds(n))),
            _ => None,
        });
    }
    if c.eat("wait until") {
        let Some(condition) = condition(&mut c) else {
            return Ok(None);
        };
        return Ok(Some(StepKind::WaitUntil {
            within: None,
            condition,
        }));
    }
    if c.eat("wait up to") {
        let Some(n) = c.number() else { return Ok(None) };
        let within = match c.word().map(str::to_ascii_lowercase).as_deref() {
            Some("seconds" | "second") => TimeSpec::Seconds(n),
            Some("ticks" | "tick") if n.fract() == 0.0 && n >= 0.0 => TimeSpec::Ticks(n as u64),
            _ => return Ok(None),
        };
        if !c.eat("until") {
            return Ok(None);
        }
        let Some(condition) = condition(&mut c) else {
            return Ok(None);
        };
        return Ok(Some(StepKind::WaitUntil {
            within: Some(within),
            condition,
        }));
    }
    if c.eat("wait") {
        let Some(n) = c.number().filter(|n| *n >= 0.0) else {
            return Ok(None);
        };
        let unit = c.word().unwrap_or("").to_ascii_lowercase();
        return Ok((c.done() && matches!(unit.as_str(), "second" | "seconds"))
            .then_some(StepKind::WaitSeconds(n)));
    }
    if c.eat("take a screenshot called") {
        return Ok(c.quoted().filter(|_| c.done()).map(StepKind::Screenshot));
    }
    if c.eat("dump the frame called") {
        return Ok(c.quoted().filter(|_| c.done()).map(StepKind::DumpFrame));
    }
    if c.eat("note") {
        return Ok(c.quoted().filter(|_| c.done()).map(StepKind::Note));
    }
    if c.eat("include") {
        return Ok(c.quoted().filter(|_| c.done()).map(StepKind::Include));
    }
    if c.eat("select the") {
        let Some(noun) = c.word().and_then(noun) else {
            return Ok(None);
        };
        if !c.eat("of class") {
            return Ok(None);
        }
        return Ok(c
            .quoted()
            .filter(|_| c.done())
            .map(|class| StepKind::SelectClass { noun, class }));
    }
    if c.eat("order") {
        let who = if c.eat("the selection") {
            Who::Selection
        } else if let Some(q) = c.quoted() {
            Who::Name(q)
        } else if let Some(w) = c.word().filter(|w| !w.eq_ignore_ascii_case("to")) {
            Who::Name(w.to_owned())
        } else {
            return Ok(None);
        };
        if !c.eat("to attack") {
            return Ok(None);
        }
        let target = c.quoted().or_else(|| c.word().map(str::to_owned));
        return Ok(target
            .filter(|_| c.done())
            .map(|target| StepKind::OrderAttack { who, target }));
    }
    if c.eat("expect") {
        return expect(&mut c);
    }
    Ok(None)
}

fn expect(c: &mut Cur) -> Result<Option<StepKind>, ParseError> {
    if c.eat("no crash") {
        return Ok(c.done().then_some(StepKind::ExpectNoCrash));
    }
    if c.eat("shots") {
        let Some(a) = c.quoted() else { return Ok(None) };
        if !c.eat("and") {
            return Ok(None);
        }
        let Some(b) = c.quoted() else { return Ok(None) };
        if !c.eat("to match") {
            return Ok(None);
        }
        let within = if c.eat("within") {
            match c.number() {
                Some(x) => Some(x),
                None => return Ok(None),
            }
        } else {
            None
        };
        return Ok(c.done().then_some(StepKind::ShotsMatch { a, b, within }));
    }
    if c.eat("digest at tick") {
        let Some(at) = c
            .word()
            .and_then(|w| TimeSpec::parse(w).filter(|t| matches!(t, TimeSpec::Ticks(_))))
        else {
            return Ok(None);
        };
        if c.eat("is") {
            return Ok(c
                .quoted()
                .filter(|_| c.done())
                .map(|hex| StepKind::ExpectDigest { at, hex }));
        }
        if c.eat("to match run") {
            return Ok(c
                .quoted()
                .filter(|_| c.done())
                .map(|scenario| StepKind::ExpectDigestMatches { at, scenario }));
        }
        return Ok(None);
    }
    if c.eat("mark") {
        let Some(mark) = c.quoted() else {
            return Ok(None);
        };
        if !c.eat("by tick") {
            return Ok(None);
        }
        let Some(by) = c
            .word()
            .and_then(|w| TimeSpec::parse(w).filter(|t| matches!(t, TimeSpec::Ticks(_))))
        else {
            return Ok(None);
        };
        return Ok(c.done().then_some(StepKind::ExpectMark { mark, by }));
    }
    if c.0.trim_start().starts_with('"') {
        let Some(log) = c.quoted() else {
            return Ok(None);
        };
        let negated = if c.eat("contains") || c.eat("contain") {
            false
        } else if c.eat("does not contain") || c.eat("not contain") || c.eat("not contains") {
            true
        } else {
            return Ok(None);
        };
        return Ok(c
            .quoted()
            .filter(|_| c.done())
            .map(|text| StepKind::ExpectLog { log, text, negated }));
    }
    Ok(condition(c).filter(|_| c.done()).map(StepKind::Expect))
}

fn noun(w: &str) -> Option<Noun> {
    Some(match w.to_ascii_lowercase().as_str() {
        "ship" | "ships" => Noun::Ship,
        "station" | "stations" => Noun::Station,
        "planet" | "planets" => Noun::Planet,
        "object" | "objects" => Noun::Object,
        _ => return None,
    })
}

/// `3 ships on team 1`, `no ship is on team 2`, `no ships on team 2`.
fn condition(c: &mut Cur) -> Option<Condition> {
    let count = if c.eat("no") {
        0
    } else {
        let n = c
            .number()
            .filter(|n| *n >= 0.0 && n.fract() == 0.0 && *n <= f64::from(u32::MAX))?;
        n as u32
    };
    let noun = c.word().and_then(noun)?;
    let _ = c.eat("is") || c.eat("are");
    if !c.eat("on team") {
        return None;
    }
    let t = c
        .number()
        .filter(|t| t.fract() == 0.0 && t.abs() <= f64::from(i32::MAX))?;
    Some(Condition {
        count,
        noun,
        team: t as i32,
    })
}

/// Refit steps with no use here, by family.
fn dropped(text: &str) -> Option<DroppedKind> {
    let c = Cur(text);
    let any = |ws: &[&str]| ws.iter().any(|w| c.starts(w));
    if any(&[
        "launch the game",
        "quit the game",
        "stop the game",
        "expect the game is still running",
        "expect the game to have exited",
    ]) {
        return Some(DroppedKind::Process);
    }
    if any(&[
        "click",
        "right-click",
        "double-click",
        "drag",
        "right-drag",
        "press",
        "type",
        "move the mouse",
        "hover over",
        "take the focus",
        "show a bar",
        "hide the bar",
    ]) {
        return Some(DroppedKind::Input);
    }
    if c.starts("wait up to") || c.starts("wait for") {
        // A log wait names a log and says what it says, or waits for the logs.
        let log = text.contains(" to say ") || text.contains("the logs the next steps expect");
        return Some(if log {
            DroppedKind::LogWait
        } else {
            DroppedKind::Ocr
        });
    }
    if c.starts("expect")
        && text[6..].trim_start().starts_with('"')
        && (text.ends_with(" is visible") || text.ends_with(" is not visible"))
    {
        return Some(DroppedKind::Ocr);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::BenchCommand;

    fn kind(s: &str) -> StepKind {
        parse_step(s, 1, None, None).expect("parses").kind
    }

    #[test]
    fn scene_steps() {
        let StepKind::Scene { commands } = kind("Scene \"pause; orbit ship 90 20 100\".") else {
            panic!()
        };
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].command, BenchCommand::Pause);
        let StepKind::ExpectSceneAnswers {
            command,
            text,
            negated,
        } = kind("Expect scene \"selection\" answers \"selected 20\"")
        else {
            panic!()
        };
        assert_eq!(
            (command.command, text.as_str(), negated),
            (BenchCommand::Selection, "selected 20", false)
        );
        assert!(matches!(
            kind("Expect scene \"tick\" does not answer \"paused\""),
            StepKind::ExpectSceneAnswers { negated: true, .. }
        ));
        assert!(matches!(
            kind("Expect scene \"tick\" not answer \"paused\""),
            StepKind::ExpectSceneAnswers { negated: true, .. }
        ));
    }

    #[test]
    fn time_steps() {
        assert_eq!(kind("Run 200 ticks"), StepKind::Run(TimeSpec::Ticks(200)));
        assert_eq!(kind("Run 1 tick"), StepKind::Run(TimeSpec::Ticks(1)));
        assert_eq!(
            kind("run 3.2 seconds"),
            StepKind::Run(TimeSpec::Seconds(3.2))
        );
        assert_eq!(kind("Run 1.5 ticks"), StepKind::Unrecognised);
        assert_eq!(kind("Wait 5 seconds"), StepKind::WaitSeconds(5.0));
        assert_eq!(kind("Wait 1 second"), StepKind::WaitSeconds(1.0));
    }

    #[test]
    fn object_phrases() {
        let c = |count, noun, team| Condition { count, noun, team };
        assert_eq!(
            kind("Expect 3 ships on team 1"),
            StepKind::Expect(c(3, Noun::Ship, 1))
        );
        assert_eq!(
            kind("Expect no ship is on team 2"),
            StepKind::Expect(c(0, Noun::Ship, 2))
        );
        assert_eq!(
            kind("Expect 1 station on team 0"),
            StepKind::Expect(c(1, Noun::Station, 0))
        );
        assert_eq!(
            kind("Wait until no ship is on team 2"),
            StepKind::WaitUntil {
                within: None,
                condition: c(0, Noun::Ship, 2)
            }
        );
        assert_eq!(
            kind("Wait up to 20 seconds until no ships are on team 2"),
            StepKind::WaitUntil {
                within: Some(TimeSpec::Seconds(20.0)),
                condition: c(0, Noun::Ship, 2)
            }
        );
        assert_eq!(
            kind("Select the ships of class \"Galaxy\""),
            StepKind::SelectClass {
                noun: Noun::Ship,
                class: "Galaxy".into()
            }
        );
        assert_eq!(
            kind("Order the selection to attack \"target\""),
            StepKind::OrderAttack {
                who: Who::Selection,
                target: "target".into()
            }
        );
        assert_eq!(
            kind("Order shooter to attack target"),
            StepKind::OrderAttack {
                who: Who::Name("shooter".into()),
                target: "target".into()
            }
        );
    }

    #[test]
    fn frames_digests_marks_logs() {
        assert_eq!(
            kind("Take a screenshot called \"firing\""),
            StepKind::Screenshot("firing".into())
        );
        assert_eq!(
            kind("Dump the frame called \"firing\""),
            StepKind::DumpFrame("firing".into())
        );
        assert_eq!(
            kind("Expect shots \"first\" and \"first again\" to match within 0.01"),
            StepKind::ShotsMatch {
                a: "first".into(),
                b: "first again".into(),
                within: Some(0.01)
            }
        );
        assert_eq!(
            kind("Expect shots \"a\" and \"b\" to match"),
            StepKind::ShotsMatch {
                a: "a".into(),
                b: "b".into(),
                within: None
            }
        );
        assert_eq!(kind("Expect no crash"), StepKind::ExpectNoCrash);
        assert_eq!(
            kind("Expect digest at tick 200 is \"00ff\""),
            StepKind::ExpectDigest {
                at: TimeSpec::Ticks(200),
                hex: "00ff".into()
            }
        );
        assert_eq!(
            kind("Expect digest at tick 200 to match run \"planet\""),
            StepKind::ExpectDigestMatches {
                at: TimeSpec::Ticks(200),
                scenario: "planet".into()
            }
        );
        assert_eq!(
            kind("Expect mark \"fire.start obj=shooter\" by tick 90"),
            StepKind::ExpectMark {
                mark: "fire.start obj=shooter".into(),
                by: TimeSpec::Ticks(90)
            }
        );
        assert_eq!(
            kind("Expect \"bench.log\" contains \"scene ready\""),
            StepKind::ExpectLog {
                log: "bench.log".into(),
                text: "scene ready".into(),
                negated: false
            }
        );
        assert_eq!(
            kind("Expect \"bench.log\" not contain \"error\""),
            StepKind::ExpectLog {
                log: "bench.log".into(),
                text: "error".into(),
                negated: true
            }
        );
        assert_eq!(kind("Note \"hello\""), StepKind::Note("hello".into()));
        assert_eq!(
            kind("Include \"_enter\""),
            StepKind::Include("_enter".into())
        );
    }

    #[test]
    fn dropped_and_unrecognised() {
        use DroppedKind::*;
        for (s, k) in [
            ("Launch the game", Process),
            ("Launch the game with \"-nointro x\"", Process),
            ("Launch the game into mission \"a2_borg01\"", Process),
            ("Quit the game", Process),
            ("Stop the game", Process),
            ("Expect the game is still running", Process),
            ("Click \"Single Player\"", Input),
            ("Right-click at 100,200", Input),
            ("Press ctrl+s", Input),
            ("Type \"hi\"", Input),
            ("Move the mouse to 1,2", Input),
            ("Wait for the main menu", Ocr),
            ("Wait up to 60 seconds for the briefing", Ocr),
            ("Expect \"Fleet\" is visible", Ocr),
            (
                "Wait up to 50 seconds for the logs the next steps expect",
                LogWait,
            ),
            (
                "Wait up to 240 seconds for \"Scene.log\" to say \"paused at tick\"",
                LogWait,
            ),
        ] {
            assert_eq!(kind(s), StepKind::Dropped(k), "{s}");
        }
        for s in [
            "Check that the screenshot shows a battle",
            "Dance",
            "Run",
            "Scene pause",
            "Expect 3 frogs on team 1",
        ] {
            assert_eq!(kind(s), StepKind::Unrecognised, "{s}");
        }
    }

    #[test]
    fn normalisation_and_players() {
        let s = parse_step(
            "Then Host: Take a screenshot called \u{201c}fancy\u{201d}.",
            4,
            None,
            None,
        )
        .unwrap();
        assert_eq!(s.player.as_deref(), Some("Host"));
        assert_eq!(s.kind, StepKind::Screenshot("fancy".into()));
        assert_eq!(s.text, "Take a screenshot called \"fancy\"");
        assert!(matches!(
            kind("and Expect no crash."),
            StepKind::ExpectNoCrash
        ));
    }

    #[test]
    fn bad_commands_in_steps_are_errors_with_the_line() {
        let e = Scenario::parse("# T\n\n1. Run 5 ticks\n2. Scene \"kill\"\n").unwrap_err();
        assert_eq!(e.line, 4);
        assert!(e.message.contains("usage: kill"));
        let e = Scenario::parse("1. Expect scene \"heal x maybe\" answers \"ok\"\n").unwrap_err();
        assert_eq!(e.line, 1);
        // An unknown verb is fine: the runner answers `! unsupported`.
        assert!(Scenario::parse("1. Scene \"frobnicate\"\n").is_ok());
    }

    #[test]
    fn headers_and_structure() {
        let sc = Scenario::parse(
            "# Firing\n\nMod: refit\nClock: bench\nTimeout: 10 min\nSize: 1280x720\n\
             Setup: testbench/scene/bench-setup.sh firing Seed=7 Step=16 Pause=200\n\
             Launch: -nointro a2_borg01\nResolutions: 16:9, 4:3     (aspects)\n\n\
             Prose line one.\nProse line two.\n\n## Start\n1. Launch the game.\n2. Run 200 ticks\n\n\
             ## Check\n- Expect no crash.\n* Frobnicate the widget\n",
        )
        .unwrap();
        assert_eq!(sc.title, "Firing");
        let h = &sc.headers;
        assert_eq!(h.mod_name.as_deref(), Some("refit"));
        assert_eq!(h.clock, Some(Clock::Bench));
        assert_eq!(h.timeout_secs, Some(600));
        assert_eq!(h.size, Some((1280, 720)));
        let scene = h.scene.as_ref().unwrap();
        assert_eq!(scene.name, "firing");
        assert_eq!(
            scene.overrides,
            [
                ("Seed".into(), "7".into()),
                ("Step".into(), "16".into()),
                ("Pause".into(), "200".into())
            ]
        );
        let other: Vec<_> = h
            .other
            .iter()
            .map(|x| (x.key.as_str(), x.value.as_str()))
            .collect();
        assert_eq!(
            other,
            [
                ("Launch", "-nointro a2_borg01"),
                ("Resolutions", "16:9, 4:3")
            ]
        );
        assert_eq!(sc.description, "Prose line one.\nProse line two.");
        let steps: Vec<_> = sc
            .steps
            .iter()
            .map(|s| (s.line, s.phase.as_deref()))
            .collect();
        assert_eq!(
            steps,
            [
                (15, Some("Start")),
                (16, Some("Start")),
                (19, Some("Check")),
                (20, Some("Check"))
            ]
        );
        assert_eq!(sc.tally(), (2, 1, 1));
    }

    #[test]
    fn scene_header_and_other_setup_scripts() {
        let sc = Scenario::parse(
            "Scene: firing Seed=7\nSetup: ui/bench-asi.sh --menus\n1. Run 1 tick\n",
        )
        .unwrap();
        assert_eq!(sc.headers.scene.unwrap().name, "firing");
        assert_eq!(sc.headers.other[0].key, "Setup");
        assert!(Scenario::parse("Timeout: soon\n").is_err());
        assert_eq!(Scenario::parse("Clock: sundial\n").unwrap_err().line, 1);
    }

    #[test]
    fn includes_expand_with_a_loader() {
        let mut load = |name: &str| {
            (name == "_frag")
                .then(|| "# Frag\nIgnored: x\n1. Run 5 ticks\n2. Expect no crash\n".to_owned())
        };
        let sc = Scenario::parse_with("# Main\n1. Include \"_frag\".\n2. Run 1 tick\n", &mut load)
            .unwrap();
        assert_eq!(sc.steps.len(), 3);
        assert_eq!(sc.steps[0].kind, StepKind::Run(TimeSpec::Ticks(5)));
        assert_eq!(sc.steps[0].fragment.as_deref(), Some("_frag"));
        assert_eq!(sc.steps[2].fragment, None);
        let e = Scenario::parse_with("1. Include \"_nope\"\n", &mut load).unwrap_err();
        assert_eq!(
            (e.line, e.message.as_str()),
            (1, "Include \"_nope\": no such fragment")
        );
        // Without a loader the include stays a step.
        let sc = Scenario::parse("1. Include \"_frag\"\n").unwrap();
        assert_eq!(sc.steps[0].kind, StepKind::Include("_frag".into()));
    }
}
