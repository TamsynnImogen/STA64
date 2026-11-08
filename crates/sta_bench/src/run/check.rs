//! Checks on sim state, beyond the step kinds the scenario parser types.
//!
//! The parser leaves these as `Unrecognised`; the runner reads their text
//! here. One predicate type serves `Expect <pred>` (at a tick that has been
//! reached, default now) and `Wait [up to T] until <pred>` (state predicates,
//! now only). Grammar, keywords case-insensitive, `"..."` is a quoted name:
//!
//! ```text
//! Expect [no] mark "KIND k=v ..." (by tick N | at tick N | between ticks A and B)
//! Expect "OBJ" exists | does not exist                    [at tick N]
//! Expect "OBJ" FIELD is CMP                               [at tick N]
//! Expect "OBJ" is within R of (X Y Z | "OBJ2")            [at tick N]
//! Expect player P resource NAME is CMP                    [at tick N]
//! Expect player P is [not] eliminated                     [at tick N]
//! Expect player P sees "OBJ" | does not see "OBJ"         [at tick N]
//! Wait [up to N seconds|ticks] until <state predicate>
//! Run until tick N
//! Save snapshot as "FILE"
//!
//! FIELD: hull hull.max shield[.fore|.aft|.port|.starboard] shield.max speed x y z
//! CMP:   V [within E] | at least V | at most V | above V | below V | between A and B
//! NAME:  dilithium latinum metal biomatter (or 0..3)
//! ```
//!
//! Times are ticks (`200`) or seconds (`3.2s`). Numbers compare as the sim's
//! fixed point turned into `f64`; `is V` without `within` allows 1e-6.

use sta_core::{PlayerId, TICK_HZ, Tick};
use sta_sim::view::UnitFlags;

use super::bench::{Bench, owner_of};
use super::marks::{self, Spec};
use super::to_f64;
use crate::parse::TimeSpec;
use crate::parse::scenario::{Condition, Noun};

#[derive(Debug, Clone)]
pub enum Window {
    By(Tick),
    At(Tick),
    Between(Tick, Tick),
}

#[derive(Debug, Clone, Copy)]
pub enum Cmp {
    Is(f64, f64),
    AtLeast(f64),
    AtMost(f64),
    Above(f64),
    Below(f64),
    Between(f64, f64),
}

impl Cmp {
    fn holds(self, v: f64) -> bool {
        match self {
            Cmp::Is(x, e) => (v - x).abs() <= e,
            Cmp::AtLeast(x) => v >= x,
            Cmp::AtMost(x) => v <= x,
            Cmp::Above(x) => v > x,
            Cmp::Below(x) => v < x,
            Cmp::Between(a, b) => v >= a && v <= b,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Field {
    Hull,
    HullMax,
    Shield(usize),
    ShieldMax,
    Speed,
    X,
    Y,
    Z,
}

#[derive(Debug, Clone)]
pub enum Place {
    Point([f64; 3]),
    Obj(String),
}

#[derive(Debug, Clone)]
pub enum Pred {
    Field {
        obj: String,
        field: Field,
        cmp: Cmp,
        at: Option<Tick>,
    },
    Near {
        obj: String,
        radius: f64,
        to: Place,
        at: Option<Tick>,
    },
    Exists {
        obj: String,
        yes: bool,
        at: Option<Tick>,
    },
    Resource {
        player: u8,
        res: usize,
        cmp: Cmp,
        at: Option<Tick>,
    },
    Eliminated {
        player: u8,
        yes: bool,
        at: Option<Tick>,
    },
    Sees {
        player: u8,
        obj: String,
        yes: bool,
        at: Option<Tick>,
    },
    Mark {
        spec: Spec,
        yes: bool,
        window: Window,
    },
}

impl Pred {
    /// Whether the predicate only looks at the present (a `Wait` can poll it).
    fn is_state(&self) -> bool {
        match self {
            Pred::Field { at, .. }
            | Pred::Near { at, .. }
            | Pred::Exists { at, .. }
            | Pred::Resource { at, .. }
            | Pred::Eliminated { at, .. }
            | Pred::Sees { at, .. } => at.is_none(),
            Pred::Mark { .. } => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Extra {
    Expect(Pred),
    Wait {
        within: Option<TimeSpec>,
        pred: Pred,
    },
    RunUntil(Tick),
    SaveSnapshot(String),
}

// ---- parsing --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
}

struct Cur {
    toks: Vec<Tok>,
    i: usize,
}

fn tokens(text: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut s = String::new();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some(c) => s.push(c),
                    None => return Err("unterminated quote".into()),
                }
            }
            out.push(Tok::Quoted(s));
        } else {
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '"' {
                    break;
                }
                s.push(c);
                chars.next();
            }
            out.push(Tok::Word(s));
        }
    }
    Ok(out)
}

