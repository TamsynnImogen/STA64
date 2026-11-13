"""Tests for tools/dsl_to_lua.py on synthetic code: no game file is read.

The lifter is fed hand-written instruction lists in the debug compiler's
shapes (all addresses, slots, names and strings here are made up), and the
Lua it renders is checked for the statements it must contain; when `luac`
is on PATH the rendered functions are also parsed by it.

  python3 tools/test_dsl_to_lua.py
"""

import importlib.util
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
SPEC = importlib.util.spec_from_file_location("dsl_to_lua", ROOT / "tools" / "dsl_to_lua.py")
dsl = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(dsl)

BASE = 0x10000000
CHK = 0x10009000
SI = 0x10050000  # g_pSI
WORLD = 0x10050004  # p_WORLD
P_ALPHA = 0x10050008  # p_ALPHA (hot_int *)
P_WAIT = 0x1005000C  # p_tWait (timer_int *)
SHIP = 0x10050010  # ship handle
STRINGS = {0x10040000: "ship_label", 0x10040010: "DoThing", 0x10040020: "a_path"}

EXPORTS = {
    "?g_pSI@@3PAVScriptInterface@@A": SI,
    "?p_WORLD@@3PAVcharacter@@A": WORLD,
    "?p_ALPHA@@3PAVhot_int@@A": P_ALPHA,
    "?p_tWait@@3PAVtimer_int@@A": P_WAIT,
    "?ship@@3HA": SHIP,
}

# Made-up ScriptInterface: slot -> mangled name (the decoration carries the types).
SI_NAMES = [
    "?ScriptCreate_hot_int@ScriptInterfaceImp@@UAEPAVhot_int@@PAD@Z",  # 0
    "?GetHandle@ScriptInterfaceImp@@UAEHPAD@Z",  # 1
    "?MoveTo@ScriptInterfaceImp@@UAEXHPAD@Z",  # 2
    "?IsAlive@ScriptInterfaceImp@@UAE_NH@Z",  # 3
    "?SetSpeed@ScriptInterfaceImp@@UAEXHM@Z",  # 4
    "?Enable@ScriptInterfaceImp@@UAEXH_N@Z",  # 5
    "?SetAngularVelocity@ScriptInterfaceImp@@UAEXHAAVVector3@@@Z",  # 6: Vector3 &
    "?GetDistance@ScriptInterfaceImp@@UAEMAAH0@Z",  # 7: int &, int &
    "?RecruitSpecialForces@ScriptInterfaceImp@@UAE_NPADHAAH@Z",  # 8: ..., int &out
    "?BuildAt@ScriptInterfaceImp@@UAEHPADH0H@Z",  # 9: char *, int, char *, int
    "?SetText@ScriptInterfaceImp@@UAEXPAD@Z",  # 10: char *
]
ENGINE = {"ScriptInterfaceImp": SI_NAMES}


class FakePe:
    base = BASE
    data = bytearray(0x60000)  # the image, indexed by address - BASE (see put())

    def off(self, rva):
        return rva if 0 <= rva < len(self.data) else None

    def section_of(self, addr):
        if 0x10001000 <= addr < 0x10030000:
            return ".text"
        if addr in STRINGS or 0x10040000 <= addr < 0x10050000:
            return ".rdata"
        if 0x10050000 <= addr < 0x10051000:
            return ".data"
        return None

    def cstr(self, addr):
        if addr in STRINGS:
            return STRINGS[addr]
        raise dsl.DecodeError("no string")

    def f32(self, addr):
        return 1.5

    def f64(self, addr):
        import struct as _s
        return _s.unpack_from("<d", self.data, addr - BASE)[0]


def put(addr, *words, size=4):
    """Write little-endian words (or bytes, size=1) into the fake image."""
    for k, w in enumerate(words):
        FakePe.data[addr - BASE + k * size:addr - BASE + (k + 1) * size] = (w & (2 ** (8 * size) - 1)).to_bytes(size, "little")


def program(funcs):
    """Instruction list from {start: [text, ...]}; each instruction takes 4 bytes."""
    ins = []
    for start, lines in sorted(funcs.items()):
        a = start
        for line in lines:
            ins.append((a, line))
            a += 4
    return ins


PROLOGUE = [
    "push   ebp",
    "mov    ebp,esp",
    "sub    esp,0x40",
    "push   ebx",
    "push   esi",
    "push   edi",
    "lea    edi,[ebp-0x40]",
    "mov    ecx,0x10",
    "mov    eax,0xcccccccc",
    "rep stos DWORD PTR es:[edi],eax",
]
EPILOGUE = ["pop    edi", "pop    esi", "pop    ebx", "add    esp,0x40", "cmp    ebp,esp",
            f"call   {CHK:#x}", "mov    esp,ebp", "pop    ebp", "ret"]


def si_call(slot, pushes):
    out = ["mov    esi,esp"] + [f"push   {p}" for p in pushes]
    out += [f"mov    eax,ds:{SI:#x}", "mov    edx,DWORD PTR [eax]", f"mov    ecx,DWORD PTR ds:{SI:#x}",
            f"call   DWORD PTR [edx+{slot * 4:#x}]", "cmp    esi,esp", f"call   {CHK:#x}"]
    return out


def var_call(var, slot, pushes=()):
    out = ["mov    esi,esp"] + [f"push   {p}" for p in pushes]
    out += [f"mov    eax,ds:{var:#x}", "mov    edx,DWORD PTR [eax]", f"mov    ecx,DWORD PTR ds:{var:#x}",
            f"call   DWORD PTR [edx+{slot * 4:#x}]", "cmp    esi,esp", f"call   {CHK:#x}"]
    return out


def make(funcs, exports=None):
    ex = dict(EXPORTS)
    ex.update(exports or {})
    return dsl.Dsl("synthetic", ENGINE, pe=FakePe(), ins=program(funcs), exports=ex)


