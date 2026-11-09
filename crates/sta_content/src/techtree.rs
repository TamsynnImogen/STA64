//! Tech trees and build lists (CON-06): what a player may build, given what
//! they already own. See `docs/data/techtree.md` for the file forms.
//!
//! Two on-disk forms feed one model:
//!
//! - `.tt` (Armada I and II, and Fleet Operations' fallback): one entry per
//!   line, `<odf> <N> <prerequisite odf>x N`. `0` means buildable from the
//!   start, `N > 0` needs all `N` listed ODFs to exist, `-1` and `-2` withhold
//!   the item (mission techtrees use `-1`); see [`TechEntry::dependency_build_possible`]. `#include other.tt` pulls in a base
//!   file; a file's own entries override what it includes. `//` and `/* */`
//!   are comments.
//! - `.xml` (Fleet Operations): `<object ODF="x.odf">` with `<requirement>`
//!   children, and `<capacitylist>` tables.
//!
//! A `.tt` entry's prerequisites become one [`Requirement`] of
//! [`Condition::Have`]s, so there is a single evaluator ([`TechTree::availability`]).
//!
//! How the XML is read (inferred from the installed files; the game's own
//! code was not consulted):
//!
//! - Requirements of an object without a `logicGroups` attribute must all
//!   hold. Requirements sharing a `logicGroups` value form one group whose
//!   members must all hold; if an object has any groups, at least one group
//!   must hold. So distinct groups are alternatives (a shared "dummy" object
//!   unlocks when any race's upgrade exists) and one group is a plain AND.
//! - A requirement's conditions all hold together. `race="x"` on a
//!   requirement is a condition too: the player must be race `x`.
//! - `<obj>` needs the named ODF owned (`mustNotHave="true"` inverts it),
//!   `<ally race=…>` needs an ally of that race, `<capList>` names a
//!   `<capacitylist>`: it holds while the owned members' `capValue`s, each
//!   times its count, sum to less than `maxCapValue`. `<cap/>` with no list
//!   is a unique-research cap: the object itself counts against its own
//!   `maxCapValue` (1 when absent).
//! - `buildPossible="false"` withholds the object. `dependencyBuildPossible="false"`
//!   blocks everything that needs it, even when the player has it (a `.tt`
//!   `-1`; `-2` withholds but still lets dependents build, see tech1.tt).
//! - `buttonHideUnavailable="true"` tells the UI to hide the button instead
//!   of greying it ([`Availability::Locked::hidden`]).
//!
//! Names are ODF stems, lower-cased without `.odf`, as everywhere in the
//! content seam. An ODF the tree does not list is [`Availability::NotListed`];
//! whether that blocks building is the caller's call (the sim treats it as
//! blocked).

use std::collections::{BTreeMap, BTreeSet};

use crate::{ContentDb, Warning};

/// Lower-cased ODF stem: `"FBase.odf"` -> `"fbase"`.
pub fn stem(name: &str) -> String {
    let n = name.trim().rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    match n.strip_suffix(".odf") {
        Some(s) => s.to_owned(),
        None => n,
    }
}

/// One thing a requirement checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// The player owns (or is building, as the caller counts) the ODF.
    /// `must_not`: it must not.
    Have { odf: String, must_not: bool },
    /// The player has an ally of this race.
    Ally(String),
    /// The player is this race.
    Race(String),
    /// Unique research: the object counts against its own cap.
    Unique { max: i64, tooltip: String, tooltip_reached: String, tooltip_over: String },
    /// A named [`CapacityList`] still has room.
    CapList(String),
}

/// A set of conditions that hold together.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Requirement {
    /// `logicGroups`; requirements of one group form an AND, groups are ORed.
    pub group: Option<u32>,
    pub hide_unavailable: bool,
    pub conditions: Vec<Condition>,
}

/// A `<capacitylist>`: a shared limit (special stations, unique research).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapacityList {
    pub name: String,
    pub max: i64,
    pub tooltip: String,
    pub tooltip_reached: String,
    pub tooltip_over: String,
    /// `(odf, capValue)` in file order.
    pub members: Vec<(String, i64)>,
}

/// One buildable thing's gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TechEntry {
    /// ODF stem.
    pub odf: String,
    /// `-1` or `-2` in a `.tt`, `buildPossible="false"` in XML.
    pub withheld: bool,
    /// False for `-1` in a `.tt` and `dependencyBuildPossible="false"` in
    /// XML: nothing that needs this ODF can be built, even if the player has
    /// it. `-2` withholds the ODF but keeps this true ("if somehow I already
    /// existed, you could build anything dependent on me", tech1.tt).
    /// Default true.
    pub dependency_build_possible: bool,
    pub requirements: Vec<Requirement>,
}

