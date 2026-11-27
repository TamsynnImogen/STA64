"""Writes the expected-value tables of crates/sta_mission_convert/tests/core.rs (LUA-15c).

Runs the ORIGINAL Python functions of tools/{drl,dsl,a1}_to_lua.py on inputs invented here (made-up
symbols, made-up expression trees, random numbers from a fixed seed) and writes input/expected
pairs as Rust constants. It NEVER reads any game data or any file but the tools' own source.
Deterministic: the same Python version gives the same file (the texts of some Python exceptions
differ between versions; the committed table was made with CPython 3.14, and its first line says
so).

Usage (from the repository root):
    python3 -I crates/sta_mission_convert/tests/core_data/gen_tables.py . [output.rs]
The output defaults to crates/sta_mission_convert/tests/core_data/tables.rs next to this script.
It takes a second. `tests/core.rs` has a test that runs it into a temp file and compares it with
the committed table (skipped without python3).

Conventions: an input is the Python `repr` of a tuple of arguments (a literal the Rust test parses),
the expected value is the Python `repr` of the result, `!DecodeError:<message>` for a DecodeError
and `!raw` for any other exception (table RAW_TEXT: `!raw:<class>:<text>`).
"""
import os
import random
import struct
import sys

ROOT = sys.argv[1]
DEFAULT_OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "tables.rs")
OUT_PATH = sys.argv[2] if len(sys.argv) > 2 else DEFAULT_OUT
sys.path.insert(0, ROOT + "/tools")
import drl_to_lua as drl  # noqa: E402
import dsl_to_lua as dsl  # noqa: E402
import a1_to_lua as a1  # noqa: E402

rng = random.Random(20261127)
OUT = []


def rs_lit(s):
    out = ['"']
    for c in s:
        o = ord(c)
        if c == "\\":
            out.append("\\\\")
        elif c == '"':
            out.append('\\"')
        elif c == "\n":
            out.append("\\n")
        elif c == "\r":
            out.append("\\r")
        elif c == "\t":
            out.append("\\t")
        elif 0x20 <= o < 0x7F:
            out.append(c)
        else:
            out.append("\\u{%x}" % o)
    out.append('"')
    return "".join(out)


def run(f, *args):
    try:
        return repr(f(*args))
    except drl.DecodeError as e:
        return "!DecodeError:" + str(e)
    except Exception:
        return "!raw"


def table(name, rows, doc):
    OUT.append(f"/// {doc} ({len(rows)} pairs)")
    OUT.append(f"pub const {name}: &[(&str, &str)] = &[")
    for inp, exp in rows:
        OUT.append(f"    ({rs_lit(inp)}, {rs_lit(exp)}),")
    OUT.append("];")
    OUT.append("")
    COUNTS[name] = len(rows)


COUNTS = {}

# ---------------------------------------------------------------- python values

pool = [
    None, True, False, 0, 1, -1, 2, 7, 255, 2**40, -(2**40), 0.0, -0.0, 1.0, 0.5, 2.5, -2.5, 1e20, 1e-7,
    0.30000000000000004, "", "a", "ab", "B", "a'b", (), (1,), (1, 2), (1.0, 2), (1, 2, 3), ("a", 1), ("a", 2),
    ("b", 1), [], [1], [1, 2], [1.0, 2], ["a"], (None,), ((1, 2), 3), ((1, 2), 4), (True, "x"), ("const", 1),
]
rows = []
pairs_ = [(a, b) for a in pool for b in pool]
rng.shuffle(pairs_)
for a, b in pairs_[:420]:
    if True:
        eq = a == b
        try:
            lt = a < b
        except TypeError:
            lt = "!raw"
        rows.append((repr((a, b)), repr((eq, lt))))
# whole-number floats near 2**63 against ints (equality, ordering and, in Rust, hashing)
edge = [9.2e18, 9200000000000000000, 2**62, float(2**62), 2**63 - 1, float(2**63), -(2**63), -float(2**63), 9.223372036854775e18,
        9223372036854774784, 9223372036854775807, 1e19, -9.2e18, 4611686018427387904.0]
for a in edge:
    for b in edge:
        rows.append((repr((a, b)), repr((a == b, a < b))))
table("PY_EQ_LT", rows, "`(a, b)` -> `(a == b, a < b or '!raw')`")

reprs = [
    None, True, False, 0, -7, 2**40, 1.0, -0.0, 0.1, 1e16, 1e15, 1.5e-7, 123456789.123, 1e100, float("inf"),
    float("-inf"), float("nan"), "", "x", "it's", 'say "hi"', "both ' and \"", "tab\there", "nl\n", "back\\slash",
    "\x00\x1f\x7f", "\x80\xa0\xad\xe9", (), (1,), (1, 2), ("a", (1, [2, 3])), [], [()], [(1,)], ["a", 2.5, None],
    ("cmp", "<", ("var", "x", "hot_int"), ("const", 1)), ("and", [("not", ("truth", ("obj", "y"))), ("const", 0)]),
]
table("PY_REPR", [(repr((x,)), repr((repr(x), str(x), bool(x)))) for x in reprs],
      "`x` -> `(repr(x), str(x), bool(x))`")

# sorting
sort_pool = [0, 1, -1, 2, 3, 2.5, -0.5, 1.0, True, False, 0.0, 10, "a", "b", "ab", "B", "", "é",
             (1, "a"), (1, "b"), (2, "a"), (1,), (1, 2), (1.0, "a"), ("a", 1), ("a", 2, 3), ("b",), (),
             [1], [1, 2], [0, 5]]
srows = []
for _ in range(220):
    kind = rng.choice(["num", "str", "tup", "tupnum", "list", "mixed"])
    if kind == "num":
        src = [x for x in sort_pool if isinstance(x, (int, float, bool))]
    elif kind == "str":
        src = [x for x in sort_pool if isinstance(x, str)]
    elif kind == "tup":
        src = [x for x in sort_pool if isinstance(x, tuple) and x[:1] and not isinstance(x[0], str)] + [(1, "z"), (0, "q")]
    elif kind == "tupnum":
        src = [(rng.choice([0, 1, 2, 1.0, True]), rng.choice(["a", "b", "c"])) for _ in range(6)]
    elif kind == "list":
        src = [x for x in sort_pool if isinstance(x, list)] + [[0], [1, 1], [1, 0]]
    else:
        src = sort_pool
    n = rng.randint(0, 9)
    lst = [rng.choice(src) for _ in range(n)]
    srows.append((repr((lst,)), run(sorted, lst)))
