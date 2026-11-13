"""Tests for tools/convert_install_missions.py (no game file is read).

  python3 tools/test_convert_install_missions.py
"""

import os
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import convert_install_missions as c  # noqa: E402


class KeyAndDir(unittest.TestCase):
    def test_key_vectors_match_sta_mission_converted(self):
        # The same vectors as crates/sta_mission/src/converted.rs.
        with tempfile.TemporaryDirectory() as t:
            for name, mod, want in [
                ("Star Trek Armada II (Stock)", None, "star-trek-armada-ii-stock"),
                ("Star Trek Armada II", "Fleet Ops 4.0", "star-trek-armada-ii+fleet-ops-4-0"),
                ("***", None, "install"),
            ]:
                d = os.path.join(t, name)
                os.makedirs(d)
                self.assertEqual(c.key(d, mod), want)

    def test_data_dir_vectors_match_platform(self):
        self.assertEqual(c.data_dir({"STA64_DATA_DIR": "/d", "HOME": "/h"}, "linux"), "/d")
        self.assertEqual(c.data_dir({"XDG_DATA_HOME": "/x", "HOME": "/h"}, "linux"), "/x/sta64")
        self.assertEqual(c.data_dir({"HOME": "/h"}, "linux"), "/h/.local/share/sta64")
        self.assertEqual(c.data_dir({"LOCALAPPDATA": "/l"}, "win32"), "/l/sta64")
        self.assertEqual(c.data_dir({"HOME": "/h"}, "darwin"), "/h/Library/Application Support/sta64")
        self.assertIsNone(c.data_dir({}, "linux"))

    def test_refuses_an_output_inside_the_install(self):
        with tempfile.TemporaryDirectory() as t:
            with self.assertRaises(SystemExit) as e:
                c.main(["--install", t, "--out", os.path.join(t, "out")])
            self.assertIn("inside the install", str(e.exception))


if __name__ == "__main__":
    unittest.main()
