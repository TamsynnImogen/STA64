#!/usr/bin/env python3
"""Writes expected.txt for cases.txt: the intermediate forms the ORIGINAL tools/dsl_to_lua.py
produces for the authored cases (LUA-15d). THIS SCRIPT READS NO GAME FILES: every input is an
invented listing and an invented flat image written in cases.txt.

    python3 -I gen_pairs.py <repo>/tools cases.txt expected.txt

The committed expected.txt is the reference: it was made with CPython 3.14, the version the
converter's Python reference uses (its first line says so). The texts of raw exceptions differ
between CPython versions, so the staleness test in tests/dsl_lift.rs compares only under that
version and says so when it skips.

The Rust test (tests/dsl_lift.rs) parses the same cases.txt and compares its own output with
expected.txt. The calls the lifter meets are run through the call model of lift_common.py (the
same model lift::call_model has in lift.rs), because the real `Lifter.call` is the next step's.

cases.txt format (one directive per line, `#` starts a comment line):

    case <name>              starts a case; `end` closes it
    std                      the standard flat image: base 0x10000000, 0x4000 bytes, sections
                             .text 0x1000+0x1000, .rdata 0x2000+0x400, .data 0x2400+0x400,
                             .bss 0x2800+0x400 (offsets from the base)
    base <addr> / size <n> / section <name> <va> <size>      a custom image
    put <addr> <hex bytes...>                    bytes at a virtual address
    dwords <addr> <n>...                         little-endian 32-bit words
    cstr <addr> <text>                           text and a NUL (`\\n` and `\\t` escapes)
    export <symbol> <addr>                       an export, in the order written
    code <addr>                                  instructions follow, each indented by two
                                                 spaces and 2 bytes apart; `@name:` defines a
                                                 label here, `@name` in a text is its address
    run <what> [arg]                             lift <addr> | blocks <addr> | facts <addr> |
                                                 dsl | static <addr> | statics | dimm <value> |
                                                 global_at <addr> | entry <addr> | dword <addr> |
                                                 take <n> <pushed words> (Lifter.take on a state)
"""

import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script's folder

import lift_common as lc  # noqa: E402


class FakePe:
    """The flat image of crates/sta_mission_convert/src/image.rs (FlatImage), in Python: the
    bytes are indexed by address - base; the sections only name ranges."""

    def __init__(self, base, size):
        self.base = base
        self.data = bytearray(size)
        self.sections = []  # (name, va, size)

    def off(self, rva):
        return rva if 0 <= rva < len(self.data) else None

    def section_of(self, addr):
        for nm, va, sz in self.sections:
            if self.base + va <= addr < self.base + va + sz:
                return nm
        return None

    def cstr(self, addr):
        o = self.off(addr - self.base)
        if o is None:
            raise DecodeError(f"string at {addr:#x} outside the file")
        return bytes(self.data[o:self.data.index(b"\0", o)]).decode("latin1")

    def f32(self, addr):
        o = self.off(addr - self.base)
        if o is None:
            raise DecodeError(f"float at {addr:#x} outside the file")
        return struct.unpack_from("<f", self.data, o)[0]

    def f64(self, addr):
        o = self.off(addr - self.base)
        if o is None:
            raise DecodeError(f"double at {addr:#x} outside the file")
        return struct.unpack_from("<d", self.data, o)[0]


import struct  # noqa: E402

DecodeError = None


STD_EXPORTS = [
    ("?g_pSI@@3PAVScriptInterface@@A", 0x10002800), ("?p_WORLD@@3PAVcharacter@@A", 0x10002804),
    ("?p_ALPHA@@3PAVhot_int@@A", 0x10002808), ("?p_BETA@@3PAVhot_float@@A", 0x1000280C),
    ("?cnt@@3HA", 0x10002810), ("?flt@@3MA", 0x10002814), ("?arr@@3PAHA", 0x10002818),
    ("?farr@@3PAMA", 0x10002828), ("?vec@@3VVector3@@A", 0x10002838), ("?flag@@3_NA", 0x10002844),
]

PROLOGUE = ["push   ebp", "mov    ebp,esp", "sub    esp,0x44", "push   ebx", "push   esi", "push   edi",
            "lea    edi,[ebp-0x44]", "mov    ecx,0x11", "mov    eax,0xcccccccc",
            "rep stos DWORD PTR es:[edi],eax"]
EPILOGUE = ["pop    edi", "pop    esi", "pop    ebx", "mov    esp,ebp", "pop    ebp", "ret"]


def num(s):
    return int(s, 16) if s.startswith("0x") else int(s)