STEP = 0x10002000
STEP_BODY = PROLOGUE + [
    # ship = GetHandle("ship_label")
    *si_call(1, ["0x10040000"]),
    f"mov    ds:{SHIP:#x},eax",
    # MoveTo(ship, "a_path")
    "mov    esi,esp",
    "push   0x10040020",
    f"mov    eax,ds:{SHIP:#x}",
    "push   eax",
    f"mov    ecx,DWORD PTR ds:{SI:#x}",
    "mov    edx,DWORD PTR [ecx]",
    f"mov    ecx,DWORD PTR ds:{SI:#x}",
    "call   DWORD PTR [edx+0x8]",
    "cmp    esi,esp",
    f"call   {CHK:#x}",
    # SetSpeed(ship, 2.0f)
    "mov    esi,esp",
    "push   0x40000000",
    f"mov    eax,ds:{SHIP:#x}",
    "push   eax",
    f"mov    ecx,DWORD PTR ds:{SI:#x}",
    "mov    edx,DWORD PTR [ecx]",
    f"mov    ecx,DWORD PTR ds:{SI:#x}",
    "call   DWORD PTR [edx+0x10]",
    "cmp    esi,esp",
    f"call   {CHK:#x}",
    # p_ALPHA = 1; p_tWait.start(10.0f)
    *var_call(P_ALPHA, 7, ["0x1"]),
    *var_call(P_WAIT, 9, ["0x41200000"]),
    "xor    eax,eax",
] + EPILOGUE


def branch_body():
    """if (IsAlive(ship)) p_ALPHA = 1; else p_ALPHA = 2;"""
    b = PROLOGUE[:]
    b += ["mov    esi,esp", f"mov    eax,ds:{SHIP:#x}", "push   eax", f"mov    ecx,DWORD PTR ds:{SI:#x}",
          "mov    edx,DWORD PTR [ecx]", f"mov    ecx,DWORD PTR ds:{SI:#x}", "call   DWORD PTR [edx+0xc]",
          "cmp    esi,esp", f"call   {CHK:#x}", "and    eax,0xff", "test   eax,eax"]
    start = STEP
    n_before = len(b)
    then = var_call(P_ALPHA, 7, ["0x1"])
    els = var_call(P_ALPHA, 7, ["0x2"])
    a_je = start + 4 * n_before
    a_then = a_je + 4
    a_jmp = a_then + 4 * len(then)
    a_else = a_jmp + 4
    a_end = a_else + 4 * len(els)
    b += [f"je     {a_else:#x}"] + then + [f"jmp    {a_end:#x}"] + els + ["xor    eax,eax"] + EPILOGUE
    return b


def loop_body():
    """for (i = 0; i < 3; i++) p_ALPHA = i;"""
    b = PROLOGUE[:]
    b += ["mov    DWORD PTR [ebp-0x4],0x0"]
    a_jmp = STEP + 4 * len(b)
    b += ["jmp    PLACE_COND"]
    a_inc = STEP + 4 * len(b)
    b += ["mov    eax,DWORD PTR [ebp-0x4]", "add    eax,0x1", "mov    DWORD PTR [ebp-0x4],eax"]
    a_cond = STEP + 4 * len(b)
    b += ["cmp    DWORD PTR [ebp-0x4],0x3", "jge    PLACE_END"]
    body = ["mov    esi,esp", "mov    ecx,DWORD PTR [ebp-0x4]", "push   ecx", f"mov    edx,ds:{P_ALPHA:#x}",
            "mov    eax,DWORD PTR [edx]", f"mov    ecx,DWORD PTR ds:{P_ALPHA:#x}", "call   DWORD PTR [eax+0x1c]",
            "cmp    esi,esp", f"call   {CHK:#x}", f"jmp    {a_inc:#x}"]
    b += body
    a_end = STEP + 4 * len(b)
    b += ["xor    eax,eax"] + EPILOGUE
    b = [x.replace("PLACE_COND", f"{a_cond:#x}").replace("PLACE_END", f"{a_end:#x}") for x in b]
    return b


def luac_ok(src):
    luac = shutil.which("luac")
    if not luac:
        return True
    with tempfile.NamedTemporaryFile("w", suffix=".lua", delete=False) as fh:
        fh.write("local M, A, VarOp, T, N, I, I32, O, S = {}, nil, nil, nil, nil, nil, nil, {}, {}\n")
        fh.write(src)
        name = fh.name
    try:
        r = subprocess.run([luac, "-p", name], capture_output=True, text=True)
        return r.returncode == 0 or print(r.stderr)
    finally:
        pathlib.Path(name).unlink()


def luac_ok_file(path):
    luac = shutil.which("luac")
    if not luac:
        return True
    r = subprocess.run([luac, "-p", str(path)], capture_output=True, text=True)
    return r.returncode == 0 or print(r.stderr)


def render(d, f, name="step"):
    nm = dsl.Namer(d, {f.addr: f}, {f.addr: "step"})
    return "\n".join(dsl.render_function(f, nm, name))


