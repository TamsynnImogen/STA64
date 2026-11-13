//! Campaigns: which missions each installed campaign holds and in what order,
//! when a campaign opens, and what a won or lost mission leads to (LUA-08).
//!
//! # Rules and the data they read
//!
//! - **Mission order:** `mshell.set`, one `.bzn` per line, `//` comments. On
//!   the Armada II shell (stock, Roots and its mods) a `-` line closes a
//!   campaign group; Armada I pads its groups with `none` lines. Read from data.
//! - **Names, titles, locked text:** `label.map`: `single_player` keys
//!   `tutorials`, `federation_campaign`, `klingon_campaign`, `borg_campaign`
//!   (Armada I: `race_names`), `mission_select` keyed by `.bzn`, and the
//!   `*_incomplete` keys for a locked campaign. Read from data.
//! - **Unlocks:** each slot keeps the furthest mission reached and a complete
//!   flag. Armada II: Klingon needs Federation complete, Borg needs Klingon;
//!   Tutorials and Federation are open (measured). Armada I: each campaign
//!   needs the previous one (not measured; the Finale gate least of all).
//! - **`GIVE_ALL_MISSIONS`** (`ART_CFG.h`): non-zero opens every mission and
//!   marks every campaign complete (measured on Armada II).
//! - **Win and loss:** winning the furthest mission reached moves the counter
//!   on, winning the last marks the campaign complete, the next mission
//!   follows a win; a loss changes nothing (measured on Armada II). No
//!   installed campaign carries other state between missions (read from
//!   data); [`CampaignProgress::carried`] is for the host and native campaigns.
//! - **Briefing:** the objectives text the mission's script names
//!   (`Objectives:load` in Lua and its includes, a `*.txt` string in the
//!   legacy DLL, an Armada II `R.drl`'s `S.dsl` included), parsed as the
//!   runtime parses it; plus the `.bik` names the script holds (for MED-04).
//!
//! # The runner
//!
//! [`CampaignRunner`] starts a mission as a [`GameSetup`] from a
//! [`MissionHost`] plus the mission's Lua [`MissionRuntime`] as its controller,
//! steps both, and watches for the mission's end: MMM's
//! `Mission:finish(success, delay)`, and `SucceedMission` / `FailMission`
//! in the `A2` / `A1` tables or as globals once LUA-05 / LUA-06 bind them.
//! On the outcome it records progress and says what comes next
//! ([`Transition`]).
//!
//! Progress saves as a small versioned text ([`CampaignProgress::save`]),
//! deterministic byte for byte, tied to the campaign book by a digest.

use std::collections::BTreeMap;
use std::sync::Arc;

use sta_content::{ContentDb, LabelMap, MapDef, OdfIndex, Profile};
use sta_core::{PlayerId, TeamId};
use sta_sim::command::Command;
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};
use sta_vfs::Vfs;

use crate::api::{MissionConfig, PlayerNumbering};
use crate::runtime::MissionRuntime;
use crate::Resolution;

// ---------------------------------------------------------------------------
// Reading install data
// ---------------------------------------------------------------------------

/// Where campaign data is read from: the mounted game ([`Vfs`]) or files in
/// memory ([`MemSource`]) for tests and tools.
pub trait CampaignSource {
    /// A file by VFS-style path (`mshell.set`, `bzn/m01.bzn`).
    fn read(&self, path: &str) -> Option<Vec<u8>>;

    fn exists(&self, path: &str) -> bool {
        self.read(path).is_some()
    }

    /// `label.map` (every layer for a VFS).
    fn labels(&self) -> LabelMap {
        self.read("label.map").map(|b| LabelMap::parse_bytes(&b)).unwrap_or_default()
    }

    /// A one-line value of a cfg header (`ART_CFG.h`, `GIVE_ALL_MISSIONS`).
    fn cfg_value(&self, file: &str, name: &str) -> Option<String> {
        let text = sta_content::strings::decode_text(&self.read(file)?);
        sta_content::layered::scan(&text).into_iter().rev().find(|(k, _, _)| k == name).map(|(_, v, _)| v)
    }
}

impl CampaignSource for Vfs {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        Vfs::read(self, path).and_then(Result::ok)
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve(path).is_some()
    }

    fn labels(&self) -> LabelMap {
        LabelMap::load(self)
    }

    /// With inheritance between layers (D11).
    fn cfg_value(&self, file: &str, name: &str) -> Option<String> {
        sta_content::layered::resolve_header(self, file, 0)?.entries.get(name).map(|e| e.value.clone())
    }
}

/// Files held in memory; paths compare case-insensitively, `\` as `/`.
#[derive(Debug, Clone, Default)]
pub struct MemSource {
    files: BTreeMap<String, Vec<u8>>,
}

impl MemSource {
    pub fn new() -> MemSource {
        MemSource::default()
    }

    pub fn with(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> MemSource {
        self.insert(path, bytes);
        self
    }

    pub fn insert(&mut self, path: &str, bytes: impl Into<Vec<u8>>) {
        self.files.insert(norm(path), bytes.into());
    }
}

impl CampaignSource for MemSource {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(&norm(path)).cloned()
    }
}

fn norm(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// Which executable's single-player shell the campaigns follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellLayout {
    /// `Armada.exe`: five campaign slots, `none`-padded groups in `mshell.set`.
    Armada1,
    /// `Armada2.exe` and the Roots engine: four slots, `-`-closed groups.
    Armada2,
}

impl ShellLayout {
    /// The layout for a content profile: Armada I's own, otherwise Armada II's
    /// (Roots, Fleet Ops 4 and STA2 Classic run on the Armada II shell).
    pub fn for_profile(profile: Profile) -> ShellLayout {
        match profile {
            Profile::Sta1Classic => ShellLayout::Armada1,
            _ => ShellLayout::Armada2,
        }
    }

    fn slots(self) -> &'static [Slot] {
        match self {
            ShellLayout::Armada1 => &A1_SLOTS,
            ShellLayout::Armada2 => &A2_SLOTS,
        }
    }
}

