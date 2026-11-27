//! The instruction text, printed the way GNU objdump 2.47 prints it with `-M intel` (the
//! printer is this crate's own; `iced-x86` only decodes).
//!
//! Evidence grade: the forms are read from the reference tool's output on authored bytes and
//! checked against it on the installed mission DLLs by the install-gated test.
//!
//! The rules, in the order the text is built:
//! - the mnemonic is lower case; the decoder's width variants of `pusha`, `popa`, `pushf`,
//!   `popf` and `iret` use the plain name, `xlatb` is `xlat`, the string forms (`movsb`, `stosd`,
//!   ...) are `movs`, `stos`, ..., `sal` is `shl`, `fwait` is `fwait`, and `sgdt`, `sidt`, `lgdt`,
//!   `lidt` get a `d`;
//! - prefix words come first, one space after each, in the order of the bytes; the mnemonic with
//!   its prefix words is padded to six columns plus one space (a single space past that), and
//!   the operands follow, separated by `,`;
//! - registers are lower case; a memory operand is `SIZE PTR seg:[base+index*scale+disp]` with
//!   the size word from the operand width (none for `lea`, the descriptor-table forms and the
//!   x87 environment forms), the segment only when a prefix names one, a displacement written
//!   `+0x..` or `-0x..` (signed, with a `+0x0` for a zero displacement that is encoded), `*1`
//!   spelled out, a SIB byte without an index spelled `eiz`, no scale in 16-bit addressing;
//! - an address without any register is `ds:0x..` (or the override), and the `moffs` forms of
//!   `mov` (`a0` to `a3`) carry no size word;
//! - immediates are unsigned hex of the operand size (`0xfffffffc` for a sign-extended byte),
//!   a shift by one prints `1`, branch targets are absolute hex addresses, a far target is
//!   `0xsel:0xoff`;
//! - x87: `st` for the fixed top-of-stack operand, `st(n)` for the indexed one, the fixed one
//!   being first except in the `dc` and `de` forms; `fxch`, `fcom`, `fcomp`, `fucom` and
//!   `fucomp` name only the indexed register;
//! - a prefix the instruction does not use is a word of its own (`repz`, `repnz`, `lock`, `cs`
//!   to `gs`, `data16`, `addr16`); `rep` on the string forms that do not compare, `bnd` on near
//!   branches, `xacquire` and `xrelease` after `lock` (and on stores and exchanges to memory);
//!   whether a prefix is used is read from the decoder (does the decoding change without it)
//!   and from the presence of a memory operand;
//! - a `w` suffix on the 16-bit operand size forms of push and pop of segment registers and
//!   immediates, `pusha`, `popa`, `pushf`, `popf`, `ret`, `retf`, `leave`, `iret`, `enter`,
//!   near `call` and `jmp` and the long conditional jumps;
//! - `66 90` is `xchg ax,ax`.
//!
//! [`super::decode`] holds what the tool reads differently from the decoder.

use iced_x86::{Decoder, DecoderOptions, Instruction, Mnemonic, OpKind, Register};
use std::fmt::Write;

/// Legacy prefix bytes (32-bit mode has no REX).
pub(crate) fn is_prefix(b: u8) -> bool {
    matches!(
        b,
        0xf0 | 0xf2 | 0xf3 | 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65 | 0x66 | 0x67
    )
}

fn is_seg_prefix(b: u8) -> bool {
    matches!(b, 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65)
}

/// Name objdump gives an unconsumed prefix byte.
pub(crate) fn prefix_name(b: u8) -> Option<&'static str> {
    Some(match b {
        0xf3 => "repz",
        0xf2 => "repnz",
        0xf0 => "lock",
        0x2e => "cs",
        0x36 => "ss",
        0x3e => "ds",
        0x26 => "es",
        0x64 => "fs",
        0x65 => "gs",
        0x66 => "data16",
        0x67 => "addr16",
        0x9b => "fwait",
        _ => return None,
    })
}

/// What the instruction bytes look like outside the decoder's view: the legacy prefix run, the
/// opcode and where the ModRM byte is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Layout {
    /// Index of the first byte after the prefixes.
    pub opcode_pos: usize,
    /// The opcode byte after any `0f`, `0f 38` or `0f 3a` escape.
    pub op: u8,
    /// Escape: 0 one-byte map, 1 `0f`, 2 `0f 38`, 3 `0f 3a`.
    pub map: u8,
    pub modrm_pos: usize,
}

