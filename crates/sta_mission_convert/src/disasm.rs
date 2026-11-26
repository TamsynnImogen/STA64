//! The listing the lifters read (filled in by the decoder step, LUA-15b).
//!
//! Contract: one `(address, text)` per instruction, in address order, where the text is what
//! `objdump -d -M intel --no-show-raw-insn` prints after the tab, trimmed. The decoder is
//! `iced-x86` (decoder only); the printer is this crate's own.

/// One decoded instruction: its virtual address and its text.
pub type Ins = (u32, String);
