//! The grouped layout of [`Cst::format_grouped`]: entries gathered by role,
//! in the order of a [`GroupTable`] (EDT-06).
//!
//! The table is data. The built-in one, `fo_groups.txt`, names keys only and
//! was authored for STA64 from what the keys do; the Fleet Operations guide's
//! own templates are not in the repository or in any install, so its exact
//! order is not reproduced. A mod can pass its own table.
//!
//! What moves and what does not:
//!
//! * An entry moves together with the comment lines directly above it and the
//!   continuation rows below it. Nothing is dropped or edited; blank lines
//!   between entries are regenerated (one between groups).
//! * Entries never cross an `#` directive, a `[section]` row, a line the
//!   reader ignores or a continuation row without an entry above it.
//! * Comment lines after the last entry of a stretch stay after it.
//! * A file is only reordered when no key occurs twice (any case) and every
//!   line is UTF-8. A repeated key means the last one wins, which order would
//!   change, and the reader's key order ([`crate::Odf::keys`]) is the file's
//!   order, so such a file is only whitespace-formatted.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::{Cst, Line, LineKind};

const FO_GROUPS: &str = include_str!("fo_groups.txt");

/// An ordered list of key groups. See `fo_groups.txt` for the text format.
#[derive(Debug, Clone)]
pub struct GroupTable {
    names: Vec<String>,
    exact: HashMap<String, usize>,
    digits: HashMap<String, usize>,
    /// (pattern, group, is_prefix)
    stars: Vec<(String, usize, bool)>,
}

impl GroupTable {
    /// Parse the table text. The error names the 1-based line.
    pub fn parse(text: &str) -> Result<GroupTable, String> {
        let mut t = GroupTable {
            names: Vec::new(),
            exact: HashMap::new(),
            digits: HashMap::new(),
            stars: Vec::new(),
        };
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                t.names.push(name.trim().to_owned());
                continue;
            }
            let Some(group) = t.names.len().checked_sub(1) else {
                return Err(format!("line {}: key before the first [group]", n + 1));
            };
            let key = line.to_ascii_lowercase();
            if let Some(p) = key.strip_suffix('*') {
                if p.is_empty() || p.contains('*') {
                    return Err(format!("line {}: bad pattern", n + 1));
                }
                t.stars.push((p.to_owned(), group, true));
            } else if let Some(p) = key.strip_prefix('*') {
                if p.contains('*') {
                    return Err(format!("line {}: bad pattern", n + 1));
                }
                t.stars.push((p.to_owned(), group, false));
            } else if let Some(p) = key.strip_suffix('#') {
                t.digits.entry(p.to_owned()).or_insert(group);
            } else {
                t.exact.entry(key).or_insert(group);
            }
        }
        if t.names.is_empty() {
            return Err("no [group] in the table".into());
        }
        Ok(t)
    }

    /// The built-in table (`fo_groups.txt`).
    pub fn fo() -> &'static GroupTable {
        static T: OnceLock<GroupTable> = OnceLock::new();
        T.get_or_init(|| GroupTable::parse(FO_GROUPS).expect("fo_groups.txt is valid"))
    }

    /// Group names in output order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Index of the group of `key` (any case); `names().len()` for a key
    /// the table does not name.
    pub fn group_of(&self, key: &str) -> usize {
        let key = key.trim().to_ascii_lowercase();
        if let Some(&g) = self.exact.get(&key) {
            return g;
        }
        let stem = key.trim_end_matches(|c: char| c.is_ascii_digit());
        if stem.len() < key.len()
            && let Some(&g) = self.digits.get(stem)
        {
            return g;
        }
        let mut best: Option<(usize, usize)> = None;
        for &(ref p, g, prefix) in &self.stars {
            let hit = if prefix {
                key.starts_with(p.as_str())
            } else {
                key.ends_with(p.as_str())
            };
            if hit && best.is_none_or(|(len, _)| p.len() > len) {
                best = Some((p.len(), g));
            }
        }
        best.map_or(self.names.len(), |(_, g)| g)
    }
}