pub(crate) fn layout(bytes: &[u8]) -> Layout {
    let mut i = 0;
    while i < bytes.len() && is_prefix(bytes[i]) {
        i += 1;
    }
    let opcode_pos = i;
    let b = |k: usize| bytes.get(k).copied().unwrap_or(0);
    let (map, op, modrm_pos) = if b(i) == 0x0f {
        match b(i + 1) {
            0x38 => (2, b(i + 2), i + 3),
            0x3a => (3, b(i + 2), i + 3),
            o => (1, o, i + 2),
        }
    } else {
        (0, b(i), i + 1)
    };
    Layout {
        opcode_pos,
        op,
        map,
        modrm_pos,
    }
}

fn reg_name(r: Register) -> String {
    format!("{r:?}").to_ascii_lowercase()
}

/// The mnemonic as objdump spells it (iced names the width variants of a few separately).
fn mnemonic_name(m: Mnemonic) -> String {
    let n = format!("{m:?}").to_ascii_lowercase();
    match m {
        Mnemonic::Pushad => "pusha".into(),
        Mnemonic::Popad => "popa".into(),
        Mnemonic::Pushfd => "pushf".into(),
        Mnemonic::Popfd => "popf".into(),
        Mnemonic::Iretd => "iret".into(),
        Mnemonic::Wait => "fwait".into(),
        Mnemonic::Xlatb => "xlat".into(),
        Mnemonic::Sal => "shl".into(),
        Mnemonic::Sgdt | Mnemonic::Sidt | Mnemonic::Lgdt | Mnemonic::Lidt => n + "d",
        _ => n,
    }
}

/// The size word objdump puts before `PTR` for a memory operand of this width, if any.
fn ptr_word(ms: iced_x86::MemorySize) -> Option<&'static str> {
    use iced_x86::MemorySize as M;
    Some(match ms {
        M::UInt8 | M::Int8 => "BYTE",
        M::UInt16 | M::Int16 | M::WordOffset => "WORD",
        M::UInt32
        | M::Int32
        | M::Float32
        | M::DwordOffset
        | M::SegPtr16
        | M::Bound16_WordWord
        | M::Packed32_UInt8
        | M::Packed32_Int8
        | M::Packed32_UInt16
        | M::Packed32_Int16 => "DWORD",
        M::UInt64
        | M::Int64
        | M::Float64
        | M::QwordOffset
        | M::Bound32_DwordDword
        | M::Packed64_UInt8
        | M::Packed64_Int8
        | M::Packed64_UInt16
        | M::Packed64_Int16
        | M::Packed64_UInt32
        | M::Packed64_Int32
        | M::Packed64_Float32 => "QWORD",
        M::Float80 | M::Bcd => "TBYTE",
        M::Fword6 | M::SegPtr32 | M::Fword10 => "FWORD",
        M::UInt128
        | M::Int128
        | M::Float128
        | M::Packed128_UInt8
        | M::Packed128_Int8
        | M::Packed128_UInt16
        | M::Packed128_Int16
        | M::Packed128_UInt32
        | M::Packed128_Int32
        | M::Packed128_UInt64
        | M::Packed128_Int64
        | M::Packed128_Float32
        | M::Packed128_Float64 => "XMMWORD",
        _ => return None,
    })
}

fn push_hex(s: &mut String, v: u64) {
    let _ = write!(s, "0x{v:x}");
}

fn seg_of_byte(b: u8) -> &'static str {
    match b {
        0x2e => "cs",
        0x36 => "ss",
        0x3e => "ds",
        0x26 => "es",
        0x64 => "fs",
        _ => "gs",
    }
}

fn is_reg16(r: Register) -> bool {
    matches!(r, Register::BX | Register::BP | Register::SI | Register::DI)
}

/// The parts of a memory operand, in the tool's notation.
pub(crate) struct MemParts<'a> {
    pub size: Option<&'a str>,
    pub seg: Option<&'a str>,
    pub base: Option<String>,
    pub index: Option<String>,
    pub scale: u32,
    /// A SIB byte without an index register: spelled `eiz`.
    pub eiz: bool,
    pub addr16: bool,
    /// Signed displacement when the operand has one.
    pub disp: Option<i64>,
    /// The displacement as an unsigned address (used when there is no register at all).
    pub abs: u64,
}

