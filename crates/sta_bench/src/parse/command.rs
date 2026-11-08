//! The command vocabulary (`testbench/scene/README.md`, "Commands", plus the
//! STA64 additions in `docs/TESTBENCH.md`).
//!
//! Grammar rules, from `scene.c`'s `run_command`:
//! - words are split on spaces and tabs; verbs are case-insensitive;
//! - a line whose first word starts with `;` or `#` is a comment;
//! - `a; b` runs two commands (`a2test drive` writes them as separate lines);
//! - a point is three numbers, otherwise a single word names an object.
//!
//! Where STA64 differs from Refit:
//! - a known verb with the wrong arguments is an [`ParseError`] (Refit answers
//!   `! usage` or silently falls through to "unknown command");
//! - an unknown verb is [`BenchCommand::Unknown`], not an error: the runner
//!   answers `! unsupported`, as the scene still has to load;
//! - `at <time> <command>` names the tick a command lands on.

use super::error::ParseError;
use super::value::{System, Target, TimeSpec, number, on_off};

/// A command and, when written `at <time> <command>`, when it lands.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedCommand {
    pub at: Option<TimeSpec>,
    pub command: BenchCommand,
}

/// A craft switch that Refit sets with `<verb> <name> on|off` or an `[Object]` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    Heal,
    Engines,
    Weapons,
    Immortal,
    Energy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VentSystem {
    Shields,
    Engines,
    Weapons,
    Sensors,
    Life,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VentTarget {
    All,
    One(VentSystem),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloakMode {
    On,
    Off,
    /// Fully cloaked at once.
    Now,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    Crew,
    Officers,
    Dilithium,
    Latinum,
    Metal,
    Biomatter,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CargoAmount {
    Full,
    Empty,
    /// The hold's new level.
    Level(f64),
}

/// A planet's population. `Full` is the class's maximum; `Off` makes it neutral again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Population {
    Full,
    Off,
    Count(u32),
}

/// `tech <team | object> ...`: an object stands for its team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TechWho {
    Team(i32),
    Object(String),
}

/// One bench command.
///
/// Orders go to the sim as `Order`s, bench edits as `Edit`s, and view and
/// query commands never touch sim state (TESTBENCH, "Commands"). Names are the
/// scene's object names, or `#<id>` for a sim id; resolving them is the
/// runner's job.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum BenchCommand {
    // View.
    Orbit {
        target: Target,
        yaw: f64,
        pitch: f64,
        distance: f64,
    },
    Glide {
        seconds: f64,
        yaw: f64,
        pitch: f64,
        distance: f64,
    },
    Spin {
        degrees_per_second: f64,
    },
    /// `camera <ex> <ey> <ez> <point | object>`.
    Camera {
        eye: [f64; 3],
        look: Target,
    },
    /// `camera rts` / `camera off`: back to the game's own camera.
    CameraRts,
    /// `zoom [level]`; no level only reads.
    Zoom(Option<f64>),
    Center(String),
    /// `hud`, `grid`, `cursor`, `notices`, `tooltips`, `hover`, `shieldfx`.
    Display {
        system: System,
        on: bool,
    },

    // Time.
    Pause,
    Resume,
    PauseAt(TimeSpec),
    Tick,
    Seed(u64),

    // Query.
    Query,
    Objects,
    Marks,
    Selection,
    /// `select a b c`; `select+ a b c` adds to the selection.
    Select {
        names: Vec<String>,
        additive: bool,
    },

    // Orders.
    Goto {
        who: Vec<String>,
        to: Target,
        warp: bool,
    },
    Attack {
        who: String,
        target: String,
    },
    Stop(String),
    Build {
        rig: String,
        odf: String,
        at: Target,
    },
    Formation {
        who: Vec<String>,
        kind: i64,
    },
    Cloak {
        name: String,
        mode: CloakMode,
    },
    Special {
        name: String,
        weapon: String,
        target: Option<String>,
    },

    // Bench edits.
    Spawn {
        name: String,
        odf: String,
        pos: [f64; 3],
        heading: f64,
        team: i32,
    },
    Kill(String),
    Damage {
        name: String,
        amount: f64,
    },
    Set {
        switch: Switch,
        name: String,
        on: bool,
    },
    Vent {
        name: String,
        target: VentTarget,
        on: bool,
    },
    Tech {
        who: TechWho,
        item: String,
        on: bool,
    },
    Cargo {
        name: String,
        resource: Resource,
        amount: CargoAmount,
    },
    Colonize {
        planet: String,
        population: Population,
        team: i32,
    },
    /// `succeed [seconds]` / `fail [seconds]`.
    EndMission {
        won: bool,
        delay_seconds: f64,
    },

    /// A verb this vocabulary does not know (or Refit-only ones such as
    /// `waypoint-key`). The runner answers `! unsupported: <verb>`.
    Unknown {
        verb: String,
        text: String,
    },
}

