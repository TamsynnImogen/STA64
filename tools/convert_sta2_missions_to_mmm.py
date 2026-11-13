#!/usr/bin/env python3
"""Generate MegaDroid Mission Mod Lua from clean-room STA2 analysis.

The input is the clean-room JSON manifest produced by
``analyze_sta2_missions.py``. Generated Lua deliberately keeps unresolved
conditions disabled and records evidence plus candidate MMM equivalents for
human review. Schema 3 can additionally overlay one opt-in executable tutorial
pilot on a full review-safe test pack. BZN metadata may be read to give every
Lua file the map-title filename MegaDroid expects. The tool never copies or
executes the original mission binaries.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Sequence


CONDITION_HINTS = {
    "GetTime": "Game.time",
    "IsVisibleToTeam": "entity:canPlayerSee(Player(teamNumber))",
    "GetRealTeam": "entity.player.number",
    "CountUnits": "#EntityFinder(function(e) return e:isType(Entity.Type.Craft) and <filters> end):find()",
    "CountStations": "#EntityFinder(function(e) return e:isType(Entity.Type.Craft) and <station filters> end):find()",
    "CountCapitalShips": "#EntityFinder(function(e) return e:isType(Entity.Type.Craft) and <capital-ship filters> end):find()",
    "CountVisibleUnits": "#EntityFinder(function(e) return e:isType(Entity.Type.Craft) and e:canPlayerSee(Player(teamNumber)) and <filters> end):find()",
    "CountUnitsNearObject": "#EntityFinder(function(e) return e:isType(Entity.Type.Craft) and <distance/object filters> end):find()",
    "CountVisibleUnitsNearObject": "#EntityFinder(function(e) return e:isType(Entity.Type.Craft) and e:canPlayerSee(Player(teamNumber)) and <distance/object filters> end):find()",
    "GetDistance": "(entityA.position - entityB.position):length() -- verify Vector API/version",
    "Exists": "entity ~= nil and entity.valid",
    "DoesNotExist": "entity == nil or not entity.valid",
    "IsAlive": "entity ~= nil and entity.valid and not entity.destroyed",
    "IsExploding": "entity ~= nil and entity.valid and entity.destroyed -- verify explosion timing",
    "IsCompromised": "entity ~= nil and entity.valid and entity.crew <= 0",
    "GetCrew": "entity.crew",
    "UnderAttack": "entity ~= nil and entity.valid and entity.underAttack",
    "IsCargoShipReadyToLoad": "cargoShip ~= nil and cargoShip.valid -- inspect CargoShip properties",
    "IsCargoShipFullAndReadyToSend": "cargoShip ~= nil and cargoShip.valid and cargoShip.spaceRemaining <= 0",
    "IsMoviePlaying": "Cineractive.active -- verify whether the original queried Bink playback",
    "GetCurrentHealth": "entity.hull",
    "GetDilithium": "Player(teamNumber).resources.dilithium",
    "GetMetal": "Player(teamNumber).resources.metal -- Fleet Ops may expose this as tritanium",
    "GetLatinum": "Player(teamNumber).resources.latinum",
    "IsAudioMessageDone": "Timer/Monitor synchronized to Media:playSound duration; MMM exposes no documented completion query",
    "IsShipAliveAndOffMap": "entity ~= nil and entity.valid and <outside Map.min/Map.max>",
    "HasTimeElapsed": "Game.time >= startTime + duration",
    "GetLatinumTraded": "Player(teamNumber):getAmountTraded(Resource.Latinum, direction, otherPlayer)",
}


ACTION_HINTS = {
    "GetTime": "Game.time",
    "SucceedMission": "Mission:finish(true, delay)",
    "FailMission": "Mission:finish(false, delay)",
    "BuildObject": "Entity.add(odf, Player(teamNumber), position)",
    "RemoveObject": "Entity.remove(entity)",
    "RemoveAllObjects": "for _, entity in pairs(EntityFinder(<filters>):find()) do Entity.remove(entity) end",
    "ExplodeNow": "entity:explode()",
    "Attack": "entity:giveOrder(GameObject.Order.Attack, target)",
    "AI_Attack": "entity:giveOrder(GameObject.Order.Attack, target)",
    "Goto": "entity:giveOrder(GameObject.Order.Go, positionOrPath)",
    "Patrol": "entity:giveOrder(GameObject.Order.Patrol, positionOrPath)",
    "Stop": "entity:giveOrder(GameObject.Order.Stop)",
    "Mine": "entity:giveOrder(GameObject.Order.Mine)",
    "Colonize": "entity:giveOrder(GameObject.Order.Colonize, target)",
    "Cloak": "entity:cloak()",
    "Decloak": "entity:decloak()",
    "SetPosition": "entity.position = position",
    "PointAt": "entity.forward = <direction vector>",
    "SetImpulseSpeed": "entity.impulseSpeed = speed",
    "ResetImpulseSpeed": "entity.impulseSpeed = <original speed>",
    "SetCurrentHealth": "entity.hull = healthFraction",
    "SetMaxShields": "entity.maxShields = shieldPoints",
    "SetShieldPercent": "entity.shields = shieldFraction",
    "SetCrew": "entity.crew = amount",
    "SetSpecialEnergy": "entity.specialEnergy = fraction",
    "SetName": "entity.name = name",
    "SetRealTeam": "entity.player = Player(teamNumber)",
    "SetPerceivedTeam": "entity.perceivedTeam = Player(teamNumber)",
    "SetForceOntoMap": "entity.forceOntoMap = enabled",
    "CraftCannotDie": "entity.invincible = true",
    "SetRedAlert": "entity.alert = Craft.AlertState.Red",
    "SetGreenAlert": "entity.alert = Craft.AlertState.Green",
    "SetDilithium": "Player(teamNumber).resources.dilithium = amount",
    "AddDilithium": "Player(teamNumber).resources.dilithium = Player(teamNumber).resources.dilithium + amount",
    "SetMetal": "Player(teamNumber).resources.metal = amount -- verify Fleet Ops resource name",
    "AddMetal": "Player(teamNumber).resources.metal = Player(teamNumber).resources.metal + amount -- verify Fleet Ops resource name",
    "SetLatinum": "Player(teamNumber).resources.latinum = amount",
    "AddLatinum": "Player(teamNumber).resources.latinum = Player(teamNumber).resources.latinum + amount",
    "SetBiomatter": "Player(teamNumber).resources.biomatter = amount",
    "AddBiomatter": "Player(teamNumber).resources.biomatter = Player(teamNumber).resources.biomatter + amount",
    "SetRelation": "Player(teamA):setRelation(Player(teamB), Player.Relation.<relation>)",
    "AI_Team_Load_AIP": "Player(teamNumber):loadAIP(aipFilename)",
    "PlayMusic": "Media:playMusic(filename)",
    "AudioMessage": "Media:playSound(filename)",
    "StopAudioMessage": "Media:stopSounds() -- MMM stops all sounds, not one handle",
    "PlayCinematicMovie": "Media:playMovie(filename)",
    "StopMovie": "Media:stopMovie()",
    "MuteSounds": "Media:stopSounds() -- not equivalent to timed muting",
    "CineractiveStart": "Cineractive.active = true",
    "CineractiveEnd": "Cineractive.active = false",
    "CineractiveShot": "Cineractive:lookFrom(<entity/path/position>); Cineractive:lookAt(<entity/position>)",
    "CineractiveCameraOffset": "Cineractive:setCameraOffset(offset)",
    "CineractiveConstantSpeed": "<configure Path speed on Cineractive:lookFrom>",
    "FadeIn": "Cineractive:fadeIn(time)",
    "FadeOut": "Cineractive:fadeOut(time)",
    "CenterCamera": "Camera:center(entityOrPosition)",
    "SetCameraZoom": "Camera:zoom(amount)",
    "ShowTitle": "UI:addTitle(text, x, y, time, fade)",
    "RemoveTitle": "<allow the MMM title lifetime to expire or replace it>",
    "ShowSubtitle": "UI:addSubtitle(text, x, y, time, fade)",
    "RemoveSubtitle": "<allow the MMM subtitle lifetime to expire or replace it>",
    "MinimapMessage": "UI:addMinimapMark(position, radius, shrinkRate, color)",
    "SetTeamText": "UI.teamText = text",
    "ShowTradesTo": "UI:showTradesTo(Player(teamNumber))",
    "ObjectivesDisplay_Set_Text_From_File": "Objectives:load(filename)",
    "ObjectivesDisplay_Set_Visibility": "Objectives.visible = visible",
    "ObjectivesDisplaySetObjectiveComplete": "Objectives.objectives[index].state = true",
    "DisableSave": "Game.allowSave = false",
    "EnableSave": "Game.allowSave = true",
    "AllowWarp": "Game.allowWarp = enabled",
}


FILE_SUFFIXES = (".aip", ".avi", ".bik", ".mp3", ".odf", ".ogg", ".txt", ".wav")

PILOT_MISSION_ID = "a2_tutorial1"
PILOT_LUA_FILE = "UI Tutorial.lua"
# Confirmed from the stock AllianceEnemy/Neutral/Ally::Get implementations.
RELATION_NAMES = {0: "Enemy", 1: "Neutral", 2: "Ally"}
TITLE_LIFETIMES = {
    "MISSION_TUT_UI_TITLE": 4.0,
    "MISSION_TUT_UI_SUBTITLE": 10.0,
    "MISSION_TUT_UI_SUBTITLE_1": 2.0,
    "MISSION_TUT_UI_SUBTITLE_2": 3.0,
    "MISSION_TUT_UI_SUBTITLE_3": 5.0,
    "MISSION_TUT_UI_SUBJECT_1": 10.0,
}

PILOT_SUPPORTED_CALLS = {
    "AudioMessage",
    "CenterCamera",
    "CineractiveConstantSpeed",
    "CineractiveEnd",
    "CineractiveShot",
    "CineractiveStart",
    "DisableChat",
    "DisableSave",
    "FadeIn",
    "FerengiCanSteal",
    "FerengiCanTrade",
    "GetTime",
    "ObjectivesDisplay_Set_Text_From_File",
    "ObjectivesDisplay_Set_Visibility",
    "PlayMusic",
    "RemoveTitle",
    "SetCanBeDecommissioned",
    "SetRelation",
    "ShowSubtitle",
    "ShowTitle",
    "StopAudioMessage",
    "SucceedMission",
    "WaitForEnterKey",
}


def lua_identifier(value: str, prefix: str = "Generated") -> str:
    words = [word for word in re.split(r"[^A-Za-z0-9]+", value) if word]
    identifier = "".join(word[:1].upper() + word[1:] for word in words)
    if not identifier:
        identifier = prefix
    if identifier[0].isdigit():
        identifier = prefix + identifier
    return identifier


def lua_quote(value: str) -> str:
    escaped = (
        value.replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("\r", "\\r")
        .replace("\n", "\\n")
        .replace("\t", "\\t")
    )
    return f'"{escaped}"'


def parse_localized_strings(path: Path) -> dict[str, str]:
    """Read the simple key/value rows used by Dynamic_Localized_Strings.h."""
    text = path.read_text(encoding="utf-8", errors="replace")
    result: dict[str, str] = {}
    pattern = re.compile(r'^\s*"([^"\\]+)"\s*,\s*"((?:[^"\\]|\\.)*)"', re.MULTILINE)
    for match in pattern.finditer(text):
        value = bytes(match.group(2), "utf-8").decode("unicode_escape")
        result[match.group(1)] = value
    return result


def parse_bzn_title(path: Path) -> str:
    """Read the title from the leading mission record in a binary BZN."""
    data = path.read_bytes()
    values = [
        match.group(0).decode("ascii")
        for match in re.finditer(rb"[\x20-\x7e]{4,}", data)
    ]
    try:
        marker = values.index("mission")
        uuid = values[marker + 1]
        title = values[marker + 2]
    except (ValueError, IndexError) as error:
        raise ValueError(f"cannot locate mission title in BZN: {path}") from error
    if not re.fullmatch(r"\{[0-9A-Fa-f-]{36}\}", uuid):
        raise ValueError(f"unexpected mission record in BZN: {path}")
    if len(title) >= 2 and title[0] == title[-1] == '"':
        title = title[1:-1]
    title = title.strip()
    if not title:
        raise ValueError(f"empty mission title in BZN: {path}")
    return title


def _lua_filename_for_title(title: str) -> str:
    if re.search(r'[<>:"/\\|?*\x00-\x1f]', title) or title.endswith((".", " ")):
        raise ValueError(f"mission title is not a portable filename: {title!r}")
    return title + ".lua"


def resolve_bzn_metadata(
    mission_ids: Sequence[str],
    bzn_dir: Path,
    localizations: dict[str, str],
) -> dict[str, dict[str, str]]:
    """Resolve each map's BZN filename, titles, and required MMM Lua filename."""
    if not bzn_dir.is_dir():
        raise ValueError(f"BZN directory does not exist: {bzn_dir}")
    available = {
        path.name.casefold(): path
        for path in bzn_dir.iterdir()
        if path.is_file() and path.suffix.casefold() == ".bzn"
    }
    metadata: dict[str, dict[str, str]] = {}
    used_lua_files: dict[str, str] = {}
    for mission_id in mission_ids:
        expected = (
            "a2_tutorial_UI.bzn"
            if mission_id.casefold() == PILOT_MISSION_ID
            else mission_id + ".bzn"
        )
        path = available.get(expected.casefold())
        if path is None:
            raise ValueError(f"BZN for {mission_id} was not found in {bzn_dir}")
        raw_title = parse_bzn_title(path)
        title = localizations.get(raw_title, raw_title).strip()
        lua_file = _lua_filename_for_title(title)
        collision = used_lua_files.get(lua_file.casefold())
        if collision is not None:
            raise ValueError(
                f"BZN titles for {collision} and {mission_id} both map to {lua_file}"
            )
        used_lua_files[lua_file.casefold()] = mission_id
        metadata[mission_id.casefold()] = {
            "bzn_file": path.name,
            "raw_title": raw_title,
            "title": title,
            "lua_file": lua_file,
        }
    return metadata


