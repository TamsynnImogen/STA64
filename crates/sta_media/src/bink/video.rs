//! Bink 1 video: planes of 8x8 blocks coded through per-row "bundles" of
//! Huffman-coded values. Follows the clean-room description in the MIT
//! `mercs2_bink` specification (`SPEC-video.md`); the pre-`i` revisions
//! differ only in the colour bundle, the plane size prefixes and the chroma
//! plane order.

use super::bits::Bits;
use super::tables::{QUANT_INTER, QUANT_INTRA, SCAN, TREE_BYTES, TREE_MAXLEN, TREE_OFF};

/// Header video flag: an alpha plane precedes the luma plane.
pub(crate) const FLAG_ALPHA: u32 = 0x0010_0000;
/// Header video flag: no chroma planes.
pub(crate) const FLAG_GRAY: u32 = 0x0002_0000;

/// Coded order → row-major 8x8 position, for DCT coefficients and residues.
const PERM: [u8; 64] = [
    0, 1, 8, 9, 2, 3, 10, 11, 4, 5, 12, 13, 6, 7, 14, 15, //
    20, 21, 28, 29, 22, 23, 30, 31, 16, 17, 24, 25, 32, 33, 40, 41, //
    34, 35, 42, 43, 48, 49, 56, 57, 50, 51, 58, 59, 18, 19, 26, 27, //
    36, 37, 44, 45, 38, 39, 46, 47, 52, 53, 60, 61, 54, 55, 62, 63,
];

/// Run lengths for block-type symbols 12..15.
const TYPE_RUN: [usize; 4] = [4, 8, 12, 32];

/// One Huffman tree: a fixed code table plus a symbol map from the stream.
#[derive(Clone, Copy)]
struct Tree {
    off: u16,
    maxlen: u8,
    map: [u8; 16],
}

impl Tree {
    fn read(b: &mut Bits) -> Tree {
        let t = b.read(4) as usize;
        let mut map = [0u8; 16];
        for (i, m) in map.iter_mut().enumerate() {
            *m = i as u8;
        }
        if t != 0 {
            if b.bit() {
                let k = b.read(3) as usize;
                let mut used = [false; 16];
                for m in map.iter_mut().take(k + 1) {
                    let s = b.read(4) as u8;
                    *m = s;
                    used[s as usize] = true;
                }
                let mut i = k + 1;
                for (s, u) in used.iter().enumerate() {
                    if i >= 16 {
                        break;
                    }
                    if !u {
                        map[i] = s as u8;
                        i += 1;
                    }
                }
            } else {
                let depth = b.read(2);
                let mut p = [0u8; 16];
                for i in (0..16).step_by(2) {
                    if b.bit() {
                        p[i] = i as u8 + 1;
                        p[i + 1] = i as u8;
                    } else {
                        p[i] = i as u8;
                        p[i + 1] = i as u8 + 1;
                    }
                }
                map = p;
                // Successive merges of runs of 2, then 4, then 8.
                let mut size = 2;
                for _ in 0..depth.min(3) {
                    let mut out = [0u8; 16];
                    for start in (0..16).step_by(size * 2) {
                        merge(
                            b,
                            &map[start..start + size],
                            &map[start + size..start + 2 * size],
                            &mut out[start..start + 2 * size],
                        );
                    }
                    map = out;
                    size *= 2;
                }
            }
        }
        Tree {
            off: TREE_OFF[t],
            maxlen: TREE_MAXLEN[t],
            map,
        }
    }

    #[inline]
    fn decode(&self, b: &mut Bits) -> u8 {
        let v = b.peek(self.maxlen as u32) as usize;
        let e = TREE_BYTES[self.off as usize + v];
        b.skip((e >> 4) as u32);
        self.map[(e & 15) as usize]
    }
}

fn merge(b: &mut Bits, a: &[u8], c: &[u8], out: &mut [u8]) {
    let (mut i, mut j, mut o) = (0, 0, 0);
    loop {
        if b.bit() {
            out[o] = c[j];
            j += 1;
        } else {
            out[o] = a[i];
            i += 1;
        }
        o += 1;
        if i == a.len() {
            out[o..].copy_from_slice(&c[j..]);
            return;
        }
        if j == c.len() {
            out[o..].copy_from_slice(&a[i..]);
            return;
        }
    }
}

