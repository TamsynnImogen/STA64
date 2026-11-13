#!/usr/bin/env python3
"""Convert Armada I legacy missions (`<name>S.dsl` + `<name>R.drl`) into Lua (LUA-14).

The Armada I front end for LUA-11's lifter (`tools/dsl_to_lua.py`): the same
statement and expression shapes, the same Lua output (rule and script
engine, `A(...)` guard, counters), with engine calls going to the `A1` table
instead of `A2`, so the mission runtime, the scheduler and the stand-in host
are shared.

What differs from Armada II (evidence: read from the DLLs and Armada.exe,
static; notes are kept outside the repository):

- The DLLs are release builds that import every engine function and object
  by its decorated name from `Armada.exe` (the rule DLL also imports the
  mission DLL's variables, handles and actions). A call site names its callee
  through the import table; the decoration gives calling convention,
  parameter and return types, so arguments are read from a modelled stack.
- No frame pointer: locals and outgoing arguments are `esp`-relative. Each
  basic block knows the stack depth at its entry (the same on every path, or
  the function is not decoded); pushed values stay symbolic, and a `cdecl`
  callee's arguments stay on the stack until the caller's `add esp`, often
  for several calls at once. A value that differs between the paths reaching
  a block goes through the function's slot table (`L[...]`), as registers do
  (`R.x`).
- Variables are `hot_int` / `hot_float` / `dirty_int` / `timer_int` objects:
  reads are `operator int()`, writes `operator=`, timers `start`/`stop`,
  called with the object as `this`. Their start values are the engine's
  constructors' (hot -1, timer 0; read from Armada.exe, static), the same as
  Armada II's, so the shared prelude's variable model applies.
- Rules are registered in the rule DLL's `DllMain` attach code
  (`IIIE::create_rule(condition, action, name)` on `WORLD`'s rule engine,
  `hot_type::add_dependent_rule(rule)` on each waking variable); an action
  is the mission DLL's exported `void Name(void)`, which queues a named
  script (`character::new_script`) and its steps (`script::add_step`).
- Start values and handles are set in the mission DLL's `DllMain` attach
  code: converted as `ScriptInit`, run by the shared engine after the rules
  are registered (the variable changes it makes wake their rules on the
  first tick either way).

Nothing is guessed. A function the lifter cannot follow (an instruction or
stack shape it does not model) becomes a stub that, when run, counts itself
in `Mission.notDecoded` and every engine call it would have made in
`Mission.undecoded`. A single call whose argument cannot be read is written
as `UD(name, why)`: it counts in `Mission.undecoded`, is reported as a
problem, answers nil, and the rest of the function runs. The rule engine's
scheduling is Armada II's (`tools/dsl_to_lua.py`): same class names and
start values in both executables; Armada I's scheduler itself is not
measured.

The output, one `missions/<map>.lua` per map whose `.bzn` names the rule
DLL, is written to `--out`, outside the repository and every install;
nothing read from a game file is committed (D2: the DLLs are read, never
loaded).

usage:
  a1_to_lua.py --install <A1 root> --out <dir> [--only fed1] [--json r.json] [--include-unmapped]

`--include-unmapped` also converts the DLL pairs no `.bzn` names (six in the stock
install: cut campaign content the shell's campaign list `mshell.set` does not reach),
as `missions/<dll stem>.lua`.

Needs `objdump` (binutils) on PATH.
"""

import argparse
import collections
import json
import os
import re
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import drl_to_lua as drl  # noqa: E402
import dsl_to_lua as dsl  # noqa: E402

TOOL = "tools/a1_to_lua.py"
DecodeError = drl.DecodeError
REG32 = dsl.REG32
SAVED = ("ebx", "esi", "edi", "ebp")

# Variable methods by the decorated method name (operator codes `??4` = `=`,
# `??B` = conversion). The value of `this` names the variable.
VAR_METHODS = {"op4": "set", "opB": "get", "start": "start", "stop": "stop", "update": "update",
               "mark_dirty": "mark_dirty", "set_clean": "set_clean", "is_dirty": "is_dirty", "time": "time"}
VAR_CLASSES = drl.VAR_KINDS + ("hot_type",)
# `WORLD`'s rule engine (`IIIE`) is the member at this offset; rules are
# created and trashed on it (read from the DLLs and Armada.exe, static).
WORLD_IIIE = 0x38
MEM = ("mem",)  # a stack slot whose value is in the function's slot table
UNINIT = ("stackslot",)  # reserved by `sub esp` / `push reg`, not yet written


# ------------------------------------------------------------------ signatures


def signature(sym):
    """(convention, class, method, return code, parameter codes) of a
    decorated function name, or None for data and names it does not read."""
    m = re.fullmatch(r"\?\?([0-9A-Z_])([A-Za-z_]\w*)@@([A-Z])([AB])([AEG])(.*)", sym)
    if m:
        name, cls, conv, rest = "op" + m.group(1), m.group(2), m.group(5), m.group(6)
    else:
        m = re.fullmatch(r"\?([A-Za-z_]\w*)@([A-Za-z_]\w*)@@([A-Z])([AB])([AEG])(.*)", sym)
        if m:
            name, cls, conv, rest = m.group(1), m.group(2), m.group(5), m.group(6)
        else:
            m = re.fullmatch(r"\?([A-Za-z_]\w*)@@Y([AGI])(.*)", sym)
            if not m:
                return None
            name, cls, conv, rest = m.group(1), None, m.group(2), m.group(3)
    convention = {"E": "thiscall", "A": "cdecl", "G": "stdcall", "I": "fastcall"}[conv]
    if (cls is None and convention == "thiscall") or convention == "fastcall":
        return None  # fastcall passes arguments in registers: not modelled
    try:
        types = parse_types(rest)
    except (ValueError, IndexError):
        return None
    if not types:
        return None
    ret, params = types[0], types[1:]
    if params == ["X"]:
        params = []
    return convention, cls, name, ret, params


def parse_types(s):
    """MSVC type codes of a decorated signature's return type and parameters
    (up to the closing `@`/`Z`): primitives, pointers and references,
    named classes, function pointers (`P6...`) and back-references."""
    pos, seen, out = [0], [], []

    def one():
        i = pos[0]
        c = s[i]
        if c == "_":
            pos[0] += 2
            return s[i:i + 2]
        if c == "?":
            pos[0] += 2  # ?A / ?B: storage class of a returned value
            return "?A" + one()
        if c in "PQAB" and s[i + 1] == "6":
            pos[0] += 3  # function pointer: P6 <convention> <return> <params> Z
            one()
            while s[pos[0]] not in "@Z":
                one()
            pos[0] += 1
            return s[i:pos[0]]
        if c in "PQAB" and s[i + 1] in "ABCD":
            pos[0] += 2
            return c + s[i + 1] + one()
        if c in "VUTW":
            j = s.index("@@", i)
            pos[0] = j + 2
            return s[i:j + 2]
        if c.isdigit():
            pos[0] += 1
            return seen[int(c)]
        pos[0] += 1
        return c

    while pos[0] < len(s) and s[pos[0]] not in "@Z":
        t = one()
        if out and len(t) > 1 and not t.startswith("?A"):
            seen.append(t)  # only parameter types are remembered for back-references
        out.append(t)
        if len(out) == 2 and t == "X":
            break
    return out


def slots_of(params, ret):
    """Stack slots a call's arguments take (a struct returned by value adds
    the hidden result pointer)."""
    return sum(dsl.push_count(p) for p in params) + (1 if ret.startswith("?A") else 0)


# ------------------------------------------------------------------ the DLLs


