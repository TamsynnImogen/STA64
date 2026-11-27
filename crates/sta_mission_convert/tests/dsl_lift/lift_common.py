"""Shared pieces of the LUA-15d lifter-core drivers (this file reads no files at all).

The lifter core of tools/dsl_to_lua.py is ported in crates/sta_mission_convert/src/{dsl,lift}.rs.
Its `Lifter.step` hands every `call` instruction to `Lifter.call`, which is the next step's port
(LUA-15h). Until then the Rust lifter runs `lift::call_model`, and this module patches the
Python's `Lifter.call` with the same model, so both sides lift whole functions and the
intermediate forms can be compared. `canonical` prints a `Lifted` in the text form of
`Lifted::canonical` in lift.rs: every value as Python's `repr`.

Used by gen_pairs.py (authored cases, no game data) and dump_install.py (the install-gated
parity test).
"""

import os
import re
import struct
import sys

sys.dont_write_bytecode = True


def load_tool(tools_dir):
    """Import tools/dsl_to_lua.py (and drl_to_lua.py behind it) from `tools_dir`."""
    sys.path.insert(0, tools_dir)
    import dsl_to_lua as dsl  # noqa: E402
    return dsl


def canonical(f):
    """The text form of one `Lifted` (see `Lifted::canonical` in lift.rs)."""
    out = [f"func {f.addr:#x}"]

    def line(k, v):
        out.append(f" {k} {v}")

    line("error", repr(f.error))
    line("instructions", f.instructions)
    line("params", f.params)
    line("loops", f.loops)
    line("calls", repr(list(f.calls)))
    line("steps", repr(list(f.steps)))
    line("engine_calls", repr(list(f.engine_calls.items())))
    line("loctypes", repr(list(f.loctypes.items())))
    line("floatlocs", repr(sorted(f.floatlocs)))
    line("itype", repr(list(f.itype.items())))
    line("vectors", repr(sorted(f.vectors, key=repr)))
    line("litype", repr(list(f.litype.items())))
    for b, stmts in f.blocks:
        out.append(f" block {b:#x}")
        for s in stmts:
            out.append("  " + repr(s))
    return "\n".join(out) + "\n"


def install_call_model(dsl):
    """Replace `Lifter.call` with the model of `lift::call_model` in lift.rs: the paths of the
    original that need nothing but the core, and one opaque call for every other call."""
    DecodeError = dsl.DecodeError

    def call(self, f, st, stmts, ntmp, a, ops, i):
        d = self.d
        m = re.fullmatch(r"0x([0-9a-f]+)", ops)
        if m:
            tgt = int(m.group(1), 16)
            if tgt == d.chkesp:
                return None
            body = d.entry(tgt)
            if d.is_chkstk(body):
                st.body = False
                return None
            if not d.is_ftol(body):
                self.snap(st, stmts, lambda e: e[0] in ("obj", "elem", "vfield", "glob", "var", "locidx")
                          or (e[0] == "loc" and e in st.escaped))
            if d.is_ftol(body):
                v = self.fpop(st, a)
                st.r["eax"] = ("ftol", v)
                st.r["edx"] = None
                return None
            nxt = d.ins[i + 1][1] if i + 1 < len(d.ins) else ""
            mm = re.fullmatch(r"add\s+esp,(0x[0-9a-f]+)", nxt)
            if mm:
                n = int(mm.group(1), 16) // 4
                if n > len(st.args):
                    raise DecodeError(f"internal call with {n} arguments, {len(st.args)} pushed at {a:#x}")
                args = list(reversed(st.args[len(st.args) - n:]))
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
        self.snap(st, stmts, lambda e: e[0] == "loc" and e in st.escaped)
        args = self.take(st, len(st.args), a)
        t = self.new_tmp(ntmp)
        stmts.append(("set", t, ("icall", tgt, args)))
        self.clobber(st)
        st.r["eax"] = t
        st.fret = t
        return None

    dsl.Lifter.call = call


def kind_facts(d, addr):
    """`helper_kind`, `vector_kind`, `is_chkstk`, `is_ftol` of one address, as one line."""
    return " ".join([repr(d.helper_kind(addr)), repr(d.vector_kind(addr)), repr(d.is_chkstk(addr)),
                     repr(d.is_ftol(addr))])


def dsl_facts(d):
    """The Dsl-level state after construction, as lines."""
    out = [f"thunks {sorted(d.thunks.items())!r}", f"chkesp {d.chkesp!r}"]
    out.append("data " + repr([(a, n, k, s, m) for a, (n, k, s, m) in d.data.items()]))
    out.append("code_names " + repr(list(d.code_names.items())))
    return out


def static_refs_line(d):
    return "static_refs " + repr(list(d.static_refs.items()))


def function_list(d):
    """The functions of a DLL this test lifts: every exported code entry (thunks followed) in
    export order, then every direct call target of the listing not yet listed, in listing
    order (targets with no instruction are skipped)."""
    out = []
    seen = set()
    for n, a in d.exports.items():
        if d.pe.section_of(a) == ".text":
            e = d.entry(a)
            if e not in seen:
                seen.add(e)
                out.append(e)
    for _, t in d.ins:
        m = re.fullmatch(r"call\s+0x([0-9a-f]+)", t)
        if m:
            tgt = int(m.group(1), 16)
            tgt = d.thunks.get(tgt, tgt)
            if tgt in d.at and tgt not in seen:
                seen.add(tgt)
                out.append(tgt)
    return out