impl Cur {
    /// Consume the words of `phrase` if they are next.
    fn eat(&mut self, phrase: &str) -> bool {
        let words: Vec<&str> = phrase.split_whitespace().collect();
        let ok = words.iter().enumerate().all(|(k, w)| matches!(self.toks.get(self.i + k), Some(Tok::Word(t)) if t.eq_ignore_ascii_case(w)));
        if ok {
            self.i += words.len();
        }
        ok
    }

    fn quoted(&mut self) -> Result<String, String> {
        match self.toks.get(self.i) {
            Some(Tok::Quoted(s)) => {
                self.i += 1;
                Ok(s.clone())
            }
            _ => Err("expected a quoted name".into()),
        }
    }

    fn word(&mut self) -> Result<String, String> {
        match self.toks.get(self.i) {
            Some(Tok::Word(s)) => {
                self.i += 1;
                Ok(s.clone())
            }
            _ => Err("expected a word".into()),
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        let w = self.word()?;
        w.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("`{w}`: expected a number"))
    }

    fn time(&mut self) -> Result<Tick, String> {
        let w = self.word()?;
        let t = TimeSpec::parse(&w)
            .ok_or_else(|| format!("`{w}`: expected ticks (200) or seconds (3.2s)"))?;
        Ok(t.ticks(TICK_HZ))
    }

    fn done(&self) -> bool {
        self.i >= self.toks.len()
    }

    fn end(&self) -> Result<(), String> {
        if self.done() {
            Ok(())
        } else {
            Err(format!(
                "unexpected `{}`",
                self.toks[self.i..]
                    .iter()
                    .map(tok_text)
                    .collect::<Vec<_>>()
                    .join(" ")
            ))
        }
    }

    fn opt_at(&mut self) -> Result<Option<Tick>, String> {
        if self.eat("at tick") {
            self.time().map(Some)
        } else {
            Ok(None)
        }
    }

    fn cmp(&mut self) -> Result<Cmp, String> {
        Ok(if self.eat("at least") {
            Cmp::AtLeast(self.number()?)
        } else if self.eat("at most") {
            Cmp::AtMost(self.number()?)
        } else if self.eat("above") {
            Cmp::Above(self.number()?)
        } else if self.eat("below") {
            Cmp::Below(self.number()?)
        } else if self.eat("between") {
            let a = self.number()?;
            if !self.eat("and") {
                return Err("`between A and B`".into());
            }
            Cmp::Between(a, self.number()?)
        } else {
            let v = self.number()?;
            let e = if self.eat("within") {
                self.number()?
            } else {
                1e-6
            };
            Cmp::Is(v, e)
        })
    }
}

fn tok_text(t: &Tok) -> String {
    match t {
        Tok::Word(w) => w.clone(),
        Tok::Quoted(q) => format!("\"{q}\""),
    }
}

fn field(w: &str) -> Option<Field> {
    Some(match w.to_ascii_lowercase().as_str() {
        "hull" => Field::Hull,
        "hull.max" => Field::HullMax,
        "shield" | "shield.fore" => Field::Shield(0),
        "shield.aft" => Field::Shield(1),
        "shield.port" => Field::Shield(2),
        "shield.starboard" => Field::Shield(3),
        "shield.max" => Field::ShieldMax,
        "speed" => Field::Speed,
        "x" => Field::X,
        "y" => Field::Y,
        "z" => Field::Z,
        _ => return None,
    })
}