/// `{SIZE PTR }{seg:}[base+index*scale+disp]`, or `{SIZE PTR }seg:0xaddr` without registers.
pub(crate) fn fmt_mem(p: &MemParts) -> String {
    let mut s = String::new();
    if let Some(w) = p.size {
        s.push_str(w);
        s.push_str(" PTR ");
    }
    if p.base.is_none() && p.index.is_none() && !p.eiz {
        // Absolute address: a segment is always named.
        s.push_str(p.seg.unwrap_or("ds"));
        s.push(':');
        push_hex(&mut s, p.abs);
        return s;
    }
    if let Some(g) = p.seg {
        s.push_str(g);
        s.push(':');
    }
    s.push('[');
    let mut any = false;
    if let Some(b) = &p.base {
        s.push_str(b);
        any = true;
    }
    if p.index.is_some() || p.eiz {
        if any {
            s.push('+');
        }
        s.push_str(p.index.as_deref().unwrap_or("eiz"));
        if !p.addr16 {
            let _ = write!(s, "*{}", p.scale);
        }
        any = true;
    }
    if let Some(v) = p.disp {
        if v < 0 {
            s.push('-');
            push_hex(&mut s, v.unsigned_abs());
        } else {
            if any {
                s.push('+');
            }
            push_hex(&mut s, v as u64);
        }
    }
    s.push(']');
    s
}

struct Ctx<'a> {
    ins: &'a Instruction,
    bytes: &'a [u8],
    lay: Layout,
    /// The segment override byte that counts (the last one), if any.
    seg_override: Option<u8>,
}

impl Ctx<'_> {
    /// `moffs` forms (`a0` to `a3`) have no ModRM byte.
    fn has_modrm(&self) -> bool {
        !(self.lay.map == 0 && (0xa0..=0xa3).contains(&self.lay.op))
    }

    fn mem(&self, size: Option<&str>) -> String {
        let ins = self.ins;
        let base = ins.memory_base();
        let index = ins.memory_index();
        let dsz = ins.memory_displ_size();
        let scale = ins.memory_index_scale();
        let eiz = index == Register::None && self.sib().is_some_and(|sb| scale > 1 || sb & 7 != 4);
        let d = ins.memory_displacement64();
        let disp = match dsz {
            0 => None,
            2 => Some(d as u16 as i16 as i64),
            _ => Some(d as u32 as i32 as i64),
        };
        let abs = if dsz == 2 {
            d & 0xffff
        } else {
            d & 0xffff_ffff
        };
        fmt_mem(&MemParts {
            size,
            seg: self.seg_override.map(seg_of_byte),
            base: (base != Register::None).then(|| reg_name(base)),
            index: (index != Register::None).then(|| reg_name(index)),
            scale,
            eiz,
            addr16: is_reg16(base) || matches!(index, Register::SI | Register::DI),
            disp,
            abs,
        })
    }

    /// The SIB byte, when the instruction has a 32-bit ModRM memory operand that uses one.
    fn sib(&self) -> Option<u8> {
        let ins = self.ins;
        if !self.has_modrm() || ins.op_kinds().all(|k| k != OpKind::Memory) {
            return None;
        }
        let p = self.lay.modrm_pos;
        let m = *self.bytes.get(p)?;
        // 16-bit addressing has no SIB: its registers are 16 bits wide.
        if is_reg16(ins.memory_base()) || matches!(ins.memory_index(), Register::SI | Register::DI)
        {
            return None;
        }
        if m >> 6 != 3 && m & 7 == 4 {
            self.bytes.get(p + 1).copied()
        } else {
            None
        }
    }
}

fn imm_text(k: OpKind, ins: &Instruction, i: u32) -> String {
    let mut s = String::new();
    let v: u64 = match k {
        OpKind::Immediate8 | OpKind::Immediate8_2nd => ins.immediate(i) & 0xff,
        OpKind::Immediate16 | OpKind::Immediate8to16 => ins.immediate(i) & 0xffff,
        _ => ins.immediate(i) & 0xffff_ffff,
    };
    push_hex(&mut s, v);
    s
}

/// An instruction cut off by the end of the section: the tool prints the first byte as a prefix
/// name or as a data byte and moves on by one.
pub(crate) fn truncated(bytes: &[u8]) -> String {
    match bytes.first().copied().and_then(prefix_name) {
        Some(n) => n.to_string(),
        // The tool prints the byte with `%#x`, which leaves the prefix off a zero.
        None => match bytes.first().copied().unwrap_or(0) {
            0 => ".byte 0".to_string(),
            b => format!(".byte 0x{b:x}"),
        },
    }
}

