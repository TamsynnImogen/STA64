from __future__ import annotations

import unittest
import tempfile
from pathlib import Path

from tools.convert_sta2_missions_to_mmm import (
    RELATION_NAMES,
    call_hint,
    convert_manifest,
    lua_identifier,
    lua_quote,
    parse_bzn_title,
    parse_localized_strings,
    resolve_bzn_metadata,
)


def fixture_manifest() -> dict[str, object]:
    return {
        "schema_version": 2,
        "format": "sta2-compiled-mission-manifest",
        "missions": [
            {
                "id": "a2_test01",
                "action_topology": [
                    {
                        "name": "Mission_Win",
                        "strings": ["victory.wav"],
                        "script_interface_calls": [
                            {"slot": 265, "name": "SucceedMission"},
                            {"slot": 310, "name": "AudioMessage"},
                        ],
                    }
                ],
                "rule_topology": [
                    {
                        "name": "VictoryRule",
                        "actions": ["Mission_Win"],
                        "condition_symbols": ["p_VICTORY", "g_pSI"],
                        "condition_strings": [],
                        "script_interface_calls": [
                            {"slot": 79, "name": "IsAlive"}
                        ],
                    }
                ],
            }
        ],
    }


def pilot_fixture_manifest() -> dict[str, object]:
    def call(name: str) -> dict[str, object]:
        return {
            "name": name,
            "code_offset": 10,
            "arguments": [],
            "arguments_exact": True,
        }

    run_callbacks: list[dict[str, object]] = [
        {
            "rva": 100,
            "script_interface_sequence": [call("DisableSave")],
            "state_sequence": [
                {
                    "owner": "p_INTRO",
                    "name": "assign",
                    "code_offset": 5,
                    "arguments": [{"kind": "immediate", "value": 0}],
                    "arguments_exact": True,
                },
                {
                    "owner": "p_tRemoveAuthorization",
                    "name": "start",
                    "code_offset": 20,
                    "arguments": [{"kind": "float", "value": 17.0}],
                    "arguments_exact": True,
                },
            ],
        }
    ]
    for rva in range(101, 115):
        run_callbacks.append(
            {
                "rva": rva,
                "script_interface_sequence": [call("WaitForEnterKey")],
                "state_sequence": [],
            }
        )
    run_callbacks.append(
        {
            "rva": 115,
            "script_interface_sequence": [],
            "state_sequence": [
                {
                    "owner": "p_MISSION_END",
                    "name": "assign",
                    "code_offset": 10,
                    "arguments": [{"kind": "immediate", "value": 1}],
                    "arguments_exact": True,
                }
            ],
        }
    )

    action_names = [
        "RunTutorial",
        "Show_Main_Title",
        "Remove_Main_Title",
        "Change_Status",
        "Approved",
        "Intro_Wav",
        "EndMissionScript",
    ]
    actions = [
        {
            "name": "RunTutorial",
            "callbacks": run_callbacks,
            "strings": [],
            "script_interface_calls": [],
        }
    ]
    actions.extend(
        {
            "name": name,
            "callbacks": [
                {
                    "rva": 200 + index,
                    "script_interface_sequence": [
                        call(
                            "SucceedMission"
                            if name == "EndMissionScript"
                            else "DisableSave"
                        )
                    ],
                    "state_sequence": [],
                }
            ],
            "strings": [],
            "script_interface_calls": [],
        }
        for index, name in enumerate(action_names[1:])
    )
    rule_names = [
        "IntroRule",
        "ShowMainTitleRule",
        "RemoveMainTitleRule",
        "Change_Status",
        "Approved",
        "Intro_Wav",
        "EndMissionRule",
    ]
    timer_symbols = [
        None,
        "p_tDisplayMainTitle",
        "p_tRemoveMainTitle",
        "p_tChangeStatus",
        "p_tChangeToApproved",
        "p_tIntroWav",
        None,
    ]
    rules = []
    for name, action_name, timer_symbol in zip(rule_names, action_names, timer_symbols):
        symbol = timer_symbol or (
            "p_INTRO" if name == "IntroRule" else "p_MISSION_END"
        )
        rules.append(
            {
                "name": name,
                "actions": [action_name],
                "condition_ir": {
                    "kind": "timer_expired" if timer_symbol else "state_truthy",
                    "symbol": symbol,
                    "symbol_type": "timer_int" if timer_symbol else "hot_int",
                    "confidence": "high",
                },
                "condition_symbols": [symbol],
                "condition_strings": [],
                "script_interface_calls": [],
            }
        )
    return {
        "schema_version": 3,
        "format": "sta2-compiled-mission-manifest",
        "missions": [
            {
                "id": "a2_tutorial1",
                "bootstrap_topology": [
                    {
                        "name": "ScriptInit",
                        "callbacks": [
                            {
                                "rva": 50,
                                "script_interface_sequence": [
                                    {
                                        "name": "GetHandle",
                                        "code_offset": 30,
                                        "arguments": [
                                            {"kind": "string", "value": "Enterprise"}
                                        ],
                                        "arguments_exact": True,
                                        "result_symbol": "Enterprise",
                                    }
                                ],
                                "state_sequence": [
                                    {
                                        "owner": "p_MISSION_END",
                                        "name": "assign",
                                        "code_offset": 10,
                                        "arguments": [{"kind": "immediate", "value": 0}],
                                        "arguments_exact": True,
                                    },
                                    {
                                        "owner": "p_INTRO",
                                        "name": "assign",
                                        "code_offset": 20,
                                        "arguments": [{"kind": "immediate", "value": 1}],
                                        "arguments_exact": True,
                                    },
                                ],
                            }
                        ],
                    }
                ],
                "action_topology": actions,
                "rule_topology": rules,
            }
        ],
    }


