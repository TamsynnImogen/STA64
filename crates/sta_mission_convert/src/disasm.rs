//! x86 decoding for the lifters: the text of every instruction in objdump's `-M intel` style,
//! so the ported lifters read the same lines the Python tools read.

use crate::pe::Pe;

/// `(address, text)` of every instruction in the PE's code sections, in address order; the text is
/// what `objdump -d -M intel --no-show-raw-insn` prints after the tab (trimmed).
pub fn disassemble(_pe: &Pe) -> Vec<(u32, String)> {
    todo!("LUA-15: iced-x86 with objdump-compatible formatting")
}