def _argument(call: dict[str, object], index: int) -> dict[str, object]:
    arguments = call.get("arguments", [])
    if not isinstance(arguments, list) or index >= len(arguments):
        raise ValueError(f"{call.get('name', 'call')} has no argument {index}")
    argument = arguments[index]
    if not isinstance(argument, dict):
        raise ValueError(f"{call.get('name', 'call')} argument {index} is malformed")
    return argument


def _argument_value(call: dict[str, object], index: int) -> object:
    argument = _argument(call, index)
    if "value" not in argument:
        raise ValueError(
            f"{call.get('name', 'call')} argument {index} is not a literal value"
        )
    return argument["value"]


def _callback_events(callback: dict[str, object]) -> list[dict[str, object]]:
    events: list[dict[str, object]] = []
    for call in callback.get("script_interface_sequence", []):
        event = dict(call)
        event["event_kind"] = "call"
        events.append(event)
    for operation in callback.get("state_sequence", []):
        event = dict(operation)
        event["event_kind"] = "state"
        events.append(event)
    return sorted(
        events,
        key=lambda event: (
            int(event.get("code_offset", 0)),
            0 if event["event_kind"] == "call" else 1,
        ),
    )


def _condition_expression(condition_ir: dict[str, object]) -> str:
    symbol = lua_quote(str(condition_ir["symbol"]))
    if condition_ir.get("kind") == "state_truthy":
        return f'(self.state[{symbol}] or 0) ~= 0'
    if condition_ir.get("kind") == "timer_expired":
        return (
            f"self.deadlines[{symbol}] ~= nil and "
            f"Game.time >= self.deadlines[{symbol}]"
        )
    raise ValueError(f"unsupported condition IR: {condition_ir.get('kind')}")