class A1Dll(dsl.Dsl):
    """A release-built Armada I mission or rule DLL."""

    def __init__(self, path, ins=None, pe=None, exports=None, entry_point=None):
        super().__init__(path, {}, pe=pe, ins=ins, exports=exports)
        pe = self.pe
        self.stubs = {}  # `jmp [IAT]` -> import symbol
        for a, t in self.ins:
            m = re.fullmatch(r"jmp\s+DWORD PTR ds:0x([0-9a-f]+)", t)
            if m and int(m.group(1), 16) in pe.imports:
                self.stubs[a] = pe.imports[int(m.group(1), 16)][1]
        self.entries = set(self.thunks.values()) | set(self.code_names)
        if entry_point is None:
            b = pe.data
            peo = struct.unpack_from("<I", b, 0x3C)[0]
            entry_point = pe.base + struct.unpack_from("<I", b, peo + 24 + 16)[0]
        self.entry_point = entry_point
        self.dllmain = self.find_dllmain()

    def entry(self, addr):
        addr = self.thunks.get(addr, addr)
        if addr not in self.at:
            raise DecodeError(f"no instruction at {addr:#x}")
        return addr

    def resolve(self, addr):
        """A call or jump target: ("imp", symbol) or ("fn", address)."""
        a = self.thunks.get(addr, addr)
        if a in self.stubs:
            return "imp", self.stubs[a]
        if a not in self.at:
            raise DecodeError(f"no instruction at {a:#x}")
        return "fn", a

    def import_sym(self, iat):
        x = self.pe.imports.get(iat)
        return x[1] if x else None

    def find_dllmain(self):
        """The C runtime's DLL entry calls `DllMain(hinst, reason, reserved)`
        through the incremental-link table with its own three arguments."""
        i = self.at.get(self.entry_point)
        if i is None:
            return None
        pushes = 0
        found = []
        for _, t in self.ins[i:i + 160]:
            if t.startswith("push "):
                pushes += 1
                continue
            m = re.fullmatch(r"call\s+0x([0-9a-f]+)", t)
            if m and pushes >= 3 and int(m.group(1), 16) in self.thunks:
                found.append(self.thunks[int(m.group(1), 16)])
            if t.startswith("ret"):
                break
            if t.startswith("call") or t.startswith("j"):
                pushes = 0
        return found[0] if found and all(f == found[0] for f in found) else None


# ------------------------------------------------------------------ lifting


def imports_in(d, seen):
    """Imported functions a function calls (directly, through a register
    loaded from the import table, through a stub, or as a tail jump)."""
    regs, out = {}, []
    for x in sorted(seen):
        t = d.ins[d.at[x]][1]
        m = re.fullmatch(r"mov\s+(e\w\w),DWORD PTR ds:0x([0-9a-f]+)", t)
        if m:
            regs[m.group(1)] = d.import_sym(int(m.group(2), 16))
            continue
        m = re.fullmatch(r"(?:call|jmp)\s+(?:DWORD PTR ds:)?0x([0-9a-f]+)", t)
        sym = None
        if m:
            tgt = int(m.group(1), 16)
            sym = d.import_sym(tgt)
            if sym is None and t.startswith("call") and "ds:" not in t:
                try:
                    kind, y = d.resolve(tgt)
                    sym = y if kind == "imp" else None
                except DecodeError:
                    sym = None
        m2 = re.fullmatch(r"call\s+(e\w\w)", t)
        if m2:
            sym = regs.get(m2.group(1))
        if sym and signature(sym):
            out.append(call_name(sym))
    return out


def call_name(sym):
    """The name a call is counted under: the function's own name, or
    `class.method` for a method."""
    s = signature(sym)
    if s and s[1]:
        return f"{s[1]}.{s[2]}"
    return drl.plain_name(sym)


class Lifted(dsl.Lifted):
    def __init__(self, addr):
        super().__init__(addr)
        self.undecoded = []  # (call, why) written as UD(...)
        self.rules = []  # [name, condition addr, action target, deps]
        self.imports_called = []  # every import the function calls, in address order


class State(dsl.State):
    def __init__(self):
        super().__init__()
        self.depth = 0  # dwords pushed since the function's entry
        self.slots = {}  # stack position (1 = just below the return address) -> value
        self.entry_mem = set()  # positions whose value is in the slot table at block entry
        self.loc_in = set()  # of those, the ones this block reads
        self.frame = None  # depth `ebp` was set at (`mov ebp,esp`), if any
        self.byte_slots = set()  # slots holding a one-byte temporary


PURE = ("const", "float", "str", "code", "tmp", "impaddr", "varptr", "world", "iiie", "script", "saved",
        "rule", "this", "field", "farith", "arith", "neg", "cmp", "not", "and", "or", "truth", "u",
        "ftol", "sign", "abs", "sqrt", "bnot", "vec", "boolarg")


def pure(v):
    """A value that means the same in every block of the function (it reads no
    register, stack slot or memory that could change)."""
    if v is None or not isinstance(v, tuple) or not v:
        return False
    k = v[0]
    if k == "addr":
        return v[1][0] in ("obj", "elem", "vfield", "gvar", "loc") and all(pure(x) for x in v[1][2:] if isinstance(x, tuple))
    if k not in PURE:
        return False
    for x in v[1:]:
        if isinstance(x, tuple) and x and isinstance(x[0], str) and not pure(x):
            return False
        if isinstance(x, list) and not all(pure(y) for y in x):
            return False
    return True


