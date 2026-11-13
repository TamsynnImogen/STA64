#!/usr/bin/env python3
"""Inventory the engine calls made by Armada I and Armada II mission DLLs.

Stock missions are compiled 32-bit DLLs (docs/MISSIONS.md):

- Armada II: `missions/<name>S.dsl` (the script) and `<name>R.drl` (its
  rules). A `.dsl` exports `g_pSI`, a `ScriptInterface *` the engine fills
  in, and calls the engine only through it: `g_pSI->Method(...)`, a virtual
  call through the vtable of the engine's `ScriptInterfaceImp`. The slot
  order and method names are read from the user's own `Armada2.exe` and
  `armada2.map` (shipped with the game); the DLLs are disassembled with
  `objdump` and every load of `g_pSI` is followed through registers to the
  virtual call it feeds.
- Armada I: `missions/*.dsl|.drl` and `AI/*.dsl|.drl` import engine
  functions by name from `Armada.exe`; their import tables are the API.

Rule DLLs (`R.drl`) are not call-free: their conditions call the engine too. They
import `g_pSI` instead of exporting it, so they are decoded with
tools/drl_to_lua.py and reported in their own section (`--rules-install` adds
further Armada II-family roots, such as a mod that ships `.drl` files).

Writes a Markdown report: every engine call, grouped by area, with how many
scripts use it (and, for A2, how many call sites), and the STA64 Lua
binding that covers it today, if any. Only names and counts are recorded:
no addresses, offsets or code (docs/ARCHITECTURE.md, "Legal boundary").

usage: mission_dll_inventory.py --a2 <A2 install> --a1 <A1 install> --repo . --out docs/MISSION_DLL_API.md
Needs objdump (binutils) and llvm-undname (LLVM) on PATH.
"""

import argparse
import collections
import glob
import os
import re
import struct
import subprocess
import sys


def run(args, data=None):
    return subprocess.run(args, input=data, capture_output=True, text=True, check=False).stdout


def demangle(names):
    """MSVC names to signatures; a name that does not demangle stays as it is."""
    out = run(["llvm-undname"], "\n".join(names) + "\n")
    # One block per input: the name echoed, the signature (absent on failure), a blank line.
    blocks = [b.split("\n") for b in out.strip("\n").split("\n\n")]
    if len(blocks) != len(names):
        sys.exit(f"llvm-undname gave {len(blocks)} results for {len(names)} names")
    return [b[1].strip() if len(b) > 1 and b[1].strip() else n for n, b in zip(names, blocks)]


def find(root, pattern):
    """Case-insensitive glob under root (installs mix case)."""
    rx = re.compile(fnmatch_to_re(pattern), re.I)
    out = []
    for d, _, files in os.walk(root):
        for f in files:
            rel = os.path.relpath(os.path.join(d, f), root).replace("\\", "/")
            if rx.fullmatch(rel):
                out.append(os.path.join(d, f))
    return sorted(out)


def fnmatch_to_re(p):
    return "".join(".*" if c == "*" else "[^/]" if c == "?" else re.escape(c) for c in p)


# ---------------------------------------------------------------- Armada II


def a2_slots(exe, mapf):
    """[(slot, demangled signature)] of ScriptInterfaceImp's vtable."""
    names = {}
    for line in open(mapf, encoding="latin1"):
        m = re.match(r"\s*[0-9a-f]{4}:[0-9a-f]{8}\s+(\S+)\s+([0-9a-f]{8})\s", line)
        if m:
            names.setdefault(int(m.group(2), 16), m.group(1))
    b = open(exe, "rb").read()
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    n = struct.unpack_from("<H", b, pe + 6)[0]
    opt = struct.unpack_from("<H", b, pe + 20)[0]
    base = struct.unpack_from("<I", b, pe + 24 + 28)[0]
    secs = []
    for i in range(n):
        nm, vs, va, rs, ra = struct.unpack_from("<8sIIII", b, pe + 24 + opt + i * 40)
        secs.append((base + va, vs, ra, nm))
    text = next(s for s in secs if s[3].startswith(b".text"))

    def off(va):
        for s, vs, ra, _ in secs:
            if s <= va < s + vs:
                return ra + va - s
        return None

    vt = next(a for a, v in names.items() if v == "??_7ScriptInterfaceImp@@6B@")
    raw = []
    i = 0
    while True:
        if i > 0 and (vt + 4 * i) in names:
            break  # the next symbol starts
        a = struct.unpack_from("<I", b, off(vt) + 4 * i)[0]
        if not text[0] <= a < text[0] + text[1]:
            break
        raw.append(names.get(a, f"?unnamed_{i}@@"))
        i += 1
    return list(enumerate(demangle(raw)))


REG = r"(e[abcd]x|e[sd]i|ebp)"


def a2_usage(slots, dlls):
    use = collections.Counter()
    per = collections.defaultdict(set)
    scripts = 0
    for f in dlls:
        b = open(f, "rb").read()
        pe = struct.unpack_from("<I", b, 0x3C)[0]
        base = struct.unpack_from("<I", b, pe + 24 + 28)[0]
        ex = run(["objdump", "-p", f])
        eat = {int(i): int(r, 16) for i, r in re.findall(r"\[\s*(\d+)\]\s+\+base\[\s*\d+\]\s+([0-9a-f]+)\s+Export RVA", ex)}
        rva = None
        for _, ordinal, name in re.findall(r"\[\s*(\d+)\]\s+\+base\[\s*(\d+)\]\s+[0-9a-f]+\s+(\S+)\s*$", ex, re.M):
            if name.startswith("?g_pSI@@"):
                rva = eat.get(int(ordinal) - 1)
        if rva is None:
            continue  # rules (.drl) import g_pSI: read by a2_rule_conditions
        scripts += 1
        target = f"{base + rva:#x}"
        state = {}
        for line in run(["objdump", "-d", "-M", "intel", "--no-show-raw-insn", f]).splitlines():
            parts = line.split("\t", 1)
            if len(parts) < 2:
                if line.endswith(">:"):
                    state = {}
                continue
            ins = parts[1].strip()
            m = re.match(r"mov\s+" + REG + r",DWORD PTR ds:(0x[0-9a-f]+)", ins)
            if m:
                state[m.group(1)] = "si" if m.group(2) == target else None
                continue
            m = re.match(r"mov\s+" + REG + r",DWORD PTR \[" + REG + r"\]$", ins)
            if m:
                state[m.group(1)] = "vt" if state.get(m.group(2)) == "si" else None
                continue
            m = re.match(r"mov\s+" + REG + "," + REG + "$", ins)
            if m:
                state[m.group(1)] = state.get(m.group(2))
                continue
            m = re.match(r"call\s+DWORD PTR \[" + REG + r"(?:\+(0x[0-9a-f]+))?\]", ins)
            if m:
                if state.get(m.group(1)) == "vt":
                    slot = int(m.group(2) or "0", 16) // 4
                    use[slot] += 1
                    per[slot].add(os.path.basename(f))
                for r in ("eax", "ecx", "edx"):
                    state.pop(r, None)
                continue
            if ins.startswith(("call", "ret")):
                for r in ("eax", "ecx", "edx"):
                    state.pop(r, None)
            m = re.match(r"(?:mov|lea|pop|xor|add|sub|and|or|movzx|movsx|imul|shl|shr|sar|inc|dec|neg|not)\s+" + REG + r"\b", ins)
            if m:
                state.pop(m.group(1), None)
    return use, per, scripts