impl TechEntry {
    /// ODFs this entry directly needs owned (`Have` conditions that are not
    /// `must_not`), in file order, without duplicates. For a `.tt` entry that
    /// is exactly its prerequisite list.
    pub fn prerequisites(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for c in self.requirements.iter().flat_map(|r| &r.conditions) {
            if let Condition::Have { odf, must_not: false } = c
                && !out.contains(&odf.as_str())
            {
                out.push(odf);
            }
        }
        out
    }
}

/// What a player has, as the tech tree asks it. The sim implements this over
/// its entities; [`Owned`] is a plain value for tests and the AI's planning.
pub trait TechState {
    /// How many of this ODF stem the player has.
    fn count(&self, odf: &str) -> u32;
    /// The player's race (lower-case, e.g. `borg`, `unitedfederation`).
    fn race(&self) -> &str;
    /// Whether the player has an ally of this race.
    fn has_ally(&self, race: &str) -> bool;
}

/// A [`TechState`] held as plain data.
#[derive(Debug, Clone, Default)]
pub struct Owned {
    pub race: String,
    pub allies: BTreeSet<String>,
    counts: BTreeMap<String, u32>,
}

impl Owned {
    pub fn new(race: &str) -> Owned {
        Owned { race: race.to_ascii_lowercase(), ..Default::default() }
    }

    /// Own `n` more of `odf`.
    pub fn with(mut self, odf: &str, n: u32) -> Owned {
        *self.counts.entry(stem(odf)).or_default() += n;
        self
    }

    pub fn with_ally(mut self, race: &str) -> Owned {
        self.allies.insert(race.to_ascii_lowercase());
        self
    }
}

impl TechState for Owned {
    fn count(&self, odf: &str) -> u32 {
        self.counts.get(odf).copied().unwrap_or(0)
    }
    fn race(&self) -> &str {
        &self.race
    }
    fn has_ally(&self, race: &str) -> bool {
        self.allies.contains(race)
    }
}

/// Why a requirement is not met.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unmet {
    /// Needs this ODF.
    Missing(String),
    /// Needs this ODF, which the tree marks as never obtainable by building
    /// (`-1`, `dependencyBuildPossible="false"`).
    Unobtainable(String),
    /// Must not have this ODF.
    Forbidden(String),
    /// Needs an ally of this race.
    NoAlly(String),
    /// Only this race can build it.
    WrongRace(String),
    /// A capacity list is full.
    CapacityFull(String),
    /// Unique research already done.
    AlreadyResearched,
}

/// Whether an ODF may be built now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// `-1` / `buildPossible="false"`.
    Withheld,
    /// The tree has no entry for it.
    NotListed,
    /// Requirements fail. `unmet` lists every failing condition of the
    /// ungrouped requirements and, when no group holds, of every group.
    /// `hidden`: a failing requirement asked the button to be hidden.
    Locked { unmet: Vec<Unmet>, hidden: bool },
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Availability::Available)
    }
}

/// One slot of a builder's build list with its current availability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOption {
    pub def: crate::DefRef,
    pub availability: Availability,
}

/// A parsed tech tree: entries by ODF stem and capacity lists by name.
#[derive(Debug, Clone, Default)]
pub struct TechTree {
    pub name: String,
    entries: BTreeMap<String, TechEntry>,
    caps: BTreeMap<String, CapacityList>,
    files: Vec<String>,
}

impl TechTree {
    /// Load a `.tt` (with its `#include`s) or `.xml` by file name; `load`
    /// maps a name (as written, e.g. `tech1.tt`) to its text. Missing
    /// includes, include loops and malformed lines are warnings.
    pub fn load(name: &str, load: &mut dyn FnMut(&str) -> Option<String>) -> (TechTree, Vec<Warning>) {
        let mut tree = TechTree { name: name.to_owned(), ..Default::default() };
        let mut warnings = Vec::new();
        match load(name) {
            Some(text) => {
                if name.to_ascii_lowercase().ends_with(".xml") {
                    tree.files.push(name.to_owned());
                    parse_xml(name, &text, &mut tree, &mut warnings);
                } else {
                    let mut stack = vec![name.to_ascii_lowercase()];
                    apply_tt(name, &text, load, &mut tree, &mut stack, &mut warnings);
                }
            }
            None => warnings.push(Warning::new(name, "not found")),
        }
        tree.finish(&mut warnings);
        (tree, warnings)
    }

    /// Parse one `.tt` or `.xml` text with no includes available.
    pub fn parse(name: &str, text: &str) -> (TechTree, Vec<Warning>) {
        let mut first = Some(text.to_owned());
        Self::load(name, &mut |n| if n == name { first.take() } else { None })
    }