fn resource(w: &str) -> Option<usize> {
    Some(match w.to_ascii_lowercase().as_str() {
        "dilithium" => 0,
        "latinum" => 1,
        "metal" => 2,
        "biomatter" => 3,
        n => n.parse().ok().filter(|&i| i < 4)?,
    })
}

fn pred(c: &mut Cur) -> Result<Pred, String> {
    let no = c.eat("no mark");
    let p = if no || c.eat("mark") {
        let yes = !no;
        let spec = marks::parse_spec(&c.quoted()?)?;
        let window = if c.eat("by tick") {
            Window::By(c.time()?)
        } else if c.eat("at tick") {
            Window::At(c.time()?)
        } else if c.eat("between ticks") {
            let a = c.time()?;
            if !c.eat("and") {
                return Err("`between ticks A and B`".into());
            }
            Window::Between(a, c.time()?)
        } else {
            return Err("a mark needs `by tick N`, `at tick N` or `between ticks A and B`".into());
        };
        Pred::Mark { spec, yes, window }
    } else if c.eat("player") {
        let player: u8 = c
            .word()?
            .parse()
            .map_err(|_| "player: expected a number".to_string())?;
        if c.eat("resource") {
            let name = c.word()?;
            let res = resource(&name).ok_or_else(|| format!("`{name}`: not a resource"))?;
            let at1 = c.opt_at()?;
            if !c.eat("is") {
                return Err("expected `is`".into());
            }
            let cmp = c.cmp()?;
            Pred::Resource {
                player,
                res,
                cmp,
                at: at1.or(c.opt_at()?),
            }
        } else if c.eat("is not eliminated") {
            Pred::Eliminated {
                player,
                yes: false,
                at: c.opt_at()?,
            }
        } else if c.eat("is eliminated") {
            Pred::Eliminated {
                player,
                yes: true,
                at: c.opt_at()?,
            }
        } else if c.eat("sees") {
            Pred::Sees {
                player,
                obj: c.quoted()?,
                yes: true,
                at: c.opt_at()?,
            }
        } else if c.eat("does not see") {
            Pred::Sees {
                player,
                obj: c.quoted()?,
                yes: false,
                at: c.opt_at()?,
            }
        } else {
            return Err(
                "after `player N`: resource, is [not] eliminated, sees, does not see".into(),
            );
        }
    } else {
        let obj = c.quoted()?;
        if c.eat("exists") {
            Pred::Exists {
                obj,
                yes: true,
                at: c.opt_at()?,
            }
        } else if c.eat("does not exist") {
            Pred::Exists {
                obj,
                yes: false,
                at: c.opt_at()?,
            }
        } else if c.eat("is within") {
            let radius = c.number()?;
            if !c.eat("of") {
                return Err("`is within R of ...`".into());
            }
            let to = if matches!(c.toks.get(c.i), Some(Tok::Quoted(_))) {
                Place::Obj(c.quoted()?)
            } else {
                Place::Point([c.number()?, c.number()?, c.number()?])
            };
            Pred::Near {
                obj,
                radius,
                to,
                at: c.opt_at()?,
            }
        } else {
            let w = c.word()?;
            let field = field(&w).ok_or_else(|| format!("`{w}`: expected exists, is within, or a field (hull, shield.fore, speed, x, y, z ...)"))?;
            let at1 = c.opt_at()?;
            if !c.eat("is") {
                return Err("expected `is`".into());
            }
            let cmp = c.cmp()?;
            Pred::Field {
                obj,
                field,
                cmp,
                at: at1.or(c.opt_at()?),
            }
        }
    };
    Ok(p)
}