# ---------------------------------------------------------------- Armada I


def a1_usage(dlls):
    per = collections.defaultdict(set)
    for f in dlls:
        cur = None
        for line in run(["objdump", "-p", f]).splitlines():
            m = re.match(r"\s*DLL Name:\s*(\S+)", line)
            if m:
                cur = m.group(1).lower()
                continue
            m = re.match(r"\s+[0-9a-f]{8}\s+\S+\s+[0-9a-f]{4}\s+(\S+)", line)
            if m and cur == "armada.exe":
                per[m.group(1)].add(os.path.basename(f))
    names = sorted(per)
    return [(d, per[n]) for n, d in zip(names, demangle(names))]


# ---------------------------------------------------------------- report

AREAS = [
    ("Script state and save/load", r"hot_int|dirty_int|timer_int|\bin\(|\bout\(|load_scripts|save_scripts|linked_list|list_cell|new_script|character|AddToLoad|AddToSave|script::|WORLD|\bhot_type::|\bIIIE::"),
    ("Bridge scenes and movies", r"Bridge|Movie|Cinematic"),
    ("Cineractives and camera", r"Cineractive|Camera|FadeIn|FadeOut|Letterbox"),
    ("Titles, text and objectives", r"Title|Text|Objective|AdLog|Subtitle|Message(?!.*Audio)|Hint|Tooltip"),
    ("Audio and music", r"Audio|Music|Sound|Speech|VO\b"),
    ("Orders and AI", r"AI_|Goto|Patrol|Attack|Follow|Escort|Guard|Order|Move|Dock|Warp|Stop|Kamikaze|Formation|Retreat|Repair|Build|Mine|Colonize|Assimilate|Board|Tow|Scan|Explore|Defend|Capture|Speed"),
    ("Resources and crew", r"Dilithium|Latinum|Metal|Biomatter|Crew|Officer|Resource|Supply|Credits"),
    ("Teams, players and relations", r"Team|Player|Ally|Alliance|Relation|Enemy|Race"),
    ("Objects: create, find, query, change", r"Object|Create|Remove|Destroy|Location|Position|Health|Shield|Hull|Alive|Exists|Count|Nearest|Distance|Label|Handle|Cloak|Visible|Select|Damage|Weapon|Target|Die|Invulner|Special"),
]


def area(sig):
    for name, rx in AREAS:
        if re.search(rx, sig):
            return name
    return "Other"


def short(sig, cls=None):
    s = re.sub(r"^(public|private|protected): ", "", sig)
    s = re.sub(r"\b(virtual|__thiscall|__cdecl|__stdcall|class|struct|enum) ", "", s)
    if cls:
        s = s.replace(cls + "::", "")
    return s


def lua_names(repo, module):
    """Names bound in one expected production API module."""
    names = set()
    path = os.path.join(repo, "crates/sta_mission/src/api", module)
    try:
        with open(path, encoding="utf-8") as source:
            src = source.read()
    except OSError:
        return names
    names |= set(re.findall(r'(?:set|create_function|add_method|add_function|add_meta_method|add_field_method_get|add_field_method_set)\(\s*"([A-Za-z_][A-Za-z_0-9]*)"', src))
    return names


# Engine call -> the existing MMM-style Lua binding that already does its job.
# Only listed when the binding exists in sta_mission today.
COVERED = {
    "RemoveObject": ("Entity:remove", "entity.rs", ("remove",)),
    "GetLocation": ("Entity.position", "entity.rs", ("position",)),
    "CraftCannotDie": ("Entity.invincible", "entity.rs", ("invincible",)),
    "SetRelation": ("Player:setRelation", "player.rs", ("setRelation",)),
    "GetRelation": ("Player:getRelation", "player.rs", ("getRelation",)),
    "IsAlive": ("Entity.destroyed / Entity.valid", "entity.rs", ("destroyed", "valid")),
    "Cloak": ("Entity:cloak", "entity.rs", ("cloak",)),
    "Decloak": ("Entity:decloak", "entity.rs", ("decloak",)),
    "GetHull": ("Entity.hull", "entity.rs", ("hull",)),
    "GetShields": ("Entity.shields", "entity.rs", ("shields",)),
}


