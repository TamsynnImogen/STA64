#!/usr/bin/env python3
"""Writes `cases.txt`: invented rule-DLL listings and what the ORIGINAL Python functions of
`tools/drl_to_lua.py` make of them (LUA-15g).

This script reads no game files. Every listing, address, symbol, string and engine name below is
invented for the test; the expected values are computed by importing `tools/drl_to_lua.py` and
running its own `Drl.condition`, `Drl.rules` and `decode` on those listings (the file reading
and `objdump` of the original are replaced by the invented data). The Rust test
`tests/drl.rs` runs the same cases through `sta_mission_convert::drl` and compares.

    python3 -I crates/sta_mission_convert/tests/drl_cases/gen_cases.py [output file]

Without an argument it writes `cases.txt` next to itself; the test `cases_file_is_current`
regenerates into a temporary file and compares (CPython 3.14 only: some exception texts differ
between versions).

File format (`cases.txt`, one `tag|payload` per line):
    case|<name>                       starts a case
    iat|<addr>|<dll>|<symbol>         an import (in this order)
    str|<addr>|<text>                 a NUL-terminated string in the image
    f32|<addr>|<bytes hex>            four bytes of a float (little endian)
    f64|<addr>|<bytes hex>            eight bytes of a double
    export|<size>                     the export directory size field
    engine|<class>|<name>|<name>...   engine vtable slot names (slot order)
    ins|<addr>|<text>                 the listing, in order
    op|condition|<addr> / op|rules / op|decode|<file name>
    then the expected result lines: ok|.. err|.. raw|.. slots|.. shapes|.. rule|.. dep|.. out|..
"""

import importlib.util
import pathlib
import re
import struct
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SPEC = importlib.util.spec_from_file_location("drl_to_lua", ROOT / "tools" / "drl_to_lua.py")
drl = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(drl)

CHK = 0x10009000
TEXT = (0x10001000, 0x10036000)
RDATA = (0x10036000, 0x10040000)


class FakePe:
    base = 0x10000000

    def __init__(self, imports, strings, floats, doubles, export_size):
        self.imports = dict(imports)
        self.strings = strings
        self.floats = floats
        self.doubles = doubles
        self.export_size = export_size

    def f64(self, addr):
        if addr in self.doubles:
            return self.doubles[addr]
        raise drl.DecodeError(f"double at {addr:#x} outside the file")

    def section_of(self, addr):
        if TEXT[0] <= addr < TEXT[1]:
            return ".text"
        if RDATA[0] <= addr < RDATA[1]:
            return ".rdata"
        return None

    def cstr(self, addr):
        if addr in self.strings:
            return self.strings[addr]
        raise drl.DecodeError(f"string at {addr:#x} outside the file")

    def f32(self, addr):
        if addr in self.floats:
            return self.floats[addr]
        raise drl.DecodeError(f"float at {addr:#x} outside the file")


FILES = {}


def fake_pe(path):
    return FILES[path]["pe"]


def fake_disassemble(path):
    return FILES[path]["ins"]


drl.Pe = fake_pe
drl.disassemble = fake_disassemble

# ------------------------------------------------------------------ listing builders

HOT = """mov    eax,ds:{iat:#x}
mov    ecx,DWORD PTR [eax]
mov    edx,DWORD PTR ds:{iat:#x}
mov    eax,DWORD PTR [edx]
mov    edx,DWORD PTR [eax]
mov    esi,esp
call   DWORD PTR [edx+0x20]
cmp    esi,esp
call   {chk:#x}"""

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

EPILOGUE = f"""pop    edi
pop    esi
pop    ebx
add    esp,0x44
cmp    ebp,esp
call   {CHK:#x}
mov    esp,ebp
pop    ebp
ret"""

# a call of an engine method on g_pSI (slot 0x7f0 / 4) with one `int` handle argument
ENGINE_CALL = f"""mov    esi,esp
mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
push   ecx
mov    edx,DWORD PTR ds:0x10041008
mov    ecx,DWORD PTR [edx]
mov    eax,ds:0x10041008
mov    edx,DWORD PTR [eax]
mov    eax,DWORD PTR [edx]
call   DWORD PTR [eax+0x7f0]
cmp    esi,esp
call   {CHK:#x}
and    eax,0xff"""

