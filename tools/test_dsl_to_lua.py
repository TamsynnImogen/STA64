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
]
ENGINE = {"ScriptInterfaceImp": SI_NAMES}


class FakePe:
    base = BASE
    data = b""

    def section_of(self, addr):
        if 0x10001000 <= addr < 0x10030000:
            return ".text"
        if addr in STRINGS or 0x10040000 <= addr < 0x10041000:
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

    def test_return_types(self):
        self.assertEqual(dsl.msvc_ret(SI_NAMES[3]), "_N")
        self.assertEqual(dsl.msvc_ret(SI_NAMES[2]), "X")
        self.assertEqual(dsl.msvc_ret(SI_NAMES[0]), "PAVhot_int@@")
        self.assertEqual(dsl.push_count("VVector3@@"), 3)

    def test_output_refused_inside_the_repository(self):
        self.assertIsNotNone(dsl.drl.refuse_output(str(ROOT / "missions"), []))
        with tempfile.TemporaryDirectory() as t:
            self.assertIsNotNone(dsl.drl.refuse_output(str(pathlib.Path(t) / "out"), [t]))


if __name__ == "__main__":
    unittest.main()