# Exact Armada II ScriptInterface signatures implemented by the DLL-shaped
# A2 table. MMM equivalents above remain a separate, older classification.
A2_SIGNATURES = {
    "bool IsAlive(int)": "A2.IsAlive",
    "bool IsStation(int)": "A2.IsStation",
    "bool Exists(int)": "A2.Exists",
    "bool DoesNotExist(int)": "A2.DoesNotExist",
    "Vector3 const & GetLocation(int)": "A2.GetLocation",
    "int GetRealTeam(int)": "A2.GetRealTeam",
    "void RemoveObject(int)": "A2.RemoveObject",
    "void CraftCannotDie(int, bool)": "A2.CraftCannotDie",
    "void Cloak(int)": "A2.Cloak",
    "void Decloak(int)": "A2.Decloak",
    "void DisableEngines(int, bool)": "A2.DisableEngines",
    "void DisableWeapons(int, bool)": "A2.DisableWeapons",
    "float GetCurrentHealth(int)": "A2.GetCurrentHealth",
    "float GetMaxHealth(int)": "A2.GetMaxHealth",
    "void SetCurrentHealth(int, float)": "A2.SetCurrentHealth",
    "void SetSpecialEnergy(int, float)": "A2.SetSpecialEnergy",
    "unsigned int RandomGetUnsigned(unsigned int)": "A2.RandomGetUnsigned",
    "bool IsOdf(int, char*)": "A2.IsOdf",
    "float GetDistance(int&, int&)": "A2.GetDistance",
    "bool IsRace(int, char*const)": "A2.IsRace",
    "bool IsVisibleToTeam(int, int)": "A2.IsVisibleToTeam",
    "char const* GetOriginalRace(int)": "A2.GetOriginalRace",
    "void ObjectivesDisplay_Set_Visibility(bool)": "A2.ObjectivesDisplay_Set_Visibility",
    "void ObjectivesDisplaySetObjectiveComplete(int, bool)": "A2.ObjectivesDisplaySetObjectiveComplete",
    # Rule-condition queries (conditions.rs): `A2.paths` serves the path overload.
    "bool HasTimeElapsed(float)": "A2.HasTimeElapsed",
    "bool IsAudioMessageDone(int)": "A2.IsAudioMessageDone",
    "bool IsCargoShipFullAndReadyToSend(int)": "A2.IsCargoShipFullAndReadyToSend",
    "bool IsCargoShipReadyToLoad(int)": "A2.IsCargoShipReadyToLoad",
    "bool IsCompromised(int, int)": "A2.IsCompromised",
    "bool IsExploding(int)": "A2.IsExploding",
    "bool IsMoviePlaying(void)": "A2.IsMoviePlaying",
    "bool IsShipAliveAndOffMap(int)": "A2.IsShipAliveAndOffMap",
    "bool UnderAttack(int, float)": "A2.UnderAttack",
    "float GetCrew(int)": "A2.GetCrew",
    "float GetDistance(int&, char*, int)": "A2.GetDistance",
    "float GetLatinumTraded(int, int)": "A2.GetLatinumTraded",
    "float GetTime(void)": "A2.GetTime",
    "int CountCapitalShips(int)": "A2.CountCapitalShips",
    "int CountStations(int)": "A2.CountStations",
    "int CountUnits(int, char*)": "A2.CountUnits",
    "int CountUnitsNearObject(int, float, int, char*)": "A2.CountUnitsNearObject",
    "int CountVisibleUnits(int, int, bool, char*)": "A2.CountVisibleUnits",
    "int CountVisibleUnitsNearObject(int, float, int, bool, char*)": "A2.CountVisibleUnitsNearObject",
    "int GetDilithium(int)": "A2.GetDilithium",
    "int GetLatinum(int)": "A2.GetLatinum",
    "int GetMetal(int)": "A2.GetMetal",
    # Script-taking presentation calls (native_flow.rs) and titles (native_text.rs).
    "void FadeIn(script *, float, float, float, bool)": "A2.FadeIn",
    "void FadeOut(script *, float, float, float, bool)": "A2.FadeOut",
    "void CineractiveStart(script *, bool)": "A2.CineractiveStart",
    "int AudioMessage(script *, char const *, bool)": "A2.AudioMessage",
    "int PlayMusic(script *, char const *, bool)": "A2.PlayMusic",
    "void StopAudioMessage(int)": "A2.StopAudioMessage",
    "void PlayCinematicMovie(script *, char *, int, bool)": "A2.PlayCinematicMovie",
    "void WaitForEnterKey(script *)": "A2.WaitForEnterKey",
    "int ShowTitle(int, int, char const *, bool)": "A2.ShowTitle",
    "int ShowSubtitle(int, int, char const *, bool)": "A2.ShowSubtitle",
    "void RemoveTitle(int, bool)": "A2.RemoveTitle",
    "void RemoveSubtitle(int, bool)": "A2.RemoveSubtitle",
    "void SetForceOntoMap(int, bool)": "A2.SetForceOntoMap",
    "void SetRealTeam(int, int)": "A2.SetRealTeam",
    "void SetPerceivedTeam(int, int)": "A2.SetPerceivedTeam",
    "int GetPerceivedTeam(int)": "A2.GetPerceivedTeam",
    "void ObjectivesDisplay_Set_Text_From_File(char*, bool)": "A2.ObjectivesDisplay_Set_Text_From_File",
    # Exact presentation requests (native_camera.rs, native_presentation.rs):
    # one queued `Edit::Present` per call, no client consumer yet.
    "void CenterCamera(int)": "A2.CenterCamera",
    "void SetCameraZoom(float)": "A2.SetCameraZoom",
    "void CineractiveConstantSpeed(bool)": "A2.CineractiveConstantSpeed",
    "void CineractiveCameraOffset(float, float, float, bool)": "A2.CineractiveCameraOffset",
    "void CineractiveCameraOffset(char*, float, bool)": "A2.CineractiveCameraOffset",
    "void CineractiveShot(int, int)": "A2.CineractiveShot",
    "void CineractiveShot(char*, int, int)": "A2.CineractiveShot",
    "void CineractiveShot(char*, float, int)": "A2.CineractiveShot",
    "void AddTransporterEffect(int)": "A2.AddTransporterEffect",
    "void RemoveBillboardEffect(int)": "A2.RemoveBillboardEffect",
    "void GridVisible(bool)": "A2.GridVisible",
    "void RemoveFromAdLog(int)": "A2.RemoveFromAdLog",
    "void Starfield_Load_New_Background_Geometry(char*, char*)": "A2.Starfield_Load_New_Background_Geometry",
    "void MinimapMessage(int, int, float)": "A2.MinimapMessage",
    "void MinimapMessage(char*, int, float, int)": "A2.MinimapMessage",
}