def _relation_statement(call: dict[str, object]) -> str:
    team_a = int(_argument_value(call, 0))
    team_b = int(_argument_value(call, 1))
    relation_value = int(_argument_value(call, 2))
    relation = RELATION_NAMES.get(relation_value)
    if relation is None:
        raise ValueError(f"unknown eTeamRelation value: {relation_value}")
    return (
        f"Player( {team_a} ):setRelation( Player( {team_b} ), "
        f"Player.Relation.{relation} )"
    )


def _handle_expression(symbol: str) -> str:
    return f"self.handles[{lua_quote(symbol)}]"


def _pilot_state_lines(operation: dict[str, object]) -> list[str]:
    if not operation.get("arguments_exact"):
        raise ValueError(f"inexact state operation on {operation.get('owner')}")
    owner = lua_quote(str(operation["owner"]))
    name = operation.get("name")
    if name == "assign":
        value = _argument_value(operation, 0)
        if isinstance(value, bool):
            literal = "true" if value else "false"
        elif isinstance(value, (int, float)):
            literal = repr(value)
        else:
            raise ValueError(f"unsupported state value for {operation.get('owner')}")
        return [f"self.state[{owner}] = {literal}"]
    if name == "start":
        duration = float(_argument_value(operation, 0))
        return [f"self.deadlines[{owner}] = Game.time + {duration!r}"]
    if name == "stop":
        return [f"self.deadlines[{owner}] = nil"]
    raise ValueError(f"unsupported state operation: {name}")


def _pilot_call_lines(
    call: dict[str, object], localizations: dict[str, str]
) -> list[str]:
    name = str(call.get("name", ""))
    if name not in PILOT_SUPPORTED_CALLS:
        raise ValueError(f"unsupported tutorial pilot call: {name}")
    if not call.get("arguments_exact"):
        raise ValueError(f"inexact tutorial pilot arguments: {name}")

    if name == "DisableSave":
        return ["Game.allowSave = false"]
    if name == "SetRelation":
        return [_relation_statement(call)]
    if name == "SetCanBeDecommissioned":
        symbol = str(_argument(call, 0).get("symbol", "unknown"))
        return [
            f"-- COMPAT: MMM has no documented decommission lock for {symbol}."
        ]
    if name in {"FerengiCanSteal", "FerengiCanTrade"}:
        return [f"-- COMPAT: MMM has no documented {name} equivalent."]
    if name == "DisableChat":
        return [
            "-- COMPAT: chat remains enabled because it carries the pilot's next command."
        ]
    if name == "FadeIn":
        return [f"Cineractive:fadeIn( {float(_argument_value(call, 1))!r} )"]
    if name == "CineractiveStart":
        skippable = bool(_argument_value(call, 1))
        return [
            "Cineractive.active = true",
            f"Cineractive.skippable = {'true' if skippable else 'false'}",
        ]
    if name == "CineractiveShot":
        path = str(_argument_value(call, 0))
        speed = float(_argument_value(call, 1))
        target = str(_argument(call, 2).get("symbol", "Enterprise"))
        handle = _handle_expression(target)
        return [
            f"Cineractive:lookFrom( Path( {lua_quote(path)} ), {speed!r} )",
            f"if {handle} ~= nil then Cineractive:lookAt( {handle} ) end",
        ]
    if name == "PlayMusic":
        return [f"Media:playMusic( {lua_quote(str(_argument_value(call, 1)))} )"]
    if name == "CineractiveConstantSpeed":
        return [
            "-- COMPAT: constant-speed intent is represented by lookFrom(path, speed)."
        ]
    if name == "CineractiveEnd":
        return ["-- CineractiveEnd is delivered through cineractiveFinish below."]
    if name == "StopAudioMessage":
        return [
            "Media:stopSounds( ) -- COMPAT: broader than stopping one stock handle."
        ]
    if name == "CenterCamera":
        symbol = str(_argument(call, 0).get("symbol", "Enterprise"))
        handle = _handle_expression(symbol)
        return [f"if {handle} ~= nil then Camera:center( {handle} ) end"]
    if name == "RemoveTitle":
        return ["-- COMPAT: the matching MMM title expires via its generated lifetime."]
    if name in {"ShowTitle", "ShowSubtitle"}:
        x = int(_argument_value(call, 0))
        y = int(_argument_value(call, 1))
        key = str(_argument_value(call, 2))
        text = localizations.get(key, key)
        fade = bool(_argument_value(call, 3))
        lifetime = TITLE_LIFETIMES.get(key, 5.0)
        method = "addTitle" if name == "ShowTitle" else "addSubtitle"
        return [
            f"UI:{method}( {lua_quote(text)}, {x}, {y}, {lifetime!r}, "
            f"{'true' if fade else 'false'} )"
        ]
    if name == "ObjectivesDisplay_Set_Text_From_File":
        return [f"Objectives:load( {lua_quote(str(_argument_value(call, 0)))} )"]
    if name == "ObjectivesDisplay_Set_Visibility":
        visible = bool(_argument_value(call, 0))
        return [f"Objectives.visible = {'true' if visible else 'false'}"]
    if name == "AudioMessage":
        return [f"Media:playSound( {lua_quote(str(_argument_value(call, 1)))} )"]
    if name == "GetTime":
        return ["-- GetTime result is consumed by SucceedMission."]
    if name == "SucceedMission":
        return ["Mission:finish( true, 0 )"]
    if name == "WaitForEnterKey":
        return []
    raise AssertionError(name)


