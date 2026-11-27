//! Formation data (CON-59, ADR 0011 G3): the formations the Armada II family
//! lays its fleets out in.
//!
//! Two sources, both content (ADR 0004 D1, D9):
//!
//! - **Files.** `AI/formations/*.frm` of the install or mod, read through the
//!   VFS at run time and never committed (ADR 0003). A file is a small C-like
//!   source (the same dialect as the AI player files): `formationName`,
//!   `m_formationID`, `m_shipRadius` and the `formationSpec` rows, each row
//!   `rank, x, y, z` and seven `shipType` letters, the priority list of the
//!   slot. The row layout is the one the header the files include declares.
//!   Evidence: read from data (names, row layout); the 200-slot limit and the
//!   unit of the numbers (multiples of `FORMATION_SPACING`) read from the
//!   executable, static.
//! - **Code.** Three formations those games build from `FORMATION_SPACING`
//!   and a member count ([`BuiltIn`]). Evidence: read from the executable,
//!   static; not measured.
//!
//! Numbers stay as the file writes them, in multiples of `FORMATION_SPACING`
//! (offsets and `m_shipRadius` alike); [`Formation::offset`] and
//! [`Formation::ship_radius`] give world units for a given spacing
//! ([`GroupRules::formation_spacing`](crate::GroupRules)). The simulation's
//! slot assignment, scale and facing are SIM-76's.
//!
//! A file STA64 cannot read in full is reported as a [`Warning`] with its
//! file and line and what is unusable is skipped. INTERIM: the original
//! engine refuses the whole file on a bad value; STA64 keeps the other rows
//! (the package row's rule).

use sta_core::{Fixed, Vec3};

use crate::digest::{self, Canon, DigestEntry, fixed_text, quoted};
use crate::Warning;

/// The most slots a formation has (read from the executable, static); later
/// rows of a file are dropped with a warning.
pub const MAX_SLOTS: usize = 200;

/// How many `shipType` letters a slot lists.
pub const PRIORITIES: usize = 7;

/// Where the files live.
pub const DIR: &str = "AI/formations";

/// One slot of a formation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// `m_leaderRank`: the order slots are filled in for leadership; the
    /// lowest rank leads (G2).
    pub rank: i32,
    /// The offset from the formation's centre, multiples of the spacing.
    pub offset: Vec3,
    /// The `shipType` letters this slot takes, first choice first.
    pub ship_types: [char; PRIORITIES],
}

/// Which formation STA64 built in code rather than read from a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuiltIn {
    /// Slots on the four diagonals of a plane, ring by ring.
    X,
    /// Slots on the four axes of a plane, ring by ring.
    Cross,
    /// A square spiral in the horizontal plane.
    Sheet,
}

impl BuiltIn {
    /// STA64's own label.
    pub fn label(self) -> &'static str {
        match self {
            BuiltIn::X => "x",
            BuiltIn::Cross => "cross",
            BuiltIn::Sheet => "sheet",
        }
    }
}

/// Where a formation came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The virtual path of the `.frm` file.
    File(String),
    BuiltIn(BuiltIn),
}

/// A formation: a name, a ship radius to scale by and its slots in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formation {
    pub name: String,
    /// `m_formationID`; `None` for a built-in formation.
    pub id: Option<i32>,
    /// `m_shipRadius`, multiples of the spacing: the member size the offsets
    /// are written for.
    pub radius: Fixed,
    /// In file order (G3: slots are taken in this order).
    pub slots: Vec<Slot>,
    pub origin: Origin,
}

impl Formation {
    /// Slot `i`'s offset in world units for `spacing`.
    pub fn offset(&self, i: usize, spacing: Fixed) -> Option<Vec3> {
        let o = self.slots.get(i)?.offset;
        Some(Vec3 { x: o.x * spacing, y: o.y * spacing, z: o.z * spacing })
    }

