#!/usr/bin/env python3
"""Campaign readiness table: how many campaign missions STA64 can run (LUA-05).

One command:

    python3 tools/campaign_readiness.py [--a1 DIR] [--a2 DIR] [--roots DIR]
                                        [--ticks 300] [--detail-dir DIR]

Runs `tools/agent-cargo --light run -p sta_mission --example campaign_readiness`
(the probe in crates/sta_mission/src/readiness.rs: each mission started the way
the campaign runner starts it, on its own map, N ticks, no player input) and
adds a static read of the legacy-DLL missions. Columns, each a subset of the
one before it (a funnel):

  Lua        (a) the mission resolves to a Lua script STA64 can load
  runs       (b) it loads and runs N ticks with no runtime error other than
             a stop at an unsupported call (a bound call with no seam or data
             raises by design; see `is_unsupported_stop`)
  bound      (c) and the run touched no unbound name (no `stub global/field`)
  working    (d) and nothing it called is bound-but-unsupported (counted) and
             no unsupported call stopped it

The "legacy" columns are not runnable results. For a mission with only DLL
rules they count the engine calls its `.dsl` / `.drl` conditions make (read by
tools/mission_dll_inventory.py and tools/drl_to_lua.py) and say whether every
one is bound / bound and working on STA64's `A2` / `A1` table, i.e. the state
the mission would reach if converted. Calls that are only DLL plumbing
(variables, timers, rule machinery) are left out; a legacy count is not an
evidence of a working conversion. Only names and counts are printed: no game
text, address or code. Detail (Markdown and JSON) goes to `--detail-dir`,
outside the repository.

Needs `objdump` and `llvm-undname` for the legacy columns (skipped with a note
when missing) and the installs. Install roots and the detail directory come
from options or environment variables, never from built-in paths:

  --a1 / STA64_TEST_A1_INSTALL              Armada I install
  --a2 / STA64_TEST_A2_STOCK_INSTALL        Armada II stock install
  --roots / STA64_TEST_ROOTS_INSTALL        Fleet Operations Roots install
  --detail-dir / STA64_RESEARCH_DIR         research directory (the report goes
                                            to <dir>/campaign-readiness)

An install with no option and no variable, or whose path is absent, is skipped.

Converted-mission progress (LUA-11), one command:

    python3 tools/campaign_readiness.py --converted [--a2 DIR] [--ticks 9000]
                                        [--no-standin] [--label NAME] [--only a2_fed01]

converts the stock Armada II missions with `tools/dsl_to_lua.py` into a
temporary directory (removed afterwards; `--keep-converted DIR` keeps it there),
runs each headless in the campaign runner (`examples/converted_progress.rs`: no
window, no audio, the stand-in host answers audio/movie/Enter/cineractive completion
reports) and prints, per mission, how far it got: rules that held, steps run,
the first call that was unbound or raised, whether a player objective was
issued. `--label before|after` names the files in the detail directory
(`converted-<label>.md/.json`). Progress is how far the converted script runs;
no mission is playable until a real win is reached.
"""

import argparse
import collections
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
ENV = {
    "a1": "STA64_TEST_A1_INSTALL",
    "a2": "STA64_TEST_A2_STOCK_INSTALL",
    "roots": "STA64_TEST_ROOTS_INSTALL",
}
RESEARCH_ENV = "STA64_RESEARCH_DIR"
DEFAULTS = {k: os.environ.get(v) for k, v in ENV.items()}

LABELS = {
    "a1": "Armada I",
    "a2": "Armada II (stock)",
    "roots/base": "FO Roots (base)",
    "roots/STA2 Classic Mod": "STA2 Classic (Roots mod)",
    "roots/STA2 Classic Mod Enhanced Gameplay": "STA2 Classic Enhanced Gameplay (Roots mod)",
}


def label(install):
    return LABELS.get(install, install.replace("roots/", "Roots mod ") if install.startswith("roots/") else install)


# ------------------------------------------------------------ pure parts


def parse_rows(text):
    """JSON lines of the example -> list of dicts (blank lines skipped)."""
    return [json.loads(l) for l in text.splitlines() if l.strip()]


STUB_PREFIX = "stub "