def _pilot_event_lines(
    event: dict[str, object], localizations: dict[str, str]
) -> list[str]:
    if event.get("event_kind") == "state":
        return _pilot_state_lines(event)
    return _pilot_call_lines(event, localizations)


def _tutorial_steps(callbacks: Sequence[dict[str, object]]) -> list[list[dict[str, object]]]:
    steps: list[list[dict[str, object]]] = []
    current: list[dict[str, object]] = []
    for callback in callbacks:
        for event in _callback_events(callback):
            if event.get("event_kind") == "call" and event.get("name") == "WaitForEnterKey":
                steps.append(current)
                current = []
            else:
                current.append(event)
    if current:
        steps.append(current)
    return steps


def comment_values(values: Sequence[str]) -> str:
    return ", ".join(values) if values else "none observed"


def matching_files(strings: Sequence[str], suffixes: Sequence[str]) -> list[str]:
    lowered = tuple(suffix.casefold() for suffix in suffixes)
    return [value for value in strings if value.casefold().endswith(lowered)]


def call_hint(
    call_name: str,
    strings: Sequence[str],
    *,
    condition: bool,
) -> str | None:
    hints = CONDITION_HINTS if condition else ACTION_HINTS
    hint = hints.get(call_name)
    if hint is None:
        return None
    relevant: list[str] = []
    if call_name in {"PlayMusic", "AudioMessage", "StopAudioMessage"}:
        relevant = matching_files(strings, (".mp3", ".ogg", ".wav"))
    elif call_name in {"PlayCinematicMovie", "StopMovie"}:
        relevant = matching_files(strings, (".avi", ".bik"))
    elif call_name == "AI_Team_Load_AIP":
        relevant = matching_files(strings, (".aip",))
    elif call_name == "ObjectivesDisplay_Set_Text_From_File":
        relevant = matching_files(strings, (".txt",))
    elif call_name == "BuildObject":
        relevant = matching_files(strings, (".odf",))
    if relevant:
        hint += " ; observed files: " + ", ".join(lua_quote(value) for value in relevant)
    return hint


def action_index(mission: dict[str, object]) -> dict[str, dict[str, object]]:
    return {
        str(action["name"]): action
        for action in mission.get("action_topology", [])
    }


def render_rule_functions(
    class_name: str,
    mission_id: str,
    rule: dict[str, object],
    action_details: dict[str, dict[str, object]],
    rule_number: int,
) -> tuple[list[str], dict[str, object]]:
    rule_name = str(rule["name"])
    function_suffix = lua_identifier(rule_name, "Rule")
    condition_function = f"condition{function_suffix}"
    action_function = f"action{function_suffix}"
    condition_calls = [
        str(call.get("name", f"slot_{call['slot']}"))
        for call in rule.get("script_interface_calls", [])
    ]
    condition_strings = [str(value) for value in rule.get("condition_strings", [])]
    condition_hints = [
        hint
        for call_name in condition_calls
        if (hint := call_hint(call_name, condition_strings, condition=True))
    ]
    actions = [str(value) for value in rule.get("actions", [])]
    action_calls: list[str] = []
    action_strings: list[str] = []
    action_hints: list[str] = []
    for action_name in actions:
        details = action_details.get(action_name, {})
        strings = [str(value) for value in details.get("strings", [])]
        for value in strings:
            if value not in action_strings:
                action_strings.append(value)
        for call in details.get("script_interface_calls", []):
            call_name = str(call.get("name", f"slot_{call['slot']}"))
            if call_name not in action_calls:
                action_calls.append(call_name)
            hint = call_hint(call_name, strings, condition=False)
            if hint and hint not in action_hints:
                action_hints.append(hint)

    lines = [
        f"function {class_name}:{condition_function}( )",
        f"    -- Original DRL rule: {rule_name}",
        "    -- Condition symbols: "
        + comment_values([str(value) for value in rule.get("condition_symbols", [])]),
        "    -- ScriptInterface queries: " + comment_values(condition_calls),
        "    -- Condition literals: " + comment_values(condition_strings),
    ]
    if condition_hints:
        lines.append("    -- Candidate MMM expressions (arguments/operators require review):")
        lines.extend(f"    --   {hint}" for hint in condition_hints)
    else:
        lines.append("    -- No verified automatic condition mapping is available yet.")
    lines.extend(
        [
            "    -- Disabled until the original comparisons and boolean operators are recovered.",
            "    return false",
            "end",
            "",
            f"function {class_name}:{action_function}( )",
            "    -- Original DSL action(s): " + comment_values(actions),
            "    -- ScriptInterface action sequence: " + comment_values(action_calls),
            "    -- Action literals: " + comment_values(action_strings),
        ]
    )
    if action_hints:
        lines.append("    -- Candidate MMM statements (arguments/order require review):")
        lines.extend(f"    --   {hint}" for hint in action_hints)
    else:
        lines.append("    -- No verified automatic action mapping is available yet.")
    lines.extend(["    -- TODO: implement and verify this action.", "end", ""])

    rule_record = {
        "name": rule_name,
        "condition_function": condition_function,
        "action_function": action_function,
        "condition_calls": condition_calls,
        "condition_hints": condition_hints,
        "actions": actions,
        "action_calls": action_calls,
        "action_hints": action_hints,
        "status": "review_required",
    }
    rule_record["monitor_name"] = f"{mission_id}.{rule_number:03d}.{rule_name}"
    return lines, rule_record


