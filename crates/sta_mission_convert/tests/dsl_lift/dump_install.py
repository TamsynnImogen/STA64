#!/usr/bin/env python3
"""Install-gated half of the LUA-15d parity test: the intermediate form the ORIGINAL
tools/dsl_to_lua.py computes for every function of one mission DLL, in the canonical text form
that crates/sta_mission_convert/tests/dsl_lift_parity.rs compares with the Rust lifter's.

THIS SCRIPT READS GAME FILES: the DLL it is given and the objdump listing of it. It is run only
by the gated test, from a temp directory outside the repository, with the install's own files
named on the command line; it writes only to the output file it is given and prints nothing. Its
output is game-derived: it goes to a temp directory and, on a failure, to target/scratch/, never
into the repository.

    python3 -I dump_install.py <repo>/tools <dll> <objdump listing text> <output file>

The listing is the output of `objdump -d -M intel --no-show-raw-insn <dll>`, parsed the way
drl_to_lua.disassemble parses it. Calls go through the call model of lift_common.py (the
Python's `Lifter.call` is the next step's port).
"""

import os
import re
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script's folder

import lift_common as lc  # noqa: E402


def parse_listing(text):
    """drl_to_lua.disassemble's parse of the objdump text."""
    return [(int(a, 16), t.strip()) for a, t in re.findall(r"^\s*([0-9a-f]+):\t(.*)$", text, re.M)]


def main(argv):
    tools, dll, listing, out_path = argv[1:5]
    dsl = lc.load_tool(tools)
    lc.install_call_model(dsl)
    with open(listing, encoding="utf-8", errors="replace", newline="") as fh:
        ins = parse_listing(fh.read())
    out = []
    try:
        pe = dsl.drl.Pe(dll)
        d = dsl.Dsl(dll, {}, pe=pe, ins=ins, exports=dsl.read_exports(pe))
    except dsl.DecodeError as e:
        out.append(f"new error {str(e)!r}")
        write(out_path, out)
        return
    out.extend(lc.dsl_facts(d))
    funcs = lc.function_list(d)
    out.append(f"functions {funcs!r}")
    lf = dsl.Lifter(d)
    try:
        for a in funcs:
            out.append(f"facts {a:#x} {lc.kind_facts(d, a)}")
            out.append(lc.canonical(lf.lift(a)).rstrip("\n"))
    except Exception:  # a raw exception aborts the real tool's run
        out.append("raw")
    out.append(lc.static_refs_line(d))
    write(out_path, out)


def write(path, lines):
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main(sys.argv)