/// Significant bits of `x` as the format counts them (9..=15).
fn bitlen(x: usize) -> u32 {
    match x {
        0..0x200 => 9,
        0x200..0x400 => 10,
        0x400..0x800 => 11,
        0x800..0x1000 => 12,
        0x1000..0x2000 => 13,
        0x2000..0x4000 => 14,
        _ => 15,
    }
}

/// Pre-decoded values consumed in order by the blocks.
struct Bundle {
    data: Vec<i32>,
    cur: usize,
    end: usize,
    len_bits: u32,
    max: usize,
}

impl Bundle {
    fn new(len_bits: u32, max: usize) -> Self {
        Self {
            data: vec![0; max + 8],
            cur: 0,
            end: 0,
            len_bits,
            max,
        }
    }

    fn reset(&mut self) {
        self.cur = 0;
        self.end = 0;
    }

    /// Read a new count if exhausted; `None` if nothing is to be decoded
    /// (still holding data, or terminated by a zero/oversized count).
    fn start(&mut self, b: &mut Bits) -> Option<usize> {
        if self.cur != self.end {
            return None;
        }
        let count = b.read(self.len_bits) as usize;
        if count == 0 || count > self.max {
            self.cur = 4;
            self.end = 0;
            return None;
        }
        self.cur = 0;
        self.end = count;
        Some(count)
    }

    #[inline]
    fn available(&self) -> bool {
        self.cur < self.end
    }

    #[inline]
    fn next(&mut self) -> i32 {
        let v = self.data.get(self.cur).copied().unwrap_or(0);
        self.cur += 1;
        v
    }
}

/// The trees and bundles of one plane.
struct PlaneCtx {
    block_tree: Tree,
    sub_tree: Tree,
    colour_hi: [Tree; 16],
    colour_lo: Tree,
    pattern_tree: Tree,
    mx_tree: Tree,
    my_tree: Tree,
    run_tree: Tree,
    prev_hi: u8,
    old_colours: bool,
    block: Bundle,
    sub: Bundle,
    colour: Bundle,
    pattern: Bundle,
    mx: Bundle,
    my: Bundle,
    intra_dc: Bundle,
    inter_dc: Bundle,
    run: Bundle,
}

impl PlaneCtx {
    fn new(width: usize, old_colours: bool) -> Self {
        let bw = width >> 3;
        let cap = |n: usize| (n + 0x20f) & !15;
        let dc_max = ((bw * 2 + 0x40f) & !15) / 2;
        let dummy = Tree {
            off: 0,
            maxlen: 4,
            map: [0; 16],
        };
        Self {
            block_tree: dummy,
            sub_tree: dummy,
            colour_hi: [dummy; 16],
            colour_lo: dummy,
            pattern_tree: dummy,
            mx_tree: dummy,
            my_tree: dummy,
            run_tree: dummy,
            prev_hi: 0,
            old_colours,
            block: Bundle::new(bitlen(bw + 511), cap(bw)),
            sub: Bundle::new(bitlen((width >> 4) + 511), cap(width >> 4)),
            colour: Bundle::new(bitlen(bw * 64 + 511), cap(bw * 64)),
            pattern: Bundle::new(bitlen(bw * 8 + 511), cap(bw * 8)),
            mx: Bundle::new(bitlen(bw + 511), cap(bw)),
            my: Bundle::new(bitlen(bw + 511), cap(bw)),
            intra_dc: Bundle::new(bitlen(bw + 511), dc_max),
            inter_dc: Bundle::new(bitlen(bw + 511), dc_max),
            run: Bundle::new(bitlen(bw * 48 + 511), cap(bw * 48)),
        }
    }