/// Parse one command. `Ok(None)` for a blank line or a comment.
pub fn parse_command(text: &str) -> Result<Option<TimedCommand>, ParseError> {
    let words: Vec<&str> = text.split([' ', '\t']).filter(|w| !w.is_empty()).collect();
    let Some(first) = words.first() else {
        return Ok(None);
    };
    if first.starts_with([';', '#']) {
        return Ok(None);
    }
    let fail = |m: String| ParseError::new(1, m);

    let (at, words) = if first.eq_ignore_ascii_case("at") {
        let spec = words
            .get(1)
            .ok_or_else(|| fail("`at` needs a time and a command".into()))?;
        let at = TimeSpec::parse(spec).ok_or_else(|| {
            fail(format!(
                "`at {spec}`: expected ticks (200) or seconds (3.2s)"
            ))
        })?;
        if words.len() < 3 {
            return Err(fail("`at` needs a command after the time".into()));
        }
        (Some(at), &words[2..])
    } else {
        (None, &words[..])
    };
    let command = parse_words(words, text).map_err(fail)?;
    Ok(Some(TimedCommand { at, command }))
}

/// Parse `a; b; c`, the form `Scene "..."` steps and `Camera=` use. A text that
/// starts with a comment mark is one comment.
pub fn parse_command_list(text: &str) -> Result<Vec<TimedCommand>, ParseError> {
    if text.trim_start().starts_with([';', '#']) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for part in text.split(';') {
        out.extend(parse_command(part)?);
    }
    Ok(out)
}

/// Parse a command file (`Scene.cmd`): one command list per line, errors carry the line.
pub fn parse_script(text: &str) -> Result<Vec<(usize, TimedCommand)>, ParseError> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let cmds = parse_command_list(line).map_err(|e| e.at_line(i + 1))?;
        out.extend(cmds.into_iter().map(|c| (i + 1, c)));
    }
    Ok(out)
}

fn usage(u: &str) -> String {
    format!("usage: {u}")
}

/// Check the argument count against a usage line.
fn arity(args: &[&str], min: usize, max: usize, u: &str) -> Result<(), String> {
    if args.len() < min {
        Err(usage(u))
    } else if args.len() > max {
        Err(format!("{}; unexpected `{}`", usage(u), args[max]))
    } else {
        Ok(())
    }
}

fn num(s: &str, what: &str) -> Result<f64, String> {
    number(s).ok_or_else(|| format!("{what} must be a number, not `{s}`"))
}

fn int(s: &str, what: &str) -> Result<i32, String> {
    let v = num(s, what)?;
    if v.fract() == 0.0 && v >= f64::from(i32::MIN) && v <= f64::from(i32::MAX) {
        Ok(v as i32)
    } else {
        Err(format!("{what} must be a whole number, not `{s}`"))
    }
}

fn flag(s: &str) -> Result<bool, String> {
    on_off(s).ok_or_else(|| format!("expected on or off, not `{s}`"))
}

fn names(s: &str) -> Result<Vec<String>, String> {
    let v: Vec<String> = s
        .split(',')
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect();
    if v.is_empty() {
        Err("expected an object name".into())
    } else {
        Ok(v)
    }
}

