"""Tests for tools/drl_to_lua.py on synthetic code: no game file is read.

The condition decoder is fed instruction lists in the shape the Armada II
debug compiler emits (checked against the installed `.drl` files, see the
LUA-07 PR); the scaffold renderer is checked against the synthetic scaffold
that crates/sta_mission/tests/drl.rs loads into the mission runtime.
"""

import importlib.util
import pathlib
import re
import struct
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("drl_to_lua", ROOT / "tools" / "drl_to_lua.py")
drl = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(drl)

CHK = 0x10009000
IAT = {
    0x10041000: ("synthS.dsl", "?p_ALPHA@@3PAVhot_int@@A"),
    0x10041004: ("synthS.dsl", "?p_ALWAYS_EVALUATE@@3PAVdirty_int@@A"),
    0x10041008: ("synthS.dsl", "?g_pSI@@3PAVScriptInterface@@A"),
    0x1004100C: ("synthS.dsl", "?ship@@3HA"),
    0x10041010: ("synthS.dsl", "?p_POWER@@3PAVhot_float@@A"),
}


class FakePe:
    base = 0x10000000
    imports = IAT
    floats = {0x10037000: 2.5}

    def section_of(self, addr):
        if addr in self.floats:
            return ".rdata"
        return ".text" if 0x10001000 <= addr < 0x10037000 else None

    def cstr(self, addr):
        raise drl.DecodeError("no string")

    def f32(self, addr):
        return self.floats[addr]


class FakeDrl(drl.Drl):
    """A Drl over a hand-written instruction list."""

    def __init__(self, text, engine=None):
        self.pe = FakePe()
        self.engine = engine or {}
        self.ins = []
        a = 0x10002000
        for line in text.strip().splitlines():
            self.ins.append((a, line.strip()))
            a += 4
        self.at = {x: i for i, (x, _) in enumerate(self.ins)}
        self.thunks = {}
        self.chkesp = CHK


def hot(iat):
    """Read a hot variable through its vtable (`operator int`, slot 8)."""
    return f"""mov    eax,ds:{iat:#x}
mov    ecx,DWORD PTR [eax]
mov    edx,DWORD PTR ds:{iat:#x}
mov    eax,DWORD PTR [edx]
mov    edx,DWORD PTR [eax]
mov    esi,esp
call   DWORD PTR [edx+0x20]
cmp    esi,esp
call   {CHK:#x}"""


PROLOGUE = """push   ebp
mov    ebp,esp
sub    esp,0x44
push   ebx
push   esi
push   edi
lea    edi,[ebp-0x44]
mov    ecx,0x11
mov    eax,0xcccccccc
rep stos DWORD PTR es:[edi],eax"""

EPILOGUE = """pop    edi
pop    esi
pop    ebx
add    esp,0x44
cmp    ebp,esp
call   {chk}
mov    esp,ebp
pop    ebp
ret""".format(chk=hex(CHK))


def and_chain(*reads, invert_last=False):
    """`a && b && ...` as the compiler materialises it in [ebp-0x4]."""
    body = []
    # labels are filled in after layout: use placeholders
    for r in reads:
        body.append(r)
        body.append("test   eax,eax")
        body.append("je     FALSE")
    body += ["mov    DWORD PTR [ebp-0x4],0x1", "jmp    JOIN", "FALSE: mov    DWORD PTR [ebp-0x4],0x0",
             "JOIN: mov    eax,DWORD PTR [ebp-0x4]"]
    return "\n".join(body)


def layout(text):
    """Resolve FALSE:/JOIN: labels into addresses (4 bytes per line)."""
    lines = [x.strip() for x in text.strip().splitlines() if x.strip()]
    labels = {}
    for i, x in enumerate(lines):
        m = re.match(r"(\w+): (.*)$", x)
        if m and m.group(1).isupper():
            labels[m.group(1)] = 0x10002000 + 4 * i
            lines[i] = m.group(2)
    out = []
    for x in lines:
        for k, v in labels.items():
            x = re.sub(rf"\b{k}\b", hex(v), x)
        out.append(x)
    return "\n".join(out)