    fn read_header(&mut self, b: &mut Bits) {
        self.block_tree = Tree::read(b);
        self.sub_tree = Tree::read(b);
        for t in self.colour_hi.iter_mut() {
            *t = Tree::read(b);
        }
        self.colour_lo = Tree::read(b);
        self.prev_hi = 0;
        self.pattern_tree = Tree::read(b);
        self.mx_tree = Tree::read(b);
        self.my_tree = Tree::read(b);
        self.run_tree = Tree::read(b);
        for bundle in [
            &mut self.block,
            &mut self.sub,
            &mut self.colour,
            &mut self.pattern,
            &mut self.mx,
            &mut self.my,
            &mut self.intra_dc,
            &mut self.inter_dc,
            &mut self.run,
        ] {
            bundle.reset();
        }
    }

    fn refill(&mut self, b: &mut Bits) {
        read_types(b, &mut self.block, &self.block_tree);
        read_types(b, &mut self.sub, &self.sub_tree);
        self.read_colours(b);
        if let Some(n) = self.pattern.start(b) {
            for i in 0..n {
                let lo = self.pattern_tree.decode(b);
                let hi = self.pattern_tree.decode(b);
                self.pattern.data[i] = ((hi << 4) | lo) as i32;
            }
        }
        read_motion(b, &mut self.mx, &self.mx_tree);
        read_motion(b, &mut self.my, &self.my_tree);
        read_dcs(b, &mut self.intra_dc, false);
        read_dcs(b, &mut self.inter_dc, true);
        if let Some(n) = self.run.start(b) {
            if b.bit() {
                let v = b.read(4) as i32;
                self.run.data[..n].fill(v);
            } else {
                for i in 0..n {
                    self.run.data[i] = self.run_tree.decode(b) as i32;
                }
            }
        }
        // The x read position follows the y one.
        self.mx.cur = self.my.cur;
    }

    fn read_colours(&mut self, b: &mut Bits) {
        let Some(n) = self.colour.start(b) else {
            return;
        };
        let fill = b.bit();
        let reps = if fill { 1 } else { n };
        for i in 0..reps {
            let hi = self.colour_hi[self.prev_hi as usize].decode(b);
            self.prev_hi = hi;
            let lo = self.colour_lo.decode(b);
            let mut v = ((hi << 4) | lo) as i32;
            if self.old_colours {
                // Before revision `i` colours are sign-magnitude around 128.
                let sign = ((v as u8 as i8) >> 7) as i32;
                v = (((v & 0x7f) ^ sign) - sign + 0x80) & 0xff;
            }
            self.colour.data[i] = v;
        }
        if fill {
            let v = self.colour.data[0];
            self.colour.data[..n].fill(v);
        }
    }
}

fn read_types(b: &mut Bits, bundle: &mut Bundle, tree: &Tree) {
    let Some(n) = bundle.start(b) else { return };
    if b.bit() {
        let v = b.read(4) as i32;
        bundle.data[..n].fill(v);
        return;
    }
    let mut last = 0i32;
    let mut i = 0;
    while i < n {
        let s = tree.decode(b);
        if s < 12 {
            last = s as i32;
            bundle.data[i] = last;
            i += 1;
        } else {
            let r = TYPE_RUN[(s - 12) as usize];
            if r > n - i {
                break;
            }
            bundle.data[i..i + r].fill(last);
            i += r;
        }
    }
}

fn read_motion(b: &mut Bits, bundle: &mut Bundle, tree: &Tree) {
    let Some(n) = bundle.start(b) else { return };
    if b.bit() {
        let mut v = b.read(4) as i32;
        if v != 0 && b.bit() {
            v = -v;
        }
        bundle.data[..n].fill(v);
        return;
    }
    for i in 0..n {
        let mut v = tree.decode(b) as i32;
        if v != 0 && b.bit() {
            v = -v;
        }
        bundle.data[i] = v;
    }
}

fn read_dcs(b: &mut Bits, bundle: &mut Bundle, signed: bool) {
    let Some(n) = bundle.start(b) else { return };
    let mut v: i32 = if signed {
        let mut v = b.read(10) as i32;
        if v != 0 && b.bit() {
            v = -v;
        }
        v
    } else {
        b.read(11) as i32
    };
    bundle.data[0] = v;
    let mut i = 1;
    while i < n {
        let g = (n - i).min(8);
        let w = b.read(4);
        for _ in 0..g {
            if w != 0 {
                let mut d = b.read(w) as i32;
                if d != 0 && b.bit() {
                    d = -d;
                }
                v = v.wrapping_add(d);
            }
            bundle.data[i] = v as i16 as i32;
            i += 1;
        }
    }
}