/// One campaign slot of a shell: its id, the label naming it, and which slot
/// must be complete first with the label saying so.
struct Slot {
    id: &'static str,
    name_label: Option<&'static str>,
    requires: Option<(usize, Option<&'static str>)>,
}

const A2_SLOTS: [Slot; 4] = [
    Slot { id: "tutorials", name_label: Some("tutorials"), requires: None },
    Slot { id: "federation", name_label: Some("federation_campaign"), requires: None },
    Slot { id: "klingon", name_label: Some("klingon_campaign"), requires: Some((1, Some("federation_incomplete"))) },
    Slot { id: "borg", name_label: Some("borg_campaign"), requires: Some((2, Some("klingon_incomplete"))) },
];

const A1_SLOTS: [Slot; 5] = [
    Slot { id: "federation", name_label: Some("federation"), requires: None },
    Slot { id: "klingon", name_label: Some("klingon"), requires: Some((0, Some("federation_incomplete"))) },
    Slot { id: "romulan", name_label: Some("romulan"), requires: Some((1, Some("klingon_incomplete"))) },
    Slot { id: "borg", name_label: Some("borg"), requires: Some((2, Some("romulan_incomplete"))) },
    // Chained like the others; not measured.
    Slot { id: "finale", name_label: None, requires: Some((3, None)) },
];

/// How a mission's script runs in STA64.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionScript {
    /// A Lua mission found by the production resolver ([`crate::resolve`]).
    Lua { path: String },
    /// Only the legacy DLL rules exist: needs a Lua conversion.
    LegacyDll { files: Vec<String> },
    /// No script: a plain scenario with no win event.
    None,
}

/// The objectives text a mission names, as the runtime parses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Briefing {
    /// The file it came from (`objectives/m01_goals.txt`).
    pub file: String,
    pub title: String,
    pub text: String,
    pub goals: Vec<String>,
}

/// One mission of a campaign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignMission {
    /// 0-based position in its campaign.
    pub index: usize,
    /// The map as `mshell.set` lists it (`m01.bzn`).
    pub bzn: String,
    /// `mission_select` title from `label.map`.
    pub title: Option<String>,
    /// Whether `bzn/<bzn>` exists.
    pub map_present: bool,
    pub script: MissionScript,
    /// Objectives files the script names, in the order it names them.
    pub objective_files: Vec<String>,
    /// The first of them that reads and parses.
    pub briefing: Option<Briefing>,
    /// `.bik` movies the script names, in string order (not play order; MED-04 lists them, the script plays them on its story events).
    pub movies: Vec<String>,
    /// Engine calls the script uses to end the mission (`Mission:finish`,
    /// `SucceedMission`, `FailMission`), found by a scan of its text.
    pub end_calls: Vec<String>,
}

impl CampaignMission {
    /// The stem of the map (`m01`).
    pub fn stem(&self) -> &str {
        let s = self.bzn.rsplit(['/', '\\']).next().unwrap_or(&self.bzn);
        map_stem(s)
    }
}

/// One campaign: a slot of the shell with its missions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Campaign {
    /// Stable id of the slot (`federation`), used in saved progress.
    pub id: String,
    /// Position in `mshell.set`.
    pub slot: usize,
    /// Display name from `label.map`, if the install names it.
    pub name: Option<String>,
    pub missions: Vec<CampaignMission>,
    /// The campaign that must be complete before this one opens.
    pub requires: Option<String>,
    /// What the shell says while it is locked.
    pub locked_text: Option<String>,
}

/// Every campaign of one install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignBook {
    pub layout: ShellLayout,
    pub campaigns: Vec<Campaign>,
    /// `ART_CFG.h`'s `GIVE_ALL_MISSIONS` is non-zero.
    pub give_all_missions: bool,
    /// Lines that could not be used, and why.
    pub warnings: Vec<String>,
}

/// A campaign mission that cannot run to a win in STA64 yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    pub campaign: String,
    pub index: usize,
    pub bzn: String,
    pub reason: String,
}

impl CampaignBook {
    /// Read every campaign from `src` (see the module docs for the files).
    pub fn load(src: &dyn CampaignSource, layout: ShellLayout) -> CampaignBook {
        let labels = src.labels();
        let mut warnings = Vec::new();
        let groups = match src.read("mshell.set") {
            Some(b) => {
                let (groups, warn) = parse_mshell(&sta_content::strings::decode_text(&b), layout);
                warnings.extend(warn);
                groups
            }
            None => {
                warnings.push("no mshell.set: no campaigns".to_owned());
                Vec::new()
            }
        };
        let slots = layout.slots();
        if groups.iter().skip(slots.len()).any(|g| !g.is_empty()) {
            warnings.push(format!(
                "mshell.set has {} groups; the shell shows {}, the rest are ignored",
                groups.len(),
                slots.len()
            ));
        }
        let label = |key: &str| labels.get(key).first().cloned();
        let campaigns = groups
            .into_iter()
            .zip(slots)
            .enumerate()
            .map(|(slot_index, (group, slot))| Campaign {
                id: slot.id.to_owned(),
                slot: slot_index,
                name: slot.name_label.and_then(label),
                missions: group.iter().enumerate().map(|(i, bzn)| load_mission(src, &labels, i, bzn)).collect(),
                requires: slot.requires.map(|(r, _)| slots[r].id.to_owned()),
                locked_text: slot.requires.and_then(|(_, l)| l).and_then(label),
            })
            .collect();
        let give_all_missions = src
            .cfg_value("ART_CFG.h", "GIVE_ALL_MISSIONS")
            .and_then(|v| v.trim().parse::<i64>().ok())
            .is_some_and(|v| v != 0);
        CampaignBook { layout, campaigns, give_all_missions, warnings }
    }

    /// [`CampaignBook::load`] from a mounted install, with the layout of its
    /// detected profile.
    pub fn load_vfs(vfs: &Vfs) -> CampaignBook {
        CampaignBook::load(vfs, ShellLayout::for_profile(Profile::detect(vfs)))
    }

