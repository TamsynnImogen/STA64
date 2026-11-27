//! The listing the lifters read: one `(address, text)` per instruction, in address order, where
//! the text is what `objdump -d -M intel --no-show-raw-insn` prints after the tab, trimmed
//! (GNU objdump 2.47 is the reference). The decoder is `iced-x86` (decoder only); the printer
//! ([`print`]) is this crate's own.
//!
//! Which bytes are swept and where the tool leaves gaps (runs of zero bytes) is [`sweep`]'s
//! business; the rules and their grades are documented there.

mod decode;
mod print;
mod sweep;

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
    pub fn from_image(img: &dyn Image) -> Self {
        Self::new(disassemble(img))
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
        let n = l.bytes().take_while(u8::is_ascii_hexdigit).count();
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
pub fn disassemble(img: &dyn Image) -> Vec<Ins> {
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
    out
}
