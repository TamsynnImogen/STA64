#!/usr/bin/env python3
"""Offline tests for refit_side.py (synthetic data only): python3 tools/pair/test_refit_side.py"""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import refit_side as rs  # noqa: E402

PAIR = """\
; comment
[Scene]
Fog=0
Camera=orbit 1 2 3

[Marks]
Ordnance=1

[Pair]
Games=a2,roots
Duration=12
Frames=3,8
At=1 goto ship 2000 0 3200
At=1 goto turner 3200 0 2000
At=5 weapons ship on
a2.Race=federation
a2.Launch=-nointro a2_borg01
roots.Race=unitedfederation
roots.Size=1280x960
roots.Launch=-nointro -mod "STA2 Classic Mod" fulltech.bzn

[Object.ship]
Odf=fgalaxy
roots.Odf=fgalaxy2
a2.Heading=90
X=1
"""


class Ini(unittest.TestCase):
    def test_repeated_at_kept_in_order(self):
        p = rs.load_pair(PAIR, "a2")
        self.assertEqual(p.at, [(1.0, "goto ship 2000 0 3200"), (1.0, "goto turner 3200 0 2000"), (5.0, "weapons ship on")])
        self.assertEqual(p.frames, [3.0, 8.0])
        self.assertEqual(p.duration, 12.0)

    def test_game_overrides(self):
        a = rs.load_pair(PAIR, "a2")
        r = rs.load_pair(PAIR, "roots")
        self.assertEqual(a.size, (1280, 720))
        self.assertEqual(r.size, (1280, 960))
        self.assertEqual(a.race, "federation")
        self.assertEqual(r.race, "unitedfederation")
        self.assertEqual(a.launch, ["-nointro", "a2_borg01"])
        self.assertEqual(r.launch, ["-nointro", "-mod", "STA2 Classic Mod", "fulltech.bzn"])
        ra = rs.render_scene_ini(a)
        rr = rs.render_scene_ini(r)
        self.assertIn("Odf=fgalaxy\n", ra)
        self.assertIn("Heading=90", ra)
        self.assertIn("Odf=fgalaxy2", rr)
        self.assertNotIn("Odf=fgalaxy\n", rr)
        self.assertNotIn("Heading", rr)
        self.assertNotIn("roots.", ra)
        self.assertNotIn("a2.", rr)

    def test_render_scene_ini(self):
        ini = rs.render_scene_ini(rs.load_pair(PAIR, "a2"))
        self.assertNotIn("[Pair]", ini)
        self.assertNotIn("At=", ini)
        self.assertIn("[Scene]\nFog=0\nCamera=orbit 1 2 3\nRace=federation\n", ini)
        self.assertIn("[Marks]\nOrdnance=1\nSimTime=1\nState=250\n", ini)
        self.assertLess(ini.index("[Marks]"), ini.index("[Object.ship]"))

    def test_simtime_forced_and_state_kept(self):
        txt = "[Scene]\nFog=0\n[Marks]\nSimTime=0\nState=100\n[Pair]\nGames=a2\nDuration=10\n"
        ini = rs.render_scene_ini(rs.load_pair(txt, "a2"))
        self.assertIn("SimTime=1", ini)
        self.assertNotIn("SimTime=0", ini)
        self.assertIn("State=100", ini)
        self.assertNotIn("State=250", ini)

    def test_missing_marks_section_added(self):
        ini = rs.render_scene_ini(rs.load_pair("[Scene]\nFog=0\n[Object.x]\nOdf=a\n[Pair]\nGames=a2\nDuration=10\n", "a2"))
        self.assertIn("[Marks]\nSimTime=1\nState=250\n", ini)

    def test_refusals(self):
        with self.assertRaisesRegex(rs.PairError, "Armada I"):
            rs.load_pair(PAIR, "a1")
        with self.assertRaisesRegex(rs.PairError, "does not list"):
            rs.load_pair("[Pair]\nGames=a2\nDuration=10\n", "roots")