/// A point (three numbers) or an object name, and how many words it used.
fn target(args: &[&str], u: &str) -> Result<(Target, usize), String> {
    match args {
        [] => Err(usage(u)),
        [x, y, z, ..] if [x, y, z].iter().all(|s| number(s).is_some()) => {
            Ok((Target::Point([num(x, "x")?, num(y, "y")?, num(z, "z")?]), 3))
        }
        [first, ..] if number(first).is_some() => Err(format!(
            "`{first}` starts a point, which needs x y z; {}",
            usage(u)
        )),
        [name, ..] => Ok((Target::Object((*name).to_owned()), 1)),
    }
}

fn parse_words(w: &[&str], text: &str) -> Result<BenchCommand, String> {
    let verb = w[0].to_ascii_lowercase();
    let a = &w[1..];
    Ok(match verb.as_str() {
        "orbit" => {
            let u = "orbit <object | x y z> <yaw> <pitch> <distance>";
            let (target, used) = target(a, u)?;
            arity(a, used + 3, used + 3, u)?;
            BenchCommand::Orbit {
                target,
                yaw: num(a[used], "yaw")?,
                pitch: num(a[used + 1], "pitch")?,
                distance: num(a[used + 2], "distance")?,
            }
        }
        "glide" => {
            arity(a, 4, 4, "glide <seconds> <yaw> <pitch> <distance>")?;
            BenchCommand::Glide {
                seconds: num(a[0], "seconds")?,
                yaw: num(a[1], "yaw")?,
                pitch: num(a[2], "pitch")?,
                distance: num(a[3], "distance")?,
            }
        }
        "spin" => {
            arity(a, 1, 1, "spin <degrees per second>")?;
            BenchCommand::Spin {
                degrees_per_second: num(a[0], "rate")?,
            }
        }
        "camera" => {
            let u = "camera <ex> <ey> <ez> <object | tx ty tz> | camera rts";
            if a.len() == 1
                && (a[0].eq_ignore_ascii_case("rts") || a[0].eq_ignore_ascii_case("off"))
            {
                BenchCommand::CameraRts
            } else {
                if a.len() < 4 {
                    return Err(usage(u));
                }
                let eye = [num(a[0], "ex")?, num(a[1], "ey")?, num(a[2], "ez")?];
                let (look, used) = target(&a[3..], u)?;
                arity(a, 3 + used, 3 + used, u)?;
                BenchCommand::Camera { eye, look }
            }
        }
        "zoom" => {
            arity(a, 0, 1, "zoom [level]")?;
            BenchCommand::Zoom(a.first().map(|s| num(s, "level")).transpose()?)
        }
        "center" => {
            arity(a, 1, 1, "center <name>")?;
            BenchCommand::Center(a[0].to_owned())
        }
        "hud" | "grid" | "cursor" | "notices" | "tooltips" | "hover" | "shieldfx" => {
            arity(a, 1, 1, &format!("{verb} on|off"))?;
            let system = System::from_key(&verb).expect("matched above");
            BenchCommand::Display {
                system,
                on: flag(a[0])?,
            }
        }

        "pause" | "resume" | "tick" | "query" | "objects" | "marks" | "selection" => {
            arity(a, 0, 0, &verb)?;
            match verb.as_str() {
                "pause" => BenchCommand::Pause,
                "resume" => BenchCommand::Resume,
                "tick" => BenchCommand::Tick,
                "query" => BenchCommand::Query,
                "objects" => BenchCommand::Objects,
                "marks" => BenchCommand::Marks,
                _ => BenchCommand::Selection,
            }
        }
        "pause-at" => {
            arity(a, 1, 1, "pause-at <ticks | Ns>")?;
            BenchCommand::PauseAt(
                TimeSpec::parse(a[0]).ok_or_else(|| {
                    format!("expected ticks (200) or seconds (3.2s), not `{}`", a[0])
                })?,
            )
        }
        "seed" => {
            arity(a, 1, 1, "seed <n>")?;
            let v = num(a[0], "seed")?;
            if v < 0.0 || v.fract() != 0.0 || v > u64::MAX as f64 {
                return Err(format!("seed must be a whole number, not `{}`", a[0]));
            }
            BenchCommand::Seed(v as u64)
        }

        "select" | "select+" => {
            if a.is_empty() {
                return Err(usage(&format!("{verb} <name> [<name> ...]")));
            }
            BenchCommand::Select {
                names: a.iter().map(|s| (*s).to_owned()).collect(),
                additive: verb == "select+",
            }
        }

        "goto" | "move" => {
            let u = "goto <name>[,<name>...] <object | x y z> [warp]";
            if a.len() < 2 {
                return Err(usage(u));
            }
            let who = names(a[0])?;
            let (to, used) = target(&a[1..], u)?;
            let rest = &a[1 + used..];
            let warp = match rest {
                [] => false,
                [w] if w.eq_ignore_ascii_case("warp") => true,
                [other, ..] => return Err(format!("{}; unexpected `{other}`", usage(u))),
            };
            BenchCommand::Goto { who, to, warp }
        }
        "attack" => {
            arity(a, 2, 2, "attack <name> <target>")?;
            BenchCommand::Attack {
                who: a[0].to_owned(),
                target: a[1].to_owned(),
            }
        }
        "stop" => {
            arity(a, 1, 1, "stop <name>")?;
            BenchCommand::Stop(a[0].to_owned())
        }
        "build" => {
            let u = "build <rig> <odf> <x y z | object>";
            if a.len() < 3 {
                return Err(usage(u));
            }
            let (at, used) = target(&a[2..], u)?;
            arity(a, 2 + used, 2 + used, u)?;
            BenchCommand::Build {
                rig: a[0].to_owned(),
                odf: a[1].to_owned(),
                at,
            }
        }
        "formation" => {
            arity(a, 2, 2, "formation <name>[,<name>...] <param>")?;
            BenchCommand::Formation {
                who: names(a[0])?,
                kind: i64::from(int(a[1], "param")?),
            }
        }
        "cloak" => {
            arity(a, 2, 2, "cloak <name> on|off|now")?;
            let mode = if a[1].eq_ignore_ascii_case("now") {
                CloakMode::Now
            } else if flag(a[1])? {
                CloakMode::On
            } else {
                CloakMode::Off
            };
            BenchCommand::Cloak {
                name: a[0].to_owned(),
                mode,
            }
        }
        "special" => {
            arity(a, 2, 3, "special <name> <weapon> [<target>]")?;
            BenchCommand::Special {
                name: a[0].to_owned(),
                weapon: a[1].to_owned(),
                target: a.get(2).map(|s| (*s).to_owned()),
            }
        }

        "spawn" => {
            arity(a, 5, 7, "spawn <name> <odf> <x> <y> <z> [heading] [team]")?;
            BenchCommand::Spawn {
                name: a[0].to_owned(),
                odf: a[1].to_owned(),
                pos: [num(a[2], "x")?, num(a[3], "y")?, num(a[4], "z")?],
                heading: a
                    .get(5)
                    .map(|s| num(s, "heading"))
                    .transpose()?
                    .unwrap_or(0.0),
                team: a.get(6).map(|s| int(s, "team")).transpose()?.unwrap_or(1),
            }
        }
        "kill" => {
            arity(a, 1, 1, "kill <name>")?;
            BenchCommand::Kill(a[0].to_owned())
        }
        "damage" => {
            arity(a, 2, 2, "damage <name> <amount>")?;
            BenchCommand::Damage {
                name: a[0].to_owned(),
                amount: num(a[1], "amount")?,
            }
        }
        "heal" | "engines" | "weapons" | "immortal" | "energy" => {
            arity(a, 2, 2, &format!("{verb} <name> on|off"))?;
            let switch = match verb.as_str() {
                "heal" => Switch::Heal,
                "engines" => Switch::Engines,
                "weapons" => Switch::Weapons,
                "immortal" => Switch::Immortal,
                _ => Switch::Energy,
            };
            BenchCommand::Set {
                switch,
                name: a[0].to_owned(),
                on: flag(a[1])?,
            }
        }
        "vent" => {
            arity(a, 3, 3, "vent <name> <system | all> on|off")?;
            let target = match a[1].to_ascii_lowercase().as_str() {
                "all" => VentTarget::All,
                s => VentTarget::One(vent_system(s).ok_or_else(|| {
                    format!("system: shields, engines, weapons, sensors, life or all, not `{s}`")
                })?),
            };
            BenchCommand::Vent {
                name: a[0].to_owned(),
                target,
                on: flag(a[2])?,
            }
        }
        "tech" => {
            arity(a, 2, 3, "tech <team | object> <item> [on|off]")?;
            let who = match number(a[0]) {
                Some(_) => TechWho::Team(int(a[0], "team")?),
                None => TechWho::Object(a[0].to_owned()),
            };
            let on = a.get(2).map(|s| flag(s)).transpose()?.unwrap_or(true);
            BenchCommand::Tech {
                who,
                item: a[1].to_owned(),
                on,
            }
        }
        "cargo" => {
            arity(a, 2, 3, "cargo <name> <resource> [<amount> | full | empty]")?;
            let resource = resource(a[1]).ok_or_else(|| {
                format!(
                    "resource: dilithium, latinum, metal, biomatter, crew or officers, not `{}`",
                    a[1]
                )
            })?;
            let amount = match a.get(2).map(|s| s.to_ascii_lowercase()).as_deref() {
                None | Some("full") => CargoAmount::Full,
                Some("empty") => CargoAmount::Empty,
                Some(_) => CargoAmount::Level(num(a[2], "amount")?),
            };
            BenchCommand::Cargo {
                name: a[0].to_owned(),
                resource,
                amount,
            }
        }
        "colonize" => {
            arity(
                a,
                1,
                3,
                "colonize <planet> [<population> | full | off] [team]",
            )?;
            BenchCommand::Colonize {
                planet: a[0].to_owned(),
                population: a.get(1).map_or(Ok(Population::Full), |s| population(s))?,
                team: a.get(2).map(|s| int(s, "team")).transpose()?.unwrap_or(1),
            }
        }
        "succeed" | "fail" => {
            arity(a, 0, 1, &format!("{verb} [seconds]"))?;
            BenchCommand::EndMission {
                won: verb == "succeed",
                delay_seconds: a
                    .first()
                    .map(|s| num(s, "seconds"))
                    .transpose()?
                    .unwrap_or(0.0),
            }
        }

        _ => BenchCommand::Unknown {
            verb,
            text: text.trim().to_owned(),
        },
    })
}