    /// `m_shipRadius` in world units for `spacing`.
    pub fn ship_radius(&self, spacing: Fixed) -> Fixed {
        self.radius * spacing
    }

    /// The slot that leads: the lowest rank, the first in file order on a tie.
    pub fn leader_slot(&self) -> Option<usize> {
        self.slots.iter().enumerate().min_by_key(|(i, s)| (s.rank, *i)).map(|(i, _)| i)
    }

    /// A formation STA64 builds in code, `slots` slots (at least one). Slot 0
    /// is the centre; the others go outward in rings of four (X and cross) or
    /// in a square spiral (sheet). Read from the executable, static; not
    /// measured.
    pub fn built_in(kind: BuiltIn, slots: usize) -> Formation {
        let slots = slots.max(1);
        let diag = Fixed::from_f64(f64::from(std::f32::consts::FRAC_1_SQRT_2));
        let (one, zero) = (Fixed::ONE, Fixed::ZERO);
        let at = |x: Fixed, y: Fixed, z: Fixed| Vec3 { x, y, z };
        let types: [char; PRIORITIES] = match kind {
            BuiltIn::Cross => ['B', '?', '-', '-', '-', '-', '-'],
            BuiltIn::X | BuiltIn::Sheet => ['B', 'C', 'D', 'F', 'S', 'X', '?'],
        };
        let mut out = Vec::with_capacity(slots);
        out.push(Slot { rank: 0, offset: at(zero, zero, zero), ship_types: types });
        match kind {
            BuiltIn::X | BuiltIn::Cross => {
                let dirs = match kind {
                    BuiltIn::X => [at(diag, diag, zero), at(diag, -diag, zero), at(-diag, diag, zero), at(-diag, -diag, zero)],
                    _ => [at(one, zero, zero), at(-one, zero, zero), at(zero, one, zero), at(zero, -one, zero)],
                };
                for i in 1..slots {
                    let ring = Fixed::from_int(i.div_ceil(4) as i32);
                    let d = dirs[i % 4];
                    out.push(Slot { rank: i as i32, offset: at(d.x * ring, d.y * ring, d.z * ring), ship_types: types });
                }
            }
            BuiltIn::Sheet => {
                // Heading east, then north, then west, then south, each leg one
                // step longer than the leg two before it: a square spiral.
                let steps = [at(one, zero, zero), at(zero, zero, one), at(-one, zero, zero), at(zero, zero, -one)];
                let (mut x, mut z) = (0i32, 0i32);
                let (mut dir, mut bound, mut lower) = (0usize, 1i32, -1i32);
                let mut pos = at(zero, zero, zero);
                for i in 1..slots {
                    let s = steps[dir];
                    pos = at(pos.x + s.x, pos.y + s.y, pos.z + s.z);
                    match dir {
                        0 => {
                            x += 1;
                            if x >= bound {
                                dir = 1;
                            }
                        }
                        1 => {
                            z += 1;
                            if z >= bound {
                                dir = 2;
                            }
                        }
                        2 => {
                            x -= 1;
                            if x <= lower {
                                dir = 3;
                            }
                        }
                        _ => {
                            z -= 1;
                            if z <= lower {
                                dir = 0;
                                bound += 1;
                                lower -= 1;
                            }
                        }
                    }
                    out.push(Slot { rank: i as i32, offset: pos, ship_types: types });
                }
            }
        }
        Formation {
            name: kind.label().to_owned(),
            id: None,
            // 0.45 of the spacing (read from the executable, static).
            radius: Fixed::from_f64(0.45),
            slots: out,
            origin: Origin::BuiltIn(kind),
        }
    }

