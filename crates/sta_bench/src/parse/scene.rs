//! Scene files: Refit's `.ini` (`testbench/scene/README.md`, "Scene files")
//! plus STA64's additions (`Map=`, `MapObjects=`, `Profile=`, `Start=`,
//! `Pause=3.2s`; TESTBENCH, "Scenes").
//!
//! Refit reads the file with Windows' `GetPrivateProfile*`, and one file has to
//! run on both engines, so the lexing follows it:
//! - section and key names are case-insensitive, and same-named sections merge;
//! - only a line that *starts* with `;` is a comment (`a=b ; c` keeps `; c`);
//! - the first of two equal keys wins; surrounding quotes on a value are removed;
//! - a key nobody asks for is ignored. That is also how `Map=`, `Profile=` and
//!   the rest are invisible to Refit; here they are typed, and any other key is
//!   kept in [`SceneFile::unknown`] rather than refused.
//!
//! One place where the engines would disagree silently: Refit reads `Pause=3.2s`
//! as tick 3 (its `s_atof` stops at the `s`). A cross-engine file has to use a
//! plain tick count there until Refit learns the suffix.
//!
//! Where STA64 is stricter: a typed key whose value does not parse is an error
//! with the line (Refit reads `Heal=yes` as 0), and an `[Object.*]` without
//! `Odf=` is an error.

use super::command::{
    Population, TimedCommand, VentTarget, parse_command, population, vent_system,
};
use super::error::ParseError;
use super::value::{System, TimeSpec, number};

/// A parsed scene file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneFile {
    pub scene: SceneSection,
    pub marks: MarksSection,
    /// `[Object.<name>]` sections in file order, which is build order.
    pub objects: Vec<SceneObject>,
    /// Keys (and whole sections) this parser has no type for, in file order.
    pub unknown: Vec<UnknownKey>,
    /// Things worth knowing that did not stop the parse (a duplicate key).
    pub warnings: Vec<ParseWarning>,
}

/// `[Scene]`. `None` means the key was absent (or empty); the comment gives
/// Refit's default, which the runner applies (STA64's differs where noted).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSection {
    /// `Enable`, default 1.
    pub enable: bool,
    /// `Delay`: mission ticks before the scene is built. Refit 30, STA64 0.
    pub delay: Option<u32>,
    /// `Fog`, `Hud`, `Grid`, `Cursor`, `Notices`, `Tooltips`, `Hover`,
    /// `ShieldFx`, `Music`, `Captions` as written, in file order. Kept even
    /// when STA64 has no such system yet, so the runner can answer
    /// `! unsupported: <system>`.
    pub switches: Vec<SwitchSetting>,
    /// `Race`.
    pub race: Option<String>,
    /// `Anchor`: `camera` when the key is absent.
    pub anchor: Anchor,
    /// `Center`: an object to centre the RTS camera on.
    pub center: Option<String>,
    /// `Zoom`, 0 (closest) to 1 (farthest).
    pub zoom: Option<f64>,
    /// `Camera`: a `camera` or `orbit` command run once the scene is built.
    pub camera: Option<TimedCommand>,
    /// `Seed`.
    pub seed: Option<u64>,
    /// `Step`, milliseconds. STA64: the render frame length under the bench clock.
    pub step_ms: Option<u32>,
    /// `Pause`: ticks, or seconds (`3.2s`), after the scene is built.
    pub pause: Option<TimeSpec>,
    /// STA64: `Map=<name>`, a BZN from the install with its own objects.
    pub map: Option<String>,
    /// STA64: `MapObjects=keep|strip`. Unset: `strip` when the file lists
    /// objects, `keep` when it lists none ([`SceneFile::map_objects`]).
    pub map_objects: Option<MapObjects>,
    /// STA64: `Profile=` (`FO4`), a content profile instead of Refit's
    /// profile-dependent behaviour.
    pub profile: Option<String>,
    /// STA64: `Start=snapshot:<file>` / `Start=replay:<file>`.
    pub start: Option<Start>,
    /// STA64: `Allies=1,2`: these players share the first one's team (each
    /// `Team=N` is otherwise a team of its own).
    pub allies: Vec<u8>,
}

