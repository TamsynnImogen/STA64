//! A lossless ODF syntax tree and a formatter over it (EDT-06).
//!
//! [`Cst::parse`] splits the raw bytes of a file into lines and classifies
//! each the way [`crate::parse`] reads it (blank, comment, directive,
//! section, entry, continuation row, other). Nothing is dropped or decoded:
//! comments, indentation, key order, `#include` rows, every line ending (LF,
//! CRLF, mixed, missing at the end of the file), a byte-order mark and bytes
//! that are not UTF-8 are all kept, so [`Cst::to_bytes`] returns the input
//! byte for byte.
//!
//! [`Cst::format`] rewrites only whitespace, never values: it normalises
//! `key = value` spacing, trims trailing blanks, puts one space before a
//! trailing `//` comment and collapses runs of blank lines. Keys, values,
//! comment text, line order, line endings and lines that are not valid UTF-8
//! are untouched, so the formatted file resolves to the same values as the
//! input (checked against `install` ODFs in `tests/cst_install.rs`). The
//! "grouped layout after the FO Guide templates" of the WORKPLAN row is not
//! done here: `format_odfs.py` and the templates are not in the repository,
//! and reordering keys can change what a layered resolve returns when a key
//! is written twice (the last wins) and changes [`crate::Odf::keys`] order.
//! It belongs in an opt-in mode over this tree.
//!
//! Dropping repeated blank lines shifts the line numbers the reader reports
//! (`Entry::line`, duplicate diagnostics) for a formatted file.
//!
//! Lines that are not valid UTF-8 are classified as the reader sees them (it
//! decodes lossily), but [`Cst::format`] copies them verbatim.

use std::ops::Range;

/// What a line is. Mirrors the branches of [`crate::parse`], in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Nothing, or only whitespace.
    Blank,
    /// Only a `//` comment.
    Comment,
    /// A `#` row: `#include "x.odf"` or another directive.
    Directive,
    /// `[section]`.
    Section,
    /// `key = value`, possibly with a trailing comment.
    Entry,
    /// A row of quoted strings; it continues the entry above it when one is
    /// open (see [`Line::continues`]).
    Continuation,
    /// Content the reader ignores (stray text, `= value` without a key).
    Other,
}

/// One line of the file: its text, its end-of-line bytes and its class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// Bytes of the line without its terminator.
    text: Range<usize>,
    /// The terminator: `\n`, `\r\n` or empty (last line, no newline).
    eol: Range<usize>,
    /// Offset in `text` of the `//` that starts the comment, if any.
    comment: Option<usize>,
    /// For a [`LineKind::Continuation`]: an entry is open above it.
    pub continues: bool,
    /// The text is valid UTF-8, so `comment` is a byte offset into it. When
    /// it is not, the line was classified on its lossy decoding and
    /// `comment` is `None`.
    valid: bool,
}

/// The syntax tree of one ODF file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cst {
    src: Vec<u8>,
    lines: Vec<Line>,
}

impl Cst {
    /// Parse any bytes. Never fails: unreadable content is kept as
    /// [`LineKind::Other`].
    pub fn parse(src: &[u8]) -> Cst {
        let mut lines = Vec::new();
        let mut open = false;
        let mut pos = 0;
        while pos < src.len() {
            let nl = src[pos..].iter().position(|&b| b == b'\n').map(|i| pos + i);
            let (text_end, end) = match nl {
                Some(i) if i > pos && src[i - 1] == b'\r' => (i - 1, i + 1),
                Some(i) => (i, i + 1),
                None => (src.len(), src.len()),
            };
            let line = classify(src, pos..text_end, text_end..end, &mut open);
            lines.push(line);
            pos = end;
        }
        Cst {
            src: src.to_vec(),
            lines,
        }
    }