/// Coefficient-list entry kinds.
const G20: u8 = 0;
const G16: u8 = 1;
const G4: u8 = 2;
const SINGLE: u8 = 3;

/// The work list of the coefficient and residue readers: a deque that can
/// grow at both ends while it is walked.
struct CoefList {
    k: [u8; 192],
    mode: [u8; 192],
    used: [bool; 192],
    head: usize,
    tail: usize,
}

impl CoefList {
    fn new(init: &[(u8, u8)]) -> Self {
        let mut l = Self {
            k: [0; 192],
            mode: [0; 192],
            used: [false; 192],
            head: 64,
            tail: 64,
        };
        for &(k, m) in init {
            l.append(k, m);
        }
        l
    }

    fn append(&mut self, k: u8, mode: u8) {
        self.k[self.tail] = k;
        self.mode[self.tail] = mode;
        self.used[self.tail] = true;
        self.tail += 1;
    }

    fn prepend(&mut self, k: u8, mode: u8) {
        self.head -= 1;
        self.k[self.head] = k;
        self.mode[self.head] = mode;
        self.used[self.head] = true;
    }
}

#[inline]
fn coef_value(b: &mut Bits, level: u32) -> i32 {
    let f = b.read(level) as i32;
    let top = 1i32 << (level - 1);
    let mag = top | (f & (top - 1));
    if f & top != 0 { -mag } else { mag }
}

/// Read the AC coefficients of one block into `c` (row-major), leaving `c[0]`.
fn read_coefs(b: &mut Bits, c: &mut [i32; 64]) {
    let mut t = [0i32; 64];
    let n = b.read(4);
    let mut list = CoefList::new(&[
        (4, G20),
        (24, G20),
        (44, G20),
        (1, SINGLE),
        (2, SINGLE),
        (3, SINGLE),
    ]);
    for level in (1..=n).rev() {
        let mut i = list.head;
        while i < list.tail {
            if !list.used[i] || !b.bit() {
                i += 1;
                continue;
            }
            let k = list.k[i];
            match list.mode[i] {
                G20 => {
                    list.k[i] = k + 4;
                    list.mode[i] = G16;
                    read_four_coefs(b, &mut list, &mut t, k, level);
                }
                G16 => {
                    list.mode[i] = G4;
                    list.append(k + 4, G4);
                    list.append(k + 8, G4);
                    list.append(k + 12, G4);
                }
                G4 => {
                    list.used[i] = false;
                    i += 1;
                    read_four_coefs(b, &mut list, &mut t, k, level);
                }
                _ => {
                    t[k as usize] = coef_value(b, level);
                    list.used[i] = false;
                    i += 1;
                }
            }
        }
    }
    for k in 1..64 {
        c[PERM[k] as usize] = t[k];
    }
}

fn read_four_coefs(b: &mut Bits, list: &mut CoefList, t: &mut [i32; 64], k: u8, level: u32) {
    for j in 0..4 {
        if !b.bit() {
            t[(k + j) as usize] = coef_value(b, level);
        } else {
            list.prepend(k + j, SINGLE);
        }
    }
}