/// Read a step the scenario parser left unrecognised. `Ok(None)`: not ours.
pub fn parse(text: &str) -> Result<Option<Extra>, String> {
    let mut c = Cur {
        toks: tokens(text)?,
        i: 0,
    };
    let extra = if c.eat("expect") {
        Extra::Expect(pred(&mut c)?)
    } else if c.eat("wait") {
        let within = if c.eat("up to") {
            let w = c.word()?;
            let n = w
                .parse::<f64>()
                .map_err(|_| format!("`{w}`: expected a number"))?;
            let t = if c.eat("seconds") || c.eat("second") {
                TimeSpec::Seconds(n)
            } else if c.eat("ticks") || c.eat("tick") {
                TimeSpec::Ticks(n as Tick)
            } else {
                return Err("`up to N seconds` or `up to N ticks`".into());
            };
            Some(t)
        } else {
            None
        };
        if !c.eat("until") {
            return Ok(None);
        }
        Extra::Wait {
            within,
            pred: pred(&mut c)?,
        }
    } else if c.eat("run until tick") || c.eat("run to tick") {
        Extra::RunUntil(c.time()?)
    } else if c.eat("save snapshot as") {
        Extra::SaveSnapshot(c.quoted()?)
    } else {
        return Ok(None);
    };
    c.end()?;
    Ok(Some(extra))
}

// ---- evaluation -----------------------------------------------------------

fn unit_view<'a>(
    b: &'a Bench,
    obj: &str,
    at: Option<Tick>,
) -> Result<(&'a sta_sim::view::UnitView, Tick), String> {
    let view = match at {
        Some(t) => b.view_at(t)?,
        None => b.view(),
    };
    let id = b.resolve(obj)?;
    view.unit(id)
        .map(|u| (u, view.tick))
        .ok_or_else(|| format!("`{obj}` does not exist at tick {}", view.tick))
}

fn dist(a: [f64; 3], p: [f64; 3]) -> f64 {
    ((a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2) + (a[2] - p[2]).powi(2)).sqrt()
}