def render_mission(
    mission: dict[str, object],
    metadata: dict[str, str] | None = None,
) -> tuple[str, dict[str, object]]:
    mission_id = str(mission["id"])
    class_name = lua_identifier(mission_id, "Mission") + "Mission"
    details = action_index(mission)
    rules = mission.get("rule_topology", [])
    lines = [
        "-- AUTO-GENERATED CLEAN-ROOM MMM CONVERSION SCAFFOLD",
        f"-- Source inventory mission: {mission_id}",
        "-- This file is syntactically valid but intentionally not behaviorally complete.",
        "-- Every unresolved condition returns false so incomplete rules cannot fire accidentally.",
        "",
        f"class '{class_name}'",
        "",
        f"function {class_name}:__init( )",
        "    self.ruleMonitors = { }",
        "end",
        "",
    ]
    rule_records: list[dict[str, object]] = []
    for rule_number, rule in enumerate(rules, start=1):
        rule_lines, record = render_rule_functions(
            class_name, mission_id, rule, details, rule_number
        )
        lines.extend(rule_lines)
        rule_records.append(record)

    lines.extend([f"function {class_name}:registerRules( )", "    local app = self", ""])
    for rule_number, record in enumerate(rule_records, start=1):
        variable = f"monitor{rule_number:03d}"
        monitor_name = str(record["monitor_name"])
        condition_function = str(record["condition_function"])
        action_function = str(record["action_function"])
        lines.extend(
            [
                f"    local {variable} = Monitor( {lua_quote(monitor_name)}, false, nil,",
                "        function( monitor, argument )",
                f"            return argument:{condition_function}( )",
                "        end, app )",
                f"    {variable}:hook( {lua_quote('fire.' + monitor_name)}, MonitorHook.Type.OnTrue, nil,",
                "        function( monitor, hook )",
                f"            app:{action_function}( )",
                "            monitor:stop( )",
                "        end, nil )",
                f"    self.ruleMonitors[#self.ruleMonitors + 1] = {variable}",
                "",
            ]
        )
    lines.extend(
        [
            "end",
            "",
            f"function {class_name}:setup( )",
            "    self:registerRules( )",
            "end",
            "",
            f"MMM.register( {class_name}( ) )",
            "",
        ]
    )

    condition_call_count = sum(len(record["condition_calls"]) for record in rule_records)
    action_call_count = sum(len(record["action_calls"]) for record in rule_records)
    condition_hint_count = sum(len(record["condition_hints"]) for record in rule_records)
    action_hint_count = sum(len(record["action_hints"]) for record in rule_records)
    report = {
        "id": mission_id,
        "class": class_name,
        "lua_file": metadata["lua_file"] if metadata else f"{mission_id}.lua",
        "rules": len(rule_records),
        "review_required_rules": len(rule_records),
        "executable_rules": 0,
        "condition_calls": condition_call_count,
        "condition_hints": condition_hint_count,
        "action_calls": action_call_count,
        "action_hints": action_hint_count,
        "rule_details": rule_records,
    }
    if metadata:
        report.update(metadata)
    return "\n".join(lines), report


def _named_topology(
    records: Sequence[dict[str, object]], name: str
) -> dict[str, object]:
    for record in records:
        if str(record.get("name")) == name:
            return record
    raise ValueError(f"required topology entry is missing: {name}")


def _indented(lines: Sequence[str], spaces: int = 4) -> list[str]:
    prefix = " " * spaces
    return [prefix + line if line else "" for line in lines]


def _bootstrap_evidence(
    mission: dict[str, object]
) -> tuple[list[dict[str, object]], list[dict[str, object]], list[dict[str, object]]]:
    script_init = _named_topology(mission.get("bootstrap_topology", []), "ScriptInit")
    states: list[dict[str, object]] = []
    handles: list[dict[str, object]] = []
    relations: list[dict[str, object]] = []
    for callback in script_init.get("callbacks", []):
        for event in _callback_events(callback):
            if event.get("event_kind") == "state" and event.get("name") == "assign":
                states.append(event)
            elif event.get("event_kind") == "call" and event.get("name") == "GetHandle":
                handles.append(event)
            elif event.get("event_kind") == "call" and event.get("name") == "SetRelation":
                relations.append(event)
    if not states or not handles:
        raise ValueError("a2_tutorial1 ScriptInit evidence is incomplete")
    return states, handles, relations