/// Read a residue block (signed bytes, coded order) with at most `limit + 1`
/// updates.
fn read_residue(b: &mut Bits, limit: u32) -> [i8; 64] {
    let mut r = [0i8; 64];
    let mut nz: Vec<u8> = Vec::with_capacity(64);
    let mut done = 0u32;
    // Count an update; true when this was the last allowed one.
    let bump = |done: &mut u32| {
        let stop = *done == limit;
        *done += 1;
        stop
    };
    let bits = b.read(3);
    let mut v = 1i32 << bits;
    let mut list = CoefList::new(&[(4, G20), (24, G20), (44, G20), (0, G4)]);
    for _ in 0..=bits {
        let known = nz.len();
        for &idx in &nz[..known] {
            if b.bit() {
                let cur = r[idx as usize] as i32;
                let nv = if cur < 0 { cur - v } else { cur + v };
                r[idx as usize] = nv as i8;
                if bump(&mut done) {
                    return r;
                }
            }
        }
        let mut i = list.head;
        while i < list.tail {
            if !list.used[i] || !b.bit() {
                i += 1;
                continue;
            }
            let k = list.k[i];
            match list.mode[i] {
                G20 | G4 => {
                    if list.mode[i] == G20 {
                        list.k[i] = k + 4;
                        list.mode[i] = G16;
                    } else {
                        list.used[i] = false;
                        i += 1;
                    }
                    for j in 0..4u8 {
                        if !b.bit() {
                            let idx = k + j;
                            nz.push(idx);
                            r[idx as usize] = if b.bit() { -v } else { v } as i8;
                            if bump(&mut done) {
                                return r;
                            }
                        } else {
                            list.prepend(k + j, SINGLE);
                        }
                    }
                }
                G16 => {
                    list.mode[i] = G4;
                    list.append(k + 4, G4);
                    list.append(k + 8, G4);
                    list.append(k + 12, G4);
                }
                _ => {
                    nz.push(k);
                    r[k as usize] = if b.bit() { -v } else { v } as i8;
                    list.used[i] = false;
                    i += 1;
                    if bump(&mut done) {
                        return r;
                    }
                }
            }
        }
        v >>= 1;
    }
    r
}

#[inline]
fn idct8(e: [i32; 8]) -> [i32; 8] {
    let m = |a: i32, k: i32| a.wrapping_mul(k) >> 11;
    let a = e[0].wrapping_add(e[4]);
    let b = e[0].wrapping_sub(e[4]);
    let s = e[2].wrapping_add(e[6]);
    let t = m(e[2].wrapping_sub(e[6]), 0xB50).wrapping_sub(s);
    let e0 = a.wrapping_add(s);
    let e3 = a.wrapping_sub(s);
    let e1 = b.wrapping_add(t);
    let e2 = b.wrapping_sub(t);
    let u = e[5].wrapping_add(e[3]);
    let v = e[5].wrapping_sub(e[3]);
    let w = e[1].wrapping_add(e[7]);
    let x = e[1].wrapping_sub(e[7]);
    let o0 = u.wrapping_add(w);
    let k = m(x.wrapping_add(v), 0xEC8);
    let o1 = m(v, -0x14E8).wrapping_sub(o0).wrapping_add(k);
    let o2 = m(w.wrapping_sub(u), 0xB50).wrapping_sub(o1);
    let o3 = m(x, 0x8A9).wrapping_sub(k).wrapping_add(o2);
    [
        e0.wrapping_add(o0),
        e1.wrapping_add(o1),
        e2.wrapping_add(o2),
        e3.wrapping_sub(o3),
        e3.wrapping_add(o3),
        e2.wrapping_sub(o2),
        e1.wrapping_sub(o1),
        e0.wrapping_sub(o0),
    ]
}

/// Dequantise and inverse-transform; result before the final `>> 8`.
fn idct(c: &[i32; 64], q: &[i32; 64]) -> [i32; 64] {
    let mut tmp = [0i32; 64];
    for col in 0..8 {
        let mut e = [0i32; 8];
        for (r, x) in e.iter_mut().enumerate() {
            *x = c[r * 8 + col].wrapping_mul(q[r * 8 + col]) >> 11;
        }
        let z = idct8(e);
        for r in 0..8 {
            tmp[r * 8 + col] = z[r];
        }
    }
    let mut out = [0i32; 64];
    for r in 0..8 {
        let mut e = [0i32; 8];
        e.copy_from_slice(&tmp[r * 8..r * 8 + 8]);
        let z = idct8(e);
        for col in 0..8 {
            out[r * 8 + col] = z[col].wrapping_add(0x7f) >> 8;
        }
    }
    out
}

/// One plane buffer with the decoder's own pitch.
#[derive(Clone)]
pub(crate) struct PlaneBuf {
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub data: Vec<u8>,
}