IAT = [
    (0x10041000, "synthS.dsl", "?p_ALPHA@@3PAVhot_int@@A"),
    (0x10041004, "engine.dll", "?p_ALWAYS_EVALUATE@@3PAVdirty_int@@A"),
    (0x10041008, "synthS.dsl", "?g_pSI@@3PAVScriptInterface@@A"),
    (0x1004100C, "synthS.dsl", "?ship@@3HA"),
    (0x10041010, "synthS.dsl", "?p_POWER@@3PAVhot_float@@A"),
    (0x10041014, "synthS.dsl", "?Do_Alpha@@YAXXZ"),
    (0x10041018, "synthS.dsl", "?p_tWait@@3PAVtimer_int@@A"),
    (0x1004101C, "synthS.dsl", "?speed@@3MA"),
    (0x10041020, "synthS.dsl", "?arr@@3PAHA"),
    (0x10041024, "synthS.dsl", "?Do_Beta@@YAXXZ"),
    (0x10041028, "synthS.dsl", "?p_WORLD@@3PAVcharacter@@A"),
    (0x1004102C, "synthS.dsl", "?Check_It@@YAHH@Z"),
    (0x10041030, "synthS.dsl", "?odd@@3VFoo@@A"),
    (0x10041034, "synthS.dsl", "Do-It"),
    (0x10041038, "synthS.dsl", "?p_ship@@3PAVhot_int@@A"),
]
FLOATS = {0x10037000: 2.5, 0x10037004: 9.0, 0x10037008: 0.1}
DOUBLES = {0x10037010: 3.25}
STRINGS = {0x10036000: "hello", 0x10036010: "it's a \\ test", 0x10036100: "RuleOne", 0x10036110: "RuleTwo", 0x10036120: "Do-it"}


def resolve(text, base):
    """Lines (4 bytes each) from `base`; `NAME: text` defines a label, bare NAME uses one."""
    lines = [x.strip() for x in text.strip().splitlines() if x.strip()]
    labels = {}
    for i, x in enumerate(lines):
        m = re.match(r"([A-Z]+\d*): (.*)$", x)
        if m:
            labels[m.group(1)] = base + 4 * i
            lines[i] = m.group(2)
    out = []
    for i, x in enumerate(lines):
        for k, v in labels.items():
            x = re.sub(rf"\b{k}\b", hex(v), x)
        out.append((base + 4 * i, x))
    return out


def func(body, base, chk=True):
    return resolve(PROLOGUE + "\n" + body + "\n" + EPILOGUE, base)


def hot(iat):
    return HOT.format(iat=iat, chk=CHK)


def and_chain(*reads):
    body = []
    for r in reads:
        body += [r, "test   eax,eax", "je     FALSE"]
    body += ["mov    DWORD PTR [ebp-0x4],0x1", "jmp    JOIN", "FALSE: mov    DWORD PTR [ebp-0x4],0x0",
             "JOIN: mov    eax,DWORD PTR [ebp-0x4]"]
    return "\n".join(body)


def result_of(body):
    return body + "\nmov    DWORD PTR [ebp-0x4],0x1\njmp    JOIN\nFALSE: mov    DWORD PTR [ebp-0x4],0x0\nJOIN: mov    eax,DWORD PTR [ebp-0x4]"


# ------------------------------------------------------------------ case runner

OUT = []


class Case:
    def __init__(self, name, ins, engine=None, export_size=0, iat=None, strings=None, floats=None):
        self.name, self.ins, self.engine = name, ins, engine
        self.iat = IAT if iat is None else iat
        self.strings = STRINGS if strings is None else strings
        self.floats = FLOATS if floats is None else floats
        self.export_size = export_size
        self.lines = [f"case|{name}"]
        for a, dll, sym in self.iat:
            self.lines.append(f"iat|{a:#x}|{dll}|{sym}")
        for a, s in self.strings.items():
            self.lines.append(f"str|{a:#x}|{s}")
        for a, f in self.floats.items():
            self.lines.append(f"f32|{a:#x}|{struct.pack('<f', f).hex()}")
        for a, f in DOUBLES.items():
            self.lines.append(f"f64|{a:#x}|{struct.pack('<d', f).hex()}")
        if export_size:
            self.lines.append(f"export|{export_size}")
        for cls, names in (engine or {}).items():
            self.lines.append("engine|" + cls + "|" + "|".join(names))
        for a, t in ins:
            self.lines.append(f"ins|{a:#x}|{t}")
        floats32 = {a: struct.unpack("<f", struct.pack("<f", f))[0] for a, f in self.floats.items()}
        FILES[name + "R.drl"] = {
            "pe": FakePe({a: (d, s) for a, d, s in self.iat}, self.strings, floats32, DOUBLES, export_size),
            "ins": list(ins),
        }

    def finish(self):
        OUT.extend(self.lines)


def raw_name(e):
    return type(e).__name__


def condition(name, body, addr=0x10002000, engine=None, full=False, **kw):
    ins = body if full else func(body, 0x10002000)
    c = Case(name, ins, engine, **kw)
    c.lines.append(f"op|condition|{addr:#x}")
    try:
        d = drl.Drl(name + "R.drl", engine)
        e = d.condition(addr)
        c.lines.append("ok|" + drl.render(e))
        c.lines.append("slots|" + ",".join(str(s) for s in sorted(d.engine_slots)))
        c.lines.append("shapes|" + ",".join(drl.call_shapes(e)))
    except drl.DecodeError as ex:
        c.lines.append("err|" + str(ex))
    except Exception as ex:  # a raw exception aborts the Python run
        c.lines.append("raw|" + raw_name(ex))
    c.finish()