impl Default for SceneSection {
    fn default() -> Self {
        Self {
            enable: true,
            delay: None,
            switches: Vec::new(),
            race: None,
            anchor: Anchor::Camera,
            center: None,
            zoom: None,
            camera: None,
            seed: None,
            step_ms: None,
            pause: None,
            map: None,
            map_objects: None,
            profile: None,
            start: None,
            allies: Vec::new(),
        }
    }
}

impl SceneSection {
    /// What the file says about a system, if anything (the last assignment wins).
    pub fn switch(&self, system: System) -> Option<bool> {
        self.switches
            .iter()
            .rev()
            .find(|s| s.system == system)
            .map(|s| s.on)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchSetting {
    pub system: System,
    pub on: bool,
    pub line: usize,
}

/// `Anchor=`: how object positions read. Refit looks at the first letter only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    /// Offsets from the RTS camera's interest point.
    #[default]
    Camera,
    /// World coordinates.
    World,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapObjects {
    Keep,
    Strip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    Snapshot(String),
    Replay(String),
}

/// `[Marks]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarksSection {
    /// `Enable`, default 1.
    pub enable: bool,
    /// `FireGap`, seconds, default 1.0.
    pub fire_gap: f64,
    /// `Still`, seconds, default 0.3.
    pub still: f64,
    /// `SimTime`, default 0: every mark carries `t=`, the simulation's
    /// seconds (Refit's `bench/measure-marks`; STA64's marks always have the
    /// tick, so this only shapes what the original-game side writes).
    pub sim_time: bool,
    /// `Ordnance`, default 0: one `ord.new` / `ord.gone` mark per ordnance.
    pub ordnance: bool,
    /// `State`, milliseconds: one `state` mark per scene craft that often.
    pub state_ms: Option<u32>,
}

impl Default for MarksSection {
    fn default() -> Self {
        Self {
            enable: true,
            fire_gap: 1.0,
            still: 0.3,
            sim_time: false,
            ordnance: false,
            state_ms: None,
        }
    }
}

/// `[Object.<name>]`, with Refit's defaults filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneObject {
    /// The part after `Object.`, as written.
    pub name: String,
    /// `Odf`: required.
    pub odf: String,
    /// `Team`, default 1 (planets and nebulae take 0).
    pub team: i32,
    /// `X`, `Y`, `Z`, default 0. Y is up.
    pub pos: [f64; 3],
    /// `Heading`, degrees; 0 faces +z.
    pub heading: f64,
    /// `Immortal`, default **1**.
    pub immortal: bool,
    /// `Heal`, default 0.
    pub heal: bool,
    /// `Energy`, default 0.
    pub energy: bool,
    /// `Tech`: items made available to the object's team.
    pub tech: Vec<String>,
    /// `Engines`, default 1.
    pub engines: bool,
    /// `Weapons`, default 1.
    pub weapons: bool,
    /// `Vent`: `none`/empty is `None`.
    pub vent: Option<VentTarget>,
    /// `Attack`: the name of an object to attack, ordered once everything is built.
    pub attack: Option<String>,
    /// `Population`: a planet colonised at once. Empty: not colonised.
    pub population: Option<Population>,
    /// `Colonist`, default 1: the team that holds a planet with `Population=`.
    pub colonist: i32,
    /// The line of the section header.
    pub line: usize,
}

/// A key (or section) with no typed field. Section and key are as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownKey {
    pub section: String,
    /// Empty when the whole section is unknown and was empty.
    pub key: String,
    pub value: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWarning {
    pub line: usize,
    pub message: String,
}

