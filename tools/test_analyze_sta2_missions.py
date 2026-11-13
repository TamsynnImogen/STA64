from __future__ import annotations

import struct
import tempfile
import unittest
from pathlib import Path

from tools.analyze_sta2_missions import (
    PeFormatError,
    PeImage,
    _call_arguments,
    classify_exports,
    extract_action_topology,
    extract_rule_topology,
    extract_script_interface,
    extract_references,
    parse_map_symbols,
    pair_missions,
)


def synthetic_pe32() -> bytes:
    """Build a tiny metadata-only PE image without using proprietary fixtures."""
    data = bytearray(0x700)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", data, 0x84, 0x14C, 1, 946684800, 0, 0, 0xE0, 0x210E)

    optional = 0x98
    struct.pack_into("<H", data, optional, 0x10B)
    struct.pack_into("<I", data, optional + 28, 0x10000000)
    struct.pack_into("<H", data, optional + 68, 2)
    struct.pack_into("<I", data, optional + 92, 16)
    struct.pack_into("<II", data, optional + 96, 0x1100, 0xA0)
    struct.pack_into("<II", data, optional + 104, 0x1200, 0x80)

    section = optional + 0xE0
    struct.pack_into(
        "<8sIIIIIIHHI",
        data,
        section,
        b".rdata\0\0",
        0x500,
        0x1000,
        0x500,
        0x200,
        0,
        0,
        0,
        0,
        0x40000040,
    )

    def offset(rva: int) -> int:
        return 0x200 + rva - 0x1000

    struct.pack_into(
        "<IIHHIIIIIII",
        data,
        offset(0x1100),
        0,
        946684800,
        0,
        0,
        0x1180,
        1,
        1,
        1,
        0x1140,
        0x1144,
        0x1148,
    )
    struct.pack_into("<I", data, offset(0x1140), 0x1300)
    struct.pack_into("<I", data, offset(0x1144), 0x1190)
    struct.pack_into("<H", data, offset(0x1148), 0)
    data[offset(0x1180) : offset(0x1180) + 12] = b"fixture.dsl\0"
    export_name = b"?Mission_Win@@YAXXZ\0"
    data[offset(0x1190) : offset(0x1190) + len(export_name)] = export_name

    struct.pack_into("<IIIII", data, offset(0x1200), 0x1250, 0, 0, 0x1230, 0x1250)
    data[offset(0x1230) : offset(0x1230) + 13] = b"KERNEL32.dll\0"
    struct.pack_into("<II", data, offset(0x1250), 0x1260, 0)
    struct.pack_into("<H", data, offset(0x1260), 7)
    data[offset(0x1262) : offset(0x1262) + 13] = b"GetTickCount\0"
    data[0x680:0x6A6] = b"a2_fixture_A.txt\0WORLD_Start_Mission\0"
    return bytes(data)


