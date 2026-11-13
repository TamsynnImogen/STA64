#!/usr/bin/env python3
"""Convert Armada II legacy missions (`<name>S.dsl` + `<name>R.drl`) into Lua (LUA-11).

What it implements (evidence: read from the executables, static; notes are
kept outside the repository):

- The mission DLL (`S.dsl`, a debug build) exports the rule variables, the
  object handles, `g_pSI`, `p_WORLD`, `ScriptExtendInterface`, `ScriptInit`
  and one `void Name(void)` function per action. `ScriptExtendInterface`
  creates the variables by name through `ScriptInterface`; `ScriptInit` sets
  their start values and resolves the handles by map label. An action queues
  a named engine script (`p_WORLD->new_script(name)`) and adds its steps
  (`script::add_step`); the steps hold the mission's code: `ScriptInterface`
  calls, variable writes, timer starts and stops.
- The rule DLL (`R.drl`) registers each rule (condition, action, wake-up
  variables); `tools/drl_to_lua.py` decodes it.

The tool lifts every function the mission reaches (init, actions, steps and
their helpers) instruction by instruction into Lua: each basic block becomes a
labelled block, branches become `goto`, the compiler's stack slots and
registers that live across blocks become table entries. Anything it does not
recognise stops that one function, which is reported with the instruction
and left as a stub that records it ran; nothing is guessed.

The output, one `missions/<mission>.lua` per mission, carries a small rule and
script engine of its own (prelude below) with the engine's scheduling, and
calls the `A2` table by the `ScriptInterface` method's own name through a
guard that counts unbound and failing calls instead of raising.

Names and structure only reach the output, which is written to `--out`, a
folder outside the repository and outside every install; nothing read from a
game file is committed (docs/ARCHITECTURE.md, "Legal boundary"; D2: the DLLs
are read, never loaded).

usage:
  dsl_to_lua.py --install <A2 root> --out <dir> [--only a2_fed01] [--json r.json]
                [--engine <A2 root with Armada2.exe + armada2.map>]

Using the output: `<dir>/missions/<map>.lua` is found by the mission
resolver under the map's BZN stem (sta_mission::resolve), so `<dir>` works as
a VFS layer above the install (`Vfs::push_dir`, as the install-gated test
crates/sta_mission/tests/converted_legacy.rs does) or as a local mod folder;
the campaign book then lists the mission as Lua instead of legacy DLL.

Not reproduced (listed in each script's counters at run time): the engine's
own writes to its variables beyond the cineractive flags (team strength,
moons, warp attempts), steps waiting on fades / cineractives / audio, and the
native save/load callbacks (STA64 saves the sim, not the script).
Needs `objdump` (binutils) on PATH.
"""

import argparse
import json
import os
import re
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import drl_to_lua as drl  # noqa: E402

TOOL = "tools/dsl_to_lua.py"

DecodeError = drl.DecodeError
REG32 = ("eax", "ebx", "ecx", "edx", "esi", "edi")
REG8 = {"al": "eax", "bl": "ebx", "cl": "ecx", "dl": "edx", "ah": "eax", "ax": "eax", "cx": "ecx", "dx": "edx"}
JCC = {"je": "==", "jne": "~=", "jl": "<", "jge": ">=", "jle": "<=", "jg": ">",
       "jb": "<", "jae": ">=", "jbe": "<=", "ja": ">", "js": "<", "jns": ">="}
SETCC = {"sete": "==", "setne": "~=", "setl": "<", "setge": ">=", "setle": "<=", "setg": ">",
         "setb": "<", "setae": ">=", "setbe": "<=", "seta": ">"}
NEG = drl.NEG

# Engine slots the lifter names itself (read from the executable, static; checked
# against the user's Armada2.exe + armada2.map by drl.check_engine plus below).
CHARACTER_NEW_SCRIPT = 1  # character::new_script(char *) -> script *
SCRIPT_ADD_STEP = 0  # script::add_step(int (*)(script *))
IIIE_TRASH_RULE = (2, 3)  # IIIE::trash_rule(char *) / (rule *)
OBJECT_CLASSES = ("SiegeFleet", "MoveFleet", "AttackFleet")
# ScriptInterface methods the converted script implements itself.
INTERNAL = re.compile(r"ScriptCreate_|ScriptGet_|ScriptDestroy_|(Add|Remove)(To|From)(Load|Save)$")
VAR_SLOTS = {4: "is_dirty", 5: "set_clean", 6: "mark_dirty", 7: "set", 8: "get",
             9: "start", 10: "stop", 11: "update", 12: "time"}


def msvc_ret(symbol):
    """Return-type code of an MSVC member function name ('X' void, 'H' int,
    'M' float, '_N' bool, 'P...' pointer, '?AV...' struct by value), or None."""
    m = re.match(r"\?[A-Za-z_][A-Za-z0-9_]*@[A-Za-z_][A-Za-z0-9_]*@@[QU]A[EG](.*)$", symbol)
    if not m:
        return None
    s = m.group(1)
    if s.startswith("_N"):
        return "_N"
    if s.startswith("?A"):
        j = s.find("@@")
        return s[:j + 2] if j > 0 else s[:3]
    if s[:2] in ("PA", "PB", "AB", "AA"):
        if s[2] in "VU":
            return s[:s.index("@@") + 2]
        return s[:3]
    return s[:1]


def push_count(t):
    """Stack slots one parameter of MSVC type code t takes."""
    if t == "N":
        return 2
    if t.startswith("VVector3"):
        return 3
    if t in ("_J", "_K"):
        return 2
    return 1


# ------------------------------------------------------------------ the DLL


class Dsl:
    """The parts of a mission DLL the lifter reads."""

    def __init__(self, path, engine=None, pe=None, ins=None, exports=None):
        self.path = path
        self.pe = pe or drl.Pe(path)
        self.engine = engine or {}
        self.ins = ins if ins is not None else drl.disassemble(path)
        self.at = {a: i for i, (a, _) in enumerate(self.ins)}
        self.exports = exports if exports is not None else read_exports(self.pe)
        self.thunks = {}
        for a, t in self.ins:
            if t == "int3":
                continue
            m = re.match(r"jmp\s+0x([0-9a-f]+)$", t)
            if not m:
                break
            self.thunks[a] = int(m.group(1), 16)
        self.chkesp = chkesp(self.ins)
        # Exported data: address -> (plain name, kind, size, mangled)
        self.data = {}
        datas = sorted((a, n) for n, a in self.exports.items() if self.pe.section_of(a) != ".text")
        for i, (a, n) in enumerate(datas):
            size = (datas[i + 1][0] - a) if i + 1 < len(datas) else 4
            self.data[a] = (drl.plain_name(n), data_kind(n), min(size, 4096), n)
        self.code_names = {}
        for n, a in self.exports.items():
            if self.pe.section_of(a) == ".text":
                self.code_names[self.entry(a)] = drl.plain_name(n)
        self._ftol = {}

    def entry(self, addr):
        addr = self.thunks.get(addr, addr)
        if addr not in self.at:
            raise DecodeError(f"no instruction at {addr:#x}")
        return addr

    def si(self, slot):
        names = self.engine.get("ScriptInterfaceImp") or []
        return names[slot] if slot < len(names) else None

    def var_slot(self, kind, slot):
        names = self.engine.get(kind) or []
        return VAR_SLOTS.get(slot), (names[slot] if slot < len(names) else None)

    def global_at(self, addr):
        """(name, kind, offset) of the exported datum holding addr, or None."""
        if addr in self.data:
            n, k, _, _ = self.data[addr]
            return n, k, 0
        best = None
        for a, (n, k, size, _) in self.data.items():
            if a < addr < a + size and k in ("intarray", "Vector3", "floatarray"):
                best = (n, k, addr - a)
        return best

    def is_chkstk(self, addr):
        """The C runtime's stack probe (`_chkstk`), which large frames call
        instead of `sub esp,N`."""
        i = self.at.get(addr)
        if i is None:
            return False
        text = [t for _, t in self.ins[i:i + 16]]
        return any(t.startswith("cmp    eax,0x1000") for t in text) and "mov    esp,ecx" in text

    def is_ftol(self, addr):
        """The C runtime's float-to-int conversion (`_ftol`): x87 truncation
        to an integer through a changed control word."""
        if addr in self._ftol:
            return self._ftol[addr]
        i = self.at.get(addr)
        ok = False
        if i is not None:
            seen = []
            for _, t in self.ins[i:i + 24]:
                seen.append(t.split()[0])
                if t.startswith("ret"):
                    break
            ok = ("fnstcw" in seen or "fstcw" in seen) and "fistp" in seen and "fldcw" in seen
        self._ftol[addr] = ok
        return ok


def chkesp(ins):
    seen = {}
    for i, (_, t) in enumerate(ins[:-1]):
        if re.match(r"cmp\s+e(si|di|bp),esp$", t):
            m = re.match(r"call\s+0x([0-9a-f]+)$", ins[i + 1][1])
            if m:
                a = int(m.group(1), 16)
                seen[a] = seen.get(a, 0) + 1
    return max(seen, key=seen.get) if seen else None


def read_exports(pe):
    b = pe.data
    peo = struct.unpack_from("<I", b, 0x3C)[0]
    dd = peo + 24 + 96
    erva = struct.unpack_from("<I", b, dd)[0]
    if not erva:
        return {}
    o = pe.off(erva)
    _, nnames, eat, enpt, eord = struct.unpack_from("<IIIII", b, o + 20)
    out = {}
    for i in range(nnames):
        nm = pe.cstr(pe.base + struct.unpack_from("<I", b, pe.off(enpt) + 4 * i)[0])
        ordi = struct.unpack_from("<H", b, pe.off(eord) + 2 * i)[0]
        out[nm] = pe.base + struct.unpack_from("<I", b, pe.off(eat) + 4 * ordi)[0]
    return out


def data_kind(symbol):
    k = drl.import_kind(symbol)
    if k != "other":
        return k
    if symbol.endswith("@@3VVector3@@A"):
        return "Vector3"
    if re.search(r"@@3PAMA$", symbol):
        return "floatarray"
    if re.search(r"@@3_NA$", symbol):
        return "int"
    return "other"