def is_unsupported_stop(error, unsupported):
    """Mirror of `sta_mission::readiness::is_unsupported_stop`: the error is a
    stop at a bound-but-unsupported call (`A1.X`/`A2.X is not available`, or
    the text of a counted `unsupported` entry), not a runtime error."""
    for k, _ in unsupported:
        if not k.startswith(STUB_PREFIX) and k in error:
            return True
    for m in re.finditer(r" is not available", error):
        head = error[:m.start()]
        name = re.search(r"[A-Za-z0-9_.]*$", head).group(0)
        if name.startswith(("A1.", "A2.")):
            return True
    return False


def split_errors(r):
    """(real runtime errors, unsupported-call stops) of one mission row."""
    real, stops = [], []
    for e in r.get("errors") or []:
        (stops if is_unsupported_stop(e, r.get("unsupported") or []) else real).append(e)
    return real, stops


def funnel(rows):
    """Counts of one install's mission rows: total, a, b, c, d (each a subset of the last)."""
    n = {"missions": 0, "lua": 0, "runs": 0, "bound": 0, "working": 0}
    for r in rows:
        n["missions"] += 1
        lua = r.get("kind") == "lua"
        real, stops = split_errors(r)
        runs = lua and r.get("started") and not real
        bound = runs and not r.get("unbound")
        working = bound and not r.get("unsupported") and not stops
        n["lua"] += lua
        n["runs"] += bool(runs)
        n["bound"] += bool(bound)
        n["working"] += bool(working)
    return n


def top_names(rows, field, limit=15):
    """Names in `field` ('unbound' / 'unsupported') by the number of missions they appear in."""
    c = collections.Counter()
    for r in rows:
        for name in {k for k, _ in r.get(field, [])}:
            c[name] += 1
    return c.most_common(limit)


def legacy_status(calls):
    """calls: [(name, kind)] with kind working / raises / missing.

    Returns (n_calls, n_bound, n_working, all_bound, all_working)."""
    n = len(calls)
    bound = sum(1 for _, k in calls if k != "missing")
    working = sum(1 for _, k in calls if k == "working")
    return n, bound, working, bound == n, working == n


def legacy_counts(legacy_rows):
    """legacy_rows: [{'calls': [(name, kind)], ...}] -> (missions, all bound, all bound and working)."""
    b = w = 0
    for r in legacy_rows:
        _, _, _, ab, aw = legacy_status(r["calls"])
        b += ab
        w += aw
    return len(legacy_rows), b, w


def render_table(groups):
    """groups: [(label, funnel dict, (legacy missions, bound, working) or None)] -> Markdown."""
    head = ("| install | missions | (a) Lua | (b) runs | (c) bound | (d) working "
            "| legacy missions | legacy: calls bound if converted | legacy: bound and working if converted |")
    out = [head, "|---|--:|--:|--:|--:|--:|--:|--:|--:|"]
    tot = collections.Counter()
    lt = [0, 0, 0]
    for name, f, leg in groups:
        lm, lb, lw = leg if leg else (None, None, None)
        out.append(f"| {name} | {f['missions']} | {f['lua']} | {f['runs']} | {f['bound']} | {f['working']} | "
                   + (f"{lm} | {lb} | {lw} |" if leg else "- | - | - |"))
        tot.update(f)
        if leg:
            lt = [lt[0] + lm, lt[1] + lb, lt[2] + lw]
    out.append(f"| **all** | {tot['missions']} | {tot['lua']} | {tot['runs']} | {tot['bound']} | {tot['working']} | "
               f"{lt[0]} | {lt[1]} | {lt[2]} |")
    return "\n".join(out)


def mission_status(r):
    if r.get("kind") != "lua":
        return {"legacy": "legacy DLL only", "none": "no script"}.get(r.get("kind"), r.get("kind"))
    if not r.get("started"):
        return "does not start"
    real, stops = split_errors(r)
    if real:
        return "runtime error"
    if r.get("unbound"):
        return "unbound names"
    if stops:
        return "stopped at an unsupported call"
    if r.get("unsupported"):
        return "unsupported calls"
    return "working"


def one_line(s, n=160):
    s = " ".join(str(s).split())
    return s if len(s) <= n else s[: n - 3] + "..."


