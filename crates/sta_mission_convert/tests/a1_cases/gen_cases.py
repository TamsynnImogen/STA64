#!/usr/bin/env python3
"""Writes expected.txt for cases.txt: what the ORIGINAL tools/a1_to_lua.py computes for the
authored cases (LUA-15e). THIS SCRIPT READS NO GAME FILES: every input is an invented listing and
an invented flat image written in cases.txt. Deterministic.

    python3 -I gen_cases.py <repo>/tools cases.txt expected.txt

tests/a1.rs parses the same cases.txt and compares its own output with expected.txt.
"""

try:
    import resource

    resource.setrlimit(resource.RLIMIT_AS, (2 * 1024**3, 2 * 1024**3))
except (ImportError, ValueError, OSError):  # no `resource` (Windows) or the limit is refused (macOS)
    pass

import os  # noqa: E402
import re  # noqa: E402
import struct  # noqa: E402
import sys  # noqa: E402

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script's folder
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "dsl_lift"))

import lift_common as lc  # noqa: E402


def num(s):
    return int(s, 16) if s.startswith("0x") else int(s)


class Fixture:
    def __init__(self):
        self.consts = {}
        self.imports = []  # (dll, symbol)
        self.exports = []  # (symbol, expr)
        self.strings = {}
        self.words = []  # (address, [expr...]): 32-bit words in the image
        self.base = []  # (start, [lines])
        self.rbase = []
        self.cases = []
        self.iat = {}

    def expand(self, text):
        def rep(m):
            body = m.group(1)
            if body.startswith("iat:"):
                return f"DWORD PTR ds:{self.iat[body[4:]]:#x}"
            if "+" in body:
                n, k = body.split("+")
                return f"{self.consts[n] + int(k):#x}"
            return f"{self.consts[body]:#x}"
        return re.sub(r"\{([^}]+)\}", rep, text)

    def addr(self, text):
        return num(self.expand(text)) if "{" in text else num(text)


def parse(text, a1):
    fx = Fixture()
    cur = None
    block = None
    for raw in text.replace("\r", "").split("\n"):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        if raw.startswith("  "):
            block[1].append(raw[2:])
            continue
        word, _, rest = raw.partition(" ")
        if word == "const":
            n, v = rest.split(" ")
            fx.consts[n] = num(v)
        elif word == "import":
            dll, sym = rest.split(" ")
            slot = 0x10060000 + 4 * len(fx.imports)
            fx.imports.append((dll, sym))
            fx.iat[a1.drl.plain_name(sym) if not sym.startswith("??") else sym] = slot
        elif word == "export":
            sym, _, expr = rest.rpartition(" ")
            fx.exports.append((sym, expr))
        elif word == "string":
            a, _, s = rest.partition(" ")
            fx.strings[num(a)] = s
        elif word == "words":
            parts = rest.split(" ")
            fx.words.append((num(parts[0]), parts[1:]))
        elif word in ("base", "rbase"):
            block = (num(rest), [])
            (fx.base if word == "base" else fx.rbase).append(block)
        elif word == "case":
            cur = {"name": rest, "code": [], "rcode": [], "runs": []}
            fx.cases.append(cur)
        elif word in ("code", "rcode"):
            block = (fx.addr(rest), [])
            cur[word].append(block)
        elif word == "run":
            cur["runs"].append(rest)
        elif word == "end":
            cur = None
        else:
            raise SystemExit(f"cases.txt: unknown directive {word!r}")
    return fx


def make_pe_class(fx, a1):
    imports = {0x10060000 + 4 * i: (d, s) for i, (d, s) in enumerate(fx.imports)}
    strings = fx.strings
    image = bytearray(0x70000)  # indexed by address - base, like FlatImage
    for a, exprs in fx.words:
        for k, e in enumerate(exprs):
            struct.pack_into("<I", image, a - 0x10000000 + 4 * k, fx.addr(e))

    class FakePe:
        base = 0x10000000
        data = bytes(image)

        def __init__(self):
            self.imports = imports

        def off(self, rva):
            return rva if 0 <= rva < len(image) else None

        def section_of(self, addr):
            if 0x10001000 <= addr < 0x10030000:
                return ".text"
            if 0x10040000 <= addr < 0x10041000 or 0x10060000 <= addr < 0x10061000:
                return ".rdata"
            if 0x10050000 <= addr < 0x10051000:
                return ".data"
            return None

        def cstr(self, addr):
            if addr in strings:
                return strings[addr]
            raise a1.DecodeError("no string")

        def f32(self, addr):
            return 1.5

        def f64(self, addr):
            return 1.5

    return FakePe


def program(code):
    """4 bytes per instruction from each start; `@chain N` is N blocks of `test eax,eax` and a
    `je` to the next block."""
    ins = []
    for start, lines in sorted(code.items()):
        a = start
        for line in lines:
            if line.startswith("@chain "):
                for _ in range(int(line.split(" ")[1])):
                    ins.append((a, "test   eax,eax"))
                    ins.append((a + 4, f"je     {a + 8:#x}"))
                    a += 8
                continue
            ins.append((a, line))
            a += 4
    return ins