    /// Parse one formation file's `text`; `file` names it in warnings. `None`
    /// (with a warning saying why) when the file has no name, no id, no usable
    /// ship radius or no usable slot.
    pub fn parse(file: &str, text: &str) -> (Option<Formation>, Vec<Warning>) {
        let mut warnings = Vec::new();
        let toks = lex(text);
        let mut name = None;
        let mut id = None;
        let mut radius = None;
        let mut rows: Option<Vec<Slot>> = None;
        let mut warn = |line: usize, msg: String| warnings.push(Warning::new(file, format!("line {line}: {msg}")));
        let mut i = 0;
        while i < toks.len() {
            let (Tok::Ident(word), line) = (&toks[i].0, toks[i].1) else {
                i += 1;
                continue;
            };
            let key = word.to_ascii_lowercase();
            if !matches!(key.as_str(), "formationname" | "m_formationid" | "m_shipradius" | "formationspec") {
                i += 1;
                continue;
            }
            // `[n]` and the like, then `=`.
            let mut j = i + 1;
            while j < toks.len() && !matches!(toks[j].0, Tok::P('=') | Tok::P(';')) {
                j += 1;
            }
            if !matches!(toks.get(j), Some((Tok::P('='), _))) {
                warn(line, format!("{word} has no value"));
                i = j.max(i + 1);
                continue;
            }
            j += 1;
            match key.as_str() {
                "formationname" => match toks.get(j) {
                    Some((Tok::Str(s), _)) if !s.is_empty() => name = Some(s.clone()),
                    _ => warn(line, "formationName is not a text".to_owned()),
                },
                "m_formationid" => match toks.get(j) {
                    Some((Tok::Num(n), _)) => match n.parse::<i32>() {
                        Ok(v) => id = Some(v),
                        Err(_) => warn(line, "m_formationID is not a whole number".to_owned()),
                    },
                    _ => warn(line, "m_formationID is not a whole number".to_owned()),
                },
                "m_shipradius" => match toks.get(j).and_then(|(t, _)| number(t)).filter(|v| *v > 0.0 && *v <= 1.0e6) {
                    Some(v) => radius = Some(Fixed::from_f64(f64::from(v))),
                    None => warn(line, "m_shipRadius is not a positive number".to_owned()),
                },
                _ => {
                    if !matches!(toks.get(j), Some((Tok::P('{'), _))) {
                        warn(line, "formationSpec has no list".to_owned());
                    } else {
                        let (slots, end) = read_rows(&toks, j + 1, &mut warn);
                        rows = Some(slots);
                        i = end;
                        continue;
                    }
                }
            }
            i = j;
        }
        let rows = rows.filter(|r| !r.is_empty());
        for (absent, what) in [(name.is_none(), "formationName"), (id.is_none(), "m_formationID"), (radius.is_none(), "m_shipRadius"), (rows.is_none(), "formationSpec row")] {
            if absent {
                warnings.push(Warning::new(file, format!("no usable {what}; file skipped")));
            }
        }
        let (Some(name), Some(id), Some(radius), Some(slots)) = (name, id, radius, rows) else {
            return (None, warnings);
        };
        (Some(Formation { name, id: Some(id), radius, slots, origin: Origin::File(file.to_owned()) }), warnings)
    }

    fn digest_text(&self) -> String {
        let slots = self.slots.iter().map(|s| {
            let types: String = s.ship_types.iter().collect();
            format!("{}:{}:{}:{}:{}", s.rank, fixed_text(s.offset.x), fixed_text(s.offset.y), fixed_text(s.offset.z), quoted(&types))
        });
        Canon::new().text("name", &self.name).fixed("radius", self.radius).list("slots", slots).finish()
    }
}

/// The formations of the mounted content, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Formations {
    list: Vec<Formation>,
}

