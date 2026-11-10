import hashlib
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("stage_a2_ports.py")
SPEC = importlib.util.spec_from_file_location("stage_a2_ports", SCRIPT)
stager = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stager)


class StageA2PortsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.parent = self.root / "outputs"
        self.parent.mkdir()
        self.out = self.parent / "staged"

    def tearDown(self):
        self.temp.cleanup()

    def write_port(self, name, data):
        path = self.source / name
        path.write_bytes(data)
        return path

    @staticmethod
    def port_bytes(map_text, body=b"print('synthetic fixture')\n"):
        return b"-- Synthetic mission\r\n-- Map: " + map_text + b"\r\n" + body

    def test_stages_identical_bytes_aliases_hashes_and_content_free_manifest(self):
        data = self.port_bytes(b"(A2_Sample_Map.BZN.); repeated as a2_sample_map.bzn")
        self.write_port("Mission.lua", data)
        nested = self.source / "nested"
        nested.mkdir()
        (nested / "ignored.lua").write_bytes(self.port_bytes(b"a2_nested.bzn"))
        (self.source / "readme.txt").write_text("not a Lua input", encoding="utf-8")

        manifest = stager.stage(self.source, self.out)

        original = "missions/Mission.lua"
        alias = "missions/a2_sample_map.lua"
        for relative in (original, alias):
            self.assertEqual((self.out / relative).read_bytes(), data)
        entry = manifest["entries"][0]
        self.assertEqual(entry["source"], "Mission.lua")
        self.assertEqual(entry["map"], "A2_Sample_Map.BZN")
        self.assertEqual(entry["bytes"], len(data))
        self.assertEqual(entry["sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual([item["path"] for item in entry["outputs"]], [original, alias])
        self.assertTrue(all(item["sha256"] == hashlib.sha256(data).hexdigest() for item in entry["outputs"]))
        self.assertTrue(manifest["complete"])
        manifest_bytes = (self.out / "manifest.json").read_bytes()
        self.assertEqual(json.loads(manifest_bytes), manifest)
        self.assertNotIn(b"synthetic fixture", manifest_bytes)
        self.assertFalse((self.out / "missions/ignored.lua").exists())
        self.assertEqual((self.source / "Mission.lua").read_bytes(), data)

    def test_body_bytes_are_not_decoded_as_header_text(self):
        data = b"-- a2_bytes.bzn.\n" + b"-- padding\n" * 7 + b"local raw='\xff'\n"
        self.write_port("Bytes.lua", data)
        manifest = stager.stage(self.source, self.out)
        for output in manifest["entries"][0]["outputs"]:
            self.assertEqual((self.out / output["path"]).read_bytes(), data)

    def test_source_symlink_is_rejected_before_output_creation(self):
        outside = self.root / "outside.lua"
        data = self.port_bytes(b"a2_outside.bzn.")
        outside.write_bytes(data)
        (self.source / "Linked.lua").symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "symlink"):
            stager.stage(self.source, self.out)
        self.assertFalse(self.out.exists())
        self.assertEqual(outside.read_bytes(), data)

    def test_casefold_equivalent_alias_is_written_once(self):
        data = self.port_bytes(b"a2_DEMO.bzn.")
        self.write_port("A2_demo.lua", data)

        manifest = stager.stage(self.source, self.out)

        self.assertEqual([item["path"] for item in manifest["entries"][0]["outputs"]], ["missions/A2_demo.lua"])
        self.assertEqual((self.out / "missions/A2_demo.lua").read_bytes(), data)

    def test_header_requires_one_distinct_first_eight_line_map_token(self):
        cases = {
            "missing": b"-- No map token in this header\n",
            "empty": b"",
            "ambiguous": b"-- a2_alpha.bzn and A2_beta.BZN\n",
            "backup": b"-- map a2_alpha.bzn.bak\n",
            "ninth-line-only": b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\na2_alpha.bzn\n",
        }
        for label, data in cases.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temp:
                root = pathlib.Path(temp)
                source = root / "source"
                source.mkdir()
                (source / "bad.lua").write_bytes(data)
                out = root / "out"
                with self.assertRaises(ValueError):
                    stager.stage(source, out)
                self.assertFalse(out.exists())

    def test_duplicate_maps_and_casefold_output_collisions_fail_before_creation(self):
        cases = (
            (("one.lua", b"-- a2_same.bzn\n"), ("two.lua", b"-- A2_SAME.BZN\n")),
            (("a2_alpha.lua", b"-- a2_beta.bzn\n"), ("other.lua", b"-- a2_alpha.bzn\n")),
            (("Mission.lua", b"-- a2_one.bzn\n"), ("mission.lua", b"-- a2_two.bzn\n")),
        )
        for ports in cases:
            with self.subTest(names=tuple(name for name, _ in ports)), tempfile.TemporaryDirectory() as temp:
                root = pathlib.Path(temp)
                source = root / "source"
                source.mkdir()
                for name, header in ports:
                    (source / name).write_bytes(header + b"print('x')\n")
                out = root / "out"
                with self.assertRaises(ValueError):
                    stager.stage(source, out)
                self.assertFalse(out.exists())

    def test_all_inputs_are_validated_before_destination_creation(self):
        self.write_port("good.lua", self.port_bytes(b"a2_good.bzn"))
        self.write_port("bad.lua", b"-- missing map token\nprint('x')\n")

        with self.assertRaises(ValueError):
            stager.stage(self.source, self.out)

        self.assertFalse(self.out.exists())

    def test_existing_directory_empty_directory_and_symlink_are_never_overwritten(self):
        self.write_port("mission.lua", self.port_bytes(b"a2_mission.bzn"))

        self.out.mkdir()
        marker = self.out / "keep.txt"
        marker.write_text("preserve", encoding="utf-8")
        with self.assertRaises(FileExistsError):
            stager.stage(self.source, self.out)
        self.assertEqual(marker.read_text(encoding="utf-8"), "preserve")

        empty = self.parent / "empty"
        empty.mkdir()
        with self.assertRaises(FileExistsError):
            stager.stage(self.source, empty)
        self.assertEqual(list(empty.iterdir()), [])

        target = self.parent / "target"
        target.mkdir()
        link = self.parent / "link"
        link.symlink_to(target, target_is_directory=True)
        with self.assertRaises(FileExistsError):
            stager.stage(self.source, link)
        self.assertTrue(link.is_symlink())
        self.assertEqual(list(target.iterdir()), [])

    def test_output_inside_source_and_missing_parent_are_rejected(self):
        self.write_port("mission.lua", self.port_bytes(b"a2_mission.bzn"))
        with self.assertRaises(ValueError):
            stager.stage(self.source, self.source / "out")
        with self.assertRaises(FileNotFoundError):
            stager.stage(self.source, self.root / "missing" / "out")
        self.assertEqual(sorted(p.name for p in self.source.iterdir()), ["mission.lua"])

    def test_source_names_reject_path_separators(self):
        with self.assertRaises(ValueError):
            stager._validate_source_name("nested/mission.lua")
        with self.assertRaises(ValueError):
            stager._validate_source_name("nested\\mission.lua")

    def test_cli_stages_synthetic_input(self):
        data = self.port_bytes(b"a2_cli.bzn")
        self.write_port("cli.lua", data)
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--ports", str(self.source), "--out", str(self.out)],
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Staged 1 Lua ports as 2 files", result.stdout)
        self.assertEqual((self.out / "missions/cli.lua").read_bytes(), data)
        self.assertTrue((self.out / "manifest.json").is_file())


if __name__ == "__main__":
    unittest.main()
