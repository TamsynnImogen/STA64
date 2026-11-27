//! The listing the lifters read: one `(address, text)` per instruction, in address order, where
//! the text is what `objdump -d -M intel --no-show-raw-insn` prints after the tab, trimmed
//! (GNU objdump 2.47 is the reference). The decoder is `iced-x86` (decoder only); the printer
//! ([`print`]) is this crate's own.
//!
//! Which bytes are swept and where the tool leaves gaps (runs of zero bytes) is [`sweep`]'s
//! business; the rules and their grades are documented there.

mod badtab;
mod decode;
mod print;
mod sweep;
mod vextab;

use std::collections::HashMap;

use crate::Image;

pub use sweep::disassemble_bytes;

/// One decoded instruction: its virtual address and its text.
pub type Ins = (u32, String);

/// The instructions of an image and the index of each address in them (`ins` and `at` of the
/// Python tools' `Dll` classes).
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub ins: Vec<Ins>,
    pub at: HashMap<u32, usize>,
}

impl Listing {
    /// Builds the index; of two equal addresses the later wins, as in Python's
    /// `{a: i for i, (a, _) in enumerate(ins)}`.
    pub fn new(ins: Vec<Ins>) -> Self {
        let at = ins.iter().enumerate().map(|(i, (a, _))| (*a, i)).collect();
        Self { ins, at }
    }

    /// The linear sweep of the image's code sections.
    ///
    /// Fails for an image with a COFF symbol table (see [`disassemble`]).
    pub fn from_image(img: &dyn Image) -> crate::Result<Self> {
        Ok(Self::new(disassemble(img)?))
    }

    /// The lines the Python tools keep of the reference tool's text, for parity runs.
    pub fn parse_objdump(text: &str) -> Self {
        Self::new(parse_objdump(text))
    }
}

/// Port of the regex of `drl_to_lua.disassemble`: `^\s*([0-9a-f]+):\t(.*)$` per line, the text
/// stripped.
pub fn parse_objdump(text: &str) -> Vec<Ins> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let l = line.trim_start();
        let n = l
            .bytes()
            .take_while(|&b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            .count();
        if n == 0 || !l[n..].starts_with(":\t") {
            continue;
        }
        let Some(addr) = u64::from_str_radix(&l[..n], 16)
            .ok()
            .and_then(|a| u32::try_from(a).ok())
        else {
            continue;
        };
        out.push((addr, l[n + 2..].trim().to_string()));
    }
    out
}

/// The listing of every code section of the image, in section order.
///
/// A file with a COFF symbol table is refused with a decode error: the reference tool prints
/// symbol names after branch targets and restarts its sweep at every symbol, which this listing
/// does not reproduce, so the text would silently differ from what the Python tools read.
pub fn disassemble(img: &dyn Image) -> crate::Result<Vec<Ins>> {
    if img.has_symbol_table() {
        return Err(crate::DecodeError::new(
            "the file has a COFF symbol table (the reference disassembler annotates branch targets and restarts at symbols; not reproduced)",
        ));
    }
    let mut out = Vec::new();
    for s in img.sections() {
        if !sweep::is_swept(s) {
            continue;
        }
        let Some(bytes) = sweep::section_bytes(img, s) else {
            continue;
        };
        let base = img.base().wrapping_add(s.va);
        disassemble_bytes(base, bytes, &mut out);
    }
    Ok(out)
}