# Exact Armada I import signatures implemented by the DLL-named `A1` table
# (crates/sta_mission/src/api/a1/). The key is the demangled signature the
# stock DLLs import from Armada.exe, the value the Lua name. An import counts
# as callable only when the name is also bound by a literal
# `table.set("Name", ...)` in a module registered by api/a1/mod.rs, so this
# is source-level coverage, not proof of native simulation parity.
A1_SIGNATURES = {
    "hot_float strength_team_1": "A1.strength_team_1",
    "hot_float strength_team_2": "A1.strength_team_2",
    "float hot_float::operator float(void)": "A1.hot_float_value",
    "group::group(char *)": "A1.GroupNew",
    "group::~group(void)": "A1.GroupDelete",
    "void Add_Team_To_Group(int, group &)": "A1.Add_Team_To_Group",
    "int Load_Rules(char *)": "A1.Load_Rules",
    "int Load_Action_Scripts(char *)": "A1.Load_Action_Scripts",
    "void AI_Attack(int)": "A1.AI_Attack",
    "void AI_Attack(int, Vector3)": "A1.AI_Attack",
    "void AI_Attack(int, int)": "A1.AI_Attack",
    "void AI_Team_Load_AIP(int, char *)": "A1.AI_Team_Load_AIP",
    "float AddCrew(int, float)": "A1.AddCrew",
    "int AddDilithium(int, int)": "A1.AddDilithium",
    "float AddOfficers(int, float)": "A1.AddOfficers",
    "void AddTransporterEffect(int)": "A1.AddTransporterEffect",
    "void AllowMultipleSpecialWeaponFire(int, bool)": "A1.AllowMultipleSpecialWeaponFire",
    "void Attack(int, Vector3)": "A1.Attack",
    "void Attack(int, int, int)": "A1.Attack",
    "void AttackMainEnemyBase(int, bool)": "A1.AttackMainEnemyBase",
    "int AudioMessage(char const *)": "A1.AudioMessage",
    "void Build(int, char *)": "A1.Build",
    "int BuildObject(char *, int, char *, int)": "A1.BuildObject",
    "int BuildObject(char *, int, int, float, float, float)": "A1.BuildObject",
    "void CenterCamera(char *, int)": "A1.CenterCamera",
    "void CineractiveCameraOffset(char *, float, bool)": "A1.CineractiveCameraOffset",
    "void CineractiveCameraOffset(float, float, float, bool)": "A1.CineractiveCameraOffset",
    "void CineractiveEnd(void)": "A1.CineractiveEnd",
    "void CineractiveShot(char *, float, char *, float)": "A1.CineractiveShot",
    "void CineractiveShot(char *, float, char *, int)": "A1.CineractiveShot",
    "void CineractiveShot(char *, float, int)": "A1.CineractiveShot",
    "void CineractiveShot(char *, int, char *, float)": "A1.CineractiveShot",
    "void CineractiveShot(char *, int, char *, int)": "A1.CineractiveShot",
    "void CineractiveShot(char *, int, int)": "A1.CineractiveShot",
    "void CineractiveShot(int, int)": "A1.CineractiveShot",
    "void CineractiveStart(void)": "A1.CineractiveStart",
    "void ClearRestrictions(int)": "A1.ClearRestrictions",
    "void Cloak(int)": "A1.Cloak",
    "int CountUnits(int, char *)": "A1.CountUnits",
    "int CountUnitsNearObject(int, float, int, char *)": "A1.CountUnitsNearObject",
    "int CountUnitsNearPoint(char *, float, int, char *, int)": "A1.CountUnitsNearPoint",
    "int CountVisibleUnits(int, int, bool, char *)": "A1.CountVisibleUnits",
    "int CountVisibleUnitsNearObject(int, float, int, bool, char *)": "A1.CountVisibleUnitsNearObject",
    "void CraftCannotDie(int, bool)": "A1.CraftCannotDie",
    "int CreateSpecialFlag(int, char *)": "A1.CreateSpecialFlag",
    "void Decloak(int)": "A1.Decloak",
    "void DestroyCritical(int)": "A1.DestroyCritical",
    "void DestroyEngines(int)": "A1.DestroyEngines",
    "void DestroyLifeSupport(int)": "A1.DestroyLifeSupport",
    "void DestroySensors(int)": "A1.DestroySensors",
    "void DestroyShieldGenerator(int)": "A1.DestroyShieldGenerator",
    "void DestroyWeapons(int)": "A1.DestroyWeapons",
    "void DisableAllAITeams(void)": "A1.DisableAllAITeams",
    "void DisableEngines(int, bool)": "A1.DisableEngines",
    "void DisableShieldGenerator(int, bool)": "A1.DisableShieldGenerator",
    "void DisableTeam(int)": "A1.DisableTeam",
    "void DisableWeapons(int, bool)": "A1.DisableWeapons",
    "bool DoesNotExist(int)": "A1.DoesNotExist",
    "void EnableAllAITeams(void)": "A1.EnableAllAITeams",
    "void EnableLight(int, bool)": "A1.EnableLight",
    "void EnableTeam(int)": "A1.EnableTeam",
    "void EnableWormholeTravel(int, bool)": "A1.EnableWormholeTravel",
    "void EndCredits(void)": "A1.EndCredits",
    "bool Exists(int)": "A1.Exists",
    "void FadeIn(float, float, float)": "A1.FadeIn",
    "void FadeOut(float, float, float)": "A1.FadeOut",
    "void FadeRemove(void)": "A1.FadeRemove",
    "void FailMission(float, char *)": "A1.FailMission",
    "int GetAllyShipAttacks(int, int)": "A1.GetAllyShipAttacks",
    "int GetAllyStationAttacks(int, int)": "A1.GetAllyStationAttacks",
    "Vector3 GetAttackLocation(int, int)": "A1.GetAttackLocation",
    "float GetCrew(int)": "A1.GetCrew",
    "float GetDilithiumOnFreighter(int)": "A1.GetDilithiumOnFreighter",
    "float GetDistance(int &, char *, int)": "A1.GetDistance",
    "float GetDistance(int &, int &)": "A1.GetDistance",
    "int GetHandle(char *)": "A1.GetHandle",
    "Vector3 const & GetLocation(char *, int)": "A1.GetLocation",
    "Vector3 const & GetLocation(int)": "A1.GetLocation",
    "int GetLocationOfFlag(int)": "A1.GetLocationOfFlag",
    "int GetNearestEnemy(int, float)": "A1.GetNearestEnemy",
    "int GetNearestObjectNearMe(int, float, int, char *, int, bool)": "A1.GetNearestObjectNearMe",
    "int GetNearestObjectNearPosition(Vector3 const &, float, int, char *, int, bool, int)": "A1.GetNearestObjectNearPosition",
    "int GetNearestStarbase(int, int)": "A1.GetNearestStarbase",
    "int GetNearestTranswarpGate(int)": "A1.GetNearestTranswarpGate",
    "int GetPercievedTeamOfFlag(int)": "A1.GetPercievedTeamOfFlag",
    "float GetRealShieldPercent(int)": "A1.GetRealShieldPercent",
    "int GetRealTeam(int)": "A1.GetRealTeam",
    "int GetRealTeamOfFlag(int)": "A1.GetRealTeamOfFlag",
    "float GetShieldPercent(int)": "A1.GetShieldPercent",
    "float GetTime(void)": "A1.GetTime",
    "int GetTranswarpExitWormhole(int)": "A1.GetTranswarpExitWormhole",
    "void Goto(int, char *, int)": "A1.Goto",
    "void Goto(int, int, int)": "A1.Goto",
    "void GridVisible(bool)": "A1.GridVisible",
    "void ImmediateCloak(int)": "A1.ImmediateCloak",
    "bool InAttackMode(int)": "A1.InAttackMode",
    "bool InQueue(int)": "A1.InQueue",
    "bool InStopMode(int)": "A1.InStopMode",
    "bool IsAlive(int)": "A1.IsAlive",
    "bool IsAudioMessageDone(int)": "A1.IsAudioMessageDone",
    "bool IsCompromised(int, int)": "A1.IsCompromised",
    "bool IsDerelict(int)": "A1.IsDerelict",
    "bool IsExploding(int)": "A1.IsExploding",
    "bool IsMoviePlaying(void)": "A1.IsMoviePlaying",
    "bool IsThereARomulanSpyOnShip(int)": "A1.IsThereARomulanSpyOnShip",
    "bool IsValid(int)": "A1.IsValid",
    "bool IsVisibleToTeam(int, int)": "A1.IsVisibleToTeam",
    "void LinkWormHoles(int, int)": "A1.LinkWormHoles",
    "void Mine(int, int)": "A1.Mine",
    "void MinimapMessage(char *, int, float, int)": "A1.MinimapMessage",
    "void MinimapMessage(int, int, float)": "A1.MinimapMessage",
    "void ObjectivesDisplay_Set_Text_From_File(char *)": "A1.ObjectivesDisplay_Set_Text_From_File",
    "void ObjectivesDisplay_Set_Visibility(bool)": "A1.ObjectivesDisplay_Set_Visibility",
    "void ObstacleAvoidence(bool)": "A1.ObstacleAvoidence",
    "bool OutOfSpecialAttackMode(int)": "A1.OutOfSpecialAttackMode",
    "void Patrol(int, char *)": "A1.Patrol",
    "void PauseSimulation(void)": "A1.PauseSimulation",
    "void PlayBridgeMovie(char *, int)": "A1.PlayBridgeMovie",
    "void PointAt(int, char *)": "A1.PointAt",
    "void PointAt(int, int)": "A1.PointAt",
    "void PreLoadBridgeScene(BridgeDisplay::BridgeType)": "A1.PreLoadBridgeScene",
    "bool RecruitSpecialForces(char *, int, int &)": "A1.RecruitSpecialForces",
    "void RemoveAllCraft(bool, int)": "A1.RemoveAllCraft",
    "void RemoveAllObjects(void)": "A1.RemoveAllObjects",
    "void RemoveBillboardEffect(int)": "A1.RemoveBillboardEffect",
    "void RemoveObject(int)": "A1.RemoveObject",
    "void RemoveSubtitle(int, bool)": "A1.RemoveSubtitle",
    "void RemoveTitle(int, bool)": "A1.RemoveTitle",
    "int ReplaceObject(int, char *)": "A1.ReplaceObject",
    "void ResetWarpSpeed(int)": "A1.ResetWarpSpeed",
    "void RestrictSpecialFlag(int, SpecialFlag::Restriction)": "A1.RestrictSpecialFlag",
    "void RestrictSpecialFlag(int, SpecialFlag::Restriction, int)": "A1.RestrictSpecialFlag",
    "void SearchAndDestroy(int)": "A1.SearchAndDestroy",
    "void SetBridgeSceneRace(BridgeDisplay::BridgeType)": "A1.SetBridgeSceneRace",
    "void SetCrew(int, float)": "A1.SetCrew",
    "int SetDilithium(int, int)": "A1.SetDilithium",
    "void SetGreenAlert(int)": "A1.SetGreenAlert",
    "void SetImpulseSpeed(int, float)": "A1.SetImpulseSpeed",
    "void SetIsAITeam(int const &, bool const &)": "A1.SetIsAITeam",
    "void SetMaxShields(int, float)": "A1.SetMaxShields",
    "float SetOfficers(int, float)": "A1.SetOfficers",
    "void SetPerceivedTeam(int, int)": "A1.SetPerceivedTeam",
    "void SetPosition(int, char *)": "A1.SetPosition",
    "void SetRace(int, char const *)": "A1.SetRace",
    "void SetRealTeam(int, int)": "A1.SetRealTeam",
    "void SetRedAlert(int)": "A1.SetRedAlert",
    "void SetRelation(int, int, Team::TeamRelation)": "A1.SetRelation",
    "void SetShieldPercent(int, float)": "A1.SetShieldPercent",
    "void SetSpecialEnergy(int, float)": "A1.SetSpecialEnergy",
    "void SetSpecialForcesFlag(int, bool)": "A1.SetSpecialForcesFlag",
    "void SetSpecialWeaponAttackInterval(int, float)": "A1.SetSpecialWeaponAttackInterval",
    "float SetTeamCrew(int, float)": "A1.SetTeamCrew",
    "void SetWarpSpeed(int, float)": "A1.SetWarpSpeed",
    "void SetWormholeTeam(int, int)": "A1.SetWormholeTeam",
    "void SetYellowAlert(int)": "A1.SetYellowAlert",
    "void ShowBridgeOnScreen(bool)": "A1.ShowBridgeOnScreen",
    "void ShowBridgeScene(bool)": "A1.ShowBridgeScene",
    "int ShowSubtitle(int, int, int, int, char const *, bool)": "A1.ShowSubtitle",
    "int ShowTitle(int, int, int, int, char const *, bool)": "A1.ShowTitle",
    "void Special_Attack(int, Vector3, char const *)": "A1.Special_Attack",
    "void Special_Attack(int, char const *)": "A1.Special_Attack",
    "void Special_Attack(int, int, char const *)": "A1.Special_Attack",
    "void Starfield_Load_Background_Geometry(char *)": "A1.Starfield_Load_Background_Geometry",
    "void StartCredits(void)": "A1.StartCredits",
    "void Stop(int, int)": "A1.Stop",
    "void StopAllShips(int, char const *, bool, bool)": "A1.StopAllShips",
    "void StopAudioMessage(int)": "A1.StopAudioMessage",
    "void StopMovie(void)": "A1.StopMovie",
    "void SucceedMission(float, char *)": "A1.SucceedMission",
    "void TransferFlag(int, int)": "A1.TransferFlag",
    "bool UnderAttack(int, float)": "A1.UnderAttack",
    "void UnpauseSimulation(void)": "A1.UnpauseSimulation",
}


