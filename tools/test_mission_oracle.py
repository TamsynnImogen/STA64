"""Differential test of the two independent legacy mission-DLL readers.

`tools/analyze_sta2_missions.py` (a pattern-based reader of the DLLs' registration
code) and this repository's converters (`dsl_to_lua.py`, `a1_to_lua.py`, which lift the
code to Lua) find the same facts by different methods; where both are exact they must
agree. Install-gated: it reads the installs at run time and compares in memory, so
nothing derived from a game file is written anywhere or embedded here.

  STA64_TEST_A2_STOCK_INSTALL=<Armada II root>  STA64_TEST_A1_INSTALL=<Armada I root> \\
      python3 tools/test_mission_oracle.py

Armada II, every stock mission: the multiset of (rule name, action) and the initial
value of every variable (`ScriptInit`) are identical. Armada I: the analyzer's rule
names are a subset of the converter's (the analyzer drops the rules whose action it
cannot resolve; the converter finds them by their registration). Left out, because the
analyzer is known to be unreliable there: calls on fleet objects, `ScriptGet_*` and
`ScriptDestroy_*` counts, `SetSpecialForcesFlag` / `SetRelation` in a2_borg01. Needs
`objdump`; the start values also need `llvm-undname` (the analyzer types the variables
from the demangled exports). Skipped without the install or `objdump`.
"""

import collections
import importlib.util
import os
import pathlib
import re
import shutil
import sys
import unittest
import warnings

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "tools" / (name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod  # the analyzer's dataclasses look their module up
    spec.loader.exec_module(mod)
    return mod


RULE = re.compile(r"\n    name = '((?:[^'\\]|\\.)*)',\n    action = '((?:[^'\\]|\\.)*)',")
VAROP = re.compile(r"VarOp\('((?:[^'\\]|\\.)*)', 'set', (-?[0-9][0-9.e+-]*)\)")


def lua_rules(src):
    return collections.Counter(RULE.findall(src))


def init_values(src):
    """{variable: sorted values} the converted `ScriptInit` sets, following the helpers it
    calls (it usually tail-calls one that holds the whole body)."""
    funcs = {m.group(1): m.group(0) for m in re.finditer(r"^function M\.(\w+)\(.*?^end$", src, re.M | re.S)}
    out, seen, todo = {}, set(), ["ScriptInit"]
    while todo:
        name = todo.pop()
        if name in seen or name not in funcs:
            continue
        seen.add(name)
        for var, v in VAROP.findall(funcs[name]):
            out.setdefault(var, []).append(float(v))
        todo += re.findall(r"\bM\.(helper_\w+)\(", funcs[name])
    return {k: sorted(v) for k, v in out.items()}


def hers_init_values(mission):
    out = {}
    for boot in mission["bootstrap_topology"]:
        if boot["name"] != "ScriptInit":
            continue
        for cb in boot["callbacks"]:
            for s in cb["state_sequence"]:
                args = s.get("arguments") or []
                if s["name"] == "assign" and args and args[0].get("kind") == "immediate":
                    out.setdefault(s["owner"], []).append(float(args[0]["value"]))
    return {k: sorted(v) for k, v in out.items()}


def hers_rules(mission):
    out = collections.Counter()
    for r in mission["rule_topology"]:
        for a in (r["actions"][:1] or [None]):
            out[(r["name"], a)] += 1
    return out


class MissionOracle(unittest.TestCase):
    A2 = os.environ.get("STA64_TEST_A2_STOCK_INSTALL")
    A1 = os.environ.get("STA64_TEST_A1_INSTALL")

    @classmethod
    def setUpClass(cls):
        warnings.simplefilter("ignore", ResourceWarning)  # the converters leave their input files to the GC
        if not shutil.which("objdump"):
            raise unittest.SkipTest("objdump not on PATH")
        cls.ana = load("analyze_sta2_missions")
        cls.dsl = load("dsl_to_lua")
        cls.a1 = load("a1_to_lua")

    def test_armada_ii_rules_actions_and_start_values_agree(self):
        if not self.A2:
            self.skipTest("set STA64_TEST_A2_STOCK_INSTALL to an Armada II root")
        root = pathlib.Path(self.A2)
        ana, dsl = self.ana, self.dsl
        si = ana.extract_script_interface(root / "Armada2.exe", root / "armada2.map", demangle=True)
        manifest = ana.analyze_directory(root / "missions", demangle=True, script_interface=si)
        hers = {m["id"].casefold(): m for m in manifest["missions"]}
        engine = self.dsl.drl.engine_vtables(str(root), self.dsl.drl.ENGINE_CLASSES + self.dsl.OBJECT_CLASSES)
        pairs = {os.path.basename(s)[:-5].casefold(): (s, r) for s, r in dsl.find_missions(str(root))}
        self.assertEqual(set(hers), set(pairs))
        self.assertEqual(len(hers), 36)
        rules = values = 0
        for key in sorted(hers):
            mission, src, _ = dsl.convert(*pairs[key], engine)
            ours, theirs = lua_rules(src), hers_rules(hers[key])
            self.assertEqual(ours, theirs, f"{mission}: rule or rule-to-action differs")
            a, b = init_values(src), hers_init_values(hers[key])
            for var, vals in b.items():
                self.assertEqual(a.get(var), vals, f"{mission}: start value of {var}")
            # without llvm-undname the analyzer cannot type the variables: no start values
            rules += sum(theirs.values())
            values += sum(len(v) for v in b.values())
        print(f"A2: {rules} rules and {values} start values identical in {len(hers)} missions")
        self.assertGreater(rules, 1000)
        if shutil.which("llvm-undname"):
            self.assertGreater(values, 600)

    def test_armada_i_analyzer_rules_are_a_subset_of_the_converters(self):
        if not self.A1:
            self.skipTest("set STA64_TEST_A1_INSTALL to an Armada I root")
        root = pathlib.Path(self.A1)
        ana, a1 = self.ana, self.a1
        manifest = ana.analyze_directory(root / "missions", demangle=False, include_non_a2=True)
        hers = {m["id"].casefold(): m for m in manifest["missions"]}
        checked = ours_total = 0
        for map_name, dsl_path, drl_path in a1.find_missions(str(root), include_unmapped=True):
            mission, src, _ = a1.convert(dsl_path, drl_path, map_name)
            self.assertIn(mission.casefold(), hers, f"the analyzer found no {mission}")
            theirs = collections.Counter(r["name"] for r in hers[mission.casefold()]["rule_topology"])
            ours = collections.Counter(n for n, _ in RULE.findall(src))
            missing = theirs - ours
            self.assertFalse(missing, f"{mission}: the analyzer finds rules the converter does not: {dict(missing)}")
            checked += sum(theirs.values())
            ours_total += sum(ours.values())
            # where the analyzer resolved an action, the converter agrees
            pairs = dict(RULE.findall(src))
            for r in hers[mission.casefold()]["rule_topology"]:
                if r["actions"] and r["name"] in pairs:
                    self.assertEqual(pairs[r["name"]], r["actions"][0], f"{mission}: {r['name']}")
        print(f"A1: the analyzer's {checked} rules are all in the converter's {ours_total}")
        self.assertGreater(checked, 1000)
        self.assertGreater(ours_total, checked)


if __name__ == "__main__":
    unittest.main()