class PeImageTests(unittest.TestCase):
    def test_parses_pe_headers_exports_and_imports(self) -> None:
        image = PeImage(synthetic_pe32())
        self.assertEqual(image.bits, 32)
        self.assertEqual(image.image_base, 0x10000000)
        self.assertEqual(image.sections[0].name, ".rdata")
        self.assertEqual(image.exports()[0]["name"], "?Mission_Win@@YAXXZ")
        self.assertEqual(image.exports()[0]["ordinal"], 1)
        self.assertEqual(image.imports()[0]["library"], "KERNEL32.dll")
        self.assertEqual(image.imports()[0]["symbols"][0]["name"], "GetTickCount")
        self.assertEqual(image.imports()[0]["symbols"][0]["iat_rva"], 0x1250)

    def test_rejects_non_pe_input(self) -> None:
        with self.assertRaises(PeFormatError):
            PeImage(b"not a PE image")

    def test_extracts_mission_references(self) -> None:
        references = extract_references(synthetic_pe32())
        self.assertIn("a2_fixture_A.txt", references["txt"])
        self.assertIn("KERNEL32.dll", references["dll"])

    def test_detects_script_interface_version(self) -> None:
        data = bytearray(synthetic_pe32())
        implementation_offset = 0x200 + 0x1300 - 0x1000
        data[implementation_offset : implementation_offset + 8] = (
            b"\x55\x8b\xec\x83\x7d\x0c\x33\x74"
        )
        image = PeImage(bytes(data))
        interface = image.script_interface(
            [{"name": "ScriptExtendInterface", "ordinal": 1, "rva": 0x1300}]
        )
        self.assertIsNotNone(interface)
        self.assertEqual(interface["version"], 51)

    def test_classifies_decorated_action(self) -> None:
        classified = classify_exports(
            [{"name": "?Mission_Win@@YAXXZ", "ordinal": 1, "rva": 0x1300}]
        )
        self.assertEqual(classified["actions"], ["Mission_Win"])

    def test_extracts_structural_rule_topology(self) -> None:
        data = bytearray(synthetic_pe32())
        section_header = 0x98 + 0xE0
        data[section_header : section_header + 8] = b".text\0\0\0"
        struct.pack_into("<I", data, section_header + 36, 0x60000020)

        def offset(rva: int) -> int:
            return 0x200 + rva - 0x1000

        data[offset(0x1230) : offset(0x1230) + 13] = b"fixture.dsl\0\0"
        struct.pack_into(
            "<BIBIBI",
            data,
            offset(0x1320),
            0x68,
            0x100013A0,
            0x68,
            0x10001340,
            0x68,
            0x10001360,
        )
        action = b"\xff\x15\x50\x12\x00\x10\xc3" + b"\xcc" * 8
        condition = b"\xff\x15\x50\x12\x00\x10\xc3" + b"\xcc" * 8
        data[offset(0x1340) : offset(0x1340) + len(action)] = action
        data[offset(0x1360) : offset(0x1360) + len(condition)] = condition
        data[offset(0x13A0) : offset(0x13A0) + 12] = b"ExampleRule\0"

        image = PeImage(bytes(data))
        imports = image.imports()
        topology = extract_rule_topology(
            image,
            imports,
            "fixture.dsl",
            {
                "bootstrap": [],
                "actions": ["GetTickCount"],
                "state": [],
                "other": [],
            },
        )
        self.assertEqual(len(topology), 1)
        self.assertEqual(topology[0]["name"], "ExampleRule")
        self.assertEqual(topology[0]["actions"], ["GetTickCount"])
        self.assertEqual(topology[0]["condition_actions"], ["GetTickCount"])

    def test_extracts_dsl_action_topology(self) -> None:
        data = bytearray(synthetic_pe32())
        section_header = 0x98 + 0xE0
        data[section_header : section_header + 8] = b".text\0\0\0"
        struct.pack_into("<I", data, section_header + 36, 0x60000020)

        def offset(rva: int) -> int:
            return 0x200 + rva - 0x1000

        action = (
            b"\xa1\x80\x13\x00\x10"
            b"\x8b\x08"
            b"\xff\x51\x10"
            b"\x68\xa0\x13\x00\x10"
            b"\xc3"
            + b"\xcc" * 8
        )
        data[offset(0x1300) : offset(0x1300) + len(action)] = action
        data[offset(0x13A0) : offset(0x13A0) + 12] = b"hello.wav\0\0\0"
        image = PeImage(bytes(data))
        exports = [
            {"name": "?Mission_Win@@YAXXZ", "ordinal": 1, "rva": 0x1300},
            {
                "name": "?g_pSI@@3PAVScriptInterface@@A",
                "ordinal": 2,
                "rva": 0x1380,
            },
        ]
        topology = extract_action_topology(
            image,
            exports,
            {
                "bootstrap": [],
                "actions": ["Mission_Win"],
                "state": ["g_pSI"],
                "other": [],
            },
            {4: {"slot": 4, "name": "TestCall"}},
        )
        self.assertEqual(len(topology), 1)
        self.assertEqual(topology[0]["name"], "Mission_Win")
        self.assertEqual(topology[0]["referenced_state"], ["g_pSI"])
        self.assertEqual(topology[0]["script_interface_slots"], [4])
        self.assertEqual(topology[0]["script_interface_calls"][0]["name"], "TestCall")
        self.assertEqual(topology[0]["strings"], ["hello.wav"])

    def test_follows_direct_export_helper_and_tracks_return_store(self) -> None:
        data = bytearray(synthetic_pe32())
        section_header = 0x98 + 0xE0
        data[section_header : section_header + 8] = b".text\0\0\0"
        struct.pack_into("<I", data, section_header + 36, 0x60000020)

        def offset(rva: int) -> int:
            return 0x200 + rva - 0x1000

        entry = b"\xe8\x3b\x00\x00\x00\xc3" + b"\xcc" * 8
        helper = (
            b"\x8b\xf4"
            b"\x68\xa0\x13\x00\x10"
            b"\xa1\x80\x13\x00\x10"
            b"\x8b\x08"
            b"\xff\x51\x10"
            b"\xa3\x90\x13\x00\x10"
            b"\xc3"
            + b"\xcc" * 8
        )
        data[offset(0x1300) : offset(0x1300) + len(entry)] = entry
        data[offset(0x1340) : offset(0x1340) + len(helper)] = helper
        data[offset(0x13A0) : offset(0x13A0) + 11] = b"Enterprise\0"
        image = PeImage(bytes(data))
        exports = [
            {"name": "?ScriptInit@@YAXXZ", "ordinal": 1, "rva": 0x1300},
            {
                "name": "?g_pSI@@3PAVScriptInterface@@A",
                "demangled": "class ScriptInterface *g_pSI",
                "ordinal": 2,
                "rva": 0x1380,
            },
            {
                "name": "?Enterprise@@3HA",
                "demangled": "int Enterprise",
                "ordinal": 3,
                "rva": 0x1390,
            },
        ]
        topology = extract_action_topology(
            image,
            exports,
            {
                "bootstrap": ["ScriptInit"],
                "actions": ["ScriptInit"],
                "state": ["g_pSI"],
                "other": ["Enterprise"],
            },
            {
                4: {
                    "slot": 4,
                    "name": "GetHandle",
                    "prototype": "public: virtual int __thiscall ScriptInterfaceImp::GetHandle(char *)",
                }
            },
            follow_direct_helpers=True,
        )
        self.assertEqual(topology[0]["helper_rvas"], [0x1340])
        call = topology[0]["callbacks"][0]["script_interface_sequence"][0]
        self.assertTrue(call["arguments_exact"])
        self.assertEqual(call["arguments"][0]["value"], "Enterprise")
        self.assertEqual(call["result_symbol"], "Enterprise")

    def test_recovers_x87_call_result_stack_argument(self) -> None:
        image = PeImage(synthetic_pe32())
        code = b"\x8b\xf4\x6a\x00\x51\xd9\x1c\x24"
        arguments, exact = _call_arguments(
            image,
            code,
            len(code),
            "void SucceedMission(float, char *)",
            {},
        )
        self.assertTrue(exact)
        self.assertEqual(arguments[0]["kind"], "x87_result")
        self.assertEqual(arguments[1]["value"], 0)

    def test_resolves_script_interface_vtable_from_map(self) -> None:
        data = bytearray(synthetic_pe32())
        section_header = 0x98 + 0xE0
        data[section_header : section_header + 8] = b".text\0\0\0"
        struct.pack_into("<I", data, section_header + 36, 0x60000020)

        def offset(rva: int) -> int:
            return 0x200 + rva - 0x1000

        struct.pack_into("<II", data, offset(0x1400), 0x10001300, 0)
        map_text = (
            " 0001:00000300 ?CountUnits@ScriptInterfaceImp@@UAEHHPAD@Z 10001300 f i x.obj\n"
            " 0002:00000400 ??_7ScriptInterfaceImp@@6B@ 10001400 x.obj\n"
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "Armada2.exe"
            map_path = root / "armada2.map"
            executable.write_bytes(data)
            map_path.write_text(map_text, encoding="utf-8")
            interface = extract_script_interface(executable, map_path, demangle=False)
        self.assertEqual(interface["method_count"], 1)
        self.assertEqual(interface["methods"][0]["name"], "CountUnits")

    def test_parses_linker_map_symbols(self) -> None:
        by_address, by_name = parse_map_symbols(
            " 0002:00001380 ??_7ScriptInterfaceImp@@6B@ 006af380 module.obj\n"
        )
        self.assertEqual(by_name["??_7ScriptInterfaceImp@@6B@"], 0x006AF380)
        self.assertEqual(by_address[0x006AF380], ["??_7ScriptInterfaceImp@@6B@"])


class MissionPairTests(unittest.TestCase):
    def test_pairs_case_insensitive_tutorial_names(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "a2_tutorial2s.dsl").touch()
            (root / "a2_tutorial2R.drl").touch()
            pairs, warnings = pair_missions(root)
        self.assertEqual(warnings, [])
        self.assertEqual(len(pairs), 1)
        self.assertTrue(pairs[0].complete)
        self.assertEqual(pairs[0].mission_id, "a2_tutorial2")


if __name__ == "__main__":
    unittest.main()