impl PlaneBuf {
    fn new(width: usize, height: usize, pitch: usize, fill: u8) -> Self {
        Self {
            width,
            height,
            pitch,
            data: vec![fill; pitch * height],
        }
    }
}

/// Decode one plane from `data` into `dst`, using `refp` as the previous
/// frame. Returns the bytes consumed (whole 32-bit words).
fn decode_plane(
    data: &[u8],
    ctx: &mut PlaneCtx,
    dst: &mut PlaneBuf,
    refp: &PlaneBuf,
) -> Result<usize, String> {
    let mut b = Bits::new(data);
    let (w, h, p) = (dst.width, dst.height, dst.pitch);
    let max_src = ((h - 8) * p + w - 8) as isize;
    ctx.read_header(&mut b);
    let mut blk = [0u8; 64];
    for y in (0..h).step_by(8) {
        ctx.refill(&mut b);
        if b.overrun() {
            return Err(format!("plane data ends in row {y}"));
        }
        let mut x = 0;
        while x < w {
            if !ctx.block.available() {
                x += 8;
                continue;
            }
            let t = ctx.block.next();
            let base = y * p + x;
            match t {
                0 => copy_block(&refp.data, base as isize, &mut dst.data, base, p),
                1 => {
                    if w - x < 16 {
                        x += 8;
                        continue;
                    }
                    if y & 8 != 0 || !ctx.sub.available() {
                        x += 16;
                        continue;
                    }
                    let s = ctx.sub.next();
                    if !matches!(s, 3 | 5 | 6 | 8 | 9) {
                        x += 16;
                        continue;
                    }
                    if h - y < 16 {
                        x += 8;
                        continue;
                    }
                    decode_small(&mut b, ctx, s, &mut blk);
                    for r in 0..16 {
                        let row = &mut dst.data[base + r * p..base + r * p + 16];
                        for (c, px) in row.iter_mut().enumerate() {
                            *px = blk[(r / 2) * 8 + c / 2];
                        }
                    }
                    x += 16;
                    continue;
                }
                2 | 4 | 7 => {
                    let mx = ctx.mx.next() as isize;
                    let my = ctx.my.next() as isize;
                    let src = (y as isize + my) * p as isize + x as isize + mx;
                    if src >= 0 && src <= max_src {
                        match t {
                            2 => copy_block(&refp.data, src, &mut dst.data, base, p),
                            4 => {
                                let limit = b.read(7);
                                let r = read_residue(&mut b, limit);
                                for row in 0..8 {
                                    for col in 0..8 {
                                        let pos = row * 8 + col;
                                        let s = refp.data[src as usize + row * p + col];
                                        dst.data[base + row * p + col] =
                                            s.wrapping_add(r[INV_PERM[pos] as usize] as u8);
                                    }
                                }
                            }
                            _ => {
                                let mut c = [0i32; 64];
                                c[0] = ctx.inter_dc.next();
                                read_coefs(&mut b, &mut c);
                                let q = b.read(4) as usize;
                                let out = idct(&c, &QUANT_INTER[q]);
                                for row in 0..8 {
                                    for col in 0..8 {
                                        let s = refp.data[src as usize + row * p + col];
                                        dst.data[base + row * p + col] =
                                            s.wrapping_add(out[row * 8 + col] as u8);
                                    }
                                }
                            }
                        }
                    }
                }
                3 | 5 | 6 | 8 | 9 => {
                    decode_small(&mut b, ctx, t, &mut blk);
                    for r in 0..8 {
                        dst.data[base + r * p..base + r * p + 8]
                            .copy_from_slice(&blk[r * 8..r * 8 + 8]);
                    }
                }
                _ => {}
            }
            x += 8;
        }
    }
    if b.overrun() {
        return Err("plane data ends early".into());
    }
    Ok(b.words_used_bytes())
}

static INV_PERM: [u8; 64] = {
    let mut inv = [0u8; 64];
    let mut k = 0;
    while k < 64 {
        inv[PERM[k] as usize] = k as u8;
        k += 1;
    }
    inv
};

