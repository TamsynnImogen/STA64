//! AI player files (`.aip`, CON-14): typed parameters for the skirmish AI.
//!
//! An AIP is a small C-like file (see `docs/data/aip.md`):
//!
//! - `#include "base.aip"` pulls in a base AIP; a file's own declarations
//!   override anything it includes. `aipdef.h` is included the same way and
//!   brings `#define`s (`MAX_ELEMENTS`, `UNLIMITED`, `YES`) and the build-list
//!   element `struct`s.
//! - `int NAME = value;` (also `float`, `double`, `char NAME[n] = "…";`) sets a
//!   parameter.
//! - `Way_Better_Build_List_Element the_build_list[MAX_ELEMENTS] = { "bbase", 1,0, -1, 1, … };`
//!   is a build list: a flat initialiser, cut into rows by the element
//!   struct's field count (or, when the struct is unknown, at each string).
//! - `//`, `/* */` and lines starting with `#` that are not directives are
//!   comments.
//!
//! Installed files are hand-written and not always valid C (a missing `;`, a
//! stray `};`, `;` between list rows); those are read the way the game reads
//! them and reported as [`Warning`]s rather than failing.
//!
//! Numbers become [`Fixed`] once, here, as everywhere in the content seam.

use std::collections::{BTreeMap, BTreeSet};

use sta_core::Fixed;

use crate::Warning;

/// One parsed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AipValue {
    Int(i64),
    /// A number with a fraction or exponent.
    Num(Fixed),
    Str(String),
    /// A name no `#define` resolved.
    Name(String),
}

impl AipValue {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            AipValue::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_fixed(&self) -> Option<Fixed> {
        match self {
            AipValue::Int(v) => Some(Fixed::from_f64(*v as f64)),
            AipValue::Num(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            AipValue::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// A scalar parameter (`int MAX_EXEC_ATTACK = 1;`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AipVar {
    /// As written.
    pub name: String,
    /// Declared type (`int`, `float`, `char[]`, ...).
    pub ty: String,
    pub value: AipValue,
}

/// An array declaration with an initialiser, usually a build list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AipList {
    pub name: String,
    /// Element type (`Way_Better_Build_List_Element`, ...).
    pub element: String,
    /// The element struct's field names when it is known, else empty.
    pub fields: Vec<String>,
    pub rows: Vec<Vec<AipValue>>,
}

/// One row of a build list, by the field names of `aipdef.h`'s structs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildItem {
    /// `unit_type_name`: the ODF name, without `.odf`.
    pub unit: String,
    /// `units_to_build`.
    pub count: Fixed,
    /// `m_base_num`: which base builds it (stations).
    pub base: Option<i64>,
    /// `m_buildMAX`: how many may be built from this line (`-1` = no limit).
    pub max: Option<i64>,
    /// `m_techLevel`: positive defines that tech level; negative is the
    /// highest tech level to build it at.
    pub tech_level: Option<i64>,
}

/// A resolved AIP: its includes applied, its own declarations last.
#[derive(Debug, Clone, Default)]
pub struct Aip {
    pub name: String,
    vars: BTreeMap<String, AipVar>,
    lists: BTreeMap<String, AipList>,
    structs: BTreeMap<String, Vec<String>>,
    defines: BTreeMap<String, String>,
}

impl Aip {
    /// Parse `name` and everything it includes. `load` maps an include name
    /// (as written, e.g. `"aipdef.h"`) to its text. Missing includes, include
    /// loops and malformed lines are warnings.
    pub fn load(name: &str, load: &mut dyn FnMut(&str) -> Option<String>) -> (Aip, Vec<Warning>) {
        let mut aip = Aip { name: name.to_owned(), ..Default::default() };
        let mut warnings = Vec::new();
        let mut stack = Vec::new();
        match load(name) {
            Some(text) => apply(name, &text, load, &mut aip, &mut stack, &mut warnings),
            None => warnings.push(Warning::new(name, "not found")),
        }
        (aip, warnings)
    }

    /// Parse one file's text with no includes available.
    pub fn parse(name: &str, text: &str) -> (Aip, Vec<Warning>) {
        let text = text.to_owned();
        let mut first = Some(text);
        Self::load(name, &mut |n| if n == name { first.take() } else { None })
    }

    pub fn var(&self, name: &str) -> Option<&AipVar> {
        self.vars.get(&name.to_ascii_lowercase())
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.var(name)?.value.as_int()
    }

