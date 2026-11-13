import importlib.util
import os
import pathlib
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).with_name("campaign_readiness.py")
SPEC = importlib.util.spec_from_file_location("campaign_readiness", SCRIPT)
cr = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cr)


def row(kind="lua", started=True, errors=(), unbound=(), unsupported=(), **kw):
    return {"install": "x", "campaign": "c", "index": 0, "bzn": "m.bzn", "kind": kind, "started": started,
            "errors": list(errors), "unbound": [[u, 1] for u in unbound],
            "unsupported": [[u, 1] for u in unsupported], **kw}


class ReadinessTests(unittest.TestCase):
    def test_funnel_is_nested(self):
        rows = [
            row(),                                              # working
            row(unsupported=["Player.x = y"]),                  # bound, not working
            row(unbound=["stub global Foo"]),                   # runs, not bound
            row(errors=["boom"]),                               # fails to run
            row(started=False),                                 # does not start
            row(kind="legacy"),
            row(kind="none"),
        ]
        f = cr.funnel(rows)
        self.assertEqual(f, {"missions": 7, "lua": 5, "runs": 3, "bound": 2, "working": 1})

    def test_unsupported_call_stops_are_not_runtime_errors(self):
        gap = "runtime error: A2.SetCrew is not available: no seam\nstack traceback"
        pend = "A2.Stop: handle is still being built; use it after the next tick (handle 7)"
        rows = [
            row(errors=[gap], unsupported=["A2.SetCrew: no seam"]),                 # runs, bound, not working
            row(errors=[pend], unsupported=["A2.Stop: handle is still being built; use it after the next tick"]),
            row(errors=["A1.Foo is not available: x"]),                              # shape alone is enough
            row(errors=[gap, "attempt to call a nil value"], unsupported=["A2.SetCrew: no seam"]),  # real error too
            row(errors=["A2.Stop argument 1 must be an integer"], unsupported=["A2.Stop: handle is still being built; use it after the next tick"]),
        ]
        self.assertEqual(cr.funnel(rows), {"missions": 5, "lua": 5, "runs": 3, "bound": 3, "working": 0})
        self.assertEqual(cr.mission_status(rows[0]), "stopped at an unsupported call")
        self.assertEqual(cr.mission_status(rows[3]), "runtime error")

    def test_defaults_come_from_environment_only(self):
        self.assertEqual(set(cr.DEFAULTS), set(cr.ENV))
        self.assertNotIn("/home/", pathlib.Path(cr.__file__).read_text())

    def test_parse_rows_skips_blank_lines(self):
        self.assertEqual(len(cr.parse_rows('{"a":1}\n\n{"a":2}\n')), 2)

    def test_top_names_counts_missions_not_hits(self):
        rows = [row(unbound=["stub global A"]), row(unbound=["stub global A", "stub field B"])]
        rows[0]["unbound"][0][1] = 9
        self.assertEqual(cr.top_names(rows, "unbound"), [("stub global A", 2), ("stub field B", 1)])

    def test_legacy_status(self):
        calls = [("A", "working"), ("B", "raises"), ("C", "missing")]
        self.assertEqual(cr.legacy_status(calls), (3, 2, 1, False, False))
        self.assertEqual(cr.legacy_status([("A", "working")]), (1, 1, 1, True, True))
        self.assertEqual(cr.legacy_status([])[3:], (True, True))

    def test_legacy_counts(self):
        rows = [{"calls": [("A", "working")]}, {"calls": [("A", "raises")]}, {"calls": [("A", "missing")]}]
        self.assertEqual(cr.legacy_counts(rows), (3, 2, 1))

    def test_table_totals(self):
        t = cr.render_table([
            ("One", {"missions": 3, "lua": 0, "runs": 0, "bound": 0, "working": 0}, (3, 2, 1)),
            ("Two", {"missions": 2, "lua": 2, "runs": 1, "bound": 1, "working": 0}, None),
        ])
        last = t.splitlines()[-1]
        self.assertIn("| **all** | 5 | 2 | 1 | 1 | 0 | 3 | 2 | 1 |", last)
        self.assertIn("| Two | 2 | 2 | 1 | 1 | 0 | - | - | - |", t)

    def test_mission_status(self):
        self.assertEqual(cr.mission_status(row()), "working")
        self.assertEqual(cr.mission_status(row(errors=["e"])), "runtime error")
        self.assertEqual(cr.mission_status(row(kind="legacy")), "legacy DLL only")

    def test_locate_is_case_insensitive_and_prefers_first_dir(self):
        with tempfile.TemporaryDirectory() as t:
            a, b = pathlib.Path(t, "mod"), pathlib.Path(t, "base")
            for d in (a, b):
                (d / "Missions").mkdir(parents=True)
            (b / "Missions" / "X1S.dsl").write_text("")
            (a / "Missions" / "x1s.DSL").write_text("")
            got = cr.locate_mission_file([str(a), str(b)], "X1S.dsl")
            self.assertEqual(os.path.dirname(os.path.dirname(got)), str(a))
            self.assertIsNone(cr.locate_mission_file([str(a)], "nope.dsl"))

    def test_partner_dsl_and_mod_chain(self):
        self.assertEqual(cr.with_partners(["a_fed01R.drl"]), ["a_fed01R.drl", "a_fed01S.dsl"])
        self.assertEqual(cr.with_partners(["a_fed01r.drl"]), ["a_fed01r.drl", "a_fed01s.dsl"])
        with tempfile.TemporaryDirectory() as t:
            mods = pathlib.Path(t, "Data", "Mods")
            (mods / "Child").mkdir(parents=True)
            (mods / "Base").mkdir()
            (mods / "Child" / "info.ini").write_text('[mod]\nParentMod="Base"\n')
            got = [os.path.basename(d) for d in cr.mod_chain_dirs(t, "Child")]
            self.assertEqual(got, ["Child", "Base"])

    def test_detail_mentions_unbound_names(self):
        rows = {"a2": [row(unbound=["stub global Foo"], kind="lua")]}
        md = cr.render_detail(rows, {}, 300)
        self.assertIn("`stub global Foo`", md)


if __name__ == "__main__":
    unittest.main()