fn copy_block(src: &[u8], src_off: isize, dst: &mut [u8], dst_off: usize, pitch: usize) {
    let s = src_off as usize;
    for r in 0..8 {
        dst[dst_off + r * pitch..dst_off + r * pitch + 8]
            .copy_from_slice(&src[s + r * pitch..s + r * pitch + 8]);
    }
}

/// Decode an 8x8 block of a reference-free kind (run, intra, fill, pattern,
/// raw) into `blk`, row-major.
fn decode_small(b: &mut Bits, ctx: &mut PlaneCtx, kind: i32, blk: &mut [u8; 64]) {
    match kind {
        3 => {
            let scan = &SCAN[b.read(4) as usize];
            let mut i = 0usize;
            let mut left = 64usize;
            loop {
                let r = ctx.run.next() as usize + 1;
                if left < r {
                    break;
                }
                left -= r;
                if !b.bit() {
                    for _ in 0..r {
                        blk[scan[i] as usize] = ctx.colour.next() as u8;
                        i += 1;
                    }
                } else {
                    let c = ctx.colour.next() as u8;
                    for _ in 0..r {
                        blk[scan[i] as usize] = c;
                        i += 1;
                    }
                }
                if left <= 1 {
                    break;
                }
            }
            if i == 63 {
                blk[scan[63] as usize] = ctx.colour.next() as u8;
            }
        }
        5 => {
            let mut c = [0i32; 64];
            c[0] = ctx.intra_dc.next();
            read_coefs(b, &mut c);
            let q = b.read(4) as usize;
            let out = idct(&c, &QUANT_INTRA[q]);
            for (d, s) in blk.iter_mut().zip(out.iter()) {
                *d = *s as u8;
            }
        }
        6 => {
            let c = ctx.colour.next() as u8;
            blk.fill(c);
        }
        8 => {
            let c0 = ctx.colour.next() as u8;
            let c1 = ctx.colour.next() as u8;
            for r in 0..8 {
                let bits = ctx.pattern.next();
                for col in 0..8 {
                    blk[r * 8 + col] = if bits >> col & 1 != 0 { c1 } else { c0 };
                }
            }
        }
        _ => {
            for px in blk.iter_mut() {
                *px = ctx.colour.next() as u8;
            }
        }
    }
}

/// Decoder state for the video of one movie.
pub(crate) struct VideoDecoder {
    revision: u8,
    flags: u32,
    /// Two buffer sets (Y, A, B, alpha), ping-ponged per frame.
    sets: [Vec<PlaneBuf>; 2],
    cur: usize,
    ctxs: Vec<PlaneCtx>,
}

/// Plane slots within a set.
const Y: usize = 0;
const A: usize = 1;
const B: usize = 2;
const ALPHA: usize = 3;

impl VideoDecoder {
    pub fn new(revision: u8, flags: u32, width: usize, height: usize) -> Self {
        let yw = (width + 7) & !7;
        let yh = (height + 7) & !7;
        let cw = (width.div_ceil(2) + 7) & !7;
        let ch = (height.div_ceil(2) + 7) & !7;
        let cp = (cw + 15) & !15;
        let set = vec![
            PlaneBuf::new(yw, yh, yw, 0),
            PlaneBuf::new(cw, ch, cp, 128),
            PlaneBuf::new(cw, ch, cp, 128),
            PlaneBuf::new(yw, yh, yw, 255),
        ];
        let old = revision < b'i';
        let ctxs = vec![
            PlaneCtx::new(yw, old),
            PlaneCtx::new(cw, old),
            PlaneCtx::new(cw, old),
            PlaneCtx::new(yw, old),
        ];
        Self {
            revision,
            flags,
            sets: [set.clone(), set],
            cur: 0,
            ctxs,
        }
    }

    pub fn has_alpha(&self) -> bool {
        self.flags & FLAG_ALPHA != 0
    }

    /// Clear the reference frames (before decoding from a key frame).
    pub fn reset(&mut self) {
        for set in &mut self.sets {
            set[Y].data.fill(0);
            set[A].data.fill(128);
            set[B].data.fill(128);
            set[ALPHA].data.fill(255);
        }
        self.cur = 0;
    }