class A1Lifter(dsl.Lifter):
    def __init__(self, dll):
        super().__init__(dll)
        self.no_params = set()
        self.rule_globals = {}  # rule-pointer global -> rule index (registration)
        self.rules = []

    # ------------------------------------------------------------ CFG

    def is_tail(self, start, tgt):
        d = self.d
        a = d.thunks.get(tgt, tgt)
        return a in d.stubs or tgt in d.thunks or (a in d.entries and a != start)

    def cfg(self, start):
        """Blocks and edges: {leader: [successors]}; a jump to an import, a
        link-table thunk or another function is a tail call (no edge)."""
        d = self.d
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
                    if mn == "jmp":
                        if not self.is_tail(start, tgt):
                            leaders.add(tgt)
                            todo.append(tgt)
                        break
                    leaders.add(tgt)
                    todo.append(tgt)
                    leaders.add(d.ins[i + 1][0])
                    todo.append(d.ins[i + 1][0])
                    break
                if mn == "jmp":
                    m = re.fullmatch(r"jmp\s+DWORD PTR ds:0x([0-9a-f]+)", t)
                    if m and int(m.group(1), 16) in d.pe.imports:
                        break
                    tbl = self.switch_table(i)
                    self.switches[x] = tbl
                    for tgt in tbl:
                        leaders.add(tgt)
                        todo.append(tgt)
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
        leaders &= seen
        # successors, by walking each block to its end
        succs = {}
        for b in leaders:
            i = d.at[b]
            first = True
            while True:
                x, t = d.ins[i]
                if not first and x in leaders:
                    succs[b] = [x]
                    break
                first = False
                mn = t.split()[0]
                m = re.match(r"(j\w+)\s+0x([0-9a-f]+)$", t)
                if m:
                    tgt = int(m.group(2), 16)
                    if mn == "jmp":
                        succs[b] = [] if self.is_tail(start, tgt) else [tgt]
                    else:
                        succs[b] = [tgt, d.ins[i + 1][0]]
                    break
                if mn == "jmp":
                    succs[b] = list(self.switches.get(x, []))
                    break
                if mn == "ret":
                    succs[b] = []
                    break
                i += 1
        return leaders, seen, succs

    def lift(self, start):
        d = self.d
        f = Lifted(start)
        self.cur = f
        self.switches = {}
        try:
            leaders, seen, succs = self.cfg(start)
            f.instructions = len(seen)
            preds = collections.defaultdict(list)
            for b, ss in succs.items():
                for s in ss:
                    preds[s].append(b)
            # topological order (the CFG must be acyclic)
            order, mark = [], {}

            def visit(b):
                if mark.get(b) == 1:
                    raise DecodeError(f"loop at {b:#x}")
                if mark.get(b) == 2:
                    return
                mark[b] = 1
                for s in sorted(succs[b], reverse=True):
                    visit(s)
                mark[b] = 2
                order.append(b)

            visit(start)
            order.reverse()
            f.imports_called = imports_in(d, seen)
            self.check_helpers(start, seen)
            ntmp = [0]
            self.ntmp = ntmp
            ends, blocks = {}, {}
            for b in order:
                st = self.entry_state(b, start, preds, ends)
                stmts = []
                i = d.at[b]
                first = True
                while True:
                    a, t = d.ins[i]
                    if not first and a in leaders:
                        break
                    first = False
                    parts = t.split(None, 1)
                    mn, ops = parts[0], (parts[1] if len(parts) > 1 else "")
                    self.cur_state = st
                    r = self.step(f, st, stmts, ntmp, a, mn, ops, i, start == b)
                    if r == "ret":
                        break
                    if isinstance(r, tuple) and r[0] == "switch":
                        stmts.append(("switch", r[1], r[2]))
                        break
                    if isinstance(r, tuple):
                        kind, tgt, pred = r
                        if kind == "jmp":
                            stmts.append(("goto", tgt))
                            break
                        stmts.append(("if", drl.simplify(pred), tgt))
                        break
                    i += 1
                if st.fpu:
                    raise DecodeError(f"x87 value live across a block end at {a:#x}")
                ends[b] = st
                blocks[b] = stmts
            # values a later block reads from registers or the slot table, also
            # through blocks that pass them on untouched (backward liveness)
            live_r, live_s = {}, {}
            for b in reversed(order):
                st = ends[b]
                out_r, out_s = set(), set()
                for s in succs[b]:
                    out_r |= live_r[s]
                    out_s |= live_s[s]
                st.out_r, st.out_s = out_r, out_s
                live_r[b] = st.reg_in | {r for r in out_r if st.r.get(r, "unset") == "unset"}
                live_s[b] = st.loc_in | {j for j in out_s if st.slots.get(j, MEM) == MEM and j in st.entry_mem}
            if live_r[start] - {"ebx", "esi", "edi", "ebp"}:
                raise DecodeError(f"register {sorted(live_r[start])} read before it is set")
            for b in order:
                st, stmts = ends[b], blocks[b]
                pairs = []
                need_r, need_s = st.out_r, st.out_s
                for r in sorted(need_r):
                    v = st.r.get(r)
                    if v is None or v == ("reg", r):
                        continue
                    pairs.append((("reg", r), v))
                for j in sorted(need_s):
                    v = st.slots.get(j, MEM)
                    if v in (MEM, UNINIT) or v is None:
                        continue
                    pairs.append((("loc", -4 * j), v))
                if pairs:
                    k = len(stmts)
                    while k > 0 and stmts[k - 1][0] in ("goto", "if", "return", "switch"):
                        k -= 1
                    for j in range(k, len(stmts)):
                        sj = stmts[j]
                        if sj[0] in ("if", "switch"):
                            ntmp[0] += 1
                            t = ("tmp", ntmp[0])
                            stmts.insert(k, ("set", t, sj[1]))
                            k += 1
                            stmts[j + 1] = (sj[0], t) + sj[2:]
                    stmts.insert(k, ("multiset", pairs))
                # a block that falls through to a block lifted out of order needs a jump
                nxt = succs[b]
                if stmts and stmts[-1][0] == "if" and len(nxt) == 2:
                    stmts.append(("goto", nxt[1]))
                elif len(nxt) == 1 and (not stmts or stmts[-1][0] not in ("goto", "return", "switch")):
                    stmts.append(("goto", nxt[0]))
            f.blocks = [[b, blocks[b]] for b in order]
        except DecodeError as e:
            f.error = str(e)
            if not f.imports_called:
                f.imports_called = imports_in(d, self.linear(start))
        return f

    def linear(self, start):
        """Addresses from `start` in address order up to the function's end, for
        listing the calls of a function the lifter could not follow."""
        d, out = self.d, set()
        i = d.at.get(start)
        while i is not None and i < len(d.ins) and len(out) < 4000:
            a, t = d.ins[i]
            if out and a in d.entries:
                break
            out.add(a)
            if t.startswith("int3") and len(out) > 1:
                break
            i += 1
        return out

    def check_helpers(self, start, seen):
        """Internal calls and tail jumps pass no arguments the lifter could miss:
        the callee must read none (the missions' helpers take none)."""
        d = self.d
        for x in seen:
            m = re.fullmatch(r"(?:call|jmp)\s+0x([0-9a-f]+)", d.ins[d.at[x]][1])
            if not m:
                continue
            tgt = int(m.group(1), 16)
            if d.ins[d.at[x]][1].startswith("jmp") and not self.is_tail(start, tgt):
                continue
            try:
                kind, body = d.resolve(tgt)
            except DecodeError:
                continue
            if kind != "fn" or body == start or d.is_ftol(body) or body in self.no_params:
                continue
            if self.reads_params(body):
                raise DecodeError(f"call at {x:#x} to a helper that takes arguments")
            self.no_params.add(body)

    def reads_params(self, body):
        d = self.d
        for a in self.linear(body):
            t = d.ins[d.at[a]][1]
            if re.search(r"\[(esp|ebp)\+0x[0-9a-f]+\]", t):
                return True  # conservative: any positive stack offset
        return False

    def entry_state(self, b, start, preds, ends):
        st = State()
        st.body = True
        if b == start:
            for r in SAVED:
                st.r[r] = ("saved", r)
            return st
        ps = preds[b]
        if not ps or any(p not in ends for p in ps):
            raise DecodeError(f"block {b:#x} reached before its predecessors")
        depths = {ends[p].depth for p in ps}
        frames = {ends[p].frame for p in ps}
        if len(depths) != 1 or len(frames) != 1:
            raise DecodeError(f"stack depth differs between the paths into {b:#x}")
        st.depth = depths.pop()
        st.frame = frames.pop()
        for r in REG32 + ("ebp",):
            vals = [ends[p].r.get(r, "unset") for p in ps]
            if all(v == vals[0] for v in vals) and vals[0] != "unset" and pure(vals[0]):
                st.r[r] = vals[0]
        for j in range(1, st.depth + 1):
            vals = [ends[p].slots.get(j, MEM) for p in ps]
            if all(v == vals[0] for v in vals) and (vals[0] == UNINIT or pure(vals[0])):
                st.slots[j] = vals[0]
            else:
                st.slots[j] = MEM
                st.entry_mem.add(j)
        return st

    # ------------------------------------------------------------ stack

    def stack_off(self, st, reg, k, a, byte=False):
        if reg == "esp":
            off = -4 * st.depth + k
        elif reg == "ebp":
            if st.frame is None:
                raise DecodeError(f"`ebp` used without a frame at {a:#x}")
            off = -4 * st.frame + k
        else:
            raise DecodeError(f"stack access through {reg} at {a:#x}")
        if off % 4:
            # a one-byte temporary (a `bool const &` argument): its dword slot,
            # only while nothing else lives in that slot
            j = -(off - off % 4) // 4
            if not byte or (st.slots.get(j, MEM) != UNINIT and j not in st.byte_slots):
                raise DecodeError(f"unaligned stack access at {a:#x}")
            st.byte_slots.add(j)
            off -= off % 4
        if off == 0:
            raise DecodeError(f"access to the return address at {a:#x}")
        return off

    def slot_lv(self, st, off, a):
        if off >= 4:
            return ("param", (off - 4) // 4)
        j = -off // 4
        if j > st.depth:
            raise DecodeError(f"access below the stack pointer at {a:#x}")
        return ("loc", off)

    def push(self, st, v):
        st.depth += 1
        st.slots[st.depth] = v
        st.entry_mem.discard(st.depth)

    def drop(self, st, n, a):
        if n > st.depth:
            raise DecodeError(f"stack underflow at {a:#x}")
        for _ in range(n):
            st.slots.pop(st.depth, None)
            st.entry_mem.discard(st.depth)
            st.byte_slots.discard(st.depth)
            st.depth -= 1

    def slot_read(self, st, j):
        v = st.slots.get(j, MEM)
        if v == MEM:
            if j in st.entry_mem:
                st.loc_in.add(j)
            return ("loc", -4 * j)
        return v

    def slot_write(self, st, stmts, j, v):
        self.snap(st, stmts, lambda e: e == ("loc", -4 * j))
        st.slots[j] = v
        st.entry_mem.discard(j)

    # ------------------------------------------------------------ operands

    def read(self, st, op, a):
        op = op.strip()
        if op == "ebp":
            v = st.r.get("ebp", "unset")
            if v == "unset":
                st.reg_in.add("ebp")
                return ("reg", "ebp")
            return v
        if op == "esp":
            raise DecodeError(f"`esp` as a value at {a:#x}")
        return super().read(st, op, a)

    def lvalue(self, st, op, a, lea=False):
        byte = lea or op.strip().startswith("BYTE PTR")
        op2 = re.sub(r"^(?:DWORD|BYTE|WORD|QWORD) PTR ", "", op.strip())
        m = re.fullmatch(r"\[(esp|ebp)(?:([+-])(0x[0-9a-f]+))?\]", op2)
        if m:
            k = int(m.group(3), 16) if m.group(3) else 0
            if m.group(2) == "-":
                k = -k
            if m.group(1) == "ebp" and st.frame is None:
                return ("deref", self.read(st, "ebp", a), k)  # `ebp` as a plain register
            return self.slot_lv(st, self.stack_off(st, m.group(1), k, a, byte), a)
        m = re.fullmatch(r"\[(e\w\w)\+(e\w\w)\*4\]", op2)
        if m:
            base = self.read(st, m.group(1), a)
            idx = self.read(st, m.group(2), a)
            return ("deref", ("index", base, idx), 0)
        return super().lvalue(st, op, a)

    def write(self, f, st, stmts, x, v, a):
        if x == "ebp" and st.frame is None:
            st.r["ebp"] = v
            return
        super().write(f, st, stmts, x, v, a)

    def glob(self, addr, a):
        sym = self.d.import_sym(addr)
        if sym is not None:
            return ("impslot", sym)
        return super().glob(addr, a)

    def load(self, st, lv, a):
        k = lv[0]
        if k == "impslot":
            return ("impaddr", lv[1])
        if k == "loc":
            return self.slot_read(st, -lv[1] // 4)
        if k == "param":
            return st.mem.get(lv, lv)
        if k == "deref":
            base, off = lv[1], lv[2]
            if base and base[0] == "index":
                arr, idx = base[1], base[2]
                if arr and arr[0] == "impaddr" and dsl.data_kind(arr[1]) == "intarray":
                    return ("elem", drl.plain_name(arr[1]), idx)
                if arr and arr[0] == "addr" and arr[1][0] == "obj":
                    return ("elem", arr[1][1], idx)
                raise DecodeError(f"indexed load from {arr and arr[0]} at {a:#x}")
            if base and base[0] == "impaddr":
                return self.import_data(base[1], off, a)
            if base and base[0] == "addr" and base[1][0] == "loc":
                return self.slot_read(st, -(base[1][1] + off) // 4) if off % 4 == 0 else self.bad(a)
            if base and base[0] == "addr" and base[1][0] == "param" and off == 0:
                return self.load(st, base[1], a)
        return super().load(st, lv, a)

    def bad(self, a):
        raise DecodeError(f"unaligned stack access at {a:#x}")

    def import_data(self, sym, off, a):
        kind = dsl.data_kind(sym)
        name = drl.plain_name(sym)
        if kind in ("int", "float") and off == 0:
            return ("obj", name)
        if kind == "intarray" and off % 4 == 0:
            return ("elem", name, ("const", off // 4))
        if kind == "Vector3" and off in (0, 4, 8):
            return ("vfield", name, off // 4)
        raise DecodeError(f"read of imported {kind} {name}+{off:#x} at {a:#x}")

    def store(self, f, st, stmts, dst, v, a):
        lv = self.lvalue(st, dst, a)
        if lv[0] == "deref" and lv[1] and lv[1][0] == "addr" and lv[1][1][0] == "loc":
            lv = ("loc", lv[1][1][1] + lv[2])
        if lv[0] == "deref" and lv[1] and lv[1][0] == "impaddr":
            kind = dsl.data_kind(lv[1][1])
            name = drl.plain_name(lv[1][1])
            off = lv[2]
            if kind in ("int", "float") and off == 0:
                lv = ("obj", name)
            elif kind == "intarray":
                lv = ("elem", name, ("const", off // 4))
            elif kind == "Vector3" and off in (0, 4, 8):
                lv = ("vfield", name, off // 4)
            else:
                raise DecodeError(f"store to imported {kind} {name} at {a:#x}")
            stmts.append(("set", lv, self.snap(st, stmts, dsl.reads_location(lv), v)))
            return
        if lv[0] == "loc":
            if lv[1] % 4:
                raise DecodeError(f"unaligned stack store at {a:#x}")
            self.slot_write(st, stmts, -lv[1] // 4, v)
            return
        if v is not None and v[0] == "rule" and lv[0] == "gvar":
            self.rule_globals[lv[1]] = v[1]
            return
        if lv[0] == "param":
            self.snap(st, stmts, lambda e: e == lv)
            st.mem[lv] = v
            stmts.append(("set", lv, v))
            return
        dsl.Lifter.store(self, f, st, stmts, dst, v, a)

    def snap(self, st, stmts, test, value=None):
        cache = {}

        def fix(v):
            if v is None or not isinstance(v, tuple) or not dsl.contains(v, test):
                return v
            if v[0] in ("tmp", "addr", "code", "const", "float", "str"):
                return v
            if v not in cache:
                self.ntmp[0] += 1
                t = ("tmp", self.ntmp[0])
                stmts.append(("set", t, v))
                cache[v] = t
            return cache[v]

        for k in list(st.r):
            st.r[k] = fix(st.r[k])
        for j in list(st.slots):
            if st.slots[j] not in (MEM, UNINIT):
                st.slots[j] = fix(st.slots[j])
        st.fpu = [fix(v) for v in st.fpu]
        st.fret = fix(st.fret)
        for k in list(st.mem):
            if st.mem[k] != dsl.lv_value(k):
                st.mem[k] = fix(st.mem[k])
        return fix(value)

    # ------------------------------------------------------------ instructions

    def step(self, f, st, stmts, ntmp, a, mn, ops, i, entry):
        d = self.d
        r = st.r
        if mn == "ret":
            if st.depth != 0:
                raise DecodeError(f"unbalanced stack at a return ({st.depth} slots) at {a:#x}")
            v = r.get("eax", "unset")
            stmts.append(("return", self.read(st, "eax", a) if v == "unset" else v))
            return "ret"
        if mn == "push":
            if entry and ops.strip() in ("eax", "ecx", "edx") and ops.strip() not in st.r:
                self.push(st, UNINIT)  # `push ecx` reserving a slot: the value is never used
                return None
            self.push(st, self.read(st, ops, a))
            return None
        if mn == "pop":
            if st.depth == 0:
                raise DecodeError(f"pop of the return address at {a:#x}")
            v = self.slot_read(st, st.depth)
            self.drop(st, 1, a)
            if ops.strip() in REG32 or ops.strip() == "ebp":
                r[ops.strip()] = v
            else:
                self.store(f, st, stmts, ops, v, a)
            return None
        if mn in ("add", "sub") and ops.startswith("esp,"):
            n = int(ops.split(",")[1], 16)
            if n % 4:
                raise DecodeError(f"`{mn} esp,{n:#x}` at {a:#x}")
            if mn == "add":
                self.drop(st, n // 4, a)
            else:
                for _ in range(n // 4):
                    self.push(st, UNINIT)
            return None
        if mn == "leave":
            if st.frame is None:
                raise DecodeError(f"`leave` without a frame at {a:#x}")
            self.drop(st, st.depth - st.frame, a)
            r["ebp"] = self.slot_read(st, st.depth)
            self.drop(st, 1, a)
            st.frame = None
            return None
        if mn == "mov":
            dst, src = [x.strip() for x in ops.split(",", 1)]
            if dst == "ebp" and src == "esp":
                st.frame = st.depth
                return None
            if dst == "esp" and src == "ebp":
                if st.frame is None:
                    raise DecodeError(f"`mov esp,ebp` without a frame at {a:#x}")
                self.drop(st, st.depth - st.frame, a)
                return None
            if dst in REG32 and src == "esp":
                if not st.depth:
                    raise DecodeError(f"address of the return slot at {a:#x}")
                r[dst] = ("addr", ("loc", -4 * st.depth))
                return None
            if dst == "esp" or (dst == "ebp" and st.frame is not None):
                raise DecodeError(f"`mov {ops}` at {a:#x}")
            if dst in REG32 or dst == "ebp":
                r[dst] = self.read(st, src, a)
                return None
            if dst in ("al", "cl", "dl", "bl"):
                r[dsl.REG8[dst]] = self.read(st, src, a)
                return None
            self.store(f, st, stmts, dst, self.read(st, src, a), a)
            return None
        if mn == "lea":
            dst, src = [x.strip() for x in ops.split(",", 1)]
            if dst not in REG32:
                raise DecodeError(f"`lea {ops}` at {a:#x}")
            lv = self.lvalue(st, src, a, lea=True)
            if lv[0] == "deref" and lv[1] and lv[1][0] == "impaddr" and dsl.data_kind(lv[1][1]) == "character":
                r[dst] = ("worldoff", lv[2])
                return None
            r[dst] = ("addr", lv)
            return None
        if mn in ("add", "sub") and ops.split(",")[0].strip() in REG32:
            x, y = [s.strip() for s in ops.split(",", 1)]
            vx = self.read(st, x, a)
            if vx and vx[0] == "impaddr" and dsl.data_kind(vx[1]) == "character" and re.fullmatch(r"0x[0-9a-f]+", y):
                r[x] = ("worldoff", int(y, 16) if mn == "add" else -int(y, 16))
                return None
        if mn == "call":
            return self.call(f, st, stmts, ntmp, a, ops, i)
        if mn == "jmp":
            m = re.fullmatch(r"0x([0-9a-f]+)", ops)
            tail = None
            if m and self.is_tail(f.addr, int(m.group(1), 16)):
                tail = int(m.group(1), 16)
            m2 = re.fullmatch(r"DWORD PTR ds:0x([0-9a-f]+)", ops)
            if m2 and d.import_sym(int(m2.group(1), 16)):
                return self.tail_import(f, st, stmts, ntmp, a, d.import_sym(int(m2.group(1), 16)))
            if tail is not None:
                kind, x = d.resolve(tail)
                if kind == "imp":
                    return self.tail_import(f, st, stmts, ntmp, a, x)
                if st.depth:
                    raise DecodeError(f"tail call with {st.depth} slots on the stack at {a:#x}")
                t = self.new_tmp(ntmp)
                n = self.ret_slots(x)
                stmts.append(("set", t, ("fcall", x, [("param", k) for k in range(n)])))
                if x not in f.calls:
                    f.calls.append(x)
                stmts.append(("return", t))
                return "ret"
        return super().step(f, st, stmts, ntmp, a, mn, ops, i, entry)

    def arith(self, f, st, stmts, a, mn, ops):
        x, y = [s.strip() for s in ops.split(",", 1)]
        if mn == "or" and y == "0xffffffff":
            self.write(f, st, stmts, x, ("const", -1), a)  # `or r,-1`: r = -1
            st.flags = ("test", ("const", -1))
            return None
        if mn == "and" and y == "0x0":
            self.write(f, st, stmts, x, ("const", 0), a)
            st.flags = ("test", ("const", 0))
            return None
        if "," not in y and not (x == y and mn in ("xor", "sub", "sbb")):
            vx = self.read(st, x, a)
            vy = self.read(st, y, a)
            if vx is None or vy is None:
                raise DecodeError(f"`{mn} {ops}` on an unknown value at {a:#x}")
        return super().arith(f, st, stmts, a, mn, ops)

    # ------------------------------------------------------------ calls

    def ret_slots(self, body):
        """Arguments a stdcall function pops (`ret N`), else 0."""
        i = self.d.at.get(body)
        while i is not None and i < len(self.d.ins):
            t = self.d.ins[i][1]
            if t.startswith("ret"):
                m = re.fullmatch(r"ret\s+0x([0-9a-f]+)", t)
                return int(m.group(1), 16) // 4 if m else 0
            i += 1
        return 0

    def call(self, f, st, stmts, ntmp, a, ops, i):
        d = self.d
        m = re.fullmatch(r"0x([0-9a-f]+)", ops)
        if m:
            kind, x = d.resolve(int(m.group(1), 16))
            if kind == "fn":
                return self.internal(f, st, stmts, ntmp, a, x, i)
            return self.engine(f, st, stmts, ntmp, a, x, -4 * st.depth)
        v = self.read(st, ops, a)
        if v and v[0] == "impaddr" and signature(v[1]):
            return self.engine(f, st, stmts, ntmp, a, v[1], -4 * st.depth)
        raise DecodeError(f"indirect call `{ops}` ({v and v[0]}) at {a:#x}")

    def internal(self, f, st, stmts, ntmp, a, body, i):
        d = self.d
        if d.is_ftol(body):
            v = self.fpop(st, a)
            st.r["eax"] = ("ftol", v)
            st.r["edx"] = None
            return None
        n = self.ret_slots(body)
        pops = n
        if not n:
            nxt = d.ins[i + 1][1] if i + 1 < len(d.ins) else ""
            mm = re.fullmatch(r"add\s+esp,(0x[0-9a-f]+)", nxt)
            n = int(mm.group(1), 16) // 4 if mm else 0
        if n > st.depth:
            raise DecodeError(f"internal call with {n} arguments, {st.depth} pushed at {a:#x}")
        self.snap(st, stmts, lambda e: e[0] in ("obj", "elem", "vfield", "glob", "var", "loc"))
        args = [self.arg(self.slot_read(st, st.depth - k), a) for k in range(n)]
        if pops:
            self.drop(st, pops, a)
        if body not in f.calls:
            f.calls.append(body)
        t = self.new_tmp(ntmp)
        stmts.append(("set", t, ("fcall", body, args)))
        self.clobber(st)
        st.r["eax"] = t
        st.fret = t
        return None

    def tail_import(self, f, st, stmts, ntmp, a, sym):
        if st.depth:
            raise DecodeError(f"tail call with {st.depth} slots on the stack at {a:#x}")
        self.engine(f, st, stmts, ntmp, a, sym, 4)
        v = st.r.get("eax")
        stmts.append(("return", v))
        return "ret"

    def arg_at(self, st, off, a):
        if off >= 4:
            lv = ("param", (off - 4) // 4)
            return st.mem.get(lv, lv)
        j = -off // 4
        if j < 1 or j > st.depth:
            raise DecodeError(f"argument outside the stack at {a:#x}")
        return self.slot_read(st, j)

    def engine(self, f, st, stmts, ntmp, a, sym, base):
        """A call to an imported function: arguments start at stack offset
        `base` (the top of the stack, or the caller's own arguments for a tail
        call)."""
        sig = signature(sym)
        plain = call_name(sym)
        if sig is None:
            raise DecodeError(f"cannot read the signature of {sym}")
        conv, cls, name, ret, params = sig
        n = slots_of(params, ret)
        raw = [self.arg_at(st, base + 4 * k, a) for k in range(n)]
        this = st.r.get("ecx", "unset") if conv == "thiscall" else None
        if this == "unset":
            this = self.read(st, "ecx", a)
        if conv in ("thiscall", "stdcall") and base < 4:
            self.drop(st, n, a)
        self.snap(st, stmts, lambda e: e[0] == "loc")
        special = self.special(f, st, stmts, ntmp, a, conv, cls, name, ret, params, raw, this, sym)
        if special:
            return None
        if cls is not None:
            return self.undecoded(f, st, stmts, ntmp, plain, ret, f"method of {cls} on {this and this[0]}")
        hidden = None
        if ret.startswith("?A"):
            hidden, raw = raw[0], raw[1:]
            if not (hidden and hidden[0] == "addr" and hidden[1][0] == "loc"):
                return self.undecoded(f, st, stmts, ntmp, plain, "X", f"struct result into {hidden and hidden[0]}")
        try:
            args = self.typed(raw, params, plain, a)
        except DecodeError as e:
            return self.undecoded(f, st, stmts, ntmp, plain, ret, str(e).split(" at 0x")[0])
        f.engine_calls[plain] = f.engine_calls.get(plain, 0) + 1
        call = ("call", plain, args)
        if ret == "X":
            stmts.append(("eval", call))
            self.clobber(st)
            return None
        t = self.new_tmp(ntmp)
        stmts.append(("set", t, call))
        self.clobber(st)
        if hidden:
            j0 = -hidden[1][1] // 4
            for k in range(3):
                self.slot_write(st, stmts, j0 - k, ("field", t, k))
            st.r["eax"] = hidden
            return None
        st.r["eax"] = t
        st.fret = t if ret == "M" else None
        return None

    def undecoded(self, f, st, stmts, ntmp, name, ret, why):
        f.undecoded.append((name, why))
        e = ("undecodedcall", name, why)
        if ret == "X":
            stmts.append(("eval", e))
            self.clobber(st)
            return None
        t = self.new_tmp(ntmp)
        stmts.append(("set", t, e))
        self.clobber(st)
        st.r["eax"] = t
        st.fret = t if ret == "M" else None
        return None

    def this_var(self, this):
        """The variable a method's `this` names: an exported object of the
        mission DLL or one imported by name."""
        if not this:
            return None
        if this[0] == "addr" and this[1][0] == "obj":
            name = this[1][1]
            g = next((v for v in self.d.data.values() if v[0] == name), None)
            if g and g[1] in drl.VAR_KINDS:
                return name, g[1]
        if this[0] == "impaddr":
            k = dsl.data_kind(this[1])
            if k in drl.VAR_KINDS:
                return drl.plain_name(this[1]), k
        return None

    def special(self, f, st, stmts, ntmp, a, conv, cls, name, ret, params, raw, this, sym):
        """Engine plumbing the converted script implements itself; True when handled."""
        if cls in VAR_CLASSES and name in VAR_METHODS:
            var = self.this_var(this)
            op = VAR_METHODS[name]
            if var is None:
                self.undecoded(f, st, stmts, ntmp, f"{cls}.{name}", ret, f"variable method on {this and this[0]}")
                return True
            vname, kind = var
            try:
                args = self.typed(raw, params, f"{cls}.{name}", a)
            except DecodeError as e:
                self.undecoded(f, st, stmts, ntmp, f"{cls}.{name}", ret, str(e).split(" at 0x")[0])
                return True
            if op == "get":
                v = ("var", vname, kind)
                self.clobber(st)
                st.r["eax"] = v
                st.fret = v if kind == "hot_float" else None
                return True
            if op in ("time", "is_dirty"):
                t = self.new_tmp(ntmp)
                stmts.append(("set", t, ("varq", vname, op)))
                self.clobber(st)
                st.r["eax"] = t
                st.fret = t
                return True
            self.snap(st, stmts, lambda e: e[0] == "var" and e[1] == vname)
            stmts.append(("var", vname, op, args))
            self.clobber(st)
            return True
        if cls == "character" and name == "new_script":
            (nm,) = raw
            t = self.new_tmp(ntmp)
            stmts.append(("set", t, ("newscript", self.arg(nm, a))))
            self.clobber(st)
            st.r["eax"] = ("script", t)
            return True
        if cls == "script" and name == "add_step":
            (fn,) = raw
            if not fn or fn[0] != "code" or not this or this[0] != "script":
                raise DecodeError(f"add_step with {fn and fn[0]} on {this and this[0]} at {a:#x}")
            body = self.d.entry(fn[1])
            f.steps.append(body)
            stmts.append(("addstep", this[1], body))
            self.clobber(st)
            return True
        if cls == "IIIE" and name == "create_rule":
            if this != ("worldoff", WORLD_IIIE):
                raise DecodeError(f"create_rule on {this} at {a:#x}")
            cond, act, nm = raw
            if not (cond and cond[0] == "code" and act and act[0] == "code" and nm and nm[0] == "str"):
                raise DecodeError(f"create_rule arguments not constant at {a:#x}")
            idx = len(self.rules)
            self.rules.append([nm[1], cond[1], act[1], []])
            f.rules.append(idx)
            self.clobber(st)
            st.r["eax"] = ("rule", idx)
            return True
        if cls == "IIIE" and name == "trash_rule":
            (x,) = raw
            if this != ("worldoff", WORLD_IIIE):
                raise DecodeError(f"trash_rule on {this} at {a:#x}")
            if x and x[0] == "str":
                stmts.append(("trashrule", x))
            elif x and x[0] in ("rule", "glob"):
                idx = x[1] if x[0] == "rule" else self.rule_globals.get(x[1])
                if idx is None:
                    raise DecodeError(f"trash_rule of an unknown rule at {a:#x}")
                stmts.append(("trashrule", ("str", self.rules[idx][0])))
            else:
                raise DecodeError(f"trash_rule of {x and x[0]} at {a:#x}")
            self.clobber(st)
            return True
        if cls == "hot_type" and name in ("add_dependent_rule", "remove_dependent_rule"):
            (x,) = raw
            var = self.this_var(this)
            idx = None
            if x and x[0] == "rule":
                idx = x[1]
            elif x and x[0] == "glob":
                idx = self.rule_globals.get(x[1])
            if var is None or idx is None:
                raise DecodeError(f"{name} with {x and x[0]} on {this and this[0]} at {a:#x}")
            if name == "add_dependent_rule":
                self.rules[idx][3].append(var[0])
            self.clobber(st)
            return True
        if cls == "linked_list" and name in ("Add_Data", "Remove_Data"):
            stmts.append(("comment", f"{name}: native save/load callback, not used"))
            self.clobber(st)
            st.r["eax"] = ("const", 1)
            return True
        return False

    def typed(self, raw, params, what, a):
        """Arguments by parameter type; an `int &` is the int it refers to and
        a `Vector3 const &` the vector."""
        out, k = [], 0
        for p in params:
            c = dsl.push_count(p)
            if p in ("AAH", "ABH", "AAM", "ABM"):
                out.append(self.deref_value(self.ref_lv(raw[k], what, a), a))
            elif p in ("AA_N", "AB_N"):
                out.append(("boolarg", self.deref_value(self.ref_lv(raw[k], what, a), a)))
            elif p in ("ABVVector3@@", "AAVVector3@@", "PAVVector3@@", "PBVVector3@@"):
                lv = self.ref_lv(raw[k], what, a)
                out.append(("vec", [self.deref_value(self.lv_plus(lv, 4 * n, a), a) for n in range(3)]))
            elif p.startswith("VVector3"):
                out.append(("vec", [self.arg(x, a) for x in raw[k:k + 3]]))
            elif p in ("PAD", "PBD") and raw[k] == ("const", 0):
                out.append(("nil",))  # a null `char *`: no name
            elif c == 1:
                v = raw[k]
                if v and v[0] == "addr" and v[1][0] == "loc":
                    raise DecodeError(f"{what}: pointer to a stack slot at {a:#x}")
                if p == "M" and v and v[0] == "const":
                    v = ("float", struct.unpack("<f", struct.pack("<I", v[1] & 0xFFFFFFFF))[0])
                v = self.arg(v, a)
                out.append(("boolarg", v) if p == "_N" else v)
            else:
                raise DecodeError(f"{what}: parameter type {p} at {a:#x}")
            k += c
        return out

    def ref_lv(self, v, what, a):
        """The location a pointer or reference argument names."""
        if v and v[0] == "addr":
            return v[1]
        if v and v[0] == "impaddr":
            return ("deref", v, 0)
        raise DecodeError(f"{what}: reference argument {v and v[0]} at {a:#x}")

    def lv_plus(self, lv, off, a):
        if off == 0:
            return lv
        if lv[0] == "loc":
            return ("loc", lv[1] + off)
        if lv[0] == "obj":
            return self.glob_off(lv, off, a)
        if lv[0] == "vfield":
            return ("vfield", lv[1], lv[2] + off // 4)
        if lv[0] == "deref":
            return ("deref", lv[1], lv[2] + off)
        raise DecodeError(f"offset into {lv[0]} at {a:#x}")

    def deref_value(self, lv, a):
        if lv[0] == "loc":
            return self.arg(self.slot_read(self.cur_state, -lv[1] // 4), a)
        if lv[0] == "obj":
            g = next((v for v in self.d.data.values() if v[0] == lv[1]), None)
            if g and g[1] in ("int", "float"):
                return ("obj", lv[1])
            if g and g[1] == "Vector3":
                return ("vfield", lv[1], 0)
            raise DecodeError(f"reference to {g and g[1]} {lv[1]} at {a:#x}")
        if lv[0] in ("elem", "vfield"):
            return lv
        if lv[0] == "deref" and lv[1] and lv[1][0] == "impaddr":
            return self.import_data(lv[1][1], lv[2], a)
        if lv[0] == "gvar":
            return ("glob", lv[1])
        raise DecodeError(f"reference to {lv[0]} at {a:#x}")

    def arg(self, v, a):
        if v is None:
            raise DecodeError(f"unknown call argument at {a:#x}")
        if v[0] == "stackslot":
            raise DecodeError(f"argument slot not filled at {a:#x}")
        if v[0] in ("saved", "worldoff", "impaddr", "rule", "index"):
            raise DecodeError(f"argument is a {v[0]} at {a:#x}")
        return v


# ------------------------------------------------------------------ rendering


def render_expr(e, nm, ctx="num"):
    if e[0] == "nil":
        return "false" if ctx == "bool" else ("0" if ctx == "num" else "nil")
    if e[0] == "undecodedcall":
        s = f"UD({dsl.lua_str(e[1])}, {dsl.lua_str(e[2])})"
        return f"T({s})" if ctx == "bool" else (f"N({s})" if ctx == "num" else s)
    if e[0] in ("saved", "worldoff", "impaddr", "rule", "index", "impslot", "mem", "stackslot"):
        raise DecodeError(f"cannot render {e[0]}")
    return _render_expr(e, nm, ctx)


_render_expr = dsl.render_expr
dsl.render_expr = render_expr  # the A2 renderer recurses through the module name


def render_failed(name, why, calls):
    lst = ", ".join(dsl.lua_str(c) for c in calls)
    return [f"function M.{name}(...)",
            f"    M.notDecoded({dsl.lua_str(name)}, {dsl.lua_str(why)}, {{ {lst} }})",
            "    return 0",
            "end"]


A1_PRELUDE = r"""
-- Armada I: a call or a function the converter could not decode is counted
-- here when it runs (never dropped silently): Mission.undecoded[call].
local function UD(name, why)
    Mission.undecoded[name] = (Mission.undecoded[name] or 0) + 1
    Mission.callFailed = true
    problem(name, 'undecoded', why)
    return nil
end
function M.notDecoded(name, why, calls)
    Mission.notDecoded[name] = (Mission.notDecoded[name] or 0) + 1
    Mission.ran[#Mission.ran + 1] = name .. ' (not decoded)'
    problem(name, 'not decoded', why)
    for _, c in ipairs(calls or {}) do
        Mission.undecoded[c] = (Mission.undecoded[c] or 0) + 1
    end
end
"""


def a1_prelude():
    src = dsl.PRELUDE
    for old, new in (("local f = A2 and A2[name]", "local f = A1 and A1[name]"),):
        if old not in src:
            raise DecodeError(f"tools/dsl_to_lua.py prelude changed: `{old}` not found")
        src = src.replace(old, new)
    return src


# ------------------------------------------------------------------ mission


def classify(f):
    """complete / partial (some call written as UD) / not decoded."""
    if f.error:
        return "not"
    return "partial" if f.undecoded else "complete"


def norm(why):
    """A decode failure reason without addresses, for the leftover table."""
    why = re.sub(r"0x[0-9a-f]+", "#", why)
    why = re.sub(r"\(\d+ slots\)", "(n slots)", why)
    return why


def lift_mission(dsl_path, drl_path):
    S = dsl_path if isinstance(dsl_path, A1Dll) else A1Dll(dsl_path)
    R = drl_path if isinstance(drl_path, A1Dll) or drl_path is None else A1Dll(drl_path)
    # rule DLL: DllMain and what it calls register the rules
    rules, rfuncs, rerr = [], {}, []
    if R:
        rl = A1Lifter(R)
        if R.dllmain is None:
            rerr.append("no DllMain found")
        else:
            todo = [R.dllmain]
            while todo:
                x = todo.pop(0)
                if x in rfuncs:
                    continue
                f = rl.lift(x)
                rfuncs[x] = f
                if f.error:
                    rerr.append(f"registration {x - R.pe.base:#x}: {f.error}")
                todo.extend(f.calls)
        for name, cond, act, deps in rl.rules:
            rules.append({"name": name, "cond": cond, "act": act, "deps": deps})
        # conditions
        conds = {}
        for r in rules:
            c = R.entry(r["cond"])
            r["cond"] = c
            if c not in conds:
                conds[c] = rl.lift(c)
            # the action: a link-table thunk to the mission DLL's exported action
            kind, x = R.resolve(r["act"])
            if kind == "imp":
                r["action"] = drl.plain_name(x)
            else:
                w = rl.lift(x)
                calls = [s for s in w.imports_called if s]
                r["action"] = calls[0] if len(calls) == 1 and not w.error else None
                if r["action"] is None:
                    r["action_error"] = w.error or "action wrapper does not call one action"
        rfuncs.update({c: f for c, f in conds.items() if c not in rfuncs})
        registration_calls = collections.Counter()
        for f in rfuncs.values():
            if f.addr not in conds:
                for k, v in f.engine_calls.items():
                    registration_calls[k] += v
    else:
        conds, registration_calls = {}, collections.Counter()
    # mission DLL: DllMain (start values, handles) and the actions with their steps
    sl = A1Lifter(S)
    funcs, roles = {}, {}
    todo = []
    if S.dllmain:
        roles[S.dllmain] = "init"
        todo.append(S.dllmain)
    for n, a in S.exports.items():
        if S.pe.section_of(a) == ".text":
            e = S.entry(a)
            roles[e] = "action"
            todo.append(e)
    while todo:
        a = todo.pop(0)
        if a in funcs:
            continue
        f = sl.lift(a)
        funcs[a] = f
        for s in f.steps:
            roles.setdefault(s, "step")
            todo.append(s)
        for c in f.calls:
            roles.setdefault(c, "helper")
            todo.append(c)
    return S, R, funcs, roles, rules, conds, rerr, registration_calls


class CondNamer:
    def __init__(self, d, names):
        self.d = d
        self.names = names

    def fn(self, a):
        return self.names.get(a) or f"R_fn_{a - self.d.pe.base:x}"


def lua_ident(s):
    s = re.sub(r"[^A-Za-z0-9_]", "_", s)
    return s if re.match(r"[A-Za-z_]", s) else "_" + s


def emit_function(L, f, nm, name, role):
    w = L.append
    w(f"-- {role}: {name}" + (f"  (not decoded: {f.error})" if f.error else ""))
    if not f.error:
        try:
            L.extend(dsl.render_function(f, nm, name))
            w("")
            return
        except Exception as e:  # noqa: BLE001 - any renderer failure is that function's stub
            f.error = f"render: {type(e).__name__}: {e}"
    L.extend(render_failed(name, f.error, sorted(set(c for c in f.imports_called if c))))
    w("")


def convert(dsl_path, drl_path, map_name=None):
    """Lua source and a report for one Armada I mission."""
    S, R, funcs, roles, rules, conds, rerr, regcalls = lift_mission(dsl_path, drl_path)
    nm = dsl.Namer(S, funcs, roles)
    mission = re.sub(r"[sS]\.dsl$", "", os.path.basename(S.path))
    L = []
    w = L.append
    w(f"-- {map_name or mission}: converted by {TOOL} (LUA-14) from {os.path.basename(S.path)}"
      + (f" and {os.path.basename(R.path)}" if R else "") + ".")
    w("-- Generated on the user's machine from the user's install; not for redistribution.")
    w("-- Engine calls go to A1 by the imported function's name; unbound, failing or")
    w("-- undecoded calls are counted in Mission.missing / failed / undecoded and answer nil.")
    w("")
    w("Mission_ = {")
    w(f"    name = {dsl.lua_str(mission)}, game = 'a1',")
    w("    vars = {}, state = {}, objects = {}, globals = {}, timers = {}, scripts = {},")
    w("    rules = {}, affected = {}, satisfied = {}, fired = {}, fireCount = {}, ran = {},")
    w("    calls = {}, missing = {}, failed = {}, notDecoded = {}, stepErrors = {},")
    w("    conditionErrors = {}, engineVars = {}, engineWritten = {}, engineReads = {}, stepCount = 0, callFailed = false,")
    w("    problems = {}, problemOrder = {}, blocked = {}, invalid = {}, unresolved = {},")
    w("    undecoded = {},")
    w("}")
    w(a1_prelude())
    w(A1_PRELUDE)
    # variables: the mission DLL's exported objects; the engine's own by import
    for a, (n, k, size, _) in sorted(S.data.items()):
        if k in drl.VAR_KINDS:
            w(f"BindVar({dsl.lua_str(n)}, {dsl.lua_str(k)}, NewVar({dsl.lua_str(k)}, {dsl.lua_str(n)}))")
    engine_vars = set()
    for d in (S, R):
        if not d:
            continue
        for dll, sym in d.pe.imports.values():
            k = dsl.data_kind(sym)
            if k in drl.VAR_KINDS and not dll.lower().endswith(".dsl"):
                engine_vars.add((drl.plain_name(sym), k))
    for n, k in sorted(engine_vars):
        w(f"BindVar({dsl.lua_str(n)}, {dsl.lua_str(k)}, EngineVar({dsl.lua_str(n)}, {dsl.lua_str(k)}))")
    for a, (n, k, size, _) in sorted(S.data.items()):
        if k in ("intarray", "floatarray", "Vector3"):
            w(f"O.{n} = {{}}")
        elif k in ("int", "float"):
            w(f"O.{n} = {'0' if k == 'int' else '0.0'}")
    w("")
    w("-- ---------------------------------------------------------------- mission DLL")
    rank = ["init", "action", "step", "helper"]
    for a in sorted(funcs, key=lambda a: (rank.index(roles.get(a, "helper")) if roles.get(a) in rank else 9, a)):
        emit_function(L, funcs[a], nm, nm.fn(a), roles.get(a, "helper"))
    if S.dllmain:
        w("function M.ScriptInit() M." + nm.fn(S.dllmain) + "(0, 1, 0) end")
        w("")
    # rule conditions
    cnames = {}
    used = set()
    for r in rules:
        if r["cond"] not in cnames:
            base = "R_" + lua_ident(r["name"])
            n, k = base, 1
            while n in used:
                k += 1
                n = f"{base}_{k}"
            used.add(n)
            cnames[r["cond"]] = n
    w("-- ---------------------------------------------------------------- rule conditions")
    cn = CondNamer(R, cnames) if R else None
    for c in sorted(conds, key=lambda c: cnames.get(c, "")):
        emit_function(L, conds[c], cn, cn.fn(c), "condition")
    w("-- ---------------------------------------------------------------- rules")
    w(f"-- {len(rules)} rules from {os.path.basename(R.path) if R else '-'}, in creation order.")
    seen = {}
    for r in rules:
        seen[r["name"]] = seen.get(r["name"], 0) + 1
        name = r["name"] if seen[r["name"]] == 1 else f"{r['name']}#{seen[r['name']]}"
        w("Mission.rules[#Mission.rules + 1] = {")
        w(f"    name = {dsl.lua_str(name)},")
        w(f"    action = {dsl.lua_str(r.get('action') or '?')},")
        w(f"    deps = {{ {', '.join(dsl.lua_str(x) for x in r['deps'])} }},")
        w(f"    when = function(S, O) return T(M.{cnames[r['cond']]}()) end,")
        w("}")
    w("")
    report = mission_report(mission, map_name, funcs, roles, rules, conds, rerr, regcalls, S)
    names = sorted(set(report["engine_calls"]))
    w("-- Engine calls the converted code makes (bodies and rule conditions).")
    w("Mission.engineNames = { " + ", ".join(dsl.lua_str(n) for n in names) + " }")
    w(dsl.ENGINE)
    return mission, "\n".join(L) + "\n", report


def mission_report(mission, map_name, funcs, roles, rules, conds, rerr, regcalls, S):
    by_role = {}
    leftovers = collections.Counter()
    undecoded = collections.Counter()
    engine = collections.Counter()
    for role, f in [(roles.get(a, "helper"), f) for a, f in funcs.items()] + [("condition", f) for f in conds.values()]:
        r = by_role.setdefault(role, {"total": 0, "complete": 0, "partial": 0, "not": 0})
        r["total"] += 1
        r[classify(f)] += 1
        if f.error:
            leftovers[norm(f.error)] += 1
        for name, why in f.undecoded:
            undecoded[name] += 1
            leftovers["call: " + norm(why)] += 1
        for k, v in f.engine_calls.items():
            engine[k] += v
    actions = sum(1 for a in funcs if roles.get(a) == "action")
    return {
        "mission": mission,
        "map": map_name,
        "rules": len(rules),
        "rules_with_deps": sum(1 for r in rules if r["deps"]),
        "rule_actions_resolved": sum(1 for r in rules if r.get("action")),
        "registration_errors": rerr,
        "registration_calls": dict(regcalls),
        "actions": actions,
        "functions": by_role,
        "undecoded_calls": dict(undecoded),
        "leftovers": dict(leftovers.most_common()),
        "engine_calls": dict(sorted(engine.items())),
        "init": bool(S.dllmain),
    }


# ------------------------------------------------------------------ install


def find_missions(root, include_unmapped=False):
    """(map stem, S.dsl, R.drl) for every map whose `.bzn` names an Armada I
    rule DLL present in `missions/`. With `include_unmapped`, the pairs no
    `.bzn` names follow, keyed by their lower-case DLL stem (`federation4`):
    the engine reaches a mission only through the DLL names inside its map
    and the campaign list (`mshell.set`), so these are not campaign missions."""
    mdir = None
    for d in os.listdir(root):
        if d.lower() == "missions" and os.path.isdir(os.path.join(root, d)):
            mdir = os.path.join(root, d)
    bdir = next((os.path.join(root, d) for d in os.listdir(root) if d.lower() == "bzn"), None)
    if not mdir:
        return []
    files = {f.lower(): f for f in os.listdir(mdir)}
    out = []
    pairs = {}
    for f in files:
        if f.endswith("r.drl") and (f[:-5] + "s.dsl") in files:
            pairs[f] = (os.path.join(mdir, files[f[:-5] + "s.dsl"]), os.path.join(mdir, files[f]))
    if bdir and os.path.isdir(bdir):
        for b in sorted(os.listdir(bdir)):
            if not b.lower().endswith(".bzn"):
                continue
            try:
                data = open(os.path.join(bdir, b), "rb").read()
            except OSError:
                continue
            for m in re.finditer(rb"([A-Za-z0-9_]+[rR]\.drl)", data):
                key = m.group(1).decode("latin1").lower()
                if key in pairs:
                    out.append((b[:-4].lower(),) + pairs[key])
                    break
    if include_unmapped:
        named = {os.path.realpath(o[2]) for o in out}
        for key in sorted(pairs):
            dsl_path, drl_path = pairs[key]
            if os.path.realpath(drl_path) not in named and key[:-5] not in {o[0] for o in out}:
                out.append((key[:-5], dsl_path, drl_path))
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--install", required=True, help="Armada I root holding missions/ and bzn/")
    ap.add_argument("--out", help="output folder (outside the repository and every install)")
    ap.add_argument("--only", action="append", help="map to convert, e.g. fed1 (repeatable)")
    ap.add_argument("--json", help="write the conversion report as JSON")
    ap.add_argument("--include-unmapped", action="store_true",
                    help="also convert DLL pairs no .bzn names (unreachable leftovers), as missions/<dll stem>.lua")
    a = ap.parse_args(argv)
    for flag, path in (("--out", a.out), ("--json", a.json)):
        if path:
            why = drl.refuse_output(path, [a.install])
            if why:
                sys.exit(f"{flag} {why}")
    report = {"missions": []}
    failed = 0
    for map_name, dsl_path, drl_path in find_missions(a.install, a.include_unmapped):
        if a.only and map_name not in [x.lower() for x in a.only]:
            continue
        try:
            mission, src, rep = convert(dsl_path, drl_path, map_name)
        except Exception as e:  # noqa: BLE001 - one mission's failure must not stop the others
            failed += 1
            print(f"  FAIL {map_name}: {type(e).__name__}: {e}")
            report["missions"].append({"map": map_name, "error": str(e)})
            continue
        fr = rep["functions"]
        print(f"  {map_name} ({mission}): rules {rep['rules']}, "
              + ", ".join(f"{k} {v['complete']}+{v['partial']}p/{v['total']}" for k, v in sorted(fr.items())))
        for k, v in sorted(rep["registration_calls"].items()):
            print(f"    registration: {k} x{v} in the rule DLL's set-up code is not converted")
        for e in rep["registration_errors"]:
            print(f"    registration: {e}")
        report["missions"].append(rep)
        if a.out:
            out = os.path.join(os.path.realpath(a.out), "missions")
            os.makedirs(out, exist_ok=True)
            with open(os.path.join(out, map_name + ".lua"), "w", encoding="utf-8", newline="\n") as fh:
                fh.write(src)
    if a.json:
        with open(a.json, "w", encoding="utf-8") as fh:
            json.dump(report, fh, indent=1)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