    pub fn campaign(&self, id: &str) -> Option<&Campaign> {
        self.campaigns.iter().find(|c| c.id == id)
    }

    /// Identity of the campaign list (ids and maps, in order), so saved
    /// progress is not applied to a different book. FNV-1a, 64 bit.
    pub fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |s: &str| {
            for b in s.bytes().chain([0]) {
                h ^= u64::from(b.to_ascii_lowercase());
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        eat(match self.layout {
            ShellLayout::Armada1 => "a1",
            ShellLayout::Armada2 => "a2",
        });
        for c in &self.campaigns {
            eat(&c.id);
            for m in &c.missions {
                eat(&m.bzn);
            }
        }
        h
    }

    /// Every mission that cannot run to a win yet, with the reason.
    pub fn blocked(&self) -> Vec<Blocked> {
        let mut out = Vec::new();
        for c in &self.campaigns {
            for m in &c.missions {
                if let Some(reason) = blocked_reason(self.layout, m) {
                    out.push(Blocked { campaign: c.id.clone(), index: m.index, bzn: m.bzn.clone(), reason });
                }
            }
        }
        out
    }
}

fn blocked_reason(layout: ShellLayout, m: &CampaignMission) -> Option<String> {
    if !m.map_present {
        return Some(format!("map bzn/{} is missing", m.bzn));
    }
    let lane = match layout {
        ShellLayout::Armada1 => "LUA-06 (Armada I calls)",
        ShellLayout::Armada2 => "LUA-05 (Armada II calls)",
    };
    match &m.script {
        // A Lua script has a win event when it names `Mission:finish` or `SucceedMission`
        // (a converted Armada II mission, LUA-11); `FailMission` alone is a loss only.
        MissionScript::Lua { .. } if m.end_calls.iter().any(|c| c.starts_with("Mission") || c == "SucceedMission") => None,
        MissionScript::Lua { path } => Some(format!("{path} calls no Mission:finish; no win event")),
        MissionScript::LegacyDll { files } => {
            let calls = if m.end_calls.is_empty() { String::new() } else { format!("; it ends through {}", m.end_calls.join(", ")) };
            Some(format!("legacy DLL only ({}): needs a Lua conversion on {lane}{calls}", files.join(", ")))
        }
        MissionScript::None => Some("no mission script: no win event".to_owned()),
    }
}

/// `mshell.set`'s groups of `.bzn` names, and a warning when a group
/// boundary had to be guessed.
///
/// Lines are trimmed and a `//` comment is cut off. Armada II closes each
/// group with `-`. Armada I pads groups of equal width with `none`: when the
/// line count divides evenly by the shell's slots, each slot is that many
/// lines and an all-`none` slot is an empty campaign (kept, so later
/// campaigns stay in their slots); otherwise groups are split at `none` runs
/// and the result is flagged (not measured).
fn parse_mshell(text: &str, layout: ShellLayout) -> (Vec<Vec<String>>, Option<String>) {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.split("//").next().unwrap_or("").trim())
        .filter(|t| !t.is_empty())
        .collect();
    let is_none = |t: &str| t.eq_ignore_ascii_case("none");
    match layout {
        ShellLayout::Armada2 => {
            let mut groups = Vec::new();
            let mut cur: Vec<String> = Vec::new();
            for t in lines {
                if t == "-" {
                    groups.push(std::mem::take(&mut cur));
                } else {
                    cur.push(t.to_owned());
                }
            }
            if !cur.is_empty() {
                groups.push(cur);
            }
            (groups, None)
        }
        ShellLayout::Armada1 => {
            let slots = layout.slots().len();
            if !lines.is_empty() && lines.len().is_multiple_of(slots) {
                let width = lines.len() / slots;
                let groups = lines
                    .chunks(width)
                    .map(|c| c.iter().filter(|t| !is_none(t)).map(|t| (*t).to_owned()).collect())
                    .collect();
                return (groups, None);
            }
            let mut groups = Vec::new();
            let mut cur: Vec<String> = Vec::new();
            for t in lines {
                if is_none(t) {
                    if !cur.is_empty() {
                        groups.push(std::mem::take(&mut cur));
                    }
                } else {
                    cur.push(t.to_owned());
                }
            }
            if !cur.is_empty() {
                groups.push(cur);
            }
            let warn = (!groups.is_empty()).then(|| {
                format!("mshell.set does not split into {slots} equal slots; groups split at `none` lines (an empty campaign would shift later ones)")
            });
            (groups, warn)
        }
    }
}

