#!/usr/bin/env python3
"""Tests of the pure parts of tools/mission_playthrough.py (no build, no install)."""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mission_playthrough as mp  # noqa: E402


def ev(t, kind, what):
    return {"t": t, "tick": int(t * 30), "kind": kind, "what": what}


TIMELINE = [
    ev(0.0, "present", "Cineractive { on: true }"),
    ev(0.5, "present", "Music { name: \"intro\" }"),
    ev(3.0, "present", "Title { id: 1 }"),
    ev(9.0, "present", "Cineractive { on: false }"),
    ev(10.0, "objective", "issued #1 Pending"),
]


class Reference(unittest.TestCase):
    def test_parse(self):
        e, tol, strict = mp.parse_reference("# x\ntolerance 1.5\nstrict present\n0 present Cineractive\n+3 present Title\n* end won\n")
        self.assertEqual(tol, 1.5)
        self.assertEqual(strict, {"present"})
        self.assertEqual([(x["t"], x["rel"], x["kind"], x["text"]) for x in e],
                         [(0.0, False, "present", "Cineractive"), (3.0, True, "present", "Title"), (None, False, "end", "won")])

    def test_exact_sequence_has_no_difference(self):
        e, tol, _ = mp.parse_reference("0 present on: true\n+0.5 present Music\n3 present Title\n* present on: false\n10 objective issued")
        self.assertEqual(mp.compare(TIMELINE, e, tol), [])

    def test_missing_order_and_timing(self):
        e, tol, _ = mp.parse_reference("0 present Title\n0 present Music\n* present Fade\n5 objective issued")
        d = mp.compare(TIMELINE, e, tol)
        self.assertTrue(d[0].startswith("timing:"), d)
        self.assertTrue(d[1].startswith("out of order:"), d)
        self.assertTrue(d[2].startswith("missing:"), d)
        self.assertTrue(d[3].startswith("timing:"), d)

    def test_strict_lists_extras_up_to_the_last_match(self):
        e, tol, strict = mp.parse_reference("strict present\n* present on: true\n* present on: false")
        d = mp.compare(TIMELINE, e, tol, strict)
        self.assertEqual(len(d), 2)
        self.assertTrue(all(x.startswith("extra:") for x in d))

    def test_summary(self):
        s = mp.summary(TIMELINE + [ev(11, "unbound", "Foo"), ev(12, "end", "won")])
        self.assertIn("end: won", s)
        self.assertIn("unbound (1): Foo", s)


if __name__ == "__main__":
    unittest.main()