    /// The file rebuilt from its lines and their terminators: the input,
    /// byte for byte. (Built from the tree, not copied, so comparing it with
    /// the input checks the tree.)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.src.len());
        for l in &self.lines {
            out.extend_from_slice(self.text(l));
            out.extend_from_slice(self.eol(l));
        }
        out
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Text of a line, without its terminator.
    pub fn text(&self, line: &Line) -> &[u8] {
        &self.src[line.text.clone()]
    }

    /// The terminator of a line (`b"\n"`, `b"\r\n"` or empty).
    pub fn eol(&self, line: &Line) -> &[u8] {
        &self.src[line.eol.clone()]
    }

    /// The `#include` names, in order, as [`crate::parse`] reads them.
    pub fn includes(&self) -> Vec<String> {
        let mut out = Vec::new();
        for l in self.lines.iter().filter(|l| l.kind == LineKind::Directive) {
            let text = String::from_utf8_lossy(self.text(l));
            let code = text[..comment_start(&text).unwrap_or(text.len())].trim();
            let rest = code.strip_prefix('#').unwrap_or("").trim_start();
            if rest.len() >= 7
                && rest.is_char_boundary(7)
                && rest[..7].eq_ignore_ascii_case("include")
            {
                let name = crate::unquote(rest[7..].trim());
                if !name.is_empty() {
                    out.push(name.to_owned());
                }
            }
        }
        out
    }

    /// The formatted file. Idempotent. See the module docs for what changes.
    pub fn format(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.src.len());
        let mut blank_run = 0;
        for l in &self.lines {
            let text = std::str::from_utf8(self.text(l)).ok().filter(|_| l.valid);
            let Some(text) = text else {
                blank_run = 0;
                out.extend_from_slice(self.text(l));
                out.extend_from_slice(self.eol(l));
                continue;
            };
            if l.kind == LineKind::Blank {
                blank_run += 1;
                if blank_run > 1 {
                    continue;
                }
            } else {
                blank_run = 0;
            }
            let (code, comment) = match l.comment {
                Some(c) => (&text[..c], Some(text[c..].trim_end())),
                None => (text, None),
            };
            let code = code.trim();
            match l.kind {
                LineKind::Blank => {}
                LineKind::Entry => {
                    // The reader splits at the first '='.
                    let (k, v) = code.split_once('=').unwrap_or((code, ""));
                    out.extend_from_slice(k.trim().as_bytes());
                    out.extend_from_slice(b" =");
                    if !v.trim().is_empty() {
                        out.push(b' ');
                        out.extend_from_slice(v.trim().as_bytes());
                    }
                }
                _ => out.extend_from_slice(code.as_bytes()),
            }
            if let Some(c) = comment {
                if !code.is_empty() && code != "\u{feff}" {
                    out.push(b' ');
                }
                out.extend_from_slice(c.as_bytes());
            }
            out.extend_from_slice(self.eol(l));
        }
        out
    }
}

fn classify(src: &[u8], text: Range<usize>, eol: Range<usize>, open: &mut bool) -> Line {
    let mut line = Line {
        kind: LineKind::Other,
        text: text.clone(),
        eol,
        comment: None,
        continues: false,
        valid: true,
    };
    // The reader decodes lossily; so does the classification. Every syntax
    // character is ASCII, so the decoding cannot change what it finds.
    let lossy = String::from_utf8_lossy(&src[text]);
    line.valid = matches!(lossy, std::borrow::Cow::Borrowed(_));
    let s: &str = &lossy;
    let cut = comment_start(s);
    line.comment = cut.filter(|_| line.valid);
    let code = s[..cut.unwrap_or(s.len())].trim();
    if code.is_empty() {
        line.kind = if cut.is_some() {
            LineKind::Comment
        } else {
            LineKind::Blank
        };
        return line;
    }
    if code.starts_with('#') {
        *open = false;
        line.kind = LineKind::Directive;
    } else if code.starts_with('[') && code.ends_with(']') {
        *open = false;
        line.kind = LineKind::Section;
    } else if is_quoted_continuation(code) {
        line.kind = LineKind::Continuation;
        line.continues = *open;
    } else {
        *open = false;
        if code
            .split_once('=')
            .is_some_and(|(k, _)| !k.trim().is_empty())
        {
            line.kind = LineKind::Entry;
            *open = true;
        }
    }
    line
}