for lst in ([], [3, 1, 2], [1, True, 1.0, 0, False, 0.0, 2], [(1, 2), (1, 2.0), (1.0, 2), (True, 2)],
            [(2, "b"), (1, "z"), (2, "a"), (1, "a")], [1, "a"], [(1, 2), (1, "a")], [(1,), [1]], [None, None]):
    srows.append((repr((lst,)), run(sorted, lst)))
table("PY_SORT", srows, "`(list,)` -> `sorted(list)` (stable, Python `<`)")

# ---------------------------------------------------------------- floats
vals = []
for e in range(-330, 310, 5):
    for m in ("1", "1.5", "2.5", "3.14159", "9.999999999999999", "4.94065645841246544", "1.7976931348623157", "5"):
        vals.append(float(f"{m}e{e}"))
for k in range(0, 24):
    for d in (0, 1, -1, 2):
        vals.append(float(10**k + d))
        vals.append(float(2**k + d))
vals += [999999999999999.0, 1e15, 1e15 + 2, 999999999999998.5, 9007199254740993.0, 1e16, 1.5e16, 123456789012345678.0,
         0.1, 0.2, 0.3, 1 / 3, 2 / 3, 1e-4, 9.9999e-5, 0.0001, 0.00011, 123.456, 100.0, 1e2, 12345.0, 0.5, 1.25,
         0.1 + 0.2, 1e22, 1e23, 8.41e21, 5e-324, 2.2250738585072014e-308, 1.7976931348623157e308, 0.0, -0.0,
         float("inf"), float("-inf"), float("nan"), 3.4028234663852886e38, 1e-45, 16777216.0, 16777217.0]
for _ in range(180):
    vals.append(struct.unpack("<f", struct.pack("<f", rng.uniform(-1e6, 1e6)))[0])
    vals.append(struct.unpack("<f", struct.pack("<I", rng.getrandbits(32)))[0])
for _ in range(260):
    vals.append(rng.uniform(-1000, 1000))
    vals.append(float(rng.randint(-10**9, 10**9)))
    vals.append(rng.randint(1, 10**6) / 10 ** rng.randint(0, 9))
for _ in range(220):
    vals.append(struct.unpack("<d", struct.pack("<Q", rng.getrandbits(64)))[0])
bits = []
seen = set()
for x in vals:
    b = struct.unpack("<Q", struct.pack("<d", x))[0]
    if b not in seen:
        seen.add(b)
        bits.append(b)
rng.shuffle(bits)
bits = bits[:1000]
for x in (0.0, -0.0, float("inf"), float("-inf"), float("nan"), 5e-324, 1e16, 1e15, 0.1 + 0.2):
    b = struct.unpack("<Q", struct.pack("<d", x))[0]
    if b not in bits:
        bits.append(b)
# exact powers of two: the rounding interval is asymmetric there (the nearest decimal of the
# shortest length may not read back), e.g. 2**-1016
import math  # noqa: E402

for k in list(range(-1074, 1024, 17)) + [-1016, -1074, 1023, 52, 53, 54, 63, 64]:
    for sign in ((1.0, -1.0) if k % 5 == 0 else (1.0,)):
        b = struct.unpack("<Q", struct.pack("<d", sign * math.ldexp(1.0, k)))[0]
        if b not in bits:
            bits.append(b)
OUT.append(f"/// `(bits, repr(x), lua_num(x))` for floats `x` ({len(bits)} values)")
OUT.append("pub const FLOAT_BITS: &[(u64, &str, &str)] = &[")
for b in bits:
    x = struct.unpack("<d", struct.pack("<Q", b))[0]
    OUT.append(f"    (0x{b:016x}, {rs_lit(repr(x))}, {rs_lit(drl.lua_num(x))}),")
OUT.append("];")
OUT.append("")
COUNTS["FLOAT_BITS"] = len(bits)

# lua_num on non-floats
lrows = [(repr((x,)), run(drl.lua_num, x)) for x in (0, 1, -1, 2**31, -(2**31), 12345, True, False, "abc", "", None, (1, 2), [1])]
table("LUA_NUM_OTHER", lrows, "`(x,)` -> `lua_num(x)` for non-floats")

# hex
hv = [0, 1, -1, 4, -4, 15, 16, 255, -255, 256, 4096, 0x7FFFFFFF, -0x80000000, 0x100000000, 2**62, -(2**62), 2**63 - 1,
      -(2**63) + 1, 0x401000, 0x10001234]
for _ in range(80):
    hv.append(rng.randint(-(2**40), 2**40))
    hv.append(rng.randint(-300, 300))
table("HEX", [(repr((n,)), repr((f"{n:#x}", f"{n:x}"))) for n in hv], "`(n,)` -> `(f'{n:#x}', f'{n:x}')`")

# strings
srep = ["", "a", "plain text 123", "it's", 'say "hi"', "both ' and \"", "\\", "a\\b", "''", '""', "\n", "\r\n", "\t",
        "x\ny\tz", "\x00", "\x1f", "\x7f", "caf\xe9", "\xff", "\xa0", "\xad", "\x85", "µ",
        "a'b\\c'", "'\\'", "line1\nline2", "O'Brien \"quoted\""]
for i in range(256):
    srep.append(chr(i))
table("STR", [(repr((s,)), repr((repr(s), drl.lua_str(s), len(s)))) for s in srep],
      "`(s,)` -> `(repr(s), lua_str(s), len(s))`")

idents = ["", "a", "_", "_a1", "A_b", "1a", "9", "a b", "a-b", "a.b", "a\n", "\xe9a", "a\xe9", "ab12", "x$", "Mission_Win",
          "²", "a²", " ", "a ", "-1", "p_STAT", "ScriptGet_x"]