def normalized_signature(sig):
    """Normalize demangled decoration while retaining the full call shape."""
    sig = re.sub(r"^(?:public|private|protected):\s*", "", sig.strip())
    sig = re.sub(r"\b(?:virtual|__thiscall|__cdecl|__stdcall|class|struct|enum)\s+", "", sig)
    sig = re.sub(r"\bScriptInterfaceImp::", "", sig)
    sig = re.sub(r"\s+", " ", sig)
    sig = re.sub(r"\s*([(),*&])\s*", r"\1", sig)
    return sig


def a2_source_names(repo):
    """Literal A2 table.set names in registered production api/a2 modules."""
    base = os.path.join(repo, "crates/sta_mission/src/api/a2")
    try:
        with open(os.path.join(base, "mod.rs"), encoding="utf-8") as source:
            registration = source.read()
    except OSError:
        return set()
    modules = re.findall(
        r"\b([A-Za-z_][A-Za-z_0-9]*)::install\s*\(\s*lua\s*,\s*ctx\s*,\s*&a2\s*\)",
        registration,
    )
    if not modules or not re.search(r'raw_set\s*\(\s*"A2"\s*,\s*a2\s*\)', registration):
        return set()
    names = set()
    for module in modules:
        path = os.path.join(base, f"{module}.rs")
        if not os.path.isfile(path):
            continue
        with open(path, encoding="utf-8") as source:
            src = source.read()
        names |= set(
            re.findall(
                r'(?m)^\s*table\s*\.\s*set\s*\(\s*"([A-Za-z_][A-Za-z_0-9]*)"\s*,',
                src,
            )
        )
    return names


def a1_source_names(repo):
    """Literal A1 table.set names in registered production api/a1 modules."""
    base = os.path.join(repo, "crates/sta_mission/src/api/a1")
    try:
        with open(os.path.join(base, "mod.rs"), encoding="utf-8") as source:
            registration = source.read()
    except OSError:
        return set()
    modules = re.findall(
        r"\b([A-Za-z_][A-Za-z_0-9]*)::install\s*\(\s*lua\s*,\s*&env\s*,",
        registration,
    )
    if not modules or not re.search(r'raw_set\s*\(\s*"A1"\s*,\s*table\s*\)', registration):
        return set()
    names = set()
    for module in modules:
        path = os.path.join(base, f"{module}.rs")
        if not os.path.isfile(path):
            continue
        with open(path, encoding="utf-8") as source:
            src = source.read()
        # A commented-out registration is not a binding.
        src = "\n".join(line for line in src.splitlines() if not line.lstrip().startswith("//"))
        names |= set(
            re.findall(
                r'(?m)^\s*table\s*\.\s*set\s*\(\s*"([A-Za-z_][A-Za-z_0-9]*)"\s*,',
                src,
            )
        )
    return names