impl SceneFile {
    /// Parse the text of a scene file.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut out = Self::default();
        for sect in lex(text, &mut out.warnings)? {
            let lname = sect.name.to_ascii_lowercase();
            let mut entries = Entries::new(sect.entries);
            if lname == "scene" {
                out.scene = scene_section(&mut entries, &mut out.warnings)?;
            } else if lname == "marks" {
                out.marks = marks_section(&mut entries, &mut out.warnings)?;
            } else if let Some(name) = lname
                .starts_with("object.")
                .then(|| &sect.name["object.".len()..])
                .filter(|n| !n.is_empty())
            {
                out.objects.push(object_section(
                    name,
                    sect.line,
                    &mut entries,
                    &mut out.warnings,
                )?);
            } else if entries.items.is_empty() {
                out.unknown.push(UnknownKey {
                    section: sect.name.clone(),
                    key: String::new(),
                    value: String::new(),
                    line: sect.line,
                });
            }
            out.unknown.extend(entries.leftover(&sect.name));
        }
        Ok(out)
    }

    /// `MapObjects`, with TESTBENCH's default applied: `strip` when the file
    /// lists objects (a hand-made scene wants a clean stage), `keep` when not.
    pub fn map_objects(&self) -> MapObjects {
        self.scene
            .map_objects
            .unwrap_or(if self.objects.is_empty() {
                MapObjects::Keep
            } else {
                MapObjects::Strip
            })
    }

    pub fn object(&self, name: &str) -> Option<&SceneObject> {
        self.objects
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
    }

    /// Systems the file switches, for the runner's `! unsupported: <system>` log.
    pub fn switched_systems(&self) -> impl Iterator<Item = System> + '_ {
        self.scene.switches.iter().map(|s| s.system)
    }
}

// ---- lexing ---------------------------------------------------------------

struct RawEntry {
    key: String,
    value: String,
    line: usize,
}

struct RawSection {
    name: String,
    line: usize,
    entries: Vec<RawEntry>,
}

/// Split into sections of `key=value` entries, merging sections whose names
/// match ignoring case (at the first one's place).
fn lex(text: &str, warnings: &mut Vec<ParseWarning>) -> Result<Vec<RawSection>, ParseError> {
    let mut sections: Vec<RawSection> = Vec::new();
    let mut current: Option<usize> = None;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = if n == 1 {
            raw.trim_start_matches('\u{feff}')
        } else {
            raw
        }
        .trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest
                .split_once(']')
                .map(|(name, _)| name.trim())
                .ok_or_else(|| ParseError::new(n, "section header without `]`"))?;
            if name.is_empty() {
                return Err(ParseError::new(n, "empty section name"));
            }
            let at = sections
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(name))
                .unwrap_or_else(|| {
                    sections.push(RawSection {
                        name: name.to_owned(),
                        line: n,
                        entries: Vec::new(),
                    });
                    sections.len() - 1
                });
            current = Some(at);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            warnings.push(ParseWarning {
                line: n,
                message: format!("ignored line without `=`: `{line}`"),
            });
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(ParseError::new(n, "`=` without a key"));
        }
        let Some(at) = current else {
            return Err(ParseError::new(
                n,
                format!("key `{key}` before any [section]"),
            ));
        };
        sections[at].entries.push(RawEntry {
            key: key.to_owned(),
            value: unquote(value.trim()).to_owned(),
            line: n,
        });
    }
    Ok(sections)
}

fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

// ---- typed access ---------------------------------------------------------

struct Item {
    entry: RawEntry,
    used: bool,
}

struct Entries {
    items: Vec<Item>,
}

impl Entries {
    fn new(entries: Vec<RawEntry>) -> Self {
        Self {
            items: entries
                .into_iter()
                .map(|entry| Item { entry, used: false })
                .collect(),
        }
    }

    /// The first non-empty-or-not value of `key`; later duplicates are marked
    /// used and warned about.
    fn raw(&mut self, key: &str, warnings: &mut Vec<ParseWarning>) -> Option<(String, usize)> {
        let mut found: Option<(String, usize)> = None;
        for it in self
            .items
            .iter_mut()
            .filter(|it| it.entry.key.eq_ignore_ascii_case(key))
        {
            it.used = true;
            if found.is_none() {
                found = Some((it.entry.value.clone(), it.entry.line));
            } else {
                warnings.push(ParseWarning {
                    line: it.entry.line,
                    message: format!(
                        "`{key}` given twice; the first (line {}) wins",
                        found.as_ref().map_or(0, |f| f.1)
                    ),
                });
            }
        }
        found
    }