def build(fx, a1, FakePe, blocks, base_blocks, name, exports, ep):
    code = {}
    for start, lines in base_blocks:
        code[start] = [fx.expand(x) for x in lines]
    for start, lines in blocks:
        code[start] = [fx.expand(x) for x in lines]
    return a1.A1Dll(name, ins=program(code), pe=FakePe(), exports=exports, entry_point=0x10001100)


def lifted_dump(a1, d, f):
    out = lc.canonical(f).rstrip("\n").split("\n")
    out.append(f" undecoded {list(f.undecoded)!r}")
    out.append(f" rules_made {f.rules!r}")
    out.append(f" imports_called {f.imports_called!r}")
    out.append(f" classify {a1.classify(f)!r}")
    dsl = a1.dsl
    try:
        nm = dsl.Namer(d, {f.addr: f}, {f.addr: "step"})
        body = "\n".join(dsl.render_function(f, nm, "fn"))
        out.append("render ok")
        out.append(body)
    except Exception as e:  # noqa: BLE001
        out.append(f"render error {type(e).__name__} {str(e)!r}")
    return out


def report_dump(rep):
    fn = [(k, [v["total"], v["complete"], v["partial"], v["not"]]) for k, v in rep["functions"].items()]
    return [
        f"mission {rep['mission']!r}", f"map {rep['map']!r}", f"rules {rep['rules']}",
        f"rules_with_deps {rep['rules_with_deps']}", f"rule_actions_resolved {rep['rule_actions_resolved']}",
        f"registration_errors {rep['registration_errors']!r}",
        f"registration_calls {list(rep['registration_calls'].items())!r}",
        f"actions {rep['actions']}", f"functions {fn!r}",
        f"undecoded_calls {list(rep['undecoded_calls'].items())!r}",
        f"leftovers {list(rep['leftovers'].items())!r}",
        f"engine_calls {list(rep['engine_calls'].items())!r}", f"init {rep['init']!r}",
    ]


def run_case(fx, a1, FakePe, case):
    out = [f"== {case['name']}"]
    exports = {sym: fx.addr(expr) for sym, expr in fx.exports}
    for r in case["runs"]:
        what, _, arg = r.partition(" ")
        out.append(f"-- {r}")
        if what == "lift":
            d = build(fx, a1, FakePe, case["code"], fx.base, "Test1S.dsl", exports, None)
            lf = a1.A1Lifter(d)
            try:
                f = lf.lift(fx.addr(arg))
            except a1.DecodeError as e:
                out.append(f"raise {str(e)!r}")
                continue
            out.extend(lifted_dump(a1, d, f))
        elif what == "try_convert":
            S = build(fx, a1, FakePe, case["code"], fx.base, "Test1S.dsl", exports, None)
            try:
                a1.convert(S, None, "test1")
                out.append("converted")
            except Exception as e:  # noqa: BLE001 - main's per-mission catch
                out.append(f"FAIL {type(e).__name__} {str(e)!r}")
        elif what == "mission":
            S = build(fx, a1, FakePe, case["code"], fx.base, "Test1S.dsl", exports, None)
            R = None
            if case["rcode"]:
                R = build(fx, a1, FakePe, case["rcode"], fx.rbase, "Test1R.drl", {}, None)
            mission, src, rep = a1.convert(S, R, "test1")
            head, body = src.split("\n", 1)
            pre, rest = body.split(a1.a1_prelude(), 1)
            _, after = rest.split(a1.A1_PRELUDE, 1)
            mid, _ = after.split(a1.dsl.ENGINE, 1)
            out.append(f"mission {mission!r}")
            out.append("prefix")
            out.append(pre.rstrip("\n"))
            out.append("body")
            out.append(mid.rstrip("\n"))
            out.extend(report_dump(rep))
        else:
            raise SystemExit(f"cases.txt: unknown run {what!r}")
    return "\n".join(out) + "\n"


def main(argv):
    tools, cases_path, out_path = argv[1:4]
    # The tool's own `main` calls `convert` two frames below the module; here `run_case` adds one,
    # so the limit is one higher and the `RecursionError` cases fail at the same chain length.
    sys.setrecursionlimit(sys.getrecursionlimit() + 1)
    sys.path.insert(0, tools)
    import a1_to_lua as a1  # noqa: E402
    with open(cases_path, encoding="utf-8", newline="") as fh:
        fx = parse(fh.read(), a1)
    FakePe = make_pe_class(fx, a1)
    with open(out_path, "w", encoding="utf-8", newline="\n") as fh:
        for c in fx.cases:
            fh.write(run_case(fx, a1, FakePe, c))


if __name__ == "__main__":
    main(sys.argv)
