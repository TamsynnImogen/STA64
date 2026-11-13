#!/usr/bin/env python3
"""Decode Armada rule DLLs (`*R.drl`) and write Lua mission scaffolds (LUA-07).

What it implements (evidence: measured on the installed Armada II
campaign; notes are kept outside the repository): a `.drl` holds no rule
table, its rules are code. One function registers each rule with the
engine's rule engine (name, condition function, action function) and links
it to the `.dsl` variables that wake it; a condition reads those variables
and object handles and calls the engine through `g_pSI`; an action calls
one function the `.dsl` exports. The tool follows that code symbolically
through every branch and reports anything it does not recognise instead of
guessing. It writes, per mission, a Lua scaffold for the STA64 mission
runtime: variables and handles by name, the rules in creation order with
their conditions and dependencies, registered as named condition monitors
(docs/MISSIONS.md), and one stub per `.dsl` action. Engine calls go to the
`A2` table by the method's own name through a guard.

Not recoverable from a `.drl`: the action bodies, the start values of
variables and handles (both in the `.dsl`) and the engine's rule
scheduling. Rule DLLs of another shape (Armada I's) are reported as skipped.

Names and structure only: the scaffolds are written to `--out`, a folder the
user chooses outside the repository, and nothing read from a game file is
committed (docs/ARCHITECTURE.md, "Legal boundary").

usage:
  drl_to_lua.py --install <game root> [--install ...] --out <dir>
                [--engine <A2 root>] [--only a2_fed01] [--json report.json]

`--engine` names the `ScriptInterface` slots from `Armada2.exe` and
`armada2.map` (shipped with Armada II); by default the first install that
has both is used. Without it, engine calls are written as `slot_<n>`.
Needs `objdump` (binutils) on PATH. Exit status 1 if any `.drl` fails to
decode.
"""

import argparse
import json
import os
import re
import struct
import subprocess
import sys

TOOL = "tools/drl_to_lua.py"

# Rule-engine vtable slots the decoder relies on (measured; checked against the
# user's executable at run time when its map is present, see check_engine).
CHARACTER_GET_IIIE = 0  # character::GetIIIE()
IIIE_CREATE_RULE = 1  # IIIE::create_rule(int (*)(), void (*)(), char *)
HOT_ADD_DEPENDENT_RULE = 1  # hot_type::add_dependent_rule(rule *)
HOT_VALUE = 8  # hot_int::operator int() / hot_float::operator float()

VAR_KINDS = ("hot_int", "hot_float", "dirty_int", "timer_int")


class DecodeError(Exception):
    pass


class Unsupported(DecodeError):
    """A rule DLL of a shape this tool does not decode (Armada I's)."""


# ------------------------------------------------------------------ PE file


class Pe:
    def __init__(self, path):
        self.path = path
        b = self.data = open(path, "rb").read()
        if b[:2] != b"MZ":
            raise DecodeError("not a PE file")
        pe = struct.unpack_from("<I", b, 0x3C)[0]
        if b[pe:pe + 4] != b"PE\0\0":
            raise DecodeError("not a PE file")
        machine, n = struct.unpack_from("<HH", b, pe + 4)
        if machine != 0x14C:
            raise DecodeError(f"machine {machine:#x}, expected i386")
        opt = struct.unpack_from("<H", b, pe + 20)[0]
        self.base = struct.unpack_from("<I", b, pe + 24 + 28)[0]
        self.sections = []
        for i in range(n):
            nm, vs, va, rs, ra = struct.unpack_from("<8sIIII", b, pe + 24 + opt + i * 40)
            self.sections.append((nm.rstrip(b"\0").decode("latin1"), va, vs, ra, rs))
        dd = pe + 24 + 96
        self.export_size = struct.unpack_from("<I", b, dd + 4)[0]
        self.imports = {}  # IAT slot VA -> (dll, symbol)
        p = self.off(struct.unpack_from("<I", b, dd + 8)[0])
        while p is not None:
            ilt, _, _, name, iat = struct.unpack_from("<IIIII", b, p)
            if name == 0:
                break
            dll = self.cstr(self.base + name)
            q = self.off(ilt or iat)
            i = 0
            while True:
                e = struct.unpack_from("<I", b, q + 4 * i)[0]
                if e == 0:
                    break
                sym = f"#{e & 0xFFFF}" if e & 0x80000000 else self.cstr(self.base + e + 2)
                self.imports[self.base + iat + 4 * i] = (dll, sym)
                i += 1
            p += 20

    def off(self, rva):
        for _, va, vs, ra, rs in self.sections:
            if va <= rva < va + max(vs, rs) and rva - va < rs:
                return ra + rva - va
        return None

    def section_of(self, addr):
        for nm, va, vs, _, _ in self.sections:
            if self.base + va <= addr < self.base + va + vs:
                return nm
        return None

    def cstr(self, addr):
        o = self.off(addr - self.base)
        if o is None:
            raise DecodeError(f"string at {addr:#x} outside the file")
        return self.data[o:self.data.index(b"\0", o)].decode("latin1")

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


def disassemble(path):
    try:
        out = subprocess.run(["objdump", "-d", "-M", "intel", "--no-show-raw-insn", path],
                             capture_output=True, text=True, check=True).stdout
    except FileNotFoundError:
        sys.exit("objdump not found: install binutils (e.g. `pacman -S binutils`, `apt install binutils`)")
    return [(int(a, 16), t.strip()) for a, t in re.findall(r"^\s*([0-9a-f]+):\t(.*)$", out, re.M)]


def plain_name(symbol):
    """`?Name@Class@@...` or `?Name@@...` -> `Name`; anything else unchanged."""
    m = re.match(r"\?([A-Za-z_][A-Za-z0-9_]*)@", symbol)
    return m.group(1) if m else symbol


def import_kind(symbol):
    """What a `.dsl`/engine import is, from its MSVC decoration."""
    for k in VAR_KINDS + ("character", "ScriptInterface"):
        if re.search(rf"@@3PAV{k}@@A$", symbol) or re.search(rf"@@3V{k}@@A$", symbol):
            return k
    if symbol.endswith("@@3HA"):
        return "int"
    if symbol.endswith("@@3PAHA"):
        return "intarray"
    if symbol.endswith("@@3MA"):
        return "float"
    if re.search(r"@@YAXXZ$", symbol):
        return "action"
    if re.search(r"@@Y[AGI]", symbol):
        return "function"
    return "other"


# ------------------------------------------------------------------ engine names


def msvc_params(symbol):
    """Parameter type codes of an MSVC member-function name, or None.

    Enough of the decoration grammar to tell which arguments are `float`
    (`M`): primitives, pointers/references, named types and back-references."""
    m = re.match(r"\?[A-Za-z_][A-Za-z0-9_]*@[A-Za-z_][A-Za-z0-9_]*@@[QU]A[EG](.*)$", symbol) \
        or re.match(r"\?[A-Za-z_][A-Za-z0-9_]*@@Y[AGI](.*)$", symbol)
    if not m:
        return None
    s, pos, seen = m.group(1), [0], []

    def one():
        i = pos[0]
        if i >= len(s):
            raise ValueError
        c = s[i]
        if c == "_":
            pos[0] += 2
            return s[i:i + 2]
        if c in "PQAB" and i + 1 < len(s) and s[i + 1] in "ABCD":
            pos[0] += 2
            return c + one()
        if c in "VUT":
            j = s.index("@@", i)
            pos[0] = j + 2
            return s[i:j + 2]
        if c == "W":
            j = s.index("@@", i)
            pos[0] = j + 2
            return s[i:j + 2]
        if c == "?":
            pos[0] += 2  # ?A / ?B storage class of a return value
            return one()
        if c.isdigit():
            pos[0] += 1
            return seen[int(c)] if int(c) < len(seen) else "?"
        pos[0] += 1
        return c

    try:
        one()  # return type
        params = []
        while pos[0] < len(s) and s[pos[0]] not in "@Z":
            t = one()
            if t == "X" and not params:
                return []
            if len(t) > 1:
                seen.append(t)
            params.append(t)
        return params
    except (ValueError, IndexError):
        return None