    /// File names that contributed, the root first.
    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entry for an ODF (name with or without `.odf`, any case).
    pub fn entry(&self, odf: &str) -> Option<&TechEntry> {
        self.entries.get(&stem(odf))
    }

    /// Entries sorted by ODF stem.
    pub fn entries(&self) -> impl Iterator<Item = &TechEntry> {
        self.entries.values()
    }

    pub fn capacity_list(&self, name: &str) -> Option<&CapacityList> {
        self.caps.get(&name.to_ascii_lowercase())
    }

    pub fn capacity_lists(&self) -> impl Iterator<Item = &CapacityList> {
        self.caps.values()
    }

    /// ODFs `odf` directly needs owned (see [`TechEntry::prerequisites`]).
    pub fn prerequisites(&self, odf: &str) -> Vec<&str> {
        self.entry(odf).map(TechEntry::prerequisites).unwrap_or_default()
    }

    /// Every ODF that must exist, directly or through other prerequisites,
    /// before `odf` is buildable, nearest first, without duplicates. Follows
    /// `Have` conditions only, so alternatives (XML groups) are all listed.
    pub fn prerequisites_deep(&self, odf: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut queue = std::collections::VecDeque::from([stem(odf)]);
        let mut seen = BTreeSet::from([stem(odf)]);
        while let Some(n) = queue.pop_front() {
            for p in self.prerequisites(&n) {
                if seen.insert(p.to_owned()) {
                    out.push(p.to_owned());
                    queue.push_back(p.to_owned());
                }
            }
        }
        out
    }

    /// Whether `odf` may be built by a player in `state`.
    pub fn availability(&self, odf: &str, state: &dyn TechState) -> Availability {
        let me = stem(odf);
        let Some(entry) = self.entries.get(&me) else { return Availability::NotListed };
        if entry.withheld {
            return Availability::Withheld;
        }
        let mut unmet = Vec::new();
        let mut hidden = false;
        let mut groups: BTreeMap<u32, Vec<&Requirement>> = BTreeMap::new();
        for r in &entry.requirements {
            match r.group {
                Some(g) => groups.entry(g).or_default().push(r),
                None => {
                    let u = self.unmet_of(r, &me, state);
                    hidden |= r.hide_unavailable && !u.is_empty();
                    unmet.extend(u);
                }
            }
        }
        if !groups.is_empty() {
            let mut group_unmet = Vec::new();
            let mut group_hidden = false;
            let mut any_holds = false;
            for rs in groups.values() {
                let mut u = Vec::new();
                let mut h = false;
                for r in rs {
                    let m = self.unmet_of(r, &me, state);
                    h |= r.hide_unavailable && !m.is_empty();
                    u.extend(m);
                }
                if u.is_empty() {
                    any_holds = true;
                    break;
                }
                group_hidden |= h;
                group_unmet.extend(u);
            }
            if !any_holds {
                hidden |= group_hidden;
                unmet.extend(group_unmet);
            }
        }
        unmet.dedup();
        if unmet.is_empty() {
            Availability::Available
        } else {
            Availability::Locked { unmet, hidden }
        }
    }

    /// A builder's build list, in button order, each with its availability.
    /// `builder` is a unit ODF (any case, `.odf` optional) of `db`; an
    /// unknown builder has an empty list.
    pub fn build_options(&self, db: &ContentDb, builder: &str, state: &dyn TechState) -> Vec<BuildOption> {
        let Some(unit) = db.key(&stem(builder)).and_then(|k| db.unit(k)) else { return Vec::new() };
        unit.build
            .list
            .iter()
            .map(|d| BuildOption { def: d.clone(), availability: self.availability(&d.odf, state) })
            .collect()
    }

    /// Entries and prerequisites that name no definition of `db`, as
    /// warnings (cap lists and race/ally names are not ODFs and skipped).
    pub fn check_against(&self, db: &ContentDb) -> Vec<Warning> {
        let mut out = Vec::new();
        for e in self.entries.values() {
            if db.key(&e.odf).is_none() {
                out.push(Warning::new(&e.odf, "tech tree entry has no definition"));
            }
            for p in e.prerequisites() {
                if db.key(p).is_none() {
                    out.push(Warning::new(&e.odf, format!("prerequisite {p} has no definition")));
                }
            }
        }
        out
    }