    pub fn fixed(&self, name: &str) -> Option<Fixed> {
        self.var(name)?.value.as_fixed()
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        self.var(name)?.value.as_str()
    }

    /// Scalar parameters in name order.
    pub fn vars(&self) -> impl Iterator<Item = &AipVar> {
        self.vars.values()
    }

    pub fn list(&self, name: &str) -> Option<&AipList> {
        self.lists.get(&name.to_ascii_lowercase())
    }

    /// Lists in name order.
    pub fn lists(&self) -> impl Iterator<Item = &AipList> {
        self.lists.values()
    }

    /// `#define` value text.
    pub fn define(&self, name: &str) -> Option<&str> {
        self.defines.get(name).map(String::as_str)
    }

    /// A list read as build items. Rows without a leading unit name are
    /// skipped.
    pub fn build_list(&self, name: &str) -> Option<Vec<BuildItem>> {
        let list = self.list(name)?;
        let col = |field: &str, pos: usize| -> usize {
            list.fields.iter().position(|f| f.eq_ignore_ascii_case(field)).unwrap_or(pos)
        };
        let (unit, count, base, max, tech) =
            (col("unit_type_name", 0), col("units_to_build", 1), col("m_base_num", 2), col("m_buildMAX", 3), col("m_techLevel", 4));
        let width = if list.fields.is_empty() { usize::MAX } else { list.fields.len() };
        let int = |row: &[AipValue], i: usize| if i < width { row.get(i).and_then(AipValue::as_int) } else { None };
        Some(
            list.rows
                .iter()
                .filter_map(|row| {
                    Some(BuildItem {
                        unit: row.get(unit)?.as_str()?.to_owned(),
                        count: row.get(count).and_then(AipValue::as_fixed).unwrap_or(Fixed::ONE),
                        base: int(row, base),
                        max: int(row, max),
                        tech_level: int(row, tech),
                    })
                })
                .collect(),
        )
    }
}

// ------------------------------------------------------------------ parsing

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Num(String),
    Str(String),
    P(char),
}

const TYPES: [&str; 8] = ["int", "float", "double", "char", "bool", "long", "short", "string"];
const QUALIFIERS: [&str; 5] = ["const", "static", "unsigned", "signed", "extern"];

fn apply(
    name: &str,
    text: &str,
    load: &mut dyn FnMut(&str) -> Option<String>,
    aip: &mut Aip,
    stack: &mut Vec<String>,
    warnings: &mut Vec<Warning>,
) {
    let key = name.to_ascii_lowercase();
    if stack.contains(&key) {
        warnings.push(Warning::new(name, format!("include loop: {} -> {name}", stack.join(" -> "))));
        return;
    }
    stack.push(key);
    let text = strip_comments(text);
    let mut body = String::new();
    // Directives are line-based and act in order, so run each one when met,
    // after the declarations before it.
    for line in text.lines() {
        let l = strip_line_comment(line).trim();
        let Some(rest) = l.strip_prefix('#') else {
            body.push_str(line);
            body.push('\n');
            continue;
        };
        let rest = rest.trim_start();
        let (word, arg) = rest.split_at(rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len()));
        match word.to_ascii_lowercase().as_str() {
            "include" => {
                declarations(name, &std::mem::take(&mut body), aip, warnings);
                let inc = arg.trim().trim_matches(['"', '<', '>']).trim();
                match load(inc) {
                    Some(t) => apply(inc, &t, load, aip, stack, warnings),
                    None => warnings.push(Warning::new(name, format!("include not found: {inc}"))),
                }
            }
            "define" => {
                let arg = arg.trim();
                let (k, v) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
                if !k.is_empty() {
                    aip.defines.insert(k.to_owned(), v.trim().to_owned());
                }
            }
            "undef" => {
                aip.defines.remove(arg.trim());
            }
            // `#` comment lines and unsupported directives (`#if`, ...).
            "if" | "ifdef" | "ifndef" | "elif" | "else" | "endif" | "pragma" => {
                warnings.push(Warning::new(name, format!("directive ignored: #{word}")));
            }
            _ => {}
        }
    }
    declarations(name, &body, aip, warnings);
    stack.pop();
}

/// Remove `//` and `/* */` comments in one pass, as C does: inside a `//`
/// comment a `/*` means nothing (`//****line 10****`), and neither starts
/// inside a string. Newlines are kept so line structure survives.
fn strip_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            in_str = !(c == b'"' || c == b'\n');
            out.push(c);
            i += 1;
        } else if c == b'"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                if b[i] == b'\n' {
                    out.push(b'\n');
                }
                i += 1;
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn strip_line_comment(line: &str) -> &str {
    let mut in_str = false;
    let b = line.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b'"' => in_str = !in_str,
            b'/' if !in_str && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
    }
    line
}