ENGINE_CLASSES = ("ScriptInterfaceImp", "character", "IIIE", "hot_int", "hot_float", "dirty_int", "timer_int")


def engine_vtables(root, classes=ENGINE_CLASSES):
    """{class: [mangled method per vtable slot]} from Armada2.exe + armada2.map."""
    files = {f.lower(): os.path.join(root, f) for f in os.listdir(root)}
    exe, mapf = files.get("armada2.exe"), files.get("armada2.map")
    if not exe or not mapf:
        return None
    names = {}
    for line in open(mapf, encoding="latin1"):
        m = re.match(r"\s*[0-9a-f]{4}:[0-9a-f]{8}\s+(\S+)\s+([0-9a-f]{8})\s", line)
        if m:
            names.setdefault(int(m.group(2), 16), m.group(1))
    pe = Pe(exe)
    text = next((pe.base + va, vs) for nm, va, vs, _, _ in pe.sections if nm.startswith(".text"))
    out = {}
    for cls in classes:
        vt = next((a for a, v in names.items() if v == f"??_7{cls}@@6B@"), None)
        if vt is None:
            continue
        slots = []
        while True:
            if slots and vt + 4 * len(slots) in names:
                break  # the next symbol starts
            o = pe.off(vt + 4 * len(slots) - pe.base)
            a = struct.unpack_from("<I", pe.data, o)[0]
            if not text[0] <= a < text[0] + text[1]:
                break
            slots.append(names.get(a, f"?unnamed_{len(slots)}@@"))
        out[cls] = slots
    return out


def check_engine(vt):
    """The slot numbers this decoder relies on, against the user's executable."""
    want = [
        ("character", CHARACTER_GET_IIIE, "?GetIIIE@character@@"),
        ("IIIE", IIIE_CREATE_RULE, "?create_rule@IIIE@@UAEPAVrule@@P6AHXZP6AXXZPAD@Z"),
        ("hot_int", HOT_ADD_DEPENDENT_RULE, "?add_dependent_rule@hot_type@@"),
        ("hot_int", HOT_VALUE, "??Bhot_int@@"),
        ("hot_float", HOT_VALUE, "??Bhot_float@@"),
        ("timer_int", HOT_VALUE, "??Bhot_int@@"),
        ("dirty_int", HOT_VALUE, "??Bhot_int@@"),
    ]
    bad = []
    for cls, slot, prefix in want:
        got = vt.get(cls, [])
        if slot >= len(got) or not got[slot].startswith(prefix):
            bad.append(f"{cls} slot {slot}: expected {prefix}, found {got[slot] if slot < len(got) else 'none'}")
    return bad


# ------------------------------------------------------------------ symbolic values
#
# Values are tuples:
#   ("iat", sym)        address of an imported variable (an IAT slot's content)
#   ("val", sym)        an imported variable's value (object pointer or int)
#   ("vt", value)       the vtable of an object value
#   ("glob", addr)      a global of the .drl itself
#   ("const", n)        an integer, ("float", x), ("str", s), ("code", addr)
#   ("iiie",), ("rule", i)
# and expressions over .dsl state:
#   ("var", sym, kind)  a hot/timer/dirty variable's current value
#   ("obj", sym)        an object handle (int global)
#   ("call", name, args), ("not", x), ("cmp", op, a, b), ("arith", op, a, b),
#   ("and", [...]), ("or", [...]), ("bool", x)


REG32 = {"eax", "ebx", "ecx", "edx", "esi", "edi", "ebp", "esp"}
REG8 = {"al": "eax", "cl": "ecx", "dl": "edx", "bl": "ebx", "ah": "eax"}
NEG = {"==": "~=", "~=": "==", "<": ">=", ">=": "<", "<=": ">", ">": "<="}
JCC = {"je": "==", "jne": "~=", "jl": "<", "jge": ">=", "jle": "<=", "jg": ">"}
SETCC = {"sete": "==", "setne": "~=", "setl": "<", "setge": ">=", "setle": "<=", "setg": ">"}