    /// A typed value; absent and empty are both `None` (Refit's "default").
    fn get<T>(
        &mut self,
        key: &str,
        warnings: &mut Vec<ParseWarning>,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<Option<T>, ParseError> {
        match self.raw(key, warnings) {
            Some((v, line)) if !v.is_empty() => parse(&v)
                .map(Some)
                .map_err(|m| ParseError::new(line, format!("`{key}={v}`: {m}"))),
            _ => Ok(None),
        }
    }

    fn string(&mut self, key: &str, w: &mut Vec<ParseWarning>) -> Option<String> {
        self.raw(key, w).map(|(v, _)| v).filter(|v| !v.is_empty())
    }

    fn flag(&mut self, key: &str, w: &mut Vec<ParseWarning>) -> Result<Option<bool>, ParseError> {
        self.get(key, w, |v| int(v).map(|n| n != 0))
    }

    fn float(&mut self, key: &str, w: &mut Vec<ParseWarning>) -> Result<Option<f64>, ParseError> {
        self.get(key, w, |v| {
            number(v).ok_or_else(|| "expected a number".into())
        })
    }

    fn leftover(self, section: &str) -> Vec<UnknownKey> {
        self.items
            .into_iter()
            .filter(|it| !it.used)
            .map(|it| UnknownKey {
                section: section.to_owned(),
                key: it.entry.key,
                value: it.entry.value,
                line: it.entry.line,
            })
            .collect()
    }
}

fn int(v: &str) -> Result<i64, String> {
    let n = number(v).ok_or("expected a whole number")?;
    if n.fract() == 0.0 && n.abs() <= 9e15 {
        Ok(n as i64)
    } else {
        Err("expected a whole number".into())
    }
}

fn unsigned(v: &str) -> Result<u64, String> {
    u64::try_from(int(v)?).map_err(|_| "must not be negative".into())
}

fn scene_section(e: &mut Entries, w: &mut Vec<ParseWarning>) -> Result<SceneSection, ParseError> {
    let mut s = SceneSection::default();
    if let Some(on) = e.flag("Enable", w)? {
        s.enable = on;
    }
    s.delay = e.get("Delay", w, |v| {
        unsigned(v).and_then(|n| u32::try_from(n).map_err(|e| e.to_string()))
    })?;
    // Switches in file order, not in this table's: collect the lines first.
    let mut switches = Vec::new();
    for key in [
        "Fog", "Hud", "Grid", "Cursor", "Notices", "Tooltips", "Hover", "ShieldFx", "Music",
        "Captions",
    ] {
        let line = e
            .items
            .iter()
            .find(|it| it.entry.key.eq_ignore_ascii_case(key))
            .map_or(0, |it| it.entry.line);
        if let Some(on) = e.flag(key, w)? {
            switches.push(SwitchSetting {
                system: System::from_key(key).expect("a system key"),
                on,
                line,
            });
        }
    }
    switches.sort_by_key(|s| s.line);
    s.switches = switches;
    s.race = e.string("Race", w);
    // Refit: the key's first letter decides, and an empty value is not `camera`.
    if let Some((v, _)) = e.raw("Anchor", w) {
        s.anchor = if v.starts_with(['c', 'C']) {
            Anchor::Camera
        } else {
            Anchor::World
        };
    }
    s.center = e.string("Center", w);
    s.zoom = e.float("Zoom", w)?;
    s.camera = match e.raw("Camera", w) {
        Some((v, line)) if !v.is_empty() => {
            let c = parse_command(&v).map_err(|er| er.at_line(line))?;
            if c.as_ref().is_some_and(|c| c.at.is_some()) {
                return Err(ParseError::new(
                    line,
                    "`Camera=` runs when the scene is built; `at` has no meaning there",
                ));
            }
            c
        }
        _ => None,
    };
    s.seed = e.get("Seed", w, unsigned)?;
    s.step_ms = e.get("Step", w, |v| {
        unsigned(v).and_then(|n| u32::try_from(n).map_err(|e| e.to_string()))
    })?;
    s.pause = e.get("Pause", w, |v| {
        TimeSpec::parse(v).ok_or_else(|| "expected ticks (200) or seconds (3.2s)".into())
    })?;
    s.map = e.string("Map", w);
    s.map_objects = e.get("MapObjects", w, |v| match v.to_ascii_lowercase().as_str() {
        "keep" => Ok(MapObjects::Keep),
        "strip" => Ok(MapObjects::Strip),
        _ => Err("expected keep or strip".into()),
    })?;
    s.profile = e.string("Profile", w);
    s.allies = e
        .get("Allies", w, |v| {
            v.split(',')
                .map(|t| t.trim().parse::<u8>().map_err(|e| e.to_string()))
                .collect::<Result<Vec<u8>, String>>()
        })?
        .unwrap_or_default();
    s.start = e.get("Start", w, |v| match v.split_once(':') {
        Some((kind, file)) if !file.is_empty() && kind.eq_ignore_ascii_case("snapshot") => {
            Ok(Start::Snapshot(file.to_owned()))
        }
        Some((kind, file)) if !file.is_empty() && kind.eq_ignore_ascii_case("replay") => {
            Ok(Start::Replay(file.to_owned()))
        }
        _ => Err("expected snapshot:<file> or replay:<file>".into()),
    })?;
    Ok(s)
}

fn marks_section(e: &mut Entries, w: &mut Vec<ParseWarning>) -> Result<MarksSection, ParseError> {
    let d = MarksSection::default();
    Ok(MarksSection {
        enable: e.flag("Enable", w)?.unwrap_or(d.enable),
        fire_gap: e.float("FireGap", w)?.unwrap_or(d.fire_gap),
        still: e.float("Still", w)?.unwrap_or(d.still),
        sim_time: e.flag("SimTime", w)?.unwrap_or(d.sim_time),
        ordnance: e.flag("Ordnance", w)?.unwrap_or(d.ordnance),
        state_ms: e.get("State", w, |v| {
            int(v).and_then(|n| u32::try_from(n).map_err(|_| "expected milliseconds".into()))
        })?
        .filter(|&n| n > 0),
    })
}

fn object_section(
    name: &str,
    line: usize,
    e: &mut Entries,
    w: &mut Vec<ParseWarning>,
) -> Result<SceneObject, ParseError> {
    let odf = e
        .string("Odf", w)
        .ok_or_else(|| ParseError::new(line, format!("[Object.{name}] has no `Odf=`")))?;
    let team = |e: &mut Entries, w: &mut Vec<ParseWarning>, key: &str, default: i32| {
        e.get(key, w, |v| {
            int(v).and_then(|n| i32::try_from(n).map_err(|e| e.to_string()))
        })
        .map(|t| t.unwrap_or(default))
    };
    Ok(SceneObject {
        name: name.to_owned(),
        odf,
        team: team(e, w, "Team", 1)?,
        pos: [
            e.float("X", w)?.unwrap_or(0.0),
            e.float("Y", w)?.unwrap_or(0.0),
            e.float("Z", w)?.unwrap_or(0.0),
        ],
        heading: e.float("Heading", w)?.unwrap_or(0.0),
        immortal: e.flag("Immortal", w)?.unwrap_or(true),
        heal: e.flag("Heal", w)?.unwrap_or(false),
        energy: e.flag("Energy", w)?.unwrap_or(false),
        tech: e
            .string("Tech", w)
            .map(|t| t.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        engines: e.flag("Engines", w)?.unwrap_or(true),
        weapons: e.flag("Weapons", w)?.unwrap_or(true),
        vent: e
            .get("Vent", w, |v| match v.to_ascii_lowercase().as_str() {
                "none" => Ok(None),
                "all" => Ok(Some(VentTarget::All)),
                s => vent_system(s)
                    .map(|s| Some(VentTarget::One(s)))
                    .ok_or_else(|| {
                        "expected all, none, shields, engines, weapons, sensors or life".into()
                    }),
            })?
            .flatten(),
        attack: e.string("Attack", w),
        population: e.get("Population", w, population)?,
        colonist: team(e, w, "Colonist", 1)?,
        line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{BenchCommand, Target};

    fn parse(s: &str) -> SceneFile {
        SceneFile::parse(s).expect("parses")
    }

    #[test]
    fn minimal_object_gets_refit_defaults() {
        let f = parse("[Object.a]\nOdf=fgalaxy\n");
        let o = &f.objects[0];
        assert_eq!(o.name, "a");
        assert_eq!((o.team, o.pos, o.heading), (1, [0.0; 3], 0.0));
        assert!(o.immortal && o.engines && o.weapons);
        assert!(!o.heal && !o.energy);
        assert_eq!(o.vent, None);
        assert_eq!(o.colonist, 1);
        assert_eq!(f.scene, SceneSection::default());
        assert_eq!(f.marks, MarksSection::default());
        assert!(f.scene.enable);
    }

    #[test]
    fn windows_ini_rules() {
        let f = parse(
            "\u{feff}; leading comment\r\n[ scene ]\r\nSEED = 7\r\nhud=0\r\nHud=1\r\n\
             Race=\"federation\"\r\n[OBJECT.Ship]\r\nodf=fgalaxy ; not a comment\r\n\
             [object.ship]\r\nteam=2\r\n[Scene]\r\nStep=16\r\n",
        );
        // Names are case-insensitive, same-named sections merge, the first
        // duplicate key wins, and an inline `;` stays in the value.
        assert_eq!(f.scene.seed, Some(7));
        assert_eq!(f.scene.step_ms, Some(16));
        assert_eq!(f.scene.switch(System::Hud), Some(false));
        assert_eq!(f.warnings.len(), 1);
        assert_eq!(f.warnings[0].line, 5);
        assert_eq!(f.scene.race.as_deref(), Some("federation"));
        assert_eq!(f.objects.len(), 1);
        assert_eq!(f.objects[0].odf, "fgalaxy ; not a comment");
        assert_eq!(f.objects[0].team, 2);
        assert_eq!(f.objects[0].name, "Ship");
    }

    #[test]
    fn stood_down_switches_are_kept_in_file_order() {
        let f = parse("[Scene]\nMusic=0\nFog=1\nHud=0\nHover=2\nShieldFx=0\n");
        let order: Vec<_> = f.switched_systems().map(System::name).collect();
        assert_eq!(order, ["music", "fog", "hud", "hover", "shieldfx"]);
        assert_eq!(f.scene.switch(System::Hover), Some(true));
        assert_eq!(f.scene.switch(System::Grid), None);
    }

    #[test]
    fn sta64_keys_are_typed() {
        let f = parse(
            "[Scene]\nMap=a2_borg01\nMapObjects=KEEP\nProfile=FO4\nStart=snapshot:minute12.snap\n\
             Pause=3.2s\nDelay=0\nZoom=0.37\nCamera=orbit ship 230 15 450\nCenter=ship\nAnchor=World\n",
        );
        let s = &f.scene;
        assert_eq!(s.map.as_deref(), Some("a2_borg01"));
        assert_eq!(s.map_objects, Some(MapObjects::Keep));
        assert_eq!(s.profile.as_deref(), Some("FO4"));
        assert_eq!(s.start, Some(Start::Snapshot("minute12.snap".into())));
        assert_eq!(s.pause, Some(TimeSpec::Seconds(3.2)));
        assert_eq!(s.delay, Some(0));
        assert_eq!(s.zoom, Some(0.37));
        assert_eq!(s.anchor, Anchor::World);
        assert_eq!(s.center.as_deref(), Some("ship"));
        let cam = s.camera.as_ref().unwrap();
        assert!(
            matches!(&cam.command, BenchCommand::Orbit { target: Target::Object(o), .. } if o == "ship")
        );
        assert!(f.unknown.is_empty());
    }

    #[test]
    fn map_objects_default_follows_who_wrote_the_scene() {
        assert_eq!(parse("[Scene]\nMap=x\n").map_objects(), MapObjects::Keep);
        assert_eq!(
            parse("[Scene]\nMap=x\n[Object.a]\nOdf=b\n").map_objects(),
            MapObjects::Strip
        );
        assert_eq!(
            parse("[Scene]\nMapObjects=keep\n[Object.a]\nOdf=b\n").map_objects(),
            MapObjects::Keep
        );
    }

    #[test]
    fn anchor_follows_refits_first_letter_rule() {
        assert_eq!(parse("[Scene]\n").scene.anchor, Anchor::Camera);
        assert_eq!(
            parse("[Scene]\nAnchor=camera\n").scene.anchor,
            Anchor::Camera
        );
        assert_eq!(parse("[Scene]\nAnchor=Cam\n").scene.anchor, Anchor::Camera);
        assert_eq!(parse("[Scene]\nAnchor=world\n").scene.anchor, Anchor::World);
        assert_eq!(parse("[Scene]\nAnchor=\n").scene.anchor, Anchor::World);
    }

    #[test]
    fn unknown_keys_and_sections_are_kept() {
        let f =
            parse("[Scene]\nSky=nebula\n[Object.a]\nOdf=x\nGlow=3\n[Extras]\nFoo=bar\n[Empty]\n");
        let got: Vec<_> = f
            .unknown
            .iter()
            .map(|u| (u.section.as_str(), u.key.as_str(), u.value.as_str(), u.line))
            .collect();
        assert_eq!(
            got,
            [
                ("Scene", "Sky", "nebula", 2),
                ("Object.a", "Glow", "3", 5),
                ("Extras", "Foo", "bar", 7),
                ("Empty", "", "", 8),
            ]
        );
    }

    #[test]
    fn object_keys() {
        let f = parse(
            "[Object.p]\nOdf=pb_clssm\nTeam=0\nPopulation=full\nColonist=2\n\
             [Object.s]\nOdf=fgalaxy\nTech=gcorbom fedpod1\nVent=shields\nEnergy=1\nAttack=p\nPopulation=12\nImmortal=0\n\
             [Object.n]\nOdf=x\nVent=none\nPopulation=\n",
        );
        let p = f.object("P").unwrap();
        assert_eq!(
            (p.team, p.population, p.colonist),
            (0, Some(Population::Full), 2)
        );
        let s = f.object("s").unwrap();
        assert_eq!(s.tech, ["gcorbom", "fedpod1"]);
        assert_eq!(
            s.vent,
            Some(VentTarget::One(crate::parse::command::VentSystem::Shields))
        );
        assert!(s.energy && !s.immortal);
        assert_eq!(s.attack.as_deref(), Some("p"));
        assert_eq!(s.population, Some(Population::Count(12)));
        let n = f.object("n").unwrap();
        assert_eq!((n.vent, n.population), (None, None));
    }

    #[test]
    fn errors_carry_the_line() {
        let e = |s: &str| SceneFile::parse(s).unwrap_err();
        assert_eq!(
            e("[Scene]\nSeed=abc\n").to_string(),
            "line 2: `Seed=abc`: expected a whole number"
        );
        assert_eq!(e("[Scene]\n\nPause=3.5\n").line, 3);
        assert_eq!(e("[Object.a]\nOdf=x\nHeal=yes\n").line, 3);
        assert_eq!(e("[Object.a]\nTeam=1\n").line, 1);
        assert!(e("[Object.a]\nTeam=1\n").message.contains("Odf"));
        assert_eq!(e("X=1\n").line, 1);
        assert_eq!(e("[Scene\n").line, 1);
        assert_eq!(e("[Scene]\nMapObjects=maybe\n").line, 2);
        assert_eq!(e("[Scene]\nStart=disk:x\n").line, 2);
        assert_eq!(e("[Scene]\nCamera=orbit ship 1 2\n").line, 2);
        assert_eq!(e("[Scene]\nCamera=at 5 pause\n").line, 2);
        assert_eq!(e("[Object.a]\nOdf=x\nVent=gills\n").line, 3);
    }

    #[test]
    fn lines_without_equals_are_warnings() {
        let f = parse("[Scene]\n# a hash is not a comment here\nSeed=1\n");
        assert_eq!(f.scene.seed, Some(1));
        assert_eq!(f.warnings.len(), 1);
        assert_eq!(f.warnings[0].line, 2);
    }

    #[test]
    fn marks_section() {
        let f = parse("[Marks]\nEnable=0\nFireGap=2.5\n");
        assert_eq!(
            f.marks,
            MarksSection {
                enable: false,
                fire_gap: 2.5,
                ..MarksSection::default()
            }
        );
        // Refit's measuring keys (`bench/measure-marks`): typed, not unknown.
        let f = parse("[Marks]\nSimTime=1\nOrdnance=1\nState=250\n");
        assert!(f.marks.sim_time && f.marks.ordnance);
        assert_eq!(f.marks.state_ms, Some(250));
        assert!(f.unknown.is_empty());
        assert_eq!(parse("[Marks]\nState=0\n").marks.state_ms, None);
    }
}