def render_detail(by_install, legacy_by_install, ticks):
    """Per-mission Markdown."""
    out = ["# Campaign readiness, per mission", "",
           f"Horizon {ticks} ticks, no player input. Statuses: working, unsupported calls, stopped at an unsupported call, unbound names, "
           "runtime error, does not start; legacy missions show the static counterpart.", ""]
    for install, rows in by_install.items():
        out += [f"## {label(install)}", ""]
        lua = [r for r in rows if r.get("kind") == "lua"]
        if lua:
            out += ["| campaign | # | map | status | unbound | unsupported | error |", "|---|--:|---|---|--:|--:|---|"]
            for r in lua:
                real, stops = split_errors(r)
                err = (real or stops or [None])[0] or (r.get("start_error") or "")
                out.append(f"| {r['campaign']} | {r['index'] + 1} | {r['bzn']} | {mission_status(r)} | "
                           f"{len(r['unbound'])} | {len(r['unsupported'])} | {one_line(err).replace('|', '/')} |")
            out.append("")
            for field, title in (("unbound", "Unbound names"), ("unsupported", "Unsupported calls")):
                top = top_names(lua, field, 100)
                if top:
                    out += [f"### {title} by missions affected", ""] + [f"- {n} missions: `{k}`" for k, n in top] + [""]
        leg = legacy_by_install.get(install)
        if leg:
            out += ["| campaign | # | map | files | calls | bound | working | not bound | bound, raises/no effect |",
                    "|---|--:|---|---|--:|--:|--:|---|---|"]
            for r in leg:
                n, b, w, _, _ = legacy_status(r["calls"])
                miss = ", ".join(sorted(c for c, k in r["calls"] if k == "missing"))
                raises = ", ".join(sorted(c for c, k in r["calls"] if k == "raises"))
                files = "+".join(r["files"])
                note = f" ({r['note']})" if r.get("note") else ""
                out.append(f"| {r['campaign']} | {r['index'] + 1} | {r['bzn']} | {files}{note} | {n} | {b} | {w} | {miss} | {raises} |")
            out.append("")
        none = [r for r in rows if r.get("kind") == "none"]
        if none:
            out += ["No script: " + ", ".join(r["bzn"] for r in none), ""]
    return "\n".join(out) + "\n"


# ------------------------------------------------------------ install access


def listdir_ci(path, name):
    """Entry of `path` equal to `name` ignoring case, or None."""
    try:
        for e in os.listdir(path):
            if e.lower() == name.lower():
                return os.path.join(path, e)
    except OSError:
        pass
    return None


def locate_mission_file(dirs, name):
    """`missions/<name>` under the first of `dirs` that has it (mods first)."""
    for d in dirs:
        m = listdir_ci(d, "missions")
        f = listdir_ci(m, name) if m else None
        if f:
            return f
    return None


def mod_chain_dirs(roots_dir, mod):
    """Directories of `mod` and its `ParentMod` chain (info.ini), nearest first."""
    out, seen = [], set()
    data = listdir_ci(roots_dir, "Data") or roots_dir
    mods = listdir_ci(data, "Mods") or os.path.join(data, "Mods")
    while mod and mod.lower() not in seen:
        seen.add(mod.lower())
        d = listdir_ci(mods, mod)
        if not d:
            break
        out.append(d)
        mod = None
        ini = listdir_ci(d, "info.ini")
        if ini:
            for line in open(ini, encoding="latin1"):
                k, _, v = line.partition("=")
                if k.strip().lower() == "parentmod":
                    mod = v.strip().strip('"')
    return out


def with_partners(files):
    """A2 mission files: a `<x>R.drl` rule DLL goes with its `<x>S.dsl` script."""
    out = list(files)
    for f in files:
        if f.lower().endswith("r.drl"):
            partner = f[:-5] + ("S.dsl" if f[-5] == "R" else "s.dsl")
            if partner.lower() not in (x.lower() for x in out):
                out.append(partner)
    return out


# ------------------------------------------------------------ converted missions


def converted_status(r):
    """(stage, stopping call) of one `converted_progress` row. Honest labels:
    `objective` = the first player objective was issued, `ended` = the script
    ended the mission, otherwise what stopped it."""
    if not r.get("started"):
        return "does not start", r.get("start_error") or ""
    # `A2.<call>: ...` errors are counted rejections of an edit by the sim, not script errors
    real = [e for e in r.get("errors") or [] if ": simulation rejected" not in e]
    if real:
        return "runtime error", one_line(real[0])
    if r.get("ended") is not None:
        return ("ended: won" if r["ended"] else "ended: lost"), ""
    if r.get("end_blocked"):
        return "end blocked", r["end_blocked"]
    first = (r.get("problems") or [None])[0]
    stop = f"{first[0]} ({first[1]})" if first else ""
    if r.get("first_objective_tick") is not None:
        return "objective issued", stop
    waiting = [s for s in r.get("scripts") or [] if s[2] > 0]
    if waiting:
        return "stalled on a script wait", waiting[0][0]
    if first:
        return "stalled at a call", stop
    return "idle: no rule fires", ""


