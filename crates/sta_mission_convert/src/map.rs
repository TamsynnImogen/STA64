//! The engine executable's linker `.map` text and the names of its vtable slots (port of
//! `drl_to_lua.engine_vtables` and the two `check_engine` functions).
//!
//! Both are read from the player's own install at run time; this crate holds no engine names.
//! The class list and the expected slots are arguments, supplied by the lifter steps.

use crate::image::Image;
use crate::{DecodeError, Pe, Result};
use indexmap::IndexMap;
use std::path::Path;

/// Python's `str.isspace` (what `\s` matches in a `str` pattern): Unicode whitespace plus
/// U+001C to U+001F.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

fn lower_hex(c: char) -> bool {
    matches!(c, '0'..='9' | 'a'..='f')
}

/// One map line: `\s*[0-9a-f]{4}:[0-9a-f]{8}\s+(\S+)\s+([0-9a-f]{8})\s` matched from the start.
/// Returns (symbol, address).
fn map_line(line: &str) -> Option<(&str, u32)> {
    let s = line.trim_start_matches(py_space);
    let b = s.as_bytes();
    // `seg:offset`: four hex digits, a colon, eight hex digits (all ASCII).
    // Matched on bytes: a non-ASCII character in the first 13 bytes is simply no match, and when
    // all 13 are ASCII, `s[13..]` is on a character boundary.
    let hex = |c: &u8| c.is_ascii_digit() || (b'a'..=b'f').contains(c);
    if b.len() < 13 || !(b[..4].iter().all(hex) && b[4] == b':' && b[5..13].iter().all(hex)) {
        return None;
    }
    let rest = &s[13..];
    let after_ws = rest.trim_start_matches(py_space);
    if after_ws.len() == rest.len() {
        return None;
    }
    let sym_end = after_ws.find(py_space)?;
    let sym = &after_ws[..sym_end];
    if sym.is_empty() {
        return None;
    }
    let rest = &after_ws[sym_end..];
    let after_ws = rest.trim_start_matches(py_space);
    if after_ws.len() == rest.len() {
        return None;
    }
    let mut chars = after_ws.chars();
    let hex: String = chars.by_ref().take(8).collect();
    if hex.len() != 8 || !hex.chars().all(lower_hex) {
        return None;
    }
    // The pattern needs one whitespace character after the address.
    if !chars.next().is_some_and(py_space) {
        return None;
    }
    Some((sym, u32::from_str_radix(&hex, 16).ok()?))
}

/// Symbol names by address from a `.map` file.
#[derive(Debug, Clone, Default)]
pub struct MapFile {
    /// Address to symbol; the first symbol seen at an address wins; in file order.
    pub names: IndexMap<u32, String>,
}

impl MapFile {
    /// Parses map text. Lines end at `\n`, `\r\n` or `\r` (Python's universal newlines); a
    /// final line with no line end does not match, as in the Python tool (its pattern needs
    /// a whitespace character after the address).
    pub fn parse(text: &str) -> Self {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut names = IndexMap::new();
        for line in text.split_inclusive('\n') {
            if let Some((sym, addr)) = map_line(line) {
                names.entry(addr).or_insert_with(|| sym.to_owned());
            }
        }
        Self { names }
    }

    /// Reads a map file, its bytes as Latin-1 (like the Python tool).
    pub fn open(path: &Path) -> Result<Self> {
        let bytes =
            std::fs::read(path).map_err(|e| DecodeError::io(format!("{}: {e}", path.display())))?;
        let text: String = bytes.iter().map(|&c| c as char).collect();
        Ok(Self::parse(&text))
    }

    /// Address of the first symbol (in file order) with exactly this name.
    pub fn address_of(&self, symbol: &str) -> Option<u32> {
        self.names
            .iter()
            .find(|(_, v)| v.as_str() == symbol)
            .map(|(&a, _)| a)
    }
}

/// Vtable slot names per class: `{class: [symbol per slot]}`, in the order of the class list.
pub type Vtables = IndexMap<String, Vec<String>>;