class LiftTests(unittest.TestCase):
    def test_straight_line_step(self):
        d = make({STEP: STEP_BODY})
        f = dsl.Lifter(d).lift(STEP)
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertIn("O.ship = A('GetHandle', 'ship_label')", src)
        self.assertIn("A('MoveTo', O.ship, 'a_path')", src)
        self.assertIn("A('SetSpeed', O.ship, 2.0)", src)
        self.assertIn("VarOp('p_ALPHA', 'set', 1)", src)
        self.assertIn("VarOp('p_tWait', 'start', 10.0)", src)
        self.assertIn("do return 0 end", src)
        self.assertEqual(f.engine_calls, {"GetHandle": 1, "MoveTo": 1, "SetSpeed": 1})
        self.assertTrue(luac_ok(src))

    def test_a_bool_parameter_is_passed_as_a_lua_boolean(self):
        # Enable(ship, 1): the pushed word is an int, the binding takes a boolean
        body = PROLOGUE + ["mov    esi,esp", "push   0x1", f"mov    eax,ds:{SHIP:#x}", "push   eax",
                           f"mov    ecx,DWORD PTR ds:{SI:#x}", "mov    edx,DWORD PTR [ecx]",
                           f"mov    ecx,DWORD PTR ds:{SI:#x}", "call   DWORD PTR [edx+0x14]",
                           "cmp    esi,esp", f"call   {CHK:#x}", "xor    eax,eax"] + EPILOGUE
        d = make({STEP: body})
        f = dsl.Lifter(d).lift(STEP)
        self.assertIsNone(f.error)
        self.assertIn("A('Enable', O.ship, true)", render(d, f))

    def test_if_else_becomes_gotos(self):
        d = make({STEP: branch_body()})
        f = dsl.Lifter(d).lift(STEP)
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertRegex(src, r"if not \(T\(A\('IsAlive', O\.ship\)\)\) then goto b\w+ end")
        self.assertIn("VarOp('p_ALPHA', 'set', 1)", src)
        self.assertIn("VarOp('p_ALPHA', 'set', 2)", src)
        self.assertEqual(f.loops, 0)
        self.assertTrue(luac_ok(src))

    def test_loop_with_a_stack_counter(self):
        d = make({STEP: loop_body()})
        f = dsl.Lifter(d).lift(STEP)
        self.assertIsNone(f.error)
        self.assertEqual(f.loops, 1)
        src = render(d, f)
        self.assertIn("L[-4] = 0", src)
        self.assertIn("VarOp('p_ALPHA', 'set', N(L[-4]))", src)
        self.assertIn("L[-4] = I32(N(L[-4]) + 1)", src)
        self.assertRegex(src, r"if N\(L\[-4\]\) >= 3 then goto|if L\[-4\] >= 3 then goto")
        self.assertTrue(luac_ok(src))

    def test_unknown_instruction_is_reported_not_guessed(self):
        body = PROLOGUE + ["cpuid"] + EPILOGUE
        d = make({STEP: body})
        f = dsl.Lifter(d).lift(STEP)
        self.assertIn("cpuid", f.error)
        src = render(d, f)
        self.assertIn("M.notDecoded('step'", src)
        self.assertTrue(luac_ok(src))

    def test_action_queues_a_script_with_its_step(self):
        action = 0x10003000
        body = PROLOGUE + [
            "mov    esi,esp", "push   0x10040010", f"mov    eax,ds:{WORLD:#x}", "mov    edx,DWORD PTR [eax]",
            f"mov    ecx,DWORD PTR ds:{WORLD:#x}", "call   DWORD PTR [edx+0x4]", "cmp    esi,esp", f"call   {CHK:#x}",
            "mov    DWORD PTR [ebp-0x4],eax", "mov    esi,esp", f"push   {STEP:#x}", "mov    eax,DWORD PTR [ebp-0x4]",
            "mov    edx,DWORD PTR [eax]", "mov    ecx,DWORD PTR [ebp-0x4]", "call   DWORD PTR [edx]",
            "cmp    esi,esp", f"call   {CHK:#x}",
        ] + EPILOGUE
        d = make({action: body, STEP: STEP_BODY}, {"?DoThing@@YAXXZ": action})
        funcs, roles = dsl.lift_mission(d)
        self.assertEqual(roles[action], "action")
        self.assertEqual(roles[STEP], "step")
        self.assertTrue(all(f.error is None for f in funcs.values()))
        nm = dsl.Namer(d, funcs, roles)
        self.assertEqual(nm.fn(STEP), "DoThing_step")
        src = "\n".join(dsl.render_function(funcs[action], nm, "DoThing"))
        self.assertIn("t[1] = NewScript('DoThing')", src)
        self.assertIn("AddStep(t[1], M.DoThing_step, 'DoThing_step')", src)

    def test_swap_keeps_the_values_read_before_the_writes(self):
        body = PROLOGUE + ["mov    eax,DWORD PTR [ebp-0x4]", "mov    ecx,DWORD PTR [ebp-0x8]",
                           "mov    DWORD PTR [ebp-0x4],ecx", "mov    DWORD PTR [ebp-0x8],eax",
                           "xor    eax,eax"] + EPILOGUE
        d = make({STEP: body})
        f = dsl.Lifter(d).lift(STEP)
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertIn("t[1] = L[-4]", src)
        self.assertIn("L[-4] = L[-8]", src)
        self.assertIn("L[-8] = t[1]", src)

    def test_negated_literal_is_not_a_comment(self):
        nm = dsl.Namer(make({STEP: STEP_BODY}), {}, {})
        self.assertEqual(dsl.render_expr(("neg", ("const", -1)), nm), "(-(-1))")
        self.assertEqual(dsl.drl.lua_num(float("inf")), "math.huge")

    def test_return_types(self):
        self.assertEqual(dsl.msvc_ret(SI_NAMES[3]), "_N")
        self.assertEqual(dsl.msvc_ret(SI_NAMES[2]), "X")
        self.assertEqual(dsl.msvc_ret(SI_NAMES[0]), "PAVhot_int@@")
        self.assertEqual(dsl.push_count("VVector3@@"), 3)

    def test_output_refused_inside_the_repository(self):
        self.assertIsNotNone(dsl.drl.refuse_output(str(ROOT / "missions"), []))
        with tempfile.TemporaryDirectory() as t:
            self.assertIsNotNone(dsl.drl.refuse_output(str(pathlib.Path(t) / "out"), [t]))


# ---------------------------------------------------------------- by-pointer arguments (LUA-11)