class Seconds(unittest.TestCase):
    def test_suffix_and_required_duration(self):
        p = rs.load_pair("[Pair]\nGames=a2\nDuration=12s\nFrames=2,5s\nAt=1.5s stop x\nroots.At=9 stop y\n", "a2")
        self.assertEqual((p.duration, p.frames, p.at), (12.0, [2.0, 5.0], [(1.5, "stop x")]))
        r = rs.load_pair("[Pair]\nGames=a2,roots\nDuration=3\nAt=1 stop x\nroots.At=9 stop y\n", "roots")
        self.assertEqual(r.at, [(1.0, "stop x")])  # a prefixed At= is no override
        with self.assertRaises(rs.PairError):
            rs.load_pair("[Pair]\nGames=a2\n", "a2")
        with self.assertRaises(rs.PairError):
            rs.load_pair("[Pair]\nGames=a2\nDuration=x\n", "a2")


class Marks(unittest.TestCase):
    TXT = """\
epoch 0x01dd72e7 0x8c5e6c41 1000
1000 ready t=0.7 objects=2
1010 state t=0.7 obj=a pos=1.0,0.0,2.0 hull=5.0
1100 cmd t=1.03 text=goto ship 2000 0 3200
1200 cmd t=1.50 text=pause
1300 cmd t=2.50 text=pause
1400 state t=2.75 obj=a
"""

    def test_parse(self):
        ms = rs.parse_marks(self.TXT)
        self.assertEqual(len(ms), 6)
        self.assertEqual(rs.ready_t(ms), 0.7)
        self.assertEqual(rs.latest_t(ms), 2.75)
        c = ms[2]
        self.assertEqual((c["kind"], c["t"], c["text"]), ("cmd", 1.03, "goto ship 2000 0 3200"))
        self.assertEqual(ms[1]["f"]["pos"], "1.0,0.0,2.0")

    def test_match_first_unused(self):
        ms = rs.parse_marks(self.TXT)
        got = rs.match_commands(ms, ["goto ship 2000 0 3200", "pause", "pause", "pause", "nothing"])
        self.assertEqual(got, [1.03, 1.5, 2.5, None, None])
        self.assertEqual(rs.rel(1.03, 0.7), 0.33)
        self.assertIsNone(rs.rel(None, 0.7))

    def test_no_ready(self):
        self.assertIsNone(rs.ready_t(rs.parse_marks("5 state t=1.0\n")))


class Guards(unittest.TestCase):
    def test_out_in_checkout_refused(self):
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, ".git"))
            os.makedirs(os.path.join(d, "bench", "pairs"))
            inside = os.path.join(d, "x", "out")
            self.assertEqual(rs.checkout_containing(inside), os.path.realpath(d))
            pf = os.path.join(d, "bench", "pairs", "p.ini")
            open(pf, "w").write("[Pair]\nGames=a2\nDuration=10\n")
            self.assertEqual(rs.main(["--pair", pf, "--game", "a1", "--out", inside]), 2)
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, ".git"))  # git but no bench/pairs
            self.assertIsNone(rs.checkout_containing(os.path.join(d, "out")))

    def test_main_refuses_checkout_out(self):
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, ".git"))
            os.makedirs(os.path.join(d, "bench", "pairs"))
            pf = os.path.join(d, "bench", "pairs", "p.ini")
            open(pf, "w").write("[Pair]\nGames=a2\nDuration=10\n")
            self.assertEqual(rs.main(["--pair", pf, "--game", "a2", "--out", os.path.join(d, "o"), "--dry-run"]), 2)
            self.assertFalse(os.path.exists(os.path.join(d, "o")))

    def test_dry_run(self):
        with tempfile.TemporaryDirectory() as d:
            pf = os.path.join(d, "p.ini")
            open(pf, "w").write(PAIR)
            self.assertEqual(rs.main(["--pair", pf, "--game", "roots", "--out", os.path.join(d, "o"), "--dry-run"]), 0)


if __name__ == "__main__":
    unittest.main()