fn load_mission(src: &dyn CampaignSource, labels: &LabelMap, index: usize, bzn: &str) -> CampaignMission {
    let map_path = format!("bzn/{bzn}");
    let map_bytes = src.read(&map_path);
    let map = map_bytes.as_deref().and_then(|b| sta_bzn::parse(b).ok()).unwrap_or_default();
    let exists = |p: &str| src.exists(p);
    let map_file = format!("{}.bzn", map_stem(bzn));
    let script = match crate::resolve_with(&exists, labels.get(&map_file), bzn, &map) {
        Resolution::Lua { path, .. } => MissionScript::Lua { path },
        Resolution::LegacyDll { files } => MissionScript::LegacyDll { files },
        Resolution::None => MissionScript::None,
    };
    let texts: Vec<Vec<u8>> = match &script {
        MissionScript::Lua { path } => lua_texts(src, path),
        MissionScript::LegacyDll { files } => legacy_files(files)
            .iter()
            .filter_map(|f| src.read(&format!("missions/{f}")).or_else(|| src.read(f)))
            .collect(),
        MissionScript::None => Vec::new(),
    };
    let mut names = Vec::new();
    for t in &texts {
        for n in file_names(t) {
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    let ends_with = |n: &String, ext: &str| n.to_ascii_lowercase().ends_with(ext);
    let objective_files: Vec<String> = names.iter().filter(|n| ends_with(n, ".txt")).cloned().collect();
    let movies = names.iter().filter(|n| ends_with(n, ".bik")).cloned().collect();
    let briefing = objective_files.iter().find_map(|f| read_briefing(src, f));
    let mut end_calls = Vec::new();
    for call in ["Mission:finish", "Mission.finish", "SucceedMission", "FailMission"] {
        if texts.iter().any(|t| contains(t, call.as_bytes())) {
            end_calls.push(call.to_owned());
        }
    }
    CampaignMission {
        index,
        bzn: bzn.to_owned(),
        title: labels.get(&map_file).first().cloned(),
        map_present: map_bytes.is_some(),
        script,
        objective_files,
        briefing,
        movies,
        end_calls,
    }
}

/// A Lua mission and every file it `include()`s: a mission may end or load
/// its objectives from an included file.
fn lua_texts(src: &dyn CampaignSource, path: &str) -> Vec<Vec<u8>> {
    let mut seen = vec![path.to_ascii_lowercase()];
    let mut todo = vec![path.to_owned()];
    let mut out = Vec::new();
    while let Some(p) = todo.pop() {
        let Some(text) = src.read(&p) else { continue };
        for name in includes(&text) {
            let name = name.replace('\\', "/");
            let Some(found) = [format!("missions/{name}"), name].into_iter().find(|c| src.exists(c)) else { continue };
            if !seen.contains(&found.to_ascii_lowercase()) && seen.len() < 64 {
                seen.push(found.to_ascii_lowercase());
                todo.push(found);
            }
        }
        out.push(text);
    }
    out
}

/// The names in `include("X")` / `include "X"` calls.
fn includes(text: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(text);
    let mut out = Vec::new();
    let mut rest = text.as_ref();
    while let Some(i) = rest.find("include") {
        rest = &rest[i + "include".len()..];
        let t = rest.trim_start().strip_prefix('(').unwrap_or(rest.trim_start()).trim_start();
        let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        if let Some(end) = t[1..].find(q) {
            out.push(t[1..1 + end].to_owned());
        }
    }
    out
}

/// The legacy files to scan: those the map names, and for an Armada II
/// `<name>R.drl` (rules) its `<name>S.dsl` (script), which holds the strings
/// (`docs/MISSIONS.md`).
fn legacy_files(files: &[String]) -> Vec<String> {
    let mut out = files.to_vec();
    for f in files {
        let lower = f.to_ascii_lowercase();
        if let Some(stem) = lower.strip_suffix("r.drl") {
            let dsl = format!("{}S.dsl", &f[..stem.len()]);
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&dsl)) {
                out.push(dsl);
            }
        }
    }
    out
}

/// `x.bzn` -> `x` (case-insensitive), any other name unchanged.
fn map_stem(name: &str) -> &str {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match name.len().checked_sub(4).and_then(|n| name.get(n..).map(|ext| (n, ext))) {
        Some((n, ext)) if ext.eq_ignore_ascii_case(".bzn") => &name[..n],
        _ => name,
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// File names (`*.txt`, `*.bik`) in a script or DLL: runs of name characters
/// ending in one of those extensions. Lua string literals and the DLLs'
/// embedded strings both appear this way.
fn file_names(bytes: &[u8]) -> Vec<String> {
    let is_name = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b' ');
    let mut out = Vec::new();
    for run in bytes.split(|&b| !is_name(b)) {
        let s = String::from_utf8_lossy(run);
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        if (lower.ends_with(".txt") || lower.ends_with(".bik")) && s.len() > 4 && !s.contains("  ") {
            // Quotes and brackets end a run, so a Lua literal arrives whole,
            // spaces included.
            out.push(s.to_owned());
        }
    }
    out
}

fn read_briefing(src: &dyn CampaignSource, file: &str) -> Option<Briefing> {
    let (path, bytes) = [format!("objectives/{file}"), file.to_owned()]
        .into_iter()
        .find_map(|p| src.read(&p).map(|b| (p, b)))?;
    let text = sta_content::strings::decode_text(&bytes);
    let parsed = crate::api::objectives_file::parse(text.as_bytes()).ok()?;
    Some(Briefing { file: path, title: parsed.title, text: parsed.briefing, goals: parsed.goals })
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

/// One campaign's progress, as the stock shell keeps it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CampaignState {
    /// The furthest mission reached, 0-based (the shell lists `reached + 1`).
    pub reached: usize,
    /// Its last mission has been won.
    pub complete: bool,
}

/// Progress through every campaign of a book.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CampaignProgress {
    /// [`CampaignBook::digest`] of the book this belongs to.
    pub book: u64,
    pub campaigns: BTreeMap<String, CampaignState>,
    /// State carried between missions by the host; no stock campaign has any.
    pub carried: BTreeMap<String, String>,
}

/// Why saved progress could not be restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressError {
    /// Not a progress file, or a malformed line (1-based).
    Format(usize, String),
    /// A version this build does not read.
    Version(u32),
    /// Saved for a different campaign list.
    Book { saved: u64, current: u64 },
}

impl std::fmt::Display for ProgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProgressError::Format(line, why) => write!(f, "campaign progress line {line}: {why}"),
            ProgressError::Version(v) => write!(f, "campaign progress version {v} is not supported"),
            ProgressError::Book { saved, current } => {
                write!(f, "campaign progress is for book {saved:016x}, the installed campaigns are {current:016x}")
            }
        }
    }
}

impl std::error::Error for ProgressError {}

const PROGRESS_MAGIC: &str = "sta64-campaign-progress";
/// The version [`CampaignProgress::save`] writes.
pub const PROGRESS_VERSION: u32 = 1;

impl CampaignProgress {
    /// A new player's progress: nothing reached; with `GIVE_ALL_MISSIONS`,
    /// everything (as the stock shell does on opening).
    pub fn new(book: &CampaignBook) -> CampaignProgress {
        let mut p = CampaignProgress {
            book: book.digest(),
            campaigns: book.campaigns.iter().map(|c| (c.id.clone(), CampaignState::default())).collect(),
            carried: BTreeMap::new(),
        };
        p.apply_give_all(book);
        p
    }

