#!/usr/bin/env python3
"""Play one campaign mission headless with a scripted player and compare its
timeline with a reference sequence (LUA-17).

    python3 tools/mission_playthrough.py --install DIR [--mod NAME] [--converted DIR]
        --mission a2_fed01 [--plan FILE.lua] [--ticks N] [--out timeline.jsonl]
        [--reference REF.txt] [--summary]

Runs `tools/agent-cargo --light run -q -p sta_mission --example mission_playthrough`
(crates/sta_mission/src/playthrough.rs: the campaign runner, the stand-in host,
the plan as the human seat's only input) and writes the timeline as JSON lines
(`{"t": seconds, "tick": n, "kind": "...", "what": "..."}`). `--timeline FILE`
compares an existing timeline instead of running.

`--converted` is the converter's output folder (`<data dir>/converted/<key>`, see
tools/convert_install_missions.py); without it the install's own Lua missions run
(Fleet Operations / Roots). The plan is a Lua chunk run as a coroutine; see the
prelude in playthrough.rs for its functions (`units`, `move`, `attack`, `build`,
`wait`, `wait_until`, `control`, `objectives`, `paths`, `label`, `note`).

Reference file (kept outside the repository, it describes a stock mission):

    # comment
    tolerance 2.0                 seconds a timed entry may be off (default 2)
    strict present objective      kinds whose unreferenced events are listed as extra
    <time> <kind> <text>          one expected event, in order

`<time>` is seconds from the start (`12.5`), relative to the previous expected
event (`+3`), or `*` (any time). `<kind>` is a timeline kind (`present`, `call`,
`objective`, `briefing`, `rule`, `end`, ...); `<text>` must occur in the event's
`what` (case-sensitive). Expected events are matched in order, each after the
previous match; the report lists every difference: missing, out of order,
timing off, and the extra events of the strict kinds. Exit status 0 when there
is no difference, 1 otherwise.
"""

import argparse
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)


# ------------------------------------------------------------ pure parts


def parse_timeline(text):
    return [json.loads(l) for l in text.splitlines() if l.strip().startswith("{")]


def parse_reference(text):
    """-> (entries, tolerance, strict kinds); an entry is dict(t, rel, kind, text, line)."""
    entries, tol, strict = [], 2.0, set()
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        head, _, rest = line.partition(" ")
        if head == "tolerance":
            tol = float(rest)
            continue
        if head == "strict":
            strict |= set(rest.split())
            continue
        kind, _, what = rest.strip().partition(" ")
        if head == "*":
            t, rel = None, False
        elif head.startswith("+"):
            t, rel = float(head[1:]), True
        else:
            t, rel = float(head), False
        entries.append({"t": t, "rel": rel, "kind": kind, "text": what.strip(), "line": n})
    return entries, tol, strict


def matches(e, r):
    return e["kind"] == r["kind"] and r["text"] in e["what"]


def compare(timeline, entries, tol=2.0, strict=()):
    """Ordered match of `entries` against `timeline`. -> list of difference strings."""
    diffs, at, prev_t, used = [], 0, 0.0, set()
    for r in entries:
        hit = next((i for i in range(at, len(timeline)) if i not in used and matches(timeline[i], r)), None)
        label = f"line {r['line']}: {r['kind']} {r['text']!r}"
        if hit is None:
            earlier = next((i for i in range(0, at) if i not in used and matches(timeline[i], r)), None)
            if earlier is not None:
                diffs.append(f"out of order: {label} happened at {timeline[earlier]['t']:.2f} s, before the previous expected event")
                used.add(earlier)
            else:
                diffs.append(f"missing: {label}")
            continue
        e = timeline[hit]
        used.add(hit)
        at = hit + 1
        if r["t"] is not None:
            want = prev_t + r["t"] if r["rel"] else r["t"]
            if abs(e["t"] - want) > tol:
                diffs.append(f"timing: {label} at {e['t']:.2f} s, expected {want:.2f} s ({e['t'] - want:+.2f})")
        prev_t = e["t"]
    if strict and entries:
        last = max(used) if used else -1
        for i, e in enumerate(timeline[: last + 1]):
            if i not in used and e["kind"] in strict:
                diffs.append(f"extra: {e['t']:.2f} s {e['kind']} {e['what'][:160]}")
    return diffs


def summary(timeline):
    """Short text: end, objectives, problems."""
    out = []
    by = {}
    for e in timeline:
        by.setdefault(e["kind"], []).append(e)
    end = by.get("end", [{"t": timeline[-1]["t"] if timeline else 0, "what": "no end"}])[-1]
    out.append(f"end: {end['what']} at {end['t']:.1f} s")
    for e in by.get("objective", []):
        out.append(f"  {e['t']:8.1f} objective {e['what']}")
    for e in by.get("briefing", []):
        out.append(f"  {e['t']:8.1f} briefing {e['what']}")
    for k in ("unbound", "raised", "unsupported", "error", "step-error", "plan-error", "camera-ignored"):
        if by.get(k):
            names = sorted({e["what"].split("(")[0][:100] for e in by[k]})
            out.append(f"{k} ({len(by[k])}): " + "; ".join(names[:30]))
    return "\n".join(out)


# ------------------------------------------------------------ running


def run(args):
    cmd = [os.path.join(REPO, "tools", "agent-cargo"), "--light", "run", "-q", "-p", "sta_mission",
           "--example", "mission_playthrough", "--", "--install", args.install, "--mission", args.mission,
           "--ticks", str(args.ticks)]
    for flag in ("mod", "converted", "plan", "seed"):
        v = getattr(args, flag)
        if v is not None:
            cmd += [f"--{flag}", str(v)]
    p = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True)
    if p.returncode != 0:
        sys.stderr.write(p.stderr[-4000:])
        sys.exit(p.returncode)
    return p.stdout


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--install")
    ap.add_argument("--mod")
    ap.add_argument("--converted")
    ap.add_argument("--mission")
    ap.add_argument("--plan")
    ap.add_argument("--ticks", type=int, default=30 * 60 * 30)
    ap.add_argument("--seed", type=int)
    ap.add_argument("--out", help="write the timeline here (JSON lines)")
    ap.add_argument("--timeline", help="compare this timeline instead of running")
    ap.add_argument("--reference", help="reference sequence to compare with")
    ap.add_argument("--summary", action="store_true", help="print a short summary")
    args = ap.parse_args(argv)
    if args.timeline:
        text = open(args.timeline, encoding="utf-8").read()
    else:
        if not (args.install and args.mission):
            ap.error("--install and --mission are needed to run")
        text = run(args)
        if args.out:
            with open(args.out, "w", encoding="utf-8") as f:
                f.write(text)
    timeline = parse_timeline(text)
    if args.summary or not args.reference:
        print(summary(timeline))
    if args.reference:
        entries, tol, strict = parse_reference(open(args.reference, encoding="utf-8").read())
        diffs = compare(timeline, entries, tol, strict)
        print(f"reference: {len(entries)} expected events, {len(diffs)} differences (tolerance {tol} s)")
        for d in diffs:
            print("  " + d)
        return 1 if diffs else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
