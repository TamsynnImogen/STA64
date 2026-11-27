//! Reading one instruction the way the reference tool does: the decoder reads the bytes, and
//! the cases where the tool's reading differs from the decoder's (instructions it refuses, `lock`
//! on anything, the `fwait` byte, `(bad)` and how many bytes it takes) are settled here.

use iced_x86::{Decoder, DecoderError, DecoderOptions, Instruction};

use super::print::{self, Layout, MemParts, Quirks, Wait, fmt_mem, is_prefix, layout, prefix_name};

/// A decoder over one section and the position it reads at.
pub(crate) struct Sweeper<'a> {
    data: &'a [u8],
    base: u32,
    dec: Decoder<'a>,
}

/// The x87 register forms (second byte `c0` to `ff`) the tool prints as `(bad)`.
fn x87_reg_bad(op: u8, m: u8) -> bool {
    match op {
        0xd9 => matches!(m, 0xd1..=0xdf | 0xe2 | 0xe3 | 0xe6 | 0xe7 | 0xef),
        0xda => matches!(m, 0xe0..=0xe8 | 0xea..=0xff),
        0xdb => matches!(m, 0xe6 | 0xe7 | 0xf8..=0xff),
        0xdc => matches!(m, 0xd0..=0xdf),
        0xdd => matches!(m, 0xc8..=0xcf | 0xf0..=0xff),
        0xde => matches!(m, 0xd0..=0xd8 | 0xda..=0xdf),
        0xdf => matches!(m, 0xc8..=0xdf | 0xe1..=0xe7 | 0xf8..=0xff),
        _ => false,
    }
}

/// The x87 memory forms (reg field of the second byte) the tool prints as `(bad)` followed by the
/// memory operand.
fn x87_mem_bad(op: u8, reg: u8) -> bool {
    matches!((op, reg), (0xd9, 1) | (0xdb, 4) | (0xdb, 6) | (0xdd, 5))
}

const REG32: [&str; 8] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"];
const REG16: [&str; 8] = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di"];
const SREG: [&str; 8] = ["es", "cs", "ss", "ds", "fs", "gs", "?", "?"];

/// The memory operand of a ModRM byte at `p` in `b` (32-bit addressing, or 16-bit with `a16`),
/// as the tool writes it, and the bytes the ModRM, SIB and displacement take.
fn modrm_mem(b: &[u8], p: usize, a16: bool, seg: Option<&str>) -> Option<(String, usize)> {
    let m = *b.get(p)?;
    let (md, rm) = (m >> 6, m & 7);
    let rd = |o: usize, n: usize| -> Option<i64> {
        let s = b.get(o..o + n)?;
        Some(match n {
            1 => s[0] as i8 as i64,
            2 => i16::from_le_bytes([s[0], s[1]]) as i64,
            _ => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as i64,
        })
    };
    if a16 {
        let (base, index): (Option<&str>, Option<&str>) = match rm {
            0 => (Some("bx"), Some("si")),
            1 => (Some("bx"), Some("di")),
            2 => (Some("bp"), Some("si")),
            3 => (Some("bp"), Some("di")),
            4 => (Some("si"), None),
            5 => (Some("di"), None),
            6 if md == 0 => (None, None),
            6 => (Some("bp"), None),
            _ => (Some("bx"), None),
        };
        let (dn, disp) = match (md, rm) {
            (0, 6) | (2, _) => (2, rd(p + 1, 2)?),
            (1, _) => (1, rd(p + 1, 1)?),
            _ => (0, 0),
        };
        let parts = MemParts {
            size: None,
            seg,
            base: base.map(str::to_string),
            index: index.map(str::to_string),
            scale: 1,
            eiz: false,
            addr16: true,
            disp: (dn != 0 && (base.is_some() || index.is_some())).then_some(disp),
            abs: (disp as u64) & 0xffff,
        };
        return Some((fmt_mem(&parts), 1 + dn));
    }
    let mut len = 1;
    if rm == 4 {
        let (mut base, mut index, mut eiz) = (None, None, false);
        let sib = *b.get(p + 1)?;
        len += 1;
        let (sc, ix, bs) = (sib >> 6, (sib >> 3) & 7, sib & 7);
        let scale = 1u32 << sc;
        if ix != 4 {
            index = Some(REG32[ix as usize].to_string());
        } else if sc != 0 || bs != 4 {
            eiz = true;
        }
        if !(bs == 5 && md == 0) {
            base = Some(REG32[bs as usize].to_string());
        }
        let nodisp_base = bs == 5 && md == 0;
        let (dn, disp) = match md {
            0 if nodisp_base => (4, rd(p + len, 4)?),
            1 => (1, rd(p + len, 1)?),
            2 => (4, rd(p + len, 4)?),
            _ => (0, 0),
        };
        let parts = MemParts {
            size: None,
            seg,
            base,
            index,
            scale,
            eiz,
            addr16: false,
            disp: (dn != 0).then_some(disp),
            abs: (disp as u64) & 0xffff_ffff,
        };
        return Some((fmt_mem(&parts), len + dn));
    }
    let (dn, disp) = match (md, rm) {
        (0, 5) => (4, rd(p + 1, 4)?),
        (1, _) => (1, rd(p + 1, 1)?),
        (2, _) => (4, rd(p + 1, 4)?),
        _ => (0, 0),
    };
    let base = (!(md == 0 && rm == 5)).then(|| REG32[rm as usize].to_string());
    let parts = MemParts {
        size: None,
        seg,
        base,
        index: None,
        scale: 1,
        eiz: false,
        addr16: false,
        disp: (dn != 0 && md != 0).then_some(disp),
        abs: (disp as u64) & 0xffff_ffff,
    };
    Some((fmt_mem(&parts), len + dn))
}