    /// Decode one video packet.
    pub fn decode(&mut self, packet: &[u8]) -> Result<(), String> {
        let new = self.cur ^ 1;
        let (lo, hi) = self.sets.split_at_mut(1);
        let (dst, refs) = if new == 1 {
            (&mut hi[0], &lo[0])
        } else {
            (&mut lo[0], &hi[0])
        };
        let prefixed = self.revision >= b'i';
        let mut pos = 0usize;
        let slice = |pos: usize| {
            packet
                .get(pos..)
                .ok_or_else(|| "video packet too short".to_string())
        };
        if self.flags & FLAG_ALPHA != 0 {
            pos = sized_plane(packet, pos, prefixed, |d| {
                decode_plane(d, &mut self.ctxs[ALPHA], &mut dst[ALPHA], &refs[ALPHA])
            })?;
        }
        pos = sized_plane(packet, pos, prefixed, |d| {
            decode_plane(d, &mut self.ctxs[Y], &mut dst[Y], &refs[Y])
        })?;
        if self.flags & FLAG_GRAY == 0 {
            pos += decode_plane(slice(pos)?, &mut self.ctxs[A], &mut dst[A], &refs[A])?;
            decode_plane(slice(pos)?, &mut self.ctxs[B], &mut dst[B], &refs[B])?;
        }
        self.cur = new;
        Ok(())
    }

    /// The displayed planes, cropped to the picture: (Y, U, V, alpha).
    pub fn planes(&self) -> (&PlaneBuf, &PlaneBuf, &PlaneBuf, Option<&PlaneBuf>) {
        let set = &self.sets[self.cur];
        // Revision `h` onwards code V before U; earlier revisions U before V.
        let (u, v) = if self.revision >= b'h' {
            (&set[B], &set[A])
        } else {
            (&set[A], &set[B])
        };
        (&set[Y], u, v, self.has_alpha().then_some(&set[ALPHA]))
    }
}

/// Decode a plane that may carry a 32-bit size prefix counting itself;
/// returns where the next plane starts.
fn sized_plane(
    packet: &[u8],
    pos: usize,
    prefixed: bool,
    decode: impl FnOnce(&[u8]) -> Result<usize, String>,
) -> Result<usize, String> {
    if prefixed {
        let size = packet
            .get(pos..pos + 4)
            .map(|s| u32::from_le_bytes(s.try_into().unwrap()) as usize)
            .ok_or("video packet too short")?;
        if size < 4 || pos + size > packet.len() {
            return Err(format!("plane size {size} outside the packet"));
        }
        decode(&packet[pos + 4..pos + size])?;
        Ok(pos + size)
    } else {
        let data = packet.get(pos..).ok_or("video packet too short")?;
        Ok(pos + decode(data)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_lengths() {
        assert_eq!(bitlen(75 + 511), 10);
        assert_eq!(bitlen(75 * 64 + 511), 13);
        assert_eq!(bitlen(38 * 64 + 511), 12);
        assert_eq!(bitlen(511), 9);
    }

    #[test]
    fn identity_and_dc_idct() {
        // A lone DC of 1 with quantiser 0x10000 gives (1 << 5) per pass... check flatness.
        let mut c = [0i32; 64];
        c[0] = 64;
        let out = idct(&c, &QUANT_INTRA[0]);
        assert!(out.iter().all(|&v| v == out[0]));
    }

    #[test]
    fn perm_is_a_permutation() {
        let mut seen = [false; 64];
        for &p in &PERM {
            assert!(!seen[p as usize]);
            seen[p as usize] = true;
        }
        assert_eq!(INV_PERM[8], 2);
    }

    #[test]
    fn old_colour_mapping() {
        // Sign-magnitude: 0x00 → 128, 0x05 → 133, 0x85 → 123, 0xff → 1.
        let map = |v: i32| {
            let sign = ((v as u8 as i8) >> 7) as i32;
            (((v & 0x7f) ^ sign) - sign + 0x80) & 0xff
        };
        assert_eq!(map(0), 128);
        assert_eq!(map(5), 133);
        assert_eq!(map(0x85), 123);
        assert_eq!(map(0xff), 1);
    }
}