def a1_kinds(repo):
    """name -> 'retained' | 'error' | 'presentation' | 'working', from the A1 sources.

    `retained`: bound through `gap_write` or `.gap("Name"` (the request is kept and
    counted, nothing happens). `error`: bound through `no_data` / `no_order_state`
    (always raises). `presentation`: bound in presentation.rs and only queues an
    event (no consumer yet). Everything else is `working`. Mixed names (an
    implemented overload beside a retained one) stay `working` (A1_PARTIAL).
    """
    base = os.path.join(repo, "crates/sta_mission/src/api/a1")
    kinds = {}
    sources = {}
    for module in ("objects", "orders", "teams", "presentation", "seams"):
        try:
            with open(os.path.join(base, f"{module}.rs"), encoding="utf-8") as source:
                text = source.read()
        except OSError:
            continue
        sources[module] = "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("//"))
    retained = set()
    errors = set()
    for text in sources.values():
        retained |= set(re.findall(r'gap_write\(\s*lua,\s*env,\s*"(\w+)"', text))
        retained |= set(re.findall(r'\.gap\(\s*"(\w+)"', text))
        errors |= set(re.findall(r'(?:no_data|no_order_state)\(\s*lua,\s*env,\s*"(\w+)"', text))
        errors |= set(re.findall(r'\.no_data\(\s*"(\w+)"', text))
    retained -= A1_PARTIAL
    errors -= A1_PARTIAL
    queries = {"IsMoviePlaying", "IsAudioMessageDone"}
    presentation = {
        n for n in re.findall(r'(?m)^\s*table\s*\.\s*set\s*\(\s*"(\w+)"', sources.get("presentation", ""))
        if not n.startswith("ObjectivesDisplay_") and n not in queries
    }
    for name in a1_source_names(repo):
        kinds[name] = (
            "error" if name in errors and name not in retained else
            "retained" if name in retained else
            "presentation" if name in presentation else
            "working"
        )
    return kinds


# Names with an implemented overload beside a retained one: counted as working.
A1_PARTIAL = {"Attack", "GetCrew"}


def a2_raising(repo):
    """Names of A2 calls bound only to raise-and-count (no data behind them)."""
    try:
        with open(os.path.join(repo, "crates/sta_mission/src/api/a2/conditions.rs"), encoding="utf-8") as f:
            src = "\n".join(l for l in f.read().splitlines() if not l.lstrip().startswith("//"))
    except OSError:
        return set()
    return set(re.findall(r'no_data_fn\(\s*lua,\s*ctx,\s*"(\w+)"', src))


def param_count(sig):
    """Number of parameters in a normalized signature (`(void)` is none)."""
    inner = sig[sig.index("(") + 1 : sig.rindex(")")]
    return 0 if inner in ("", "void") else inner.count(",") + 1


# How a bound rule-condition call behaves, in report order.
KINDS = ("working", "raises", "missing")


def a2_condition_kind(sig, repo, raising):
    lua = a2_exact_for(sig, repo)
    if not lua:
        return "missing"
    return "raises" if lua.split(".", 1)[1] in raising else "working"


def a1_condition_kind(sig, repo, kinds):
    lua = a1_exact_for(sig, repo)
    if not lua:
        return "missing"
    return "working" if kinds.get(lua.split(".", 1)[1], "working") in ("working", "presentation") else "raises"


def a2_rule_conditions(roots, engine_root):
    """Per A2-family install: the engine calls its rule DLLs make from conditions.

    Decodes every `R.drl` with tools/drl_to_lua.py (conditions only: actions are one
    `.dsl` function each). Returns [{label, drls, rules, decoded, failed, calls}] with
    `calls = {normalized signature: (set of DLL names, condition sites)}`."""
    import drl_to_lua as drl

    engine = drl.engine_vtables(engine_root)
    if not engine:
        sys.exit("rule conditions need Armada2.exe and armada2.map in the A2 install")
    out = []
    for root in roots:
        files = drl.find_drls(root)
        info = {"label": os.path.basename(os.path.normpath(root)), "drls": len(files), "rules": 0,
                "decoded": 0, "failed": 0, "calls": {}}
        per_file = []
        for f in files:
            try:
                r = drl.decode(f, engine)
            except drl.DecodeError:
                info["failed"] += 1
                continue
            per_file.append((os.path.basename(f), r))
        mangled = sorted({sig for _, r in per_file for sig in r["call_signatures"]})
        norm = dict(zip(mangled, (normalized_signature(x) for x in demangle(mangled)))) if mangled else {}
        for name, r in per_file:
            info["rules"] += len(r["rules"])
            info["decoded"] += sum(1 for x in r["rules"] if x.get("condition_lua") is not None)
            for sig in r["call_signatures"]:
                n = norm[sig]
                key = f"{method_name(n)}/{param_count(n)}"
                files_, sites = info["calls"].get(n, (set(), 0))
                files_.add(name)
                info["calls"][n] = (files_, sites + r["call_sites"].get(key, 0))
        out.append(info)
    return out


def a1_rule_conditions(a1_root):
    """The engine calls Armada I's mission rule DLLs import (their conditions: the
    actions live in the `.dsl`). Rule-engine plumbing is left out. AI rule DLLs
    (`AI/*.drl`) import none and are not counted."""
    files = find(a1_root, "missions/*.drl")
    return len(files), [(sig, fs) for sig, fs in a1_usage(files) if area(sig) != "Script state and save/load"]


def a1_exact_for(sig, repo):
    """Return the A1 Lua name only for an exact import signature and source binding."""
    key = normalized_signature(sig)
    lua_name = next(
        (name for signature, name in A1_SIGNATURES.items() if normalized_signature(signature) == key),
        "",
    )
    if not lua_name or lua_name.split(".", 1)[1] not in a1_source_names(repo):
        return ""
    return lua_name


def a2_exact_for(sig, repo):
    """Return the A2 Lua name only for an exact signature and source binding."""
    key = normalized_signature(sig)
    lua_name = next(
        (name for signature, name in A2_SIGNATURES.items() if normalized_signature(signature) == key),
        "",
    )
    if not lua_name or lua_name.split(".", 1)[1] not in a2_source_names(repo):
        return ""
    return lua_name


def covered_for(name, repo, sig=""):
    """Return a verified existing MMM-style binding for the right module."""
    covered = COVERED.get(name)
    if not covered:
        return ""
    if name == "GetLocation" and normalized_signature(sig) != normalized_signature(
        "Vector3 const & GetLocation(int)"
    ):
        return ""
    display, module, expected_names = covered
    bound = lua_names(repo, module)
    if all(expected in bound for expected in expected_names):
        return display
    return ""


def mmm_names(repo):
    """lower-case name -> `Scope.name` for every name in docs/MMM_API.md."""
    out = {}
    scope = ""
    try:
        text = open(os.path.join(repo, "docs/MMM_API.md"), encoding="utf-8").read()
    except OSError:
        return out
    for line in text.splitlines():
        m = re.match(r"### `([^`]+)`", line)
        if m:
            scope = m.group(1)
            continue
        if re.match(r"(Functions|Methods|Properties|Values|Free functions):", line):
            for n in re.findall(r"`([A-Za-z_][A-Za-z_0-9]*)`", line):
                out.setdefault(n.lower(), f"{scope}.{n}" if scope and scope != "(global)" else n)
    return out


# DLL call -> MMM name, where the name differs but the job is the same
# (checked by hand against MMM_API.md).
MMM_ALIAS = {
    "SetRedAlert": "AlertState.RedAlert",
    "SetYellowAlert": "AlertState.YellowAlert",
    "SetGreenAlert": "AlertState.GreenAlert",
    "Goto": "missionGotoPath",
    "Patrol": "missionPatrolPath",
    "AudioMessage": "playSound",
    "StopAudioMessage": "stopSounds",
    "AddTitle": "addTitle",
    "AddSubtitle": "addSubtitle",
    "CineractiveCameraOffset": "_Cineractive.setCameraOffset",
    "AddDilithium": "GiveResource",
    "AddLatinum": "GiveResource",
    "AddMetal": "GiveResource",
    "AddBiomatter": "GiveResource",
}