    /// `GIVE_ALL_MISSIONS`: every counter at its last mission, every campaign complete.
    pub fn apply_give_all(&mut self, book: &CampaignBook) {
        if !book.give_all_missions {
            return;
        }
        for c in &book.campaigns {
            self.campaigns.insert(
                c.id.clone(),
                CampaignState { reached: c.missions.len().saturating_sub(1), complete: !c.missions.is_empty() },
            );
        }
    }

    pub fn state(&self, campaign: &str) -> CampaignState {
        self.campaigns.get(campaign).copied().unwrap_or_default()
    }

    /// Whether `campaign` is open: it has missions and what it requires is complete.
    pub fn is_open(&self, book: &CampaignBook, campaign: &str) -> bool {
        book.campaign(campaign).is_some_and(|c| {
            !c.missions.is_empty() && c.requires.as_deref().is_none_or(|r| self.state(r).complete)
        })
    }

    /// The missions the shell lists for an open campaign (0..=reached).
    pub fn available(&self, book: &CampaignBook, campaign: &str) -> std::ops::Range<usize> {
        match book.campaign(campaign) {
            Some(c) if self.is_open(book, campaign) => 0..self.state(campaign).reached.saturating_add(1).min(c.missions.len()),
            _ => 0..0,
        }
    }

    /// The versioned text form: deterministic for equal progress. It carries
    /// [`CampaignBook::digest`], so progress saved before `mshell.set` (or the
    /// shell layout) changed is refused by [`CampaignProgress::restore_for`].
    pub fn save(&self) -> String {
        let mut s = format!("{PROGRESS_MAGIC} {PROGRESS_VERSION}\nbook {:016x}\n", self.book);
        for (id, st) in &self.campaigns {
            s += &format!("campaign {} {} {}\n", escape(id), st.reached, u8::from(st.complete));
        }
        for (k, v) in &self.carried {
            s += &format!("carried {} {}\n", escape(k), escape(v));
        }
        s += "end\n";
        s
    }

    /// Read [`CampaignProgress::save`]'s text back.
    pub fn restore(text: &str) -> Result<CampaignProgress, ProgressError> {
        let bad = |n: usize, why: &str| ProgressError::Format(n, why.to_owned());
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l));
        let (_, head) = lines.next().ok_or_else(|| bad(1, "empty"))?;
        let version = head
            .strip_prefix(PROGRESS_MAGIC)
            .and_then(|v| v.strip_prefix(' '))
            .and_then(|v| v.parse::<u32>().ok())
            .ok_or_else(|| bad(1, "not a campaign progress file"))?;
        if version != PROGRESS_VERSION {
            return Err(ProgressError::Version(version));
        }
        let mut p = CampaignProgress::default();
        let mut book = None;
        let mut ended = false;
        for (n, line) in lines {
            if ended {
                return Err(bad(n, "text after end"));
            }
            let f: Vec<&str> = line.split(' ').collect();
            match f.as_slice() {
                ["book", d] if book.is_none() => {
                    if d.len() != 16 || !d.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                        return Err(bad(n, "book digest is not 16 lower-case hex digits"));
                    }
                    book = Some(u64::from_str_radix(d, 16).map_err(|_| bad(n, "bad book digest"))?);
                }
                ["campaign", id, reached, complete] => {
                    let st = CampaignState {
                        reached: canonical_number(reached).ok_or_else(|| bad(n, "bad mission counter"))?,
                        complete: match *complete {
                            "0" => false,
                            "1" => true,
                            _ => return Err(bad(n, "complete is not 0 or 1")),
                        },
                    };
                    if p.campaigns.insert(unescape(id).ok_or_else(|| bad(n, "bad escape"))?, st).is_some() {
                        return Err(bad(n, "campaign listed twice"));
                    }
                }
                ["carried", k, v] => {
                    let k = unescape(k).ok_or_else(|| bad(n, "bad escape"))?;
                    if p.carried.insert(k, unescape(v).ok_or_else(|| bad(n, "bad escape"))?).is_some() {
                        return Err(bad(n, "carried key listed twice"));
                    }
                }
                ["end"] => ended = true,
                _ => return Err(bad(n, "unknown line")),
            }
        }
        if !ended {
            return Err(bad(text.lines().count(), "missing end"));
        }
        p.book = book.ok_or_else(|| bad(2, "missing book"))?;
        Ok(p)
    }

    /// [`CampaignProgress::restore`] for `book`: refuses another book's
    /// progress and fills campaigns the file does not list.
    pub fn restore_for(book: &CampaignBook, text: &str) -> Result<CampaignProgress, ProgressError> {
        let mut p = CampaignProgress::restore(text)?;
        if p.book != book.digest() {
            return Err(ProgressError::Book { saved: p.book, current: book.digest() });
        }
        p.fit(book);
        Ok(p)
    }

    /// Every campaign of `book` listed, each counter within its campaign.
    pub fn fit(&mut self, book: &CampaignBook) {
        for c in &book.campaigns {
            let st = self.campaigns.entry(c.id.clone()).or_default();
            st.reached = st.reached.min(c.missions.len().saturating_sub(1));
        }
    }
}

/// Percent-escape everything but `[A-Za-z0-9._-]` so fields never hold spaces.
fn escape(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
            o.push(b as char);
        } else {
            o += &format!("%{b:02X}");
        }
    }
    if o.is_empty() { "%".to_owned() } else { o }
}

/// Decimal digits without sign or leading zero.
fn canonical_number(s: &str) -> Option<usize> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))).then(|| s.parse().ok()).flatten()
}

/// The inverse of [`escape`], accepting only what `escape` writes.
fn unescape(s: &str) -> Option<String> {
    let out = unescape_any(s)?;
    (escape(&out) == s).then_some(out)
}

fn unescape_any(s: &str) -> Option<String> {
    if s == "%" {
        return Some(String::new());
    }
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

// ---------------------------------------------------------------------------
// The runner
// ---------------------------------------------------------------------------

/// How a mission ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissionOutcome {
    pub won: bool,
    /// The delay the script asked for before the mission closes, in seconds
    /// (`Mission:finish(success, delay)`; the host decides how to show it).
    pub delay: f64,
    /// The tick the script ended the mission on.
    pub tick: u64,
}