    fn unmet_of(&self, r: &Requirement, me: &str, state: &dyn TechState) -> Vec<Unmet> {
        let mut out = Vec::new();
        for c in &r.conditions {
            match c {
                Condition::Have { odf, must_not } => {
                    let has = state.count(odf) > 0;
                    if !*must_not && self.entries.get(odf).is_some_and(|e| !e.dependency_build_possible) {
                        out.push(Unmet::Unobtainable(odf.clone()));
                    } else if *must_not && has {
                        out.push(Unmet::Forbidden(odf.clone()));
                    } else if !*must_not && !has {
                        out.push(Unmet::Missing(odf.clone()));
                    }
                }
                Condition::Ally(race) => {
                    if !state.has_ally(race) {
                        out.push(Unmet::NoAlly(race.clone()));
                    }
                }
                Condition::Race(race) => {
                    if state.race() != race {
                        out.push(Unmet::WrongRace(race.clone()));
                    }
                }
                Condition::Unique { max, .. } => {
                    if i64::from(state.count(me)) >= *max {
                        out.push(Unmet::AlreadyResearched);
                    }
                }
                Condition::CapList(name) => {
                    // An undefined list was warned about at load; it imposes no limit.
                    if let Some(list) = self.caps.get(name) {
                        let used: i64 = list.members.iter().map(|(o, v)| v * i64::from(state.count(o))).sum();
                        if used >= list.max {
                            out.push(Unmet::CapacityFull(name.clone()));
                        }
                    }
                }
            }
        }
        out
    }

    fn finish(&mut self, warnings: &mut Vec<Warning>) {
        let mut missing = BTreeSet::new();
        for e in self.entries.values() {
            for c in e.requirements.iter().flat_map(|r| &r.conditions) {
                if let Condition::CapList(n) = c
                    && !self.caps.contains_key(n)
                {
                    missing.insert((e.odf.clone(), n.clone()));
                }
            }
        }
        for (odf, n) in missing {
            warnings.push(Warning::new(odf, format!("capacity list {n} is not defined")));
        }
    }
}

// ---------------------------------------------------------------- .tt

/// Drops `//` comments and `/* */` blocks (a `//` wins over a `/*` inside it,
/// as in `//***[ BORG ]***`), keeping line breaks.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let line = rest.find("//");
        let block = rest.find("/*");
        match (line, block) {
            (Some(l), b) if b.is_none_or(|b| l < b) => {
                out.push_str(&rest[..l]);
                rest = rest[l..].find('\n').map_or("", |n| &rest[l + n..]);
            }
            (_, Some(b)) => {
                out.push_str(&rest[..b]);
                out.push(' ');
                match rest[b + 2..].find("*/") {
                    Some(j) => {
                        // Keep the block's line breaks so line counts hold.
                        out.push_str(&"\n".repeat(rest[b..b + 2 + j].matches('\n').count()));
                        rest = &rest[b + 2 + j + 2..];
                    }
                    None => return out,
                }
            }
            _ => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

fn apply_tt(
    name: &str,
    text: &str,
    load: &mut dyn FnMut(&str) -> Option<String>,
    tree: &mut TechTree,
    stack: &mut Vec<String>,
    warnings: &mut Vec<Warning>,
) {
    tree.files.push(name.to_owned());
    let text = strip_comments(text);
    let mut own: BTreeSet<String> = BTreeSet::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let rest = rest.trim_start();
            if let Some(inc) = rest.strip_prefix("include") {
                let inc = inc.trim().trim_matches(|c| c == '"' || c == '<' || c == '>');
                let key = inc.to_ascii_lowercase();
                if stack.contains(&key) {
                    warnings.push(Warning::new(name, format!("include loop: {inc}")));
                } else {
                    match load(inc) {
                        Some(t) => {
                            stack.push(key);
                            apply_tt(inc, &t, load, tree, stack, warnings);
                            stack.pop();
                        }
                        None => warnings.push(Warning::new(name, format!("include not found: {inc}"))),
                    }
                }
            }
            // Any other `#` line is a comment.
            continue;
        }
        let mut tok = line.split_whitespace();
        let (Some(odf), Some(n)) = (tok.next(), tok.next()) else {
            warnings.push(Warning::new(name, format!("unrecognised line: {line}")));
            continue;
        };
        let Ok(n) = n.parse::<i64>() else {
            warnings.push(Warning::new(name, format!("unrecognised line: {line}")));
            continue;
        };
        let deps: Vec<String> = tok.map(stem).collect();
        let key = stem(odf);
        if n > 0 && deps.len() != n as usize {
            warnings.push(Warning::new(name, format!("{key}: says {n} prerequisites, lists {}", deps.len())));
        }
        if n <= 0 && !deps.is_empty() {
            warnings.push(Warning::new(name, format!("{key}: lists prerequisites but says {n}")));
        }
        if !own.insert(key.clone()) {
            warnings.push(Warning::new(name, format!("{key}: listed twice, the later line wins")));
        }
        let requirements = if n > 0 && !deps.is_empty() {
            vec![Requirement {
                group: None,
                hide_unavailable: false,
                conditions: deps.into_iter().map(|odf| Condition::Have { odf, must_not: false }).collect(),
            }]
        } else {
            Vec::new()
        };
        tree.entries.insert(
            key.clone(),
            TechEntry { odf: key, withheld: n < 0, dependency_build_possible: n != -1, requirements },
        );
    }
}