# ------------------------------------------------------------------ lifting
#
# Expressions are tuples (drl_to_lua's shapes plus):
#   ("loc", off)        a stack slot of the function ([ebp+off])
#   ("reg", r)          a register value that lives across blocks
#   ("tmp", n)          the result of the n-th call of the function
#   ("glob", addr)      a global of the DLL that it does not export
#   ("param", i)        the function's i-th argument
#   ("varptr", name)    a rule variable (p_X); ("si",); ("world",); ("iiie",)
#   ("script", e)       a script object; ("code", addr) a function address
#   ("addr", lvalue)    the address of a stack slot or global
#   ("field", e, k)     component k (0, 1, 2) of a vector value e
# Statements:
#   ("set", lvalue, e) ("eval", e) ("var", name, op, args) ("goto", label)
#   ("if", pred, label) ("return", e) ("comment", text)


class Lifted:
    def __init__(self, addr):
        self.addr = addr
        self.blocks = []  # [(label addr, [stmts])]
        self.calls = []  # internal functions called
        self.steps = []  # (script name expr, step function addr)
        self.engine_calls = {}  # name -> count
        self.error = None
        self.instructions = 0
        self.params = 0
        self.loops = 0
        self.loctypes = {}


class Lifter:
    def __init__(self, dsl):
        self.d = dsl

    # ------------------------------------------------------------ CFG

    def switch_table(self, i):
        """Targets of a `switch` jump table: `cmp idx,N; ja default; ...;
        jmp [idx*4+table]` (the compiler bounds the index first)."""
        d = self.d
        a, t = d.ins[i]
        m = re.fullmatch(r"jmp\s+DWORD PTR \[(e\w\w)\*4\+(0x[0-9a-f]+)\]", t)
        if not m:
            raise DecodeError(f"indirect jump `{t}` at {a:#x}")
        n = None
        for _, u in reversed(d.ins[max(0, i - 8):i]):
            mm = re.fullmatch(r"cmp\s+.*,(0x[0-9a-f]+)", u)
            if mm:
                n = int(mm.group(1), 16)
                break
        if n is None or n > 256:
            raise DecodeError(f"jump table without a bound at {a:#x}")
        base = int(m.group(2), 16)
        out = []
        for k in range(n + 1):
            o = d.pe.off(base + 4 * k - d.pe.base)
            if o is None:
                raise DecodeError(f"jump table outside the file at {a:#x}")
            out.append(struct.unpack_from("<I", d.pe.data, o)[0])
        return out

    def blocks(self, start):
        d = self.d
        self.switches = {}
        todo, seen, leaders = [start], set(), {start}
        while todo:
            a = todo.pop()
            if a in seen:
                continue
            i = d.at.get(a)
            if i is None:
                raise DecodeError(f"jump outside the code at {a:#x}")
            while True:
                x, t = d.ins[i]
                seen.add(x)
                mn = t.split()[0]
                m = re.match(r"(j\w+)\s+0x([0-9a-f]+)$", t)
                if m:
                    tgt = int(m.group(2), 16)
                    leaders.add(tgt)
                    todo.append(tgt)
                    if mn == "jmp":
                        if i + 1 < len(d.ins):
                            leaders.add(d.ins[i + 1][0])
                        break
                    leaders.add(d.ins[i + 1][0])
                    todo.append(d.ins[i + 1][0])
                    break
                if mn == "jmp":
                    tbl = self.switch_table(i)
                    self.switches[x] = tbl
                    for tgt in tbl:
                        leaders.add(tgt)
                        todo.append(tgt)
                    if i + 1 < len(d.ins):
                        leaders.add(d.ins[i + 1][0])
                    break
                if mn.startswith("j"):
                    raise DecodeError(f"indirect jump `{t}` at {x:#x}")
                if mn == "ret":
                    break
                i += 1
                if i >= len(d.ins):
                    raise DecodeError("ran off the code")
                if d.ins[i][0] in leaders:
                    todo.append(d.ins[i][0])
                    break
        out = sorted(leaders & seen)
        return out, seen

    # ------------------------------------------------------------ lift

    def lift(self, start):
        d = self.d
        f = Lifted(start)
        self.cur = f
        try:
            leaders, seen = self.blocks(start)
            f.instructions = len(seen)
            lset = set(leaders)
            ntmp = [0]
            reg_in = {}
            ends = {}
            succs = {}
            for b in leaders:
                st = State()
                st.body = b != start
                if b == start:
                    st.r["ecx"] = ("this",)
                stmts = []
                i = d.at[b]
                first = True
                while True:
                    a, t = d.ins[i]
                    if not first and a in lset:
                        succs[b] = [a]
                        stmts_end = None
                        break
                    first = False
                    parts = t.split(None, 1)
                    mn, ops = parts[0], (parts[1] if len(parts) > 1 else "")
                    r = self.step(f, st, stmts, ntmp, a, mn, ops, i, start == b)
                    if r == "ret":
                        succs[b] = []
                        break
                    if isinstance(r, tuple) and r[0] == "switch":
                        stmts.append(("switch", r[1], r[2]))
                        succs[b] = list(r[2])
                        break
                    if isinstance(r, tuple):
                        kind, tgt, pred = r
                        if tgt <= a:
                            f.loops += 1
                        if kind == "jmp":
                            stmts.append(("goto", tgt))
                            succs[b] = [tgt]
                            break
                        stmts.append(("if", drl.simplify(pred), tgt))
                        nxt = d.ins[i + 1][0]
                        succs[b] = [tgt, nxt]
                        i += 1
                        if d.ins[i][0] in lset:
                            break
                        continue
                    i += 1
                if st.args:
                    raise DecodeError(f"arguments pushed across a block end at {a:#x}")
                if st.fpu:
                    raise DecodeError(f"x87 value live across a block end at {a:#x}")
                reg_in[b] = st.reg_in
                ends[b] = st
                f.blocks.append([b, stmts])
            # registers live across blocks: store them where a successor reads them
            for blk in f.blocks:
                b, stmts = blk
                st = ends[b]
                need = set()
                for s in succs.get(b, []):
                    need |= reg_in.get(s, set())
                stores = []
                for r in sorted(need):
                    v = st.r.get(r)
                    if v is None or v == ("reg", r):
                        continue
                    stores.append(("set", ("reg", r), v))
                if stores:
                    k = len(stmts)
                    while k > 0 and stmts[k - 1][0] in ("goto", "if", "return", "switch"):
                        k -= 1
                    stmts[k:k] = stores
        except DecodeError as e:
            f.error = str(e)
        return f

    def step(self, f, st, stmts, ntmp, a, mn, ops, i, entry):
        d = self.d
        r = st.r
        if mn in ("int3", "nop", "fnclex", "fwait", "wait", "cld"):
            return None
        if mn == "ret":
            stmts.append(("return", r.get("eax")))
            return "ret"
        if mn == "push":
            if ops == "ebp" or (ops in ("ebx", "esi", "edi") and not st.body):
                return None  # the frame and the saved registers of a prologue
            st.args.append(self.read(st, ops, a))
            return None
        if mn == "pop":
            if st.args:
                st.args.pop()
            elif ops in REG32 and st.body and not st.restored:
                st.restored = True  # `push ecx ... rep stos ... pop ecx`: the prologue keeps `this`
            elif ops in REG32:
                r[ops] = None
            return None
        if mn == "rep":
            if not st.body:
                st.body = True  # the debug prologue's 0xcc fill
                st.args = []
                return None
            raise DecodeError(f"`rep {ops}` at {a:#x}")
        if mn == "leave":
            return None
        if mn == "lea":
            dst, src = [x.strip() for x in ops.split(",", 1)]
            if dst == "edi" and not st.body:
                return None
            if dst in REG32:
                lv = self.lvalue(st, src, a)
                r[dst] = ("addr", lv)
                st.escaped.add(lv)
                return None
            raise DecodeError(f"`lea {ops}` at {a:#x}")
        if mn in ("sub", "add") and ops.startswith("esp,"):
            n = int(ops.split(",")[1], 16)
            if mn == "add":
                k = n // 4
                st.args = st.args[:-k] if k and k <= len(st.args) else ([] if k else st.args)
            elif i + 1 < len(d.ins) and d.ins[i + 1][1] == "push   ebx":
                st.body = False  # a frame prologue (also behind a patched entry)
            elif st.body:
                if n % 4:
                    raise DecodeError(f"`sub esp,{n:#x}` at {a:#x}")
                st.args.extend([("stackslot",)] * (n // 4))
                st.sub_esp = True
            return None
        if mn == "mov":
            dst, src = [x.strip() for x in ops.split(",", 1)]
            if dst in ("esi", "edi", "ebx") and src == "esp":
                st.marks[dst] = len(st.args)  # the debug stack check (nested calls use several)
                return None
            if dst in ("ebp", "esp"):
                return None
            if dst in REG32:
                if src == "esp" and st.sub_esp:
                    r[dst] = ("stacktop",)
                    return None
                r[dst] = self.read(st, src, a)
                return None
            if dst in ("al", "cl", "dl", "bl"):
                r[REG8[dst]] = self.read(st, src, a)
                return None
            self.store(f, st, stmts, dst, self.read(st, src, a), a)
            return None
        if mn in ("movsx", "movzx"):
            dst, src = [x.strip() for x in ops.split(",", 1)]
            src = re.sub(r"^(BYTE|WORD) PTR ", "", src)
            if dst in REG32:
                r[dst] = self.read(st, src, a)
                return None
            raise DecodeError(f"`{mn} {ops}` at {a:#x}")
        if mn == "call":
            return self.call(f, st, stmts, ntmp, a, ops, i)
        if mn == "test":
            x, y = [s.strip() for s in ops.split(",", 1)]
            if x == y or (x in ("al", "eax") and y in ("al", "eax")):
                st.flags = ("test", self.read(st, x, a))
                return None
            if x == "ah" and st.fsw:
                st.flags = ("ftest", st.fsw, int(y, 16))
                return None
            if re.fullmatch(r"0x[0-9a-f]+", y):
                st.flags = ("test", ("arith", "&", self.read(st, x, a), ("const", int(y, 16))))
                return None
            raise DecodeError(f"`test {ops}` at {a:#x}")
        if mn == "cmp":
            x, y = [s.strip() for s in ops.split(",", 1)]
            if y == "esp":
                return None
            st.flags = ("cmp", self.read(st, x, a), self.read(st, y, a))
            return None
        if mn in JCC:
            if st.flags is None:
                raise DecodeError(f"branch without a compare at {a:#x}")
            return ("jcc", int(ops, 16), self.branch(mn, st.flags, a))
        if mn == "jmp":
            if a in self.switches:
                m = re.fullmatch(r"DWORD PTR \[(e\w\w)\*4\+0x[0-9a-f]+\]", ops)
                return ("switch", self.read(st, m.group(1), a), self.switches[a])
            return ("jmp", int(ops, 16), None)
        if mn in SETCC:
            dst = REG8.get(ops.strip())
            if not dst or not st.flags:
                raise DecodeError(f"`{mn} {ops}` at {a:#x}")
            r[dst] = self.branch("j" + mn[3:], st.flags, a)
            return None
        if mn in ("xor", "and", "or", "add", "sub", "imul", "shl", "sar", "shr", "adc", "sbb"):
            return self.arith(f, st, stmts, a, mn, ops)
        if mn in ("inc", "dec", "neg", "not"):
            x = ops.strip()
            v = self.read(st, x, a)
            if mn == "inc":
                nv = ("arith", "+", v, ("const", 1))
            elif mn == "dec":
                nv = ("arith", "-", v, ("const", 1))
            elif mn == "neg":
                nv = ("neg", v)
            else:
                nv = ("bnot", v)
            st.flags = ("test", nv)
            self.write(f, st, stmts, x, nv, a)
            return None
        if mn == "cdq":
            r["edx"] = ("sign", r.get("eax"))
            return None
        if mn in ("idiv", "div"):
            x = self.read(st, ops, a)
            n = r.get("eax")
            r["eax"] = ("arith", "//", n, x)
            r["edx"] = ("arith", "%", n, x)
            return None
        # x87
        return self.x87(f, st, stmts, a, mn, ops)

    # ------------------------------------------------------------ operands

    def read(self, st, op, a):
        d = self.d
        op = op.strip()
        if op in REG32:
            v = st.r.get(op, "unset")
            if v == "unset":
                st.reg_in.add(op)
                return ("reg", op)
            return v
        if op in REG8:
            reg = REG8[op]
            v = st.r.get(reg, "unset")
            if v == "unset":
                st.reg_in.add(reg)
                return ("reg", reg)
            return v
        m = re.fullmatch(r"(0x[0-9a-f]+)", op)
        if m:
            return d_immediate(d, int(m.group(1), 16))
        m = re.fullmatch(r"-?0x[0-9a-f]+", op)
        if m:
            return ("const", int(op, 16))
        if re.fullmatch(r"\d+", op):
            return ("const", int(op))
        lv = self.lvalue(st, op, a)
        return self.load(st, lv, a)

    def lvalue(self, st, op, a):
        """A memory operand as a location."""
        d = self.d
        op = re.sub(r"^(?:DWORD|BYTE|WORD|QWORD) PTR ", "", op.strip())
        m = re.fullmatch(r"ds:(0x[0-9a-f]+)", op)
        if m:
            return self.glob(int(m.group(1), 16), a)
        m = re.fullmatch(r"\[ebp([+-]0x[0-9a-f]+)\]", op)
        if m:
            off = int(m.group(1), 16)
            if off >= 8:
                return ("param", (off - 8) // 4)
            return ("loc", off)
        m = re.fullmatch(r"\[(e\w\w)\]", op)
        if m:
            return ("deref", self.read(st, m.group(1), a), 0)
        m = re.fullmatch(r"\[(e\w\w)([+-]0x[0-9a-f]+)\]", op)
        if m:
            return ("deref", self.read(st, m.group(1), a), int(m.group(2), 16))
        m = re.fullmatch(r"\[(e\w\w)\*4\+(0x[0-9a-f]+)\]", op)
        if m:
            base = self.glob(int(m.group(2), 16), a)
            if base[0] in ("obj", "elem"):
                name = base[1]
                k = base[2] if base[0] == "elem" else ("const", 0)
                idx = self.read(st, m.group(1), a)
                if k != ("const", 0):
                    idx = ("arith", "+", idx, k)
                return ("elem", name, idx)
            raise DecodeError(f"indexed access to {base} at {a:#x}")
        m = re.fullmatch(r"\[ebp\+(e\w\w)\*4([+-]0x[0-9a-f]+)\]", op)
        if m:
            return ("locidx", int(m.group(2), 16), self.read(st, m.group(1), a))
        m = re.fullmatch(r"\[(e\w\w)\+(e\w\w)\*4\]", op)
        if m:
            raise DecodeError(f"indexed access `{op}` at {a:#x}")
        raise DecodeError(f"operand `{op}` at {a:#x}")

    def glob(self, addr, a):
        d = self.d
        g = d.global_at(addr)
        if g:
            name, kind, off = g
            if kind in ("intarray", "floatarray"):
                return ("elem", name, ("const", off // 4))
            if kind == "Vector3":
                return ("vfield", name, off // 4)
            return ("obj", name) if off == 0 else ("elem", name, ("const", off // 4))
        sec = d.pe.section_of(addr)
        if sec in (".data", ".rdata", ".bss", ".idata") or sec is None:
            return ("gvar", addr)
        raise DecodeError(f"global in {sec} at {addr:#x} ({a:#x})")

    def load(self, st, lv, a):
        """The value read from a location."""
        d = self.d
        k = lv[0]
        if k == "obj":
            name = lv[1]
            g = next(v for v in d.data.values() if v[0] == name)
            kind = g[1]
            if kind in drl.VAR_KINDS:
                return ("varptr", name, kind)
            if kind == "ScriptInterface":
                return ("si",)
            if kind == "character":
                return ("world",)
            if kind in ("int", "float"):
                return ("obj", name)
            raise DecodeError(f"read of {kind} {name} at {a:#x}")
        if k == "locidx":
            return lv
        if k in ("elem", "vfield", "gvar", "param"):
            if k == "param" and lv in st.mem:
                return st.mem[lv]
            if k == "gvar" and lv in st.mem:
                return st.mem[lv]
            return lv if k != "gvar" else ("glob", lv[1])
        if k == "loc":
            if lv in st.mem:
                return st.mem[lv]
            if lv[1] in self.cur.loctypes:
                return ("objref", lv, self.cur.loctypes[lv[1]])
            return lv
        if k == "deref":
            base, off = lv[1], lv[2]
            if base is None:
                raise DecodeError(f"load through an unknown pointer at {a:#x}")
            if off == 0 and base[0] in ("varptr", "si", "world", "iiie", "script", "objref"):
                return ("vt", base)
            if base[0] == "vt":
                return ("method", base[1], off // 4)
            if base[0] == "addr":
                inner = base[1]
                if off == 0:
                    return self.load(st, inner, a)
                if inner[0] == "loc":
                    return self.load(st, ("loc", inner[1] + off), a)
                if inner[0] == "obj":
                    return self.load(st, self.glob_off(inner, off, a), a)
            if base[0] in ("tmp", "loc", "reg", "glob", "obj", "field") and off in (0, 4, 8):
                # a vector the engine returned by reference
                return ("field", base, off // 4)
            raise DecodeError(f"load [{base[0]}+{off:#x}] at {a:#x}")
        raise DecodeError(f"load of {k} at {a:#x}")

    def glob_off(self, lv, off, a):
        name = lv[1]
        addr = next(x for x, v in self.d.data.items() if v[0] == name)
        return self.glob(addr + off, a)

    def store(self, f, st, stmts, dst, v, a):
        m = re.fullmatch(r"(?:DWORD|QWORD|BYTE|WORD) PTR \[esp(?:\+(0x[0-9a-f]+))?\]", dst.strip())
        if m:
            # an argument slot reserved by `push reg` / `sub esp` and filled in place
            k = int(m.group(1), 16) // 4 if m.group(1) else 0
            if k >= len(st.args):
                raise DecodeError(f"store to an unreserved stack slot at {a:#x}")
            st.args[len(st.args) - 1 - k] = v
            return
        lv = self.lvalue(st, dst, a)
        if lv[0] == "deref":
            base, off = lv[1], lv[2]
            if base and base[0] == "stacktop":
                # a by-value struct argument built in place
                idx = len(st.args) - 1 - off // 4
                if 0 <= idx < len(st.args):
                    st.args[idx] = v
                    return
            if base and base[0] == "addr":
                inner = base[1]
                if inner[0] == "loc":
                    lv = ("loc", inner[1] + off)
                elif inner[0] == "obj" and off:
                    lv = self.glob_off(inner, off, a)
                else:
                    lv = inner
            else:
                raise DecodeError(f"store through {base and base[0]} at {a:#x}")
        if lv[0] == "obj":
            g = next(x for x in self.d.data.values() if x[0] == lv[1])
            if g[1] in drl.VAR_KINDS:
                stmts.append(("bindvar", lv[1], g[1], v))
                return
            if g[1] in ("ScriptInterface", "character"):
                stmts.append(("comment", f"{lv[1]} is set by the engine"))
                return
            if g[1] not in ("int", "float"):
                raise DecodeError(f"store to {g[1]} {lv[1]} at {a:#x}")
        if lv[0] == "locidx":
            for x in list(st.mem):
                if x[0] == "loc":
                    del st.mem[x]
        if lv[0] == "loc" and v is not None and v[0] == "objref":
            f.loctypes[lv[1]] = v[2]
        if lv[0] in ("loc", "param", "gvar"):
            st.mem[lv] = v if v is not None and v[0] in ("const", "float", "str", "code", "obj", "addr",
                                                             "varptr", "si", "world", "tmp", "script", "objref") else lv_value(lv)
        if v is None:
            raise DecodeError(f"store of an unknown value at {a:#x}")
        if v[0] == "const" and self.is_float_slot(lv):
            v = ("float", struct.unpack("<f", struct.pack("<I", v[1] & 0xFFFFFFFF))[0])
        stmts.append(("set", lv, v))

    def is_float_slot(self, lv):
        """A global the compiler stores a float into as raw bits."""
        if lv[0] == "vfield":
            return True
        if lv[0] in ("obj", "elem"):
            g = next((x for x in self.d.data.values() if x[0] == lv[1]), None)
            return bool(g) and g[1] in ("float", "floatarray")
        return False

    def write(self, f, st, stmts, x, v, a):
        if x in REG32:
            st.r[x] = v
        elif x in REG8:
            st.r[REG8[x]] = v
        else:
            self.store(f, st, stmts, x, v, a)

    def arith(self, f, st, stmts, a, mn, ops):
        x, y = [s.strip() for s in ops.split(",", 1)]
        if mn == "xor" and x == y:
            self.write(f, st, stmts, x, ("const", 0), a)
            return None
        if mn == "imul" and "," in y:
            y1, y2 = [s.strip() for s in y.split(",", 1)]
            v = ("arith", "*", self.read(st, y1, a), self.read(st, y2, a))
            self.write(f, st, stmts, x, v, a)
            return None
        vx, vy = self.read(st, x, a), self.read(st, y, a)
        if mn == "and" and vy == ("const", 0xff):
            v = vx
        elif mn == "and" and x == "esp":
            return None
        else:
            op = {"xor": "~", "and": "&", "or": "|", "add": "+", "sub": "-", "imul": "*",
                  "shl": "<<", "sar": ">>", "shr": ">>>", "adc": "+c", "sbb": "-c"}[mn]
            if mn == "sbb" and x == y:
                v = ("sbb", st.flags)
            elif mn == "adc":
                raise DecodeError(f"`adc {ops}` at {a:#x}")
            else:
                v = ("arith", op, vx, vy)
        st.flags = ("cmp", vx, vy) if mn == "sub" else ("test", v)
        self.write(f, st, stmts, x, v, a)
        return None

    def branch(self, mn, flags, a):
        if flags[0] == "test":
            v = flags[1]
            if v is None:
                raise DecodeError(f"test of an unknown value at {a:#x}")
            if mn == "je":
                return drl.negate(truth(v))
            if mn == "jne":
                return truth(v)
            if mn in ("js", "jl"):
                return ("cmp", "<", v, ("const", 0))
            if mn in ("jns", "jge"):
                return ("cmp", ">=", v, ("const", 0))
            if mn == "jle":
                return ("cmp", "<=", v, ("const", 0))
            if mn == "jg":
                return ("cmp", ">", v, ("const", 0))
        if flags[0] == "cmp":
            x, y = flags[1], flags[2]
            if x is None or y is None:
                raise DecodeError(f"compare of an unknown value at {a:#x}")
            if mn in ("js", "jns"):
                return ("cmp", JCC[mn], ("arith", "-", x, y), ("const", 0))
            return ("cmp", JCC[mn], x, y)
        if flags[0] == "ftest":
            return drl.branch_pred(mn, flags, a)
        raise DecodeError(f"`{mn}` after {flags[0]} at {a:#x}")

    # ------------------------------------------------------------ calls

    def take(self, st, n, a):
        if n > len(st.args):
            raise DecodeError(f"call takes {n} arguments, {len(st.args)} pushed at {a:#x}")
        args = st.args[len(st.args) - n:] if n else []
        st.args = st.args[:len(st.args) - n]
        if not st.args:
            st.sub_esp = False
        return list(reversed(args))

    def new_tmp(self, ntmp):
        ntmp[0] += 1
        return ("tmp", ntmp[0])

    def clobber(self, st):
        for x in ("eax", "ecx", "edx"):
            st.r[x] = None
        # a call may write through addresses the function handed out, and globals
        for lv in list(st.mem):
            if lv in st.escaped or lv[0] == "gvar":
                del st.mem[lv]

    def call(self, f, st, stmts, ntmp, a, ops, i):
        d = self.d
        m = re.fullmatch(r"0x([0-9a-f]+)", ops)
        if m:
            tgt = int(m.group(1), 16)
            if tgt == d.chkesp:
                return None
            body = d.entry(tgt)
            if d.is_chkstk(body):
                st.body = False  # `mov eax,N; call _chkstk`: a large frame's prologue
                return None
            if d.is_ftol(body):
                v = self.fpop(st, a)
                st.r["eax"] = ("ftol", v)
                st.r["edx"] = None
                return None
            n = 0
            nxt = d.ins[i + 1][1] if i + 1 < len(d.ins) else ""
            mm = re.fullmatch(r"add\s+esp,(0x[0-9a-f]+)", nxt)
            if mm:
                n = int(mm.group(1), 16) // 4
                if n > len(st.args):
                    raise DecodeError(f"internal call with {n} arguments, {len(st.args)} pushed at {a:#x}")
                args = list(reversed(st.args[len(st.args) - n:]))
                # the following `add esp` drops them
            else:
                args = []
            if body not in f.calls:
                f.calls.append(body)
            t = self.new_tmp(ntmp)
            stmts.append(("set", t, ("fcall", body, [self.arg(x, a) for x in args])))
            self.clobber(st)
            st.r["eax"] = t
            st.fret = t
            return None
        tgt = self.read(st, ops, a)
        if not tgt or tgt[0] != "method":
            raise DecodeError(f"indirect call `{ops}` ({tgt and tgt[0]}) at {a:#x}")
        obj, slot = tgt[1], tgt[2]
        if obj[0] == "si":
            name = d.si(slot)
            if name is None:
                raise DecodeError(f"ScriptInterface slot {slot} unknown at {a:#x}")
            plain = drl.plain_name(name)
            params = drl.msvc_params(name)
            if params is None:
                raise DecodeError(f"cannot read the signature of {plain} at {a:#x}")
            ret = msvc_ret(name)
            hidden = 1 if ret and ret.startswith("?A") else 0
            n = sum(push_count(p) for p in params) + hidden
            raw = self.take(st, n, a)
            if hidden:
                raw = raw[1:]
            args = self.typed(raw, params, plain, a)
            f.engine_calls[plain] = f.engine_calls.get(plain, 0) + 1
            call = ("call", plain, args)
            special = self.special(plain, args, a, ret)
            if ret == "X":
                stmts.append(special or ("eval", call))
                self.clobber(st)
                return None
            t = self.new_tmp(ntmp)
            stmts.append(("set", t, special or call))
            self.clobber(st)
            cls = re.fullmatch(r"PAV(\w+)@@", ret or "")
            st.r["eax"] = ("objref", t, cls.group(1)) if cls and cls.group(1) in OBJECT_CLASSES else t
            st.fret = t if ret == "M" else None
            return None
        if obj[0] == "varptr":
            name, kind = obj[1], obj[2]
            op, _ = d.var_slot(kind, slot)
            if op is None:
                raise DecodeError(f"{kind} slot {slot} at {a:#x}")
            nargs = 1 if op in ("set", "start", "update") else 0
            args = self.take(st, nargs, a)
            if op in ("start", "update") and args[0] and args[0][0] == "const":
                args[0] = ("float", struct.unpack("<f", struct.pack("<I", args[0][1] & 0xFFFFFFFF))[0])
            if kind == "hot_float" and op == "set" and args[0] and args[0][0] == "const":
                args[0] = ("float", struct.unpack("<f", struct.pack("<I", args[0][1] & 0xFFFFFFFF))[0])
            if op == "get":
                v = ("var", name, kind)
                self.clobber(st)
                st.r["eax"] = v
                st.fret = v if kind == "hot_float" else None
                return None
            if op in ("time", "is_dirty"):
                t = self.new_tmp(ntmp)
                stmts.append(("set", t, ("varq", name, op)))
                self.clobber(st)
                st.r["eax"] = t
                st.fret = t
                return None
            stmts.append(("var", name, op, [self.arg(x, a) for x in args]))
            self.clobber(st)
            return None
        if obj[0] == "world":
            if slot == CHARACTER_NEW_SCRIPT:
                (nm,) = self.take(st, 1, a)
                t = self.new_tmp(ntmp)
                stmts.append(("set", t, ("newscript", self.arg(nm, a))))
                self.clobber(st)
                st.r["eax"] = ("script", t)
                return None
            if slot == drl.CHARACTER_GET_IIIE:
                self.clobber(st)
                st.r["eax"] = ("iiie",)
                return None
        if obj[0] == "objref":
            cls = obj[2]
            names = d.engine.get(cls) or []
            if slot >= len(names):
                raise DecodeError(f"{cls} slot {slot} unknown at {a:#x}")
            name = names[slot]
            plain = drl.plain_name(name)
            params = drl.msvc_params(name)
            if params is None:
                raise DecodeError(f"cannot read the signature of {cls}::{plain} at {a:#x}")
            n = sum(push_count(p) for p in params)
            raw = self.take(st, n, a)
            args = self.typed(raw, params, f"{cls}::{plain}", a)
            key = f"{cls}.{plain}"
            f.engine_calls[key] = f.engine_calls.get(key, 0) + 1
            call = ("mcall", obj[1], cls, plain, args)
            ret = msvc_ret(name)
            self.clobber(st)
            if ret == "X":
                stmts.append(("eval", call))
                return None
            t = self.new_tmp(ntmp)
            stmts.append(("set", t, call))
            st.r["eax"] = t
            st.fret = t if ret == "M" else None
            return None
        if obj[0] == "iiie" and slot in IIIE_TRASH_RULE:
            (nm,) = self.take(st, 1, a)
            stmts.append(("trashrule", self.arg(nm, a)))
            self.clobber(st)
            return None
        if obj[0] == "script" and slot == SCRIPT_ADD_STEP:
            (fn,) = self.take(st, 1, a)
            if not fn or fn[0] != "code":
                raise DecodeError(f"add_step with {fn} at {a:#x}")
            body = d.entry(fn[1])
            f.steps.append(body)
            stmts.append(("addstep", obj[1], body))
            self.clobber(st)
            return None
        raise DecodeError(f"call on {obj[0]} slot {slot} at {a:#x}")

    def typed(self, raw, params, what, a):
        """Call arguments by the method's parameter types: a pushed constant in
        a `float` is the float's bits; a `Vector3` by value is three slots."""
        args, k = [], 0
        for p in params:
            c = push_count(p)
            if c == 1:
                v = raw[k]
                if p == "M" and v and v[0] == "const":
                    v = ("float", struct.unpack("<f", struct.pack("<I", v[1] & 0xFFFFFFFF))[0])
                args.append(self.arg(v, a))
            elif p.startswith("VVector3"):
                args.append(("vec", [self.arg(x, a) for x in raw[k:k + 3]]))
            else:
                raise DecodeError(f"{what}: parameter type {p} at {a:#x}")
            k += c
        return args

    def special(self, plain, args, a, ret=None):
        """Engine calls the converted script implements itself."""
        if plain.startswith("ScriptCreate_"):
            return ("newvar", plain[len("ScriptCreate_"):], args[0])
        if plain.startswith("ScriptGet_"):
            kind = re.fullmatch(r"PAV(\w+)@@", ret or "")
            return ("enginevar", plain[len("ScriptGet_"):], kind.group(1) if kind else "hot_int")
        if plain in ("AddToLoad", "AddToSave", "RemoveFromLoad", "RemoveFromSave"):
            return ("comment", f"{plain}: native save/load callback, not used")
        if plain.startswith("ScriptDestroy_"):
            return ("comment", f"{plain}: native cleanup, not used")
        return None

    def arg(self, v, a):
        if v is None:
            raise DecodeError(f"unknown call argument at {a:#x}")
        if v[0] in ("stackslot", "stacktop"):
            raise DecodeError(f"struct argument not filled at {a:#x}")
        return v

    # ------------------------------------------------------------ x87

    def fpop(self, st, a):
        if st.fpu:
            return st.fpu.pop()
        if st.fret is not None:
            v, st.fret = st.fret, None
            return v
        raise DecodeError(f"x87 stack empty at {a:#x}")

    def fread(self, st, op, a, integer=False):
        d = self.d
        m = re.fullmatch(r"(DWORD|QWORD) PTR ds:(0x[0-9a-f]+)", op)
        if m:
            addr = int(m.group(2), 16)
            g = d.global_at(addr)
            if g or d.pe.section_of(addr) in (".data", ".bss"):
                return self.read(st, op, a)
            if integer:
                raise DecodeError(f"integer load from {addr:#x} at {a:#x}")
            return ("float", d.pe.f32(addr) if m.group(1) == "DWORD" else d.pe.f64(addr))
        m = re.fullmatch(r"st\((\d)\)", op)
        if m:
            k = int(m.group(1))
            if not st.fpu and st.fret is not None:
                st.fpu.append(self.fpop(st, a))
            if k >= len(st.fpu):
                raise DecodeError(f"x87 st({k}) is empty at {a:#x}")
            return st.fpu[-1 - k]
        if op == "st":
            return self.fread(st, "st(0)", a)
        return self.read(st, op, a)

    def x87(self, f, st, stmts, a, mn, ops):
        if mn in ("fld", "fild"):
            st.fpu.append(self.fread(st, ops, a, integer=mn == "fild"))
            return None
        if mn in ("fld1", "fldz"):
            st.fpu.append(("float", 1.0 if mn == "fld1" else 0.0))
            return None
        if mn in ("fstp", "fst", "fistp", "fist"):
            v = self.fpop(st, a) if mn.endswith("p") else (st.fpu[-1] if st.fpu else st.fret)
            if mn.startswith("fi"):
                v = ("ftol", v)
            if ops.startswith("st("):
                return None
            self.store(f, st, stmts, ops, v, a)
            return None
        if mn in ("fadd", "fsub", "fmul", "fdiv", "fsubr", "fdivr", "faddp", "fsubp", "fmulp", "fdivp",
                  "fsubrp", "fdivrp", "fiadd", "fisub", "fimul", "fidiv"):
            base = mn.rstrip("p") if mn.endswith("rp") or mn in ("faddp", "fsubp", "fmulp", "fdivp") else mn
            base = base.replace("fi", "f")
            op = {"fadd": "+", "fsub": "-", "fmul": "*", "fdiv": "/", "fsubr": "r-", "fdivr": "r/"}[base]
            if ops in ("", "st(1),st") or mn.endswith("p"):
                y = self.fpop(st, a)
                x = self.fpop(st, a)
                # st(1) = st(1) op st(0), pop
                v = fbinop(op, x, y)
                st.fpu.append(v)
                return None
            top = self.fpop(st, a)
            if ops in ("st(0),st", "st,st(0)", "st(0),st(0)", "st,st"):
                v = fbinop(op, top, top)
            else:
                v = fbinop(op, top, self.fread(st, ops, a))
            st.fpu.append(v)
            return None
        if mn == "fchs":
            st.fpu.append(("neg", self.fpop(st, a)))
            return None
        if mn == "fabs":
            st.fpu.append(("abs", self.fpop(st, a)))
            return None
        if mn == "fsqrt":
            st.fpu.append(("sqrt", self.fpop(st, a)))
            return None
        if mn == "fxch":
            if len(st.fpu) < 2:
                if st.fret is not None:
                    st.fpu.insert(0, self.fpop(st, a))
            if len(st.fpu) < 2:
                raise DecodeError(f"fxch on a short x87 stack at {a:#x}")
            st.fpu[-1], st.fpu[-2] = st.fpu[-2], st.fpu[-1]
            return None
        if mn in ("fcomp", "fcom", "fcompp", "fucomp", "fucompp", "ficomp"):
            if mn in ("fcompp", "fucompp"):
                top = self.fpop(st, a)
                other = self.fpop(st, a)
            else:
                other = self.fread(st, ops, a) if ops else None
                top = self.fpop(st, a) if mn != "fcom" else (st.fpu[-1] if st.fpu else self.fpop(st, a))
                if other is None:
                    other = self.fpop(st, a)
            st.fcmp = (top, other)
            return None
        if mn == "fnstsw":
            if not st.fcmp:
                raise DecodeError(f"fnstsw without a compare at {a:#x}")
            st.fsw = st.fcmp
            st.r["eax"] = None
            return None
        raise DecodeError(f"unsupported instruction `{mn} {ops}` at {a:#x}")


class State:
    def __init__(self):
        self.r = {}
        self.args = []
        self.marks = {}
        self.fpu = []
        self.fret = None
        self.flags = None
        self.fsw = None
        self.fcmp = None
        self.body = False
        self.sub_esp = False
        self.mem = {}
        self.escaped = set()
        self.reg_in = set()
        self.restored = False


def lv_value(lv):
    return ("glob", lv[1]) if lv[0] == "gvar" else lv


def d_immediate(d, n):
    sec = d.pe.section_of(n)
    if sec in (".rdata", ".data"):
        try:
            s = d.pe.cstr(n)
            if s and all(32 <= ord(c) < 127 or c in "\n\t" for c in s):
                return ("str", s)
        except DecodeError:
            pass
        g = d.global_at(n)
        if g:
            return ("addr", ("obj", g[0]) if g[2] == 0 else ("elem", g[0], ("const", g[2] // 4)))
        return ("addr", ("gvar", n))
    if sec == ".bss":
        g = d.global_at(n)
        if g:
            return ("addr", ("obj", g[0]))
        return ("addr", ("gvar", n))
    if sec == ".text":
        return ("code", n)
    return ("const", n - (1 << 32) if n >= 1 << 31 else n)


def fbinop(op, x, y):
    """x87 arithmetic: floating point, no 32-bit wrap."""
    v = drl.binop(op, x, y)
    return ("farith",) + v[1:]


def truth(v):
    if v[0] in ("cmp", "not", "and", "or", "truth"):
        return v
    return ("truth", v)


# ------------------------------------------------------------------ mission


def lift_mission(dsl):
    """Every function a mission reaches, from its exports."""
    lf = Lifter(dsl)
    out = {}
    roots = []
    for n, a in dsl.exports.items():
        if dsl.pe.section_of(a) != ".text":
            continue
        roots.append((drl.plain_name(n), dsl.entry(a)))
    roles = {}
    for name, a in roots:
        if name in ("ScriptExtendInterface", "ScriptInit"):
            roles[a] = name
        else:
            roles[a] = "action"
    todo = [a for _, a in sorted(roots)]
    while todo:
        a = todo.pop(0)
        if a in out:
            continue
        f = lf.lift(a)
        out[a] = f
        for s in f.steps:
            roles.setdefault(s, "step")
            todo.append(s)
        for c in f.calls:
            roles.setdefault(c, "helper")
            todo.append(c)
    return out, roles


# ------------------------------------------------------------------ Lua


def lua_str(s):
    return drl.lua_str(s)


class Namer:
    def __init__(self, dsl, funcs, roles):
        self.d = dsl
        self.names = {}
        for a, n in dsl.code_names.items():
            self.names[a] = n
        steps = {}
        for a, f in sorted(funcs.items()):
            owner = self.names.get(a)
            if owner and f.steps:
                for k, s in enumerate(f.steps, 1):
                    if s not in self.names:
                        self.names[s] = f"{owner}_step{k}" if len(f.steps) > 1 else f"{owner}_step"
        for a in sorted(funcs):
            if a not in self.names:
                self.names[a] = f"{roles.get(a, 'fn')}_{a - dsl.pe.base:x}"

    def fn(self, a):
        n = self.names.get(a) or f"fn_{a - self.d.pe.base:x}"
        return n if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", n) else f"fn_{a - self.d.pe.base:x}"


def render_expr(e, nm, ctx="num"):
    """Lua source of an expression; ctx is "num" (a number), "bool" or "arg"."""
    k = e[0]
    if k in ("cmp", "not", "and", "or", "truth"):
        if k == "cmp":
            s = f"{render_expr(e[2], nm, 'num')} {e[1]} {render_expr(e[3], nm, 'num')}"
        elif k == "not":
            s = f"not ({render_expr(e[1], nm, 'bool')})"
        elif k in ("and", "or"):
            s = f" {k} ".join(f"({render_expr(x, nm, 'bool')})" for x in e[1])
        else:
            s = f"T({render_expr(e[1], nm, 'arg')})"
        return s if ctx == "bool" else f"N({s})"
    if k == "const":
        return ("true" if e[1] else "false") if ctx == "bool" else drl.lua_num(e[1])
    if k == "float":
        s = drl.lua_num(e[1])
        return s if ctx != "bool" else f"T({s})"
    if k == "str":
        return lua_str(e[1])
    if k == "var":
        s = f"S.{e[1]}"
    elif k == "obj":
        s = f"O.{e[1]}"
    elif k == "elem":
        s = f"O.{e[1]}[{render_expr(e[2], nm)}]"
    elif k == "vfield":
        s = f"O.{e[1]}[{e[2] + 1}]"
    elif k == "loc":
        s = f"L[{e[1]}]"
    elif k == "reg":
        s = f"R.{e[1]}"
    elif k == "tmp":
        s = f"t[{e[1]}]"
    elif k == "glob":
        s = f"G[{e[1] - 0:#x}]"
    elif k == "param":
        s = f"P[{e[1] + 1}]"
    elif k == "call":
        args = "".join(", " + render_expr(x, nm, "arg") for x in e[2])
        s = f"A({lua_str(e[1])}{args})"
    elif k == "fcall":
        args = ", ".join(render_expr(x, nm, "arg") for x in e[2])
        s = f"M.{nm.fn(e[1])}({args})"
    elif k == "arith":
        op = e[1]
        x, y = render_expr(e[2], nm), render_expr(e[3], nm)
        if op in ("+", "-", "*"):
            s = f"I32({x} {op} {y})"
        elif op == "/":
            s = f"({x} / {y})"
        elif op == "//":
            s = f"IDIV({x}, {y})"
        elif op == "%":
            s = f"IMOD({x}, {y})"
        elif op == "&":
            s = f"(I({x}) & I({y}))"
        elif op == "|":
            s = f"(I({x}) | I({y}))"
        elif op == "~":
            s = f"(I({x}) ~ I({y}))"
        elif op == "<<":
            s = f"I32(I({x}) << I({y}))"
        elif op == ">>":
            s = f"SAR({x}, {y})"
        elif op == ">>>":
            s = f"((I({x}) & 0xffffffff) >> I({y}))"
        else:
            raise DecodeError(f"cannot render arith {op}")
    elif k == "farith":
        s = f"({render_expr(e[2], nm)} {e[1]} {render_expr(e[3], nm)})"
    elif k == "neg":
        s = f"(-{render_expr(e[1], nm)})"
    elif k == "bnot":
        s = f"(~I({render_expr(e[1], nm)}))"
    elif k == "abs":
        s = f"math.abs({render_expr(e[1], nm)})"
    elif k == "sqrt":
        s = f"math.sqrt({render_expr(e[1], nm)})"
    elif k == "ftol":
        s = f"FTOL({render_expr(e[1], nm)})"
    elif k == "sign":
        s = f"(N({render_expr(e[1], nm)}) < 0 and -1 or 0)"
    elif k == "sbb":
        fl = e[1]
        if fl and fl[0] == "test" and fl[1] and fl[1][0] == "neg":
            s = f"(T({render_expr(fl[1][1], nm, 'arg')}) and -1 or 0)"
        elif fl and fl[0] == "cmp":
            s = f"(ULT({render_expr(fl[1], nm)}, {render_expr(fl[2], nm)}) and -1 or 0)"
        else:
            raise DecodeError("cannot render sbb")
    elif k == "mcall":
        args = "".join(", " + render_expr(x, nm, "arg") for x in e[4])
        s = f"FM({render_expr(e[1], nm, 'arg')}, {lua_str(e[2])}, {lua_str(e[3])}{args})"
    elif k == "objref":
        s = render_expr(e[1], nm, "arg")
    elif k == "locidx":
        s = f"L[{e[1]} + 4 * I({render_expr(e[2], nm)})]"
    elif k == "code":
        s = f"M.{nm.fn(nm.d.entry(e[1]))}"
    elif k == "addr":
        lv = e[1]
        s = f"Ref({lua_str(render_lv(lv, nm))})"
    elif k == "varptr":
        s = f"V.{e[1]}"
    elif k == "script":
        s = render_expr(e[1], nm)
    elif k == "field":
        s = f"VEC({render_expr(e[1], nm, 'arg')}, {e[2] + 1})"
    elif k == "vec":
        s = "Vector(" + ", ".join(render_expr(x, nm) for x in e[1]) + ")"
    elif k == "newvar":
        s = f"NewVar({lua_str(e[1])}, {render_expr(e[2], nm, 'arg')})"
    elif k == "enginevar":
        s = f"EngineVar({lua_str(e[1])}, {lua_str(e[2])})"
    elif k == "varq":
        s = f"VarQuery({lua_str(e[1])}, {lua_str(e[2])})"
    elif k == "newscript":
        s = f"NewScript({render_expr(e[1], nm, 'arg')})"
    elif k == "this":
        s = "P.this"
    elif k in ("si", "world", "iiie"):
        s = f"{k.upper()}"
    else:
        raise DecodeError(f"cannot render {k}")
    if ctx == "bool":
        return f"T({s})"
    if ctx == "num" and k in ("call", "tmp", "var", "obj", "reg", "loc", "glob", "param", "elem", "fcall", "field",
                              "vfield", "locidx", "mcall", "objref"):
        return f"N({s})"
    return s


def render_lv(lv, nm):
    k = lv[0]
    if k == "obj":
        return f"O.{lv[1]}"
    if k == "elem":
        return f"O.{lv[1]}[{render_expr(lv[2], nm)}]"
    if k == "vfield":
        return f"O.{lv[1]}[{lv[2] + 1}]"
    if k == "loc":
        return f"L[{lv[1]}]"
    if k == "locidx":
        return f"L[{lv[1]} + 4 * I({render_expr(lv[2], nm)})]"
    if k == "gvar":
        return f"G[{lv[1]:#x}]"
    if k == "param":
        return f"P[{lv[1] + 1}]"
    if k == "reg":
        return f"R.{lv[1]}"
    if k == "tmp":
        return f"t[{lv[1]}]"
    raise DecodeError(f"cannot assign to {k}")


def inline_temps(blocks):
    """`t[n] = <call>` used once, by the next statement of the same block,
    is written in place of that use."""
    uses = {}

    def count(e):
        if isinstance(e, tuple):
            if e and e[0] == "tmp":
                uses[e[1]] = uses.get(e[1], 0) + 1
                return
            for x in e[1:]:
                count(x)
        elif isinstance(e, list):
            for x in e:
                count(x)

    for _, stmts in blocks:
        for s in stmts:
            if s[0] == "set":
                count(s[2])
                if s[1][0] != "tmp":
                    count(s[1])
            else:
                for x in s[1:]:
                    count(x)

    def subst(e, n, v):
        if isinstance(e, tuple):
            if e == ("tmp", n):
                return v
            return tuple(subst(x, n, v) for x in e)
        if isinstance(e, list):
            return [subst(x, n, v) for x in e]
        return e

    for blk in blocks:
        stmts = blk[1]
        out = []
        for s in stmts:
            out.append(s)
            while len(out) >= 2:
                prev, cur = out[-2], out[-1]
                if not (prev[0] == "set" and prev[1][0] == "tmp" and uses.get(prev[1][1]) == 1):
                    break
                n = prev[1][1]
                if prev[2][0] in ("newscript",):
                    break
                new = subst(cur, n, prev[2])
                if new == cur:
                    break
                out[-2:] = [new]
        blk[1] = out


def render_function(f, nm, name):
    L = []
    w = L.append
    if f.error:
        w(f"function M.{name}(...)")
        w(f"    M.notDecoded({lua_str(name)}, {lua_str(f.error)})")
        w("    return 0")
        w("end")
        return L
    blocks = [[b, list(s)] for b, s in f.blocks]
    inline_temps(blocks)
    used_labels = set()
    for _, stmts in blocks:
        for s in stmts:
            if s[0] in ("goto", "if"):
                used_labels.add(s[-1])
            if s[0] == "switch":
                used_labels.update(s[2])
    w(f"function M.{name}(...)")
    w("    local P, L, R, t = {...}, {}, {}, {}")
    for b, stmts in blocks:
        if b in used_labels:
            w(f"    ::b{b - nm.d.pe.base:x}::")
        for s in stmts:
            w("    " + render_stmt(s, nm))
    w("end")
    return L


def render_stmt(s, nm):
    k = s[0]
    if k == "set":
        return f"{render_lv(s[1], nm)} = {render_expr(s[2], nm, 'arg')}"
    if k == "eval":
        return render_expr(s[1], nm, "arg")
    if k == "bindvar":
        return f"BindVar({lua_str(s[1])}, {lua_str(s[2])}, {render_expr(s[3], nm, 'arg')})"
    if k == "var":
        name, op, args = s[1], s[2], s[3]
        a = "".join(", " + render_expr(x, nm) for x in args)
        return f"VarOp({lua_str(name)}, {lua_str(op)}{a})"
    if k == "goto":
        return f"goto b{s[1] - nm.d.pe.base:x}"
    if k == "if":
        return f"if {render_expr(s[1], nm, 'bool')} then goto b{s[2] - nm.d.pe.base:x} end"
    if k == "return":
        v = s[1]
        return "do return end" if v is None else f"do return {render_expr(v, nm, 'arg')} end"
    if k == "comment":
        return f"-- {s[1]}"
    if k == "switch":
        idx = render_expr(s[1], nm)
        cases = " ".join(f"if X == {i} then goto b{t - nm.d.pe.base:x} end" for i, t in enumerate(s[2]))
        return f"do local X = I({idx}) {cases} end"
    if k == "addstep":
        return f"AddStep({render_expr(s[1], nm, 'arg')}, M.{nm.fn(s[2])}, {lua_str(nm.fn(s[2]))})"
    if k == "trashrule":
        return f"TrashRule({render_expr(s[1], nm, 'arg')})"
    raise DecodeError(f"cannot render statement {k}")


PRELUDE = r"""
-- ---------------------------------------------------------------- runtime
-- The engine's rule and script scheduling, written for the converted script
-- (read from the Armada II executable, static; see tools/dsl_to_lua.py).

local function T(v) return v ~= nil and v ~= false and v ~= 0 end
local function N(v) if v == true then return 1 elseif type(v) ~= 'number' then return 0 end return v end
local function I(v) v = N(v) if v >= 0 then return math.floor(v) end return -math.floor(-v) end
local function I32(v) v = N(v) if math.type(v) == 'float' then return v end v = v & 0xffffffff if v >= 0x80000000 then v = v - 0x100000000 end return v end
local function FTOL(v) return I(v) end
local function SAR(x, n) x = I(x) n = I(n) if x >= 0 then return x >> n end return ~((~x) >> n) end
local function IDIV(x, y) x, y = I(x), I(y) if y == 0 then return 0 end local q = x // y if q < 0 and q * y ~= x then q = q + 1 end return q end
local function IMOD(x, y) x, y = I(x), I(y) if y == 0 then return 0 end return x - IDIV(x, y) * y end
local function VEC(v, k) if type(v) ~= 'table' and type(v) ~= 'userdata' then return 0 end local ok, r = pcall(function() return ({v.x, v.y, v.z})[k] end) return ok and r or 0 end
local function Ref(name) return { ref = name } end
local function ULT(x, y) return math.ult(I(x) & 0xffffffff, I(y) & 0xffffffff) end

local Mission = Mission_
local M = {}
local V, S, O, G = Mission.vars, Mission.state, Mission.objects, Mission.globals

local ENGINE_WRITES = {}
local function A(name, ...)
    local f = A2 and A2[name]
    Mission.calls[name] = (Mission.calls[name] or 0) + 1
    if ENGINE_WRITES[name] then ENGINE_WRITES[name]() end
    if type(f) ~= 'function' then
        Mission.missing[name] = (Mission.missing[name] or 0) + 1
        Mission.callFailed = true
        return nil
    end
    local ok, v = pcall(f, ...)
    if not ok then
        Mission.failed[name] = (Mission.failed[name] or 0) + 1
        Mission.lastError = tostring(v)
        Mission.callFailed = true
        return nil
    end
    return v
end

-- Methods of engine objects a ScriptInterface call returned (fleets).
local function FM(obj, cls, name, ...)
    local key = cls .. '.' .. name
    Mission.calls[key] = (Mission.calls[key] or 0) + 1
    local f = type(obj) == 'table' and obj[name] or nil
    if type(f) ~= 'function' then
        Mission.missing[key] = (Mission.missing[key] or 0) + 1
        Mission.callFailed = true
        return nil
    end
    local ok, v = pcall(f, obj, ...)
    if not ok then
        Mission.failed[key] = (Mission.failed[key] or 0) + 1
        Mission.callFailed = true
        return nil
    end
    return v
end

function M.notDecoded(name, why)
    Mission.notDecoded[name] = (Mission.notDecoded[name] or 0) + 1
    Mission.ran[#Mission.ran + 1] = name .. ' (not decoded)'
end

-- Variables (hot_int, hot_float, dirty_int, timer_int). A write that changes
-- the value marks the variable dirty and puts it on the affected list; once a
-- tick, before the rules run, each affected variable wakes the rules that
-- depend on it and is set clean. A dirty_int never becomes clean (the
-- engine's ALWAYS_EVALUATE is one), so its rules are evaluated every tick.
-- A timer reads 1 when it runs out; setting it clean turns 1 into 2 and any
-- other non-zero value into 0 and keeps it affected, so a rule reading the
-- timer sees it true for exactly one evaluation.
local START = { hot_int = -1, hot_float = -1.0, dirty_int = -1, timer_int = 0 }
local function addAffected(var)
    if not var.listed then
        var.listed = true
        Mission.affected[#Mission.affected + 1] = var
    end
end
local function NewVar(kind, name)
    local var = { kind = kind, name = name, value = START[kind] or 0, deps = {}, remaining = 0, dirty = false }
    if kind == 'dirty_int' then var.dirty = true addAffected(var) end
    return var
end
-- Variables the engine owns (ALWAYS_EVALUATE, isCineractiveCancel/Done and
-- the per-team AI values): one per game, with the engine's start values; the
-- engine's own writes to them (cineractive cancel, team strength) are not
-- reproduced here.
local function EngineVar(name, kind)
    Mission.engineVars[name] = Mission.engineVars[name] or NewVar(kind or 'hot_int', name)
    return Mission.engineVars[name]
end
local function BindVar(p, kind, var)
    if type(var) ~= 'table' then var = NewVar(kind, p) end
    var.p = p
    V[p] = var
    S[p] = var.value
end
local function varOf(p)
    local var = V[p]
    if not var then var = NewVar('hot_int', p) BindVar(p, 'hot_int', var) end
    return var
end
local function setValue(var, value)
    if var.value ~= value then
        var.value = value
        S[var.p] = value
        if not var.dirty then
            var.dirty = true
            addAffected(var)
        end
    end
end
local function stopTimer(var)
    if var.remaining > 0 then
        for i, x in ipairs(Mission.timers) do
            if x == var then table.remove(Mission.timers, i) break end
        end
    end
    var.remaining = 0
end
local function VarOp(p, op, x)
    local var = varOf(p)
    if op == 'set' then
        if var.kind ~= 'hot_float' then x = I(x) else x = N(x) end
        setValue(var, x)
    elseif op == 'start' then
        x = N(x)
        if x < 0.001 then
            stopTimer(var)
            setValue(var, 1)
        else
            if var.remaining < 0.001 then Mission.timers[#Mission.timers + 1] = var end
            var.remaining = x
        end
    elseif op == 'stop' then
        stopTimer(var)
    elseif op == 'mark_dirty' then
        if not var.dirty then var.dirty = true addAffected(var) end
    elseif op == 'set_clean' then
        var.dirty = var.kind == 'dirty_int'
    end
end
-- While a cineractive runs, the engine's cineractive update writes
-- isCineractiveDone (its shots finished) and isCineractiveCancel (every
-- player skipped) each frame; with no skip input and shots running both read
-- 0 from the first frame (read from the executable, static).
ENGINE_WRITES.CineractiveStart = function()
    for _, n in ipairs({ 'isCineractiveCancel', 'isCineractiveDone' }) do
        local var = Mission.engineVars[n]
        if var then setValue(var, 0) end
    end
end

local function VarQuery(p, q)
    local var = varOf(p)
    if q == 'time' then return var.remaining end
    if q == 'is_dirty' then return (var.dirty or var.kind == 'dirty_int') and 1 or 0 end
    return 0
end

-- Scripts: an action queues a named script on the world (same names run side
-- by side); scripts run after the rules, oldest first, one step per tick each;
-- a step's return value is not used; a script whose steps have all run is
-- removed. Steps that wait on the engine (fades, cineractives, audio, movies)
-- are not held here.
local function NewScript(name)
    local s = { name = name, steps = {}, names = {}, at = 1 }
    Mission.scripts[#Mission.scripts + 1] = s
    return s
end
local function AddStep(s, fn, name)
    if type(s) ~= 'table' then return end
    s.steps[#s.steps + 1] = fn
    s.names[#s.names + 1] = name
end
local function TrashRule(name)
    for _, rule in ipairs(Mission.rules) do
        if rule.name == name then rule.trashed = true end
    end
    for i = #Mission.satisfied, 1, -1 do
        if Mission.satisfied[i].trashed then table.remove(Mission.satisfied, i) end
    end
end
"""

ENGINE = r"""
-- ---------------------------------------------------------------- engine
-- One engine tick: timers, affected variables, rules, scripts.
function Mission:runScripts()
    local i = 1
    while i <= #self.scripts do
        local s = self.scripts[i]
        local fn = s.steps[s.at]
        if fn then
            local name = s.names[s.at]
            s.at = s.at + 1
            self.stepCount = self.stepCount + 1
            if #self.ran < 2000 then self.ran[#self.ran + 1] = name end
            self.lastStep = name
            local ok, err = pcall(fn, s)
            if not ok then self.stepErrors[#self.stepErrors + 1] = name .. ': ' .. tostring(err) end
        end
        if s.steps[s.at] then
            i = i + 1
        else
            table.remove(self.scripts, i)
        end
    end
end

function Mission:runRules()
    -- affected variables wake their rules, then are set clean
    local woken, order = {}, {}
    local list = self.affected
    self.affected = {}
    for _, var in ipairs(list) do
        var.listed = false
        for _, rule in ipairs(var.deps) do
            if not woken[rule] then woken[rule] = true order[#order + 1] = rule end
        end
        if var.kind == 'timer_int' then
            if var.value == 1 then var.value = 2 elseif var.value ~= 0 then var.value = 0 else var.dirty = false end
            S[var.p] = var.value
        elseif var.kind ~= 'dirty_int' then
            var.dirty = false
        end
        if var.dirty or var.kind == 'dirty_int' then addAffected(var) end
    end
    -- a woken rule enters or leaves the satisfied list when its result changes
    for _, rule in ipairs(order) do
        if not rule.trashed then
            self.callFailed = false
            local ok, holds = pcall(rule.when, S, O)
            local r = (ok and holds == true and not self.callFailed) and 1 or 0
            if not ok then self.conditionErrors[rule.name] = tostring(holds) end
            if r ~= rule.last then
                rule.last = r
                if r == 1 then
                    table.insert(self.satisfied, 1, rule)
                    self.fired[#self.fired + 1] = rule.name
                    self.lastRule = rule.name
                else
                    for i, x in ipairs(self.satisfied) do
                        if x == rule then table.remove(self.satisfied, i) break end
                    end
                end
            end
        end
    end
    -- every satisfied rule fires every tick, newest first, until a later
    -- evaluation finds it false
    local fire = {}
    for i, rule in ipairs(self.satisfied) do fire[i] = rule end
    for _, rule in ipairs(fire) do
        if not rule.trashed then
            self.fireCount[rule.name] = (self.fireCount[rule.name] or 0) + 1
            local action = M[rule.action]
            if action then
                local ok, err = pcall(action)
                if not ok then self.stepErrors[#self.stepErrors + 1] = rule.action .. ': ' .. tostring(err) end
            else
                self.notDecoded[rule.action] = (self.notDecoded[rule.action] or 0) + 1
            end
        end
    end
end

function Mission:setup()
    -- load order of the engine: ScriptExtendInterface, the rule DLL's
    -- registration, then ScriptInit before the first tick
    if M.ScriptExtendInterface then M.ScriptExtendInterface(SI, 0x33) end
    for _, rule in ipairs(self.rules) do
        rule.last = 0
        for _, p in ipairs(rule.deps) do
            local var = varOf(p)
            var.deps[#var.deps + 1] = rule
        end
    end
    if M.ScriptInit then M.ScriptInit() end
end

function Mission:update(time)
    local dt = time - (self.time or time)
    self.time = time
    local running = {}
    for i, var in ipairs(self.timers) do running[i] = var end
    for _, var in ipairs(running) do
        if var.remaining >= 0.001 then
            var.remaining = var.remaining - dt
            if var.remaining < 0.001 then
                setValue(var, 1)
                for i, x in ipairs(self.timers) do
                    if x == var then table.remove(self.timers, i) break end
                end
            end
        end
    end
    self:runRules()
    self:runScripts()
end

MMM.register(Mission)
"""


def convert(dsl_path, drl_path, engine):
    """Lua source and a report for one mission."""
    d = Dsl(dsl_path, engine)
    funcs, roles = lift_mission(d)
    rules = drl.decode(drl_path, engine) if drl_path else None
    nm = Namer(d, funcs, roles)
    mission = re.sub(r"[sS]\.dsl$", "", os.path.basename(dsl_path))
    L = []
    w = L.append
    w(f"-- {mission}: converted by {TOOL} (LUA-11) from {os.path.basename(dsl_path)}"
      + (f" and {os.path.basename(drl_path)}" if drl_path else "") + ".")
    w("-- Generated on the user's machine from the user's install; not for redistribution.")
    w("-- Engine calls go to A2 by the ScriptInterface method's name; unbound or failing")
    w("-- calls are counted in Mission.missing / Mission.failed and answer nil.")
    w("")
    w("Mission_ = {")
    w(f"    name = {lua_str(mission)},")
    w("    vars = {}, state = {}, objects = {}, globals = {}, timers = {}, scripts = {},")
    w("    rules = {}, affected = {}, satisfied = {}, fired = {}, fireCount = {}, ran = {},")
    w("    calls = {}, missing = {}, failed = {}, notDecoded = {}, stepErrors = {},")
    w("    conditionErrors = {}, engineVars = {}, stepCount = 0, callFailed = false,")
    w("}")
    w(PRELUDE)
    # handles and arrays
    for a, (n, k, size, _) in sorted(d.data.items()):
        if k in ("intarray", "floatarray", "Vector3"):
            w(f"O.{n} = {{}}")
    w("")
    w("-- ---------------------------------------------------------------- functions")
    order = sorted(funcs, key=lambda a: (["ScriptExtendInterface", "ScriptInit", "action", "step", "helper"]
                                         .index(roles.get(a, "helper")) if roles.get(a) in
                                         ("ScriptExtendInterface", "ScriptInit", "action", "step", "helper")
                                         else 9, a))
    for a in order:
        f = funcs[a]
        w(f"-- {roles.get(a, 'helper')}: {nm.fn(a)}" + (f"  (not decoded: {f.error})" if f.error else ""))
        try:
            L.extend(render_function(f, nm, nm.fn(a)))
        except DecodeError as e:
            f.error = f"render: {e}"
            L.extend(render_function(f, nm, nm.fn(a)))
        w("")
    if rules:
        w("-- ---------------------------------------------------------------- rules")
        w(f"-- {len(rules['rules'])} rules from {rules['drl']}, in creation order.")
        for r in rules["rules"]:
            w("Mission.rules[#Mission.rules + 1] = {")
            w(f"    name = {lua_str(r['name'])},")
            w(f"    action = {lua_str(r['action'])},")
            w(f"    deps = {{ {', '.join(lua_str(x) for x in r['deps'])} }},")
            if r.get("condition_lua") is not None:
                w(f"    when = function(S, O) return {r['condition_lua']} end,")
            else:
                w(f"    -- condition not decoded: {r['error']}")
                w("    when = function(S, O) return false end,")
            w("}")
        w("")
        # the rule conditions use drl_to_lua's guard name
    report = mission_report(mission, funcs, roles, nm, rules)
    names = sorted(n for n in set(report["engine_calls"]) | set(rules["calls"] if rules else [])
                   if not INTERNAL.match(n))
    w("-- Engine calls the converted code makes (bodies and rule conditions).")
    w("Mission.engineNames = { " + ", ".join(lua_str(n) for n in names) + " }")
    w(ENGINE)
    src = "\n".join(L) + "\n"
    return mission, src, report


def mission_report(mission, funcs, roles, nm, rules):
    by_role = {}
    errors = []
    engine = {}
    for a, f in funcs.items():
        role = roles.get(a, "helper")
        r = by_role.setdefault(role, {"total": 0, "decoded": 0})
        r["total"] += 1
        if f.error:
            errors.append({"function": nm.fn(a), "role": role, "error": f.error})
        else:
            r["decoded"] += 1
            for k, v in f.engine_calls.items():
                engine[k] = engine.get(k, 0) + v
    return {
        "mission": mission,
        "functions": by_role,
        "errors": errors,
        "loops": sum(1 for f in funcs.values() if f.loops),
        "engine_calls": dict(sorted(engine.items())),
        "rules": len(rules["rules"]) if rules else 0,
        "rule_conditions_decoded": sum(1 for r in rules["rules"] if r.get("condition_lua") is not None) if rules else 0,
    }


def find_missions(root):
    out = []
    for d, _, files in os.walk(root):
        low = {f.lower(): f for f in files}
        for f in files:
            if f.lower().endswith("s.dsl"):
                stem = f[:-5]
                drlf = low.get((stem + "r.drl").lower())
                out.append((os.path.join(d, f), os.path.join(d, drlf) if drlf else None))
    return sorted(out)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--install", action="append", required=True, help="game root holding missions/*S.dsl")
    ap.add_argument("--out", help="output folder (outside the repository and every install)")
    ap.add_argument("--engine", help="Armada II root with Armada2.exe and armada2.map")
    ap.add_argument("--only", action="append", help="mission to convert, e.g. a2_fed01 (repeatable)")
    ap.add_argument("--json", help="write the conversion report as JSON")
    a = ap.parse_args(argv)
    engine = None
    for root in ([a.engine] if a.engine else a.install):
        try:
            engine = drl.engine_vtables(root, drl.ENGINE_CLASSES + OBJECT_CLASSES)
        except (OSError, DecodeError):
            engine = None
        if engine:
            break
    if not engine:
        sys.exit("no Armada2.exe + armada2.map found: pass --engine <Armada II root>")
    bad = drl.check_engine(engine) + check_engine(engine)
    if bad:
        sys.exit("engine slots differ from the ones this converter was read from:\n  " + "\n  ".join(bad))
    for flag, path in (("--out", a.out), ("--json", a.json)):
        if path:
            why = drl.refuse_output(path, a.install + ([a.engine] if a.engine else []))
            if why:
                sys.exit(f"{flag} {why}")
    report = {"missions": []}
    failed = 0
    for root in a.install:
        for dsl_path, drl_path in find_missions(root):
            mission = re.sub(r"[sS]\.dsl$", "", os.path.basename(dsl_path))
            if a.only and mission.lower() not in [x.lower() for x in a.only]:
                continue
            try:
                mission, src, rep = convert(dsl_path, drl_path, engine)
            except (DecodeError, drl.Unsupported) as e:
                failed += 1
                print(f"  FAIL {mission}: {e}")
                report["missions"].append({"mission": mission, "error": str(e)})
                continue
            fr = rep["functions"]
            tot = sum(v["total"] for v in fr.values())
            dec = sum(v["decoded"] for v in fr.values())
            print(f"  {mission}: {dec}/{tot} functions decoded "
                  + ", ".join(f"{k} {v['decoded']}/{v['total']}" for k, v in sorted(fr.items()))
                  + f"; rules {rep['rule_conditions_decoded']}/{rep['rules']}")
            report["missions"].append(rep)
            if a.out:
                out = os.path.join(os.path.realpath(a.out), "missions")
                os.makedirs(out, exist_ok=True)
                with open(os.path.join(out, mission.lower() + ".lua"), "w", encoding="utf-8", newline="\n") as fh:
                    fh.write(src)
    if a.json:
        with open(a.json, "w", encoding="utf-8") as fh:
            json.dump(report, fh, indent=1)
    return 1 if failed else 0


def check_engine(vt):
    want = [
        ("character", CHARACTER_NEW_SCRIPT, "?new_script@character@@"),
        ("timer_int", 9, "?start@timer_int@@"),
        ("timer_int", 10, "?stop@timer_int@@"),
        ("hot_int", 7, "??4hot_int@@"),
        ("hot_float", 7, "??4hot_float@@"),
        ("IIIE", 2, "?trash_rule@IIIE@@"),
    ]
    bad = []
    for cls, slot, prefix in want:
        got = vt.get(cls, [])
        if slot >= len(got) or not got[slot].startswith(prefix):
            bad.append(f"{cls} slot {slot}: expected {prefix}")
    return bad


if __name__ == "__main__":
    sys.exit(main())