/// What follows a finished mission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// A win with another mission after it: go on to that mission's briefing.
    Next { campaign: String, mission: usize },
    /// The campaign's last mission was won. `unlocked` lists campaigns that
    /// opened with it.
    CampaignComplete { campaign: String, unlocked: Vec<String> },
    /// A loss: back to the campaign's mission list, progress unchanged.
    Lost { campaign: String, mission: usize },
}

/// What [`CampaignRunner::start`] needs from the host: the game to start and
/// how to read the script.
pub trait MissionHost {
    /// The game for `mission`: map, content and seats.
    fn game_setup(&self, campaign: &Campaign, mission: &CampaignMission, seed: u64) -> Result<GameSetup, String>;

    /// The Lua script to run as the mission's controller, and its reader
    /// ([`resolved_script`] runs the one the book resolved).
    fn script(&self, _campaign: &Campaign, mission: &CampaignMission) -> Result<(String, ScriptReader), String>;

    /// The map's named paths `(label, points)`, in file order, for `A2.paths`.
    fn map_paths(&self, _campaign: &Campaign, _mission: &CampaignMission) -> Vec<(String, Vec<[f32; 3]>)> {
        Vec::new()
    }

    /// The seat the mission runs as and its player numbering.
    fn seat(&self) -> (PlayerId, PlayerNumbering) {
        (PlayerId(0), PlayerNumbering::Slots)
    }

    /// The AIPs scripts may load (`AI/AIPs/*.aip`); none by default, so a
    /// host without them leaves the mission's AI seats passive.
    fn aip_library(&self) -> sta_ai::AipLibrary {
        sta_ai::AipLibrary::new()
    }
}

/// Reads a mission script and its `include()`s by VFS-style path.
pub type ScriptReader = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// Why a mission could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    UnknownCampaign(String),
    NoSuchMission { campaign: String, mission: usize },
    /// The campaign is locked or the mission not reached yet.
    Locked { campaign: String, mission: usize },
    /// The host or the mission cannot run it (see [`CampaignBook::blocked`]).
    Blocked(String),
    /// A mission is still active: [`CampaignRunner::finish`] or
    /// [`CampaignRunner::abandon`] it first.
    Busy,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::UnknownCampaign(c) => write!(f, "no campaign {c}"),
            StartError::NoSuchMission { campaign, mission } => write!(f, "{campaign} has no mission {mission}"),
            StartError::Locked { campaign, mission } => write!(f, "{campaign} mission {mission} is locked"),
            StartError::Blocked(why) => write!(f, "cannot run: {why}"),
            StartError::Busy => write!(f, "a mission is still active"),
        }
    }
}

impl std::error::Error for StartError {}

/// A mission being played: its sim and its script.
pub struct ActiveMission {
    pub campaign: String,
    pub mission: usize,
    sim: Sim,
    runtime: MissionRuntime,
    events: Vec<SimEvent>,
    outcome: Option<MissionOutcome>,
    /// Commands the runner submitted each tick besides the script's (host input).
    pending: Vec<Command>,
    /// The AI of every seat but the human's: passive until the script loads
    /// an AIP for it (AI-06).
    ai_hub: sta_ai::MissionAiHub,
    ai_seats: Vec<Box<dyn sta_sim::setup::Controller>>,
}

/// Wraps the mission-ending calls so the runner sees the outcome. Runs once
/// after the script has loaded; names that do not exist yet are skipped, so
/// the A2 / A1 bindings are picked up as soon as they are merged. Blind spots:
/// a call during the script's top-level load, and a function reference the
/// script kept before the hook; the runtime's own finished flag catches the
/// latter for `Mission.finish` (counted as a loss, result unknown). Success
/// follows Lua truthiness.
const OUTCOME_HOOK: &str = r#"
local function record(won, delay)
  if rawget(_G, "__sta64_outcome") == nil then
    rawset(_G, "__sta64_outcome", { won = won, delay = tonumber(delay) or 0 })
  end
end
local function wrap(t, name, fixed)
  if type(t) ~= "table" then return end
  local f = rawget(t, name)
  if type(f) ~= "function" then return end
  rawset(t, name, function(...)
    local a, b = ...
    if a == t then a, b = select(2, ...) end
    if fixed == nil then record(a ~= nil and a ~= false, b) else record(fixed, a) end
    return f(...)
  end)
end
wrap(rawget(_G, "Mission"), "finish", nil)
for _, t in ipairs({ _G, rawget(_G, "A2"), rawget(_G, "A1") }) do
  wrap(t, "SucceedMission", true)
  wrap(t, "FailMission", false)
end
"#;

/// The outcome a runtime that carries [`launch_runtime`]'s hook has reached, if
/// any, as of `tick`. A host that owns the sim itself (the client's link) asks
/// this each tick instead of using [`ActiveMission`].
pub fn scan_outcome(runtime: &MissionRuntime, tick: u64) -> Option<MissionOutcome> {
    let got: mlua::Result<(Option<bool>, Option<f64>)> =
        runtime.eval("local o = rawget(_G, '__sta64_outcome') if o then return o.won, o.delay end");
    if let Ok((Some(won), delay)) = got {
        Some(MissionOutcome { won, delay: delay.unwrap_or(0.0), tick })
    } else if runtime.finished() {
        // The script reached the real finish without passing the hook
        // (it kept a reference from before the hook was installed):
        // the result is unknown, so it never advances progress.
        Some(MissionOutcome { won: false, delay: 0.0, tick })
    } else {
        None
    }
}