def render_tutorial1_pilot(
    mission: dict[str, object],
    localizations: dict[str, str],
    metadata: dict[str, str] | None = None,
) -> tuple[str, dict[str, object]]:
    """Render the first opt-in executable pilot from schema-3 exact IR."""
    if str(mission.get("id", "")).casefold() != PILOT_MISSION_ID:
        raise ValueError(
            f"executable pilot currently supports only {PILOT_MISSION_ID}"
        )

    class_name = "A2Tutorial1PilotMission"
    actions = action_index(mission)
    rules = mission.get("rule_topology", [])
    bootstrap_states, bootstrap_handles, bootstrap_relations = _bootstrap_evidence(
        mission
    )

    initial_state: dict[str, object] = {}
    for operation in bootstrap_states:
        if not operation.get("arguments_exact"):
            raise ValueError(f"inexact ScriptInit state: {operation.get('owner')}")
        initial_state[str(operation["owner"])] = _argument_value(operation, 0)
    if initial_state.get("p_INTRO") != 1 or initial_state.get("p_MISSION_END") != 0:
        raise ValueError("unexpected a2_tutorial1 initial state")

    referenced_actions = {
        str(action_name)
        for rule in rules
        for action_name in rule.get("actions", [])
    }
    for action_name in referenced_actions:
        action = _named_topology(list(actions.values()), action_name)
        callbacks = action.get("callbacks", [])
        if not callbacks:
            raise ValueError(f"{action_name} has no recovered callback sequence")
        for callback in callbacks:
            for call in callback.get("script_interface_sequence", []):
                name = str(call.get("name", ""))
                if name not in PILOT_SUPPORTED_CALLS or not call.get("arguments_exact"):
                    raise ValueError(f"{action_name} has unsupported/inexact call: {name}")
            for operation in callback.get("state_sequence", []):
                if not operation.get("arguments_exact"):
                    raise ValueError(
                        f"{action_name} has inexact state operation: {operation.get('owner')}"
                    )

    lines = [
        "-- AUTO-GENERATED CLEAN-ROOM MMM EXECUTABLE PILOT",
        "-- Source mission: a2_tutorial1 / BZN: a2_tutorial_UI.bzn / title: UI Tutorial",
        "-- Generated only from schema-3 proven IR plus the documented COMPAT adaptations below.",
        "-- Advancement adaptation: edge-detect Enter through Debug:getKeyState; chat command next remains a fallback.",
        "",
        f"class '{class_name}'",
        "",
        f"function {class_name}:__init( )",
        "    self.state = { }",
        "    self.deadlines = { }",
        "    self.handles = { }",
        "    self.ruleMonitors = { }",
        "    self.introRunning = false",
        "    self.tutorialStarted = false",
        "    self.awaitingAdvance = false",
        "    self.enterWasDown = false",
        "    self.tutorialStep = 0",
    ]
    for owner, value in sorted(initial_state.items()):
        lines.append(f"    self.state[{lua_quote(owner)}] = {value!r}")
    lines.extend(["end", "", f"function {class_name}:bindMapHandles( )"])
    for call in bootstrap_handles:
        if not call.get("arguments_exact") or not call.get("result_symbol"):
            raise ValueError("inexact ScriptInit GetHandle binding")
        label = str(_argument_value(call, 0))
        symbol = str(call["result_symbol"])
        lines.extend(
            [
                f"    self.handles[{lua_quote(symbol)}] = EntityFinder( function( e )",
                "        return e:isType( Entity.Type.GameObject ) and",
                f"            ( e.label == {lua_quote(label)} or e.name == {lua_quote(label)} )",
                "    end ):findOne( )",
            ]
        )
    lines.extend(["end", ""])

    action_methods: dict[str, str] = {}
    for action_name in sorted(referenced_actions, key=str.casefold):
        action_methods[action_name] = "run" + lua_identifier(action_name, "Action")

    run_tutorial = _named_topology(list(actions.values()), "RunTutorial")
    run_callbacks = run_tutorial.get("callbacks", [])
    first_callback = run_callbacks[0]
    run_method = action_methods["RunTutorial"]
    lines.extend([f"function {class_name}:{run_method}( )"])
    for event in _callback_events(first_callback):
        lines.extend(_indented(_pilot_event_lines(event, localizations)))
    lines.extend(
        [
            "    self.introRunning = true",
            "end",
            "",
        ]
    )

    for action_name in sorted(referenced_actions - {"RunTutorial"}, key=str.casefold):
        action = _named_topology(list(actions.values()), action_name)
        method = action_methods[action_name]
        lines.append(f"function {class_name}:{method}( )")
        for callback in action.get("callbacks", []):
            for event in _callback_events(callback):
                lines.extend(_indented(_pilot_event_lines(event, localizations)))
        lines.extend(["end", ""])

    steps = _tutorial_steps(run_callbacks[1:])
    if len(steps) != 15:
        raise ValueError(f"unexpected a2_tutorial1 step count: {len(steps)}")
    lines.extend([f"function {class_name}:runTutorialStep( )"])
    for step_number, events in enumerate(steps, start=1):
        keyword = "if" if step_number == 1 else "elseif"
        lines.append(f"    {keyword} self.tutorialStep == {step_number} then")
        for event in events:
            lines.extend(_indented(_pilot_event_lines(event, localizations), 8))
        if step_number < len(steps):
            lines.extend(
                [
                    "        self.awaitingAdvance = true",
                    "        UI:addSubtitle( \"Press Enter to continue (Esc closes chat; or type next)\", 50, 1050, 5.0, false )",
                ]
            )
        else:
            lines.append("        self.awaitingAdvance = false")
    lines.extend(["    end", "end", ""])

    lines.extend(
        [
            f"function {class_name}:advanceTutorial( )",
            "    if not self.awaitingAdvance then return end",
            "    self.awaitingAdvance = false",
            "    self.tutorialStep = self.tutorialStep + 1",
            "    self:runTutorialStep( )",
            "end",
            "",
            f"function {class_name}:onAdvance( text, hook )",
            "    self:advanceTutorial( )",
            "end",
            "",
            f"function {class_name}:update( )",
            "    local enterDown = Debug:getKeyState( \"\\r\" )",
            "    if enterDown and not self.enterWasDown then",
            "        self:advanceTutorial( )",
            "    end",
            "    self.enterWasDown = enterDown",
            "end",
            "",
            f"function {class_name}:cineractiveUpdate( )",
            "    local deadline = self.deadlines[\"p_tRemoveAuthorization\"]",
            "    if self.introRunning and deadline ~= nil and Game.time >= deadline then",
            "        Cineractive.active = false",
            "    end",
            "end",
            "",
            f"function {class_name}:cineractiveFinish( forced )",
            "    if not self.introRunning or self.tutorialStarted then return end",
            "    self.introRunning = false",
            "    self.tutorialStarted = true",
            "    self.tutorialStep = 1",
            "    self:runTutorialStep( )",
            "end",
            "",
        ]
    )

    rule_records: list[dict[str, object]] = []
    for rule_number, rule in enumerate(rules, start=1):
        condition_ir = rule.get("condition_ir")
        if not isinstance(condition_ir, dict) or condition_ir.get("confidence") != "high":
            raise ValueError(f"rule lacks proven condition IR: {rule.get('name')}")
        suffix = lua_identifier(str(rule["name"]), "Rule")
        condition_method = "condition" + suffix
        fire_method = "fire" + suffix
        expression = _condition_expression(condition_ir)
        lines.extend(
            [
                f"function {class_name}:{condition_method}( )",
                f"    return {expression}",
                "end",
                "",
                f"function {class_name}:{fire_method}( )",
            ]
        )
        for action_name in rule.get("actions", []):
            lines.append(f"    self:{action_methods[str(action_name)]}( )")
        lines.extend(["end", ""])
        rule_records.append(
            {
                "name": str(rule["name"]),
                "condition_function": condition_method,
                "action_function": fire_method,
                "monitor_name": f"a2_tutorial1.{rule_number:03d}.{rule['name']}",
                "status": "executable_with_documented_adaptations",
            }
        )

    lines.extend([f"function {class_name}:registerRules( )", "    local app = self", ""])
    for number, record in enumerate(rule_records, start=1):
        monitor = f"monitor{number:03d}"
        monitor_name = str(record["monitor_name"])
        lines.extend(
            [
                f"    local {monitor} = Monitor( {lua_quote(monitor_name)}, false, nil,",
                "        function( instance, argument )",
                f"            return argument:{record['condition_function']}( )",
                "        end, app )",
                f"    {monitor}:hook( {lua_quote('fire.' + monitor_name)}, MonitorHook.Type.OnTrue, nil,",
                "        function( instance, hook )",
                f"            app:{record['action_function']}( )",
                "            instance:stop( )",
                "        end, nil )",
                f"    self.ruleMonitors[#self.ruleMonitors + 1] = {monitor}",
                "",
            ]
        )
    lines.extend(
        [
            "end",
            "",
            f"function {class_name}:setup( )",
            "    UI:addSubtitle( \"Clean-room UI Tutorial pilot loaded\", 50, 50, 6.0, false )",
            "    self:bindMapHandles( )",
        ]
    )
    for call in bootstrap_relations:
        if not call.get("arguments_exact"):
            raise ValueError("inexact ScriptInit SetRelation")
        lines.append("    " + _relation_statement(call))
    lines.extend(
        [
            f"    TextInput:hook( \"a2_tutorial1.next\", self, {class_name}.onAdvance, nil, \"^[Nn][Ee][Xx][Tt]$\" )",
            "    self:registerRules( )",
            "end",
            "",
            f"MMM.register( {class_name}( ) )",
            "",
        ]
    )

    action_call_count = sum(
        len(callback.get("script_interface_sequence", []))
        for action_name in referenced_actions
        for callback in actions[action_name].get("callbacks", [])
    )
    state_operation_count = sum(
        len(callback.get("state_sequence", []))
        for action_name in referenced_actions
        for callback in actions[action_name].get("callbacks", [])
    )
    unresolved_keys = sorted(
        key
        for key in TITLE_LIFETIMES
        if key not in localizations
    )
    adaptations = [
        "A six-second setup banner identifies successful MMM/Lua loading during validation.",
        "WaitForEnterKey is edge-detected with Debug:getKeyState('\\r'); the TextInput chat command 'next' remains a fallback.",
        "The recovered 17-second p_tRemoveAuthorization deadline ends the MMM cineractive.",
        "RemoveTitle is represented by finite generated UI title lifetimes.",
        "StopAudioMessage maps to Media:stopSounds and therefore has broader scope.",
        "Decommission locks and Ferengi steal/trade controls have no documented MMM equivalent.",
        "Stock DisableChat is omitted because MMM exposes no documented chat-disable control; Enter may open chat, which Escape closes.",
    ]
    if unresolved_keys:
        adaptations.append(
            "Unresolved localization keys are displayed literally: "
            + ", ".join(unresolved_keys)
        )
    report = {
        "id": PILOT_MISSION_ID,
        "class": class_name,
        "lua_file": PILOT_LUA_FILE,
        "rules": len(rule_records),
        "review_required_rules": 0,
        "executable_rules": len(rule_records),
        "condition_calls": len(rule_records),
        "condition_hints": len(rule_records),
        "action_calls": action_call_count,
        "action_hints": action_call_count,
        "rule_details": rule_records,
        "evidence": {
            "exact_conditions": len(rule_records),
            "exact_action_calls": action_call_count,
            "exact_state_operations": state_operation_count,
            "script_init_state_assignments": len(bootstrap_states),
            "script_init_handle_bindings": len(bootstrap_handles),
            "tutorial_steps": len(steps),
        },
        "compatibility_adaptations": adaptations,
        "semantic_status": "executable_pilot_pending_in_engine_validation",
    }
    if metadata:
        report.update(metadata)
    return "\n".join(lines), report