pub(crate) fn vent_system(s: &str) -> Option<VentSystem> {
    Some(match s.to_ascii_lowercase().as_str() {
        "shields" => VentSystem::Shields,
        "engines" => VentSystem::Engines,
        "weapons" => VentSystem::Weapons,
        "sensors" => VentSystem::Sensors,
        "life" => VentSystem::Life,
        _ => return None,
    })
}

fn resource(s: &str) -> Option<Resource> {
    Some(match s.to_ascii_lowercase().as_str() {
        "crew" => Resource::Crew,
        "officers" => Resource::Officers,
        "dilithium" => Resource::Dilithium,
        "latinum" => Resource::Latinum,
        "metal" => Resource::Metal,
        "biomatter" => Resource::Biomatter,
        _ => return None,
    })
}

pub(crate) fn population(s: &str) -> Result<Population, String> {
    match s.to_ascii_lowercase().as_str() {
        "full" => Ok(Population::Full),
        "off" => Ok(Population::Off),
        _ => {
            let v = num(s, "population")?;
            if v < 0.0 || v.fract() != 0.0 || v > f64::from(u32::MAX) {
                return Err(format!(
                    "population must be a whole number, full or off, not `{s}`"
                ));
            }
            Ok(Population::Count(v as u32))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(s: &str) -> BenchCommand {
        parse_command(s)
            .expect("parses")
            .expect("a command")
            .command
    }

    fn err(s: &str) -> String {
        parse_command(s).expect_err("should fail").message
    }

    fn obj(s: &str) -> Target {
        Target::Object(s.into())
    }

    #[test]
    fn view_commands() {
        assert_eq!(
            cmd("orbit shooter 120 20 250"),
            BenchCommand::Orbit {
                target: obj("shooter"),
                yaw: 120.0,
                pitch: 20.0,
                distance: 250.0
            }
        );
        assert_eq!(
            cmd("orbit 2000 0 2175 90 25 700"),
            BenchCommand::Orbit {
                target: Target::Point([2000.0, 0.0, 2175.0]),
                yaw: 90.0,
                pitch: 25.0,
                distance: 700.0
            }
        );
        assert_eq!(
            cmd("glide 2.5 360 -10 300"),
            BenchCommand::Glide {
                seconds: 2.5,
                yaw: 360.0,
                pitch: -10.0,
                distance: 300.0
            }
        );
        assert_eq!(
            cmd("spin 0"),
            BenchCommand::Spin {
                degrees_per_second: 0.0
            }
        );
        assert_eq!(
            cmd("camera 1 2 3 warbird"),
            BenchCommand::Camera {
                eye: [1.0, 2.0, 3.0],
                look: obj("warbird")
            }
        );
        assert_eq!(
            cmd("camera 1 2 3 4 5 6"),
            BenchCommand::Camera {
                eye: [1.0, 2.0, 3.0],
                look: Target::Point([4.0, 5.0, 6.0])
            }
        );
        assert_eq!(cmd("camera rts"), BenchCommand::CameraRts);
        assert_eq!(cmd("CAMERA off"), BenchCommand::CameraRts);
        assert_eq!(cmd("zoom"), BenchCommand::Zoom(None));
        assert_eq!(cmd("zoom 0.37"), BenchCommand::Zoom(Some(0.37)));
        assert_eq!(cmd("center mine"), BenchCommand::Center("mine".into()));
        assert_eq!(
            cmd("hud off"),
            BenchCommand::Display {
                system: System::Hud,
                on: false
            }
        );
        assert_eq!(
            cmd("shieldfx 1"),
            BenchCommand::Display {
                system: System::ShieldFx,
                on: true
            }
        );
    }

    #[test]
    fn time_and_query_commands() {
        assert_eq!(cmd("pause"), BenchCommand::Pause);
        assert_eq!(cmd("resume"), BenchCommand::Resume);
        assert_eq!(cmd("tick"), BenchCommand::Tick);
        assert_eq!(
            cmd("pause-at 200"),
            BenchCommand::PauseAt(TimeSpec::Ticks(200))
        );
        assert_eq!(
            cmd("pause-at 3.2s"),
            BenchCommand::PauseAt(TimeSpec::Seconds(3.2))
        );
        assert_eq!(cmd("seed 7"), BenchCommand::Seed(7));
        assert_eq!(cmd("query"), BenchCommand::Query);
        assert_eq!(cmd("objects"), BenchCommand::Objects);
        assert_eq!(cmd("marks"), BenchCommand::Marks);
        assert_eq!(cmd("selection"), BenchCommand::Selection);
        assert_eq!(
            cmd("select g1 g2 #3"),
            BenchCommand::Select {
                names: vec!["g1".into(), "g2".into(), "#3".into()],
                additive: false
            }
        );
        assert_eq!(
            cmd("select+ g4"),
            BenchCommand::Select {
                names: vec!["g4".into()],
                additive: true
            }
        );
    }

    #[test]
    fn orders() {
        assert_eq!(
            cmd("goto a,b,c 1 2 3 warp"),
            BenchCommand::Goto {
                who: vec!["a".into(), "b".into(), "c".into()],
                to: Target::Point([1.0, 2.0, 3.0]),
                warp: true
            }
        );
        assert_eq!(
            cmd("goto a planet"),
            BenchCommand::Goto {
                who: vec!["a".into()],
                to: obj("planet"),
                warp: false
            }
        );
        assert_eq!(cmd("move a planet"), cmd("goto a planet"));
        assert_eq!(
            cmd("attack shooter target"),
            BenchCommand::Attack {
                who: "shooter".into(),
                target: "target".into()
            }
        );
        assert_eq!(cmd("stop shooter"), BenchCommand::Stop("shooter".into()));
        assert_eq!(
            cmd("build rig fyard 10 0 20"),
            BenchCommand::Build {
                rig: "rig".into(),
                odf: "fyard".into(),
                at: Target::Point([10.0, 0.0, 20.0])
            }
        );
        assert_eq!(
            cmd("formation a,b 2"),
            BenchCommand::Formation {
                who: vec!["a".into(), "b".into()],
                kind: 2
            }
        );
        assert_eq!(
            cmd("cloak bop now"),
            BenchCommand::Cloak {
                name: "bop".into(),
                mode: CloakMode::Now
            }
        );
        assert_eq!(
            cmd("cloak bop on"),
            BenchCommand::Cloak {
                name: "bop".into(),
                mode: CloakMode::On
            }
        );
        assert_eq!(
            cmd("special sov gcorbom"),
            BenchCommand::Special {
                name: "sov".into(),
                weapon: "gcorbom".into(),
                target: None
            }
        );
        assert_eq!(
            cmd("special sov gcorbom borg"),
            BenchCommand::Special {
                name: "sov".into(),
                weapon: "gcorbom".into(),
                target: Some("borg".into())
            }
        );
    }

    #[test]
    fn bench_edits() {
        assert_eq!(
            cmd("spawn ktgt kcruise1 2600 0 2350 180 2"),
            BenchCommand::Spawn {
                name: "ktgt".into(),
                odf: "kcruise1".into(),
                pos: [2600.0, 0.0, 2350.0],
                heading: 180.0,
                team: 2
            }
        );
        // Heading and team are optional: 0 and team 1, as Refit.
        assert_eq!(
            cmd("spawn s fgalaxy 1 2 3"),
            BenchCommand::Spawn {
                name: "s".into(),
                odf: "fgalaxy".into(),
                pos: [1.0, 2.0, 3.0],
                heading: 0.0,
                team: 1
            }
        );
        assert_eq!(cmd("kill sov"), BenchCommand::Kill("sov".into()));
        assert_eq!(
            cmd("damage sov 50.5"),
            BenchCommand::Damage {
                name: "sov".into(),
                amount: 50.5
            }
        );
        for (verb, switch) in [
            ("heal", Switch::Heal),
            ("engines", Switch::Engines),
            ("weapons", Switch::Weapons),
            ("immortal", Switch::Immortal),
            ("energy", Switch::Energy),
        ] {
            assert_eq!(
                cmd(&format!("{verb} x off")),
                BenchCommand::Set {
                    switch,
                    name: "x".into(),
                    on: false
                }
            );
        }
        assert_eq!(
            cmd("vent x shields on"),
            BenchCommand::Vent {
                name: "x".into(),
                target: VentTarget::One(VentSystem::Shields),
                on: true
            }
        );
        assert_eq!(
            cmd("vent x all off"),
            BenchCommand::Vent {
                name: "x".into(),
                target: VentTarget::All,
                on: false
            }
        );
        assert_eq!(
            cmd("tech 1 gcorbom"),
            BenchCommand::Tech {
                who: TechWho::Team(1),
                item: "gcorbom".into(),
                on: true
            }
        );
        assert_eq!(
            cmd("tech sov fedpod1 off"),
            BenchCommand::Tech {
                who: TechWho::Object("sov".into()),
                item: "fedpod1".into(),
                on: false
            }
        );
        assert_eq!(
            cmd("cargo f dilithium 75"),
            BenchCommand::Cargo {
                name: "f".into(),
                resource: Resource::Dilithium,
                amount: CargoAmount::Level(75.0)
            }
        );
        assert_eq!(
            cmd("cargo f latinum"),
            BenchCommand::Cargo {
                name: "f".into(),
                resource: Resource::Latinum,
                amount: CargoAmount::Full
            }
        );
        assert_eq!(
            cmd("cargo f crew empty"),
            BenchCommand::Cargo {
                name: "f".into(),
                resource: Resource::Crew,
                amount: CargoAmount::Empty
            }
        );
        assert_eq!(
            cmd("colonize planet"),
            BenchCommand::Colonize {
                planet: "planet".into(),
                population: Population::Full,
                team: 1
            }
        );
        assert_eq!(
            cmd("colonize planet 40 2"),
            BenchCommand::Colonize {
                planet: "planet".into(),
                population: Population::Count(40),
                team: 2
            }
        );
        assert_eq!(
            cmd("colonize planet off"),
            BenchCommand::Colonize {
                planet: "planet".into(),
                population: Population::Off,
                team: 1
            }
        );
        assert_eq!(
            cmd("succeed"),
            BenchCommand::EndMission {
                won: true,
                delay_seconds: 0.0
            }
        );
        assert_eq!(
            cmd("fail 2.5"),
            BenchCommand::EndMission {
                won: false,
                delay_seconds: 2.5
            }
        );
    }

    #[test]
    fn at_prefix() {
        let t = parse_command("at 120 kill target").unwrap().unwrap();
        assert_eq!(t.at, Some(TimeSpec::Ticks(120)));
        assert_eq!(t.command, BenchCommand::Kill("target".into()));
        let t = parse_command("AT 3.2s pause").unwrap().unwrap();
        assert_eq!(t.at, Some(TimeSpec::Seconds(3.2)));
        assert!(err("at").contains("needs a time"));
        assert!(err("at 120").contains("needs a command"));
        assert!(err("at soon pause").contains("expected ticks"));
    }

    #[test]
    fn comments_blank_and_case() {
        assert_eq!(parse_command("").unwrap(), None);
        assert_eq!(parse_command("   \t ").unwrap(), None);
        assert_eq!(parse_command("; pause").unwrap(), None);
        assert_eq!(parse_command("# pause").unwrap(), None);
        assert_eq!(cmd("  ORBIT\tship  1 2 3  "), cmd("orbit ship 1 2 3"));
    }

    #[test]
    fn unknown_verbs_are_not_errors() {
        assert_eq!(
            cmd("waypoint-key on"),
            BenchCommand::Unknown {
                verb: "waypoint-key".into(),
                text: "waypoint-key on".into()
            }
        );
        assert!(matches!(cmd("windowed"), BenchCommand::Unknown { .. }));
        assert!(matches!(
            cmd("frobnicate 1 2 3"),
            BenchCommand::Unknown { .. }
        ));
    }

    #[test]
    fn malformed_known_verbs_are_errors() {
        assert!(err("orbit").starts_with("usage: orbit"));
        assert!(err("orbit ship 1 2").starts_with("usage: orbit"));
        assert!(err("orbit ship 1 2 3 4").contains("unexpected `4`"));
        assert!(err("orbit ship a b c").contains("yaw must be a number"));
        // A half-written point is not an object called "1".
        assert!(err("orbit 1 2 ship 3 4 5").contains("starts a point"));
        assert!(err("heal x maybe").contains("on or off"));
        assert!(err("heal x").starts_with("usage"));
        assert!(err("pause now").contains("unexpected"));
        assert!(err("pause-at 1.5").contains("ticks"));
        assert!(err("seed -1").contains("whole number"));
        assert!(err("spawn a b c").starts_with("usage: spawn"));
        assert!(err("spawn a b 1 2 3 0 1.5").contains("whole number"));
        assert!(err("vent x sensor on").contains("system"));
        assert!(err("cargo f gold").contains("resource"));
        assert!(err("goto a").starts_with("usage"));
        assert!(
            err("goto a b c").contains("starts a point")
                || err("goto a b c").contains("unexpected")
        );
        assert!(err("camera on").starts_with("usage: camera"));
        // Errors from a single command are on line 1.
        assert_eq!(parse_command("kill").unwrap_err().line, 1);
    }

    #[test]
    fn lists_and_scripts() {
        let l = parse_command_list("pause; orbit ship 90 20 100 ;; resume").unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].command, BenchCommand::Pause);
        assert_eq!(l[2].command, BenchCommand::Resume);
        assert!(parse_command_list("; pause; resume").unwrap().is_empty());

        let s = parse_script("# header\npause\n\nkill a; kill b\n").unwrap();
        assert_eq!(s.iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![2, 4, 4]);
        let e = parse_script("pause\nresume\nkill\n").unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.to_string().starts_with("line 3: usage: kill"));
    }
}
