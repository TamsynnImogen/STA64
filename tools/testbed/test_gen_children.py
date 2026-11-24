"""Tests for tools/testbed/gen_children.py (TOOL-20/23).

Run: python3 -m unittest discover -s tools/testbed -p 'test_*.py'
"""
import importlib.util
import os
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("gen_children", HERE / "gen_children.py")
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)


class GroupTests(unittest.TestCase):
    def test_every_switch_in_exactly_one_group(self):
        self.assertEqual(gen.check(gen.feature_names()), [])

    def test_unknown_and_missing_switches_are_reported(self):
        names = gen.feature_names() + ["brand_new_switch"]
        problems = gen.check(names)
        self.assertTrue(any("brand_new_switch: in no group" in p for p in problems))

    def test_child_ini_sets_every_switch(self):
        names = gen.feature_names()
        with tempfile.TemporaryDirectory() as root:
            base = pathlib.Path(root, "mods", "STA64Testbed")
            base.mkdir(parents=True)
            (base / "manifest.toml").write_text('[mod]\nversion = "1.2.3"\n')
            name = gen.write(root, "Economy", set(gen.GROUPS["Economy"]) | gen.ALWAYS_ON, names, "1.2.3")
            ini = pathlib.Path(root, "mods", name, "info.ini").read_text()
            lines = dict(l.split("=", 1) for l in ini.splitlines() if "=" in l)
            for n in names:
                self.assertIn(n, lines)
            self.assertEqual(lines["extra_resources"], "1")
            self.assertEqual(lines["cheats"], "1")
            self.assertEqual(lines["a1_compat"], "0")
            self.assertEqual(lines["fire_arcs"], "0")
            self.assertEqual(lines["ParentMod"], "STA64Testbed")


if __name__ == "__main__":
    unittest.main()