class ConversionTests(unittest.TestCase):
    def test_lua_helpers(self) -> None:
        self.assertEqual(lua_identifier("a2_fed01"), "A2Fed01")
        self.assertEqual(lua_quote('a"b\\c'), '"a\\"b\\\\c"')
        self.assertEqual(RELATION_NAMES, {0: "Enemy", 1: "Neutral", 2: "Ally"})

    def test_file_aware_action_hint(self) -> None:
        hint = call_hint("AudioMessage", ["voice.wav"], condition=False)
        self.assertIn("Media:playSound", hint)
        self.assertIn('"voice.wav"', hint)

    def test_generates_review_safe_mmm_scaffold(self) -> None:
        files, report = convert_manifest(fixture_manifest())
        lua = files["a2_test01.lua"]
        self.assertIn("class 'A2Test01Mission'", lua)
        self.assertIn("return false", lua)
        self.assertIn("MonitorHook.Type.OnTrue", lua)
        self.assertIn("Mission:finish(true, delay)", lua)
        self.assertIn("Media:playSound(filename)", lua)
        self.assertIn("MMM.register( A2Test01Mission( ) )", lua)
        self.assertEqual(report["summary"]["rules"], 1)
        self.assertEqual(report["summary"]["executable_rules"], 0)

    def test_can_select_one_mission(self) -> None:
        files, report = convert_manifest(fixture_manifest(), {"A2_TEST01"})
        self.assertIn("a2_test01.lua", files)
        self.assertEqual(report["summary"]["missions"], 1)

    def test_parses_localized_string_rows(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "Dynamic_Localized_Strings.h"
            path.write_text(
                '    "MISSION_TEST", "Localized value",\n', encoding="utf-8"
            )
            self.assertEqual(
                parse_localized_strings(path)["MISSION_TEST"], "Localized value"
            )

    def test_resolves_bzn_title_to_required_lua_filename(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            bzn_dir = Path(temporary)
            path = bzn_dir / "a2_test01.bzn"
            path.write_bytes(
                b"header\x00mission\x00"
                b"{01234567-89ab-cdef-0123-456789abcdef}\x00"
                b"MISSION_TEST\x00"
            )
            self.assertEqual(parse_bzn_title(path), "MISSION_TEST")
            metadata = resolve_bzn_metadata(
                ["a2_test01"], bzn_dir, {"MISSION_TEST": "Test Mission"}
            )
            self.assertEqual(metadata["a2_test01"]["lua_file"], "Test Mission.lua")
            self.assertEqual(metadata["a2_test01"]["bzn_file"], "a2_test01.bzn")

    def test_generates_executable_tutorial_pilot_from_exact_ir(self) -> None:
        files, report = convert_manifest(
            pilot_fixture_manifest(), {"a2_tutorial1"}, pilot=True
        )
        lua = files["UI Tutorial.lua"]
        self.assertIn("self.state[\"p_INTRO\"] = 1", lua)
        self.assertIn("Game.time >= self.deadlines", lua)
        self.assertIn("TextInput:hook", lua)
        self.assertIn('Debug:getKeyState( "\\r" )', lua)
        self.assertIn("self.enterWasDown", lua)
        self.assertIn("Mission:finish", lua)
        self.assertIn("Clean-room UI Tutorial pilot loaded", lua)
        self.assertNotIn("return false", lua)
        self.assertEqual(report["mode"], "executable_pilot")
        self.assertEqual(report["summary"]["executable_rules"], 7)

    def test_overlays_pilot_on_disabled_multi_mission_test_pack(self) -> None:
        manifest = pilot_fixture_manifest()
        manifest["missions"].extend(fixture_manifest()["missions"])
        metadata = {
            "a2_tutorial1": {
                "bzn_file": "a2_tutorial_UI.bzn",
                "raw_title": "UI Tutorial",
                "title": "UI Tutorial",
                "lua_file": "UI Tutorial.lua",
            },
            "a2_test01": {
                "bzn_file": "a2_test01.bzn",
                "raw_title": "MISSION_TEST",
                "title": "Test Mission",
                "lua_file": "Test Mission.lua",
            },
        }
        files, report = convert_manifest(
            manifest, pilot=True, bzn_metadata=metadata
        )
        self.assertIn("UI Tutorial.lua", files)
        self.assertIn("Test Mission.lua", files)
        self.assertNotIn("return false", files["UI Tutorial.lua"])
        self.assertIn("return false", files["Test Mission.lua"])
        self.assertEqual(report["mode"], "mixed_a2_test_pack")
        self.assertEqual(report["summary"]["missions"], 2)
        self.assertEqual(report["summary"]["executable_rules"], 7)
        self.assertEqual(report["summary"]["review_required_rules"], 1)


if __name__ == "__main__":
    unittest.main()