def parse(text):
    """The cases of cases.txt: dicts with name, image spec, exports, code blocks, runs."""
    cases, cur = [], None
    for raw in text.split("\n"):
        line = raw.rstrip("\r")
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if line.startswith("  "):
            cur["blocks"][-1][1].append(line[2:])
            continue
        word, _, rest = line.partition(" ")
        if word == "case":
            cur = {"name": rest, "base": 0, "size": 0, "sections": [], "puts": [], "exports": [],
                   "blocks": [], "runs": []}
            cases.append(cur)
        elif word == "end":
            cur = None
        elif word == "std":
            cur["base"], cur["size"] = 0x10000000, 0x4000
            cur["sections"] = [(".text", 0x1000, 0x1000), (".rdata", 0x2000, 0x400),
                               (".data", 0x2400, 0x400), (".bss", 0x2800, 0x400)]
        elif word == "base":
            cur["base"] = num(rest)
        elif word == "size":
            cur["size"] = num(rest)
        elif word == "section":
            n, va, sz = rest.split(" ")
            cur["sections"].append((n, num(va), num(sz)))
        elif word == "put":
            a, _, hexes = rest.partition(" ")
            cur["puts"].append((num(a), bytes(int(x, 16) for x in hexes.split())))
        elif word == "dwords":
            parts = rest.split(" ")
            cur["puts"].append((num(parts[0]), parts[1:]))  # tokens: numbers or @labels
        elif word == "cstr":
            a, _, s = rest.partition(" ")
            s = s.replace("\\n", "\n").replace("\\t", "\t")
            cur["puts"].append((num(a), s.encode("latin1") + b"\0"))
        elif word == "stdexports":
            cur["exports"].extend(STD_EXPORTS)
        elif word == "export":
            sym, _, a = rest.rpartition(" ")
            cur["exports"].append((sym, num(a)))
        elif word == "code":
            cur["blocks"].append((num(rest), []))
        elif word == "run":
            cur["runs"].append(rest)
        else:
            raise SystemExit(f"cases.txt: unknown directive {word!r}")
    return cases


def listing(case):
    """The (address, text) instructions: 2 bytes apart from each `code` address, labels resolved."""
    labels, items = {}, []
    for start, lines in case["blocks"]:
        a = start
        expanded = []
        for ln in lines:
            expanded.extend(PROLOGUE if ln == "!p" else EPILOGUE if ln == "!e" else [ln])
        for ln in expanded:
            if ln.startswith("@") and ln.endswith(":"):
                labels[ln[1:-1]] = a
            else:
                items.append((a, ln))
                a += 2
    case["labels"] = labels
    out = []
    for a, t in items:
        parts = []
        i = 0
        while i < len(t):
            if t[i] == "@":
                j = i + 1
                while j < len(t) and (t[j].isalnum() or t[j] == "_"):
                    j += 1
                parts.append(f"{labels[t[i + 1:j]]:#x}")
                i = j
            else:
                parts.append(t[i])
                i += 1
        out.append((a, "".join(parts)))
    return out


def run_case(dsl, case):
    global DecodeError
    DecodeError = dsl.DecodeError
    pe = FakePe(case["base"], case["size"])
    pe.sections = list(case["sections"])
    ins = listing(case)
    for a, b in case["puts"]:
        if isinstance(b, list):  # dwords: numbers or @labels
            b = b"".join(struct.pack("<I", case["labels"][x[1:]] if x.startswith("@") else num(x)) for x in b)
        pe.data[a - pe.base:a - pe.base + len(b)] = b
    exports = dict(case["exports"])
    out = [f"== {case['name']}"]
    try:
        d = dsl.Dsl("", {}, pe=pe, ins=ins, exports=exports)
    except DecodeError as e:
        out.append(f"new error {str(e)!r}")
        return "\n".join(out) + "\n"
    lf = dsl.Lifter(d)
    for r in case["runs"]:
        what, _, arg = r.partition(" ")
        out.append(f"-- {r}")
        try:
            if what == "lift":
                out.append(lc.canonical(lf.lift(num(arg))).rstrip("\n"))
            elif what == "blocks":
                leaders, seen = lf.blocks(num(arg))
                out.append(f"leaders {leaders!r}")
                out.append(f"seen {sorted(seen)!r}")
                out.append(f"switches {list(lf.switches.items())!r}")
                out.append(f"byte_index {list(lf.byte_index.items())!r}")
            elif what == "facts":
                out.append(lc.kind_facts(d, num(arg)))
            elif what == "dsl":
                out.extend(lc.dsl_facts(d))
            elif what == "static":
                out.append(repr(d.static_class(num(arg))))
            elif what == "statics":
                out.append(lc.static_refs_line(d))
            elif what == "dimm":
                out.append(repr(dsl.d_immediate(d, num(arg))))
            elif what == "global_at":
                out.append(repr(d.global_at(num(arg))))
            elif what == "entry":
                out.append(repr(d.entry(num(arg))))
            elif what == "dword":
                out.append(repr(d.dword(num(arg))))
            elif what == "take":
                n, k = arg.split(" ")
                st = dsl.State()
                st.args = [("const", i) for i in range(int(k))]
                st.sub_esp = True
                res = lf.take(st, int(n), 0x1000)
                out.append(repr((res, st.args, st.sub_esp)))
            else:
                raise SystemExit(f"cases.txt: unknown run {what!r}")
        except DecodeError as e:
            out.append(f"error {str(e)!r}")
        except Exception as e:  # a raw Python exception aborts the real tool's run
            out.append(f"raw {type(e).__name__} {str(e)!r}")
            break
    out.append(lc.static_refs_line(d))
    return "\n".join(out) + "\n"


def main(argv):
    tools, cases_path, out_path = argv[1:4]
    dsl = lc.load_tool(tools)
    lc.install_call_model(dsl)
    with open(cases_path, encoding="utf-8", newline="") as fh:
        cases = parse(fh.read())
    # The first line names the CPython that printed the exception texts below (they differ between
    # versions); the staleness test compares only under the pinned major.minor version.
    head = f"# generated with CPython {sys.version_info[0]}.{sys.version_info[1]}\n"
    text = head + "".join(run_case(dsl, c) for c in cases)
    with open(out_path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(text)


if __name__ == "__main__":
    main(sys.argv)