def converted_funnel(rows):
    n = collections.Counter()
    for r in rows:
        stage, _ = converted_status(r)
        n["missions"] += 1
        n["loaded"] += bool(r.get("started")) and converted_status(r)[0] != "runtime error"
        n["rules"] += bool(r.get("rules_held"))
        n["steps"] += r.get("steps_run", 0) > 0
        n["objective"] += r.get("first_objective_tick") is not None
        n["ended"] += stage.startswith("ended")
        n["blocked_end"] += stage == "end blocked"
        # A playable candidate ends by its own rules with no unbound or
        # raising call on the way (LUA-13).
        n["candidate"] += stage.startswith("ended") and not r.get("problems")
        n["clean"] += not r.get("problems")
    return n


def render_converted(rows, ticks, standin):
    n = converted_funnel(rows)
    out = [f"Converted Armada II missions, {ticks} ticks each, headless, stand-in host {'on' if standin else 'off'}", "",
           "| missions | load and run | a rule held | a step ran | first objective issued | ended itself | end blocked by a gap | no unbound/raising call | playable candidates |",
           "|--:|--:|--:|--:|--:|--:|--:|--:|--:|",
           f"| {n['missions']} | {n['loaded']} | {n['rules']} | {n['steps']} | {n['objective']} | {n['ended']} | {n['blocked_end']} | {n['clean']} | {n['candidate']} |", "",
           "| mission | stage | rules held | steps run | last rule | last step | objective tick | first problem call | end |",
           "|---|---|--:|--:|---|---|--:|---|---|"]
    for r in sorted(rows, key=lambda r: r["bzn"]):
        stage, stop = converted_status(r)
        first = (r.get("problems") or [None])[0]
        call = f"{first[0]} ({first[1]})" if first else "-"
        out.append(f"| {r['bzn'].rsplit('.', 1)[0]} | {stage} | {len(r['rules_held'])}/{r['rules']} | {r['steps_run']} | "
                   f"{r.get('last_rule') or '-'} | {r.get('last_step') or '-'} | {'-' if r.get('first_objective_tick') is None else r['first_objective_tick']} | {call} | {stop if stage == 'end blocked' else stage if stage.startswith('ended') else '-'} |")
    ranked = collections.Counter()
    for r in rows:
        for name, _kind, _t, _w in r.get("problems") or []:
            ranked[name] += 1
    out += ["", "Calls that were unbound or raised, by missions affected:", ""]
    out += [f"- {c}: `{k}`" for k, c in ranked.most_common(40)] or ["- none"]
    return "\n".join(out) + "\n"


def main_converted(args):
    import shutil
    import tempfile
    a2 = args.a2
    if not a2 or not os.path.isdir(a2):
        sys.exit("--converted needs the Armada II stock install: pass --a2 or set " + ENV["a2"])
    if not args.detail_dir:
        sys.exit(f"no detail directory: pass --detail-dir or set {RESEARCH_ENV}")
    t0 = time.time()
    out = args.keep_converted or tempfile.mkdtemp(prefix="sta64-converted-")
    try:
        cmd = [sys.executable, "-I", os.path.join(HERE, "dsl_to_lua.py"), "--install", a2, "--out", out]
        if args.only:
            cmd += ["--only", args.only]
        p = subprocess.run(cmd, capture_output=True, text=True)
        if p.returncode != 0:
            sys.exit(f"conversion failed ({p.returncode}):\n{p.stderr[-2000:]}")
        cmd = [os.path.join(HERE, "agent-cargo"), "--light", "run", "-q", "-p", "sta_mission", "--example",
               "converted_progress", "--", "--install", a2, "--converted", out, "--ticks", str(args.ticks)]
        if not args.no_standin:
            cmd.append("--standin")
        if args.only:
            cmd += ["--only", args.only]
        p = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True)
        if p.returncode != 0:
            sys.exit(f"run failed ({p.returncode}):\n{p.stderr[-2000:]}")
        rows = parse_rows(p.stdout)
    finally:
        if not args.keep_converted:
            shutil.rmtree(out, ignore_errors=True)
    text = render_converted(rows, args.ticks, not args.no_standin)
    print(text + f"({time.time() - t0:.0f} s)")
    os.makedirs(args.detail_dir, exist_ok=True)
    stem = os.path.join(args.detail_dir, f"converted-{args.label}")
    with open(stem + ".md", "w") as f:
        f.write(text)
    with open(stem + ".json", "w") as f:
        json.dump({"ticks": args.ticks, "standin": not args.no_standin, "missions": rows}, f, indent=1)
    print(f"detail: {stem}.md, {stem}.json")