def render_pilot_readme(report: dict[str, object]) -> str:
    mission = report["missions"][0]
    lines = [
        "# STA2 UI Tutorial executable MMM pilot",
        "",
        "This is the first opt-in executable output from the clean-room STA2 mission IR. All seven DRL rule conditions, ScriptInit state, object bindings, callback order, state operations, and typed ScriptInterface arguments used by the pilot were recovered mechanically.",
        "",
        "It is not yet claimed as a behaviorally identical conversion. The conversion report records the deliberate Fleet Operations/MegaDroid compatibility adaptations.",
        "",
        f"- Lua file: `{mission['lua_file']}`",
        f"- Expected map: `{mission.get('bzn_file', 'a2_tutorial_UI.bzn')}` (`{mission.get('title', 'UI Tutorial')}`)",
        "- Advancement control: press Enter; if chat opens, Escape closes it. Typing `next` remains a fallback",
        "- Required stock resources: the referenced `a2_tutorial1_*.txt`, WAV, and music files",
        "- Runtime dependency: MegaDroid's `mmm_loader.dll` (not redistributed here)",
        "",
        "To test, make a working copy of the BZN, point its mission script at `mmm_loader.dll`, place this Lua file and the referenced resources where MMM can load them, and add the map to the Fleet Operations single-player configuration. Preserve the original map and game installation.",
        "",
        "## Compatibility adaptations",
        "",
    ]
    lines.extend(f"- {item}" for item in mission["compatibility_adaptations"])
    lines.append("")
    return "\n".join(lines)


def render_test_pack_readme(report: dict[str, object]) -> str:
    summary = report["summary"]
    lines = [
        "# STA2 MegaDroid mission test pack",
        "",
        "This directory contains one title-matched Lua file for every analyzed Armada II mission. It was generated from clean-room structural manifests and BZN metadata; no original map or game resource is copied here.",
        "",
        "Only **UI Tutorial** is an executable pilot. Its seven rules come from proven schema-3 IR and still require validation inside Fleet Operations. Every rule in the other 35 files deliberately returns `false`, so those review conversions cannot trigger incomplete mission actions.",
        "",
        f"- Missions/Lua files: {summary['missions']}",
        f"- Rules represented: {summary['rules']}",
        f"- Executable pilot rules: {summary['executable_rules']}",
        f"- Disabled rules requiring review: {summary['review_required_rules']}",
        "- Runtime dependency: MegaDroid's `mmm_loader.dll` (not redistributed here)",
        "- UI Tutorial control: press Enter; Escape closes chat if it opens, and typing `next` remains a fallback",
        "",
        "For each test, make a working copy of the corresponding BZN, change its mission script to `mmm_loader.dll`, and place the matching Lua file where MMM loads missions. Keep the stock files available to the mod for referenced objectives, audio, music, ODFs, paths, and labeled map objects. Preserve the original maps and game installation.",
        "",
        "## Contents",
        "",
        "| Mission ID | BZN | Required Lua filename | State | Rules |",
        "|---|---|---|---|---:|",
    ]
    for mission in report["missions"]:
        state = (
            "executable pilot; validate in engine"
            if mission["executable_rules"]
            else "disabled review scaffold"
        )
        lines.append(
            f"| `{mission['id']}` | `{mission.get('bzn_file', 'unknown')}` | "
            f"`{mission['lua_file']}` | {state} | {mission['rules']} |"
        )
    lines.append("")
    return "\n".join(lines)


def render_readme(report: dict[str, object]) -> str:
    summary = report["summary"]
    lines = [
        "# STA2 to MegaDroid Mission Mod conversion scaffolds",
        "",
        "These Lua files were generated from clean-room structural manifests. They do not contain recovered source code and the original DLLs were not executed.",
        "",
        "The scaffolds preserve rule names, state/object references, observable engine calls, and literal filenames or identifiers. Every generated condition currently returns `false`; this is deliberate because comparisons, boolean operators, numeric arguments, and full call ordering still require verification.",
        "",
        f"- Missions: {summary['missions']}",
        f"- Rules represented: {summary['rules']}",
        f"- Executable rules: {summary['executable_rules']}",
        f"- Rules requiring review: {summary['review_required_rules']}",
        f"- Condition mapping hints: {summary['condition_hints']}/{summary['condition_calls']}",
        f"- Action mapping hints: {summary['action_hints']}/{summary['action_calls']}",
        "",
        (
            "Use these files as reviewable implementation worksheets, not drop-in completed missions. Their filenames already match the resolved BZN titles expected by MMM; once a mission is verified, set the working map's script to `mmm_loader.dll`."
            if all("bzn_file" in mission for mission in report["missions"])
            else "Use these files as reviewable implementation worksheets, not drop-in completed missions. Once a mission is verified, rename its Lua file to the BZN mission title expected by MMM and set the map script to `mmm_loader.dll`."
        ),
        "",
        "## Missions",
        "",
        "| Mission | Rules | Condition hints | Action hints |",
        "|---|---:|---:|---:|",
    ]
    for mission in report["missions"]:
        lines.append(
            f"| `{mission['id']}` | {mission['rules']} | "
            f"{mission['condition_hints']}/{mission['condition_calls']} | "
            f"{mission['action_hints']}/{mission['action_calls']} |"
        )
    lines.append("")
    return "\n".join(lines)