/// The vtable of each class of `classes` that the map names (`??_7<class>@@6B@`): consecutive
/// words of the executable that point into its first `.text*` section, up to the next symbol
/// or the first word that does not. A slot no symbol names reads `?unnamed_<slot>@@`.
/// A class with no vtable symbol is left out.
pub fn engine_vtables_from(exe: &Pe, map: &MapFile, classes: &[&str]) -> Result<Vtables> {
    let text = exe
        .sections
        .iter()
        .find(|s| s.name.starts_with(".text"))
        .map(|s| (exe.base as u64 + s.va as u64, s.vsize as u64))
        .ok_or_else(|| DecodeError::raw("no .text section"))?;
    // STA64's own limit (the Python tool has none): a vtable has at most one slot per word of
    // the executable. Crafted sections that alias the same bytes cannot make the walk longer.
    let max_slots = exe.data.len() / 4;
    let mut out = Vtables::new();
    for cls in classes {
        let Some(vt) = map.address_of(&format!("??_7{cls}@@6B@")) else {
            continue;
        };
        let mut slots: Vec<String> = Vec::new();
        loop {
            if slots.len() >= max_slots {
                return Err(DecodeError::raw("vtable too large"));
            }
            let at = vt.wrapping_add((slots.len() as u32).wrapping_mul(4));
            if !slots.is_empty() && map.names.contains_key(&at) {
                break; // the next symbol starts
            }
            let word = exe.dword(at).ok_or_else(|| {
                DecodeError::raw(format!("vtable word at {at:#x} outside the file"))
            })?;
            if !(text.0 <= word as u64 && (word as u64) < text.0 + text.1) {
                break;
            }
            slots.push(
                map.names
                    .get(&word)
                    .cloned()
                    .unwrap_or_else(|| format!("?unnamed_{}@@", slots.len())),
            );
        }
        out.insert((*cls).to_owned(), slots);
    }
    Ok(out)
}

/// [`engine_vtables_from`] for an Armada II root directory: its executable and `.map` are found
/// by lower-cased file name (`exe_name`, `map_name`, lower case; a later directory entry that
/// lowers to the same name replaces an earlier one). `None` when either is missing.
///
/// Callers port the Python `except (OSError, DecodeError)` (try the next root, or go on with no
/// engine names) as "continue on `ErrorKind::Io` and `ErrorKind::Decode`"; `ErrorKind::Raw`
/// aborts, as the Python's uncaught exceptions do.
pub fn engine_vtables(
    root: &Path,
    exe_name: &str,
    map_name: &str,
    classes: &[&str],
) -> Result<Option<Vtables>> {
    let mut files = IndexMap::new();
    let rd =
        std::fs::read_dir(root).map_err(|e| DecodeError::io(format!("{}: {e}", root.display())))?;
    for entry in rd {
        let entry = entry.map_err(|e| DecodeError::io(e.to_string()))?;
        files.insert(
            entry.file_name().to_string_lossy().to_lowercase(),
            entry.path(),
        );
    }
    let (Some(exe), Some(map)) = (files.get(exe_name), files.get(map_name)) else {
        return Ok(None);
    };
    let pe = Pe::open(exe)?;
    let names = MapFile::open(map)?;
    engine_vtables_from(&pe, &names, classes).map(Some)
}

/// One slot a decoder relies on: the class, the slot number and the symbol prefix it must carry.
#[derive(Debug, Clone, Copy)]
pub struct ExpectedSlot<'a> {
    pub class: &'a str,
    pub slot: usize,
    pub prefix: &'a str,
}

/// The slots the decoder relies on, against the user's executable: one message per slot that is
/// missing or does not start with its prefix. `show_found` adds `, found <symbol or none>` (the
/// rule-DLL tool's message; the mission tool's message has no such tail).
pub fn check_engine(vt: &Vtables, want: &[ExpectedSlot<'_>], show_found: bool) -> Vec<String> {
    let mut bad = Vec::new();
    for w in want {
        let got: &[String] = vt.get(w.class).map_or(&[], |v| v.as_slice());
        if got.get(w.slot).is_none_or(|g| !g.starts_with(w.prefix)) {
            let mut msg = format!("{} slot {}: expected {}", w.class, w.slot, w.prefix);
            if show_found {
                msg.push_str(&format!(
                    ", found {}",
                    got.get(w.slot).map_or("none", |s| s.as_str())
                ));
            }
            bad.push(msg);
        }
    }
    bad
}