/// Load a mission script over a game that is already built: the runtime of
/// `path` (read through `reader`) for `seat`, with the sim's labels, the map's
/// named `paths` ([`read_map_paths`]) and first view, and [`scan_outcome`]'s hook installed. [`CampaignRunner::start`] uses
/// it; so does a host that owns its own sim (the client, INT-M6).
pub fn launch_runtime(
    path: &str,
    reader: ScriptReader,
    seed: u64,
    seat: (PlayerId, PlayerNumbering),
    setup: &GameSetup,
    sim: &Sim,
    paths: Vec<(String, Vec<[f32; 3]>)>,
) -> Result<MissionRuntime, StartError> {
    let cfg = MissionConfig {
        seat: Some(seat.0),
        player_numbering: seat.1,
        content: Some(setup.content.clone()),
        labels: sim.labels.iter().map(|(id, l)| (id, l.clone())).collect(),
        paths,
        view: Some((*sim.view()).clone()),
    };
    let runtime = MissionRuntime::load_with(path, seed, cfg, move |p| reader(p));
    if let Some(e) = runtime.errors().first() {
        return Err(StartError::Blocked(format!("{path} did not load: {e}")));
    }
    runtime.eval::<()>(OUTCOME_HOOK).map_err(|e| StartError::Blocked(format!("outcome hook: {e}")))?;
    Ok(runtime)
}

/// The named paths of the map `bzn` (`a2_fed03.bzn`) in `vfs`, in file order,
/// for `A2.paths` / `A1.paths`; empty when the map cannot be read or holds
/// none. An Armada I map keeps its paths in its own section (`Map::a1`).
pub fn read_map_paths(vfs: &Vfs, bzn: &str) -> Vec<(String, Vec<[f32; 3]>)> {
    let Some(Ok(bytes)) = vfs.read(&format!("bzn/{bzn}")) else { return Vec::new() };
    let Ok(m) = sta_bzn::parse(&bytes) else { return Vec::new() };
    if m.paths.is_empty()
        && let Some(a1) = m.a1
    {
        return a1.paths.into_iter().map(|p| (p.label, p.points)).collect();
    }
    m.paths.into_iter().map(|p| (p.label, p.points)).collect()
}

impl ActiveMission {
    /// The mission's script runtime (objectives, briefing, errors, `eval`).
    pub fn runtime(&self) -> &MissionRuntime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut MissionRuntime {
        &mut self.runtime
    }

    /// AIP loads the script asked for that named no file of the host's AIP
    /// library, so changed nothing (AI-06).
    pub fn unresolved_aips(&self) -> Vec<(PlayerId, String)> {
        self.ai_hub.unresolved()
    }

    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// Queue commands (player input) for the next tick.
    pub fn submit(&mut self, commands: impl IntoIterator<Item = Command>) {
        self.pending.extend(commands);
    }

    pub fn outcome(&self) -> Option<MissionOutcome> {
        self.outcome
    }

    /// One tick: the script thinks over the last view and events, then the
    /// sim steps its commands. Returns the outcome once the script has ended
    /// the mission.
    pub fn tick(&mut self) -> Option<MissionOutcome> {
        if self.outcome.is_some() {
            return self.outcome;
        }
        let view = self.sim.view();
        let mut commands = std::mem::take(&mut self.pending);
        self.runtime.think(&view, &self.events, &mut commands);
        // The script runs first, so a plan it loads this tick applies this tick.
        self.ai_hub.submit(self.runtime.take_aip_requests());
        for ai in &mut self.ai_seats {
            ai.think(&view, &self.events, &mut commands);
        }
        commands.sort_by_key(|c| (c.player, c.seq));
        self.check_outcome(view.tick);
        let out = self.sim.step(&commands);
        self.events = out.events;
        self.outcome
    }

    /// Look for an ended mission now (also after a host `eval`).
    pub fn check_outcome(&mut self, tick: u64) -> Option<MissionOutcome> {
        if self.outcome.is_none() {
            self.outcome = scan_outcome(&self.runtime, tick);
        }
        self.outcome
    }

    /// Tick until the mission ends or `max_ticks` pass.
    pub fn run(&mut self, max_ticks: u64) -> Option<MissionOutcome> {
        for _ in 0..max_ticks {
            if let Some(o) = self.tick() {
                return Some(o);
            }
        }
        self.outcome
    }
}

/// Plays one install's campaigns: starts missions, applies their outcomes to
/// the progress and picks what comes next.
pub struct CampaignRunner {
    pub book: CampaignBook,
    pub progress: CampaignProgress,
    active: Option<ActiveMission>,
}

impl CampaignRunner {
    /// A runner with a new player's progress.
    pub fn new(book: CampaignBook) -> CampaignRunner {
        let progress = CampaignProgress::new(&book);
        CampaignRunner { book, progress, active: None }
    }

    /// A runner over saved progress.
    pub fn with_progress(book: CampaignBook, mut progress: CampaignProgress) -> CampaignRunner {
        progress.fit(&book);
        CampaignRunner { book, progress, active: None }
    }

    pub fn active(&self) -> Option<&ActiveMission> {
        self.active.as_ref()
    }

    pub fn active_mut(&mut self) -> Option<&mut ActiveMission> {
        self.active.as_mut()
    }

    /// Start `mission` of `campaign` (an open campaign, a mission reached).
    pub fn start(&mut self, host: &dyn MissionHost, campaign: &str, mission: usize, seed: u64) -> Result<&mut ActiveMission, StartError> {
        if self.active.is_some() {
            return Err(StartError::Busy);
        }
        let c = self.book.campaign(campaign).ok_or_else(|| StartError::UnknownCampaign(campaign.to_owned()))?;
        let m = c
            .missions
            .get(mission)
            .ok_or_else(|| StartError::NoSuchMission { campaign: campaign.to_owned(), mission })?;
        if !self.progress.available(&self.book, campaign).contains(&mission) {
            return Err(StartError::Locked { campaign: campaign.to_owned(), mission });
        }
        let (path, reader) = host.script(c, m).map_err(StartError::Blocked)?;
        let setup = host.game_setup(c, m, seed).map_err(StartError::Blocked)?;
        let sim = Sim::from_setup(&setup);
        let runtime = launch_runtime(&path, reader, seed, host.seat(), &setup, &sim, host.map_paths(c, m))?;
        let ai_hub = sta_ai::MissionAiHub::new(host.aip_library());
        self.active = Some(ActiveMission {
            campaign: campaign.to_owned(),
            mission,
            sim,
            runtime,
            events: Vec::new(),
            outcome: None,
            pending: Vec::new(),
            ai_seats: setup
                .players
                .iter()
                .filter(|p| !matches!(p.control, Control::Human))
                .map(|p| ai_hub.controller(&setup, p.id))
                .collect(),
            ai_hub,
        });
        Ok(self.active.as_mut().expect("just set"))
    }