/// Joins prefix words, the mnemonic and the operand text the way the tool does.
fn join(words: &[&str], name: &str, ops: &str) -> String {
    let mut s = String::new();
    let mut l = name.len();
    for w in words {
        s.push_str(w);
        s.push(' ');
        l += w.len() + 1;
    }
    s.push_str(name);
    if !ops.is_empty() {
        let pad = if name == "(bad)" || l >= 7 { 1 } else { 7 - l };
        for _ in 0..pad {
            s.push(' ');
        }
        s.push_str(ops);
    }
    s
}

impl<'a> Sweeper<'a> {
    pub(crate) fn new(base: u32, data: &'a [u8]) -> Self {
        Self {
            data,
            base,
            dec: Decoder::with_ip(32, data, base as u64, DecoderOptions::NONE),
        }
    }

    /// The instruction at `pos`: its length in bytes and its text.
    pub(crate) fn decode(&mut self, pos: usize) -> (usize, String) {
        let data = self.data;
        // `fwait` bytes in front of an x87 instruction belong to it.
        if data[pos] == 0x9b {
            let mut k = 1;
            while pos + k < data.len() && data[pos + k] == 0x9b {
                k += 1;
            }
            if pos + k < data.len() && (0xd8..=0xdf).contains(&data[pos + k]) {
                // Of several, the last two join the x87 instruction and the others stand alone.
                if k >= 3 {
                    return (1, "fwait".to_string());
                }
                let (n, text) = self.core(pos + k, true);
                return (k + n, text);
            }
        }
        self.core(pos, false)
    }

