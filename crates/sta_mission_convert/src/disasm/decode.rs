//! Reading one instruction the way the reference tool does: the decoder reads the bytes, and
//! the cases where the tool's reading differs from the decoder's (instructions it refuses, `lock`
//! on anything, the `fwait` byte, `(bad)` and how many bytes it takes) are settled here.

use iced_x86::{Decoder, DecoderError, DecoderOptions, Instruction};

use super::badtab;
use super::vextab;
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
        // `fwait` bytes after the prefixes and in front of an x87 instruction (which may have
        // prefixes of its own) belong to it.
        let mut i = pos;
        while i < data.len() && is_prefix(data[i]) {
            i += 1;
        }
        if i < data.len() && data[i] == 0x9b {
            let np = i - pos;
            let mut k = 0;
            while i + k < data.len() && data[i + k] == 0x9b {
                k += 1;
            }
            let mut j = i + k;
            while j < data.len() && is_prefix(data[j]) {
                j += 1;
            }
            let q = j - (i + k);
            let words_of = |s: &[u8]| -> Vec<&'static str> {
                s.iter().map(|&b| prefix_name(b).unwrap_or("")).collect()
            };
            let x87 = j < data.len() && (0xd8..=0xdf).contains(&data[j]);
            if np == 0 {
                if x87 {
                    // Of several, the last two join the x87 instruction and the others stand
                    // alone.
                    if k >= 3 {
                        return (1, "fwait".to_string());
                    }
                    let mut synth: Vec<u8> = data[pos..i].to_vec();
                    let tail_end = data.len().min(i + k + 32);
                    synth.extend_from_slice(&data[i + k..tail_end]);
                    let (n, text, cut) = self.core(pos, Some(&synth), true);
                    if cut {
                        // The x87 instruction is cut off by the end of the data: the `fwait`
                        // stands alone.
                        return (1, "fwait".to_string());
                    }
                    return (n + k, text);
                }
                // `fwait`, prefixes, `fwait`: the prefixes and the first byte are one line
                // that ends in the word `fwait`.
                if k == 1 && q > 0 && j < data.len() && data[j] == 0x9b {
                    return (1 + q, join(&words_of(&data[i + 1..j]), "fwait", ""));
                }
            } else {
                if x87 && q == 0 {
                    let mut synth: Vec<u8> = data[pos..i].to_vec();
                    let tail_end = data.len().min(i + k + 32);
                    synth.extend_from_slice(&data[i + k..tail_end]);
                    let (n, text, cut) = self.core(pos, Some(&synth), true);
                    if !cut {
                        return (n + k, text);
                    }
                }
                // Prefixes in front of the `fwait`, and not one x87 instruction behind it
                // without prefixes of its own: they stay in front of the `fwait`.
                return (np + 1, join(&words_of(&data[pos..i]), "fwait", ""));
            }
        }
        let (n, text, _) = self.core(pos, None, false);
        (n, text)
    }

    /// Reads the instruction at `pos` (or the stand-in bytes `synth`, which are `pos`'s with some
    /// bytes taken out): its length (in `synth` when given), its text, and whether the end of the
    /// data cut it off.
    fn core(
        &mut self,
        pos: usize,
        synth: Option<&[u8]>,
        wait_before: bool,
    ) -> (usize, String, bool) {
        let data = self.data;
        let addr = self.base.wrapping_add(pos as u32);
        let rest = match synth {
            Some(s) => s,
            None => &data[pos..data.len().min(pos + 32)],
        };
        let nprefix = rest.iter().take_while(|&&b| is_prefix(b)).count();
        let orig_prefix = &rest[..nprefix];
        // The tool takes at most fourteen prefix bytes into one line.
        if nprefix >= 14 {
            let words: Vec<&str> = orig_prefix[..14]
                .iter()
                .map(|&b| prefix_name(b).unwrap_or(""))
                .collect();
            return (14, words.join(" "), false);
        }
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
        if let Some((n, text)) = self.special(
            eff_ref,
            orig_prefix,
            &lay,
            addr.wrapping_add(n_lock as u32),
            wait_before,
        ) {
            return (n + n_lock, text, false);
        }
        // Encodings the tool reads as `(bad)` (read from its output on authored bytes).
        if let Some(&m) = eff_ref.get(lay.modrm_pos) {
            let eff_prefix = &eff_ref[..lay.opcode_pos];
            let last = |b: u8| eff_prefix.iter().rposition(|&x| x == b);
            let class = match (last(0xf3), last(0xf2)) {
                (Some(a), Some(b)) if a > b => 3,
                (Some(_), None) => 3,
                (_, Some(_)) => 2,
                _ if eff_prefix.contains(&0x66) => 1,
                _ => 0,
            };
            let opn = match lay.map {
                0 => 1,
                1 => 2,
                _ => 3,
            };
            if let Some(k) = badtab::bad_opcode_bytes(class, lay.map, lay.op, m, opn) {
                let words: Vec<&str> = orig_prefix
                    .iter()
                    .map(|&b| prefix_name(b).unwrap_or(""))
                    .collect();
                return ((nprefix + k).min(rest.len()), join(&words, "(bad)", ""), false);
            }
        }
        let mut quirks = Quirks::default();
        let mut ins = Instruction::default();
        let mut patched: Option<Vec<u8>> = None;
        let mut dec2;
        let (valid, err, used): (bool, DecoderError, &[u8]) = if n_lock == 0 && synth.is_none() {
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
        // The decoder refuses some instructions with an operand-size or repeat prefix that the
        // tool reads with the prefix unused (it prints the prefix as a word of its own).
        let mut removed = 0usize;
        if !valid && err != DecoderError::NoMoreBytes {
            let eff_prefix = &eff_ref[..lay.opcode_pos];
            for kind in [0x66u8, 0xf3, 0xf2] {
                if !eff_prefix.contains(&kind) {
                    continue;
                }
                let b: Vec<u8> = eff_ref
                    .iter()
                    .enumerate()
                    .filter(|&(i, &x)| !(i < lay.opcode_pos && x == kind))
                    .map(|(_, &x)| x)
                    .collect();
                let mut d3 =
                    Decoder::with_ip(32, &b, addr as u64 + n_lock as u64, DecoderOptions::NONE);
                d3.decode_out(&mut ins);
                if !ins.is_invalid() {
                    removed = eff_ref.len() - b.len();
                    valid = true;
                    patched = Some(b);
                    break;
                }
            }
        }
        if !valid {
            if err == DecoderError::NoMoreBytes {
                return (1, print::truncated(rest), true);
            }
            // An instruction of more than fifteen bytes: the tool prints `(bad)` for fifteen of
            // them, naming the prefixes except the segment one its memory operand took.
            for d in 1..=nprefix {
                let mut d4 = Decoder::with_ip(32, &eff_ref[d..], 0, DecoderOptions::NONE);
                let o = d4.decode();
                if !o.is_invalid() && o.len() + d + n_lock > 15 {
                    let mem = o.op_kinds().any(|k| k == iced_x86::OpKind::Memory);
                    let last_seg = orig_prefix
                        .iter()
                        .rposition(|&b| matches!(b, 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65));
                    let words: Vec<&str> = orig_prefix
                        .iter()
                        .enumerate()
                        .filter(|&(i, _)| !(mem && Some(i) == last_seg))
                        .map(|(_, &b)| prefix_name(b).unwrap_or(""))
                        .collect();
                    return (15.min(rest.len()), join(&words, "(bad)", ""), false);
                }
            }
            // The tool takes the prefixes and the opcode bytes with it; a few encodings that are
            // not instructions to it take more.
            let opn = match (lay.map, lay.op) {
                (0, _) => 1,
                // 3DNow with a suffix that is no instruction: only the escape byte is taken.
                (1, 0x0f) => 1,
                (1, _) => 2,
                _ => 3,
            };
            let m = eff_ref.get(lay.modrm_pos).copied().unwrap_or(0);
            // VEX and EVEX with a valid map and payload: see `vex_len`.
            let p1 = eff_ref.get(lay.modrm_pos + 1).copied().unwrap_or(0);
            let vex = match (lay.map, lay.op) {
                (0, 0xc5) if m >= 0xc0 => true,
                (0, 0xc4) if m >= 0xc0 && matches!(m & 0x1f, 1..=3 | 5..=7) => true,
                (0, 0x62) if m >= 0xc0 && matches!(m & 0xf, 1..=3 | 5..=7) && p1 & 4 != 0 => true,
                _ => false,
            };
            if vex {
                let n = Self::vex_len(eff_ref, lay.opcode_pos);
                return (n + n_lock, "(bad)".to_string(), false);
            }
            let extra = match (lay.map, lay.op) {
                // XOP: with a valid map the tool takes the payload bytes and the opcode along.
                (0, 0x8f) if matches!(m & 0x1f, 8..=10) => 3,
                // EVEX with a bad map or a clear bit 2 in the second payload byte stops after
                // the payload's second byte.
                (0, 0x62) if m >= 0xc0 && m & 3 != 0 => 2,
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
                false,
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
        (used.len() + n_lock + removed, text, false)
    }

    /// The length the tool takes for a VEX or EVEX encoded instruction the decoder refuses (a
    /// register number out of range in the 32-bit mode, a mask or rounding control the
    /// instruction cannot have, a form the opcode does not have): bad operands do not change the
    /// length. An opcode the tool knows (see `vextab`) takes its ModRM, SIB and displacement and
    /// immediate bytes; with a ModRM form it does not have, it takes the opcode and the immediate
    /// bytes only; an opcode it does not know, or a vector length the opcode cannot have, takes
    /// the bytes up to and including the opcode. `eff` is the instruction's bytes, the escape
    /// byte at `at`.
    fn vex_len(eff: &[u8], at: usize) -> usize {
        let byte = |i: usize| eff.get(i).copied().unwrap_or(0);
        let kind = byte(at);
        let (kind_i, map, pp, w, mut ll, vvvv, op_at) = match kind {
            0xc5 => {
                let p = byte(at + 1);
                (0, 1, p & 3, 0, (p >> 2) & 1, (p >> 3) & 0xf, at + 2)
            }
            0xc4 => {
                let (p0, p1) = (byte(at + 1), byte(at + 2));
                (1, p0 & 0x1f, p1 & 3, p1 >> 7, (p1 >> 2) & 1, (p1 >> 3) & 0xf, at + 3)
            }
            _ => {
                let (p0, p1, p2) = (byte(at + 1), byte(at + 2), byte(at + 3));
                (2, p0 & 0xf, p1 & 3, p1 >> 7, (p2 >> 5) & 3, (p1 >> 3) & 0xf, at + 4)
            }
        };
        let vv = u32::from(vvvv != 0xf);
        let undefined = (op_at + 1).min(eff.len());
        if kind_i == 2 {
            // The vector length field and the broadcast or rounding bit make one number; a
            // field of 3 without the bit is no instruction.
            let b = (byte(at + 3) >> 4) & 1;
            if ll == 3 && b == 0 {
                return undefined;
            }
            ll = ll * 2 + b;
        }
        let Some(&op) = eff.get(op_at) else {
            return eff.len();
        };
        let v = vextab::vex_op(
            kind_i as u32,
            map as u32,
            pp as u32,
            w as u32,
            ll as u32,
            vv,
            op as u32,
        );
        if v == 0 {
            return undefined;
        }
        let imm = (v & 3) as usize;
        let Some(&m) = eff.get(op_at + 1) else {
            return eff.len();
        };
        // An opcode that has no such reg field is undefined.
        if (v >> 8) & (1 << ((m >> 3) & 7)) == 0 {
            return undefined;
        }
        let is_reg = m >= 0xc0;
        let allowed = if is_reg { v & 8 != 0 } else { v & 4 != 0 };
        if !allowed {
            return op_at + 1 + imm;
        }
        let a16 = eff[..at].contains(&0x67);
        let modrm_len = if is_reg {
            1
        } else {
            match modrm_mem(eff, op_at + 1, a16, None) {
                Some((_, n)) => n,
                None => return eff.len(),
            }
        };
        op_at + 1 + modrm_len + imm
    }

    /// Forms the decoder refuses or reads differently from the tool: `(bad)` x87 encodings and
    /// the bad operands of `lea` and the far `call` and `jmp`.
    fn special(
        &self,
        eff: &[u8],
        orig_prefix: &[u8],
        lay: &Layout,
        addr: u32,
        wait_before: bool,
    ) -> Option<(usize, String)> {
        let mp = lay.modrm_pos;
        let m = *eff.get(mp)?;
        let all_words: Vec<&str> = orig_prefix
            .iter()
            .map(|&b| prefix_name(b).unwrap_or(""))
            .collect();
        let has = |b: u8| orig_prefix.contains(&b);
        let reg = (m >> 3) & 7;
        // The 64-bit-only `swapgs` is an instruction to the tool.
        if lay.map == 1 && lay.op == 0x01 && m == 0xf8 {
            return Some((mp + 1, join(&all_words, "swapgs", "")));
        }
        // The SSE4a pair reads the opcode and ModRM bytes as operands when the ModRM byte names
        // memory, and takes only up to that byte.
        if lay.map == 1 && lay.op == 0x78 && m < 0xc0 && (has(0x66) || has(0xf2)) {
            let (name, ops) = if has(0xf2) {
                ("insertq", format!("xmm{reg},(bad),0x78,0x{m:x}"))
            } else {
                ("extrq", format!("(bad),0x78,0x{m:x}"))
            };
            return Some((mp + 1, join(&[], name, &ops)));
        }
        if lay.map == 1 && lay.op == 0x78 && m >= 0xc0 && (has(0x66) || has(0xf2)) {
            let r = eff.get(mp + 1..mp + 3)?;
            let (rm, imms) = (m & 7, format!("0x{:x},0x{:x}", r[0], r[1]));
            let (name, ops) = if has(0xf2) {
                ("insertq", format!("xmm{reg},xmm{rm},{imms}"))
            } else {
                ("extrq", format!("xmm{rm},{imms}"))
            };
            return Some((mp + 3, join(&[], name, &ops)));
        }
        // `vzeroupper` and `vzeroall` take the escape, the payload byte and the opcode.
        if lay.map == 0 && lay.op == 0xc5 && m >= 0xc0 && eff.get(mp + 1) == Some(&0x77) {
            let name = if m & 4 != 0 { "vzeroall" } else { "vzeroupper" };
            if (m >> 3) & 0xf == 0xf {
                return Some((mp + 2, join(&[], name, "")));
            }
        }
        // `sm2` is `repnz` with the first padlock form.
        if lay.map == 1 && lay.op == 0xa6 && m == 0xc0 && has(0xf2) {
            return Some((mp + 1, join(&[], "sm2", "")));
        }
        // Memory-bound-check moves with an address-size prefix: both operands are named as bad
        // where the register number is out of range.
        if lay.map == 1 && matches!(lay.op, 0x1a | 0x1b) && m < 0xc0 && has(0x67) && !has(0x66)
            && !has(0xf2) && !has(0xf3)
        {
            let bnd = if reg < 4 { format!("bnd{reg}") } else { "(bad)".to_string() };
            let (name, ops) = if lay.op == 0x1a {
                ("bndldx", format!("{bnd},(bad)"))
            } else {
                ("bndstx", format!("(bad),{bnd}"))
            };
            // The address-size prefix is named; the bad operand takes the ModRM byte only.
            return Some((mp + 1, join(&all_words, name, &ops)));
        }
        // The 8087 and 287 control instructions of the `db` group.
        if lay.map == 0 && lay.op == 0xdb && matches!(m, 0xe0 | 0xe1 | 0xe4 | 0xe5) {
            let mut name = match m {
                0xe0 => "fneni(8087 only)",
                0xe1 => "fndisi(8087 only)",
                0xe4 => "fnsetpm(287 only)",
                _ => "frstpm(287 only)",
            }
            .to_string();
            if wait_before && name.starts_with("fn") {
                name.remove(1);
            }
            return Some((mp + 1, join(&all_words, &name, "")));
        }
        // The padlock groups: only the register forms with `rm` 0 are instructions (the others
        // are `(bad)`, see `badtab`).
        if lay.map == 1 && matches!(lay.op, 0xa6 | 0xa7) && m >= 0xc0 && m & 7 == 0 {
            const A6: [&str; 8] = [
                "", "xsha1", "xsha256", "xsha384", "xsha512", "", "montmul2", "xmodexp",
            ];
            const A7: [&str; 8] = [
                "xstore-rng",
                "xcrypt-ecb",
                "xcrypt-cbc",
                "xcrypt-ctr",
                "xcrypt-cfb",
                "xcrypt-ofb",
                "",
                "xrng2",
            ];
            let t = if lay.op == 0xa6 { &A6 } else { &A7 };
            let name = t[((m >> 3) & 7) as usize];
            if !name.is_empty() {
                return Some((mp + 1, join(&all_words, name, "")));
            }
        }
        // Moves to and from the test registers: the tool takes any ModRM byte as a register pair.
        if lay.map == 1 && matches!(lay.op, 0x24 | 0x26) {
            let (reg, rm) = ((m >> 3) & 7, REG32[(m & 7) as usize]);
            let ops = if lay.op == 0x24 {
                format!("{rm},tr{reg}")
            } else {
                format!("tr{reg},{rm}")
            };
            return Some((mp + 1, join(&all_words, "mov", &ops)));
        }
        // `xbegin` and `xabort` read their operand from the ModRM byte's place.
        if lay.map == 0
            && matches!(lay.op, 0xc6 | 0xc7)
            && (m >> 3) & 7 == 7
            && m & 7 == 0
            && m < 0xc0
        {
            if lay.op == 0xc7 && has(0x66) {
                let r = eff.get(mp..mp + 2)?;
                let rel = u16::from_le_bytes([r[0], r[1]]);
                let n = mp + 2;
                let target = (addr.wrapping_add(n as u32) as u16).wrapping_add(rel);
                return Some((
                    n,
                    join(&[], "xbeginw", &format!("(bad),0x{target:x}")),
                ));
            }
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