table("LUA_IDENT", [(repr((s,)), repr((drl.lua_ident(s), a1.lua_ident(s)))) for s in idents],
      "`(s,)` -> `(drl.lua_ident(s), a1.lua_ident(s))`")

# ---------------------------------------------------------------- integers and strings
irows = []
nums = [0, 1, -1, 2, -2, 3, -3, 4, -4, 5, 7, -7, 8, -8, 255, -255, 256, 1000, -1000, 2**31, -(2**31), 2**40, -(2**40)]
for a in nums:
    for b in (1, -1, 2, -2, 3, 4, -4, 7, -7, 0, 256, 2**31):
        irows.append((repr((a, b)), run(lambda a, b: (a // b, a % b), a, b)))
table("INT_DIV", irows, "`(a, b)` -> `(a // b, a % b)`")

ints = ["0", "1", "-1", "+5", " 12 ", "\t7\n", "0x1f", "0X1F", "-0x4", "+0x4", "0x", "0x_1f", "1_000", "1__0", "_1", "1_",
        "ff", "FF", "0xFFFFFFFF", "0x100000000", "12", "-", "", " ", "1 2", "0b1", "0o7", "007", "a", "0xg", "922337203685477",
        "-922337203685477", "0x7ffffffffffffff", "²", "1\x1c", "\x1f9"]
irows = []
for s in ints:
    for base in (10, 16):
        irows.append((repr((s, base)), run(int, s, base)))
table("INT_PARSE", irows, "`(s, base)` -> `int(s, base)`")

ws = ["", " ", "  a  ", "a b", "a  b  c", "a   ", "   a b  ", "a\tb", "a\x1cb", "\x1ca\x1d", "\x85a\xa0", "a\xa0b", "\xa0",
      "one", "one two", "one two three four", " lead", "trail ", "\n\t x \r\n", "a b", "A\xdfB", "\xff", "\xb5", "\xe9\xc9",
      "MiXeD 123", "\xc0\xe0", "I\xcf", "ABC\xdf", "İ", "Σ", "aΣ", "x\x1fy z", "0123456789", "\xb2\xb3\xb9"]
srows = []
for s in ws:
    srows.append((repr((s,)), repr((s.strip(), s.split(None, 1), s.lower(), s.upper(), [c.isdigit() for c in s],
                                    [c.isspace() for c in s]))))
table("STRMETH", srows, "`(s,)` -> `(strip, split(None, 1), lower, upper, per-char isdigit, per-char isspace)`")

slrows = []
for s in ("", "a", "abc", "abcdef", "xy"):
    for a_ in (0, 1, 2, 3, 5, 9):
        for b_ in (0, 1, 2, 3, 6, 9):
            slrows.append((repr((s, a_, b_)), repr(s[a_:b_])))
table("SLICE", slrows, "`(s, a, b)` -> `s[a:b]`")

# dict order
drows = []


def dict_ops(ops):
    d = {}
    try:
        for op in ops:
            if op[0] == "set":
                d[op[1]] = op[2]
            elif op[0] == "del":
                del d[op[1]]
            elif op[0] == "setdefault":
                d.setdefault(op[1], op[2])
            elif op[0] == "pop":
                d.pop(op[1], None)
    except KeyError:
        return "!raw"
    return repr(list(d.items()))


keys = ["a", "b", "c", "d", 1, 2, 1.0, (1, 2)]
for _ in range(120):
    ops = []
    for _ in range(rng.randint(1, 12)):
        o = rng.choice(["set", "set", "del", "setdefault", "pop"])
        k = rng.choice(keys)
        ops.append((o, k, rng.randint(0, 9)) if o in ("set", "setdefault") else (o, k))
    drows.append((repr((ops,)), dict_ops(ops)))
table("DICT_OPS", drows, "`(ops,)` -> `list(d.items())` after the operations, or `!raw` for a KeyError")

mrows = []
for _ in range(60):
    pairs = [(rng.choice("abcdef"), rng.randint(0, 4)) for _ in range(rng.randint(0, 8))]
    d = dict(pairs)
    mrows.append((repr((pairs,)), run(lambda d: max(d, key=d.get), d)))
table("FIRST_MAX", mrows, "`(pairs,)` -> `max(d, key=d.get)` of `dict(pairs)`")

# ---------------------------------------------------------------- msvc
SI = [
    "?ScriptCreate_hot_int@ScriptInterfaceImp@@UAEPAVhot_int@@PAD@Z", "?GetHandle@ScriptInterfaceImp@@UAEHPAD@Z",
    "?MoveTo@ScriptInterfaceImp@@UAEXHPAD@Z", "?IsAlive@ScriptInterfaceImp@@UAE_NH@Z", "?SetSpeed@ScriptInterfaceImp@@UAEXHM@Z",
    "?Enable@ScriptInterfaceImp@@UAEXH_N@Z", "?SetAngularVelocity@ScriptInterfaceImp@@UAEXHAAVVector3@@@Z",
    "?GetDistance@ScriptInterfaceImp@@UAEMAAH0@Z", "?RecruitSpecialForces@ScriptInterfaceImp@@UAE_NPADHAAH@Z",
    "?BuildAt@ScriptInterfaceImp@@UAEHPADH0H@Z", "?SetText@ScriptInterfaceImp@@UAEXPAD@Z",
    "?UnderAttack@ScriptInterfaceImp@@UAE_NHM@Z", "?GetTime@ScriptInterfaceImp@@UAEMXZ",
    "?GetAttackLocation@@YA?AVVector3@@HH@Z", "?f@@YAPAVa@@PAVb@@0@Z", "?GetDistance@@YAMAAH0@Z", "??4hot_int@@QAEXH@Z",
    "??Bhot_int@@QAEHXZ", "?create_rule@IIIE@@QAEPAVrule@@P6AHXZP6AXXZPAD@Z", "?ship@@3HA", "?p_A@@3PAVtimer_int@@A",
    "?Enterprise@@3HA", "?Mission_Win@@YAXXZ",
]
prim = ["H", "M", "N", "X", "D", "E", "_N", "_J", "_K", "I", "K", "G", "F", "C", "_W", "PAD", "PAH", "PAM", "AAH", "ABH",
        "QAH", "PBD", "PAVfoo@@", "PBVfoo@@", "AAVbar@@", "VVector3@@", "UQux@@", "TUni@@", "W4Color@@", "0", "1", "2", "9"]
names = ["Alpha", "beta_2", "_x", "Gamma", "p_Var", "A1", "ScriptCreate_thing", "operator_x"]
classes = ["Widget", "ScriptInterfaceImp", "hot_int", "IIIE", "Foo_1"]


def rand_params():
    return "".join(rng.choice(prim) for _ in range(rng.randint(0, 4)))


syms = list(SI)
for _ in range(180):
    n = rng.choice(names)
    c = rng.choice(classes)
    ret = rng.choice(prim)
    p = rand_params() or "X"
    form = rng.choice(["mem", "mem", "free", "var", "op", "oddret"])
    if form == "mem":
        syms.append(f"?{n}@{c}@@{rng.choice('QU')}A{rng.choice('EG')}{ret}{p}@Z")
    elif form == "free":
        syms.append(f"?{n}@@Y{rng.choice('AGI')}{ret}{p}Z")
    elif form == "var":
        syms.append(f"?{n}@@3{rng.choice(['PAV' + rng.choice(['hot_int', 'hot_float', 'dirty_int', 'timer_int', 'character', 'ScriptInterface', 'zzz']) + '@@A', 'HA', 'PAHA', 'MA', 'PAMA', '_NA', 'VVector3@@A', 'V' + rng.choice(['hot_int', 'character', 'ScriptInterface']) + '@@A'])}")
    elif form == "op":
        syms.append(f"??{rng.choice('4B_5')}{c}@@{rng.choice('QU')}A{rng.choice('EG')}{ret}{p}@Z")
    else:
        syms.append(f"?{n}@{c}@@{rng.choice('QU')}A{rng.choice('EG')}{rng.choice(['?AV', '?BV', 'PAV', 'PBU', 'AAV', 'ABU', 'AA', 'PA'])}{rng.choice(['x@@', 'Vector3@@', 'q'])}{p}@Z")
# hostile / malformed
syms += ["", "?", "??", "?a", "?a@", "?a@b@@", "?a@b@@QAE", "?a@b@@QAEH", "?a@b@@QAEH@Z\n", "?a@b@@QAEH@\nZ", "?a@b@@QAEHH\n\n",
         "?a@@YAH\n", "?a@@YA", "?a@@YAH", "?a@b@@QAEPA", "?a@b@@QAEPAV", "?a@b@@QAEPAVx", "?a@b@@QAEH\xb2@Z", "?a@b@@QAEH\xb2\xb3@Z",
         "?a@b@@QAEHH5@Z", "?a@b@@QAEHP6AHXZ@Z", "?a@b@@QAEHP6AHX", "?a@b@@QAEH_", "?a@b@@QAE?", "?a@b@@QAE?AV", "?a@b@@QAE?AVx@@H@Z",
         "?a@@3PAVhot_int@@A\n", "?a@@3HA\n", "?a@@YAXXZ\n", "x@@YAXXZ", "?\xe9a@@YAXXZ", "?a\xe9@@YAXXZ", "?a@b\xe9@@QAEXXZ",
         "??4a\xe9@@QAEXH@Z", "??9a@@QAEXH@Z", "??_Ga@@QAEXH@Z", "??Za@@QAEXH@Z", "?a@@YIXH@Z", "?a@@YGXH@Z", "?a@@YGXHM@Z", "?a@b@@QBEXXZ",
         "?a@b@@AAEXXZ", "?a@b@@UAEPAVhot_int@@PAD@Z", "?a@@3_NA", "?a@@3PAMA", "?a@@3VVector3@@A", "?a@@3PAHA", "?a@@3MA"]
mrows = []
for s in syms:
    try:
        ret = dsl.msvc_ret(s)
    except Exception:
        ret = "!raw"
    sig = a1.signature(s)
    mrows.append((repr((s,)), repr((drl.plain_name(s), drl.import_kind(s), dsl.data_kind(s), drl.msvc_params(s), ret, sig))))
table("MSVC", mrows, "`(sym,)` -> `(plain_name, import_kind, data_kind, msvc_params, msvc_ret or '!raw', signature)`")

prow = []
tt = ["", "H", "HH", "XZ", "X", "HX", "_NH@Z", "PAD", "PAVfoo@@HH", "HM0", "HAAH0", "?AVVector3@@HH@Z", "?AVVector3@@VVector3@@0@Z",
      "P6AHXZ", "P6AHXZP6AXXZPAD@Z", "HP6AHXZH@Z", "HP6AHX", "H0", "H1", "H\xb2", "PAV", "P", "PA", "_", "?", "?A", "?AV", "VVector3",
      "H@Z", "MZ", "HHH@Z", "PAVa@@PAVb@@0@Z", "PAVa@@PAVb@@1@Z", "HW4Color@@0@Z", "HUQux@@T@@", "XH", "HXH", "AAH0", "AAH1@Z", "BAH"]
for _ in range(120):
    tt.append(rng.choice(prim + ["?AVfoo@@"]) + rand_params() + rng.choice(["@Z", "Z", "", "@"]))
for s in tt:
    try:
        r = repr(a1.parse_types(s))
    except ValueError:
        r = "!Value"
    except IndexError:
        r = "!Index"
    prow.append((repr((s,)), r))
table("PARSE_TYPES", prow, "`(s,)` -> `parse_types(s)` or `!Value` / `!Index`")

orow = []
for t in ["N", "H", "M", "_J", "_K", "_N", "VVector3@@", "VVector3", "PAD", "VFoo@@", "", "X"]:
    orow.append((repr(("push", t)), repr(dsl.push_count(t))))
for params, ret in ((["H", "H"], "?AVVector3@@"), (["H", "VVector3@@"], "X"), ([], "X"), (["N", "_J", "H"], "?AVQ@@"), (["_K"], "H"),
                    ([], "?A"), (["VVector3@@", "VVector3@@"], "?AVVector3@@")):
    orow.append((repr(("slots", params, ret)), repr(a1.slots_of(params, ret))))
table("PUSH_SLOTS", orow, "`('push', t)` -> `push_count(t)`; `('slots', params, ret)` -> `slots_of`")

# ---------------------------------------------------------------- algebra
T = ("const", 1)
F = ("const", 0)
A = [
    ("cmp", "<", ("var", "a", "hot_int"), ("const", 1)),
    ("cmp", ">=", ("var", "a", "hot_int"), ("const", 1)),
    ("cmp", "==", ("obj", "b"), ("const", 0)),
    ("cmp", "~=", ("obj", "b"), ("const", 0)),
    ("truth", ("var", "c", "hot_float")),
    ("truth", ("call", "Probe", [("const", 3), ("float", 0.5)])),
    ("cmp", ">", ("var", "d", "timer_int"), ("float", 2.5)),
    ("cmp", "<=", ("var", "d", "timer_int"), ("float", 2.5)),
    ("cmp", "==", ("float", 0.0), ("var", "e", "hot_int")),
    ("cmp", "==", ("float", -0.0), ("var", "e", "hot_int")),
]


def gen(depth):
    r = rng.random()
    if depth <= 0 or r < 0.35:
        c = rng.random()
        if c < 0.08:
            return rng.choice([T, F])
        return rng.choice(A)
    if r < 0.5:
        return ("not", gen(depth - 1))
    n = rng.choice([2, 2, 3])
    kind = rng.choice(["and", "or"])
    items = [gen(depth - 1) for _ in range(n)]
    return (kind, items)


def gen_wild(depth):
    r = rng.random()
    if depth <= 0 or r < 0.25:
        return rng.choice(A + [("truth", ("const", 1)), ("truth", ("const", 0)), ("truth", ("boolv", rng.choice(A))),
                               ("cmp", "==", ("boolv", rng.choice(A)), ("const", 0)),
                               ("cmp", "~=", ("boolv", rng.choice(A)), ("const", 0)),
                               ("cmp", "==", ("boolv", ("or", [rng.choice(A), rng.choice(A)])), ("const", 0)),
                               ("cmp", "~=", ("boolv", T), ("const", False)), T, F])
    if r < 0.4:
        return ("not", gen_wild(depth - 1))
    kind = rng.choice(["and", "or", "or"])
    return (kind, [gen_wild(depth - 1) for _ in range(rng.choice([1, 2, 2, 3]))])


trees = [gen(3) for _ in range(90)] + [gen_wild(3) for _ in range(40)]
# a shape the compiler's `||` produces: a or (not a and b)
for _ in range(12):
    a = rng.choice(A)
    b = rng.choice(A)
    trees.append(("or", [a, ("and", [("not", a), b])]))
    trees.append(("or", [("and", [rng.choice(A), a]), ("and", [rng.choice(A), b])]))
    p = rng.choice(A)
    trees.append(("or", [("and", [p, a]), ("and", [p, b]), ("and", [p, rng.choice(A)])]))
many = [("cmp", "<", ("var", f"v{i}", "hot_int"), ("const", i)) for i in range(17)]
trees += [("and", many), ("or", many), ("or", many[:14]), ("or", many[:15]), ("and", [many[0], ("not", many[0])]),
          ("or", [many[0], ("not", many[0])])]
trees += [("truth", ("conflict",)), ("truth", ("boolv", ("conflict",))), ("not", ("truth", ("conflict",))), ("cmp", "==", None, F),
          ("cmp", "?", ("const", 1), ("const", 1)), ("and", []), ("or", []), ("and", [T]), ("or", [F, F]), ("const", 1),
          ("var", "x", "hot_int"), ("arith", "+", ("const", 1), ("const", 2)), ("and", 5), (), ("and",), ("cmp", "<"), 5, None]


def rows_for(f, items, wrap=lambda x: (x,)):
    return [(repr(wrap(x)), run(f, *wrap(x))) for x in items]


table("SIMPLIFY", rows_for(drl.simplify, trees), "`(e,)` -> `simplify(e)`")
table("NEGATE", rows_for(drl.negate, [rng.choice(trees[:100]) for _ in range(40)] + [("cmp", op, ("const", 1), ("const", 2)) for op in
                                                                                       ("==", "~=", "<", ">=", "<=", ">", "x")] + [None, (), ("cmp", "<")]),
      "`(p,)` -> `negate(p)`")
table("AS_BOOL", rows_for(drl.as_bool, [rng.choice(trees[:100]) for _ in range(25)] + [
    ("bool", ("bool", ("var", "x", "hot_int"))), ("boolv", F), ("conflict",), ("arith", "-", ("const", 1), ("obj", "z")),
    ("arith", "+", ("const", 1), ("obj", "z")), ("var", "x", "hot_int"), ("bool", ("cmp", "<", ("const", 1), ("const", 2))),
    ("const", 3), ("float", 1.5), ("m1", ("const", 1)), None, (), 5]),
      "`(v,)` -> `as_bool(v)`")
table("TRUTH", rows_for(dsl.truth, [rng.choice(trees[:100]) for _ in range(15)] + [("var", "x", "hot_int"), ("tmp", 3), None, ()]),
      "`(v,)` -> `dsl.truth(v)`")
table("ATOM_KEY", rows_for(drl.atom_key, A + [("cmp", op, ("float", 1.5), ("float", -0.0)) for op in ("==", "~=", "<", ">=", "<=", ">")] +
                           [("var", "x", "k"), ("truth", ("const", 1)), ("cmp", ">="), ("cmp", "?", 1, 2), None]),
      "`(e,)` -> `atom_key(e)`")


def f_atoms(e):
    out = drl.atoms(e, [])
    return out


table("ATOMS", rows_for(f_atoms, [rng.choice(trees[:100]) for _ in range(30)] + [many[:3] and ("and", many[:3]), ("const", 1), ("not", ("const", 0))] + [None, (), 5]),
      "`(e,)` -> `atoms(e, [])`")
table("TRUTH_TABLE", rows_for(drl.truth_table, trees[:60] + [("or", [A[0], A[1]]), ("and", [A[0], A[1]]), ("or", [A[2], A[3]]), T, F]),
      "`(e,)` -> `truth_table(e)` (limit 14)")
table("TRUTH_TABLE_LIMIT", [(repr((e, lim)), run(drl.truth_table, e, lim)) for e in (("or", many[:5]), ("and", [many[0], ("not", many[0])])) for lim in (0, 1, 2, 4, 5, 6)],
      "`(e, limit)` -> `truth_table(e, limit)`")

erows = []
for e in [rng.choice(trees[:100]) for _ in range(50)]:
    names_ = drl.atoms(e, [])
    env = [(n, bool(rng.getrandbits(1))) for n in names_]
    if env and rng.random() < 0.15:
        env = env[:-1]  # a missing key: KeyError
    erows.append((repr((e, env)), run(lambda e, env: drl.evaluate(e, dict(env)), e, env)))
table("EVALUATE", erows, "`(e, env_pairs)` -> `evaluate(e, dict(env_pairs))`")

lists = []
for _ in range(40):
    lists.append(rng.choice([[rng.choice(A) for _ in range(rng.randint(0, 3))], [rng.choice(trees[:100]) for _ in range(rng.randint(0, 3))]]))
lists += [[], [T], [F, F], [("not", A[0]), A[0]]]
table("CONJ", rows_for(drl.conj, lists), "`(preds,)` -> `conj(preds)`")
alists = []
for _ in range(50):
    n = rng.randint(1, 4)
    alists.append([rng.choice([rng.choice(A), ("and", [rng.choice(A), rng.choice(A)]), ("and", [("not", rng.choice(A)), rng.choice(A)]),
                               ("not", rng.choice(A)), T]) for _ in range(n)])
alists += [[A[0], ("and", [("not", A[0]), A[2]])], [("and", [A[1], A[0]]), A[1]], [A[0], ("not", A[0])], []]
table("ABSORB", rows_for(drl.absorb, alists), "`(items,)` -> `absorb(items)`")
table("SIMPLIFY_NEG", rows_for(drl.simplify_neg, A + [("not", A[0]), ("not", ("not", A[1])), ("and", [A[0], A[2]]), None]), "`(x,)` -> `simplify_neg(x)`")
flist = []
for _ in range(14):
    p = rng.choice(A)
    q = rng.choice(A)
    flist.append(("or", [("and", [p, rng.choice(A)]), ("and", [p, rng.choice(A)])]))
    flist.append(("or", [("and", [p, q, rng.choice(A)]), ("and", [p, q, rng.choice(A)]), ("and", [p, rng.choice(A)])]))
flist += [("or", [A[0], A[1]]), ("or", [("and", [A[0], A[1]]), A[2]])]
table("FACTOR", rows_for(drl.factor, flist), "`(e,)` -> `factor(e)`")

brows = []
vals_ = [("const", 1), ("obj", "q"), ("var", "x", "hot_int"), ("tmp", 2), ("float", 0.25), None, ("cmp", "<", ("const", 1), ("const", 2)),
         ("arith", "-", ("obj", "p"), ("const", 4)), ("bool", ("obj", "w")), ("boolv", A[0]), ("not", A[2]), ("truth", ("obj", "k"))]
for op in ("+", "-", "*", "/", "r-", "r/", "x"):
    for x, y in ((("const", 1), ("obj", "z")), (("float", 2.0), ("float", 0.5))):
        brows.append((repr((op, x, y)), repr(drl.binop(op, x, y))))
table("BINOP", brows, "`(op, x, y)` -> `binop(op, x, y)`")
frows = [(repr((op, x, y)), repr(dsl.fbinop(op, x, y))) for op in ("+", "-", "*", "/", "r-", "r/") for x, y in
         ((("const", 1), ("obj", "z")), (("float", 2.0), ("float", 0.5)))]
table("FBINOP", frows, "`(op, x, y)` -> `dsl.fbinop(op, x, y)`")
table("LV_VALUE", rows_for(dsl.lv_value, [("gvar", 0x10002000), ("loc", -4), ("obj", "x"), ("glob", 5), (), None, ("gvar",)]), "`(lv,)` -> `lv_value(lv)`")

erows = []
syms2 = ["?Counter@@3HA", "?Table@@3PAHA", "?Rate@@3MA", "?p_T@@3PAVtimer_int@@A", "?Act@@YAXXZ", "?Fn@@YAHH@Z", "?Fl@@3VVector3@@A", 5, None]
for v in ([("iat", s) for s in syms2] + vals_ + [("code", 0x401000), ("addr", ("loc", -4)), ("elem", "t", 1), ("str", "s"), ("call", "f", []),
                                                  ("neg", 1), (), 5, ("iat",), ("iat", "x")]):
    for a in (0x10001000, 0x401234, 0):
        erows.append((repr((v, a)), run(drl.expr_arg, v, a)))
table("EXPR_ARG", erows, "`(v, a)` -> `expr_arg(v, a)`")

trows = []
bits32 = lambda f: struct.unpack("<I", struct.pack("<f", f))[0]  # noqa: E731
arg_pool = [("const", bits32(9.0)), ("const", 1), ("const", -1), ("const", 2**32 + 5), ("const", bits32(-0.5)), ("obj", "x"), ("const", 0),
            ("const", 0x7FC00000), ("const", 0x7F800000), ("const", 0xFF800000), ("const", 1), ("const", True), ("float", 1.5), ("boolarg", ("const", 1)), ("const", 1.5), ("const",)]
for _ in range(120):
    n = rng.randint(0, 3)
    args = [rng.choice(arg_pool) for _ in range(n)]
    plen = n if rng.random() < 0.8 else n + 1
    params = [rng.choice(["H", "M", "_N", "PAD", "AAH", "M", "_N"]) for _ in range(plen)]
    trows.append((repr((args, params)), run(drl.typed_args, args, params)))
trows.append((repr(([("const", 1)], None)), run(drl.typed_args, [("const", 1)], None)))
trows.append((repr(([], [])), run(drl.typed_args, [], [])))
table("TYPED_ARGS", trows, "`(args, params)` -> `typed_args(args, params)`")

prows = []
flags = []
for v in vals_:
    flags.append(("test", v))
for x, y in ((("obj", "a"), ("const", 3)), (None, ("const", 1)), (("const", 1), None), (("float", 1.5), ("var", "x", "hot_int"))):
    flags.append(("cmp", x, y))
for mask in (0x01, 0x40, 0x41, 0x05, 0x45, 0x04, 0x00, 0x100, 0x10, 0x44, 0x80):
    flags.append(("ftest", (("var", "x", "hot_int"), ("float", 2.0)), mask))
flags += [("ftest", ("a",), 1), ("ftest", 5, 1), ("ftest", (1, 2), 1.5), ("ftest", ("a", "b", "c"), 1), ("other", 1), (), None, ("test",), ("cmp", 1)]
for f in flags:
    for mn in ("je", "jne", "jl", "jge", "jle", "jg", "jb", "ja", "jmp"):
        prows.append((repr((mn, f, 0x1000A)), run(drl.branch_pred, mn, f, 0x1000A)))
table("BRANCH_PRED", prows, "`(mn, flags, a)` -> `branch_pred(mn, flags, a)`")

PREDS = {
    "head_var": lambda e: e[0] == "var",
    "obj_x": lambda e: e[0] == "obj" and e[1] == "x",
    "len3": lambda e: len(e) == 3,
    "second_is_1": lambda e: e[1] == 1,
    "never": lambda e: False,
    "always": lambda e: True,
}
ctree = [("var", "a", "k"), ("obj", "x"), ("obj", "y"), ("cmp", "<", ("obj", "x"), ("const", 1)), ("and", [("obj", "q"), ("not", ("obj", "x"))]),
         (("obj", "x"), 2), ((("var", "z", "k"), 1), 2), [("obj", "x")], [], (), (5,), ("const", 1), ("a",), ("a", 1), ((), ), ((1, 2),), ("a", [("b", 1)]),
         ("set", ("loc", -4), ("obj", "x")), ("addr", ("loc", 1)), 5, None, "s", ("x", ("y", ("z", ("obj", "x"))))]
crows = []
for e in ctree:
    for name, p in PREDS.items():
        crows.append((repr((e, name)), run(lambda e, n=name: dsl.contains(e, PREDS[n]), e)))
table("CONTAINS", crows, "`(e, predicate name)` -> `contains(e, predicate)`; predicates: head_var, obj_x, len3, second_is_1, never, always")


def rtree(depth):
    r = rng.random()
    if depth <= 0 or r < 0.4:
        return rng.choice([
            ("const", rng.choice([0, 1, -1, 7, 2**31 - 1, True, False, 2.5])), ("float", rng.choice([0.0, -0.0, 1.5, 1e16, 1e-5, 9.0, float("inf"), float("-inf"), float("nan"), 123456789012345.0, 0.1])),
            ("str", rng.choice(["", "ab", "it's", "back\\slash", "new\nline", "tab\t"])), ("var", "p_X", "hot_int"), ("obj", "Obj1"), ("objf", "Fl2"),
            ("elem", "Arr", rng.choice([0, 3, True, 1.5])), ("array", "Arr"), ("call", "Probe", []), ("call", "Probe2", [("const", 1), ("float", 2.0), ("obj", "x")]),
            ("boolarg", ("const", rng.choice([0, 1]))), ("boolarg", ("obj", "q")), ("boolarg", ("call", "G", [])),
        ])
    k = rng.choice(["cmp", "not", "and", "or", "truth", "arith", "boolv", "bool", "call", "boolarg"])
    if k == "cmp":
        return ("cmp", rng.choice(["==", "~=", "<", ">="]), rtree(depth - 1), rtree(depth - 1))
    if k == "not":
        return ("not", rtree(depth - 1))
    if k in ("and", "or"):
        return (k, [rtree(depth - 1) for _ in range(rng.choice([1, 2, 3]))])
    if k == "truth":
        return ("truth", rtree(depth - 1))
    if k == "arith":
        return ("arith", rng.choice(["+", "-", "*", "/"]), rtree(depth - 1), rtree(depth - 1))
    if k == "boolv":
        return ("boolv", rtree(depth - 1))
    if k == "bool":
        return ("bool", rtree(depth - 1))
    if k == "boolarg":
        return ("boolarg", rtree(depth - 1))
    return ("call", "Fn", [rtree(depth - 1) for _ in range(rng.choice([0, 1, 2]))])


rtrees = [rtree(3) for _ in range(70)] + [rtree(1) for _ in range(30)]
rtrees += [("neg", ("const", 1)), ("var", "x"), (), None, 5, ("call", 5, []), ("str", 5), ("call", "f", 5), ("elem", "a", "i"), ("elem", "a", None),
           ("and", 5), ("cmp", "<", ("const", 1)), ("const",), ("not",), ("truth",), (5, 1)]
rrows = []
for e in rtrees:
    for ctx in rng.sample(["bool", "num", "arg"], 2):
        rrows.append((repr((e, ctx)), run(drl.render, e, ctx)))
table("RENDER", rrows, "`(e, ctx)` -> `render(e, ctx)`")
prows = [(repr((e,)), run(drl.paren, e)) for e in [rng.choice(rtrees[:70]) for _ in range(40)] + [("and", [("const", 1)]), ("or", [("obj", "x")]), (), None]]
table("PAREN", prows, "`(e,)` -> `paren(e)`")

# ---------------------------------------------------------------- prelude digests


def fnv(s):
    h = 0xCBF29CE484222325
    for b in s.encode("utf-8"):
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


OUT.append("/// `(name, utf-8 length, line count, FNV-1a 64 of the utf-8 bytes)` of the Python text constants")
OUT.append("pub const PRELUDE_DIGESTS: &[(&str, usize, usize, u64)] = &[")
for nm, s in (("PRELUDE", dsl.PRELUDE), ("ENGINE", dsl.ENGINE), ("A1_PRELUDE", a1.A1_PRELUDE), ("a1_prelude", a1.a1_prelude())):
    OUT.append(f"    ({rs_lit(nm)}, {len(s.encode('utf-8'))}, {s.count(chr(10))}, 0x{fnv(s):016x}),")
OUT.append("];")
OUT.append("")

# ---------------------------------------------------------------- exception classes and texts
# The Armada I converter writes `{type(e).__name__}: {e}` into the Lua it emits when rendering a
# function raises, so the classes and texts of the exceptions the shared helpers can raise are
# compared with the real Python.


def run_raw(f, *args):
    try:
        return repr(f(*args))
    except drl.DecodeError as e:
        return "!DecodeError:" + str(e)
    except Exception as e:  # noqa: BLE001
        return f"!raw:{type(e).__name__}:{e}"


def _unpack(f1, mask):
    return drl.branch_pred("je", ("ftest", f1, mask), 0x1000A)


RAW_CASES = []
for v in ((1,), [], None, 5, 2.5, True, (1, 2)):
    for i in (0, 3):
        RAW_CASES.append(("item", (v, i), lambda v, i: v[i]))
for v in (None, 5, 2.5, True, (1, 2), [1], ()):
    RAW_CASES.append(("items", (v,), lambda v: list(v)))
    RAW_CASES.append(("len", (v,), lambda v: len(v)))
for a_, b_ in ((None, 1), (1, "a"), ("a", 1), ((1,), [1]), (None, None), ((1, "a"), (1, 2)), ([1], 5)):
    RAW_CASES.append(("lt", (a_, b_), lambda a, b: a < b))
for a_, b_ in ((1, 0), (-5, 0), (0, 0)):
    RAW_CASES.append(("floordiv", (a_, b_), lambda a, b: a // b))
    RAW_CASES.append(("mod", (a_, b_), lambda a, b: a % b))
for s_, base in (("zz", 10), ("x" * 300, 16), ("", 10), ("0x", 16), ("1__0", 10), ("a'b", 10), ("é" * 150, 10), ("-", 16), ("\\" * 120, 10)):
    RAW_CASES.append(("int", (s_, base), lambda s, base: int(s, base)))
for pairs, key in (([], "k"), ([("a", 1)], "b"), ([("a", 1)], ("a", 1)), ([], 5), ([], 2.5), ([], None), ([], "it's")):
    RAW_CASES.append(("del", (pairs, key), lambda p, k: dict(p).__delitem__(k)))
for v in (5, None, (1,), [1], 2.5):
    RAW_CASES.append(("lua_str", (v,), drl.lua_str))
for x in (None, "a", (), [], 2.5, True, 7):
    RAW_CASES.append(("render_elem", (("elem", "n", x), "arg"), drl.render))
for x in (2.5, None, "a", (1,), True, 5):
    RAW_CASES.append(("typed_and", (("const", x),), lambda c: drl.typed_args([c], ["M"])))
for f1 in (5, None, (1,), (1, 2, 3), [1], 2.5, (1, 2), ()):
    for mask in (1, 1.5, None, "a", 0x100):
        RAW_CASES.append(("unpack", (f1, mask), _unpack))
for op in ("??", 5, None, ("a",), "=="):
    RAW_CASES.append(("neg", (("cmp", op, 1, 2),), drl.negate))
for mn in ("jb", "ja", "jmp", "je"):
    RAW_CASES.append(("jcc", (mn,), lambda mn: drl.branch_pred(mn, ("cmp", ("const", 1), ("const", 2)), 0x1000A)))
RAW_CASES.append(("evaluate", (("truth", ("obj", "x")),), lambda e: drl.evaluate(e, {})))
RAW_CASES.append(("evaluate", (("cmp", ">=", ("obj", "it's"), 1),), lambda e: drl.evaluate(e, {})))
for sym in ("?a@b@@QAEPA", "?a@b@@QAEPAV", "?a@b@@QAEPAVx@@H@Z", "?a@b@@QAEPAVx"):
    RAW_CASES.append(("msvc_ret", (sym,), dsl.msvc_ret))
for v in (5, None, ("x",), 2.5):
    RAW_CASES.append(("expr_arg", (("iat", v),), lambda e: drl.expr_arg(e, 0x1000)))
for e_ in (None, 5, (), ("a",), [1]):
    RAW_CASES.append(("tag", (e_,), lambda e: e[0]))
rows = [(repr((nm, args)), run_raw(f, *args)) for nm, args, f in RAW_CASES]
table("RAW_TEXT", rows, "`(case, args)` -> result, `!DecodeError:<text>` or `!raw:<class>:<text>` of a shared helper")

# The first line names the CPython that produced the exception texts below (they differ between
# versions); the staleness test compares only under the pinned major.minor version.
header = f"//! generated with CPython {sys.version_info[0]}.{sys.version_info[1]}\n//!\n" + """//! Expected values for `tests/core.rs`, produced by running the ORIGINAL Python functions of
//! `tools/drl_to_lua.py`, `tools/dsl_to_lua.py` and `tools/a1_to_lua.py` on inputs invented for
//! this purpose (made-up symbols and expression trees, numbers from a fixed seed). Nothing here
//! comes from a game file. An input is the Python `repr` of the argument tuple, the expected value
//! the Python `repr` of the result; `!DecodeError:<message>` stands for a `DecodeError` and `!raw`
//! for any other exception. Generated by `gen_tables.py` next to this file (see its header for how
//! to run it); do not edit by hand.

"""
os.makedirs(os.path.dirname(os.path.abspath(OUT_PATH)), exist_ok=True)
open(OUT_PATH, "w", newline="\n").write(header + "\n".join(OUT))
for k, v in COUNTS.items():
    print(k, v)
print("total", sum(COUNTS.values()))