class Drl:
    """One decoded rule DLL."""

    def __init__(self, path, engine=None):
        self.path = path
        self.pe = Pe(path)
        self.engine = engine or {}
        self.ins = disassemble(path)
        self.at = {a: i for i, (a, _) in enumerate(self.ins)}
        if not self.ins:
            raise DecodeError("no code")
        # Function entry thunks at the start of .text.
        self.thunks = {}
        for a, t in self.ins:
            if t == "int3":
                continue
            m = re.match(r"jmp\s+0x([0-9a-f]+)$", t)
            if not m:
                break
            self.thunks[a] = int(m.group(1), 16)
        self.chkesp = self._chkesp()
        # Engine methods (mangled names) the conditions call, by vtable slot.
        self.engine_slots = set()
        self.dlls = sorted({d for d, _ in self.pe.imports.values()})

    # ---------------------------------------------------------- helpers

    def _chkesp(self):
        """The debug runtime's stack check (a no-op for the decoder)."""
        seen = {}
        for i, (_, t) in enumerate(self.ins[:-1]):
            if re.match(r"cmp\s+e(si|di|bp),esp$", t):
                m = re.match(r"call\s+0x([0-9a-f]+)$", self.ins[i + 1][1])
                if m:
                    seen[int(m.group(1), 16)] = seen.get(int(m.group(1), 16), 0) + 1
        return max(seen, key=seen.get) if seen else None

    def entry(self, addr):
        addr = self.thunks.get(addr, addr)
        if addr not in self.at:
            raise DecodeError(f"no instruction at {addr:#x}")
        return addr

    def symbol(self, addr):
        return self.pe.imports.get(addr)

    def slot_params(self, slot):
        names = self.engine.get("ScriptInterfaceImp")
        return msvc_params(names[slot]) if names and slot < len(names) else None

    def slot_name(self, cls, slot):
        names = self.engine.get(cls)
        if names and slot < len(names):
            return plain_name(names[slot])
        return f"slot_{slot}"

    # ---------------------------------------------------------- operands

    def read(self, st, op):
        op = op.strip()
        if op in REG32:
            return st["r"].get(op)
        if op in REG8:
            return st["r"].get(REG8[op])
        m = re.fullmatch(r"(?:DWORD PTR |BYTE PTR )?ds:(0x[0-9a-f]+)", op)
        if m:
            return self.load(int(m.group(1), 16))
        m = re.fullmatch(r"(?:DWORD PTR |BYTE PTR )?\[(\w+)\]", op)
        if m:
            return self.deref(st["r"].get(m.group(1)))
        m = re.fullmatch(r"(?:DWORD PTR |BYTE PTR )?\[ebp([+-]0x[0-9a-f]+)\]", op)
        if m:
            return st["local"].get(int(m.group(1), 16))
        m = re.fullmatch(r"(?:DWORD PTR )?\[(\w+)\+(0x[0-9a-f]+)\]", op)
        if m:
            base = st["r"].get(m.group(1))
            if base and base[0] == "vt":
                return ("method", base[1], int(m.group(2), 16) // 4)
            return None
        m = re.fullmatch(r"(0x[0-9a-f]+)", op)
        if m:
            return self.immediate(int(m.group(1), 16))
        return None

    def immediate(self, n):
        sec = self.pe.section_of(n)
        if sec == ".rdata" or sec == ".data":
            try:
                s = self.pe.cstr(n)
                if s and all(32 <= ord(c) < 127 for c in s):
                    return ("str", s)
            except DecodeError:
                pass
        if sec == ".text":
            return ("code", n)
        return ("const", n - (1 << 32) if n >= 1 << 31 else n)

    def load(self, addr):
        s = self.symbol(addr)
        if s:
            return ("iat", s[1])
        return ("glob", addr)

    def deref(self, v):
        if v is None:
            return None
        if v[0] == "iat":
            k = import_kind(v[1])
            if k == "int":
                return ("obj", plain_name(v[1]))
            if k == "float":
                return ("objf", plain_name(v[1]))
            if k == "intarray":
                # `int name[]`: a load through the import is its first element
                return ("elem", plain_name(v[1]), 0)
            return ("val", v[1])
        if v[0] == "val":
            return ("vt", v)
        if v[0] in ("iiie", "rule"):
            return ("vt", v)
        if v[0] == "vt":
            return ("method", v[1], 0)  # `call [vtable]`: slot 0
        return None

    def take_args(self, st, i):
        """The arguments of the call at instruction i, in declaration order:
        the pushes since the stack pointer was saved for the call's check."""
        n = 0
        if i + 1 < len(self.ins):
            m = re.match(r"cmp\s+(e[sd]i),esp$", self.ins[i + 1][1])
            if m and m.group(1) in st.setdefault("marks", {}):
                n = min(st["marks"][m.group(1)], len(st["args"]))
        args = st["args"][n:]
        st["args"] = st["args"][:n]
        return list(reversed(args))

    # ---------------------------------------------------------- registration

    def rules(self):
        """[(name, condition addr, action addr, rule global)] in creation order, plus deps."""
        rules, deps, by_glob = [], {}, {}
        for start in sorted(set(self.thunks.values())):
            if start not in self.at:
                continue
            st = {"r": {}, "local": {}, "args": []}
            i = self.at[start]
            while i < len(self.ins):
                a, t = self.ins[i]
                i += 1
                if t.startswith("ret"):
                    break
                parts = t.split(None, 1)
                mn, ops = parts[0], (parts[1] if len(parts) > 1 else "")
                if mn == "push":
                    st["args"].append(self.read(st, ops))
                elif mn == "mov":
                    dst, src = [x.strip() for x in ops.split(",", 1)]
                    m = re.fullmatch(r"(?:DWORD PTR )?ds:(0x[0-9a-f]+)", dst)
                    if m:
                        v = st["r"].get(src)
                        if v and v[0] == "rule":
                            by_glob[int(m.group(1), 16)] = v[1]
                        continue
                    m = re.fullmatch(r"(?:DWORD PTR )?\[ebp([+-]0x[0-9a-f]+)\]", dst)
                    if m:
                        st["local"][int(m.group(1), 16)] = self.read(st, src)
                        continue
                    if dst in ("esi", "edi") and src == "esp":
                        st.setdefault("marks", {})[dst] = len(st["args"])
                        continue
                    if dst in REG32:
                        v = self.read(st, src)
                        if v and v[0] == "glob" and v[1] in by_glob:
                            v = ("rule", by_glob[v[1]])
                        st["r"][dst] = v
                elif mn == "call":
                    if re.fullmatch(r"0x[0-9a-f]+", ops) and int(ops, 16) == self.chkesp:
                        continue
                    target = self.read(st, ops)
                    args = self.take_args(st, i - 1)
                    st["r"]["eax"] = None
                    if not target or target[0] != "method":
                        continue
                    obj, slot = target[1], target[2]
                    if obj and obj[0] == "val" and plain_name(obj[1]) == "p_WORLD" and slot == CHARACTER_GET_IIIE:
                        st["r"]["eax"] = ("iiie",)
                    elif obj == ("iiie",) and slot == IIIE_CREATE_RULE and len(args) >= 3:
                        cond, act, name = args[0], args[1], args[2]
                        if not (cond and cond[0] == "code" and act and act[0] == "code" and name and name[0] == "str"):
                            raise DecodeError(f"create_rule at {a:#x} with unexpected arguments {args}")
                        rules.append([name[1], cond[1], act[1]])
                        st["r"]["eax"] = ("rule", len(rules) - 1)
                    elif obj and obj[0] == "val" and slot == HOT_ADD_DEPENDENT_RULE and args and args[0] \
                            and args[0][0] == "rule" and import_kind(obj[1]) in VAR_KINDS:
                        deps.setdefault(args[0][1], []).append(plain_name(obj[1]))
                elif mn in ("add", "sub", "lea", "rep", "cmp", "test", "pop", "and", "xor", "or", "inc", "dec",
                            "neg", "sbb") or mn.startswith("j") or mn.startswith("f") or mn.startswith("set"):
                    if mn == "pop" and st["args"]:
                        st["args"].pop()
                    if mn in ("add", "sub") and ops.startswith("esp,"):
                        st["args"] = []
                    continue
        return rules, deps

    # ---------------------------------------------------------- conditions

    def condition(self, addr):
        """The boolean expression a condition function returns.

        The debug compiler's conditions only jump forward, so the function is
        a DAG: states are run in address order and merged where branches
        join, keeping the path condition as a list of predicates. A value
        that differs between the joining paths (the 0/1 the compiler stores
        for a materialised `&&`/`||`) becomes the disjunction of the paths
        that stored 1."""
        start = self.at[self.entry(addr)]
        targets, i = set(), start
        while True:
            if i >= len(self.ins):
                raise DecodeError("ran off the code")
            a, t = self.ins[i]
            m = re.match(r"j\w+\s+0x([0-9a-f]+)$", t)
            if m:
                tgt = int(m.group(1), 16)
                if tgt <= a:
                    raise DecodeError(f"backward jump at {a:#x}")
                targets.add(tgt)
            if t.startswith("ret") and not any(x > a for x in targets):
                break
            i += 1
        pending = {start: [([], {"r": {}, "local": {}, "args": [], "fpu": [], "flags": None, "body": False})]}
        results = []
        while pending:
            i = min(pending)
            path, st = merge(pending.pop(i))
            first = True
            while True:
                a, t = self.ins[i]
                if not first and a in targets:
                    pending.setdefault(i, []).append((path, st))
                    break
                first = False
                parts = t.split(None, 1)
                mn, ops = parts[0], (parts[1] if len(parts) > 1 else "")
                nxt = self.step(st, a, mn, ops, i)
                if nxt == "ret":
                    results.append((path, st["r"].get("eax")))
                    break
                if nxt is None:
                    i += 1
                    continue
                kind, target, pred = nxt
                j = self.at.get(target)
                if j is None:
                    raise DecodeError(f"jump outside the code at {a:#x}")
                if kind == "jmp":
                    pending.setdefault(j, []).append((path, st))
                    break
                pred = simplify(pred)
                if pred != FALSE:
                    pending.setdefault(j, []).append((path + [pred], clone(st)))
                if pred == TRUE:
                    break
                path = path + [simplify(negate(pred))]
                i += 1
        terms = []
        for path, v in results:
            if v is None:
                raise DecodeError("condition returns an unknown value")
            terms.append(conj(path + [as_bool(v)]))
        return simplify(("or", terms)) if terms else FALSE

    def step(self, st, a, mn, ops, i):
        r = st["r"]
        if mn == "ret":
            return "ret"
        if mn in ("push",):
            v = self.read(st, ops)
            if ops in ("ebp", "ebx", "esi", "edi") and not st["body"]:
                return None  # saved registers of the prologue
            st["args"].append(v)
            return None
        if mn == "pop":
            if st["args"]:
                st["args"].pop()
            return None
        if mn in ("int3", "nop", "fnclex", "fwait"):
            return None
        if mn == "rep":
            st["body"] = True  # the debug prologue (fill with 0xcc) is done
            st["args"] = []
            return None
        if mn == "lea" or (mn in ("sub", "add") and ops.startswith("esp,")):
            if mn == "add":
                st["args"] = []
            return None
        if mn == "mov":
            dst, src = [x.strip() for x in ops.split(",", 1)]
            if dst in ("esi", "edi") and src == "esp":
                st.setdefault("marks", {})[dst] = len(st["args"])
                return None
            if dst == "ebp" or dst == "esp":
                return None
            m = re.fullmatch(r"(?:DWORD PTR )?\[ebp([+-]0x[0-9a-f]+)\]", dst)
            if m:
                st["local"][int(m.group(1), 16)] = self.read(st, src)
                return None
            if dst in REG32:
                r[dst] = self.read(st, src)
                return None
            raise DecodeError(f"unsupported store `{mn} {ops}` at {a:#x}")
        if mn == "call":
            m = re.fullmatch(r"0x([0-9a-f]+)", ops)
            if m and int(m.group(1), 16) == self.chkesp:
                return None
            args = self.take_args(st, i)
            target = self.read(st, ops)
            m = re.fullmatch(r"(?:DWORD PTR )?ds:(0x[0-9a-f]+)", ops)
            if m and self.symbol(int(m.group(1), 16)):
                sym = self.symbol(int(m.group(1), 16))[1]
                k = import_kind(sym)
                if k not in ("function", "action"):
                    raise DecodeError(f"call to {sym} at {a:#x}")
                v = ("call", plain_name(sym), typed_args([expr_arg(x, a) for x in args], msvc_params(sym)))
            elif target and target[0] == "method":
                obj, slot = target[1], target[2]
                if obj[0] != "val":
                    raise DecodeError(f"method call on {obj} at {a:#x}")
                k = import_kind(obj[1])
                if k in VAR_KINDS:
                    if slot != HOT_VALUE or args:
                        raise DecodeError(f"{k} slot {slot} in a condition at {a:#x}")
                    v = ("var", plain_name(obj[1]), k)
                elif k == "ScriptInterface":
                    self.engine_slots.add(slot)
                    v = ("call", self.slot_name("ScriptInterfaceImp", slot),
                         typed_args([expr_arg(x, a) for x in args], self.slot_params(slot)))
                else:
                    raise DecodeError(f"call on {obj[1]} slot {slot} at {a:#x}")
            else:
                raise DecodeError(f"unsupported call `{ops}` at {a:#x}")
            r["eax"] = v
            st["fret"] = v
            for x in ("ecx", "edx"):
                r[x] = None
            return None
        if mn == "test":
            x, y = [s.strip() for s in ops.split(",", 1)]
            if x == y:
                st["flags"] = ("test", self.read(st, x))
                return None
            if x == "ah" and st.get("fsw"):
                st["flags"] = ("ftest", st["fsw"], int(y, 16))
                return None
            raise DecodeError(f"unsupported `test {ops}` at {a:#x}")
        if mn == "cmp":
            x, y = [s.strip() for s in ops.split(",", 1)]
            if y == "esp":
                return None
            st["flags"] = ("cmp", self.read(st, x), self.read(st, y))
            return None
        if mn in JCC:
            target = int(ops, 16)
            f = st["flags"]
            if f is None:
                raise DecodeError(f"branch without a compare at {a:#x}")
            return ("jcc", target, branch_pred(mn, f, a))
        if mn == "jmp":
            return ("jmp", int(ops, 16), None)
        if mn in SETCC:
            dst = REG8.get(ops.strip())
            f = st["flags"]
            if not dst or not f or f[0] != "cmp":
                raise DecodeError(f"unsupported `{mn} {ops}` at {a:#x}")
            r[dst] = ("cmp", SETCC[mn], f[1], f[2])
            return None
        if mn == "xor":
            x, y = [s.strip() for s in ops.split(",", 1)]
            if x == y and x in REG32:
                r[x] = ("const", 0)
                return None
            raise DecodeError(f"unsupported `xor {ops}` at {a:#x}")
        if mn == "and":
            x, y = [s.strip() for s in ops.split(",", 1)]
            if x in REG32 and y == "0xff":
                v = r.get(x)
                r[x] = ("bool", v) if v and v[0] not in ("cmp", "not", "and", "or", "bool", "const") else v
                st["flags"] = ("test", r[x])
                return None
            raise DecodeError(f"unsupported `and {ops}` at {a:#x}")
        if mn == "neg" and ops == "eax":
            v = r.get("eax")
            # the two compiler idioms for `x != 0` and `!x`
            r["eax"] = as_bool(v[1]) if v and v[0] == "m1" else ("neg", v)
            return None
        if mn == "sbb" and ops == "eax,eax":
            v = r.get("eax")
            if not v or v[0] != "neg":
                raise DecodeError(f"unsupported sbb at {a:#x}")
            r["eax"] = ("m1", v[1])
            return None
        if mn == "inc" and ops == "eax":
            v = r.get("eax")
            if v and v[0] == "m1":
                r["eax"] = ("not", as_bool(v[1]))
                return None
            if v and v[0] == "const":
                r["eax"] = ("const", v[1] + 1)
                return None
            r["eax"] = ("arith", "+", v, ("const", 1))
            return None
        if mn in ("add", "sub"):
            x, y = [s.strip() for s in ops.split(",", 1)]
            if x in REG32:
                vx, vy = r.get(x), self.read(st, y)
                st["flags"] = ("cmp", vx, vy) if mn == "sub" else ("test", ("arith", "+", vx, vy))
                r[x] = ("arith", {"add": "+", "sub": "-"}[mn], vx, vy)
                return None
            raise DecodeError(f"unsupported `{mn} {ops}` at {a:#x}")
        # x87
        if mn == "fstp":
            v = fpop(st, a)
            m = re.fullmatch(r"(?:DWORD|QWORD) PTR \[ebp([+-]0x[0-9a-f]+)\]", ops)
            if m:
                st["local"][int(m.group(1), 16)] = v
                return None
            if ops == "st(0)":
                return None
            raise DecodeError(f"unsupported `fstp {ops}` at {a:#x}")
        if mn in ("fld", "fild"):
            v = self.fread(st, ops, a, integer=mn == "fild")
            st["fpu"].append(v)
            return None
        if mn in ("fadd", "fsub", "fmul", "fdiv", "fsubr", "fdivr"):
            op = {"fadd": "+", "fsub": "-", "fmul": "*", "fdiv": "/", "fsubr": "r-", "fdivr": "r/"}[mn]
            top = fpop(st, a)
            if ops in ("st(0),st", "st,st(0)", "st(0),st(0)"):
                v = ("arith", "*", ("float", 2.0), top) if op == "+" else ("arith", op, top, top)
            else:
                v = binop(op, top, self.fread(st, ops, a))
            st["fpu"].append(v)
            return None
        if mn in ("fcomp", "fcom", "fcompp"):
            other = None if mn == "fcompp" else self.fread(st, ops, a)
            top = fpop(st, a) if mn != "fcom" else fpeek(st, a)
            if mn == "fcompp":
                other = fpop(st, a)
            st["fcmp"] = (top, other)
            return None
        if mn == "fnstsw" and ops == "ax":
            if not st.get("fcmp"):
                raise DecodeError(f"fnstsw without a compare at {a:#x}")
            st["fsw"] = st["fcmp"]
            r["eax"] = None
            return None
        raise DecodeError(f"unsupported instruction `{mn} {ops}` at {a:#x}")

    def fread(self, st, op, a, integer=False):
        m = re.fullmatch(r"(DWORD|QWORD) PTR ds:(0x[0-9a-f]+)", op)
        if m:
            addr = int(m.group(2), 16)
            if self.symbol(addr):
                return self.deref(("iat", self.symbol(addr)[1]))
            if integer:
                raise DecodeError(f"integer load from {addr:#x} at {a:#x}")
            return ("float", self.pe.f32(addr) if m.group(1) == "DWORD" else self.pe.f64(addr))
        m = re.fullmatch(r"(?:DWORD|QWORD) PTR \[ebp([+-]0x[0-9a-f]+)\]", op)
        if m:
            return st["local"].get(int(m.group(1), 16))
        m = re.fullmatch(r"DWORD PTR \[(\w+)\]", op)
        if m:
            return self.deref(st["r"].get(m.group(1)))
        m = re.fullmatch(r"st\((\d)\)", op)
        if m:
            fpeek(st, a)
            k = int(m.group(1))
            if k >= len(st["fpu"]):
                raise DecodeError(f"x87 st({k}) is empty at {a:#x}")
            return st["fpu"][-1 - k]
        raise DecodeError(f"unsupported x87 operand `{op}` at {a:#x}")


TRUE, FALSE = ("const", 1), ("const", 0)


def merge(incoming):
    """One state from the states that reach a join point."""
    if len(incoming) == 1:
        return incoming[0]
    paths = [p for p, _ in incoming]
    n = 0
    while all(len(p) > n for p in paths) and all(p[n] == paths[0][n] for p in paths):
        n += 1
    prefix = paths[0][:n]
    rests = [conj(p[n:]) for p in paths]
    joined = simplify(("or", rests))
    path = prefix if joined == TRUE else prefix + [joined]
    states = [s for _, s in incoming]
    out = clone(states[0])

    def pick(vals):
        if all(v == vals[0] for v in vals):
            return vals[0]
        if all(v is not None and v[0] == "const" and v[1] in (0, 1) for v in vals):
            return ("boolv", simplify(("or", [r for r, v in zip(rests, vals) if v[1] == 1])))
        return ("conflict",)

    for slot in ("r", "local"):
        keys = set().union(*(s[slot].keys() for s in states))
        out[slot] = {k: pick([s[slot].get(k) for s in states]) for k in keys}
    for slot in ("flags", "fsw", "fcmp", "fret"):
        vals = [s.get(slot) for s in states]
        out[slot] = vals[0] if all(v == vals[0] for v in vals) else None
    for slot in ("args", "fpu"):
        vals = [s.get(slot) for s in states]
        if not all(v == vals[0] for v in vals):
            raise DecodeError(f"paths join with different {slot}")
    return path, out


def atoms(e, out):
    k = e[0]
    if k in ("and", "or"):
        for x in e[1]:
            atoms(x, out)
    elif k == "not":
        atoms(e[1], out)
    elif k == "const":
        pass
    else:
        key = atom_key(e)[0]
        if key not in out:
            out.append(key)
    return out


def atom_key(e):
    """(key, polarity): `x < y` and `x >= y` share a key."""
    if e[0] == "cmp" and e[1] in ("~=", ">=", ">"):
        return repr(("cmp", NEG[e[1]], e[2], e[3])), False
    return repr(e), True


def evaluate(e, env):
    k = e[0]
    if k == "and":
        return all(evaluate(x, env) for x in e[1])
    if k == "or":
        return any(evaluate(x, env) for x in e[1])
    if k == "not":
        return not evaluate(e[1], env)
    if k == "const":
        return bool(e[1])
    key, pol = atom_key(e)
    return env[key] == pol


def truth_table(e, limit=14):
    """True/False when e is constant over its atoms, else None."""
    names = atoms(e, [])
    if len(names) > limit:
        return None
    seen = set()
    for bits in range(1 << len(names)):
        seen.add(evaluate(e, {n: bool(bits >> i & 1) for i, n in enumerate(names)}))
        if len(seen) > 1:
            return None
    return seen.pop() if seen else None


def simplify(e):
    k = e[0]
    if k == "truth":
        v = e[1]
        if v[0] == "const":
            return TRUE if v[1] else FALSE
        if v[0] == "boolv":
            return simplify(v[1])
        if v[0] == "conflict":
            raise DecodeError("a value that differs between joined paths is used")
        return e
    if k == "not":
        x = simplify(e[1])
        if x[0] == "const":
            return FALSE if x[1] else TRUE
        return negate(x)
    if k == "cmp":
        if e[2][0] == "boolv" and e[3] == ("const", 0) and e[1] in ("==", "~="):
            x = simplify(e[2][1])
            return x if e[1] == "~=" else simplify(negate(x))
        return e
    if k in ("and", "or"):
        unit, zero = (TRUE, FALSE) if k == "and" else (FALSE, TRUE)
        items = []
        for x in (simplify(y) for y in e[1]):
            if x == unit:
                continue
            if x == zero:
                return zero
            for y in (x[1] if x[0] == k else [x]):
                if y not in items:
                    items.append(y)
        if not items:
            return unit
        if len(items) == 1:
            return items[0]
        if k == "or":
            items = absorb(items)
            if len(items) == 1:
                return items[0]
        out = (k, items)
        if k == "or":
            out = factor(out)
        tt = truth_table(out) if out[0] in ("and", "or") else None
        if tt is not None:
            return TRUE if tt else FALSE
        return out
    return e


def absorb(items):
    """`a or (not a and b)` -> `a or b`: the shape `||` compiles to."""
    out = []
    for x in items:
        if x[0] == "and":
            lits = [y for y in x[1] if simplify_neg(y) not in out]
            x = lits[0] if len(lits) == 1 else ("and", lits) if lits else TRUE
        elif simplify_neg(x) in out:
            x = TRUE
        if x == TRUE:
            return [TRUE]
        if x not in out:
            out.append(x)
    return out


def simplify_neg(x):
    return x[1] if x[0] == "not" else negate(x)


def factor(e):
    """`(p and a) or (p and b)` -> `p and (a or b)`: the leading terms
    the compiler's paths share."""
    terms = [t[1] if t[0] == "and" else [t] for t in e[1]]
    n = 0
    while all(len(t) > n + 1 for t in terms) and all(t[n] == terms[0][n] for t in terms):
        n += 1
    if n == 0:
        return e
    rest = simplify(("or", [conj(t[n:]) for t in terms]))
    return simplify(("and", terms[0][:n] + [rest]))


def clone(st):
    return {k: (dict(v) if isinstance(v, dict) else list(v) if isinstance(v, list) else v) for k, v in st.items()}


def fpeek(st, a):
    """st(0) without popping; a pending call result is pushed first."""
    if not st["fpu"]:
        st["fpu"].append(fpop(st, a))
    return st["fpu"][-1]


def fpop(st, a):
    if st["fpu"]:
        return st["fpu"].pop()
    v = st.get("fret")
    if v is None:
        raise DecodeError(f"x87 stack empty at {a:#x}")
    st["fret"] = None  # a call result is consumed once
    return v


def binop(op, x, y):
    if op == "r-":
        return ("arith", "-", y, x)
    if op == "r/":
        return ("arith", "/", y, x)
    return ("arith", op, x, y)


def expr_arg(v, a):
    if v is None:
        raise DecodeError(f"unknown call argument at {a:#x}")
    if v[0] == "iat":
        # the address of a .dsl global, passed by reference (`int &`)
        k = import_kind(v[1])
        if k == "int":
            return ("obj", plain_name(v[1]))
        if k == "intarray":
            return ("array", plain_name(v[1]))
        raise DecodeError(f"reference to {k} {plain_name(v[1])} passed at {a:#x}")
    if v[0] in ("obj", "objf", "elem", "const", "float", "str", "var", "call", "arith", "cmp", "not", "bool", "truth"):
        return v
    raise DecodeError(f"unsupported call argument {v[0]} at {a:#x}")


def typed_args(args, params):
    """A pushed 32-bit constant in a `float` parameter is the float's bits."""
    if params is None or len(params) != len(args):
        return args
    out = []
    for v, t in zip(args, params):
        if t == "M" and v[0] == "const":
            v = ("float", struct.unpack("<f", struct.pack("<I", v[1] & 0xFFFFFFFF))[0])
        out.append(v)
    return out


def branch_pred(mn, f, a):
    """The predicate under which the branch is taken."""
    if f[0] == "test":
        v = f[1]
        if v is None:
            raise DecodeError(f"test of an unknown value at {a:#x}")
        if mn == "je":
            return negate(as_bool(v))
        if mn == "jne":
            return as_bool(v)
        if mn in ("jl", "jge", "jle", "jg"):
            return ("cmp", JCC[mn], v, ("const", 0))
    if f[0] == "cmp":
        x, y = f[1], f[2]
        if x is None or y is None:
            raise DecodeError(f"compare of an unknown value at {a:#x}")
        return ("cmp", JCC[mn], x, y)
    if f[0] == "ftest":
        (st0, m), mask = f[1], f[2]
        if mask & ~0x45:
            raise DecodeError(f"x87 status mask {mask:#x} at {a:#x}")
        bits = []
        if mask & 0x01:
            bits.append(("cmp", "<", st0, m))
        if mask & 0x40:
            bits.append(("cmp", "==", st0, m))
        if not bits:
            raise DecodeError(f"x87 status mask {mask:#x} at {a:#x}")
        if len(bits) == 2:
            set_ = ("cmp", "<=", st0, m)
        else:
            set_ = bits[0]
        if mn == "je":
            return negate(set_)
        if mn == "jne":
            return set_
    raise DecodeError(f"unsupported branch `{mn}` after {f[0]} at {a:#x}")


def as_bool(v):
    if v[0] in ("cmp", "not", "and", "or", "truth"):
        return v
    if v[0] == "bool":
        return as_bool(v[1])
    if v[0] == "boolv":
        return v[1]
    if v[0] == "conflict":
        raise DecodeError("a value that differs between joined paths is used")
    if v[0] == "arith" and v[1] == "-":
        return ("cmp", "~=", v[2], v[3])  # `x - y` tested for zero
    return ("truth", v)


def negate(p):
    if p[0] == "cmp":
        return ("cmp", NEG[p[1]], p[2], p[3])
    if p[0] == "not":
        return p[1]
    return ("not", p)


def conj(preds):
    if not preds:
        return ("const", 1)
    return preds[0] if len(preds) == 1 else ("and", list(preds))


# ------------------------------------------------------------------ Lua


def lua_str(s):
    return "'" + s.replace("\\", "\\\\").replace("'", "\\'") + "'"


def lua_num(x):
    if isinstance(x, float):
        if x == int(x) and abs(x) < 1e15:
            return f"{int(x)}.0"
        return repr(x)
    return str(x)


def render(e, ctx="bool"):
    """Lua source of an expression; ctx is "bool" or "num"."""
    k = e[0]
    if k in ("cmp", "not", "and", "or", "truth"):
        if k == "cmp":
            s = f"{render(e[2], 'num')} {e[1]} {render(e[3], 'num')}"
        elif k == "not":
            s = f"not {render(e[1], 'bool')}" if e[1][0] in ("truth", "call") else f"not ({render(e[1], 'bool')})"
        elif k == "and":
            s = " and ".join(paren(x) for x in e[1])
        elif k == "or":
            s = " or ".join(paren(x) for x in e[1])
        else:
            s = f"T({render(e[1], 'arg' if e[1][0] == 'call' else 'num')})"
        return s if ctx == "bool" else f"N({s})"
    if k == "boolv":
        return render(e[1], ctx)
    if k == "const":
        return ("true" if e[1] else "false") if ctx == "bool" else lua_num(e[1])
    if k == "float":
        return lua_num(e[1])
    if k == "str":
        return lua_str(e[1])
    if k == "var":
        return f"S.{e[1]}" if ctx != "bool" else f"T(S.{e[1]})"
    if k in ("obj", "objf"):
        return f"O.{e[1]}" if ctx == "arg" else f"N(O.{e[1]})" if ctx == "num" else f"T(O.{e[1]})"
    if k == "elem":
        s = f"(O.{e[1]} or {{}})[{e[2] + 1}]"
        return s if ctx == "arg" else f"N({s})" if ctx == "num" else f"T({s})"
    if k == "array":
        return f"O.{e[1]}"
    if k == "call":
        args = ", ".join(render(x, "arg") for x in e[2])
        s = f"A({lua_str(e[1])}{', ' + args if args else ''})"
        return f"N({s})" if ctx == "num" else f"T({s})" if ctx == "bool" else s
    if k == "arith":
        s = f"({render(e[2], 'num')} {e[1]} {render(e[3], 'num')})"
        return s if ctx != "bool" else f"T({s})"
    if k == "bool":
        return render(e[1], ctx)
    raise DecodeError(f"cannot render {k}")


def paren(e):
    s = render(e, "bool")
    return f"({s})" if e[0] in ("and", "or") else s


def lua_ident(s):
    return s if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", s) else None


def scaffold(m):
    """The Lua scaffold of one decoded mission (dict from decode())."""
    L = []
    w = L.append
    w(f"-- {m['mission']}: rule scaffold generated by {TOOL} (LUA-07).")
    w(f"-- Rules decoded from {m['drl']}; actions, variable values and object handles")
    w(f"-- live in {m['dsl']} and are left for the hand port. Regenerate, do not edit")
    w("-- the generated tables. Not in the .drl: the engine's rule scheduling. Here each")
    w("-- rule is a named condition monitor checked every tick that fires on a")
    w("-- false-to-true edge; the stock engine re-evaluates a rule when one of its")
    w("-- dependencies is marked dirty (that policy is not reproduced).")
    w("")
    w("local function T(v) return v ~= nil and v ~= false and v ~= 0 end")
    w("local function N(v) if v == true then return 1 elseif type(v) ~= 'number' then return 0 end return v end")
    w("")
    w("local Mission = {")
    w(f"    name = {lua_str(m['mission'])},")
    w("    state = {},        -- the .dsl's rule variables, by name")
    w("    objects = {},      -- the .dsl's object handles, set by the ported setup")
    w("    timers = {},       -- running timer_int deadlines, by name")
    w("    rules = {},        -- the decoded rule table, in creation order")
    w("    ruleMonitors = {},")
    w("    missing = {},      -- engine calls the runtime does not bind yet: name -> count")
    w("    failed = {},       -- engine calls that raised an error: name -> count")
    w("    callFailed = false, -- set while a condition's engine call is unbound or failed")
    w("    time = 0,")
    w("}")
    w("")
    w("-- Engine calls of the conditions go to the A2 table by the ScriptInterface")
    w("-- method's own name. Unbound names and errors are counted, not raised, so an")
    w("-- unported mission still runs; a rule whose engine call gave no answer does")
    w("-- not hold that tick.")
    w("local function A(name, ...)")
    w("    local f = A2[name]")
    w("    if type(f) ~= 'function' then")
    w("        Mission.missing[name] = (Mission.missing[name] or 0) + 1")
    w("        Mission.callFailed = true")
    w("        return nil")
    w("    end")
    w("    local ok, v = pcall(f, ...)")
    w("    if not ok then")
    w("        Mission.failed[name] = (Mission.failed[name] or 0) + 1")
    w("        Mission.callFailed = true")
    w("        return nil")
    w("    end")
    w("    return v")
    w("end")
    w("")
    w(f"-- Variables ({len(m['variables'])}): kind, and whether the engine supplies it.")
    w("Mission.variables = {")
    for v in m["variables"]:
        extra = ", engine = true" if v.get("engine") else ""
        w(f"    {{ name = {lua_str(v['name'])}, kind = {lua_str(v['kind'])}{extra} }},")
    w("}")
    w(f"Mission.objectNames = {{ {', '.join(lua_str(o) for o in m['objects'])} }}")
    w("")
    w("-- Start values: the .dsl's setup assigns them; the hand port puts them here")
    w("-- (unlisted variables start at 0). The engine's ALWAYS_EVALUATE reads -1")
    w("-- (measured).")
    w("Mission.initial = {")
    for v in m["variables"]:
        if v.get("engine") and v["kind"] == "dirty_int":
            w(f"    {v['name']} = -1,")
    w("}")
    w("")
    w(f"-- Rules ({len(m['rules'])}), in the order the .drl creates them.")
    for i, r in enumerate(m["rules"], 1):
        w(f"Mission.rules[{i}] = {{")
        w(f"    name = {lua_str(r['name'])},")
        w(f"    action = {lua_str(r['action'])},")
        w(f"    deps = {{ {', '.join(lua_str(d) for d in r['deps'])} }},")
        if r.get("condition_lua") is not None:
            w(f"    when = function(S, O) return {r['condition_lua']} end,")
        else:
            w(f"    -- condition not decoded: {r['error']}")
            w("    when = function(S, O) return false end,")
        w("}")
    w("")
    w(f"-- Actions ({len(m['actions'])}): the .dsl functions the rules call. Port each body")
    w(f"-- from {m['dsl']}; until then it only records that it ran.")
    w("Mission.ran = {}")
    for a in m["actions"]:
        ident = lua_ident(a)
        if ident:
            w(f"function Mission:{ident}() self.ran[#self.ran + 1] = {lua_str(a)} end")
        else:
            w(f"Mission[{lua_str(a)}] = function(self) self.ran[#self.ran + 1] = {lua_str(a)} end")
    w("")
    w("-- timer_int: reads 1 once its time has run out (measured); here starting or")
    w("-- stopping a timer clears it.")
    w("function Mission:startTimer(name, seconds)")
    w("    self.timers[name] = self.time + seconds")
    w("    self.state[name] = 0")
    w("end")
    w("")
    w("function Mission:stopTimer(name)")
    w("    self.timers[name] = nil")
    w("    self.state[name] = 0")
    w("end")
    w("")
    w("function Mission:setup()")
    w("    for _, v in ipairs(self.variables) do")
    w("        if self.state[v.name] == nil then self.state[v.name] = self.initial[v.name] or 0 end")
    w("    end")
    w("    for i, rule in ipairs(self.rules) do")
    w("        local monitor = Monitor(rule.name, false, nil, function()")
    w("            self.callFailed = false")
    w("            local holds = rule.when(self.state, self.objects) == true")
    w("            return holds and not self.callFailed  -- no answer from the engine: does not hold")
    w("        end, nil)")
    w("        monitor:hook(rule.name, MonitorHook.Type.OnTrue, nil, function()")
    w("            self[rule.action](self)")
    w("        end, nil)")
    w("        self.ruleMonitors[i] = monitor")
    w("    end")
    w("end")
    w("")
    w("function Mission:update(time)")
    w("    self.time = time")
    w("    for _, v in ipairs(self.variables) do")
    w("        local at = self.timers[v.name]")
    w("        if at and time > at then")
    w("            self.timers[v.name] = nil")
    w("            self.state[v.name] = 1")
    w("        end")
    w("    end")
    w("end")
    w("")
    w("MMM.register(Mission)")
    return "\n".join(L) + "\n"


# ------------------------------------------------------------------ decode


def decode(path, engine=None):
    d = Drl(path, engine)
    if not any(plain_name(sym) == "g_pSI" for _, sym in d.pe.imports.values()) \
            and any(dll.lower() == "armada.exe" for dll in d.dlls):
        raise Unsupported("not an Armada II rule DLL (Armada I shape): not decoded by this tool")
    if d.pe.export_size:
        raise DecodeError("has an export table; rule DLLs have none")
    created, deps = d.rules()
    if not created:
        raise DecodeError("no create_rule calls found")
    imports = {s: dll for dll, s in d.pe.imports.values()}
    dsl = next((dll for dll in d.dlls if dll.lower().endswith(".dsl")), None)
    variables, objects, actions = [], [], []
    getters = set()
    for n in (engine or {}).get("ScriptInterfaceImp", []):
        p = plain_name(n)
        if p.startswith("ScriptGet_"):
            getters.add(p[len("ScriptGet_"):])
    for s, dll in imports.items():
        k = import_kind(s)
        p = plain_name(s)
        if k in VAR_KINDS:
            v = {"name": p, "kind": k}
            bare = p[2:] if p.startswith("p_") else p
            if bare in getters or not dll.lower().endswith(".dsl"):
                v["engine"] = True
            variables.append(v)
        elif k in ("int", "intarray") and dll.lower().endswith(".dsl"):
            objects.append(p)
    rules, seen, errors = [], {}, []
    for name, cond, act in created:
        try:
            body = d.entry(act)
        except DecodeError as e:
            raise DecodeError(f"{name}: action: {e}")
        target = action_target(d, body)
        if target not in actions:
            actions.append(target)
        rule = {"name": name, "action": target, "deps": deps.get(len(rules), [])}
        seen[name] = seen.get(name, 0) + 1
        if seen[name] > 1:
            rule["name"] = f"{name}#{seen[name]}"
        try:
            e = d.condition(cond)
            rule["condition"] = e
            rule["condition_lua"] = render(e)
        except DecodeError as ex:
            rule["condition_lua"] = None
            rule["error"] = str(ex)
            errors.append(f"{rule['name']}: {ex}")
        rules.append(rule)
    base = os.path.basename(path)
    mission = re.sub(r"[rR]\.drl$", "", base)
    calls = sorted({c for r in rules if r.get("condition") for c in calls_in(r["condition"])})
    # Engine calls written in the decoded conditions, keyed `Name/<argument count>`
    # (overloads differ in arity); an expression the decoder folded counts once.
    call_sites = {}
    for r in rules:
        if r.get("condition"):
            for k in call_shapes(r["condition"]):
                call_sites[k] = call_sites.get(k, 0) + 1
    used = {x for r in rules for x in r["deps"]} | {v for r in rules if r.get("condition") for v in vars_in(r["condition"])}
    variables.sort(key=lambda v: v["name"])
    objects.sort()
    return {
        "mission": mission,
        "drl": base,
        "dsl": dsl or f"{mission}S.dsl",
        "variables": variables,
        "objects": objects,
        "actions": actions,
        "rules": rules,
        "calls": calls,
        "call_sites": call_sites,
        "call_signatures": sorted(
            (d.engine.get("ScriptInterfaceImp") or [])[s] for s in d.engine_slots
            if s < len(d.engine.get("ScriptInterfaceImp") or [])
        ),
        "unused_variables": sorted(v["name"] for v in variables if v["name"] not in used),
        "errors": errors,
    }


def action_target(d, start):
    """The one `.dsl` function an action calls."""
    i = d.at[start]
    called = []
    while True:
        a, t = d.ins[i]
        if t.startswith("ret"):
            break
        m = re.fullmatch(r"call\s+(?:DWORD PTR )?ds:(0x[0-9a-f]+)", t)
        if m:
            s = d.symbol(int(m.group(1), 16))
            if not s:
                raise DecodeError(f"action calls a non-import at {a:#x}")
            called.append(plain_name(s[1]))
        elif re.fullmatch(r"call\s+0x([0-9a-f]+)", t):
            if int(t.split()[-1], 16) != d.chkesp:
                raise DecodeError(f"action calls code of its own at {a:#x}")
        elif t.startswith("call"):
            raise DecodeError(f"action makes an indirect call at {a:#x}")
        i += 1
    if len(called) != 1:
        raise DecodeError(f"action calls {len(called)} functions, expected 1")
    return called[0]


def calls_in(e):
    if not isinstance(e, tuple):
        return
    if e[0] == "call":
        yield e[1]
    for x in e[1:]:
        if isinstance(x, tuple):
            yield from calls_in(x)
        elif isinstance(x, list):
            for y in x:
                yield from calls_in(y)


def call_shapes(e):
    if not isinstance(e, tuple):
        return
    if e[0] == "call":
        yield f"{e[1]}/{len(e[2])}"
    for x in e[1:]:
        if isinstance(x, tuple):
            yield from call_shapes(x)
        elif isinstance(x, list):
            for y in x:
                yield from call_shapes(y)


def vars_in(e):
    if not isinstance(e, tuple):
        return
    if e[0] == "var":
        yield e[1]
    for x in e[1:]:
        if isinstance(x, tuple):
            yield from vars_in(x)
        elif isinstance(x, list):
            for y in x:
                yield from vars_in(y)


def find_drls(root):
    out = []
    for d, _, files in os.walk(root):
        for f in files:
            if f.lower().endswith("r.drl"):
                out.append(os.path.join(d, f))
    return sorted(out)


def inside(path, root):
    path, root = os.path.realpath(path), os.path.realpath(root)
    try:
        return os.path.commonpath([path, root]) == root
    except ValueError:  # different drives
        return False


def refuse_output(path, installs):
    """Why an output path is refused, or None. Output holds names read from
    game files: never in the repository (its build folders excepted, for the
    tests) and never in a game install."""
    repo = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
    builds = [os.path.join(repo, "target")]
    if os.environ.get("CARGO_TARGET_DIR"):
        builds.append(os.environ["CARGO_TARGET_DIR"])
    if inside(path, repo) and not any(inside(path, b) for b in builds):
        return "is inside the repository: output holds names read from game files"
    for root in installs:
        if inside(path, root):
            return f"is inside the install {root}: installs are read-only"
    return None


def install_subdirs(installs):
    """One output folder name per install, distinct even for equal basenames."""
    out, used = {}, set()
    for root in installs:
        base = re.sub(r"[^A-Za-z0-9_.-]+", "_", os.path.basename(os.path.normpath(root))) or "install"
        name, n = base, 1
        while name.lower() in used:
            n += 1
            name = f"{base}_{n}"
        used.add(name.lower())
        out[root] = name
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--install", action="append", required=True, help="game root to search for *R.drl (repeatable)")
    ap.add_argument("--out", help="folder for the Lua scaffolds (outside the repository)")
    ap.add_argument("--engine", help="Armada II root with Armada2.exe and armada2.map, to name engine calls")
    ap.add_argument("--only", action="append", help="mission name to convert (repeatable), e.g. a2_fed01")
    ap.add_argument("--json", help="write the decode report (names and counts) as JSON")
    a = ap.parse_args(argv)

    engine, engine_root = None, a.engine
    candidates = [a.engine] if a.engine else a.install
    for root in candidates:
        try:
            engine = engine_vtables(root)
        except (OSError, DecodeError):
            engine = None
        if engine:
            engine_root = root
            break
    if engine:
        bad = check_engine(engine)
        if bad:
            sys.exit("rule-engine slots differ from the ones this decoder was measured on:\n  " + "\n  ".join(bad))
        print(f"engine names: {engine_root}")
    else:
        print("engine names: none (no Armada2.exe + armada2.map); engine calls are written as slot_<n>")

    for flag, path in (("--out", a.out), ("--json", a.json)):
        if path:
            why = refuse_output(path, a.install)
            if why:
                sys.exit(f"{flag} {why}")
    if a.out:
        out = os.path.realpath(a.out)
        os.makedirs(out, exist_ok=True)
    subdirs = install_subdirs(a.install)

    report = {"installs": []}
    failed = skipped = 0
    for root in a.install:
        files = find_drls(root)
        entry = {"install": root, "files": []}
        report["installs"].append(entry)
        print(f"\n{root}: {len(files)} R.drl")
        for f in files:
            name = re.sub(r"[rR]\.drl$", "", os.path.basename(f))
            if a.only and name not in a.only:
                continue
            rel = os.path.relpath(f, root)
            try:
                m = decode(f, engine)
            except Unsupported as e:
                skipped += 1
                print(f"  SKIP {rel}: {e}")
                entry["files"].append({"file": rel, "skipped": str(e)})
                continue
            except DecodeError as e:
                failed += 1
                print(f"  FAIL {rel}: {e}")
                entry["files"].append({"file": rel, "error": str(e)})
                continue
            nd = sum(len(r["deps"]) for r in m["rules"])
            ok = sum(1 for r in m["rules"] if r.get("condition_lua") is not None)
            print(f"  {rel}: {len(m['rules'])} rules, {nd} dependencies, {ok}/{len(m['rules'])} conditions, "
                  f"{len(m['actions'])} actions, {len(m['variables'])} variables, {len(m['objects'])} handles, "
                  f"{len(m['calls'])} engine calls")
            for e in m["errors"]:
                print(f"    condition not decoded: {e}")
            if m["errors"]:
                failed += 1
            entry["files"].append({
                "file": rel,
                "mission": m["mission"],
                "rules": len(m["rules"]),
                "dependencies": nd,
                "conditions": ok,
                "actions": len(m["actions"]),
                "variables": len(m["variables"]),
                "handles": len(m["objects"]),
                "calls": m["calls"],
                "errors": m["errors"],
            })
            if a.out:
                sub = os.path.join(out, subdirs[root])
                os.makedirs(sub, exist_ok=True)
                with open(os.path.join(sub, m["mission"] + ".lua"), "w", encoding="utf-8", newline="\n") as fh:
                    fh.write(scaffold(m))
    total = sum(len(e["files"]) for e in report["installs"])
    print(f"\n{total} R.drl read, {total - failed - skipped} decoded, {failed} failed, {skipped} skipped")
    if a.json:
        with open(a.json, "w", encoding="utf-8") as fh:
            json.dump(report, fh, indent=1)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