// ---------------------------------------------------------------- .xml

/// A parsed XML element: just what the tech tree files use.
#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn flag(&self, name: &str, default: bool) -> bool {
        match self.attr(name).map(|v| v.trim().to_ascii_lowercase()) {
            Some(v) if v == "true" || v == "1" => true,
            Some(v) if v == "false" || v == "0" => false,
            _ => default,
        }
    }
}

struct Xml<'a> {
    s: &'a str,
    i: usize,
}

type XmlResult<T> = std::result::Result<T, String>;

impl<'a> Xml<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }

    fn line(&self) -> usize {
        self.s[..self.i].bytes().filter(|b| *b == b'\n').count() + 1
    }

    fn err<T>(&self, what: &str) -> XmlResult<T> {
        Err(format!("line {}: {what}", self.line()))
    }

    fn skip_ws(&mut self) {
        let t = self.rest().trim_start();
        self.i = self.s.len() - t.len();
    }

    /// Skips comments, `<?…?>` and `<!DOCTYPE …>` at the cursor.
    fn skip_misc(&mut self) -> XmlResult<()> {
        loop {
            self.skip_ws();
            let r = self.rest();
            let end = if r.starts_with("<!--") {
                "-->"
            } else if r.starts_with("<?") {
                "?>"
            } else if r.starts_with("<!") && !r.starts_with("<![CDATA[") {
                ">"
            } else {
                return Ok(());
            };
            match r.find(end) {
                Some(j) => self.i += j + end.len(),
                None => return self.err("unterminated comment or declaration"),
            }
        }
    }

    fn name(&mut self) -> XmlResult<String> {
        let r = self.rest();
        let n = r.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))).unwrap_or(r.len());
        if n == 0 {
            return self.err("expected a name");
        }
        self.i += n;
        Ok(r[..n].to_owned())
    }

    fn element(&mut self) -> XmlResult<Node> {
        if !self.rest().starts_with('<') {
            return self.err("expected an element");
        }
        self.i += 1;
        let mut node = Node { name: self.name()?, ..Default::default() };
        loop {
            self.skip_ws();
            let r = self.rest();
            if r.starts_with("/>") {
                self.i += 2;
                return Ok(node);
            }
            if r.starts_with('>') {
                self.i += 1;
                break;
            }
            let key = self.name()?;
            self.skip_ws();
            if !self.rest().starts_with('=') {
                return self.err(&format!("attribute {key} has no value"));
            }
            self.i += 1;
            self.skip_ws();
            let q = match self.rest().chars().next() {
                Some(c @ ('"' | '\'')) => c,
                _ => return self.err(&format!("attribute {key} is not quoted")),
            };
            self.i += 1;
            let Some(j) = self.rest().find(q) else { return self.err("unterminated attribute") };
            node.attrs.push((key, unescape(&self.rest()[..j])));
            self.i += j + 1;
        }
        loop {
            let r = self.rest();
            let Some(j) = r.find('<') else { return self.err(&format!("<{}> is not closed", node.name)) };
            node.text.push_str(&unescape(&r[..j]));
            self.i += j;
            let r = self.rest();
            if r.starts_with("</") {
                self.i += 2;
                let close = self.name()?;
                if close != node.name {
                    return self.err(&format!("</{close}> closes <{}>", node.name));
                }
                self.skip_ws();
                if !self.rest().starts_with('>') {
                    return self.err("malformed closing tag");
                }
                self.i += 1;
                return Ok(node);
            }
            if r.starts_with("<![CDATA[") {
                let Some(e) = r.find("]]>") else { return self.err("unterminated CDATA") };
                node.text.push_str(&r[9..e]);
                self.i += e + 3;
            } else if r.starts_with("<!--") || r.starts_with("<?") {
                self.skip_misc()?;
            } else {
                node.children.push(self.element()?);
            }
        }
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn parse_document(text: &str) -> XmlResult<Node> {
    let mut x = Xml { s: text.trim_start_matches('\u{feff}'), i: 0 };
    x.skip_misc()?;
    let root = x.element()?;
    x.skip_misc()?;
    if !x.rest().is_empty() {
        return x.err("content after the root element");
    }
    Ok(root)
}

fn parse_xml(name: &str, text: &str, tree: &mut TechTree, warnings: &mut Vec<Warning>) {
    let root = match parse_document(text) {
        Ok(r) => r,
        Err(e) => {
            warnings.push(Warning::new(name, format!("malformed xml: {e}")));
            return;
        }
    };
    if root.name != "techtree" {
        warnings.push(Warning::new(name, format!("root element is <{}>, not <techtree>", root.name)));
        return;
    }
    for child in &root.children {
        match child.name.as_str() {
            "object" => object(name, child, tree, warnings),
            "capacitylist" => capacity_list(name, child, tree, warnings),
            other => warnings.push(Warning::new(name, format!("unknown element <{other}>"))),
        }
    }
}

fn int_attr(name: &str, n: &Node, attr: &str, warnings: &mut Vec<Warning>) -> Option<i64> {
    let v = n.attr(attr)?;
    match v.trim().parse::<i64>() {
        Ok(v) => Some(v),
        Err(_) => {
            warnings.push(Warning::new(name, format!("<{}> {attr}=\"{v}\" is not a number", n.name)));
            None
        }
    }
}

fn capacity_list(name: &str, n: &Node, tree: &mut TechTree, warnings: &mut Vec<Warning>) {
    let Some(list_name) = n.attr("name").map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()) else {
        warnings.push(Warning::new(name, "<capacitylist> without a name"));
        return;
    };
    let mut list = CapacityList {
        name: list_name.clone(),
        max: int_attr(name, n, "maxCapValue", warnings).unwrap_or(0),
        tooltip: n.attr("tooltip").unwrap_or("").to_owned(),
        tooltip_reached: n.attr("tooltipReached").unwrap_or("").to_owned(),
        tooltip_over: n.attr("tooltipOver").unwrap_or("").to_owned(),
        members: Vec::new(),
    };
    for m in &n.children {
        if m.name != "obj" {
            warnings.push(Warning::new(name, format!("{list_name}: unknown element <{}>", m.name)));
            continue;
        }
        list.members.push((stem(&m.text), int_attr(name, m, "capValue", warnings).unwrap_or(1)));
    }
    if tree.caps.insert(list_name.clone(), list).is_some() {
        warnings.push(Warning::new(name, format!("capacity list {list_name} defined twice, the later wins")));
    }
}

