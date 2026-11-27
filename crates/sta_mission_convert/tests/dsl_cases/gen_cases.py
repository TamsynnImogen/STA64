#!/usr/bin/env python3
"""Writes expected.txt for cases.txt: what the ORIGINAL tools/dsl_to_lua.py produces for the
authored cases of the call handling and the emission (LUA-15h). THIS SCRIPT READS NO GAME FILES:
every input is an invented listing, an invented flat image and invented engine names written in
cases.txt. Deterministic: no clock, no randomness, no file but its three arguments.

    python3 -I gen_cases.py <repo>/tools cases.txt expected.txt

The committed expected.txt is the reference: it was made with CPython 3.14, the version the
converter's Python reference uses (its first line says so). The texts of raw exceptions differ
between CPython versions, so the staleness test in tests/dsl.rs compares only when the running
Python has the same major.minor version and says so when it skips.

The format is that of tests/dsl_lift/gen_pairs.py (read its docstring), with these changes:

    engine <Class> <mangled name>...       the vtable of an engine class, slot 0 first
    run lift <addr>                        the intermediate form, with the REAL `Lifter.call`
    run convert                            the Lua of the whole mission `m` (the text between the
                                           runtime prelude and the engine block)
    run render <addr>                      the Lua of one lifted function
    run expr <num|bool|arg> <literal>      render_expr of a hand-made tree (a Python literal)
    run lv <literal> / run stmt <literal>  render_lv / render_stmt of one
    run func <literal>                     render_function of a function holding these blocks
                                           (an exception of the Python is printed, the case goes on)
"""

try:
    import resource
    resource.setrlimit(resource.RLIMIT_AS, (2 * 1024**3, 2 * 1024**3))
except (ImportError, ValueError, OSError):
    pass
import ast
import os
import struct
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "dsl_lift"))  # -I drops the script's folder

import gen_pairs as gp  # noqa: E402
import lift_common as lc  # noqa: E402


TREE_RUNS = ("expr", "lv", "stmt", "func")


def parse(text):
    """gen_pairs.parse plus the `engine` directive."""
    engines, rest = {}, []
    cur = None
    for raw in text.split("\n"):
        line = raw.rstrip("\r")
        if line.startswith("case "):
            cur = line[5:]
            engines[cur] = {}
        if line.startswith("engine "):
            parts = line.split(" ")
            engines[cur][parts[1]] = parts[2:]
            continue
        rest.append(raw)
    cases = gp.parse("\n".join(rest))
    for c in cases:
        c["engine"] = engines[c["name"]]
    return cases


def run_case(dsl, case):
    gp.DecodeError = dsl.DecodeError
    DecodeError = dsl.DecodeError
    pe = gp.FakePe(case["base"], case["size"])
    pe.sections = list(case["sections"])
    ins = gp.listing(case)
    for a, b in case["puts"]:
        if isinstance(b, list):
            b = b"".join(struct.pack("<I", case["labels"][x[1:]] if x.startswith("@") else gp.num(x)) for x in b)
        pe.data[a - pe.base:a - pe.base + len(b)] = b
    exports = dict(case["exports"])
    engine = case["engine"]
    out = [f"== {case['name']}"]
    try:
        d = dsl.Dsl("", engine, pe=pe, ins=ins, exports=dict(exports))
    except DecodeError as e:
        out.append(f"new error {str(e)!r}")
        return "\n".join(out) + "\n"
    lf = dsl.Lifter(d)
    for r in case["runs"]:
        what, _, arg = r.partition(" ")
        out.append(f"-- {r}")
        try:
            if what == "lift":
                out.append(lc.canonical(lf.lift(gp.num(arg))).rstrip("\n"))
            elif what == "render":
                f = lf.lift(gp.num(arg))
                funcs, roles = {f.addr: f}, {f.addr: "helper"}
                nm = dsl.Namer(d, funcs, roles)
                out.extend(dsl.render_function(f, nm, nm.fn(f.addr)))
            elif what == "convert":
                d2 = dsl.Dsl("", engine, pe=pe, ins=ins, exports=dict(exports))
                orig = dsl.Dsl
                dsl.Dsl = lambda path, eng=None: d2
                try:
                    _, src, _ = dsl.convert("mS.dsl", None, engine)
                finally:
                    dsl.Dsl = orig
                body = src.split(dsl.PRELUDE, 1)[1].split(dsl.ENGINE, 1)[0]
                out.extend(body.rstrip("\n").split("\n"))
            elif what == "statics":
                out.append(lc.static_refs_line(d))
            elif what in TREE_RUNS:
                # a hand-made (possibly malformed) tree given to the renderer directly
                nm = dsl.Namer(d, {}, {})
                if what == "expr":
                    ctx, _, lit = arg.partition(" ")
                    out.append(dsl.render_expr(ast.literal_eval(lit), nm, ctx))
                elif what == "lv":
                    out.append(dsl.render_lv(ast.literal_eval(arg), nm))
                elif what == "stmt":
                    out.append(dsl.render_stmt(ast.literal_eval(arg), nm))
                else:  # func: a list of (label address, [statements])
                    f = dsl.Lifted(0x10001000)
                    f.blocks = ast.literal_eval(arg)
                    out.extend(dsl.render_function(f, nm, "fn"))
            else:
                raise SystemExit(f"cases.txt: unknown run {what!r}")
        except DecodeError as e:
            out.append(f"error {str(e)!r}")
        except Exception as e:  # a raw Python exception aborts the real tool's run
            out.append(f"raw {type(e).__name__} {str(e)!r}")
            if what not in TREE_RUNS:
                break
    out.append(lc.static_refs_line(d))
    return "\n".join(out) + "\n"


def main(argv):
    tools, cases_path, out_path = argv[1:4]
    dsl = lc.load_tool(tools)
    with open(cases_path, encoding="utf-8", newline="") as fh:
        cases = parse(fh.read())
    # The first line names the CPython that printed the exception texts below (they differ between
    # versions); the staleness test compares only under the same major.minor version.
    head = f"# generated with CPython {sys.version_info[0]}.{sys.version_info[1]}\n"
    text = head + "".join(run_case(dsl, c) for c in cases)
    with open(out_path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(text)


if __name__ == "__main__":
    main(sys.argv)
