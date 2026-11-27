//! The linear sweep: which sections, which bytes, and where the reference tool leaves gaps.
//!
//! - Sections: those the tool marks as code (`CNT_CODE` or `MEM_EXECUTE`, either alone) that have
//!   file data, in section order, each from its start for the smaller of its virtual and raw
//!   size (the raw size when the virtual size is zero). Measured on the installed DLLs and on
//!   authored section flags.
//! - Zero runs: at an instruction boundary, a run of at least eight zero bytes (or a run of one or
//!   two that ends the section) is skipped, rounded down to a multiple of four unless it ends the
//!   section; the sweep resumes after the skipped bytes and the tool's `...` line carries no
//!   address, so the listing has none. Alignment padding and data bytes in the code section are
//!   decoded like any other bytes.
//! - There is no resynchronisation: after an instruction (or a `(bad)`) the next one starts at
//!   the next byte the tool's length rule leaves.

use super::Ins;
use super::decode::Sweeper;
use crate::Image;
use crate::image::Section;

/// A section is swept when the reference tool marks it as code: the flags `CNT_CODE` or
/// `MEM_EXECUTE` (each alone marks it; measured on authored section flags) and it has file data.
pub(crate) fn is_swept(s: &Section) -> bool {
    s.is_code() && s.raw_size != 0
}

/// The bytes of a section the tool sweeps: the smaller of the virtual and the raw size.
pub(crate) fn section_bytes<'a>(img: &'a dyn Image, s: &Section) -> Option<&'a [u8]> {
    let size = if s.vsize == 0 {
        s.raw_size
    } else {
        s.vsize.min(s.raw_size)
    } as usize;
    let off = s.raw_off as usize;
    let data = img.data();
    let end = off.checked_add(size)?.min(data.len());
    data.get(off..end)
}

/// Runs of at least this many zero bytes are skipped.
const SKIP_ZEROES: usize = 8;
/// A run of fewer than this many zero bytes that reaches the end of the section is skipped.
const SKIP_ZEROES_AT_END: usize = 3;

/// Sweeps `data` (loaded at `base`), appending one entry per instruction.
pub fn disassemble_bytes(base: u32, data: &[u8], out: &mut Vec<Ins>) {
    let len = data.len();
    let mut sw = Sweeper::new(base, data);
    let mut pos = 0usize;
    while pos < len {
        let mut z = pos;
        while z < len && data[z] == 0 {
            z += 1;
        }
        let run = z - pos;
        if run >= SKIP_ZEROES || (z == len && run > 0 && run < SKIP_ZEROES_AT_END) {
            pos = if z != len { pos + (run & !3) } else { z };
            continue;
        }
        let (n, text) = sw.decode(pos);
        out.push((base.wrapping_add(pos as u32), text));
        pos += n.max(1);
    }
}