/// Does the decoder read the instruction differently without the given prefix bytes (so the
/// instruction uses them)?
fn uses_prefix(ins: &Instruction, bytes: &[u8], drop: &[usize]) -> bool {
    let rest: Vec<u8> = bytes
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop.contains(i))
        .map(|(_, &b)| b)
        .collect();
    let mut d = Decoder::new(32, &rest, DecoderOptions::NONE);
    let o = d.decode();
    o.is_invalid() || o.code() != ins.code() || o.len() != bytes.len() - drop.len()
}

/// How the `fwait` byte (9b) in front of an instruction reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wait {
    /// No `fwait` before it, or one that vanishes into the x87 instruction after it.
    None,
    /// An `fwait` byte merged into the "no wait" form: `fnstcw` reads `fstcw`.
    Merged,
}

/// The mnemonics the `fwait` byte merges into (their "no wait" spelling starts with `fn`).
pub(crate) fn merges_wait(m: Mnemonic) -> bool {
    matches!(
        m,
        Mnemonic::Fnstcw
            | Mnemonic::Fnstsw
            | Mnemonic::Fnclex
            | Mnemonic::Fninit
            | Mnemonic::Fnsave
            | Mnemonic::Fnstenv
    )
}

/// How an instruction was read from its bytes, beyond what the decoder reports.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Quirks {
    /// A `mov` to or from a segment register whose reg field the decoder refuses: the name the
    /// tool prints for it (`?` for the two that are no register).
    pub sreg: Option<&'static str>,
    /// A `mov` to or from a control or debug register whose number or ModRM mode the decoder
    /// refuses: the register number the tool prints.
    pub special_reg: Option<u8>,
}