fn tokenize(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = strip_line_comment(line);
        let mut it = line.char_indices().peekable();
        while let Some(&(i, c)) = it.peek() {
            if c.is_whitespace() {
                it.next();
            } else if c == '"' {
                it.next();
                let start = i + 1;
                let mut end = line.len();
                for (j, d) in it.by_ref() {
                    if d == '"' {
                        end = j;
                        break;
                    }
                }
                out.push(Tok::Str(line[start..end].to_owned()));
            } else if c.is_ascii_digit() || c == '.' {
                let start = i;
                let mut end = line.len();
                let mut prev = c;
                while let Some(&(j, d)) = it.peek() {
                    // `1e-6`: a sign right after an exponent belongs to the number.
                    let sign_in_exp = (d == '-' || d == '+') && (prev == 'e' || prev == 'E');
                    if d.is_ascii_alphanumeric() || d == '.' || sign_in_exp {
                        prev = d;
                        it.next();
                    } else {
                        end = j;
                        break;
                    }
                }
                out.push(Tok::Num(line[start..end].to_owned()));
            } else if c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                let mut end = line.len();
                while let Some(&(j, d)) = it.peek() {
                    if d.is_ascii_alphanumeric() || d == '_' {
                        it.next();
                    } else {
                        end = j;
                        break;
                    }
                }
                out.push(Tok::Ident(line[start..end].to_owned()));
            } else {
                out.push(Tok::P(c));
                it.next();
            }
        }
    }
    out
}

fn number(text: &str, negative: bool) -> Option<AipValue> {
    let t = text.trim_end_matches(['f', 'F', 'l', 'L']);
    if let Ok(v) = t.parse::<i64>() {
        return Some(AipValue::Int(if negative { -v } else { v }));
    }
    let v: f64 = t.parse().ok()?;
    Some(AipValue::Num(Fixed::from_f64(if negative { -v } else { v })))
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    file: &'a str,
    aip: &'a mut Aip,
    warnings: &'a mut Vec<Warning>,
}