/// Offset of the `//` that starts a comment, outside quotes (the same rule
/// as the reader's).
fn comment_start(line: &str) -> Option<usize> {
    let mut in_quote = false;
    let b = line.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b'"' => in_quote = !in_quote,
            b'/' if !in_quote && b.get(i + 1) == Some(&b'/') => return Some(i),
            _ => {}
        }
    }
    None
}

fn is_quoted_continuation(line: &str) -> bool {
    let mut rest = line.trim();
    if !rest.starts_with('"') {
        return false;
    }
    while let Some(quoted) = rest.strip_prefix('"') {
        let Some(end) = quoted.find('"') else {
            return false;
        };
        rest = quoted[end + 1..].trim_start();
        if rest.is_empty() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    const SAMPLE: &str = "\u{feff}// header\r\n#include \"base.odf\"  // inc\r\n\r\n\r\n\r\n[Sec]\r\nclassLabel=\"starship\"   // trailing\r\n  maxHealth   =  1200  \r\nnames = \"A\" \"B\"\r\n   // between\r\n\"C // not comment\"   \r\nstray words\r\n= nokey\r\nempty =\r\nlast = 1";

    fn same_values(a: &str, b: &str) {
        let (a, b) = (parse(a), parse(b));
        assert_eq!(a.includes, b.includes);
        let kv = |r: &crate::RawOdf| {
            r.entries
                .iter()
                .map(|e| (e.key.clone(), e.value.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(kv(&a), kv(&b));
    }

    #[test]
    fn round_trips_byte_for_byte() {
        for src in [
            SAMPLE.as_bytes(),
            b"",
            b"\n",
            b"\r\n\r\n",
            b"a = 1\rb = 2\n",
            b"a = 1\n\r\nb=2\r\n",
            b"a = \xe9\xe8 // latin-1\nb = 2\n",
            b"\xff\xfe\x00a\x00",
            b"no newline",
        ] {
            let cst = Cst::parse(src);
            assert_eq!(cst.to_bytes(), src);
            let glued: Vec<u8> = cst
                .lines()
                .iter()
                .flat_map(|l| [cst.text(l), cst.eol(l)].concat())
                .collect();
            assert_eq!(glued, src, "lines plus terminators rebuild the file");
        }
    }

    #[test]
    fn classifies_like_the_reader() {
        let cst = Cst::parse(SAMPLE.as_bytes());
        let kinds: Vec<LineKind> = cst.lines().iter().map(|l| l.kind).collect();
        use LineKind::*;
        // The BOM makes the first row not a comment only-line in the reader
        // either (U+FEFF is not whitespace): it is content without a key.
        assert_eq!(
            kinds,
            [
                Other,
                Directive,
                Blank,
                Blank,
                Blank,
                Section,
                Entry,
                Entry,
                Entry,
                Comment,
                Continuation,
                Other,
                Other,
                Entry,
                Entry
            ]
        );
        assert!(cst.lines()[10].continues);
        assert_eq!(cst.includes(), parse(SAMPLE).includes);
    }

    #[test]
    fn format_keeps_values_and_is_idempotent() {
        let cst = Cst::parse(SAMPLE.as_bytes());
        let out = cst.format();
        let out_s = String::from_utf8(out.clone()).unwrap();
        same_values(SAMPLE, &out_s);
        assert_eq!(Cst::parse(&out).format(), out);
        assert!(
            out_s.contains("classLabel = \"starship\" // trailing\r\n"),
            "{out_s:?}"
        );
        assert!(out_s.contains("maxHealth = 1200\r\n"));
        assert!(out_s.contains("empty =\r\n"));
        assert!(out_s.ends_with("last = 1"), "no newline added");
        assert!(!out_s.contains("\r\n\r\n\r\n"), "blank runs collapse");
        assert!(out_s.contains("\"C // not comment\"\r\n"));
    }

    #[test]
    fn format_leaves_foreign_bytes_and_line_endings() {
        let src = b"a=1\rb = \xe9\xe8  \nc   =   3\r\n";
        let out = Cst::parse(src).format();
        // Line one holds a bare CR (not a terminator for the reader) and
        // bytes that are not UTF-8: it stays verbatim, spaces and all.
        assert_eq!(out, b"a=1\rb = \xe9\xe8  \nc = 3\r\n");
        let out = Cst::parse(b"a=1\rb = 2  \nc   =   3\r\n").format();
        assert_eq!(out, b"a = 1\rb = 2\nc = 3\r\n");
    }

    #[test]
    fn unchanged_when_already_formatted() {
        let src = "// c\n#include \"a.odf\"\n\nkey = \"v\" // x\nlist =\n\"a\" \"b\"\n";
        assert_eq!(Cst::parse(src.as_bytes()).format(), src.as_bytes());
    }

    #[test]
    fn fixture_odfs_round_trip_and_format_to_the_same_values() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/odf");
        let mut n = 0;
        let mut stack = vec![root];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "odf") {
                    let bytes = std::fs::read(&p).unwrap();
                    let cst = Cst::parse(&bytes);
                    assert_eq!(cst.to_bytes(), bytes, "{p:?}");
                    let out = cst.format();
                    assert_eq!(Cst::parse(&out).format(), out, "{p:?} idempotent");
                    same_values(
                        &String::from_utf8_lossy(&bytes),
                        &String::from_utf8_lossy(&out),
                    );
                    n += 1;
                }
            }
        }
        assert!(n > 0, "no fixture ODFs found");
    }

    #[test]
    fn matches_the_reader_on_edge_cases_and_never_panics() {
        let cases: &[&[u8]] = &[
            b"#remark\xc3\xa9 x\n",
            b"#includ\xc3\xa9 \"a.odf\"\n",
            b"#abcdef\xe9\n",
            b"#include \"caf\xe9.odf\" // c\nname = \"Caf\xe9\"\n\"more\"\n",
            b"\xef\xbb\xbf// bom comment\n",
            b"a = \"x // y\" // z\n\"q\"\n  \"r\" \"s\"  \n= nokey\n[sec]\n\"orphan\"\n",
            b"a = \"unbalanced\n// still inside the quote\n",
            b"k =\n\"a\"\n\n// c\n\"b\"\nnext = 1\n\"c\"\n",
            b"\0\0=\0\n",
        ];
        for &c in cases {
            let cst = Cst::parse(c);
            assert_eq!(cst.to_bytes(), c);
            let lossy = String::from_utf8_lossy(c).into_owned();
            let raw = parse(&lossy);
            assert_eq!(cst.includes(), raw.includes, "{c:?}");
            let entries = cst
                .lines()
                .iter()
                .filter(|l| l.kind == LineKind::Entry)
                .count();
            assert_eq!(entries, raw.entries.len(), "{c:?}");
            let out = cst.format();
            assert_eq!(Cst::parse(&out).format(), out, "{c:?} idempotent");
            same_values(&lossy, &String::from_utf8_lossy(&out));
        }
    }

    #[test]
    fn non_utf8_lines_are_classified_like_the_reader_but_kept_verbatim() {
        let src: &[u8] = b"name = \"Caf\xe9\"\n\"more\"\n";
        let cst = Cst::parse(src);
        assert_eq!(cst.lines()[0].kind, LineKind::Entry);
        assert!(cst.lines()[1].continues);
        assert_eq!(cst.format(), src);
    }

    #[test]
    fn a_bom_before_a_comment_gets_no_space() {
        let src = "\u{feff}// x\n";
        assert_eq!(Cst::parse(src.as_bytes()).format(), src.as_bytes());
    }
}