type Row = (Vec<u8>, Vec<u8>);

impl Cst {
    /// [`Cst::format`] and then the entries gathered by group, in the order
    /// of `table`. Idempotent. See the module docs for what moves.
    pub fn format_grouped(&self, table: &GroupTable) -> Vec<u8> {
        let plain = self.format();
        let f = Cst::parse(&plain);
        match f.regroup(table) {
            Some(out) => out,
            None => plain,
        }
    }

    fn key_of(&self, l: &Line) -> String {
        let t = String::from_utf8_lossy(self.text(l));
        let code = &t[..l.comment.unwrap_or(t.len())];
        code.split_once('=')
            .map_or("", |(k, _)| k)
            .trim()
            .to_ascii_lowercase()
    }

    fn regroup(&self, table: &GroupTable) -> Option<Vec<u8>> {
        if self.lines.iter().any(|l| !l.valid) {
            return None;
        }
        let mut seen = std::collections::HashSet::new();
        for l in self.lines.iter().filter(|l| l.kind == LineKind::Entry) {
            if !seen.insert(self.key_of(l)) {
                return None;
            }
        }
        let eol: Vec<u8> = self
            .lines
            .iter()
            .map(|l| self.eol(l))
            .find(|e| !e.is_empty())
            .unwrap_or(b"\n")
            .to_vec();
        let ends_without_eol = self.lines.last().is_some_and(|l| self.eol(l).is_empty());
        let row = |l: &Line| -> Row { (self.text(l).to_vec(), self.eol(l).to_vec()) };
        let blank = || -> Row { (Vec::new(), eol.clone()) };

        let mut rows: Vec<Row> = Vec::new();
        let mut body: Vec<&Line> = Vec::new();
        for l in &self.lines {
            let barrier = matches!(
                l.kind,
                LineKind::Directive | LineKind::Section | LineKind::Other
            ) || (l.kind == LineKind::Continuation && !l.continues);
            if barrier {
                rows.extend(self.group_body(&body, table, &blank)?);
                body.clear();
                rows.push(row(l));
            } else {
                body.push(l);
            }
        }
        rows.extend(self.group_body(&body, table, &blank)?);

        let last = rows.len().saturating_sub(1);
        let mut out = Vec::with_capacity(self.src.len() + 64);
        for (i, (text, e)) in rows.iter().enumerate() {
            out.extend_from_slice(text);
            if i == last && ends_without_eol {
                // The file had no final line ending: keep it so.
            } else if e.is_empty() {
                out.extend_from_slice(&eol);
            } else {
                out.extend_from_slice(e);
            }
        }
        Some(out)
    }

