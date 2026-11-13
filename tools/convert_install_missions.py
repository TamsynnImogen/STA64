#!/usr/bin/env python3
"""Convert an install's own compiled missions into the user's data directory.

Stock Armada II campaign missions are mission DLLs (`*R.drl`, `*S.dsl`) that STA64
never loads (docs/MODDING_DECISIONS.md D2). `tools/dsl_to_lua.py` converts them to
Lua; this wrapper runs it for one install and puts the output where the game
mounts it, so the campaign menu lists and starts those missions (marked
experimental):

    python3 tools/convert_install_missions.py --install "<Armada II root>"
    cargo run --release -- --install-root "<Armada II root>"

Output folder: `$STA64_DATA_DIR/converted/<key>/` or
`<user data>/sta64/converted/<key>/` (`$XDG_DATA_HOME`, `~/.local/share`,
`%LOCALAPPDATA%`, `~/Library/Application Support`; the same rule as the game's
`platform::data_dir`), where `<key>` is the install folder's name (and the mod's),
lower-cased with other characters as `-`, so each install keeps its own
conversion (`sta_mission::converted::key`). `--out DIR` overrides the folder (the
game then does not find it). The output is derived from game files: the tool
refuses a folder inside the repository or an install. Needs python3 and `objdump`.
It is a separate step: the game never runs Python.
"""

import argparse
import os
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent


def slug(s):
    out = re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")
    return out or "install"


def key(install, mod=None):
    """`sta_mission::converted::key`: slug(install folder name) [+ "+" + slug(mod)]."""
    k = slug(os.path.basename(os.path.realpath(install)))
    return k + "+" + slug(mod) if mod else k


def data_dir(env=None, system=None):
    """`platform::data_dir` (the user data folder of STA64)."""
    env = os.environ if env is None else env
    system = system or sys.platform

    def var(k):
        return env.get(k) or None

    if var("STA64_DATA_DIR"):
        return var("STA64_DATA_DIR")
    home = var("HOME") or var("USERPROFILE")
    base = var("XDG_DATA_HOME")
    if not base and system.startswith("win"):
        base = var("LOCALAPPDATA")
    if not base and home:
        if system == "darwin":
            base = os.path.join(home, "Library", "Application Support")
        elif system.startswith("win"):
            base = os.path.join(home, "AppData", "Local")
        else:
            base = os.path.join(home, ".local", "share")
    return os.path.join(base, "sta64") if base else None


def is_armada1(install):
    """An Armada I root: Armada.exe and no Armada2.exe (`tools/a1_to_lua.py` converts it)."""
    names = {n.lower() for n in os.listdir(install)}
    return "armada.exe" in names and "armada2.exe" not in names


def inside(path, root):
    path, root = os.path.realpath(path), os.path.realpath(root)
    return path == root or path.startswith(root.rstrip(os.sep) + os.sep)


def refuse(out, installs):
    """Why `out` is refused: derived from game files, so never in the repository (its build folders excepted, for the tests) or an install."""
    builds = [HERE.parent / "target"] + ([os.environ["CARGO_TARGET_DIR"]] if os.environ.get("CARGO_TARGET_DIR") else [])
    if inside(out, HERE.parent) and not any(inside(out, b) for b in builds):
        return "is inside the repository: output is derived from game files"
    for r in installs:
        if inside(out, r):
            return f"is inside the install {r}: installs are read-only"
    return None


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--install", required=True, help="the game install root (read-only)")
    ap.add_argument("--out", help="output folder instead of the data directory's")
    ap.add_argument("--only", action="append", help="one mission, e.g. a2_fed01 (repeatable)")
    a = ap.parse_args(argv)
    install = os.path.realpath(a.install)
    if not os.path.isdir(install):
        sys.exit(f"--install {a.install}: not a folder")
    installs = [install]
    # Only the base game's own missions folder: never a mod's DLLs under Data/Mods (D6).
    mdir = next((os.path.join(install, d, "missions") for d in ("", "Data") if os.path.isdir(os.path.join(install, d, "missions"))), None)
    if not mdir:
        sys.exit(f"--install {a.install}: no missions folder")
    out = a.out
    if not out:
        base = data_dir()
        if not base:
            sys.exit("no user data folder known: set STA64_DATA_DIR or pass --out")
        out = os.path.join(base, "converted", key(install))
    why = refuse(out, installs)
    if why:
        sys.exit(f"output folder {out} {why}")
    if is_armada1(install):
        cmd = [sys.executable, "-I", str(HERE / "a1_to_lua.py"), "--out", out, "--install", install]
    else:
        cmd = [sys.executable, "-I", str(HERE / "dsl_to_lua.py"), "--out", out, "--engine", install, "--install", mdir]
        # The base game's maps name the mission DLLs: each map gets the script under its own stem.
        bdir = os.path.join(os.path.dirname(mdir), "bzn")
        if os.path.isdir(bdir):
            cmd += ["--install", bdir]
    for m in a.only or []:
        cmd += ["--only", m]
    print(f"converting {install} -> {out}", flush=True)
    outm = os.path.join(out, "missions")
    if os.path.isdir(outm):  # a re-run replaces the previous conversion
        for f in os.listdir(outm):
            if f.endswith(".lua"):
                os.remove(os.path.join(outm, f))
    code = subprocess.call(cmd)
    n = len([f for f in os.listdir(outm) if f.endswith(".lua")]) if os.path.isdir(outm) else 0
    if code and not n:
        sys.exit(code)
    if code:
        print(f"warning: some missions failed to convert (see FAIL lines above); the {n} others are usable")
    print(f"{n} mission scripts in {out}")
    print("Start the game as usual; those missions are listed as experimental.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