def rules_case(name, ins, engine=None, **kw):
    c = Case(name, ins, engine, **kw)
    c.lines.append("op|rules")
    try:
        d = drl.Drl(name + "R.drl", engine)
        created, deps = d.rules()
        for n, cond, act in created:
            c.lines.append(f"rule|{n}|{cond:#x}|{act:#x}")
        for k in sorted(deps):
            c.lines.append(f"dep|{k}|{','.join(deps[k])}")
    except drl.DecodeError as ex:
        c.lines.append("err|" + str(ex))
    except Exception as ex:
        c.lines.append("raw|" + raw_name(ex))
    c.finish()


def decode_case(name, ins, engine=None, **kw):
    c = Case(name, ins, engine, **kw)
    base = name + "R.drl"
    c.lines.append(f"op|decode|{base}")
    try:
        m = drl.decode(base, engine)
        c.lines.append("calls|" + ",".join(m["calls"]))
        c.lines.append("sites|" + ",".join(f"{k}={v}" for k, v in m["call_sites"].items()))
        c.lines.append("sigs|" + ",".join(m["call_signatures"]))
        c.lines.append("unused|" + ",".join(m["unused_variables"]))
        c.lines.append("errors|" + "\\n".join(m["errors"]))
        text = drl.scaffold(m)
        first, rest = text.split("\n", 1)
        # the port names itself in the header (`sta_mission_convert::tool_token()`)
        first = first.replace("tools/drl_to_lua.py (LUA-07)", "<TOOL>")
        for line in (first + "\n" + rest).split("\n")[:-1]:
            c.lines.append("out|" + line)
    except drl.Unsupported as ex:
        c.lines.append("unsupported|" + str(ex))
    except drl.DecodeError as ex:
        c.lines.append("err|" + str(ex))
    except Exception as ex:
        c.lines.append("raw|" + raw_name(ex))
    c.finish()


# ------------------------------------------------------------------ condition cases

ALPHA, ALWAYS, SI, SHIP, POWER = 0x10041000, 0x10041004, 0x10041008, 0x1004100C, 0x10041010
TWAIT, SPEED, ARR, WORLD = 0x10041018, 0x1004101C, 0x10041020, 0x10041028
FLOAT25, FLOAT9, FLOAT01 = 0x10037000, 0x10037004, 0x10037008
SYNTH = {
    "ScriptInterfaceImp": ["?pad@ScriptInterfaceImp@@UAEXXZ"] * 508
    + ["?Probe@ScriptInterfaceImp@@UAEMHM_N@Z", "?Other@ScriptInterfaceImp@@UAE_NXZ"]
    + ["?ScriptGet_ship@ScriptInterfaceImp@@UAEHXZ"]
}

condition("single_variable", result_of(hot(ALPHA) + "\ntest   eax,eax\nje     FALSE"))
condition("and_chain_engine_call_unnamed", and_chain(hot(ALWAYS), hot(ALPHA), ENGINE_CALL))
condition("and_chain_engine_call_named", and_chain(hot(ALWAYS), hot(ALPHA), ENGINE_CALL), engine=SYNTH)

NOT_OR = hot(ALPHA) + """
neg    eax
sbb    eax,eax
inc    eax
and    eax,0xff
test   eax,eax
je     FALSE
""" + hot(ALWAYS) + """
test   eax,eax
jne    TRUE
""" + hot(ALPHA) + """
test   eax,eax
je     FALSE
TRUE: mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]"""
condition("not_and_or_materialised", NOT_OR)

condition("float_compare_doubling", hot(POWER) + """
fadd   st(0),st
fcomp  DWORD PTR ds:0x10037000
fnstsw ax
test   ah,0x1
je     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")

condition("unknown_instruction", hot(ALPHA) + "\nrol    eax,0x3")

CMPVAR = """mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,{imm}
{jcc}     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]"""
for jcc in ("je", "jne", "jl", "jge", "jle", "jg"):
    condition(f"cmp_{jcc}", CMPVAR.format(imm="0x5", jcc=jcc))
condition("cmp_negative_constant", CMPVAR.format(imm="0xffffffff", jcc="je"))
condition("cmp_unsupported_jump", CMPVAR.format(imm="0x5", jcc="jb"))
condition("cmp_with_register", CMPVAR.format(imm="eax", jcc="je"))
condition("cmp_unknown_operand", CMPVAR.format(imm="DWORD PTR [esp+0x4]", jcc="je"))
condition("test_then_jl", """mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
test   ecx,ecx
jl     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")