    fn core(&mut self, pos: usize, wait_before: bool) -> (usize, String) {
        let data = self.data;
        let addr = self.base.wrapping_add(pos as u32);
        let rest = &data[pos..data.len().min(pos + 32)];
        let nprefix = rest.iter().take_while(|&&b| is_prefix(b)).count();
        let orig_prefix = &rest[..nprefix];
        let n_lock = orig_prefix.iter().filter(|&&b| b == 0xf0).count();
        let eff: Vec<u8>;
        let eff_ref: &[u8] = if n_lock > 0 {
            eff = rest
                .iter()
                .enumerate()
                .filter(|&(i, &b)| !(i < nprefix && b == 0xf0))
                .map(|(_, &b)| b)
                .collect();
            &eff
        } else {
            rest
        };
        let lay = layout(eff_ref);
        if let Some((n, text)) =
            self.special(eff_ref, orig_prefix, &lay, addr.wrapping_add(n_lock as u32))
        {
            return (n + n_lock, text);
        }
        let mut quirks = Quirks::default();
        let mut ins = Instruction::default();
        let mut patched: Option<Vec<u8>> = None;
        let mut dec2;
        let (valid, err, used): (bool, DecoderError, &[u8]) = if n_lock == 0 {
            let _ = self.dec.set_position(pos);
            self.dec.set_ip(addr as u64);
            self.dec.decode_out(&mut ins);
            let ok = !ins.is_invalid();
            let l = if ok { ins.len() } else { 0 };
            (ok, self.dec.last_error(), &data[pos..pos + l])
        } else {
            dec2 = Decoder::with_ip(
                32,
                eff_ref,
                addr as u64 + n_lock as u64,
                DecoderOptions::NONE,
            );
            dec2.decode_out(&mut ins);
            let ok = !ins.is_invalid();
            let l = if ok { ins.len() } else { 0 };
            (ok, dec2.last_error(), &eff_ref[..l])
        };
        let used0: &[u8] = used;
        let mut valid = valid;
        if !valid && err != DecoderError::NoMoreBytes {
            // Moves to and from a control or debug register take any ModRM byte as a register.
            let mp = lay.modrm_pos;
            if lay.map == 1 && matches!(lay.op, 0x20..=0x23) && mp < eff_ref.len() {
                let mut b = eff_ref.to_vec();
                let reg = (b[mp] >> 3) & 7;
                b[mp] = (b[mp] | 0xc0) & !0x38;
                let mut d3 =
                    Decoder::with_ip(32, &b, addr as u64 + n_lock as u64, DecoderOptions::NONE);
                d3.decode_out(&mut ins);
                if !ins.is_invalid() {
                    quirks.special_reg = Some(reg);
                    valid = true;
                    patched = Some(b);
                }
            }
        }
        if !valid && err != DecoderError::NoMoreBytes {
            // Moves to and from a segment register: the tool accepts every reg field.
            let mp = lay.modrm_pos;
            if lay.map == 0 && matches!(lay.op, 0x8c | 0x8e) && mp < eff_ref.len() {
                let mut b = eff_ref.to_vec();
                let reg = (b[mp] >> 3) & 7;
                b[mp] &= !0x38;
                let mut d3 =
                    Decoder::with_ip(32, &b, addr as u64 + n_lock as u64, DecoderOptions::NONE);
                d3.decode_out(&mut ins);
                if !ins.is_invalid() {
                    quirks.sreg = Some(SREG[reg as usize]);
                    valid = true;
                    patched = Some(b);
                }
            }
        }
        if !valid {
            if err == DecoderError::NoMoreBytes {
                return (1, print::truncated(rest));
            }
            // The tool takes the prefixes and the opcode bytes with it; a few encodings that are
            // not instructions to it take more.
            let opn = match lay.map {
                0 => 1,
                1 => 2,
                _ => 3,
            };
            let m = eff_ref.get(lay.modrm_pos).copied().unwrap_or(0);
            let extra = match (lay.map, lay.op) {
                // XOP and VEX prefixes: with a valid map the tool takes the payload bytes and the
                // opcode along.
                (0, 0x8f) if matches!(m & 0x1f, 8..=10) => 3,
                (0, 0xc4) if m >= 0xc0 && matches!(m & 0x1f, 1..=3) => 3,
                (0, 0xc5) if m >= 0xc0 => 2,
                _ => 0,
            };
            // A prefix the tool took as part of a VEX or XOP escape is not named.
            let words: Vec<&str> = if extra > 0 {
                Vec::new()
            } else {
                orig_prefix
                    .iter()
                    .map(|&b| prefix_name(b).unwrap_or(""))
                    .collect()
            };
            return (
                (nprefix + opn + extra).min(rest.len()),
                join(&words, "(bad)", ""),
            );
        }
        let wait = if wait_before && print::merges_wait(ins.mnemonic()) {
            Wait::Merged
        } else {
            Wait::None
        };
        let used: &[u8] = match &patched {
            Some(b) => &b[..ins.len()],
            None => used0,
        };
        let text = print::print(&ins, used, orig_prefix, wait, quirks);
        (used.len() + n_lock, text)
    }