def _conversion_summary(
    mission_reports: Sequence[dict[str, object]],
) -> dict[str, int]:
    fields = (
        "rules",
        "executable_rules",
        "review_required_rules",
        "condition_calls",
        "condition_hints",
        "action_calls",
        "action_hints",
    )
    summary = {field: sum(int(item[field]) for item in mission_reports) for field in fields}
    summary["missions"] = len(mission_reports)
    return summary


def convert_manifest(
    manifest: dict[str, object],
    selected_missions: set[str] | None = None,
    *,
    pilot: bool = False,
    localizations: dict[str, str] | None = None,
    bzn_metadata: dict[str, dict[str, str]] | None = None,
) -> tuple[dict[str, str], dict[str, object]]:
    if manifest.get("format") != "sta2-compiled-mission-manifest":
        raise ValueError("input is not an STA2 compiled-mission manifest")
    missions = manifest.get("missions")
    if not isinstance(missions, list):
        raise ValueError("manifest has no mission list")
    if selected_missions:
        wanted = {value.casefold() for value in selected_missions}
        missions = [
            mission for mission in missions if str(mission.get("id", "")).casefold() in wanted
        ]
        found = {str(mission["id"]).casefold() for mission in missions}
        missing = sorted(wanted - found)
        if missing:
            raise ValueError("mission(s) not found: " + ", ".join(missing))
    if not missions:
        raise ValueError("no missions selected")

    if pilot:
        if int(manifest.get("schema_version", 0)) < 3:
            raise ValueError("executable pilot requires analyzer schema 3 or newer")
        if not any(
            str(mission.get("id", "")).casefold() == PILOT_MISSION_ID
            for mission in missions
        ):
            raise ValueError(
                f"executable pilot overlay requires selecting {PILOT_MISSION_ID}"
            )
        files: dict[str, str] = {}
        mission_reports: list[dict[str, object]] = []
        for mission in missions:
            mission_id = str(mission["id"])
            metadata = (bzn_metadata or {}).get(mission_id.casefold())
            if mission_id.casefold() == PILOT_MISSION_ID:
                lua, mission_report = render_tutorial1_pilot(
                    mission, localizations or {}, metadata
                )
            else:
                lua, mission_report = render_mission(mission, metadata)
            lua_file = str(mission_report["lua_file"])
            if any(name.casefold() == lua_file.casefold() for name in files):
                raise ValueError(f"duplicate generated Lua filename: {lua_file}")
            files[lua_file] = lua
            mission_reports.append(mission_report)
        mode = "executable_pilot" if len(mission_reports) == 1 else "mixed_a2_test_pack"
        report = {
            "schema_version": 2,
            "format": "sta2-megadroid-conversion-report",
            "source_manifest_schema": manifest.get("schema_version"),
            "mode": mode,
            "summary": _conversion_summary(mission_reports),
            "missions": mission_reports,
        }
        files["README.md"] = (
            render_pilot_readme(report)
            if mode == "executable_pilot"
            else render_test_pack_readme(report)
        )
        files["conversion-report.json"] = (
            json.dumps(report, indent=2, sort_keys=True) + "\n"
        )
        return files, report

    files: dict[str, str] = {}
    mission_reports: list[dict[str, object]] = []
    for mission in missions:
        mission_id = str(mission["id"])
        metadata = (bzn_metadata or {}).get(mission_id.casefold())
        lua, mission_report = render_mission(mission, metadata)
        lua_file = str(mission_report["lua_file"])
        if any(name.casefold() == lua_file.casefold() for name in files):
            raise ValueError(f"duplicate generated Lua filename: {lua_file}")
        files[lua_file] = lua
        mission_reports.append(mission_report)
    report = {
        "schema_version": 1,
        "format": "sta2-megadroid-conversion-report",
        "source_manifest_schema": manifest.get("schema_version"),
        "mode": "review_scaffold",
        "summary": _conversion_summary(mission_reports),
        "missions": mission_reports,
    }
    files["README.md"] = render_readme(report)
    files["conversion-report.json"] = json.dumps(report, indent=2, sort_keys=True) + "\n"
    return files, report


def write_conversion(files: dict[str, str], output_dir: Path, force: bool) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    collisions = sorted(name for name in files if (output_dir / name).exists())
    if collisions and not force:
        raise ValueError(
            f"output files already exist in {output_dir}; pass --force to replace: "
            + ", ".join(collisions)
        )
    for name, content in files.items():
        (output_dir / name).write_text(content, encoding="utf-8", newline="\n")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Generate review-safe MegaDroid Lua with an optional STA2 tutorial pilot overlay."
    )
    parser.add_argument("manifest", type=Path, help="sta2-missions.json from the analyzer")
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument(
        "--mission",
        action="append",
        default=[],
        help="convert one mission ID; may be repeated (default: all)",
    )
    parser.add_argument(
        "--force", action="store_true", help="replace generated files already in output-dir"
    )
    parser.add_argument(
        "--pilot",
        action="store_true",
        help="make a2_tutorial1 executable while leaving other selected missions disabled",
    )
    parser.add_argument(
        "--localization-header",
        type=Path,
        help="optional Dynamic_Localized_Strings.h used to resolve tutorial UI keys",
    )
    parser.add_argument(
        "--bzn-dir",
        type=Path,
        help="optional stock BZN directory used to generate MMM title-matched Lua filenames",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
        localizations = (
            parse_localized_strings(args.localization_header)
            if args.localization_header
            else {}
        )
        selected = {value.casefold() for value in args.mission}
        mission_ids = [
            str(mission["id"])
            for mission in manifest.get("missions", [])
            if not selected or str(mission["id"]).casefold() in selected
        ]
        bzn_metadata = (
            resolve_bzn_metadata(mission_ids, args.bzn_dir, localizations)
            if args.bzn_dir
            else None
        )
        files, report = convert_manifest(
            manifest,
            set(args.mission) or None,
            pilot=args.pilot,
            localizations=localizations,
            bzn_metadata=bzn_metadata,
        )
        write_conversion(files, args.output_dir, args.force)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    summary = report["summary"]
    if report["mode"] in {"executable_pilot", "mixed_a2_test_pack"}:
        print(
            f"Generated {summary['missions']} MMM mission Lua file(s): "
            f"{summary['executable_rules']} executable pilot rules and "
            f"{summary['review_required_rules']} disabled review rules."
        )
    else:
        print(
            f"Generated {summary['missions']} MMM Lua scaffold(s) with "
            f"{summary['rules']} review-required rules."
        )
    print(f"Output: {args.output_dir.resolve()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