def method_name(sig):
    m = re.search(r"(\w+)\(", sig.split("::")[-1])
    return m.group(1) if m else sig


def rule_section(a2_rules, a1_files, a1_calls, repo, kinds):
    """The report section on calls made from rule conditions (`.drl`)."""
    raising = a2_raising(repo)
    L = ["## Rule DLLs (`.drl`): calls made from rule conditions", "",
         "A rule DLL is a set of conditions over the script's variables; each rule runs one",
         "action of the matching `.dsl`. The conditions call the engine: they are not",
         "call-free. A **pair** is one rule DLL calling one distinct engine call; a **site** is",
         "one call written in a decoded condition (Armada II only: Armada I's release-build",
         "rule DLLs are read from their imports, which carry no sites). A call **resolves**",
         "when its exact signature is bound on the `A2` / `A1` table, and is **working** when",
         "the binding returns a value from the world view or the mission's own state; **raises**",
         "means it is bound but raises and is counted in `unsupported()` for lack of data in the",
         "sim seam (a missing seam, never a guessed value). Armada I's AI rule DLLs (`AI/*.drl`)",
         "import no engine call.", "",
         "| install | rule DLLs | rules | conditions decoded | distinct calls | pairs | pairs working | pairs raising | pairs missing | sites | sites working |",
         "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for inst in a2_rules:
        pairs = collections.Counter()
        sites = collections.Counter()
        for sig, (fs, n) in inst["calls"].items():
            k = a2_condition_kind(sig, repo, raising)
            pairs[k] += len(fs)
            sites[k] += n
        L.append(f"| {inst['label']} | {inst['drls']} | {inst['rules']} | {inst['decoded']} | {len(inst['calls'])} | "
                 f"{sum(pairs.values())} | {pairs['working']} | {pairs['raises']} | {pairs['missing']} | "
                 f"{sum(sites.values())} | {sites['working']} |")
    pairs = collections.Counter()
    for sig, fs in a1_calls:
        pairs[a1_condition_kind(sig, repo, kinds)] += len(fs)
    L.append(f"| Star Trek Armada (A1 missions) | {a1_files} | — | — | {len(a1_calls)} | {sum(pairs.values())} | {pairs['working']} | "
             f"{pairs['raises']} | {pairs['missing']} | — | — |")
    L.append("")
    if a2_rules:
        first = a2_rules[0]
        L += [f"### Armada II: conditions of `{first['label']}`", "",
              "| call | rule DLLs | sites | status | Lua name |", "|---|---:|---:|:-:|---|"]
        for sig, (fs, n) in sorted(first["calls"].items(), key=lambda kv: (-kv[1][1], kv[0])):
            lua = a2_exact_for(sig, repo)
            L.append(f"| `{short(sig)}` | {len(fs)} | {n} | {a2_condition_kind(sig, repo, raising)} | {'`' + lua + '`' if lua else ''} |")
        L.append("")
        for other in a2_rules[1:]:
            same = {k: (sorted(v[0]), v[1]) for k, v in other["calls"].items()} == {k: (sorted(v[0]), v[1]) for k, v in first["calls"].items()}
            L.append(f"`{other['label']}` has {'the same calls, DLL counts and sites' if same else 'a different rule set (see the summary row)'}.")
        L.append("")
    L += ["### Armada I: conditions of the mission rule DLLs", "",
          "| call | rule DLLs | status | Lua name |", "|---|---:|:-:|---|"]
    for sig, fs in sorted(a1_calls, key=lambda r: (-len(r[1]), r[0])):
        lua = a1_exact_for(sig, repo)
        L.append(f"| `{short(sig).replace('|', chr(92) + '|')}` | {len(fs)} | {a1_condition_kind(sig, repo, kinds)} | {'`' + lua + '`' if lua else ''} |")
    L.append("")
    return L


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a2", required=True)
    ap.add_argument("--a1", required=True)
    ap.add_argument("--rules-install", action="append", default=[],
                    help="further Armada II-family root whose R.drl rule DLLs are read (repeatable)")
    ap.add_argument("--repo", default=".")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    exe = (find(a.a2, "Armada2.exe") or [None])[0]
    mapf = (find(a.a2, "armada2.map") or [None])[0]
    if not exe or not mapf:
        sys.exit("A2 install needs Armada2.exe and armada2.map")
    slots = a2_slots(exe, mapf)
    a2_dlls = find(a.a2, "missions/*.dsl") + find(a.a2, "missions/*.drl")
    use, per, a2_scripts = a2_usage(slots, a2_dlls)
    a1_dlls = find(a.a1, "missions/*.dsl") + find(a.a1, "missions/*.drl") + find(a.a1, "AI/*.dsl") + find(a.a1, "AI/*.drl")
    a1 = a1_usage(a1_dlls)
    mmm = mmm_names(a.repo)
    a2_roots = [a.a2] + [r for r in a.rules_install if os.path.realpath(r) != os.path.realpath(a.a2)]
    a2_rules = a2_rule_conditions(a2_roots, a.a2)
    a1_rule_files, a1_rule_calls = a1_rule_conditions(a.a1)

    def mmm_for(name):
        if name in MMM_ALIAS:
            alias = MMM_ALIAS[name]
            return mmm.get(alias.split(".")[-1].lower(), alias)
        return mmm.get(name.lower(), "")

    def lua_for(name, sig="", repo=a.repo):
        return covered_for(name, repo, sig)

    def status(name, sig="", api="a1"):
        if sig and area(sig) == "Script state and save/load":
            return "n/a"
        if api == "a2":
            exact = a2_exact_for(sig, a.repo)
            if exact:
                return "A2 DLL-shaped"
            if lua_for(name, sig):
                return "MMM-equivalent"
        else:
            if a1_exact_for(sig, a.repo):
                return "A1 callable"
            if lua_for(name, sig):
                return "bound"
        if mmm_for(name):
            return "MMM"
        return "new"

    L = ["# Mission DLL engine calls (Armada I and II)", "",
         "What the stock campaign missions ask of the engine, so the Lua mission",
         "runtime can provide it (docs/MISSIONS.md). Generated by",
         "`tools/mission_dll_inventory.py` from the installs; do not edit by hand.",
         "Names and counts only: the tool reads slot order and addresses from the",
         "user's own executables each time, and none are recorded here.", ""]
    a2_used = [(i, s) for i, s in slots if use.get(i)]
    a2_cov = sum(1 for _, s in a2_used if a2_exact_for(s, a.repo))
    a2_missing = sum(
        1 for _, s in a2_used
        if area(s) != "Script state and save/load" and not a2_exact_for(s, a.repo)
    )
    a2_mmm_equiv = sum(
        1 for _, s in a2_used
        if not a2_exact_for(s, a.repo) and lua_for(method_name(s), s)
    )
    a1_cov = sum(1 for s, _ in a1 if lua_for(method_name(s), s))
    a1_callable = sum(1 for s, _ in a1 if a1_exact_for(s, a.repo))
    kinds = a1_kinds(a.repo)
    a1_by_kind = collections.Counter(
        kinds.get(a1_exact_for(s, a.repo).split(".", 1)[-1], "working")
        for s, _ in a1 if a1_exact_for(s, a.repo)
    )
    a1_missing = sum(
        1 for s, _ in a1
        if area(s) != "Script state and save/load" and not a1_exact_for(s, a.repo)
    )
    L += ["## Summary", "",
          "| | Armada II | Armada I |", "|---|---:|---:|",
          f"| mission DLLs read | {len(a2_dlls)} | {len(a1_dlls)} |",
          f"| scripts calling the engine | {a2_scripts} (`.dsl`; the `.drl` rule conditions are counted in their own section) | {len({f for _, fs in a1 for f in fs})} |",
          f"| engine API size | {len(slots)} `ScriptInterface` methods | (imports only) |",
          f"| distinct calls used | {len(a2_used)} | {len(a1)} |",
          f"| call sites | {sum(use.values())} | — |",
          f"| DLL-shaped callable (`A2` / `A1` table) | {a2_cov} | {a1_callable} |",
          f"| missing callable signatures (missing A1 calls) | {a2_missing} | {a1_missing} |",
          f"| &nbsp;&nbsp;of the A1 callable: working (sim- or view-backed) | — | {a1_by_kind['working']} |",
          f"| &nbsp;&nbsp;of the A1 callable: presentation event only, no consumer yet | — | {a1_by_kind['presentation']} |",
          f"| &nbsp;&nbsp;of the A1 callable: request retained and counted, no effect | — | {a1_by_kind['retained']} |",
          f"| &nbsp;&nbsp;of the A1 callable: always raises (no data in the world view) | — | {a1_by_kind['error']} |",
          f"| MMM job-equivalent, not A2-shaped | {a2_mmm_equiv} | — |",
          f"| bound in STA64's Lua today | — | {a1_cov} |",
          f"| in MMM's API, not yet bound | {sum(1 for _, s_ in a2_used if status(method_name(s_), s_, 'a2') == 'MMM')} | {sum(1 for s_, _ in a1 if status(method_name(s_), s_, 'a1') == 'MMM')} |",
          f"| no Lua equivalent (STA64 extension needed) | {sum(1 for _, s_ in a2_used if status(method_name(s_), s_, 'a2') == 'new')} | {sum(1 for s_, _ in a1 if status(method_name(s_), s_, 'a1') == 'new')} |",
          f"| script-state plumbing (n/a in Lua) | {sum(1 for _, s_ in a2_used if status(method_name(s_), s_, 'a2') == 'n/a')} | {sum(1 for s_, _ in a1 if status(method_name(s_), s_, 'a1') == 'n/a')} |", "",
          "- **bound**: an existing `sta_mission` binding does the same job.",
          "- **A2 DLL-shaped**: the exact normalized ScriptInterface signature has a literal",
          "  production binding registered on the `A2` namespace; this is source-level coverage,",
          "  not proof of native simulation parity.",
          "- **A1 callable**: the exact imported signature has a literal production binding",
          "  registered on the `A1` namespace (crates/sta_mission/src/api/a1/); same source-level",
          "  coverage only: it does not mean the call changes the simulation. The rows below it",
          "  split the callable ones into working, event-only, retained-without-effect and",
          "  always-raises (classified from the A1 sources; see that module's README).",
          "- **MMM-equivalent**: an existing MMM-style Lua operation does the same job, but",
          "  it is not counted as an A2 DLL-shaped callable.",
          "- **MMM**: Fleet Operations' MMM Lua API ([MMM_API.md](MMM_API.md)) has it (same name, or the",
          "  alias noted), so converted scripts can call it once STA64 binds it.",
          "- **new**: no Lua API has it; STA64's runtime needs an extension (e.g. A1's bridge scenes).",
          "- **n/a**: the DLLs' own variables, timers, save/load plumbing and rule/group",
          "  machinery (`hot_int`, `hot_float`, `timer_int`, `in`/`out`, `linked_list`,",
          "  `IIIE::create_rule`, `group`, `Load_Rules`); in Lua these are plain variables, monitors,",
          "  tables and the save system.", ""]

    def table(title, rows, has_calls):
        out = [f"## {title}", ""]
        groups = collections.defaultdict(list)
        for r in rows:
            groups[area(r[0])].append(r)
        order = [n for n, _ in AREAS] + ["Other"]
        for g in order:
            if g not in groups:
                continue
            items = sorted(groups[g], key=lambda r: (-r[1], r[0]))
            out += [f"### {g} ({len(items)})", ""]
            out += ["| call | scripts |" + (" call sites |" if has_calls else "") + " status | Lua name |",
                    "|---|---:|" + ("---:|" if has_calls else "") + ":-:|---|"]
            for r in items:
                sig, nscripts = r[0], r[1]
                cells = [f"`{sig.replace('|', '\\|')}`", str(nscripts)]
                if has_calls:
                    cells.append(str(r[2]))
                n = method_name(sig)
                api = "a2" if has_calls else "a1"
                cells.append(status(n, sig, api))
                display = (a2_exact_for(sig, a.repo) if api == "a2" else a1_exact_for(sig, a.repo))
                display = display or lua_for(n, sig) or mmm_for(n)
                cells.append(f"`{display}`" if display else "")
                out.append("| " + " | ".join(cells) + " |")
            out.append("")
        return out

    L += table("Armada II: `ScriptInterface` methods the scripts call",
               [(short(s, "ScriptInterfaceImp"), len(per[i]), use[i]) for i, s in a2_used], True)
    L += table("Armada I: engine functions the mission DLLs import",
               [(short(s), len(fs)) for s, fs in a1], False)
    unused = [short(s, "ScriptInterfaceImp") for i, s in slots if not use.get(i)]
    L += ["## Armada II methods no stock mission calls", "",
          f"{len(unused)} of the interface's {len(slots)} methods. Mods and community missions may still use them.", "",
          ", ".join(f"`{u.split('(')[0]}`" for u in sorted(set(unused))), ""]
    L += rule_section(a2_rules, a1_rule_files, a1_rule_calls, a.repo, kinds)
    L += ["## Regenerate", "", "```sh",
          "python3 tools/mission_dll_inventory.py --a2 \"$A2\" --a1 \"$A1\" --repo . --out docs/MISSION_DLL_API.md \\",
          "  [--rules-install \"$A2_STOCK\" --rules-install \"$MOD_WITH_DRL\"]",
          "```", "",
          "Each `--rules-install` adds one Armada II-family root whose `R.drl` files are decoded",
          "with `tools/drl_to_lua.py`; the engine slot names come from `$A2`."]
    open(a.out, "w").write("\n".join(L) + "\n")
    print(f"A2: {len(a2_dlls)} DLLs, {a2_scripts} scripts, {len(a2_used)}/{len(slots)} methods used, {sum(use.values())} calls; "
          f"A2 callable {a2_cov}, missing {a2_missing}; A1: {len(a1_dlls)} DLLs, {len(a1)} imports, "
          f"A1 callable {a1_callable} ({dict(a1_by_kind)}), missing {a1_missing}, MMM-equivalent {a1_cov}")


if __name__ == "__main__":
    main()
