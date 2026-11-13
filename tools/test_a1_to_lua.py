"""Tests for tools/a1_to_lua.py on synthetic code: no game file is read.

The Armada I front end is fed hand-written instruction lists in a release
compiler's shapes (no frame pointer, arguments by import signature, deferred
`add esp` cleanup); every address, symbol, slot and string here is made up.
The Lua it renders is checked for the statements it must contain; when
`luac` is on PATH the rendered functions are also parsed by it.

  python3 tools/test_a1_to_lua.py
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
SPEC = importlib.util.spec_from_file_location("a1_to_lua", ROOT / "tools" / "a1_to_lua.py")
a1 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(a1)
dsl = a1.dsl

BASE = 0x10000000
STRINGS = {0x10040000: "ship_label", 0x10040010: "the_script", 0x10040020: "rule_one", 0x10040030: "a_path"}
ALPHA, WAIT, SHIP, SPOT = 0x10050000, 0x10050040, 0x10050080, 0x10050090

# Exported data of the made-up mission DLL (one action as a thunk).
EXPORTS = {
    "?ALPHA@@3Vhot_int@@A": ALPHA,
    "?tWait@@3Vtimer_int@@A": WAIT,
    "?ship@@3HA": SHIP,
    "?spot@@3VVector3@@A": SPOT,
    "?DoIt@@YAXXZ": 0x10001004,
}

# Import table: IAT slot -> (dll, symbol).
IMPORTS = {}
IAT = {}
for i, (dll, sym) in enumerate([
    ("Armada.exe", "?WORLD@@3Vcharacter@@A"),
    ("Armada.exe", "?new_script@character@@QAEPAVscript@@PAD@Z"),
    ("Armada.exe", "?add_step@script@@QAEXP6AHPAV1@@Z@Z"),
    ("Armada.exe", "??4hot_int@@QAEXH@Z"),
    ("Armada.exe", "??Bhot_int@@QAEHXZ"),
    ("Armada.exe", "?start@timer_int@@UAEXM@Z"),
    ("Armada.exe", "?GetHandle@@YAHPAD@Z"),
    ("Armada.exe", "?Foo@@YAXHH@Z"),
    ("Armada.exe", "?Bar@@YAXH@Z"),
    ("Armada.exe", "?Attack@@YAXHVVector3@@@Z"),
    ("Armada.exe", "?GetAttackLocation@@YA?AVVector3@@HH@Z"),
    ("Armada.exe", "?GetDistance@@YAMAAH0@Z"),
    ("Armada.exe", "?Count@@YAHHPAD@Z"),
    ("Armada.exe", "?Mark@@YAXABHAB_N@Z"),
    ("Armada.exe", "?create_rule@IIIE@@QAEPAVrule@@P6AHXZP6AXXZPAD@Z"),
    ("Armada.exe", "?add_dependent_rule@hot_type@@UAEXPAVrule@@@Z"),
    ("Armada.exe", "?isCineractiveDone@@3Vhot_int@@A"),
    ("Test1S.dsl", "?ALPHA@@3Vhot_int@@A"),
    ("Test1S.dsl", "?ship@@3HA"),
    ("Test1S.dsl", "?DoIt@@YAXXZ"),
]):
    slot = 0x10060000 + 4 * i
    IMPORTS[slot] = (dll, sym)
    IAT[a1.drl.plain_name(sym) if not sym.startswith("??") else sym] = slot


def iat(name):
    return f"DWORD PTR ds:{IAT[name]:#x}"


class FakePe:
    base = BASE
    data = b""
    imports = IMPORTS

    def section_of(self, addr):
        if 0x10001000 <= addr < 0x10030000:
            return ".text"
        if 0x10040000 <= addr < 0x10041000 or 0x10060000 <= addr < 0x10061000:
            return ".rdata"
        if 0x10050000 <= addr < 0x10051000:
            return ".data"
        return None

    def cstr(self, addr):
        if addr in STRINGS:
            return STRINGS[addr]
        raise a1.DecodeError("no string")

    def f32(self, addr):
        return 1.5

    def f64(self, addr):
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


DLLMAIN, ACTION, STEP, INIT, EP = 0x10002000, 0x10004000, 0x10003000, 0x10005000, 0x10001100
THUNKS = {0x10001000: ["jmp    0x10002000", "jmp    0x10004000", "jmp    0x10003000", "jmp    0x10005000"]}
ENTRY = {EP: ["push   ebx", "push   esi", "push   edi", "call   0x10001000", "ret    0xc"]}
DLLMAIN_CODE = {DLLMAIN: [
    "mov    eax,DWORD PTR [esp+0x8]",
    "cmp    eax,0x1",
    f"jne    {DLLMAIN + 16:#x}",
    "call   0x1000100c",
    "mov    eax,0x1",
    "ret    0xc",
]}


def make(funcs, exports=None, ep=EP):
    code = dict(THUNKS)
    code.update(ENTRY)
    code.update({DLLMAIN: ["ret    0xc"], ACTION: ["ret"], STEP: ["ret"], INIT: ["ret"]})
    code.update(funcs)
    ex = dict(EXPORTS)
    ex.update(exports or {})
    return a1.A1Dll("Test1S.dsl", ins=program(code), pe=FakePe(), exports=ex, entry_point=ep)


def lift(funcs, start):
    d = make(funcs)
    lf = a1.A1Lifter(d)
    return d, lf, lf.lift(start)


def render(d, f, name="fn"):
    nm = dsl.Namer(d, {f.addr: f}, {f.addr: "step"})
    return "\n".join(dsl.render_function(f, nm, name))


def luac_ok(src):
    exe = shutil.which("luac5.4") or shutil.which("luac")
    if not exe:
        return True
    with tempfile.NamedTemporaryFile("w", suffix=".lua", delete=False) as fh:
        fh.write(src)
    try:
        return subprocess.run([exe, "-p", fh.name], capture_output=True).returncode == 0
    finally:
        pathlib.Path(fh.name).unlink()


class Signatures(unittest.TestCase):
    def test_free_functions_methods_operators_and_function_pointers(self):
        self.assertEqual(a1.signature("?GetDistance@@YAMAAH0@Z"), ("cdecl", None, "GetDistance", "M", ["AAH", "AAH"]))
        self.assertEqual(a1.signature("??4hot_int@@QAEXH@Z"), ("thiscall", "hot_int", "op4", "X", ["H"]))
        self.assertEqual(a1.signature("??Bhot_int@@QAEHXZ"), ("thiscall", "hot_int", "opB", "H", []))
        conv, cls, name, ret, params = a1.signature("?create_rule@IIIE@@QAEPAVrule@@P6AHXZP6AXXZPAD@Z")
        self.assertEqual((conv, cls, ret, len(params)), ("thiscall", "IIIE", "PAVrule@@", 3))
        self.assertEqual(a1.signature("?GetAttackLocation@@YA?AVVector3@@HH@Z")[3], "?AVVector3@@")
        self.assertIsNone(a1.signature("?ship@@3HA"))
        # back-references count parameter types only, not the return type
        self.assertEqual(a1.signature("?f@@YAPAVa@@PAVb@@0@Z")[4], ["PAVb@@", "PAVb@@"])

    def test_struct_results_take_a_hidden_slot_and_vectors_three(self):
        self.assertEqual(a1.slots_of(["H", "H"], "?AVVector3@@"), 3)
        self.assertEqual(a1.slots_of(["H", "VVector3@@"], "X"), 4)


class Steps(unittest.TestCase):
    def test_variables_handles_and_deferred_cleanup(self):
        d, _, f = lift({STEP: [
            "push   0x1",
            f"mov    ecx,{ALPHA:#x}",
            f"call   {iat('??4hot_int@@QAEXH@Z')}",
            "push   0x40e00000",
            f"mov    ecx,{WAIT:#x}",
            f"call   {iat('start')}",
            f"mov    esi,{iat('GetHandle')}",
            "push   0x10040000",
            "call   esi",
            f"mov    ds:{SHIP:#x},eax",
            "push   0x2",
            "push   eax",
            f"call   {iat('Foo')}",
            "add    esp,0xc",
            "xor    eax,eax",
            "ret",
        ]}, STEP)
        self.assertIsNone(f.error, f.error)
        lua = render(d, f)
        self.assertIn("VarOp('ALPHA', 'set', 1)", lua)
        self.assertIn("VarOp('tWait', 'start', 7.0)", lua)
        self.assertIn("A('GetHandle', 'ship_label')", lua)
        self.assertIn("A('Foo',", lua)
        self.assertIn(", 2)", lua)
        self.assertTrue(luac_ok(lua), lua)

    def test_a_value_that_differs_between_paths_goes_through_a_slot(self):
        # push (x ? 1 : 0) across a branch, then one call takes it
        d, _, f = lift({STEP: [
            f"mov    eax,DWORD PTR ds:{SHIP:#x}",
            "test   eax,eax",
            f"je     {STEP + 20:#x}",
            "push   0x1",
            f"jmp    {STEP + 24:#x}",
            "push   0x0",
            f"call   {iat('Bar')}",
            "add    esp,0x4",
            "ret",
        ]}, STEP)
        self.assertIsNone(f.error, f.error)
        lua = render(d, f)
        self.assertIn("L[-4] = 1", lua)
        self.assertIn("L[-4] = 0", lua)
        self.assertIn("A('Bar', L[-4])", lua)
        self.assertTrue(luac_ok(lua), lua)

    def test_a_value_passes_through_a_block_that_does_not_read_it(self):
        # eax = 1 or 2 on two paths, a join that branches on something else,
        # then a block that pushes eax
        d, _, f = lift({STEP: [
            f"mov    ecx,DWORD PTR ds:{SHIP:#x}",
            "test   ecx,ecx",
            f"je     {STEP + 20:#x}",
            "mov    eax,0x1",
            f"jmp    {STEP + 24:#x}",
            "mov    eax,0x2",
            "cmp    ecx,0x5",
            f"je     {STEP + 44:#x}",
            "push   eax",
            f"call   {iat('Bar')}",
            "add    esp,0x4",
            "ret",
        ]}, STEP)
        self.assertIsNone(f.error, f.error)
        lua = render(d, f)
        self.assertIn("R.eax = 1", lua.replace("R.ecx, R.eax = ", "").replace("R.eax, R.ecx = ", ""), lua)
        self.assertIn("A('Bar', R.eax)", lua)
        self.assertTrue(luac_ok(lua), lua)

    def test_a_register_proved_zero_by_the_branch_is_a_null_argument(self):
        # the compiler pushes eax after `test eax,eax; jne` because the edge proved it 0
        d, _, f = lift({STEP: [
            f"mov    eax,DWORD PTR ds:{SHIP:#x}",
            "test   eax,eax",
            f"jne    {STEP + 28:#x}",
            "push   eax",
            "push   0x3",
            f"call   {iat('Count')}",
            "add    esp,0x8",
            "ret",
        ]}, STEP)
        self.assertIsNone(f.error, f.error)
        lua = render(d, f)
        self.assertIn("A('Count', 3, nil)", lua)
        self.assertTrue(luac_ok(lua), lua)

    def test_a_register_proved_equal_to_a_constant_by_cmp_je(self):
        d, _, f = lift({STEP: [
            f"mov    eax,DWORD PTR ds:{SHIP:#x}",
            "cmp    eax,0x7",
            f"jne    {STEP + 24:#x}",
            "push   eax",
            f"call   {iat('Bar')}",
            "add    esp,0x4",
            "ret",
        ]}, STEP)
        self.assertIsNone(f.error, f.error)
        lua = render(d, f)
        self.assertIn("A('Bar', 7)", lua)

    def test_the_taken_edge_of_jne_proves_nothing(self):
        d, _, f = lift({STEP: [
            f"mov    eax,DWORD PTR ds:{SHIP:#x}",
            "test   eax,eax",
            f"je     {STEP + 24:#x}",
            "ret",
            "ret",
            "ret",
            "push   eax",
            f"call   {iat('Bar')}",
            "add    esp,0x4",
            "ret",
        ]}, STEP)
        self.assertIsNone(f.error, f.error)
        self.assertIn("A('Bar', 0)", render(d, f))

    def test_a_loop_is_not_decoded_but_its_calls_are_listed(self):
        d, _, f = lift({STEP: [f"call   {iat('Bar')}", f"jmp    {STEP:#x}"]}, STEP)
        self.assertIn("loop", f.error)
        self.assertEqual(f.imports_called, ["Bar"])

    def test_an_unbalanced_return_is_not_decoded_and_lists_its_calls(self):
        d, _, f = lift({STEP: ["push   0x1", f"call   {iat('Bar')}", "ret"]}, STEP)
        self.assertIn("unbalanced stack", f.error)
        lines = a1.render_failed("fn", f.error, sorted(set(f.imports_called)))
        self.assertIn("M.notDecoded('fn',", lines[1])
        self.assertIn("{ 'Bar' }", lines[1])

    def test_an_unreadable_argument_is_an_undecoded_call(self):
        d, _, f = lift({STEP: ["sub    esp,0x8", f"call   {iat('Foo')}", "add    esp,0x8", "ret"]}, STEP)
        self.assertIsNone(f.error, f.error)
        self.assertEqual([c for c, _ in f.undecoded], ["Foo"])
        self.assertEqual(a1.classify(f), "partial")
        lua = render(d, f)
        self.assertIn("UD('Foo',", lua)

    def test_vectors_by_value_struct_results_and_references(self):
        d, _, f = lift({STEP: [
            "sub    esp,0xc",
            "push   0x2",
            "push   0x1",
            "lea    eax,[esp+0x8]",
            "push   eax",
            f"call   {iat('GetAttackLocation')}",
            "add    esp,0xc",
            "mov    eax,esp",
            f"mov    ecx,DWORD PTR ds:{SPOT:#x}",
            "push   ecx",
            "push   ecx",
            "push   ecx",
            "mov    eax,esp",
            "mov    edx,DWORD PTR [esp+0xc]",
            "mov    DWORD PTR [eax],edx",
            "mov    edx,DWORD PTR [esp+0x10]",
            "mov    DWORD PTR [eax+0x4],edx",
            "mov    edx,DWORD PTR [esp+0x14]",
            "mov    DWORD PTR [eax+0x8],edx",
            f"push   DWORD PTR ds:{SHIP:#x}",
            f"call   {iat('Attack')}",
            "add    esp,0x1c",
            f"push   {SHIP:#x}",
            f"push   {SHIP:#x}",
            f"call   {iat('GetDistance')}",
            "add    esp,0x8",
            "fstp   st(0)",
            "ret",
        ]}, STEP)
        self.assertIsNone(f.error, f.error)
        lua = render(d, f)
        self.assertIn("A('GetAttackLocation', 1, 2)", lua.replace("t[1] = ", ""))
        self.assertIn("A('Attack', O.ship, Vector(", lua)
        self.assertIn("A('GetDistance', O.ship, O.ship)", lua)
        self.assertTrue(luac_ok(lua), lua)

    def test_const_reference_temporaries_are_their_values(self):
        # `Mark(4, true)` with both arguments as `const &` to stack temporaries,
        # the bool one a single byte
        d, _, f = lift({STEP: [
            "sub    esp,0x8",
            "lea    eax,[esp+0x3]",
            "lea    ecx,[esp+0x4]",
            "push   eax",
            "push   ecx",
            "mov    BYTE PTR [esp+0xb],0x1",
            "mov    DWORD PTR [esp+0xc],0x4",
            f"call   {iat('Mark')}",
            "add    esp,0x10",
            "ret",
        ]}, STEP)
        self.assertIsNone(f.error, f.error)
        self.assertIn("A('Mark', 4, true)", render(d, f))

    def test_a_null_name_is_nil(self):
        d, _, f = lift({STEP: ["push   0x0", "push   0x3", f"call   {iat('Count')}", "add    esp,0x8", "ret"]}, STEP)
        self.assertIn("A('Count', 3, nil)", render(d, f))


class Prelude(unittest.TestCase):
    def test_every_table_the_shared_engine_indexes_is_created(self):
        # the shared prelude and engine of tools/dsl_to_lua.py index Mission.<x>[...]:
        # each such table must exist in the A1 Mission_ literal
        import re
        S, R = Missions().mission()
        _, src, _ = a1.convert(S, R, "test1")
        literal = src[src.index("Mission_ = {"):src.index("\n}\n")]
        used = set(re.findall(r"Mission\.(\w+)\[", dsl.PRELUDE + dsl.ENGINE + a1.A1_PRELUDE))
        missing = sorted(n for n in used if not re.search(rf"\b{n} = {{}}", literal))
        self.assertEqual(missing, [])


class Missions(unittest.TestCase):
    maxDiff = 2000

    def mission(self):
        dsl_code = dict(DLLMAIN_CODE)
        dsl_code[INIT] = [
            "push   0x0",
            f"mov    ecx,{ALPHA:#x}",
            f"call   {iat('??4hot_int@@QAEXH@Z')}",
            "push   0x10040000",
            f"call   {iat('GetHandle')}",
            "add    esp,0x4",
            f"mov    ds:{SHIP:#x},eax",
            "ret",
        ]
        dsl_code[ACTION] = [
            f"mov    ecx,{iat('WORLD')}",
            "push   0x10040010",
            f"call   {iat('new_script')}",
            "push   0x10001008",
            "mov    ecx,eax",
            f"call   {iat('add_step')}",
            "ret",
        ]
        dsl_code[STEP] = ["push   0x1", f"mov    ecx,{ALPHA:#x}", f"call   {iat('??4hot_int@@QAEXH@Z')}", "xor    eax,eax", "ret"]
        S = make(dsl_code)
        cond = 0x10007000
        rule_code = dict(DLLMAIN_CODE)
        rule_code[INIT] = [
            f"mov    eax,{iat('WORLD')}",
            "push   esi",
            f"mov    esi,{iat('create_rule')}",
            "push   0x10040020",
            "push   0x10001004",
            "push   0x10001008",
            "lea    ecx,[eax+0x38]",
            "call   esi",
            "mov    ds:0x10050100,eax",
            "push   eax",
            f"mov    ecx,{iat('ALPHA')}",
            f"call   {iat('add_dependent_rule')}",
            "pop    esi",
            "ret",
        ]
        rule_code[0x10001000] = ["jmp    0x10002000", f"jmp    {0x10001010:#x}", f"jmp    {cond:#x}", "jmp    0x10005000",
                                 f"jmp    {iat('DoIt')}"]
        rule_code[cond] = [
            f"mov    ecx,{iat('ALPHA')}",
            f"call   {iat('??Bhot_int@@QAEHXZ')}",
            "test   eax,eax",
            f"je     {cond + 28:#x}",
            f"mov    eax,{iat('ship')}",
            "mov    ecx,DWORD PTR [eax]",
            f"jmp    {cond + 36:#x}",
            "xor    eax,eax",
            "ret",
            f"mov    ecx,{iat('isCineractiveDone')}",
            f"jmp    {iat('??Bhot_int@@QAEHXZ')}",
        ]
        code = dict(ENTRY)
        code.update(rule_code)
        R = a1.A1Dll("Test1R.drl", ins=program(code), pe=FakePe(), exports={}, entry_point=EP)
        return S, R

    def test_rules_actions_steps_and_init_convert(self):
        S, R = self.mission()
        self.assertEqual(S.dllmain, DLLMAIN)
        mission, src, rep = a1.convert(S, R, "test1")
        self.assertEqual(rep["rules"], 1)
        self.assertEqual(rep["rule_actions_resolved"], 1)
        self.assertEqual(rep["registration_errors"], [])
        for role in ("init", "action", "step", "condition"):
            self.assertEqual(rep["functions"][role]["not"], 0, (role, rep))
        self.assertIn("name = 'rule_one',", src)
        self.assertIn("action = 'DoIt',", src)
        self.assertIn("deps = { 'ALPHA' },", src)
        self.assertIn("when = function(S, O) return T(M.R_rule_one()) end,", src)
        self.assertIn("NewScript('the_script')", src)
        self.assertIn("AddStep(", src)
        self.assertIn("A('GetHandle', 'ship_label')", src)
        self.assertIn("O.ship = t[1]", src)
        self.assertIn("function M.ScriptInit() M.", src)
        self.assertIn("BindVar('isCineractiveDone', 'hot_int', EngineVar(", src)
        self.assertIn("local f = A1 and A1[name]", src)
        self.assertIn("do return S.isCineractiveDone end", src)
        self.assertTrue(luac_ok(src), "converted mission does not parse")


class FindMissions(unittest.TestCase):
    """Which DLL pairs convert: those a map names, and on request the ones no map names."""

    def test_unmapped_pairs_follow_only_on_request(self):
        with tempfile.TemporaryDirectory() as t:
            root = pathlib.Path(t)
            (root / "missions").mkdir()
            (root / "bzn").mkdir()
            for stem in ("One", "Two"):
                (root / "missions" / f"{stem}R.drl").write_bytes(b"x")
                (root / "missions" / f"{stem}S.dsl").write_bytes(b"x")
            (root / "bzn" / "map_a.bzn").write_bytes(b"\0onER.drl\0OneR.drl\0OneS.dsl\0")
            mapped = a1.find_missions(str(root))
            self.assertEqual([m[0] for m in mapped], ["map_a"])
            every = a1.find_missions(str(root), include_unmapped=True)
            self.assertEqual([m[0] for m in every], ["map_a", "two"])
            self.assertTrue(every[1][2].endswith("TwoR.drl"))


if __name__ == "__main__":
    unittest.main()
