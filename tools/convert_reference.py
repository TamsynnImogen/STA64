#!/usr/bin/env python3
"""Write the Python converters' reference output for installs, for the Rust port's parity harness.

    python3 tools/convert_reference.py --out <folder outside the repository> \\
        --install "<Armada I or II root>" [--install ...] \\
        [--hashseed N] [--include-unmapped] [--objdump-listings]

For each install (Armada I or Armada II, detected as `convert_install_missions.py`
does) it runs the converter as that wrapper does and writes, under
`<out>/<install folder slug>/` (a numeric suffix keeps equal slugs apart):

    missions/*.lua   `a1_to_lua.py` or `dsl_to_lua.py` output (`--include-unmapped`:
                     for Armada I also the DLL pairs no map names)
    drl/<...>/*.lua  `drl_to_lua.py` scaffolds (Armada II only)
    log.txt          the converters' console output (UTF-8; install and output paths
                     replaced by `<install>` and `<out>`)
    versions.txt     what produced the output: converter file hashes and git revision,
                     python, objdump, platform, hash seed, flags
    listings/*.txt   with `--objdump-listings`: the objdump text of each mission DLL

Parity target: the `*.lua` files. The children run with a pinned environment
(UTF-8, the given `--hashseed`, no PYTHONPATH/PYTHONHOME/PYTHONSTARTUP, no bytecode files).

Output is derived from game files: every folder this script writes is checked
first and refused inside the repository (build folders too) or inside an install,
before anything is created. Nothing here is committed or released.
"""

import argparse
import hashlib
import os
import pathlib
import platform
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent
sys.path.insert(0, str(HERE))

import convert_install_missions as cim  # noqa: E402

TOOLS = ["dsl_to_lua.py", "drl_to_lua.py", "a1_to_lua.py", "convert_install_missions.py"]
DROP = ("PYTHONPATH", "PYTHONHOME", "PYTHONSTARTUP", "PYTHONINSPECT", "PYTHONSAFEPATH")


def inside(path, root):
    """True when `path` is `root` or below it: by real path, and by identity of the
    nearest existing ancestor (a case-insensitive filesystem spells one folder two ways)."""
    path, root = os.path.realpath(path), os.path.realpath(root)
    cur = path
    while True:
        if os.path.normcase(cur) == os.path.normcase(root):
            return True
        if os.path.exists(cur) and os.path.exists(root) and os.path.samefile(cur, root):
            return True
        parent = os.path.dirname(cur)
        if parent == cur:
            return False
        cur = parent


def refuse(out, installs):
    """Why `out` is refused, or None: never in the repository, never in an install."""
    if inside(out, REPO):
        return "is inside the repository: output is derived from game files"
    for r in installs:
        if inside(out, r):
            return f"is inside the install {r}: installs are read-only"
    return None


def child_env(seed):
    env = {k: v for k, v in os.environ.items() if k not in DROP}
    env.update(PYTHONHASHSEED=str(seed), PYTHONUTF8="1", PYTHONIOENCODING="utf-8",
               PYTHONDONTWRITEBYTECODE="1", LC_ALL="C")
    return env


def scrub(text, names):
    """Replace absolute paths (longest first) so the log compares between machines."""
    for path, label in sorted(names, key=lambda x: -len(x[0])):
        text = text.replace(path, label)
    return text


def run(cmd, log, env, names):
    shown = ["python3", "-s", os.path.basename(cmd[2])] + [scrub(c, names) for c in cmd[3:]]
    log.write("$ " + " ".join(shown) + "\n")
    p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env, cwd=REPO)
    log.write(scrub(p.stdout.decode("utf-8", "replace"), names))
    log.write(f"[exit {p.returncode}]\n")
    return p.returncode


def first_line(cmd, env):
    try:
        p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
        out = p.stdout.decode("utf-8", "replace").splitlines()
        return out[0] if out else "(no output)"
    except OSError as e:
        return f"(not available: {e})"