/// Text of the instruction as objdump prints it after the tab, trimmed.
///
/// `eff` are the bytes the decoder read (the instruction without its `lock` prefixes, which the
/// decoder refuses on most instructions) and `orig_prefix` the prefix run as it stands in the
/// file, in order.
pub(crate) fn print(
    ins: &Instruction,
    eff: &[u8],
    orig_prefix: &[u8],
    wait: Wait,
    quirks: Quirks,
) -> String {
    let lay = layout(eff);
    let eff_prefix = &eff[..lay.opcode_pos];
    let m = ins.mnemonic();
    let last_seg = eff_prefix.iter().rposition(|&b| is_seg_prefix(b));
    let seg_override = last_seg.map(|i| eff_prefix[i]);
    let cx = Ctx {
        ins,
        bytes: eff,
        lay,
        seg_override,
    };
    let mut name = mnemonic_name(m);
    if wait == Wait::Merged {
        name.remove(1);
    }
    let mut ops: Vec<String> = Vec::new();
    let nops = ins.op_count();
    let mut uses_mem_seg = false;
    let mut any_mem = false;
    let kinds: Vec<OpKind> = ins.op_kinds().collect();
    let has66 = eff_prefix.contains(&0x66);
    let rel8_with_66 =
        has66 && lay.map == 0 && (matches!(lay.op, 0x70..=0x7f | 0xe0..=0xe3) || lay.op == 0xeb);
    for i in 0..nops {
        let k = kinds[i as usize];
        let t = match k {
            OpKind::Register => {
                let r = ins.op_register(i);
                if (Register::ST0..=Register::ST7).contains(&r) {
                    let n = r as u32 - Register::ST0 as u32;
                    let fixed_first = !matches!(lay.op, 0xdc | 0xde);
                    let is_fixed = nops == 2 && ((i == 0) == fixed_first);
                    if is_fixed {
                        "st".to_string()
                    } else {
                        format!("st({n})")
                    }
                } else if let (Some(n), true) = (
                    quirks.special_reg,
                    (Register::CR0..=Register::CR15).contains(&r)
                        || (Register::DR0..=Register::DR15).contains(&r),
                ) {
                    let prefix = if (Register::CR0..=Register::CR15).contains(&r) {
                        "cr"
                    } else {
                        "dr"
                    };
                    format!("{prefix}{n}")
                } else if r == Register::ES && quirks.sreg.is_some() {
                    quirks.sreg.unwrap_or("?").to_string()
                } else {
                    reg_name(r)
                }
            }
            OpKind::NearBranch16 if rel8_with_66 => {
                let rel = eff.last().copied().unwrap_or(0) as i8 as i32;
                format!(
                    "0x{:x}",
                    (ins.ip() as u32)
                        .wrapping_add(eff.len() as u32)
                        .wrapping_add(rel as u32)
                )
            }
            OpKind::NearBranch16 => format!("0x{:x}", ins.near_branch16()),
            OpKind::NearBranch32 => format!("0x{:x}", ins.near_branch32()),
            OpKind::FarBranch16 => {
                format!(
                    "0x{:x}:0x{:x}",
                    ins.far_branch_selector(),
                    ins.far_branch16()
                )
            }
            OpKind::FarBranch32 => {
                format!(
                    "0x{:x}:0x{:x}",
                    ins.far_branch_selector(),
                    ins.far_branch32()
                )
            }
            OpKind::Immediate8
            | OpKind::Immediate8_2nd
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32 => {
                if matches!(lay.op, 0xd0 | 0xd1) && lay.map == 0 && i == 1 {
                    "1".to_string()
                } else {
                    imm_text(k, ins, i)
                }
            }
            OpKind::Memory => {
                uses_mem_seg = true;
                any_mem = true;
                if m == Mnemonic::Xlatb {
                    let seg = cx.seg_override.map(seg_of_byte).unwrap_or("ds");
                    ops.push(format!("BYTE PTR {seg}:[{}]", reg_name(ins.memory_base())));
                    continue;
                }
                let size = if matches!(
                    m,
                    Mnemonic::Lea
                        | Mnemonic::Sgdt
                        | Mnemonic::Sidt
                        | Mnemonic::Lgdt
                        | Mnemonic::Lidt
                ) {
                    None
                } else if m == Mnemonic::Invlpg {
                    Some("BYTE")
                } else {
                    ptr_word(ins.memory_size())
                };
                cx.mem(if cx.has_modrm() { size } else { None })
            }
            OpKind::MemorySegESI | OpKind::MemorySegSI => {
                uses_mem_seg = true;
                any_mem = true;
                let seg = cx.seg_override.map(seg_of_byte).unwrap_or("ds");
                let w = ptr_word(ins.memory_size()).unwrap_or("BYTE");
                let r = if k == OpKind::MemorySegESI {
                    "esi"
                } else {
                    "si"
                };
                format!("{w} PTR {seg}:[{r}]")
            }
            OpKind::MemoryESEDI | OpKind::MemoryESDI => {
                any_mem = true;
                let w = ptr_word(ins.memory_size()).unwrap_or("BYTE");
                let r = if k == OpKind::MemoryESEDI {
                    "edi"
                } else {
                    "di"
                };
                format!("{w} PTR es:[{r}]")
            }
            _ => "?".to_string(),
        };
        ops.push(t);
    }
    // x87 forms that name one stack register in the tool's text but two in the decoder's.
    if ops.len() == 2
        && matches!(
            m,
            Mnemonic::Fxch | Mnemonic::Fcom | Mnemonic::Fcomp | Mnemonic::Fucom | Mnemonic::Fucomp
        )
    {
        ops.remove(0);
    }
    // String instructions carry the plain mnemonic.
    let string_op = kinds.iter().any(|k| {
        matches!(
            k,
            OpKind::MemoryESEDI | OpKind::MemorySegESI | OpKind::MemoryESDI | OpKind::MemorySegSI
        )
    });
    if string_op {
        for suf in ["b", "w", "d"] {
            if let Some(stem) = name.strip_suffix(suf).filter(|s| {
                matches!(
                    *s,
                    "movs" | "cmps" | "scas" | "lods" | "stos" | "ins" | "outs"
                )
            }) {
                name = stem.to_string();
                break;
            }
        }
    }
    // The reserved `nop` encodings read as a plain `nop` with one operand.
    if m == Mnemonic::Reservednop {
        name = "nop".into();
        ops.truncate(1);
    }
    // `66 90` is `xchg ax,ax` to the tool, not a `nop`.
    if m == Mnemonic::Nop && lay.map == 0 && lay.op == 0x90 && has66 {
        name = "xchg".into();
        ops = vec!["ax".into(), "ax".into()];
    }
    // Prefix words: those the instruction does not use are named.
    let mut out = String::new();
    let mut plen = 0;
    let mut word = |out: &mut String, n: &str| {
        out.push_str(n);
        out.push(' ');
        plen += n.len() + 1;
    };
    let last = |b: u8| eff_prefix.iter().rposition(|&x| x == b);
    let used_group = |b: u8| -> bool {
        let drop: Vec<usize> = eff_prefix
            .iter()
            .enumerate()
            .filter(|(_, x)| **x == b)
            .map(|(i, _)| i)
            .collect();
        uses_prefix(ins, eff, &drop)
    };
    let (last66, last67, lastf3, lastf2) = (last(0x66), last(0x67), last(0xf3), last(0xf2));
    let used66 = !rel8_with_66 && last66.is_some() && used_group(0x66);
    // Operand-size forms of a few instructions are spelled with a `w` suffix.
    let w_form = match lay.map {
        0 => matches!(
            lay.op,
            0x06 | 0x0e
                | 0x16
                | 0x1e
                | 0x07
                | 0x17
                | 0x1f
                | 0xc3
                | 0xc2
                | 0xcb
                | 0xca
                | 0xc9
                | 0xcf
                | 0xc8
                | 0xe8
                | 0xe9
                | 0x60
                | 0x61
                | 0x9c
                | 0x9d
                | 0x68
                | 0x6a
        ),
        1 => matches!(lay.op, 0xa0 | 0xa1 | 0xa8 | 0xa9 | 0x80..=0x8f),
        _ => false,
    };
    if used66 && w_form {
        name.push('w');
    }
    // `bnd`: the prefix on near branches.
    let modrm_reg = eff.get(lay.modrm_pos).map(|m| (m >> 3) & 7);
    let bnd = match lay.map {
        0 => {
            matches!(lay.op, 0xe8 | 0xe9 | 0xc2 | 0xc3 | 0xeb | 0x70..=0x7f)
                || (lay.op == 0xff && matches!(modrm_reg, Some(2 | 4)))
        }
        1 => matches!(lay.op, 0x80..=0x8f),
        _ => false,
    };
    let used67 = last67.is_some()
        && !(lay.map == 0 && matches!(lay.op, 0xa0..=0xa3 | 0xe0..=0xe2))
        && (any_mem || used_group(0x67));
    let usedf3 = lastf3.is_some() && used_group(0xf3);
    let usedf2 = lastf2.is_some() && used_group(0xf2);
    let has_lock = orig_prefix.contains(&0xf0);
    let mem_first = kinds.first() == Some(&OpKind::Memory);
    let mov_store = lay.map == 0 && matches!(lay.op, 0x88 | 0x89 | 0xc6 | 0xc7) && mem_first;
    let xchg_mem = lay.map == 0 && matches!(lay.op, 0x86 | 0x87) && mem_first;
    // Counts of each prefix byte seen so far in the original run, to tell the last from the
    // earlier ones.
    let orig_last = |b: u8| orig_prefix.iter().rposition(|&x| x == b);
    let orig_last_seg = orig_prefix.iter().rposition(|&b| is_seg_prefix(b));
    for (i, &p) in orig_prefix.iter().enumerate() {
        let n = match p {
            0x66 if Some(i) == orig_last(0x66) && used66 => continue,
            0x67 if Some(i) == orig_last(0x67) && used67 => continue,
            0xf3 if Some(i) == orig_last(0xf3) && usedf3 => continue,
            0xf2 if Some(i) == orig_last(0xf2) && usedf2 => continue,
            0xf2 if Some(i) == orig_last(0xf2) && bnd => "bnd",
            0xf3 if Some(i) == orig_last(0xf3)
                && (has_lock && mem_first || mov_store || xchg_mem) =>
            {
                "xrelease"
            }
            0xf2 if Some(i) == orig_last(0xf2) && (has_lock && mem_first || xchg_mem) => "xacquire",
            0xf3 if Some(i) == orig_last(0xf3)
                && string_op
                && matches!(name.as_str(), "movs" | "stos" | "lods" | "ins" | "outs") =>
            {
                "rep"
            }
            b if is_seg_prefix(b) && Some(i) == orig_last_seg && uses_mem_seg => continue,
            b => prefix_name(b).unwrap_or(""),
        };
        word(&mut out, n);
    }
    out.push_str(&name);
    let l = plen + name.len();
    let pad = if l < 7 { 7 - l } else { 1 };
    if !ops.is_empty() {
        for _ in 0..pad {
            out.push(' ');
        }
        out.push_str(&ops.join(","));
    }
    out
}