fn declarations(file: &str, text: &str, aip: &mut Aip, warnings: &mut Vec<Warning>) {
    let toks = tokenize(text);
    if toks.is_empty() {
        return;
    }
    Parser { toks, pos: 0, file, aip, warnings }.run();
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::P(c)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(Warning::new(self.file, msg));
    }

    fn run(&mut self) {
        while let Some(t) = self.peek().cloned() {
            match t {
                Tok::Ident(w) if w == "struct" => {
                    self.pos += 1;
                    self.structure();
                }
                Tok::Ident(_) => self.declaration(),
                Tok::P(';') => self.pos += 1,
                Tok::P('}') => {
                    // A stray `};` after a list already closed.
                    self.pos += 1;
                    self.eat(';');
                    self.warn("stray '}'");
                }
                Tok::P('{') => {
                    // An initialiser with no declaration in front of it.
                    self.pos += 1;
                    let n = self.list_values("(no declaration)").len();
                    self.eat(';');
                    self.warn(format!("list of {n} values without a declaration ignored"));
                }
                _ => {
                    // Values outside any list (rows left after a list's `};`,
                    // Battlezone-style bare rows): the game ignores them.
                    let start = self.pos;
                    while matches!(self.peek(), Some(t) if !matches!(t, Tok::Ident(_) | Tok::P('{') | Tok::P('}'))) {
                        self.pos += 1;
                    }
                    let first = &self.toks[start];
                    let first = match first {
                        Tok::Str(s) => format!("\"{s}\""),
                        Tok::Num(n) => n.clone(),
                        Tok::Ident(w) => w.clone(),
                        Tok::P(c) => c.to_string(),
                    };
                    self.warn(format!("{} stray tokens outside a declaration ignored (from {first})", self.pos - start));
                }
            }
        }
    }

    /// `struct Name { type field[n]; ... };`
    fn structure(&mut self) {
        let Some(Tok::Ident(name)) = self.next() else { return self.warn("struct without a name") };
        if !self.eat('{') {
            return self.warn(format!("struct {name} without a body"));
        }
        let mut fields = Vec::new();
        let mut last_ident = None;
        while let Some(t) = self.next() {
            match t {
                Tok::P('}') => break,
                Tok::P(';') => fields.extend(last_ident.take()),
                Tok::P('[') => {
                    fields.extend(last_ident.take());
                    while !matches!(self.next(), Some(Tok::P(']')) | None) {}
                }
                Tok::Ident(w) => last_ident = Some(w),
                _ => {}
            }
        }
        self.eat(';');
        self.aip.structs.insert(name.to_ascii_lowercase(), fields);
    }

    /// `[qualifiers] type name [ '[' n ']' ] [= init] ;`
    fn declaration(&mut self) {
        let mut ty = String::new();
        while let Some(Tok::Ident(w)) = self.peek() {
            if QUALIFIERS.contains(&w.as_str()) {
                self.pos += 1;
            } else {
                break;
            }
        }
        if let Some(Tok::Ident(w)) = self.next() {
            ty = w;
        }
        let Some(Tok::Ident(name)) = self.next() else {
            return self.recover(format!("declaration of type {ty} without a name"));
        };
        let mut array = false;
        if self.eat('[') {
            array = true;
            while !matches!(self.next(), Some(Tok::P(']')) | None) {}
        }
        if !self.eat('=') {
            // Declared but not set (`TYPE name[n];`): nothing to record.
            if !self.eat(';') {
                self.recover(format!("{name}: expected '=' or ';'"));
            }
            return;
        }
        if self.peek() == Some(&Tok::P('{')) {
            self.pos += 1;
            let values = self.list_values(&name);
            self.list(&ty, &name, values);
        } else {
            let value = self.scalar(&name);
            let ty = if array { format!("{ty}[]") } else { ty };
            if let Some(value) = value {
                self.aip.vars.insert(name.to_ascii_lowercase(), AipVar { name, ty, value });
            }
        }
        if !self.eat(';') {
            match self.peek() {
                None | Some(Tok::Ident(_)) | Some(Tok::P('}')) => self.warn(format!("{}: missing ';'", self.last_name())),
                Some(_) => self.recover("expected ';'".into()),
            }
        }
    }

    fn last_name(&self) -> String {
        self.aip.vars.values().last().map(|v| v.name.clone()).unwrap_or_default()
    }

    /// Skip to the next `;` after a parse error.
    fn recover(&mut self, msg: String) {
        self.warn(msg);
        while let Some(t) = self.next() {
            if t == Tok::P(';') {
                break;
            }
        }
    }

    fn value(&mut self) -> Option<AipValue> {
        let negative = self.eat('-');
        if !negative {
            self.eat('+');
        }
        match self.next()? {
            Tok::Num(n) => number(&n, negative),
            Tok::Str(s) => Some(AipValue::Str(s)),
            Tok::Ident(w) => Some(self.resolve(&w, negative, 0)),
            Tok::P(c) => {
                self.pos -= 1;
                let _ = c;
                None
            }
        }
    }

    fn resolve(&self, name: &str, negative: bool, depth: u8) -> AipValue {
        let Some(v) = self.aip.defines.get(name) else { return AipValue::Name(name.to_owned()) };
        let v = v.trim();
        if let Some(n) = number(v.trim_start_matches('-'), negative != v.starts_with('-')) {
            return n;
        }
        if let Some(s) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            return AipValue::Str(s.to_owned());
        }
        if depth < 8 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !v.is_empty() {
            return self.resolve(v, negative, depth + 1);
        }
        AipValue::Name(name.to_owned())
    }

    fn scalar(&mut self, name: &str) -> Option<AipValue> {
        let v = self.value();
        if v.is_none() {
            self.warn(format!("{name}: no value"));
        }
        v
    }

    /// Values of a brace initialiser, flattened (nested braces start a row
    /// marker `None`). Ends after the matching `}`.
    fn list_values(&mut self, name: &str) -> Vec<Option<AipValue>> {
        let mut out = Vec::new();
        let mut depth = 1;
        while depth > 0 {
            match self.peek() {
                None => {
                    self.warn(format!("{name}: list not closed"));
                    break;
                }
                Some(Tok::P('{')) => {
                    self.pos += 1;
                    depth += 1;
                    out.push(None);
                }
                Some(Tok::P('}')) => {
                    self.pos += 1;
                    depth -= 1;
                    if depth >= 1 {
                        out.push(None);
                    }
                }
                // `;` between rows is read as `,` (usrmsn01.aip).
                Some(Tok::P(',')) | Some(Tok::P(';')) => self.pos += 1,
                Some(_) => match self.value() {
                    Some(v) => out.push(Some(v)),
                    None => {
                        let t = self.next();
                        self.warn(format!("{name}: unexpected {t:?} in list"));
                    }
                },
            }
        }
        out
    }

    fn list(&mut self, ty: &str, name: &str, values: Vec<Option<AipValue>>) {
        let fields = self.aip.structs.get(&ty.to_ascii_lowercase()).cloned().unwrap_or_default();
        let nested = values.iter().any(Option::is_none);
        let mut rows: Vec<Vec<AipValue>> = Vec::new();
        if nested {
            let mut row = Vec::new();
            for v in values {
                match v {
                    Some(v) => row.push(v),
                    None if !row.is_empty() => rows.push(std::mem::take(&mut row)),
                    None => {}
                }
            }
            if !row.is_empty() {
                rows.push(row);
            }
        } else if !fields.is_empty() {
            let flat: Vec<AipValue> = values.into_iter().flatten().collect();
            if !flat.len().is_multiple_of(fields.len()) {
                self.warn(format!("{name}: {} values do not fill rows of {} ({ty})", flat.len(), fields.len()));
            }
            rows = flat.chunks(fields.len()).map(<[AipValue]>::to_vec).collect();
        } else {
            // Unknown element type: a row starts at each string after the first value.
            for v in values.into_iter().flatten() {
                if matches!(v, AipValue::Str(_)) || rows.is_empty() {
                    rows.push(Vec::new());
                }
                rows.last_mut().unwrap().push(v);
            }
        }
        let known: BTreeSet<&str> = TYPES.into_iter().collect();
        if fields.is_empty() && !known.contains(ty) {
            self.warn(format!("{name}: element type {ty} not defined (aipdef.h missing?)"));
        }
        self.aip.lists.insert(
            name.to_ascii_lowercase(),
            AipList { name: name.to_owned(), element: ty.to_owned(), fields, rows },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AIPDEF: &str = "\
// test aipdef.h
struct Build_List_Element
{
\tchar unit_type_name[80];\t// name
\tdouble units_to_build;
};
struct Way_Better_Build_List_Element
{
\tchar unit_type_name[80];
\tdouble units_to_build;
\tint m_base_num;
\tint m_buildMAX;
\tint m_techLevel;
};
#define UNLIMITED -999999
#define YES 1
#define MAX_ELEMENTS 200
/*
#define NOT_USED 2
*/
";

    fn files() -> BTreeMap<&'static str, &'static str> {
        BTreeMap::from([
            ("aipdef.h", AIPDEF),
            (
                "base.aip",
                "#include \"aipdef.h\"\nint MAX_EXEC_ATTACK = 1;\nint MAX_EXEC_DEFEND = 3;\nfloat attack_ratio = 0.75;\n",
            ),
            (
                "fed.aip",
                "// header\n#include \"base.aip\"\n\
                 int MAX_EXEC_ATTACK = 4; // override\n\
                 int checkTechnologyAvailable = YES;\n\
                 int never = UNLIMITED;\n\
                 char name[80] = \"Fed AI\";\n\
                 Way_Better_Build_List_Element the_build_list[MAX_ELEMENTS] =\n{\n\
                 \t\"fbase\",\t1,0, -1,  1,\t// starbase\n\
                 \t\"fturret\",  2,1,  4, -3,\n\
                 \t\"fcruise1\", 2.5,0, -1, -3,\n};\n",
            ),
        ])
    }

    fn load(name: &str) -> (Aip, Vec<Warning>) {
        let f = files();
        Aip::load(name, &mut |n| f.get(n.to_ascii_lowercase().as_str()).map(|s| s.to_string()))
    }

    #[test]
    fn scalars_includes_and_overrides() {
        let (aip, w) = load("fed.aip");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(aip.int("MAX_EXEC_ATTACK"), Some(4));
        assert_eq!(aip.int("max_exec_defend"), Some(3));
        assert_eq!(aip.fixed("attack_ratio"), Some(Fixed::from_f64(0.75)));
        assert_eq!(aip.int("checkTechnologyAvailable"), Some(1));
        assert_eq!(aip.int("never"), Some(-999_999));
        assert_eq!(aip.str("name"), Some("Fed AI"));
        assert_eq!(aip.var("name").unwrap().ty, "char[]");
        assert_eq!(aip.define("MAX_ELEMENTS"), Some("200"));
        assert_eq!(aip.define("NOT_USED"), None);
    }

    #[test]
    fn build_list_rows_by_struct() {
        let (aip, _) = load("fed.aip");
        let list = aip.build_list("the_build_list").unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(
            list[1],
            BuildItem { unit: "fturret".into(), count: Fixed::from_int(2), base: Some(1), max: Some(4), tech_level: Some(-3) }
        );
        assert_eq!(list[2].count, Fixed::from_f64(2.5));
        assert_eq!(aip.list("the_build_list").unwrap().element, "Way_Better_Build_List_Element");
    }

    #[test]
    fn short_struct_and_unknown_struct() {
        let text = "#include \"aipdef.h\"\nBuild_List_Element defend[MAX_ELEMENTS] = { \"fbase\", 1, \"fturret\", 3 };\n\
                    Mystery_Element m[4] = { \"a\", 1, 2, \"b\", 3 };\n";
        let f = files();
        let (aip, w) = Aip::load("x.aip", &mut |n| if n == "x.aip" { Some(text.into()) } else { f.get(n).map(|s| s.to_string()) });
        let d = aip.build_list("defend").unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!((d[1].unit.as_str(), d[1].count, d[1].base), ("fturret", Fixed::from_int(3), None));
        let m = aip.list("m").unwrap();
        assert_eq!(m.rows.len(), 2);
        assert_eq!(m.rows[0], [AipValue::Str("a".into()), AipValue::Int(1), AipValue::Int(2)]);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].message.contains("Mystery_Element not defined"));
    }

    #[test]
    fn lenient_like_the_game() {
        // Missing ';', stray '};', ';' between rows, nested rows, '#' comment.
        let text = "int a = 1\nint b = 2;\n# a comment line\n\
                    Unit_Row u[2] = { {\"x\", 1}, {\"y\", 2} };\n\
                    Slush_Row s[2] = { \"Slush\", UNLIMITED, 40; };\n\
                    };\nint c = -1e-2;\n";
        let (aip, w) = Aip::parse("odd.aip", text);
        assert_eq!(aip.int("a"), Some(1));
        assert_eq!(aip.int("b"), Some(2));
        assert_eq!(aip.fixed("c"), Some(Fixed::from_f64(-0.01)));
        assert_eq!(aip.list("u").unwrap().rows.len(), 2);
        assert_eq!(aip.list("s").unwrap().rows[0][1], AipValue::Name("UNLIMITED".into()));
        let msgs: Vec<_> = w.iter().map(|w| w.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("missing ';'")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("stray '}'")), "{msgs:?}");
    }

    #[test]
    fn leftovers_after_a_list_are_one_warning() {
        // SFC: rows after the list's `};` ("DONE TO HERE"); Orb of Time: a
        // brace block with no declaration.
        let text = "#include \"aipdef.h\"\nWay_Better_Build_List_Element l[MAX_ELEMENTS] =\n{\n\"a\", 1,0,-1,0,\n};\n\
                    \"b\", 2,0,-1,0,\n\"c\", 3,0,-1,-2,\n\
                    {\n\"d\", 1,\n};\nint after = 7;\n";
        let f = files();
        let (aip, w) = Aip::load("s.aip", &mut |n| if n == "s.aip" { Some(text.into()) } else { f.get(n).map(|s| s.to_string()) });
        assert_eq!(aip.build_list("l").unwrap().len(), 1);
        assert_eq!(aip.int("after"), Some(7));
        let msgs: Vec<_> = w.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(msgs, ["23 stray tokens outside a declaration ignored (from \"b\")", "list of 2 values without a declaration ignored"]);
    }

    #[test]
    fn line_comment_hides_block_opener() {
        let text = "int a = 1;\n//****line 10****\nint b = 2; // b /* not a block\nchar s[8] = \"x//y\";\n/* gone\nint c = 3;\n*/ int d = 4;\n";
        let (aip, w) = Aip::parse("c.aip", text);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((aip.int("a"), aip.int("b"), aip.int("c"), aip.int("d")), (Some(1), Some(2), None, Some(4)));
        assert_eq!(aip.str("s"), Some("x//y"));
    }

    #[test]
    fn include_loop_and_missing_include_warn() {
        let mut load = |n: &str| match n {
            "a.aip" => Some("#include \"b.aip\"\nint x = 1;\n".to_string()),
            "b.aip" => Some("#include \"a.aip\"\n#include \"gone.h\"\nint y = 2;\n".to_string()),
            _ => None,
        };
        let (aip, w) = Aip::load("a.aip", &mut load);
        assert_eq!((aip.int("x"), aip.int("y")), (Some(1), Some(2)));
        assert_eq!(w.len(), 2, "{w:?}");
    }
}