SETCC = """mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,0x3
{set}  {reg}
and    eax,0xff"""
for st_, reg in (("sete", "al"), ("setne", "al"), ("setl", "dl"), ("setge", "al"), ("setle", "al"), ("setg", "al"),
                 ("seta", "al"), ("sete", "ax"), ("sete", "cl")):
    condition(f"setcc_{st_}_{reg}", SETCC.format(set=st_, reg=reg))
condition("setcc_without_compare", "sete   al\nand    eax,0xff")

condition("xor_eax", "xor    eax,eax")
condition("xor_eax_inc", "xor    eax,eax\ninc    eax")
condition("xor_other", "xor    eax,ecx")
condition("inc_arith", "mov    eax,ds:0x1004100c\nmov    ecx,DWORD PTR [eax]\nmov    eax,ecx\ninc    eax\nand    eax,0xff")
condition("inc_other", "inc    ecx")
condition("sub_then_test", """mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
sub    ecx,0x2
mov    eax,ecx
test   eax,eax
je     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("add_in_register", "mov    eax,ds:0x1004100c\nmov    ecx,DWORD PTR [eax]\nadd    ecx,0x2\nmov    eax,ecx\nand    eax,0xff")
condition("add_memory_destination", "add    DWORD PTR [ebp-0x4],0x2")
condition("and_other_mask", "and    eax,0x1")
condition("or_unsupported", "or     eax,0x1")
condition("neg_other", "neg    ecx")
condition("sbb_without_neg", "sbb    eax,eax")
condition("neg_m1", "mov    eax,ds:0x1004100c\nmov    ecx,DWORD PTR [eax]\nmov    eax,ecx\nneg    eax\nsbb    eax,eax\nneg    eax\nand    eax,0xff")
condition("backward_jump", "TOP: nop\njmp    TOP")
condition("branch_without_compare", "je     FALSE\nFALSE: mov    eax,0x0")
condition("jump_outside_the_code", "jmp    0x20000000")
condition("test_two_registers", "test   eax,ecx")
condition("test_ah_without_status", "test   ah,0x1")
condition("test_status_masks", hot(POWER) + """
fcom   DWORD PTR ds:0x10037000
fnstsw ax
test   ah,0x41
jne    TRUE
mov    eax,0x0
jmp    JOIN
TRUE: mov    eax,0x1
JOIN: nop""")
for mask in ("0x5", "0x4", "0x80", "0x40", "0x44", "0x45"):
    condition(f"status_mask_{mask}", hot(POWER) + f"""
fcomp  DWORD PTR ds:0x10037000
fnstsw ax
test   ah,{mask}
jne    FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("fnstsw_without_compare", "fnstsw ax")
condition("status_je", hot(POWER) + """
fcomp  DWORD PTR ds:0x10037000
fnstsw ax
test   ah,0x1
je     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("status_other_jump", hot(POWER) + """
fcomp  DWORD PTR ds:0x10037000
fnstsw ax
test   ah,0x1
jl     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("call_action_import", "mov    esi,esp\ncall   DWORD PTR ds:0x10041014\ncmp    esi,esp\ncall   0x10009000\nand    eax,0xff")
condition("call_function_import_with_args", "mov    esi,esp\npush   0x41100000\ncall   DWORD PTR ds:0x1004102c\ncmp    esi,esp\ncall   0x10009000\nand    eax,0xff")
condition("call_non_function_import", "mov    esi,esp\ncall   DWORD PTR ds:0x10041030\ncmp    esi,esp\ncall   0x10009000\nand    eax,0xff")
condition("call_variable_with_argument", "mov    eax,ds:0x10041000\nmov    ecx,DWORD PTR [eax]\nmov    edx,DWORD PTR [ecx]\nmov    esi,esp\npush   0x1\ncall   DWORD PTR [edx+0x20]\ncmp    esi,esp\ncall   0x10009000\nand    eax,0xff")
condition("call_variable_wrong_slot", "mov    eax,ds:0x10041000\nmov    ecx,DWORD PTR [eax]\nmov    edx,DWORD PTR [ecx]\nmov    esi,esp\ncall   DWORD PTR [edx+0x24]\ncmp    esi,esp\ncall   0x10009000\nand    eax,0xff")
condition("call_other_object", "mov    eax,ds:0x10041028\nmov    ecx,DWORD PTR [eax]\nmov    edx,DWORD PTR [ecx]\nmov    esi,esp\ncall   DWORD PTR [edx+0x8]\ncmp    esi,esp\ncall   0x10009000\nand    eax,0xff")
condition("call_register", "mov    eax,0x1\ncall   eax")
condition("call_unknown_register", "call   ecx")
condition("call_vtable_slot_zero", "mov    eax,ds:0x10041000\nmov    ecx,DWORD PTR [eax]\nmov    edx,DWORD PTR [ecx]\ncall   DWORD PTR [edx]")
condition("call_on_iiie", "mov    eax,0x1\nmov    ecx,DWORD PTR [eax]\ncall   DWORD PTR [ecx+0x4]")
condition("store_to_memory", "mov    DWORD PTR [eax],ecx")
condition("store_without_comma", "mov    eax")
condition("jump_garbage_operand", "je     zzz")
condition("jmp_indirect", "jmp    DWORD PTR [eax]")
condition("return_unknown_value", "mov    eax,0x0\nmov    eax,ecx")
condition("array_element", "mov    eax,ds:0x10041020\nmov    ecx,DWORD PTR [eax]\nmov    eax,ecx\nand    eax,0xff")
condition("float_object_value", "mov    eax,ds:0x1004101c\nmov    ecx,DWORD PTR [eax]\nmov    eax,ecx\nand    eax,0xff")
condition("float_handle_compare", """mov    eax,ds:0x1004101c
fld    DWORD PTR [eax]
fcomp  DWORD PTR ds:0x10037004
fnstsw ax
test   ah,0x41
jne    FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("float_arith_forms", hot(POWER) + """
fmul   DWORD PTR ds:0x10037000
fsub   DWORD PTR ds:0x10037004
fdivr  DWORD PTR ds:0x10037008
fcompp
fnstsw ax
test   ah,0x1
je     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("float_arith_st_forms", hot(POWER) + """
fld    DWORD PTR ds:0x10037000
fsubr  st(0),st
fdiv   st,st(0)
fcompp
fnstsw ax
test   ah,0x1
je     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("float_store_and_reload", hot(POWER) + """
fstp   DWORD PTR [ebp-0x8]
fld    DWORD PTR [ebp-0x8]
fld    QWORD PTR [ebp-0x8]
fcomp  st(1)
fnstsw ax
test   ah,0x1
je     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("float_st_register_operand", hot(POWER) + """
fld    DWORD PTR ds:0x10037000
fadd   st(1)
fcom   st(0)
fnstsw ax""")
condition("float_st_empty", "fld    st(3)")
condition("float_fstp_pop", hot(POWER) + "\nfstp   st(0)\nfld    DWORD PTR ds:0x10037000\nfcomp  DWORD PTR ds:0x10037000\nfnstsw ax\ntest   ah,0x40\nje     FALSE\nmov    eax,0x1\nFALSE: nop")
condition("float_fstp_bad_target", hot(POWER) + "\nfstp   DWORD PTR [eax]")
condition("float_stack_empty", "fstp   DWORD PTR [ebp-0x8]")
condition("float_integer_load", "fild   DWORD PTR ds:0x10037000")
condition("float_integer_load_of_handle", "mov    eax,ds:0x1004100c\nfild   DWORD PTR [eax]\nfstp   DWORD PTR [ebp-0x8]")
condition("float_load_outside_file", "fld    DWORD PTR ds:0x10050000", )
condition("float_double_compare", hot(POWER) + """
fcomp  QWORD PTR ds:0x10037010
fnstsw ax
test   ah,0x1
je     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("float_double_outside_the_file", "fld    QWORD PTR ds:0x10050000")
condition("float_unsupported_operand", "fld    TBYTE PTR [ebp-0x8]")
condition("float_load_from_import_value", "mov    eax,ds:0x10041000\nmov    ecx,DWORD PTR [eax]\nfld    DWORD PTR [ecx]")
condition("float_ftol_unsupported", "fistp  DWORD PTR [ebp-0x8]")
condition("float_call_result_compared", """mov    esi,esp
mov    edx,DWORD PTR ds:0x10041008
mov    ecx,DWORD PTR [edx]
mov    eax,ds:0x10041008
mov    edx,DWORD PTR [eax]
mov    eax,DWORD PTR [edx]
call   DWORD PTR [eax+0x7f0]
cmp    esi,esp
call   0x10009000
fcomp  DWORD PTR ds:0x10037000
fnstsw ax
test   ah,0x41
jne    FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""", engine=SYNTH)

TYPED = """mov    esi,esp
push   0x1
push   0x41100000
mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
push   ecx
mov    edx,DWORD PTR ds:0x10041008
mov    ecx,DWORD PTR [edx]
mov    eax,ds:0x10041008
mov    edx,DWORD PTR [eax]
mov    eax,DWORD PTR [edx]
call   DWORD PTR [eax+0x7f0]
cmp    esi,esp
call   0x10009000
and    eax,0xff"""
condition("typed_arguments", TYPED, engine=SYNTH)
condition("typed_arguments_unnamed", TYPED)
condition("argument_string_and_escape", """mov    esi,esp
push   0x10036010
push   0x10036000
mov    edx,DWORD PTR ds:0x10041008
mov    ecx,DWORD PTR [edx]
mov    eax,ds:0x10041008
mov    edx,DWORD PTR [eax]
mov    eax,DWORD PTR [edx]
call   DWORD PTR [eax+0x7f4]
cmp    esi,esp
call   0x10009000
and    eax,0xff""", engine=SYNTH)
condition("argument_array_and_negative_constant", """mov    esi,esp
push   0xffffffff
mov    eax,0x10041020
mov    eax,ds:0x10041020
push   eax
mov    edx,DWORD PTR ds:0x10041008
mov    ecx,DWORD PTR [edx]
mov    eax,ds:0x10041008
mov    edx,DWORD PTR [eax]
mov    eax,DWORD PTR [edx]
call   DWORD PTR [eax+0x7f0]
cmp    esi,esp
call   0x10009000
and    eax,0xff""")
condition("argument_reference_to_non_int", """mov    esi,esp
mov    eax,ds:0x10041010
push   eax
mov    edx,DWORD PTR ds:0x10041008
mov    ecx,DWORD PTR [edx]
mov    eax,ds:0x10041008
mov    edx,DWORD PTR [eax]
mov    eax,DWORD PTR [edx]
call   DWORD PTR [eax+0x7f0]
cmp    esi,esp
call   0x10009000
and    eax,0xff""")
condition("argument_unknown", """mov    esi,esp
push   ecx
mov    edx,DWORD PTR ds:0x10041008
mov    ecx,DWORD PTR [edx]
mov    eax,ds:0x10041008
mov    edx,DWORD PTR [eax]
mov    eax,DWORD PTR [edx]
call   DWORD PTR [eax+0x7f0]
cmp    esi,esp
call   0x10009000
and    eax,0xff""")
condition("argument_unsupported_kind", """mov    esi,esp
mov    eax,ds:0x10041000
mov    ecx,DWORD PTR [eax]
push   ecx
mov    edx,DWORD PTR ds:0x10041008
mov    ecx,DWORD PTR [edx]
mov    eax,ds:0x10041008
mov    edx,DWORD PTR [eax]
mov    eax,DWORD PTR [edx]
call   DWORD PTR [eax+0x7f0]
cmp    esi,esp
call   0x10009000
and    eax,0xff""")
condition("join_conflict_value_used", """mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,0x1
je     A
mov    DWORD PTR [ebp-0x4],0x2
jmp    JOIN
A: mov    DWORD PTR [ebp-0x4],0x3
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("join_different_arguments", """mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,0x1
je     A
push   0x1
A: mov    eax,0x1""")
condition("join_value_and_unset_register", """mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,0x1
je     A
mov    edx,0x1
jmp    JOIN
A: mov    esi,0x0
JOIN: mov    eax,0x1""")
condition("constant_branch_folds", "xor    eax,eax\ntest   eax,eax\nje     A\nmov    eax,0x5\nA: nop")
condition("or_of_comparisons", """mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,0x1
je     TRUE
mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,0x2
jne    FALSE
TRUE: mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("and_of_three_comparisons", """mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,0x1
jle    FALSE
mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,0x9
jge    FALSE
mov    eax,ds:0x1004100c
mov    ecx,DWORD PTR [eax]
cmp    ecx,0x5
je     FALSE
mov    DWORD PTR [ebp-0x4],0x1
jmp    JOIN
FALSE: mov    DWORD PTR [ebp-0x4],0x0
JOIN: mov    eax,DWORD PTR [ebp-0x4]""")
condition("ran_off_the_code", [(0x10002000, "nop"), (0x10002004, "nop")], full=True)
condition("no_instruction_at_entry", "nop", addr=0x10002002)
# a blank text is the IndexError of `parts[0]`
condition("blank_text_raw_error", [(0x10002000, "nop"), (0x10002004, ""), (0x10002008, "ret")], full=True)
condition("push_pop_and_adjust", "push   0x1\npop    ecx\npush   0x2\nadd    esp,0x4\nmov    eax,0x1")
condition("memory_forms", """mov    ecx,DWORD PTR [ebp+0x8]
mov    edx,BYTE PTR [eax]
mov    eax,DWORD PTR [ecx+0x20]
mov    eax,DWORD PTR [ecx*4+0x4]
mov    ebx,DWORD PTR ds:0x10041000
mov    eax,ebx
and    eax,0xff""")
condition("immediate_forms", "mov    eax,0x10036000\nmov    ecx,0x10001010\nmov    edx,0xfffffffe\nmov    ebx,0x10036020\nmov    eax,ds:0x10036000")
condition("segment_and_scaled_operand_ignored", "mov    eax,DWORD PTR fs:0x0\nmov    ecx,DWORD PTR [esi+edi*4]")

# ------------------------------------------------------------------ registration cases

# thunk k jumps to the k-th function; the rule arguments are thunk addresses (as in a real DLL)
THUNKS = [(0x10001000 + 5 * k, f"jmp    {t:#x}") for k, t in enumerate([0x10002000, 0x10004000, 0x10005000, 0x10006000, 0x10007000])]
CHKFN = [(CHK, "ret")]


def register_body(rules):
    """The registration function: GetIIIE, then create_rule and the dependencies for each rule."""
    lines = ["mov    eax,ds:0x10041028", "mov    ecx,DWORD PTR [eax]", "mov    edx,DWORD PTR [ecx]", "mov    esi,esp",
             "call   DWORD PTR [edx]", "cmp    esi,esp", f"call   {CHK:#x}", "mov    DWORD PTR [ebp-0x4],eax"]
    for i, (name_addr, cond, act, deps) in enumerate(rules):
        lines += ["mov    esi,esp", f"push   {name_addr:#x}", f"push   {act:#x}", f"push   {cond:#x}",
                  "mov    ecx,DWORD PTR [ebp-0x4]", "mov    edx,DWORD PTR [ecx]", "mov    ecx,DWORD PTR [ebp-0x4]",
                  "call   DWORD PTR [edx+0x4]", "cmp    esi,esp", f"call   {CHK:#x}", "mov    DWORD PTR [ebp-0x8],eax",
                  f"mov    DWORD PTR ds:{0x10040000 + 4 * i:#x},eax"]
        for iat in deps:
            lines += [f"mov    eax,ds:{iat:#x}", "mov    ecx,DWORD PTR [eax]", "mov    edx,DWORD PTR [ecx]",
                      "mov    esi,esp", f"mov    eax,DWORD PTR ds:{0x10040000 + 4 * i:#x}", "push   eax",
                      "call   DWORD PTR [edx+0x4]", "cmp    esi,esp", f"call   {CHK:#x}"]
    return "\n".join(lines)


def action_body(iat):
    return f"call   DWORD PTR ds:{iat:#x}"


def full_dll(rules, conds, actions, extra=()):
    """Thunks, the registration function, the condition and action functions."""
    ins = list(THUNKS) + func(register_body(rules), 0x10002000)
    ins += [(0x10003800, "int3")]
    for base, body in conds:
        ins += func(body, base)
    for base, body in actions:
        ins += func(body, base)
    ins += CHKFN + list(extra)
    return sorted(ins)


COND_A = result_of(hot(ALPHA) + "\ntest   eax,eax\nje     FALSE")
COND_B = result_of(hot(POWER) + "\nfadd   st(0),st\nfcomp  DWORD PTR ds:0x10037000\nfnstsw ax\ntest   ah,0x1\nje     FALSE")

RULES2 = [(0x10036100, 0x10001005, 0x1000100A, [ALPHA, ALWAYS]), (0x10036110, 0x1000100F, 0x10001014, [0x10041018])]
CONDS2 = [(0x10004000, COND_A), (0x10006000, COND_B)]
ACTS2 = [(0x10005000, action_body(0x10041014)), (0x10007000, action_body(0x10041024))]

rules_case("registration_two_rules", full_dll(RULES2, CONDS2, ACTS2))
rules_case("registration_bad_arguments", full_dll([(0x10036100, 0x10001005, 0x5, [])], CONDS2, ACTS2))
rules_case("registration_unnamed_string", full_dll([(0x10036200, 0x10001005, 0x1000100A, [])], CONDS2, ACTS2))
rules_case("registration_no_rules", list(THUNKS) + [(0x10002000, "ret"), (0x10004000, "ret"), (0x10005000, "ret"),
                                                    (0x10006000, "ret"), (0x10007000, "ret")])
rules_case("registration_blank_text", list(THUNKS) + [(0x10002000, "push   ebp"), (0x10002004, "")])
rules_case("registration_mov_without_comma", list(THUNKS) + [(0x10002000, "mov    eax")])
rules_case("registration_thunk_target_outside", [(0x10001000, "jmp    0x10099000"), (0x10001005, "ret")])

decode_case("decode_two_rules", full_dll(RULES2, CONDS2, ACTS2))
decode_case("decode_two_rules_named", full_dll(RULES2, CONDS2, ACTS2), engine={**SYNTH, "character": [], "IIIE": [], "hot_int": []})
decode_case("decode_duplicate_rule_names", full_dll(
    [(0x10036100, 0x10001005, 0x1000100A, [ALPHA]), (0x10036100, 0x1000100F, 0x10001014, [ALPHA])], CONDS2, ACTS2))
decode_case("decode_condition_not_decoded", full_dll(
    RULES2, [(0x10004000, hot(ALPHA) + "\nrol    eax,0x3"), (0x10006000, COND_B)], ACTS2))
decode_case("decode_action_not_a_lua_identifier", full_dll(
    [(0x10036100, 0x10001005, 0x1000100A, [ALPHA])], [(0x10004000, COND_A)], [(0x10005000, action_body(0x10041034))]))
decode_case("decode_action_calls_two", full_dll(
    [(0x10036100, 0x10001005, 0x1000100A, [ALPHA])], [(0x10004000, COND_A)],
    [(0x10005000, action_body(0x10041014) + "\n" + action_body(0x10041024))]))
decode_case("decode_action_calls_none", full_dll(
    [(0x10036100, 0x10001005, 0x1000100A, [ALPHA])], [(0x10004000, COND_A)], [(0x10005000, "nop")]))
decode_case("decode_action_calls_own_code", full_dll(
    [(0x10036100, 0x10001005, 0x1000100A, [ALPHA])], [(0x10004000, COND_A)],
    [(0x10005000, "call   0x10004000")]))
decode_case("decode_action_indirect_call", full_dll(
    [(0x10036100, 0x10001005, 0x1000100A, [ALPHA])], [(0x10004000, COND_A)],
    [(0x10005000, "call   DWORD PTR [eax+0x4]")]))
decode_case("decode_action_call_of_non_import", full_dll(
    [(0x10036100, 0x10001005, 0x1000100A, [ALPHA])], [(0x10004000, COND_A)],
    [(0x10005000, "call   DWORD PTR ds:0x10042000")]))
decode_case("decode_action_outside_the_code", full_dll(
    [(0x10036100, 0x10001005, 0x10002002, [ALPHA])], [(0x10004000, COND_A)], [(0x10005000, "ret")]))
decode_case("decode_export_table", full_dll(RULES2, CONDS2, ACTS2), export_size=0x40)
decode_case("decode_armada_one_shape", full_dll(RULES2, CONDS2, ACTS2), iat=[
    (0x10041000, "synthS.dsl", "?p_ALPHA@@3PAVhot_int@@A"), (0x10041028, "synthS.dsl", "?p_WORLD@@3PAVcharacter@@A"),
    (0x10041030, "Armada.exe", "?Foo@@YAXXZ"), (0x10041004, "synthS.dsl", "?p_ALWAYS_EVALUATE@@3PAVdirty_int@@A"),
    (0x10041008, "synthS.dsl", "?ship@@3HA"), (0x1004100C, "synthS.dsl", "?Do_Alpha@@YAXXZ")])
decode_case("decode_no_create_rule", list(THUNKS) + [(0x10002000, "ret"), (0x10004000, "ret"), (0x10005000, "ret"),
                                                     (0x10006000, "ret"), (0x10007000, "ret")])
decode_case("decode_empty_listing", [])
decode_case("decode_engine_getters", full_dll(RULES2, CONDS2, ACTS2), engine=SYNTH)
decode_case("decode_unused_and_objects", full_dll(RULES2, CONDS2, ACTS2), iat=IAT + [
    (0x10041040, "synthS.dsl", "?p_UNUSED@@3PAVhot_int@@A"), (0x10041044, "synthS.dsl", "?extra@@3HA"),
    (0x10041048, "other.dll", "?outside@@3HA")])
decode_case("decode_mission_name_from_file", full_dll(RULES2, CONDS2, ACTS2), iat=[
    (a, d.replace("synthS.dsl", "other.dsl"), s) for a, d, s in IAT])
decode_case("decode_no_dsl_import", full_dll(RULES2, CONDS2, ACTS2), iat=[
    (a, d.replace("synthS.dsl", "x.dll"), s) for a, d, s in IAT])


def subdirs_case(name, roots):
    """`install_subdirs` of the original on invented paths (`normpath` of this system: POSIX)."""
    OUT.append(f"case|{name}")
    OUT.append("op|subdirs|" + "|".join(roots))
    for sub in drl.install_subdirs(roots).values():  # a dict by root; the roots here are distinct
        OUT.append(f"out|{sub}")


subdirs_case("subdirs_dotdot", ["/a/b/..", "/a/b/c/..", "x/..", "a/b/../c", "../..", "/a/b/./", "a//b", "/", ".", "", "/..",
                                "/../x", "../x", "a/../..", "/a/b/../../.."])
subdirs_case("subdirs_names", ["/a/Star Trek: A2", "/b/Star Trek: A2/", "/c/star trek: a2", "/d/ok-1.2_x", "/e/a  b",
                               "/f/--x--", "/g/aéb"])

(pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "cases.txt").write_text(
    "\n".join(OUT) + "\n", encoding="utf-8", newline="\n")
print(f"{sum(1 for x in OUT if x.startswith('case|'))} cases")
