"""Regenerates `src/disasm/badtab.rs`: which encodings GNU objdump 2.47 reads as `(bad)`.

Runs objdump 2.47 on authored bytes only (every opcode of the one-byte, 0f, 0f 38 and 0f 3a maps
with every ModRM byte, bare and after 66, f2 and f3); it never reads a game file. No test runs it;
run it by hand when the pinned objdump version changes:

    python3 tests/disasm_gen/gen_badtab.py [--check-small N]   # from the crate directory
    rustfmt --edition 2024 src/disasm/mod.rs

It streams in chunks (a few megabytes at most) and takes about two minutes.
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

OUT = os.path.join(HERE, '..', '..', 'src', 'disasm', 'badtab.rs')
PFX = [None, 0x66, 0xf2, 0xf3]
SKIP0 = {0x26, 0x2e, 0x36, 0x3e, 0x64, 0x65, 0x66, 0x67, 0xf0, 0xf2, 0xf3, 0x0f, 0x9b}
WORDS = {'data16', 'repz', 'repnz', 'lock', 'cs', 'ss', 'ds', 'es', 'fs', 'gs', 'addr16', 'rep',
         'bnd', 'notrack', 'fwait'}
LIMIT = int(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[1] == '--check-small' else None


def slots():
    for pi, p in enumerate(PFX):
        for mp, esc in enumerate([[], [0x0f], [0x0f, 0x38], [0x0f, 0x3a]]):
            for op in range(256):
                if mp == 0 and op in SKIP0:
                    continue
                if mp == 1 and op == 0x0f:
                    continue
                for m in range(256):
                    b = ([p] if p else []) + esc + [op, m]
                    key = ((pi * 4 + mp) * 256 + op) * 256 + m
                    yield key, (1 if p else 0), len(esc) + 1, bytes(b) + bytes([0x90]) * (32 - len(b))


r1, r2 = [], []


def add(rs, k):
    if rs and rs[-1][1] + 1 == k:
        rs[-1][1] = k
    else:
        rs.append([k, k])


chunk = []


def flush():
    global chunk
    if not chunk:
        return
    res = objdump_chunk([c[3] for c in chunk])
    for (key, npre, opn, _), (t, n) in zip(chunk, res):
        if t is None:
            continue
        toks = t.split(' ')
        while toks and toks[0] in WORDS:
            toks = toks[1:]
        body = ' '.join(toks)
        if n == npre + 1 and '(bad)' in body:
            add(r1, key)
        elif n == npre + opn and body == '(bad)':
            add(r2, key)
    chunk = []


check_objdump()
count = 0
for item in slots():
    chunk.append(item)
    count += 1
    if len(chunk) >= 8192:
        flush()
    if LIMIT and count >= LIMIT:
        break
flush()
print(count, len(r1), len(r2))
if LIMIT:
    sys.exit(0)


def fmt(rs):
    return ''.join('    (%#x, %#x),\n' % (a, b) for a, b in rs)


src = '''//! Which encodings the reference tool reads as `(bad)`, from its output on authored bytes
//! (generated once: every opcode of the one-byte, `0f`, `0f 38` and `0f 3a` maps with every ModRM
//! byte, bare and after each of the prefixes `66`, `f2` and `f3`, one 32-byte slot each; the
//! generator is `tests/disasm_gen/gen_badtab.py`).
//!
//! A key is `((prefix * 4 + map) * 256 + opcode) * 256 + modrm`, with `prefix` 0 for none, 1 for
//! `66`, 2 for `f2`, 3 for `f3`, and `map` 0 for the one-byte map, 1 for `0f`, 2 for `0f 38`, 3 for
//! `0f 3a`. The ranges are inclusive and sorted.

/// The tool takes the prefix and one opcode byte and prints `(bad)` (alone or as an operand: the
/// memory-only forms with a register operand, the unassigned padlock forms).
pub(crate) const BAD_ONE: &[(u32, u32)] = &[
''' + fmt(r1) + '''];

/// The tool takes the prefix and all opcode bytes and prints `(bad)`.
pub(crate) const BAD_ALL: &[(u32, u32)] = &[
''' + fmt(r2) + '''];

fn contains(t: &[(u32, u32)], k: u32) -> bool {
    t.binary_search_by(|&(a, b)| {
        if k < a {
            std::cmp::Ordering::Greater
        } else if k > b {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

/// How many opcode bytes the tool takes for the encoding when it reads it as `(bad)`:
/// `Some(1)` for the first one only, `Some(n)` for all `n`, `None` when it is an instruction.
pub(crate) fn bad_opcode_bytes(
    prefix: u8,
    map: u8,
    op: u8,
    modrm: u8,
    opcode_bytes: usize,
) -> Option<usize> {
    let k = ((prefix as u32 * 4 + map as u32) * 256 + op as u32) * 256 + modrm as u32;
    if contains(BAD_ONE, k) {
        Some(1)
    } else if contains(BAD_ALL, k) {
        Some(opcode_bytes)
    } else {
        None
    }
}
'''
with open(OUT, 'w') as f:
    f.write(src)