def git_revision():
    try:
        p = subprocess.run(["git", "-C", str(REPO), "rev-parse", "HEAD"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        return p.stdout.decode().strip() or "(unknown)"
    except OSError:
        return "(unknown)"


def missions_dir(install):
    return next((m for m in (cim.child(install, "missions"), cim.child(cim.child(install, "Data") or install, "missions")) if m), None)


def mission_dlls(install):
    mdir = missions_dir(install)
    if not mdir:
        return []
    return sorted(os.path.join(mdir, f) for f in os.listdir(mdir) if re.search(r"\.(dsl|drl)$", f, re.I))


def subfolders(installs):
    """One output folder name per install, distinct even for equal slugs."""
    out, used = [], set()
    for root in installs:
        base = cim.slug(os.path.basename(root))
        name, n = base, 1
        while name.lower() in used:
            n += 1
            name = f"{base}-{n}"
        used.add(name.lower())
        out.append(name)
    return out


def one(install, sub, a, env):
    py = sys.executable
    os.makedirs(sub, exist_ok=True)
    mdir = missions_dir(install)
    bdir = cim.child(os.path.dirname(mdir), "bzn") if mdir else None
    names = [(install, "<install>"), (sub, "<out>")]
    a1 = cim.is_armada1(install)
    with open(os.path.join(sub, "versions.txt"), "w", encoding="utf-8", newline="\n") as fh:
        fh.write(f"kind: {'armada1' if a1 else 'armada2'}\n")
        fh.write(f"tools git revision: {git_revision()}\n")
        for t in TOOLS:
            fh.write(f"sha256 tools/{t}: {hashlib.sha256((HERE / t).read_bytes()).hexdigest()}\n")
        fh.write(f"python: {sys.version.split()[0]} ({sys.implementation.name})\n")
        fh.write(f"objdump: {first_line(['objdump', '--version'], env)}\n")
        fh.write(f"platform: {platform.platform()}\n")
        fh.write(f"PYTHONHASHSEED: {a.hashseed}\n")
        fh.write(f"flags: include-unmapped={a.include_unmapped} objdump-listings={a.objdump_listings}\n")
    codes = []
    with open(os.path.join(sub, "log.txt"), "w", encoding="utf-8", newline="\n") as log:
        if a1:
            cmd = [py, "-s", str(HERE / "a1_to_lua.py"), "--out", sub, "--install", install]
            if a.include_unmapped:
                cmd.append("--include-unmapped")
            codes.append(run(cmd, log, env, names))
        else:
            if not mdir:
                sys.exit(f"--install {install}: no missions folder")
            cmd = [py, "-s", str(HERE / "dsl_to_lua.py"), "--out", sub, "--engine", install, "--install", mdir]
            if bdir:
                cmd += ["--install", bdir]
            codes.append(run(cmd, log, env, names + [(mdir, "<install>/missions")]))
            codes.append(run([py, "-s", str(HERE / "drl_to_lua.py"), "--out", os.path.join(sub, "drl"),
                              "--engine", install, "--install", mdir], log, env, names + [(mdir, "<install>/missions")]))
    if a.objdump_listings:
        ldir = os.path.join(sub, "listings")
        os.makedirs(ldir, exist_ok=True)
        for dll in mission_dlls(install):
            p = subprocess.run(["objdump", "-d", "-M", "intel", "--no-show-raw-insn", dll],
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env)
            with open(os.path.join(ldir, os.path.basename(dll).lower() + ".txt"), "wb") as fh:
                fh.write(p.stdout)
    return codes


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--install", action="append", required=True, help="game root (repeatable; read-only)")
    ap.add_argument("--out", required=True, help="output folder, outside the repository and every install")
    ap.add_argument("--hashseed", default="0", help="PYTHONHASHSEED for the converters (default 0)")
    ap.add_argument("--include-unmapped", action="store_true", help="Armada I: also convert DLL pairs no map names")
    ap.add_argument("--objdump-listings", action="store_true", help="also write the objdump text of every mission DLL")
    a = ap.parse_args(argv)
    installs = []
    for i in a.install:
        if not os.path.isdir(i):
            sys.exit(f"--install {i}: not a folder")
        installs.append(os.path.realpath(i))
    out = os.path.realpath(a.out)
    subs = [os.path.join(out, n) for n in subfolders(installs)]
    for p in [out] + subs:  # everything this script will create, before it creates anything
        why = refuse(p, installs)
        if why:
            sys.exit(f"output folder {p} {why}")
    os.makedirs(out, exist_ok=True)
    env = child_env(a.hashseed)
    worst = 0
    for install, sub in zip(installs, subs):
        for code in one(install, sub, a, env):
            worst = max(worst, code)
    return 1 if worst else 0


if __name__ == "__main__":
    sys.exit(main())