STRINGS.update({0x10040030: "path_%i", 0x10040040: "odf_a", 0x10040050: "race_x", 0x10040060: "race_y"})
SPRINTF_FN = 0x10005000
VEC3_FN = 0x10006000
STRCMP_FN = 0x10007000
REPORT_FN = 0x10008000
# the C runtime's formatter, in the shape the debug build links it: a stack FILE with the string
# flag, an unbounded count, the buffer pointers, the formatting call, the terminating NUL
SPRINTF_BODY = [
    "push   ebp", "mov    ebp,esp", "sub    esp,0x30", "push   ebx", "push   esi", "push   edi",
    "lea    eax,[ebp-0x20]", "mov    DWORD PTR [ebp-0x24],eax", "lea    ecx,[ebp+0x10]", "mov    DWORD PTR [ebp-0x2c],ecx",
    "mov    ecx,DWORD PTR [ebp-0x24]", "mov    DWORD PTR [ecx+0xc],0x42", "mov    edx,DWORD PTR [ebp-0x24]",
    "mov    eax,DWORD PTR [ebp+0x8]", "mov    DWORD PTR [edx+0x8],eax", "mov    eax,DWORD PTR [ebp-0x24]",
    "mov    DWORD PTR [eax+0x4],0x7fffffff", "mov    eax,DWORD PTR [ebp-0x28]", "pop    edi", "pop    esi", "pop    ebx",
    "mov    esp,ebp", "pop    ebp", "ret",
]
# Vector3::Vector3(float, float, float): `this` saved, three argument words stored through it
VEC3_BODY = [
    "push   ebp", "mov    ebp,esp", "sub    esp,0x44", "push   ebx", "push   esi", "push   edi", "push   ecx",
    "lea    edi,[ebp-0x44]", "mov    ecx,0x11", "mov    eax,0xcccccccc", "rep stos DWORD PTR es:[edi],eax", "pop    ecx",
    "mov    DWORD PTR [ebp-0x4],ecx", "mov    eax,DWORD PTR [ebp-0x4]", "mov    ecx,DWORD PTR [ebp+0x8]",
    "mov    DWORD PTR [eax],ecx", "mov    edx,DWORD PTR [ebp-0x4]", "mov    eax,DWORD PTR [ebp+0xc]",
    "mov    DWORD PTR [edx+0x4],eax", "mov    ecx,DWORD PTR [ebp-0x4]", "mov    edx,DWORD PTR [ebp+0x10]",
    "mov    DWORD PTR [ecx+0x8],edx", "mov    eax,DWORD PTR [ebp-0x4]", "pop    edi", "pop    esi", "pop    ebx",
    "mov    esp,ebp", "pop    ebp", "ret    0xc",
]
STRCMP_BODY = [
    "mov    edx,DWORD PTR [esp+0x4]", "mov    ecx,DWORD PTR [esp+0x8]", "test   edx,0x3", "jne    0x10007100",
    "mov    eax,DWORD PTR [edx]", "cmp    al,BYTE PTR [ecx]", "jne    0x10007100", "xor    eax,eax", "ret",
]


def call_si(slot, pushes):
    """A ScriptInterface call whose arguments are given in push order (last parameter first)."""
    return ["mov    esi,esp"] + pushes + [f"mov    eax,ds:{SI:#x}", "mov    edx,DWORD PTR [eax]",
                                         f"mov    ecx,DWORD PTR ds:{SI:#x}", f"call   DWORD PTR [edx+{slot * 4:#x}]",
                                         "cmp    esi,esp", f"call   {CHK:#x}"]


def lift_body(body, extra=None, exports=None):
    funcs = {STEP: PROLOGUE + body + ["xor    eax,eax"] + EPILOGUE}
    funcs.update(extra or {})
    d = make(funcs, exports)
    f = dsl.Lifter(d).lift(STEP)
    return d, f