def run_probe(args, root_args):
    cmd = [os.path.join(HERE, "agent-cargo"), "--light", "run", "-q", "-p", "sta_mission", "--example",
           "campaign_readiness", "--", "--ticks", str(args.ticks)]
    for k, v in root_args.items():
        cmd += [f"--{k}", v]
    p = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True)
    if p.returncode != 0:
        sys.exit(f"probe failed ({p.returncode}):\n{p.stderr[-2000:]}")
    return parse_rows(p.stdout)


# ------------------------------------------------------------ static legacy part


def legacy_calls(inv, drl, rows_by_install, roots, a2_root):
    """Per install, the static call kinds of every legacy mission.

    Returns {install: [{'campaign','index','bzn','files','calls':[(name, kind)],'note'}]}."""
    import shutil
    if not (shutil.which("objdump") and shutil.which("llvm-undname")):
        print("legacy columns skipped: objdump and llvm-undname are needed", file=sys.stderr)
        return {}
    raising = inv.a2_raising(REPO)
    a1_kinds = inv.a1_kinds(REPO)
    slots = engine = None
    if a2_root:
        exe, mapf = inv.find(a2_root, "Armada2.exe"), inv.find(a2_root, "armada2.map")
        if exe and mapf:
            slots = inv.a2_slots(exe[0], mapf[0])
            engine = drl.engine_vtables(a2_root)
    cache = {}

    def plumbing(sig):
        return inv.area(inv.normalized_signature(sig)) == "Script state and save/load"

    def a2_file(path):
        if path in cache:
            return cache[path]
        sigs = set()
        if path.lower().endswith(".dsl"):
            use, _, _ = inv.a2_usage(slots, [path])
            sigs = {s for i, s in slots if use.get(i)}
        else:
            r = drl.decode(path, engine)
            mangled = sorted(r["call_signatures"])
            sigs = set(inv.demangle(mangled)) if mangled else set()
        out = {}
        for s in sigs:
            if "(" not in s or plumbing(s):
                continue
            out[(inv.method_name(s), s)] = inv.a2_condition_kind(s, REPO, raising)
        cache[path] = out
        return out

    def a1_file(path):
        if path not in cache:
            out = {}
            for s, _ in inv.a1_usage([path]):
                if "(" in s and not plumbing(s):
                    out[(inv.method_name(s), s)] = inv.a1_condition_kind(s, REPO, a1_kinds)
            cache[path] = out
        return cache[path]

    result = {}
    for install, rows in rows_by_install.items():
        legacy = [r for r in rows if r.get("kind") == "legacy"]
        if not legacy:
            continue
        if install == "a1":
            dirs, per_file = [roots["a1"]], a1_file
        else:
            if slots is None:
                print(f"legacy columns for {label(install)} skipped: need the A2 install's Armada2.exe and armada2.map", file=sys.stderr)
                continue
            dirs = [roots["a2"]]
            if install.startswith("roots/"):
                dirs = mod_chain_dirs(roots["roots"], install[len("roots/"):]) + [roots["roots"]]
            per_file = a2_file
        out = []
        for r in legacy:
            calls, note = {}, []
            for f in (r["files"] if install == "a1" else with_partners(r["files"])):
                path = locate_mission_file(dirs, f)
                if not path:
                    note.append(f"{f} not found")
                    continue
                try:
                    calls.update(per_file(path))
                except Exception as e:  # decoder refusals: report, do not guess
                    note.append(f"{f}: {type(e).__name__}")
            out.append({"campaign": r["campaign"], "index": r["index"], "bzn": r["bzn"], "files": r["files"],
                        "calls": sorted((n, k) for (n, _), k in calls.items()), "note": "; ".join(note)})
        result[install] = out
    return result


