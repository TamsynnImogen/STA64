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
                ("@@@", None, "install"),
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
            os.makedirs(os.path.join(t, "missions"))
            open(os.path.join(t, "Armada2.exe"), "w").close()
            with self.assertRaises(SystemExit) as e:
                c.main(["--install", t, "--out", os.path.join(t, "out")])
            self.assertIn("inside the install", str(e.exception))

    def test_a_folder_that_is_neither_game_is_named_as_such(self):
        with tempfile.TemporaryDirectory() as t:
            with self.assertRaises(SystemExit) as e:
                c.main(["--install", t])
            self.assertIn("neither an Armada I root", str(e.exception))

    def test_armada1_is_told_from_armada2_by_the_exe(self):
        with tempfile.TemporaryDirectory() as t:
            open(os.path.join(t, "Armada.exe"), "w").close()
            self.assertTrue(c.is_armada1(t))
            open(os.path.join(t, "Armada2.exe"), "w").close()
            self.assertFalse(c.is_armada1(t))


class ReferenceScript(unittest.TestCase):
    def test_refuses_the_repository_its_build_folders_and_an_install(self):
        import convert_reference as r
        repo = str(pathlib.Path(__file__).resolve().parents[1])
        self.assertIn("repository", r.refuse(os.path.join(repo, "out"), []))
        self.assertIn("repository", r.refuse(os.path.join(repo, "target", "scratch", "ref"), []))
        with tempfile.TemporaryDirectory() as t:
            self.assertIn("install", r.refuse(os.path.join(t, "out"), [t]))
            with self.assertRaises(SystemExit) as e:
                r.main(["--install", t, "--out", os.path.join(repo, "target", "x")])
            self.assertIn("inside the repository", str(e.exception))

    def test_refuses_the_folder_derived_from_the_install_name(self):
        # `--out` is the parent of an install named like its own slug: the per-install
        # folder would be the install itself. Nothing may be written into it.
        import convert_reference as r
        with tempfile.TemporaryDirectory() as t:
            inst = os.path.join(t, "armada")
            os.makedirs(inst)
            open(os.path.join(inst, "Armada.exe"), "w").close()
            with self.assertRaises(SystemExit):
                r.main(["--install", inst, "--out", t])
            self.assertEqual(os.listdir(inst), ["Armada.exe"])

    def test_missing_install_creates_nothing_and_equal_slugs_stay_apart(self):
        import convert_reference as r
        with tempfile.TemporaryDirectory() as t:
            out = os.path.join(t, "out")
            with self.assertRaises(SystemExit) as e:
                r.main(["--install", os.path.join(t, "nope"), "--out", out])
            self.assertIn("not a folder", str(e.exception))
            self.assertFalse(os.path.exists(out))
        self.assertEqual(r.subfolders(["/x/a/Armada", "/x/b/armada", "/x/c/other"]), ["armada", "armada-2", "other"])


if __name__ == "__main__":
    unittest.main()