    /// Forms the decoder refuses or reads differently from the tool: `(bad)` x87 encodings and
    /// the bad operands of `lea` and the far `call` and `jmp`.
    fn special(
        &self,
        eff: &[u8],
        orig_prefix: &[u8],
        lay: &Layout,
        addr: u32,
    ) -> Option<(usize, String)> {
        let mp = lay.modrm_pos;
        let m = *eff.get(mp)?;
        let all_words: Vec<&str> = orig_prefix
            .iter()
            .map(|&b| prefix_name(b).unwrap_or(""))
            .collect();
        // The padlock group: only the register forms with `rm` 0 are instructions.
        if lay.map == 1 && lay.op == 0xa7 {
            const NAMES: [&str; 8] = [
                "xstore-rng",
                "xcrypt-ecb",
                "xcrypt-cbc",
                "xcrypt-ctr",
                "xcrypt-cfb",
                "xcrypt-ofb",
                "(bad)",
                "(bad)",
            ];
            let name = NAMES[((m >> 3) & 7) as usize];
            return Some(if m >= 0xc0 && m & 7 == 0 && name != "(bad)" {
                (mp + 1, join(&all_words, name, ""))
            } else {
                (lay.opcode_pos + 1, join(&all_words, name, "(bad)"))
            });
        }
        // `xbegin` and `xabort` read their operand from the ModRM byte's place.
        if lay.map == 0
            && matches!(lay.op, 0xc6 | 0xc7)
            && (m >> 3) & 7 == 7
            && m & 7 == 0
            && m < 0xc0
        {
            if lay.op == 0xc7 {
                let r = eff.get(mp..mp + 4)?;
                let rel = i32::from_le_bytes([r[0], r[1], r[2], r[3]]);
                let n = mp + 4;
                let target = addr.wrapping_add(n as u32).wrapping_add(rel as u32);
                return Some((
                    n,
                    join(&all_words, "xbegin", &format!("(bad),0x{target:x}")),
                ));
            }
            return Some((
                mp + 1,
                join(&all_words, "xabort", &format!("(bad),0x{m:x}")),
            ));
        }
        if lay.map != 0 {
            return None;
        }
        let seg = orig_prefix
            .iter()
            .rev()
            .find(|&&b| matches!(b, 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65))
            .map(|&b| print_seg(b));
        let words = |skip_seg: bool, skip66: bool| -> Vec<&'static str> {
            let mut v = Vec::new();
            let last_seg = orig_prefix
                .iter()
                .rposition(|&b| matches!(b, 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65));
            let last66 = orig_prefix.iter().rposition(|&b| b == 0x66);
            for (i, &b) in orig_prefix.iter().enumerate() {
                if skip_seg && Some(i) == last_seg {
                    continue;
                }
                if skip66 && Some(i) == last66 {
                    continue;
                }
                v.push(prefix_name(b).unwrap_or(""));
            }
            v
        };
        let n_pre = lay.opcode_pos;
        if (0xd8..=0xdf).contains(&lay.op) {
            if m >= 0xc0 {
                if x87_reg_bad(lay.op, m) {
                    return Some((mp + 1, join(&words(false, false), "(bad)", "")));
                }
            } else if x87_mem_bad(lay.op, (m >> 3) & 7) {
                let (mem, n) = modrm_mem(eff, mp, orig_prefix.contains(&0x67), seg)?;
                return Some((mp + n, join(&words(seg.is_some(), false), "(bad)", &mem)));
            }
            return None;
        }
        if lay.op == 0x8d && m >= 0xc0 {
            let names = if orig_prefix.contains(&0x66) {
                &REG16
            } else {
                &REG32
            };
            let ops = format!("{},(bad)", names[((m >> 3) & 7) as usize]);
            return Some((n_pre + 1, join(&words(false, true), "lea", &ops)));
        }
        if lay.op == 0xff && m >= 0xc0 && matches!((m >> 3) & 7, 3 | 5) {
            let name = if (m >> 3) & 7 == 3 { "call" } else { "jmp" };
            return Some((n_pre + 1, join(&words(false, false), name, "(bad)")));
        }
        None
    }
}

fn print_seg(b: u8) -> &'static str {
    match b {
        0x2e => "cs",
        0x36 => "ss",
        0x3e => "ds",
        0x26 => "es",
        0x64 => "fs",
        _ => "gs",
    }
}