impl Formations {
    /// Read every `AI/formations/*.frm` the mount shows. Sorted by id; a file
    /// that repeats an id of an earlier one (by id, then path) is dropped with
    /// a warning. Never called for Armada I, which has none.
    pub fn load(vfs: &sta_vfs::Vfs) -> (Formations, Vec<Warning>) {
        let mut warnings = Vec::new();
        let mut found = Vec::new();
        for path in vfs.list(DIR) {
            if !path.to_ascii_lowercase().ends_with(".frm") {
                continue;
            }
            let text = match vfs.read(&path) {
                Some(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
                Some(Err(e)) => {
                    warnings.push(Warning::new(&path, format!("unreadable: {e}")));
                    continue;
                }
                None => continue,
            };
            let (formation, w) = Formation::parse(&path, &text);
            warnings.extend(w);
            found.extend(formation);
        }
        let (formations, w) = Formations::from_parsed(found);
        warnings.extend(w);
        (formations, warnings)
    }

    /// Order parsed formations and drop repeated ids.
    pub fn from_parsed(mut found: Vec<Formation>) -> (Formations, Vec<Warning>) {
        found.sort_by(|a, b| (a.id, file_of(a)).cmp(&(b.id, file_of(b))));
        let mut warnings = Vec::new();
        let mut list: Vec<Formation> = Vec::new();
        for f in found {
            if let Some(first) = list.last().filter(|p| p.id == f.id) {
                warnings.push(Warning::new(file_of(&f), format!("m_formationID {} is already used by {}; file skipped", f.id.unwrap_or_default(), file_of(first))));
            } else {
                list.push(f);
            }
        }
        (Formations { list }, warnings)
    }

    /// Parse each `(file, text)` as the loader does (tests, content built in code).
    pub fn from_texts<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> (Formations, Vec<Warning>) {
        let mut warnings = Vec::new();
        let mut found = Vec::new();
        for (file, text) in files {
            let (f, w) = Formation::parse(file, text);
            warnings.extend(w);
            found.extend(f);
        }
        let (formations, w) = Formations::from_parsed(found);
        warnings.extend(w);
        (formations, warnings)
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// In id order.
    pub fn iter(&self) -> impl Iterator<Item = &Formation> {
        self.list.iter()
    }

    pub fn by_id(&self, id: i32) -> Option<&Formation> {
        self.list.iter().find(|f| f.id == Some(id))
    }

    /// By name, ignoring case.
    pub fn by_name(&self, name: &str) -> Option<&Formation> {
        self.list.iter().find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// The formations as digest entries (D10): one per formation, by id, with
    /// its name, radius and every slot (rank, offset, priority list). The file
    /// name is not part of the rule.
    pub fn digest_entries(&self) -> Vec<DigestEntry> {
        self.list.iter().filter_map(|f| Some(DigestEntry::new(digest::key("formation:", &f.id?.to_string()), f.digest_text()))).collect()
    }
}

fn file_of(f: &Formation) -> &str {
    match &f.origin {
        Origin::File(p) => p,
        Origin::BuiltIn(b) => b.label(),
    }
}

// ------------------------------------------------------------------ parsing

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    /// Digits, a sign, a point, an exponent or a `f` suffix, as written.
    Num(String),
    Str(String),
    Chr(char),
    P(char),
}

/// Tokens with their 1-based line. `//` and `/* */` comments and `#` lines
/// (the include and defines of the header) are skipped.
fn lex(text: &str) -> Vec<(Tok, usize)> {
    let b: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0, 1usize);
    let mut line_start = true;
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            line_start = true;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if (c == '#' && line_start) || (c == '/' && b.get(i + 1) == Some(&'/')) {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i < b.len() && !(b[i] == '*' && b.get(i + 1) == Some(&'/')) {
                if b[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
        } else {
            line_start = false;
            if c == '"' {
                let mut s = String::new();
                i += 1;
                while i < b.len() && b[i] != '"' && b[i] != '\n' {
                    s.push(b[i]);
                    i += 1;
                }
                i += 1;
                out.push((Tok::Str(s), line));
            } else if c == '\'' {
                // `'S'`; anything else is a malformed literal and stays punctuation.
                if b.get(i + 2) == Some(&'\'') && b[i + 1] != '\n' {
                    out.push((Tok::Chr(b[i + 1]), line));
                    i += 3;
                } else {
                    out.push((Tok::P('\''), line));
                    i += 1;
                }
            } else if c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && b.get(i + 1).is_some_and(|d| d.is_ascii_digit() || *d == '.')) {
                let start = i;
                let mut prev = c;
                i += 1;
                while i < b.len() {
                    let d = b[i];
                    let sign_in_exp = (d == '-' || d == '+') && (prev == 'e' || prev == 'E');
                    if d.is_ascii_alphanumeric() || d == '.' || sign_in_exp {
                        prev = d;
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push((Tok::Num(b[start..i].iter().collect()), line));
            } else if c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                    i += 1;
                }
                out.push((Tok::Ident(b[start..i].iter().collect()), line));
            } else {
                out.push((Tok::P(c), line));
                i += 1;
            }
        }
    }
    out
}

/// A number as the engine's `float` reads it (f32), finite.
fn number(t: &Tok) -> Option<f32> {
    let Tok::Num(s) = t else { return None };
    let s = s.trim_end_matches(['f', 'F']);
    s.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// The slot rows from `toks[start]` to the closing brace. Rows are read one
/// line at a time, as the installed files write them: a line holds one row
/// (or several), eleven values each. Returns the slots and the index after
/// the list.
fn read_rows(toks: &[(Tok, usize)], start: usize, warn: &mut impl FnMut(usize, String)) -> (Vec<Slot>, usize) {
    let mut end = start;
    let mut depth = 1;
    while end < toks.len() {
        match toks[end].0 {
            Tok::P('{') => depth += 1,
            Tok::P('}') => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        end += 1;
    }
    let mut slots = Vec::new();
    let mut dropped = false;
    let mut i = start;
    while i < end {
        // One line's values.
        let line = toks[i].1;
        let mut vals: Vec<&Tok> = Vec::new();
        while i < end && toks[i].1 == line {
            if !matches!(toks[i].0, Tok::P(',') | Tok::P('{') | Tok::P('}') | Tok::P(';')) {
                vals.push(&toks[i].0);
            }
            i += 1;
        }
        if vals.is_empty() {
            continue;
        }
        if !vals.len().is_multiple_of(4 + PRIORITIES) {
            warn(line, format!("slot row has {} values, expected {}; skipped", vals.len(), 4 + PRIORITIES));
            continue;
        }
        for row in vals.chunks(4 + PRIORITIES) {
            match slot_of(row) {
                Ok(slot) if slots.len() < MAX_SLOTS => slots.push(slot),
                Ok(_) => {
                    if !dropped {
                        warn(line, format!("more than {MAX_SLOTS} slots; the rest are ignored"));
                        dropped = true;
                    }
                }
                Err(why) => warn(line, format!("slot row {why}; skipped")),
            }
        }
    }
    (slots, end + 1)
}

fn slot_of(row: &[&Tok]) -> Result<Slot, &'static str> {
    let rank = match row[0] {
        Tok::Num(n) => n.parse::<i32>().map_err(|_| "has a rank that is not a whole number")?,
        _ => return Err("has a rank that is not a whole number"),
    };
    let mut xyz = [Fixed::ZERO; 3];
    for (k, t) in row[1..4].iter().enumerate() {
        let v = number(t).filter(|v| v.abs() <= 1.0e6).ok_or("has an offset that is not a number")?;
        xyz[k] = Fixed::from_f64(f64::from(v));
    }
    let mut ship_types = ['?'; PRIORITIES];
    for (k, t) in row[4..].iter().enumerate() {
        match t {
            Tok::Chr(c) => ship_types[k] = *c,
            _ => return Err("has a priority that is not a letter in quotes"),
        }
    }
    Ok(Slot { rank, offset: Vec3 { x: xyz[0], y: xyz[1], z: xyz[2] }, ship_types })
}

#[cfg(test)]
mod tests;