    /// Close the active mission with its outcome (the script's, or one the
    /// host decides, e.g. an abort as a loss) and say what comes next.
    pub fn finish(&mut self, outcome: MissionOutcome) -> Option<Transition> {
        let a = self.active.take()?;
        self.record(&a.campaign, a.mission, outcome.won).ok()
    }

    /// Drop the active mission unfinished (the player aborts): a loss.
    pub fn abandon(&mut self) -> Option<Transition> {
        let a = self.active.take()?;
        self.record(&a.campaign, a.mission, false).ok()
    }

    /// Apply a mission's result to the progress (the stock shell's rule).
    pub fn record(&mut self, campaign: &str, mission: usize, won: bool) -> Result<Transition, StartError> {
        let count = self
            .book
            .campaign(campaign)
            .ok_or_else(|| StartError::UnknownCampaign(campaign.to_owned()))?
            .missions
            .len();
        if mission >= count {
            return Err(StartError::NoSuchMission { campaign: campaign.to_owned(), mission });
        }
        if !won {
            return Ok(Transition::Lost { campaign: campaign.to_owned(), mission });
        }
        let open_before: Vec<String> = self.open_campaigns();
        let st = self.progress.campaigns.entry(campaign.to_owned()).or_default();
        let last = mission + 1 >= count;
        if mission == st.reached && !last {
            st.reached += 1;
        }
        if last {
            st.complete = true;
        }
        if last {
            let unlocked = self.open_campaigns().into_iter().filter(|c| !open_before.contains(c)).collect();
            Ok(Transition::CampaignComplete { campaign: campaign.to_owned(), unlocked })
        } else {
            Ok(Transition::Next { campaign: campaign.to_owned(), mission: mission + 1 })
        }
    }

    /// Campaigns open now, in shell order.
    pub fn open_campaigns(&self) -> Vec<String> {
        self.book
            .campaigns
            .iter()
            .filter(|c| self.progress.is_open(&self.book, &c.id))
            .map(|c| c.id.clone())
            .collect()
    }
}

/// Default [`MissionHost::script`]: the resolved Lua through `reader`.
pub fn resolved_script(mission: &CampaignMission, reader: ScriptReader) -> Result<(String, ScriptReader), String> {
    match &mission.script {
        MissionScript::Lua { path } => Ok((path.clone(), reader)),
        MissionScript::LegacyDll { files } => Err(format!("{} has only legacy DLL rules ({})", mission.bzn, files.join(", "))),
        MissionScript::None => Err(format!("{} has no mission script", mission.bzn)),
    }
}

/// A host over a mounted install: the mission's map from its BZN, installed
/// content, eight seats (seat 0 human, the others idle for the script to
/// drive), Armada team numbering as the converted missions use.
pub struct VfsHost {
    pub vfs: Arc<Vfs>,
    pub content: Arc<ContentDb>,
    pub index: Arc<OdfIndex>,
}

impl VfsHost {
    /// Index and load the install's content.
    pub fn load(vfs: Arc<Vfs>) -> Result<VfsHost, String> {
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).map_err(|e| format!("content: {e}"))?;
        Ok(VfsHost { vfs, content: Arc::new(db), index: Arc::new(index) })
    }

    pub fn reader(&self) -> ScriptReader {
        let vfs = self.vfs.clone();
        Arc::new(move |p| vfs.read(p).and_then(Result::ok))
    }
}

impl MissionHost for VfsHost {
    fn game_setup(&self, _campaign: &Campaign, mission: &CampaignMission, seed: u64) -> Result<GameSetup, String> {
        let path = format!("bzn/{}", mission.bzn);
        let bytes = self.vfs.read(&path).ok_or_else(|| format!("{path} missing"))?.map_err(|e| format!("{path}: {e}"))?;
        let bzn = sta_bzn::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
        let (map, _) = MapDef::from_bzn_in(&bzn, &self.content, &self.vfs, &self.index);
        Ok(map_setup(map, self.content.clone(), seed))
    }

    fn script(&self, _campaign: &Campaign, mission: &CampaignMission) -> Result<(String, ScriptReader), String> {
        resolved_script(mission, self.reader())
    }

    fn map_paths(&self, _campaign: &Campaign, mission: &CampaignMission) -> Vec<(String, Vec<[f32; 3]>)> {
        read_map_paths(&self.vfs, &mission.bzn)
    }

    fn seat(&self) -> (PlayerId, PlayerNumbering) {
        (PlayerId(0), PlayerNumbering::ArmadaTeams)
    }

    fn aip_library(&self) -> sta_ai::AipLibrary {
        let names = self.vfs.list("AI/AIPs");
        sta_ai::AipLibrary::load(&names, &mut |n| {
            let bytes = self.vfs.read(n)?.ok()?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        })
    }
}

/// Eight seats over `map`: each seat's race is that of the first unit it
/// owns on the map (as the A2 startup probe does), seat 0 is the human.
pub fn map_setup(map: MapDef, content: Arc<ContentDb>, seed: u64) -> GameSetup {
    let players = (0..8u8)
        .map(|id| {
            let race = map
                .objects
                .iter()
                .filter(|o| o.owner == Some(id))
                .filter_map(|o| content.key(&o.odf).and_then(|k| content.unit(k)))
                .map(|u| u.race.clone())
                .find(|r| !r.is_empty())
                .unwrap_or_default();
            PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race,
                control: if id == 0 { Control::Human } else { Control::Idle },
            }
        })
        .collect();
    GameSetup { seed, map: Arc::new(map), content, players, options: GameOptions::default() }
}

#[cfg(test)]
#[path = "campaign/tests.rs"]
mod tests;
