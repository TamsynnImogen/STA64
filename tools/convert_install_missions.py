#!/usr/bin/env python3
"""Convert an install's own compiled missions into the user's data directory.

Stock Armada II campaign missions are mission DLLs (`*R.drl`, `*S.dsl`) that STA64
never loads (docs/MODDING_DECISIONS.md D2). `tools/dsl_to_lua.py` converts them to
Lua; this wrapper runs it for one install and puts the output where the game
mounts it, so the campaign menu lists and starts those missions (marked
experimental):

    python3 tools/convert_install_missions.py --install "<Armada II root>" [--mod "<mod>"]
    cargo run --release -- --install-root "<Armada II root>" [--mod "<mod>"]

Output folder: `$STA64_DATA_DIR/converted/<key>/` or
`<user data>/sta64/converted/<key>/` (`$XDG_DATA_HOME`, `~/.local/share`,
`%LOCALAPPDATA%`, `~/Library/Application Support`; the same rule as the game's
`platform::data_dir`), where `<key>` is the install folder's name (and the mod's),
lower-cased with other characters as `-`, so each install and mod keeps its own
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
    ap.add_argument("--mod", help="the mod folder the game is started with (--mod), kept apart")
    ap.add_argument("--out", help="output folder instead of the data directory's")
    ap.add_argument("--only", action="append", help="one mission, e.g. a2_fed01 (repeatable)")
    a = ap.parse_args(argv)
    install = os.path.realpath(a.install)
    if not os.path.isdir(install):
        sys.exit(f"--install {a.install}: not a folder")
    installs = [install]
    if a.mod:
        mod_dir = os.path.join(install, a.mod)
        if not os.path.isdir(mod_dir):
            sys.exit(f"--mod {a.mod}: no folder {mod_dir}")
        installs.append(mod_dir)
    out = a.out
    if not out:
        base = data_dir()
        if not base:
            sys.exit("no user data folder known: set STA64_DATA_DIR or pass --out")
        out = os.path.join(base, "converted", key(install, a.mod))
    why = refuse(out, installs)
    if why:
        sys.exit(f"output folder {out} {why}")
    cmd = [sys.executable, "-I", str(HERE / "dsl_to_lua.py"), "--out", out]
    for r in installs:
        cmd += ["--install", r]
    for m in a.only or []:
        cmd += ["--only", m]
    print(f"converting {install} -> {out}", flush=True)
    code = subprocess.call(cmd)
    if code:
        sys.exit(code)
    n = len([f for f in os.listdir(os.path.join(out, "missions")) if f.endswith(".lua")]) if os.path.isdir(os.path.join(out, "missions")) else 0
    print(f"{n} mission scripts in {out}")
    print("Start the game as usual; those missions are listed as experimental.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