class ByPointerTests(unittest.TestCase):
    def test_a_vector_built_on_the_stack_is_passed_as_a_vector(self):
        # SetAngularVelocity(ship, {1.0, 0.0, 0.3}): three words stored in a stack slot, `lea`, push
        d, f = lift_body([
            "mov    DWORD PTR [ebp-0xc],0x3f800000", "mov    DWORD PTR [ebp-0x8],0x0", "mov    DWORD PTR [ebp-0x4],0x3e99999a",
            *call_si(6, ["lea    eax,[ebp-0xc]", "push   eax", f"mov    ecx,DWORD PTR ds:{SHIP:#x}", "push   ecx"]),
        ])
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertIn("A('SetAngularVelocity', O.ship, Vector(1.0, 0.0, 0.30000001192092896))", src)
        self.assertNotIn("Ref(", src)
        self.assertTrue(luac_ok(src))

    def test_a_slot_filled_in_an_earlier_block_reads_its_float_bits(self):
        d, f = lift_body([
            "mov    DWORD PTR [ebp-0xc],0x3f800000", "mov    DWORD PTR [ebp-0x8],0x0", "mov    DWORD PTR [ebp-0x4],0x0",
            *call_si(6, ["lea    eax,[ebp-0xc]", "push   eax", f"mov    ecx,DWORD PTR ds:{SHIP:#x}", "push   ecx"]),
            # a second call, after the first one's call dropped what was known about the slot
            *call_si(6, ["lea    eax,[ebp-0xc]", "push   eax", f"mov    ecx,DWORD PTR ds:{SHIP:#x}", "push   ecx"]),
        ])
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertEqual(src.count("A('SetAngularVelocity', O.ship, Vector("), 2)
        self.assertIn("FB(L[-12])", src)

    def test_the_vector3_constructor_fills_the_slot(self):
        d, f = lift_body([
            "push   0x40400000", "push   0x40000000", "push   0x3f800000", "lea    ecx,[ebp-0x20]", f"call   {VEC3_FN:#x}",
            *call_si(6, ["lea    eax,[ebp-0x20]", "push   eax", f"mov    ecx,DWORD PTR ds:{SHIP:#x}", "push   ecx"]),
        ], {VEC3_FN: VEC3_BODY})
        self.assertIsNone(f.error)
        self.assertEqual(d.helper_kind(VEC3_FN), "vec3")
        self.assertIn("A('SetAngularVelocity', O.ship, Vector(1.0, 2.0, 3.0))", render(d, f))

    def test_int_references_are_the_values(self):
        # GetDistance(int &, int &) takes two handles by reference
        d, f = lift_body([
            "mov    eax,DWORD PTR [ebp-0x10]", "mov    DWORD PTR [ebp-0x4],eax",
            *call_si(7, [f"push   {SHIP:#x}", "lea    eax,[ebp-0x4]", "push   eax"]),
            "fstp   DWORD PTR [ebp-0x8]",
        ])
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertIn("A('GetDistance', L[-16], O.ship)", src.replace("L[-4]", "L[-16]"))
        self.assertNotIn("Ref(", src)

    def test_an_out_parameter_writes_the_slot_after_the_call(self):
        d, f = lift_body([
            *call_si(8, ["lea    eax,[ebp-0x14]", "push   eax", "push   0x3", "push   0x10040040"]),
        ])
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertIn("t[1] = AO('RecruitSpecialForces', 'odf_a', 3, L[-20])", src)
        self.assertIn("L[-20] = OUTV(L[-20])", src)

    def test_sprintf_into_a_buffer_then_the_buffer_as_a_string(self):
        d, f = lift_body([
            "push   0x7", "push   0x10040030", "lea    eax,[ebp-0x30]", "push   eax", f"call   {SPRINTF_FN:#x}", "add    esp,0xc",
            *call_si(9, ["push   0x0", "lea    ecx,[ebp-0x30]", "push   ecx", "push   0x1", "push   0x10040040"]),
        ], {SPRINTF_FN: SPRINTF_BODY})
        self.assertIsNone(f.error)
        self.assertEqual(d.helper_kind(SPRINTF_FN), "sprintf")
        src = render(d, f)
        self.assertIn("SPRINTF('path_%i', 7)", src)
        self.assertIn("L[-48] = t[1]", src)
        self.assertIn("A('BuildAt', 'odf_a', 1, L[-48], 0)", src)
        self.assertNotIn("Ref(", src)
        # the helper is modelled, not lifted
        self.assertEqual(dsl.lift_mission(make({STEP: PROLOGUE + ["xor    eax,eax"] + EPILOGUE, SPRINTF_FN: SPRINTF_BODY}))[1].get(SPRINTF_FN), None)

    def test_strcmp_is_modelled(self):
        d, f = lift_body([
            "push   0x10040060", "push   0x10040050", f"call   {STRCMP_FN:#x}", "add    esp,0x8",
            "test   eax,eax",
        ], {STRCMP_FN: STRCMP_BODY})
        self.assertIsNone(f.error)
        self.assertEqual(d.helper_kind(STRCMP_FN), "strcmp")
        self.assertIn("STRCMP('race_x', 'race_y')", render(d, f))

    def test_the_debug_report_is_modelled_only_when_every_call_site_is_a_report(self):
        report = ["push   ebp", "mov    ebp,esp", "stos   WORD PTR es:[edi],ax", "pop    ebp", "ret"]
        site = ["push   0x10040030", "push   0x0", "push   0x5d", "push   0x10040040", "push   0x2",
                f"call   {REPORT_FN:#x}", "add    esp,0x14"]
        d, f = lift_body(site * 10, {REPORT_FN: report})
        self.assertIsNone(f.error)
        self.assertEqual(d.helper_kind(REPORT_FN), "report")
        # a routine that merely follows a `stos WORD` and is called with small constants is lifted
        d, f = lift_body(site * 3, {REPORT_FN: report})
        self.assertIsNone(d.helper_kind(REPORT_FN))

    def test_strcmp_of_a_stack_buffer_is_a_string(self):
        d, f = lift_body([
            "push   0x7", "push   0x10040030", "lea    eax,[ebp-0x30]", "push   eax", f"call   {SPRINTF_FN:#x}", "add    esp,0xc",
            "push   0x10040050", "lea    ecx,[ebp-0x30]", "push   ecx", f"call   {STRCMP_FN:#x}", "add    esp,0x8",
        ], {SPRINTF_FN: SPRINTF_BODY, STRCMP_FN: STRCMP_BODY})
        self.assertIsNone(f.error)
        self.assertIn("STRCMP(L[-48], 'race_x')", render(d, f))

    def test_a_double_nothing_writes_is_its_image_value(self):
        import struct as _s
        put(0x10040400, *_s.unpack("<II", _s.pack("<d", 2.5)))
        d, f = lift_body(["fld    QWORD PTR ds:0x10040400", "fstp   DWORD PTR [ebp-0x4]"])
        self.assertIsNone(f.error)
        self.assertIn("L[-4] = 2.5", render(d, f))

    def test_a_static_the_code_never_writes_is_its_image_value(self):
        put(0x10040100, 0x40000000)  # a read-only float constant
        put(0x10050200, 0)  # a zero-initialised static nothing writes
        put(0x10050210, 5)  # a static the code writes
        d, f = lift_body([
            "mov    eax,ds:0x10040100", "mov    DWORD PTR [ebp-0x4],eax",
            "mov    ecx,ds:0x10050200", "mov    DWORD PTR [ebp-0x8],ecx",
            "mov    DWORD PTR ds:0x10050210,0x9", "mov    edx,ds:0x10050210", "mov    DWORD PTR [ebp-0xc],edx",
        ])
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertIn("L[-4] = 1073741824", src)  # the constant's word
        self.assertIn("L[-8] = 0", src)
        self.assertIn("G[0x10050210] = 9", src)
        self.assertIn("L[-12] = 9", src)
        self.assertNotIn("G[0x10050200]", src)
        self.assertEqual(d.static_refs[0x10050210], "word")
        self.assertEqual(d.static_class(0x10040100), "rodata")
        self.assertEqual(d.static_class(0x10050200), "const")
        self.assertEqual(d.static_class(0x10050210), "mutable")

    def test_a_static_string_is_its_image_text_or_a_slot(self):
        STRINGS[0x10040300] = ""  # an empty string literal in read-only data
        d, f = lift_body([*call_si(10, ["push   0x10040300"]), *call_si(10, ["push   0x10050300"])])
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertIn("A('SetText', '')", src)  # the literal
        self.assertIn("A('SetText', G[0x10050300])", src)  # a buffer whose address the code hands out
        self.assertEqual(d.static_refs[0x10050300], "str")

    def test_two_level_jump_table(self):
        # idx in [0..4] -> byte table -> target table (3 entries)
        t1, t2 = 0x10040200, 0x10040210
        a_ret = STEP + 4 * 60
        targets = [a_ret, a_ret + 4, a_ret + 8]
        put(t1, 0, 1, 2, 1, 0, size=1)
        put(t2, *targets)
        body = PROLOGUE + [
            "mov    DWORD PTR [ebp-0x4],0x2",
            "cmp    DWORD PTR [ebp-0x4],0x4", f"ja     {a_ret + 12:#x}",
            "mov    eax,DWORD PTR [ebp-0x4]", "xor    edx,edx", f"mov    dl,BYTE PTR [eax+{t1:#x}]",
            f"jmp    DWORD PTR [edx*4+{t2:#x}]",
        ]
        body += ["nop"] * (60 - len(body))
        body += ["mov    DWORD PTR [ebp-0x8],0x1", "mov    DWORD PTR [ebp-0x8],0x2", "mov    DWORD PTR [ebp-0x8],0x3", "xor    eax,eax"] + EPILOGUE
        d = make({STEP: body})
        f = dsl.Lifter(d).lift(STEP)
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertEqual(src.count(" then goto b"), 5 + 1)  # five switch cases and the bound's branch
        self.assertIn("local X = I(N(L[-4]))", src)

    def test_unlifted_addresses_are_never_rendered_as_tables(self):
        nm = dsl.Namer(make({STEP: STEP_BODY}), {}, {})
        with self.assertRaises(dsl.DecodeError):
            dsl.render_expr(("addr", ("loc", -4)), nm)


