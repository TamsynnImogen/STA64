#!/usr/bin/env python3
"""Write the Python converters' reference output for installs, for the Rust port's parity harness.

    python3 tools/convert_reference.py --out <folder outside the repository> \\
        --install "<Armada I or II root>" [--install ...] [--objdump-listings]

For each install (Armada I or Armada II, detected as `convert_install_missions.py`
does) it runs the converter exactly as that wrapper does and writes, under
`<out>/<install folder slug>/`:

    missions/*.lua   `a1_to_lua.py` or `dsl_to_lua.py` output
    drl/<...>/*.lua  `drl_to_lua.py` scaffolds (Armada II only)
    log.txt          the converters' console output, in order
    versions.txt     python3 and objdump versions (the reference is only as stable as these)
    listings/*.txt   with `--objdump-listings`: the objdump text each mission DLL is read from

The output is derived from game files: the script refuses a folder inside the
repository (the build folders too) or inside an install, and never converts
anything the repository tracks. Nothing here is committed or released.
Children run without `-I` so that `PYTHONHASHSEED` in the environment reaches them.
"""

import argparse
import os
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent
sys.path.insert(0, str(HERE))

import convert_install_missions as cim  # noqa: E402


def inside(path, root):
    path, root = os.path.realpath(path), os.path.realpath(root)
    return path == root or path.startswith(root.rstrip(os.sep) + os.sep)


def refuse(out, installs):
    """Why `out` is refused, or None: never in the repository, never in an install."""
    if inside(out, REPO):
        return "is inside the repository: output is derived from game files"
    for r in installs:
        if inside(out, r):
            return f"is inside the install {r}: installs are read-only"
    return None


def run(cmd, log):
    log.write("$ " + " ".join(os.path.basename(c) if i == 1 else c for i, c in enumerate(cmd)) + "\n")
    log.flush()
    p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", cwd=REPO)
    log.write(p.stdout)
    log.write(f"[exit {p.returncode}]\n")
    return p.returncode


def first_line(cmd):
    try:
        p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
        return p.stdout.splitlines()[0] if p.stdout else "(no output)"
    except OSError as e:
        return f"(not available: {e})"


def mission_dlls(install):
    mdir = next((m for m in (cim.child(install, "missions"), cim.child(cim.child(install, "Data") or install, "missions")) if m), None)
    if not mdir:
        return []
    return sorted(os.path.join(mdir, f) for f in os.listdir(mdir) if re.search(r"\.(dsl|drl)$", f, re.I))


def one(install, out):
    install = os.path.realpath(install)
    sub = os.path.join(out, cim.slug(os.path.basename(install)))
    os.makedirs(sub, exist_ok=True)
    py = sys.executable
    with open(os.path.join(sub, "versions.txt"), "w", encoding="utf-8", newline="\n") as fh:
        fh.write(f"python: {sys.version.split()[0]}\n")
        fh.write(f"objdump: {first_line(['objdump', '--version'])}\n")
    codes = []
    with open(os.path.join(sub, "log.txt"), "w", encoding="utf-8", newline="\n") as log:
        if cim.is_armada1(install):
            codes.append(run([py, str(HERE / "a1_to_lua.py"), "--out", sub, "--install", install], log))
        else:
            mdir = next((m for m in (cim.child(install, "missions"), cim.child(cim.child(install, "Data") or install, "missions")) if m), None)
            if not mdir:
                sys.exit(f"--install {install}: no missions folder")
            cmd = [py, str(HERE / "dsl_to_lua.py"), "--out", sub, "--engine", install, "--install", mdir]
            bdir = cim.child(os.path.dirname(mdir), "bzn")
            if bdir:
                cmd += ["--install", bdir]
            codes.append(run(cmd, log))
            codes.append(run([py, str(HERE / "drl_to_lua.py"), "--out", os.path.join(sub, "drl"),
                              "--engine", install, "--install", mdir], log))
    if "--objdump-listings" in sys.argv:
        ldir = os.path.join(sub, "listings")
        os.makedirs(ldir, exist_ok=True)
        for dll in mission_dlls(install):
            p = subprocess.run(["objdump", "-d", "-M", "intel", "--no-show-raw-insn", dll],
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            with open(os.path.join(ldir, os.path.basename(dll).lower() + ".txt"), "wb") as fh:
                fh.write(p.stdout)
    return codes


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--install", action="append", required=True, help="game root (repeatable; read-only)")
    ap.add_argument("--out", required=True, help="output folder, outside the repository and every install")
    ap.add_argument("--objdump-listings", action="store_true", help="also write the objdump text of every mission DLL")
    a = ap.parse_args(argv)
    why = refuse(a.out, a.install)
    if why:
        sys.exit(f"--out {a.out} {why}")
    out = os.path.realpath(a.out)
    os.makedirs(out, exist_ok=True)
    worst = 0
    for install in a.install:
        for code in one(install, out):
            worst = max(worst, code)
    return 1 if worst else 0


if __name__ == "__main__":
    sys.exit(main())