fn object(name: &str, n: &Node, tree: &mut TechTree, warnings: &mut Vec<Warning>) {
    let Some(odf) = n.attr("ODF").map(stem).filter(|s| !s.is_empty()) else {
        warnings.push(Warning::new(name, "<object> without ODF"));
        return;
    };
    let mut requirements = Vec::new();
    if let Some(race) = n.attr("race") {
        requirements.push(Requirement {
            group: None,
            hide_unavailable: false,
            conditions: vec![Condition::Race(race.trim().to_ascii_lowercase())],
        });
    }
    for r in &n.children {
        if r.name != "requirement" {
            warnings.push(Warning::new(name, format!("{odf}: unknown element <{}>", r.name)));
            continue;
        }
        let mut req = Requirement {
            group: int_attr(name, r, "logicGroups", warnings).and_then(|g| u32::try_from(g).ok()),
            hide_unavailable: r.flag("buttonHideUnavailable", false),
            conditions: Vec::new(),
        };
        if let Some(race) = r.attr("race") {
            req.conditions.push(Condition::Race(race.trim().to_ascii_lowercase()));
        }
        for c in &r.children {
            match c.name.as_str() {
                "obj" => req.conditions.push(Condition::Have { odf: stem(&c.text), must_not: c.flag("mustNotHave", false) }),
                "ally" => match c.attr("race") {
                    Some(race) => req.conditions.push(Condition::Ally(race.trim().to_ascii_lowercase())),
                    None => warnings.push(Warning::new(name, format!("{odf}: <ally> without race"))),
                },
                "capList" => req.conditions.push(Condition::CapList(c.text.trim().to_ascii_lowercase())),
                "cap" => req.conditions.push(Condition::Unique {
                    max: int_attr(name, c, "maxCapValue", warnings).unwrap_or(1),
                    tooltip: c.attr("tooltip").unwrap_or("").to_owned(),
                    tooltip_reached: c.attr("tooltipReached").unwrap_or("").to_owned(),
                    tooltip_over: c.attr("tooltipOver").unwrap_or("").to_owned(),
                }),
                other => warnings.push(Warning::new(name, format!("{odf}: unknown requirement element <{other}>"))),
            }
        }
        requirements.push(req);
    }
    let entry = TechEntry {
        odf: odf.clone(),
        withheld: !n.flag("buildPossible", true),
        dependency_build_possible: n.flag("dependencyBuildPossible", true),
        requirements,
    };
    if tree.entries.insert(odf.clone(), entry).is_some() {
        warnings.push(Warning::new(name, format!("{odf}: listed twice, the later one wins")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tt(text: &str) -> TechTree {
        let (t, w) = TechTree::parse("t.tt", text);
        assert!(w.is_empty(), "{w:?}");
        t
    }

    #[test]
    fn tt_forms() {
        let t = tt("// c\n/* block\n x 1 y */\nfbase.odf 0\nFConst.odf 1 fbase.odf\nfyard 2 fbase.odf fconst.odf // two\nsecret.odf -1\ngTechFlag -1\n");
        assert_eq!(t.len(), 5);
        assert_eq!(t.prerequisites("fyard.odf"), ["fbase", "fconst"]);
        assert_eq!(t.prerequisites_deep("fyard"), ["fbase", "fconst"]);
        assert!(t.entry("SECRET").unwrap().withheld);
        assert!(t.entry("gtechflag").unwrap().withheld);
        assert_eq!(t.availability("fbase", &Owned::new("x")), Availability::Available);
        assert_eq!(t.availability("nope", &Owned::new("x")), Availability::NotListed);
        assert_eq!(t.availability("secret", &Owned::new("x")), Availability::Withheld);
    }

    #[test]
    fn minus_one_blocks_dependents_minus_two_does_not() {
        let t = tt("a -1\nb -2\nneeds_a 1 a\nneeds_b 1 b\n");
        let has_both = Owned::new("x").with("a", 1).with("b", 1);
        assert_eq!(
            t.availability("needs_a", &has_both),
            Availability::Locked { unmet: vec![Unmet::Unobtainable("a".into())], hidden: false }
        );
        assert_eq!(
            t.availability("needs_b", &Owned::new("x")),
            Availability::Locked { unmet: vec![Unmet::Missing("b".into())], hidden: false }
        );
        assert!(t.availability("needs_b", &has_both).is_available());
    }

    #[test]
    fn slashes_in_banner_comments() {
        let t = tt("//***[ BORG ]****\na 0 //* trailing\n/* x */ b 0\n/*\nc 0\n*/\n");
        assert_eq!(t.entries().map(|e| e.odf.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn tt_mismatches_are_warnings() {
        let (t, w) = TechTree::parse("t.tt", "a 2 b\nc 0 d\nwhat\ne x\na 0\n");
        assert_eq!(t.len(), 2);
        assert_eq!(w.len(), 5, "{w:?}");
    }

    #[test]
    fn prerequisites_gate_availability() {
        let t = tt("base 0\nconst 1 base\nyard 2 base const\n");
        let none = Owned::new("fed");
        assert_eq!(
            t.availability("yard", &none),
            Availability::Locked { unmet: vec![Unmet::Missing("base".into()), Unmet::Missing("const".into())], hidden: false }
        );
        let some = Owned::new("fed").with("base", 1);
        assert_eq!(
            t.availability("yard", &some),
            Availability::Locked { unmet: vec![Unmet::Missing("const".into())], hidden: false }
        );
        assert!(t.availability("const", &some).is_available());
        assert!(t.availability("yard", &some.with("CONST.odf", 1)).is_available());
    }

    #[test]
    fn includes_override_and_loops_warn() {
        let files: BTreeMap<&str, &str> = [
            ("m.tt", "#include tech1.tt\nship.odf -1\n"),
            ("tech1.tt", "ship.odf 0\nyard.odf 0\n#include m.tt\n#include gone.tt\n"),
        ]
        .into();
        let (t, w) = TechTree::load("m.tt", &mut |n| files.get(n.to_ascii_lowercase().as_str()).map(|s| s.to_string()));
        assert!(t.entry("ship").unwrap().withheld);
        assert!(!t.entry("yard").unwrap().withheld);
        assert_eq!(t.files(), ["m.tt", "tech1.tt"]);
        assert!(w.iter().any(|w| w.message.starts_with("include loop")), "{w:?}");
        assert!(w.iter().any(|w| w.message.starts_with("include not found")), "{w:?}");
        let (_, w) = TechTree::load("zzz.tt", &mut |_| None);
        assert_eq!(w[0].message, "not found");
    }

    const XML: &str = r#"<?xml version="1.0"?>
<!-- comment -->
<techtree>
  <capacitylist name="Special_Cap" maxCapValue="2" tooltip="Special ( %d / %d )" tooltipReached="Full" tooltipOver="Full">
    <obj capValue="1">spec_a.odf</obj>
    <obj capValue="2">spec_b.odf</obj>
  </capacitylist>
  <object ODF="Base.odf"/>
  <object ODF="spec_a.odf"><requirement><capList>special_cap</capList></requirement></object>
  <object ODF="warp.odf">
    <requirement logicGroups="1" buttonHideUnavailable="true"><ally race="borg"/></requirement>
    <requirement logicGroups="1"><obj>base.odf</obj></requirement>
    <requirement logicGroups="2"><obj>alt.odf</obj></requirement>
  </object>
  <object ODF="turret.odf"><requirement><obj mustNotHave="true">pod.odf</obj></requirement></object>
  <object ODF="uniq.odf"><requirement><cap maxCapValue="1" tooltip="Unique &amp; once"/></requirement></object>
  <object ODF="cube.odf" buildPossible="false" dependencyBuildPossible="false"><requirement race="borg"/></object>
  <object ODF="borgonly.odf" race="borg"/>
</techtree>"#;

    #[test]
    fn xml_requirements_groups_and_caps() {
        let (t, w) = TechTree::parse("t.xml", XML);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.len(), 7);
        assert_eq!(t.capacity_list("SPECIAL_CAP").unwrap().members.len(), 2);

        let borg = Owned::new("borg");
        // Groups are alternatives: group 1 needs ally and base, group 2 needs alt.
        assert!(!t.availability("warp", &borg).is_available());
        assert!(!t.availability("warp", &borg.clone().with("base", 1)).is_available());
        assert!(t.availability("warp", &borg.clone().with("base", 1).with_ally("borg")).is_available());
        assert!(t.availability("warp", &borg.clone().with("alt", 1)).is_available());
        match t.availability("warp", &borg) {
            Availability::Locked { unmet, hidden } => {
                assert!(hidden);
                assert!(unmet.contains(&Unmet::NoAlly("borg".into())) && unmet.contains(&Unmet::Missing("alt".into())));
            }
            a => panic!("{a:?}"),
        }

        // mustNotHave.
        assert!(t.availability("turret", &borg).is_available());
        assert_eq!(
            t.availability("turret", &borg.clone().with("pod", 1)),
            Availability::Locked { unmet: vec![Unmet::Forbidden("pod".into())], hidden: false }
        );
        // Unique research counts against itself.
        assert!(t.availability("uniq", &borg).is_available());
        assert_eq!(
            t.availability("uniq", &borg.clone().with("uniq", 1)),
            Availability::Locked { unmet: vec![Unmet::AlreadyResearched], hidden: false }
        );
        // Capacity list: spec_a counts 1, spec_b counts 2, limit 2.
        assert!(t.availability("spec_a", &borg.clone().with("spec_a", 1)).is_available());
        assert!(!t.availability("spec_a", &borg.clone().with("spec_a", 2)).is_available());
        assert!(!t.availability("spec_a", &borg.clone().with("spec_b", 1)).is_available());
        // buildPossible and race.
        assert_eq!(t.availability("cube", &borg), Availability::Withheld);
        assert!(!t.entry("cube").unwrap().dependency_build_possible);
        assert!(t.availability("borgonly", &borg).is_available());
        assert_eq!(
            t.availability("borgonly", &Owned::new("klingonempire")),
            Availability::Locked { unmet: vec![Unmet::WrongRace("borg".into())], hidden: false }
        );
        assert_eq!(t.prerequisites("warp"), ["base", "alt"]);
    }

    #[test]
    fn xml_errors_are_warnings() {
        let (_, w) = TechTree::parse("t.xml", "<techtree><object ODF=\"a.odf\"></techtree>");
        assert!(w[0].message.starts_with("malformed xml"), "{w:?}");
        let (t, w) = TechTree::parse(
            "t.xml",
            "<techtree><object ODF=\"a.odf\"><requirement><capList>missing</capList><wat/></requirement></object><bogus/></techtree>",
        );
        assert_eq!(t.len(), 1);
        assert_eq!(w.len(), 3, "{w:?}");
        // An undefined list imposes no limit.
        assert!(t.availability("a", &Owned::new("x")).is_available());
    }

    #[test]
    fn build_lists_against_the_fake_db() {
        let db = crate::fake::db();
        let t = tt("fx_station 0\nfx_yard 1 fx_station\nfx_miner 0\nfx_cruiser 1 fx_yard\nfx_frigate -1\n");
        assert!(t.check_against(&db).is_empty());
        let me = Owned::new("fx");

        let opts = t.build_options(&db, "fx_yard", &me);
        assert_eq!(opts.iter().map(|o| o.def.odf.as_str()).collect::<Vec<_>>(), ["fx_cruiser", "fx_frigate"]);
        // No yard yet: the cruiser is locked, the frigate withheld.
        assert_eq!(
            opts[0].availability,
            Availability::Locked { unmet: vec![Unmet::Missing("fx_yard".into())], hidden: false }
        );
        assert_eq!(opts[1].availability, Availability::Withheld);
        let with_yard = me.with("fx_yard", 1);
        assert!(t.build_options(&db, "FX_YARD.odf", &with_yard)[0].availability.is_available());
        // The station builds a miner with no prerequisite.
        assert!(t.build_options(&db, "fx_station", &Owned::new("fx"))[0].availability.is_available());
        assert!(t.build_options(&db, "nobody", &Owned::new("fx")).is_empty());

        let bad = tt("ghost 1 fx_yard\n");
        assert_eq!(bad.check_against(&db).len(), 1);
    }
}