# ------------------------------------------------------------ main


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    for k in DEFAULTS:
        ap.add_argument(f"--{k}", default=DEFAULTS[k], help=f"install root (default: ${ENV[k]}; skipped when unset or absent)")
    ap.add_argument("--ticks", type=int, default=None, help="ticks per mission (default 300; 9000 with --converted)")
    research = os.environ.get(RESEARCH_ENV)
    ap.add_argument("--detail-dir", default=os.path.join(research, "campaign-readiness") if research else None,
                    help=f"where the Markdown/JSON detail goes, outside the repository (default: ${RESEARCH_ENV}/campaign-readiness; required)")
    ap.add_argument("--no-legacy", action="store_true", help="skip the static legacy columns")
    ap.add_argument("--converted", action="store_true", help="converted-mission progress mode (see the module docs)")
    ap.add_argument("--no-standin", action="store_true", help="--converted: no stand-in host (waits are never reported)")
    ap.add_argument("--label", default="latest", help="--converted: detail file label (converted-<label>.md)")
    ap.add_argument("--only", help="--converted: one mission (a2_fed01)")
    ap.add_argument("--keep-converted", help="--converted: convert into this directory and keep it")
    args = ap.parse_args(argv)
    if args.converted:
        args.ticks = args.ticks or 9000
        return main_converted(args)
    args.ticks = args.ticks or 300
    if not args.detail_dir:
        sys.exit(f"no detail directory: pass --detail-dir or set {RESEARCH_ENV} (the detail must stay outside the repository)")
    t0 = time.time()

    roots = {k: getattr(args, k) for k in DEFAULTS}
    present = {k: v for k, v in roots.items() if v and os.path.isdir(v)}
    for k in sorted(set(roots) - set(present)):
        why = f"{roots[k]} is absent" if roots[k] else f"no --{k} and ${ENV[k]} is not set"
        print(f"skipping {k}: {why}", file=sys.stderr)
    if not present:
        sys.exit("no install found")
    rows = run_probe(args, present)
    errors = [r for r in rows if "error" in r]
    for r in errors:
        print(f"{r['install']}: {r['error']}", file=sys.stderr)
    rows = [r for r in rows if "error" not in r]
    by_install = collections.OrderedDict()
    for r in rows:
        by_install.setdefault(r["install"], []).append(r)

    legacy = {}
    if not args.no_legacy:
        sys.path.insert(0, HERE)
        import mission_dll_inventory as inv
        import drl_to_lua as drl
        legacy = legacy_calls(inv, drl, by_install, roots, roots.get("a2"))

    groups = [(label(i), funnel(rs), legacy_counts(legacy[i]) if i in legacy else None) for i, rs in by_install.items()]
    table = render_table(groups)
    lua_rows = [r for r in rows if r.get("kind") == "lua"]
    tops = []
    for field, title in (("unbound", "Unbound names"), ("unsupported", "Unsupported calls")):
        tops += [f"{title} (missions affected), all Lua missions:", ""]
        tops += [f"- {n}: `{k}`" for k, n in top_names(lua_rows, field)] or ["- none"]
        tops.append("")
    secs = time.time() - t0
    head = f"Campaign readiness, {args.ticks} ticks per mission (probe + static read: {secs:.0f} s)\n"
    print(head + "\n" + table + "\n\n" + "\n".join(tops))

    if args.detail_dir:
        os.makedirs(args.detail_dir, exist_ok=True)
        with open(os.path.join(args.detail_dir, "latest.md"), "w") as f:
            f.write("# " + head + "\n" + table + "\n\n" + "\n".join(tops) + "\n")
        with open(os.path.join(args.detail_dir, "detail.md"), "w") as f:
            f.write(render_detail(by_install, legacy, args.ticks))
        with open(os.path.join(args.detail_dir, "detail.json"), "w") as f:
            json.dump({"ticks": args.ticks, "missions": rows, "legacy": legacy}, f, indent=1)
        print(f"detail: {args.detail_dir}/{{latest.md,detail.md,detail.json}}")


if __name__ == "__main__":
    main()