    /// One stretch between barriers. `None` when something does not fit the
    /// model (the caller then leaves the file as `format` made it).
    fn group_body(
        &self,
        body: &[&Line],
        table: &GroupTable,
        blank: &dyn Fn() -> Row,
    ) -> Option<Vec<Row>> {
        let row = |l: &Line| -> Row { (self.text(l).to_vec(), self.eol(l).to_vec()) };
        if !body.iter().any(|l| l.kind == LineKind::Entry) {
            return Some(body.iter().map(|l| row(l)).collect());
        }
        let lead_blank = body[0].kind == LineKind::Blank;
        let trail_blank = body[body.len() - 1].kind == LineKind::Blank;
        // A comment run set off from the entry below it by a blank line is
        // not that entry's comment: it stays under the entry above it (or at
        // the top, when there is none) and keeps its blank line.
        struct Item {
            group: usize,
            lines: Vec<Row>,
            tail: Vec<Row>,
        }
        let mut header: Vec<Row> = Vec::new();
        let mut items: Vec<Item> = Vec::new();
        let mut pending: Vec<Row> = Vec::new();
        let mut blank_after_pending = false;
        let mut gap_before_trailing = false;
        let mut i = 0;
        while i < body.len() {
            let l = body[i];
            match l.kind {
                LineKind::Blank => {
                    if !pending.is_empty() {
                        blank_after_pending = true;
                    } else if !items.is_empty() {
                        gap_before_trailing = true;
                    }
                    i += 1;
                }
                LineKind::Comment => {
                    if blank_after_pending {
                        let run = std::mem::take(&mut pending);
                        match items.last_mut() {
                            Some(prev) => prev.tail.extend(run),
                            None => header.extend(run),
                        }
                        blank_after_pending = false;
                    }
                    pending.push(row(l));
                    i += 1;
                }
                LineKind::Entry => {
                    if blank_after_pending {
                        let run = std::mem::take(&mut pending);
                        match items.last_mut() {
                            Some(prev) => prev.tail.extend(run),
                            None => header.extend(run),
                        }
                    }
                    blank_after_pending = false;
                    let mut lines = std::mem::take(&mut pending);
                    lines.push(row(l));
                    // Continuation rows may follow after comments or blanks.
                    let mut end = i;
                    let mut k = i + 1;
                    while k < body.len()
                        && matches!(
                            body[k].kind,
                            LineKind::Comment | LineKind::Blank | LineKind::Continuation
                        )
                    {
                        if body[k].kind == LineKind::Continuation {
                            end = k;
                        }
                        k += 1;
                    }
                    for m in &body[i + 1..=end] {
                        if m.kind != LineKind::Blank {
                            lines.push(row(m));
                        }
                    }
                    items.push(Item {
                        group: table.group_of(&self.key_of(l)),
                        lines,
                        tail: Vec::new(),
                    });
                    gap_before_trailing = false;
                    i = end + 1;
                }
                _ => return None,
            }
        }
        items.sort_by_key(|it| it.group);
        let mut out = Vec::new();
        if lead_blank {
            out.push(blank());
        }
        if !header.is_empty() {
            out.extend(header);
            out.push(blank());
        }
        let mut prev = None;
        for it in items {
            if prev.is_some_and(|p| p != it.group) {
                out.push(blank());
            }
            prev = Some(it.group);
            out.extend(it.lines);
            if !it.tail.is_empty() {
                out.extend(it.tail);
                out.push(blank());
            }
        }
        if !pending.is_empty() {
            if gap_before_trailing {
                out.push(blank());
            }
            out.extend(pending);
        }
        if trail_blank {
            out.push(blank());
        }
        // One blank line at most in a row, and no blank at the end that the
        // stretch did not have.
        let mut deduped: Vec<Row> = Vec::with_capacity(out.len());
        for r in out {
            let is_blank = r.0.is_empty();
            if !(is_blank && deduped.last().is_some_and(|p| p.0.is_empty())) {
                deduped.push(r);
            }
        }
        while !trail_blank && deduped.last().is_some_and(|r| r.0.is_empty()) {
            deduped.pop();
        }
        Some(deduped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn sorted_pairs(src: &[u8]) -> Vec<(String, String)> {
        let r = parse(&String::from_utf8_lossy(src));
        let mut v: Vec<_> = r
            .entries
            .iter()
            .map(|e| (e.key.to_ascii_lowercase(), e.value.clone()))
            .collect();
        v.sort();
        v
    }

    const SRC: &str = "// ship\r\nmaxHealth = 100\r\n\r\n// the name\r\nunitName = \"X\"\r\nweapon1 = \"a\"\r\nmystery = 1 // keep\r\nclassLabel = \"starship\"\r\nmetalCost = 5\r\nnames = \"A\"\r\n  \"B\"\r\nweapon2 = \"b\"\r\n\r\n// end note\r\n";

    #[test]
    fn groups_entries_and_keeps_comments_continuations_and_unknown_keys() {
        let t = GroupTable::fo();
        let out = Cst::parse(SRC.as_bytes()).format_grouped(t);
        let s = String::from_utf8(out.clone()).unwrap();
        let pos = |needle: &str| s.find(needle).unwrap_or_else(|| panic!("missing {needle}"));
        assert!(pos("// the name") < pos("classLabel"), "same group keeps file order");
        assert!(pos("unitName") < pos("metalCost"));
        assert!(pos("metalCost") < pos("maxHealth"));
        assert!(pos("maxHealth") < pos("weapon1"));
        assert!(pos("weapon1") < pos("weapon2"));
        assert!(pos("weapon2") < pos("mystery = 1 // keep"));
        assert!(s.contains("names = \"A\"\r\n\"B\"\r\n") || s.contains("names = \"A\"\r\n  \"B\"\r\n"));
        assert!(s.ends_with("// end note\r\n"));
        assert!(s.contains("// the name\r\nunitName"));
        assert!(!s.contains('\n') || s.matches('\n').count() == s.matches("\r\n").count());
        assert_eq!(sorted_pairs(SRC.as_bytes()), sorted_pairs(&out));
        let again = Cst::parse(&out).format_grouped(t);
        assert_eq!(again, out, "idempotent");
    }

    #[test]
    fn repeated_keys_and_foreign_bytes_are_not_reordered() {
        let t = GroupTable::fo();
        for src in [
            &b"maxHealth = 1\nclassLabel = \"a\"\nMAXHEALTH = 2\n"[..],
            b"maxHealth = 1\nclassLabel = \"\xe9\"\n",
        ] {
            let cst = Cst::parse(src);
            assert_eq!(cst.format_grouped(t), cst.format());
        }
    }

    #[test]
    fn entries_do_not_cross_directives_or_sections() {
        let t = GroupTable::fo();
        let src = b"maxHealth = 1\n#include \"b.odf\"\nclassLabel = \"a\"\n[S]\nweapon1 = 1\nrace = \"x\"";
        let out = Cst::parse(src).format_grouped(t);
        assert_eq!(out, b"maxHealth = 1\n#include \"b.odf\"\nclassLabel = \"a\"\n[S]\nrace = \"x\"\n\nweapon1 = 1");
    }

    #[test]
    fn no_final_newline_stays_that_way_and_lf_stays_lf() {
        let t = GroupTable::fo();
        let out = Cst::parse(b"weapon1 = 1\nclassLabel = \"a\"").format_grouped(t);
        assert_eq!(out, b"classLabel = \"a\"\n\nweapon1 = 1");
        let out = Cst::parse(b"weapon1 = 1\nclassLabel = \"a\"\n").format_grouped(t);
        assert_eq!(out, b"classLabel = \"a\"\n\nweapon1 = 1\n");
    }

    #[test]
    fn a_comment_set_off_by_a_blank_line_stays_put() {
        let t = GroupTable::fo();
        let src = b"// header\n\nweapon1 = 1\n// about weapon1\n\n// about class\nclassLabel = 2\n\n// footer\n";
        let out = Cst::parse(src).format_grouped(t);
        assert_eq!(
            out,
            b"// header\n\n// about class\nclassLabel = 2\n\nweapon1 = 1\n// about weapon1\n\n// footer\n"
        );
        assert_eq!(Cst::parse(&out).format_grouped(t), out);
    }

    #[test]
    fn table_lookup_rules() {
        let t = GroupTable::parse("[a]\nfoo#\nbar*\n*baz\nexact\n[b]\nbarbaz\nbarz*\n").unwrap();
        assert_eq!(t.group_of("FOO12"), 0);
        assert_eq!(t.group_of("foo"), t.names().len());
        assert_eq!(t.group_of("barq"), 0);
        assert_eq!(t.group_of("barbaz"), 1, "exact beats patterns");
        assert_eq!(t.group_of("barzx"), 1, "longest pattern wins");
        assert_eq!(t.group_of("zzbaz"), 0);
        assert_eq!(t.group_of("nothing"), 2);
        assert!(GroupTable::parse("key\n").is_err());
        assert!(GroupTable::parse("[g]\n*\n").is_err());
    }

    #[test]
    fn built_in_table_loads_and_orders_the_common_keys() {
        let t = GroupTable::fo();
        assert!(t.group_of("classLabel") < t.group_of("metalCost"));
        assert!(t.group_of("metalCost") < t.group_of("maxHealth"));
        assert_eq!(t.group_of("weaponHardpoints3"), t.group_of("weapon1"));
        assert_eq!(t.group_of("somethingNew"), t.names().len());
    }
}