class ConditionTests(unittest.TestCase):
    def decode(self, body):
        d = FakeDrl(layout(PROLOGUE + "\n" + body + "\n" + EPILOGUE))
        return drl.render(d.condition(0x10002000))

    def test_single_variable(self):
        self.assertEqual(self.decode(hot(0x10041000)), "T(S.p_ALPHA)")

    def test_and_chain_with_engine_call(self):
        call = f"""mov    esi,esp
mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
push   ecx
mov    edx,DWORD PTR ds:0x10041008
mov    ecx,DWORD PTR [edx]
mov    eax,ds:0x10041008
mov    edx,DWORD PTR [eax]
mov    eax,DWORD PTR [edx]
call   DWORD PTR [eax+0x4dc]
cmp    esi,esp
call   {CHK:#x}
and    eax,0xff"""
        got = self.decode(and_chain(hot(0x10041004), hot(0x10041000), call))
        self.assertEqual(got, "T(S.p_ALWAYS_EVALUATE) and T(S.p_ALPHA) and T(A('slot_311', O.ship))")

    def test_not_and_or_through_materialised_values(self):
        # !alpha, then (always || alpha) via a stored 0/1, as `||` compiles.
        body = hot(0x10041000) + """
neg    eax
sbb    eax,eax
inc    eax
and    eax,0xff
test   eax,eax
je     FALSE
""" + hot(0x10041004) + """
test   eax,eax
jne    TRUE
""" + hot(0x10041000) + """
test   eax,eax
je     FALSE
TRUE: mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]"""
        # Decoded as written; the decoder does not use one term to simplify another.
        self.assertEqual(self.decode(body), "not T(S.p_ALPHA) and (T(S.p_ALWAYS_EVALUATE) or T(S.p_ALPHA))")

    def test_float_compare_and_doubling(self):
        # power*2 < 2.5 with the x87 status word (C0 = st0 < m).
        body = hot(0x10041010) + """
fadd   st(0),st
fcomp  DWORD PTR ds:0x10037000
fnstsw ax
test   ah,0x1
je     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]"""
        self.assertEqual(self.decode(body), "(2.0 * S.p_POWER) < 2.5")

    def test_unknown_instruction_is_reported_not_guessed(self):
        with self.assertRaises(drl.DecodeError):
            self.decode(hot(0x10041000) + "\nrol    eax,0x3")


class HelperTests(unittest.TestCase):
    def test_msvc_params(self):
        self.assertEqual(drl.msvc_params("?UnderAttack@ScriptInterfaceImp@@UAE_NHM@Z"), ["H", "M"])
        self.assertEqual(drl.msvc_params("?GetDistance@ScriptInterfaceImp@@UAEMAAH0@Z"), ["AH", "AH"])
        self.assertEqual(drl.msvc_params("?GetTime@ScriptInterfaceImp@@UAEMXZ"), [])

    def test_float_arguments_are_reinterpreted(self):
        bits = struct.unpack("<I", struct.pack("<f", 9.0))[0]
        self.assertEqual(drl.typed_args([("obj", "x"), ("const", bits)], ["H", "M"]),
                         [("obj", "x"), ("float", 9.0)])

    def test_import_kinds(self):
        self.assertEqual(drl.import_kind("?p_A@@3PAVtimer_int@@A"), "timer_int")
        self.assertEqual(drl.import_kind("?Enterprise@@3HA"), "int")
        self.assertEqual(drl.import_kind("?Mission_Win@@YAXXZ"), "action")


def synthetic_mission():
    """The synthetic mission whose scaffold tests/drl.rs runs."""
    alpha = ("and", [("truth", ("var", "p_ALWAYS_EVALUATE", "dirty_int")), ("truth", ("var", "p_ALPHA", "hot_int")),
                     ("not", ("truth", ("call", "IsAlive", [("obj", "ship")])))])
    timer = ("truth", ("var", "p_tWait", "timer_int"))
    missing = ("cmp", ">", ("call", "NotBoundYet", [("const", 1)]), ("const", 0))
    rules = []
    for name, action, deps, e in (("AlphaRule", "Do_Alpha", ["p_ALPHA", "p_ALWAYS_EVALUATE"], alpha),
                                  ("TimerRule", "Do_Timer", ["p_tWait"], timer),
                                  ("MissingRule", "Do_Missing", ["p_ALWAYS_EVALUATE"], missing)):
        rules.append({"name": name, "action": action, "deps": deps, "condition": e,
                      "condition_lua": drl.render(e)})
    return {
        "mission": "synthetic01",
        "drl": "synthetic01R.drl",
        "dsl": "synthetic01S.dsl",
        "variables": [
            {"name": "p_ALPHA", "kind": "hot_int"},
            {"name": "p_ALWAYS_EVALUATE", "kind": "dirty_int", "engine": True},
            {"name": "p_tWait", "kind": "timer_int"},
        ],
        "objects": ["ship"],
        "actions": ["Do_Alpha", "Do_Timer", "Do_Missing"],
        "rules": rules,
    }


class ScaffoldTests(unittest.TestCase):
    def test_rust_fixture_is_the_tools_output(self):
        src = (ROOT / "crates" / "sta_mission" / "tests" / "drl.rs").read_text(encoding="utf-8")
        m = re.search(r'const SYNTHETIC_SCAFFOLD: &str = r#"(.*?)"#;', src, re.S)
        self.assertIsNotNone(m, "fixture not found in tests/drl.rs")
        self.assertEqual(m.group(1), drl.scaffold(synthetic_mission()),
                         "regenerate the fixture: python3 tools/test_drl_to_lua.py --print-fixture")


if __name__ == "__main__":
    import sys
    if "--print-fixture" in sys.argv:
        sys.stdout.write(drl.scaffold(synthetic_mission()))
    else:
        unittest.main()
