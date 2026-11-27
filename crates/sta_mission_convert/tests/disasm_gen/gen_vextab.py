"""Regenerates `src/disasm/vextab.rs`: the VEX and EVEX opcodes GNU objdump 2.47 knows.

Runs objdump 2.47 on authored bytes only (every opcode of every map the escapes allow, for each
prefix class, width bit, vector length and `vvvv` use, with eight register and eight memory ModRM
reg fields); it never reads a game file. No test runs it; run it by hand when the pinned objdump
version changes:

    python3 tests/disasm_gen/gen_vextab.py [--check-small N]   # from the crate directory
    rustfmt --edition 2024 src/disasm/mod.rs

It streams in chunks (a few megabytes at most) and takes a few minutes.
"""
try:
    import resource
    resource.setrlimit(resource.RLIMIT_AS, (2 * 1024**3, 2 * 1024**3))
except (ImportError, ValueError, OSError):
    pass
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from chunked import check_objdump, objdump_chunk  # noqa: E402

OUT = os.path.join(HERE, '..', '..', 'src', 'disasm', 'vextab.rs')
LIMIT = int(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[1] == '--check-small' else None


def key(kind, mp, pp, w, ll, vv, op):
    return ((((((kind * 8 + mp) * 4 + pp) * 2 + w) * 8 + ll) * 2 + vv) * 256) + op


def keys():
    """Yields (key, 16 slots, opcode end) one key at a time: 8 register and 8 memory ModRM bytes."""
    for kind in (0, 1, 2):
        for mp in ([1] if kind == 0 else [1, 2, 3, 5, 6, 7]):
            for pp in range(4):
                for w in ([0] if kind == 0 else [0, 1]):
                    for ll in ([0, 1] if kind < 2 else [0, 1, 2, 3, 4, 5, 7]):
                        for vv in (0, 1):
                            vb = 0x78 if vv == 0 else 0x40
                            if kind == 0:
                                pre = [0xc5, 0xc0 | vb | (ll << 2) | pp]
                            elif kind == 1:
                                pre = [0xc4, 0xe0 | mp, (w << 7) | vb | (ll << 2) | pp]
                            else:
                                pre = [0x62, 0xf0 | mp, (w << 7) | vb | 0x04 | pp,
                                       ((ll >> 1) << 5) | ((ll & 1) << 4) | 0x08]
                            for op in range(256):
                                sl = []
                                for is_mem in (0, 1):
                                    for r in range(8):
                                        b = pre + [op, (0x00 if is_mem else 0xc0) | (r << 3)]
                                        sl.append(bytes(b) + bytes([0x90]) * (32 - len(b)))
                                yield key(kind, mp, pp, w, ll, vv, op), sl, len(pre) + 1


ranges = []


def emit(k, v):
    if ranges and ranges[-1][1] + 1 == k and ranges[-1][2] == v:
        ranges[-1][1] = k
    else:
        ranges.append([k, k, v])


def value(res, oplen):
    reg = mem = bad = None
    defined = False
    for i, (t, n) in enumerate(res):
        if t is None or t.startswith('(bad)'):
            continue
        defined = True
        if '(bad)' in t:
            if bad is None:
                bad = n - oplen
        else:
            imm = n - (oplen + 1)
            if i >= 8:
                if mem is None:
                    mem = imm
            elif reg is None:
                reg = imm
    if not defined:
        return 0
    mask = 0
    for r in range(8):
        for i in (r, 8 + r):
            t = res[i][0]
            if t is not None and not t.startswith('(bad)'):
                mask |= 1 << r
    imm = reg if reg is not None else mem if mem is not None else bad
    imm = max(0, min(3, imm if imm is not None else 0))
    return (mask << 8) | 0x10 | (0x08 if reg is not None else 0) | (0x04 if mem is not None else 0) | imm


chunk = []


def flush():
    global chunk
    if not chunk:
        return
    res = objdump_chunk([s for (_, sl, _) in chunk for s in sl])
    for j, (k, _sl, oplen) in enumerate(chunk):
        v = value(res[16 * j:16 * j + 16], oplen)
        if v:
            emit(k, v)
    chunk = []


check_objdump()
count = 0
for item in keys():
    chunk.append(item)
    count += 1
    if len(chunk) >= 256:
        flush()
    if LIMIT and count >= LIMIT:
        break
flush()
print(count, len(ranges))
if LIMIT:
    sys.exit(0)
body = ''.join('    (%#x, %#x, %#x),\n' % (a, b, v) for a, b, v in ranges)
src = '''//! The VEX and EVEX opcodes the reference tool knows, from its output on authored bytes
//! (generated once: every opcode of every map the escapes allow, for each prefix class, operand
//! width bit and vector length, with each of the eight register and the eight memory ModRM
//! reg fields, 32 bytes per slot; the generator is `tests/disasm_gen/gen_vextab.py`).
//!
//! A key is `((((((kind * 8 + map) * 4 + pp) * 2 + w) * 8 + ll) * 2 + vv) * 256) + opcode` with
//! `kind` 0 for the two-byte VEX escape (`w` is 0), 1 for the three-byte one and 2 for EVEX, `ll`
//! the vector length field (EVEX: twice the field plus the broadcast or rounding bit) and `vv` 1
//! when the `vvvv` field is not the unused value. The value is `0x10` for an opcode the tool
//! knows, `0x08` when some reg field has a register form, `0x04` when some has a memory form and
//! the low two bits count its immediate bytes; bits 8 to 15 say which ModRM reg fields the opcode
//! has. Opcodes that are not listed are undefined to the tool.

pub(crate) const VEX_OPS: &[(u32, u32, u16)] = &[
''' + body + '''];

/// The value for a key, 0 when the tool does not know the opcode.
pub(crate) fn vex_op(kind: u32, map: u32, pp: u32, w: u32, ll: u32, vv: u32, op: u32) -> u16 {
    let k = (((((kind * 8 + map) * 4 + pp) * 2 + w) * 8 + ll) * 2 + vv) * 256 + op;
    match VEX_OPS.binary_search_by(|&(a, b, _)| {
        if k < a {
            std::cmp::Ordering::Greater
        } else if k > b {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(i) => VEX_OPS[i].2,
        Err(_) => 0,
    }
}
'''
with open(OUT, 'w') as f:
    f.write(src)