class MapStemTests(unittest.TestCase):
    def test_the_script_is_named_by_the_map_that_runs_it(self):
        with tempfile.TemporaryDirectory() as t:
            bzn = pathlib.Path(t) / "BZN"
            bzn.mkdir()
            (bzn / "map_one.bzn").write_bytes(b"\0junk\0some_Mission1s.dsl\0more\0some_Mission1r.drl\0")
            (bzn / "other.bzn").write_bytes(b"x a_second.dsl y")  # not a `...S.dsl` mission script name
            (bzn / "map_one_copy.bzn").write_bytes(b"some_Mission1S.DSL")
            stems = dsl.map_stems([t])
            self.assertEqual(stems, {"some_mission1": ["map_one", "map_one_copy"]})


class PreludeTests(unittest.TestCase):
    """The prelude's runtime pieces, run by a Lua interpreter when one is on PATH."""

    LUA = shutil.which("lua5.4") or shutil.which("lua")

    def run_lua(self, body):
        if not self.LUA:
            self.skipTest("no lua on PATH")
        src = dsl.PRELUDE.replace("Mission_", "MissionX")
        head = "MissionX = { vars = {}, state = {}, objects = {}, globals = {}, timers = {}, scripts = {}, rules = {}, " \
               "affected = {}, satisfied = {}, fired = {}, fireCount = {}, ran = {}, calls = {}, missing = {}, " \
               "failed = {}, notDecoded = {}, stepErrors = {}, conditionErrors = {}, engineVars = {}, " \
               "engineWritten = {}, engineReads = {}, stepCount = 0, callFailed = false, problems = {}, " \
               "problemOrder = {}, blocked = {}, invalid = {}, unresolved = {} }\n"
        with tempfile.NamedTemporaryFile("w", suffix=".lua", delete=False) as fh:
            fh.write(head + src + "\n" + body)
            name = fh.name
        try:
            r = subprocess.run([self.LUA, name], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            return r.stdout.strip()
        finally:
            pathlib.Path(name).unlink()

    def test_sprintf_strcmp_and_float_bits(self):
        out = self.run_lua("print(SPRINTF('%ld%%|%5d|%x', 9, 3, -1), SPRINTF('p%i_%d_%s', 3, 4.0, 'x'), STRCMP('a', 'a'), STRCMP('a', 'b'), FB(1065353216), FB(0.5))")
        self.assertEqual(out, "9%|    3|ffffffff\tp3_4_x\t0\t-1\t1.0\t0.5")

    def test_engine_written_variables_are_counted_and_bindable(self):
        out = self.run_lua(
            "local v = EngineVar('strength_team_1', 'hot_float') BindVar('p_strength_team_1', 'hot_float', v)\n"
            "print(S.p_strength_team_1, S.p_strength_team_1)\n"
            "A2 = { EngineVariable = function(n) return n == 'strength_team_1' and 7.5 or nil end }\n"
            "print(S.p_strength_team_1, MissionX.engineReads.strength_team_1)")
        self.assertEqual(out.splitlines(), ["-1.0\t-1.0", "7.5\t3"])

    def test_out_parameter_keeps_the_slot_when_the_call_did_not_run(self):
        out = self.run_lua("A2 = { Recruit = function() return true, 42 end }\n"
                           "local ok = AO('Recruit') print(ok, OUTV(7))\n"
                           "local ok2 = AO('Missing') print(ok2, OUTV(7))")
        self.assertEqual(out.splitlines(), ["true\t42", "nil\t7"])


# A made-up copy of the debug compiler's `std::vector<int>` members (MSVC 6 layout), in the shapes
# the lifter recognises: every address is invented.
V_CTOR, V_BEGIN, V_END, V_SIZE, V_INDEX, V_PUSH, V_INSERT = 0x10003000, 0x10003040, 0x10003080, 0x100030c0, 0x10003100, 0x10003140, 0x10003180
V_DTOR, V_ATEXIT, V_ATEXIT_INNER, V_DESTROY = 0x100031c0, 0x10003200, 0x10003240, 0x10003280
VPRO = ["push   ebp", "mov    ebp,esp", "sub    esp,0x44", "push   ebx", "push   esi", "push   edi", "push   ecx",
        "lea    edi,[ebp-0x44]", "mov    ecx,0x11", "mov    eax,0xcccccccc", "rep stos DWORD PTR es:[edi],eax",
        "pop    ecx", "mov    DWORD PTR [ebp-0x4],ecx"]
VEPI = ["pop    edi", "pop    esi", "pop    ebx", "mov    esp,ebp", "pop    ebp"]
THIS = "mov    eax,DWORD PTR [ebp-0x4]"
VECTOR_MEMBERS = {
    V_CTOR: VPRO + ["mov    eax,DWORD PTR [ebp+0x8]", "mov    cl,BYTE PTR [eax]", "mov    edx,DWORD PTR [ebp-0x4]",
                    "mov    BYTE PTR [edx],cl", "mov    eax,DWORD PTR [ebp-0x4]", "mov    DWORD PTR [eax+0x4],0x0",
                    "mov    ecx,DWORD PTR [ebp-0x4]", "mov    DWORD PTR [ecx+0x8],0x0", "mov    edx,DWORD PTR [ebp-0x4]",
                    "mov    DWORD PTR [edx+0xc],0x0", "mov    eax,DWORD PTR [ebp-0x4]"] + VEPI + ["ret    0x4"],
    V_BEGIN: VPRO + [THIS, "mov    eax,DWORD PTR [eax+0x4]"] + VEPI + ["ret"],
    V_END: VPRO + [THIS, "mov    eax,DWORD PTR [eax+0x8]"] + VEPI + ["ret"],
    V_SIZE: VPRO + [THIS, "cmp    DWORD PTR [eax+0x4],0x0", f"jne    {V_SIZE + 4 * 28:#x}",
                    "mov    DWORD PTR [ebp-0x8],0x0", f"jmp    {V_SIZE + 4 * 36:#x}", "mov    ecx,DWORD PTR [ebp-0x4]",
                    "mov    edx,DWORD PTR [ebp-0x4]", "mov    eax,DWORD PTR [ecx+0x8]", "sub    eax,DWORD PTR [edx+0x4]",
                    "sar    eax,0x2", "mov    DWORD PTR [ebp-0x8],eax", "mov    eax,DWORD PTR [ebp-0x8]"] + VEPI + ["ret"],
    V_INDEX: VPRO + ["mov    ecx,DWORD PTR [ebp-0x4]", f"call   {V_BEGIN:#x}", "mov    ecx,DWORD PTR [ebp+0x8]",
                     "lea    eax,[eax+ecx*4]"] + VEPI + ["ret    0x4"],
    V_PUSH: VPRO + ["mov    eax,DWORD PTR [ebp+0x8]", "push   eax", "mov    ecx,DWORD PTR [ebp-0x4]", f"call   {V_END:#x}",
                    "push   eax", "mov    ecx,DWORD PTR [ebp-0x4]", f"call   {V_INSERT:#x}"] + VEPI + ["ret    0x4"],
    V_INSERT: ["push   ebp", "mov    ebp,esp", "ret    0x8"],
    V_DTOR: VPRO + [THIS, "mov    ecx,DWORD PTR [eax+0x8]", "push   ecx", "mov    edx,DWORD PTR [ebp-0x4]",
                    "mov    eax,DWORD PTR [edx+0x4]", "push   eax", "mov    ecx,DWORD PTR [ebp-0x4]", f"call   {V_DESTROY:#x}",
                    "mov    ecx,DWORD PTR [ebp-0x4]", "mov    edx,DWORD PTR [ebp-0x4]", "mov    eax,DWORD PTR [ecx+0xc]",
                    "sub    eax,DWORD PTR [edx+0x4]", "sar    eax,0x2", "push   eax", "mov    ecx,DWORD PTR [ebp-0x4]",
                    "mov    edx,DWORD PTR [ecx+0x4]", "push   edx", "mov    ecx,DWORD PTR [ebp-0x4]", f"call   {V_DESTROY:#x}",
                    "mov    eax,DWORD PTR [ebp-0x4]", "mov    DWORD PTR [eax+0x4],0x0", "mov    ecx,DWORD PTR [ebp-0x4]",
                    "mov    DWORD PTR [ecx+0x8],0x0", "mov    edx,DWORD PTR [ebp-0x4]",
                    "mov    DWORD PTR [edx+0xc],0x0"] + VEPI + ["ret"],
    V_DESTROY: ["push   ebp", "mov    ebp,esp", "ret    0x8"],
    V_ATEXIT: ["push   ebp", "mov    ebp,esp", "mov    eax,DWORD PTR [ebp+0x8]", "push   eax", f"call   {V_ATEXIT_INNER:#x}",
               "add    esp,0x4", "neg    eax", "sbb    eax,eax", "neg    eax", "dec    eax", "pop    ebp", "ret"],
    V_ATEXIT_INNER: ["push   ebp", "mov    ebp,esp", "xor    eax,eax", "pop    ebp", "ret"],
}
VEC_STATIC, VEC_GUARD = 0x10050100, 0x10050110
# `static std::vector<int> v;` and a local one, the SEH frame of a function with a destructor.
SEH_STEP = [
    "push   ebp", "mov    ebp,esp", "push   0xffffffff", "push   0x10002f00", "mov    eax,fs:0x0", "push   eax",
    "mov    DWORD PTR fs:0x0,esp", "sub    esp,0x60", "push   ebx", "push   esi", "push   edi", "lea    edi,[ebp-0x6c]",
    "mov    ecx,0x1b", "mov    eax,0xcccccccc", "rep stos DWORD PTR es:[edi],eax",
    # static guard: constructs the static once and registers its destructor
    "xor    eax,eax", f"mov    al,ds:{VEC_GUARD:#x}", "and    eax,0x1", "test   eax,eax", "jne    GUARDED",
    f"mov    cl,BYTE PTR ds:{VEC_GUARD:#x}", "or     cl,0x1", f"mov    BYTE PTR ds:{VEC_GUARD:#x},cl",
    "lea    edx,[ebp-0x40]", "push   edx", f"mov    ecx,{VEC_STATIC:#x}", f"call   {V_CTOR:#x}",
    f"push   {V_DTOR:#x}", f"call   {V_ATEXIT:#x}", "add    esp,0x4",
    # GUARDED: local vector, local.push_back(GetHandle("ship_label")), static.push_back(local[0]), n = static.size()
    "lea    eax,[ebp-0x44]", "push   eax", "lea    ecx,[ebp-0x1c]", f"call   {V_CTOR:#x}",
    "mov    DWORD PTR [ebp-0x4],0x0",
    *si_call(1, ["0x10040000"]),
    "mov    DWORD PTR [ebp-0x30],eax",
    "lea    eax,[ebp-0x30]", "push   eax", "lea    ecx,[ebp-0x1c]", f"call   {V_PUSH:#x}",
    "push   0x0", "lea    ecx,[ebp-0x1c]", f"call   {V_INDEX:#x}", "mov    ecx,DWORD PTR [eax]", "mov    DWORD PTR [ebp-0x34],ecx",
    "lea    edx,[ebp-0x34]", "push   edx", f"mov    ecx,{VEC_STATIC:#x}", f"call   {V_PUSH:#x}",
    f"mov    ecx,{VEC_STATIC:#x}", f"call   {V_SIZE:#x}", "mov    DWORD PTR [ebp-0x38],eax",
    "mov    DWORD PTR [ebp-0x4],0xffffffff", "lea    ecx,[ebp-0x1c]", f"call   {V_DTOR:#x}",
    "mov    eax,DWORD PTR [ebp-0x38]", "mov    ecx,DWORD PTR [ebp-0xc]", "mov    DWORD PTR fs:0x0,ecx",
    "pop    edi", "pop    esi", "pop    ebx", "add    esp,0x6c", "cmp    ebp,esp", f"call   {CHK:#x}",
    "mov    esp,ebp", "pop    ebp", "ret",
]


class VectorAndFrameTests(unittest.TestCase):
    def seh_dsl(self):
        funcs = {STEP: SEH_STEP, **{a: b + ["int3"] for a, b in VECTOR_MEMBERS.items()}}
        funcs[STEP] = [t.replace("GUARDED", f"{STEP + 4 * SEH_STEP.index('lea    eax,[ebp-0x44]'):#x}") for t in SEH_STEP]
        return make(funcs)

    def test_the_vector_members_are_recognised_by_their_bodies(self):
        d = self.seh_dsl()
        kinds = {d.vector_kind(a) for a in VECTOR_MEMBERS}
        self.assertEqual(kinds - {None}, {"ctor", "begin", "end", "size", "index", "push_back", "dtor"})
        self.assertEqual(d.vector_kind(V_INSERT), None)
        self.assertEqual(d.helper_kind(V_ATEXIT), "atexit")

    def test_a_seh_frame_and_vectors_lift_to_tables(self):
        d = self.seh_dsl()
        f = dsl.Lifter(d).lift(STEP)
        self.assertIsNone(f.error)
        src = render(d, f)
        self.assertIn("VCTOR(VG(0x10050100))", src)
        self.assertIn("VCTOR(VL(L, -28))", src)
        self.assertIn("VPUSH_BACK(VL(L, -28), t[2])", src)
        self.assertIn("L[-52] = VGET(VL(L, -28), VINDEX(VL(L, -28), 0))", src)
        self.assertIn("VPUSH_BACK(VG(0x10050100), L[-52])", src)
        self.assertIn("VSIZE(VG(0x10050100))", src)
        self.assertIn("VDTOR(VL(L, -28))", src)
        self.assertNotIn("fs:", src)
        self.assertEqual(f.calls, [])  # the members and `atexit` are modelled, not lifted
        self.assertTrue(luac_ok(src))


class VectorRuntime(PreludeTests):
    """The `std::vector` runtime helpers, run in the prelude of the converted script."""

    def test_iterators_are_byte_offsets_and_erase_returns_the_next(self):
        out = self.run_lua(
            "local v = VG(0x10) VPUSH_BACK(v, 7) VPUSH_BACK(v, 8) VPUSH_BACK(v, 9)\n"
            "local it = VBEGIN(v) local seen = {}\n"
            "while it ~= VEND(v) do local x = VGET(v, it) if x == 8 then it = VERASE(v, it) else seen[#seen + 1] = x it = it + 4 end end\n"
            "print(VSIZE(v), table.concat(seen, ','), VGET(v, VINDEX(v, 1)), VGET(v, VINDEX(v, 5)))\n"
            "VCLEAR(v) print(VSIZE(VG(0x10)), VEND(v))")
        self.assertEqual(out.splitlines(), ["2\t7,9\t9\t0", "0\t0"])


class InstallDecodeCounts(unittest.TestCase):
    """The stock Armada II install, read at run time (skipped without it): the decode counts
    must not regress, and no by-pointer argument or unnamed global is left in the output."""

    INSTALL = __import__("os").environ.get("STA64_TEST_A2_STOCK_INSTALL")

    def test_every_stock_mission_decodes_and_leaves_nothing_unresolved(self):
        if not self.INSTALL or not shutil.which("objdump"):
            self.skipTest("set STA64_TEST_A2_STOCK_INSTALL (and have objdump) to run")
        with tempfile.TemporaryDirectory() as t:
            out, rep = pathlib.Path(t) / "out", pathlib.Path(t) / "report.json"
            r = subprocess.run([sys.executable, "-I", str(ROOT / "tools" / "dsl_to_lua.py"), "--install", self.INSTALL,
                                "--out", str(out), "--json", str(rep)], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            import json
            missions = json.loads(rep.read_text())["missions"]
            self.assertEqual(len(missions), 36)
            total = {}
            for m in missions:
                for role, v in m["functions"].items():
                    d = total.setdefault(role, [0, 0])
                    d[0] += v["decoded"]
                    d[1] += v["total"]
            # every action, step, helper, ScriptExtendInterface and ScriptInit (the C runtime's
            # routines and the `std::vector` members are modelled)
            self.assertEqual(total["action"], [1152, 1152])
            self.assertEqual(total["ScriptInit"], [36, 36])
            self.assertEqual(total["ScriptExtendInterface"], [36, 36])
            self.assertEqual(total["helper"][0], total["helper"][1])
            self.assertEqual(total["step"], [1697, 1697])
            undecoded = [(m["mission"], e["function"]) for m in missions for e in m["errors"]]
            self.assertEqual(undecoded, [])
            files = sorted(p.name for p in (out / "missions").iterdir())
            self.assertEqual(len(files), 36)
            self.assertIn("a2_tutorial_ui.lua", files)  # the UI tutorial's map, which runs a2_Tutorial1
            self.assertNotIn("a2_tutorial1.lua", files)
            for f in (out / "missions").iterdir():
                src = f.read_text()
                self.assertNotIn("Ref(", src, f.name)
                import re
                started = set(re.findall(r"^G\[(0x[0-9a-f]+)\] = ", src, re.M))
                used = set(re.findall(r"G\[(0x[0-9a-f]+)\]", src))
                self.assertEqual(used - started, set(), f.name + ": a global read with no start value")
                self.assertTrue(luac_ok_file(f), f.name)


if __name__ == "__main__":
    unittest.main()
