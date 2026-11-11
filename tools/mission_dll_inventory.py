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
            continue  # rules (.drl): no engine calls
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
    ("Script state and save/load", r"hot_int|dirty_int|timer_int|\bin\(|\bout\(|load_scripts|save_scripts|linked_list|list_cell|new_script|character|AddToLoad|AddToSave|script::|WORLD"),
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
    "void SetForceOntoMap(int, bool)": "A2.SetForceOntoMap",
    "void SetRealTeam(int, int)": "A2.SetRealTeam",
    "void SetPerceivedTeam(int, int)": "A2.SetPerceivedTeam",
    "int GetPerceivedTeam(int)": "A2.GetPerceivedTeam",
    "void CenterCamera(int)": "A2.CenterCamera",
    "void SetCameraZoom(float)": "A2.SetCameraZoom",
    "void CineractiveConstantSpeed(bool)": "A2.CineractiveConstantSpeed",
    "void CineractiveCameraOffset(float, float, float, bool)": "A2.CineractiveCameraOffset",
    "void CineractiveCameraOffset(char*, float, bool)": "A2.CineractiveCameraOffset",
    "void CineractiveShot(int, int)": "A2.CineractiveShot",
    "void CineractiveShot(char*, int, int)": "A2.CineractiveShot",
    "void CineractiveShot(char*, float, int)": "A2.CineractiveShot",
    "void ObjectivesDisplay_Set_Text_From_File(char*, bool)": "A2.ObjectivesDisplay_Set_Text_From_File",
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a2", required=True)
    ap.add_argument("--a1", required=True)
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
        elif lua_for(name, sig):
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
    L += ["## Summary", "",
          "| | Armada II | Armada I |", "|---|---:|---:|",
          f"| mission DLLs read | {len(a2_dlls)} | {len(a1_dlls)} |",
          f"| scripts calling the engine | {a2_scripts} (`.dsl`; `.drl` rules make no calls) | {len({f for _, fs in a1 for f in fs})} |",
          f"| engine API size | {len(slots)} `ScriptInterface` methods | (imports only) |",
          f"| distinct calls used | {len(a2_used)} | {len(a1)} |",
          f"| call sites | {sum(use.values())} | — |",
          f"| A2 DLL-shaped callable | {a2_cov} | — |",
          f"| missing A2 callable signatures | {a2_missing} | — |",
          f"| MMM job-equivalent, not A2-shaped | {a2_mmm_equiv} | — |",
          f"| bound in STA64's Lua today | — | {a1_cov} |",
          f"| in MMM's API, not yet bound | {sum(1 for _, s_ in a2_used if status(method_name(s_), s_, 'a2') == 'MMM')} | {sum(1 for s_, _ in a1 if status(method_name(s_), s_, 'a1') == 'MMM')} |",
          f"| no Lua equivalent (STA64 extension needed) | {sum(1 for _, s_ in a2_used if status(method_name(s_), s_, 'a2') == 'new')} | {sum(1 for s_, _ in a1 if status(method_name(s_), s_, 'a1') == 'new')} |",
          f"| script-state plumbing (n/a in Lua) | {sum(1 for _, s_ in a2_used if status(method_name(s_), s_, 'a2') == 'n/a')} | {sum(1 for s_, _ in a1 if status(method_name(s_), s_, 'a1') == 'n/a')} |", "",
          "- **bound**: an existing `sta_mission` binding does the same job.",
          "- **A2 DLL-shaped**: the exact normalized ScriptInterface signature has a literal",
          "  production binding registered on the `A2` namespace; this is source-level coverage,",
          "  not proof of native simulation parity.",
          "- **MMM-equivalent**: an existing MMM-style Lua operation does the same job, but",
          "  it is not counted as an A2 DLL-shaped callable.",
          "- **MMM**: Fleet Operations' MMM Lua API ([MMM_API.md](MMM_API.md)) has it (same name, or the",
          "  alias noted), so converted scripts can call it once STA64 binds it.",
          "- **new**: no Lua API has it; STA64's runtime needs an extension (e.g. A1's bridge scenes).",
          "- **n/a**: the DLLs' own variables, timers and save/load plumbing (`hot_int`, `timer_int`,",
          "  `in`/`out`, `linked_list`); in Lua these are plain variables and the save system.", ""]

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
                display = a2_exact_for(sig, a.repo) if api == "a2" else ""
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
    L += ["## Regenerate", "", "```sh",
          "python3 tools/mission_dll_inventory.py --a2 \"$A2\" --a1 \"$A1\" --repo . --out docs/MISSION_DLL_API.md",
          "```"]
    open(a.out, "w").write("\n".join(L) + "\n")
    print(f"A2: {len(a2_dlls)} DLLs, {a2_scripts} scripts, {len(a2_used)}/{len(slots)} methods used, {sum(use.values())} calls; "
          f"A2 callable {a2_cov}, missing {a2_missing}; A1: {len(a1_dlls)} DLLs, {len(a1)} imports, MMM-equivalent {a1_cov}")


if __name__ == "__main__":
    main()