/// Pass with a detail line, or fail with one.
pub fn eval(b: &Bench, p: &Pred) -> Result<String, String> {
    match p {
        Pred::Field {
            obj,
            field,
            cmp,
            at,
        } => {
            let (u, tick) = unit_view(b, obj, *at)?;
            let v = to_f64(match field {
                Field::Hull => u.hull[0],
                Field::HullMax => u.hull[1],
                Field::Shield(i) => u.shields[*i][0],
                Field::ShieldMax => u.shields[0][1],
                Field::Speed => u.speed,
                Field::X => u.pos.x,
                Field::Y => u.pos.y,
                Field::Z => u.pos.z,
            });
            let msg = format!("`{obj}` {field:?} = {v} at tick {tick} (want {cmp:?})");
            if cmp.holds(v) { Ok(msg) } else { Err(msg) }
        }
        Pred::Near {
            obj,
            radius,
            to,
            at,
        } => {
            let (u, tick) = unit_view(b, obj, *at)?;
            let here = [to_f64(u.pos.x), to_f64(u.pos.y), to_f64(u.pos.z)];
            let there = match to {
                Place::Point(p) => *p,
                Place::Obj(n) => {
                    let id = b.resolve(n)?;
                    let view = match at {
                        Some(t) => b.view_at(*t)?,
                        None => b.view(),
                    };
                    let o = view
                        .unit(id)
                        .ok_or_else(|| format!("`{n}` does not exist at tick {}", view.tick))?;
                    [to_f64(o.pos.x), to_f64(o.pos.y), to_f64(o.pos.z)]
                }
            };
            let d = dist(here, there);
            let msg =
                format!("`{obj}` is {d:.2} from {there:?} at tick {tick} (want within {radius})");
            if d <= *radius { Ok(msg) } else { Err(msg) }
        }
        Pred::Exists { obj, yes, at } => {
            let view = match at {
                Some(t) => b.view_at(*t)?,
                None => b.view(),
            };
            let there = b.resolve(obj).is_ok_and(|id| view.unit(id).is_some());
            let msg = format!(
                "`{obj}` {} at tick {}",
                if there { "exists" } else { "does not exist" },
                view.tick
            );
            if there == *yes { Ok(msg) } else { Err(msg) }
        }
        Pred::Resource {
            player,
            res,
            cmp,
            at,
        } => {
            let view = match at {
                Some(t) => b.view_at(*t)?,
                None => b.view(),
            };
            let pv = view
                .player(PlayerId(*player))
                .ok_or_else(|| format!("no player {player}"))?;
            let v = pv.resources.get(*res).map_or(0.0, |&f| to_f64(f));
            let msg = format!(
                "player {player} resource {res} = {v} at tick {} (want {cmp:?})",
                view.tick
            );
            if cmp.holds(v) { Ok(msg) } else { Err(msg) }
        }
        Pred::Eliminated { player, yes, at } => {
            let view = match at {
                Some(t) => b.view_at(*t)?,
                None => b.view(),
            };
            let pv = view
                .player(PlayerId(*player))
                .ok_or_else(|| format!("no player {player}"))?;
            let msg = format!(
                "player {player} eliminated = {} at tick {}",
                pv.eliminated, view.tick
            );
            if pv.eliminated == *yes {
                Ok(msg)
            } else {
                Err(msg)
            }
        }
        Pred::Sees {
            player,
            obj,
            yes,
            at,
        } => {
            let view = match at {
                Some(t) => b.view_at(*t)?,
                None => b.view(),
            };
            let id = b.resolve(obj)?;
            let sees = view
                .visible_to(PlayerId(*player))
                .iter()
                .any(|u| u.id == id);
            let msg = format!(
                "player {player} {} `{obj}` at tick {}",
                if sees { "sees" } else { "does not see" },
                view.tick
            );
            if sees == *yes { Ok(msg) } else { Err(msg) }
        }
        Pred::Mark { spec, yes, window } => {
            let now = b.tick();
            let end = match window {
                Window::By(t) | Window::At(t) | Window::Between(_, t) => *t,
            };
            let ok = |t: Tick| match window {
                Window::By(e) => t <= *e,
                Window::At(e) => t == *e,
                Window::Between(a, e) => t >= *a && t <= *e,
            };
            let found = marks::derive(b)
                .into_iter()
                .find(|m| ok(m.tick) && spec.matches(m, b));
            let want = format!("{} {:?}", spec.kind, spec.fields);
            match (found, yes) {
                (Some(m), true) => Ok(format!("mark {want} at tick {}", m.tick)),
                (Some(m), false) => Err(format!(
                    "mark {want} exists at tick {} ({window:?})",
                    m.tick
                )),
                (None, _) if end > now => Err(format!(
                    "tick {end} not reached yet (now tick {now}); cannot decide mark {want}"
                )),
                (None, true) => Err(format!("no mark {want} in {window:?} (ran to tick {now})")),
                (None, false) => Ok(format!("no mark {want} in {window:?}")),
            }
        }
    }
}

/// `Wait ... until <pred>`: step until it holds or `limit` ticks pass.
pub fn wait(b: &mut Bench, p: &Pred, limit: Tick) -> Result<String, String> {
    if !p.is_state() {
        return Err(
            "`Wait until` takes a predicate about the present (no `at tick`, no mark)".into(),
        );
    }
    let start = b.tick();
    loop {
        if let Ok(m) = eval(b, p) {
            return Ok(format!("{m}; after {} ticks", b.tick() - start));
        }
        if b.tick() - start >= limit {
            return Err(format!(
                "not true after {limit} ticks: {}",
                eval(b, p).err().unwrap_or_default()
            ));
        }
        if !b.advance() {
            return Err("paused before the condition held".into());
        }
    }
}

// ---- typed counts ---------------------------------------------------------

/// How many living objects of `c.noun` the team has.
pub fn count(b: &Bench, c: &Condition) -> u32 {
    let owner = owner_of(c.team);
    b.view()
        .units
        .iter()
        .filter(|u| u.owner == owner && !u.flags.contains(UnitFlags::DYING))
        .filter(|u| {
            use sta_sim::world::EntityKind as K;
            let Some(e) = b.sim.unit(u.id) else {
                return false;
            };
            match c.noun {
                Noun::Ship => e.kind == K::Craft,
                Noun::Station => e.kind == K::Station,
                Noun::Planet => e.kind == K::Resource,
                Noun::Object => e.kind != K::Projectile,
            }
        })
        .count() as u32
}
