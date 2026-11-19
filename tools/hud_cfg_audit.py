#!/usr/bin/env python3
"""Audit of the stock games' HUD cfg keys against STA64's source (UI-48).

Reads the `gui_*.cfg` key list of `docs/data/cfg.md` (itself generated from
the user's installs by `tools/data_inventory.py`; digit runs are `#`) and the
string literals of the HUD code, and writes `docs/STOCK_HUD_CFG_AUDIT.md`:
every key with its panel, the games that set it and one of four states.

    drawn   the key is a string literal in `crates/sta64/src/ui/`
    typed   the key is a literal in `crates/sta_content/src/gui_cfg.rs` only
    built   the key ends in a suffix the code appends to a panel or element
            name (`format!("{prefix}TextArea")`, `k("backgroundpanel")`);
            which prefixes reach it is not checked here
    missing none of the above

A state says the name is used, not that the element is drawn correctly.

usage: tools/hud_cfg_audit.py [--repo .] [--check]
"""

import argparse
import re
import sys
from collections import Counter, OrderedDict
from pathlib import Path

# Panel by key prefix, first match wins (lower case).
PANELS = [
    ("bridge", r"^[a-z]_bridge|^bridge"),
    ("avatar box", r"^avatarbox"),
    ("build box", r"^buildbox"),
    ("button panel", r"^button|^bb_|^(comm|help|idle|menu|more|objectives|palette)_?(button|border)"),
    ("cinematic", r"^cinematic"),
    ("comm panel", r"^comm"),
    ("control panel", r"^control"),
    ("drop-player box", r"^dropplayer|^drop_"),
    ("info panel, single selection", r"^infosingle"),
    ("info panel, multi selection", r"^infomulti"),
    ("info panel, build and research", r"^infobuild|^inforesearch"),
    ("info panel, other", r"^info|^shipname|^nametext"),
    ("loading box", r"^loading"),
    ("minimap", r"^minimap"),
    ("objectives", r"^objectives"),
    ("pause box", r"^pause"),
    ("command palette", r"^popup|^static|^palette"),
    ("resource bar", r"^resource|^no(crew|officers|dilithium|latinum|metal|biomatter)|^(crew|dilithium|officers?)(backgroundpanel|icon|panelarea|textarea|defaultcursor)|_amt$"),
    ("speed rail", r"^speed"),
    ("tooltip and cursor", r"^tooltip|^cursor|^frame"),
    ("screen", r"^screen"),
]
STATES = ["drawn", "typed", "built", "missing"]

# Known limits recorded by the Armada I action-window implementation.
# Keep these separate from source detection so missing keys stay in the totals.
MISSING_REASONS = {
    "cinematiccommborder": "Placement still needs an original-game reference.",
    "cinematicmenuborder": "Placement still needs an original-game reference.",
    "cinematicminimizedpanel": "Armada I panel minimization is pending.",
    "cinematicminimizedpanel_#": "Armada I panel minimization is pending.",
}


def literals(paths):
    out = set()
    for f in paths:
        src = f.read_text(errors="replace")
        src = src.replace("'\"'", "").replace("'\\\"'", "")
        for m in re.finditer(r'"((?:[^"\\]|\\.)*)"', src):
            s = m.group(1).lower()
            if 1 < len(s) < 80:
                out.add(s)
    return out


def suffixes(lits):
    """Suffixes appended to a built name: `{prefix}TextArea` -> `textarea`."""
    out = set()
    for s in lits:
        m = re.fullmatch(r"\{[^{}]*\}([a-z_][a-z0-9_]{3,})", s)
        if m:
            out.add(m.group(1))
    return out


def used(key, lits):
    if key in lits:
        return True
    if "#" in key:
        rx = re.compile("^" + re.escape(key).replace("\\#", r"(\d+|\{[^{}]*\})") + "$")
        return any(rx.match(s) for s in lits)
    return False


def panel_of(key):
    for name, rx in PANELS:
        if re.search(rx, key):
            return name
    return "other"


def read_keys(cfg_md):
    rows, on = [], False
    for line in cfg_md.read_text().splitlines():
        if line.startswith("## "):
            on = line.strip() == "## `gui_*.cfg`"
            continue
        if on and line.startswith("| `"):
            c = [x.strip() for x in line.split("|")[1:-1]]
            games = [g for g, v in zip(("A1", "A2", "FO"), c[1:4]) if v not in ("·", "")]
            rows.append((c[0].strip("`"), games))
    return rows


def report(repo):
    crates = repo / "crates"
    ui = literals(sorted((crates / "sta64/src/ui").rglob("*.rs")))
    gui = literals([crates / "sta_content/src/gui_cfg.rs"])
    # `k("backgroundpanel")` in gui_cfg.rs appends its argument to a panel prefix.
    gui_src = (crates / "sta_content/src/gui_cfg.rs").read_text()
    built = suffixes(ui) | suffixes(gui) | {m.group(1).lower() for m in re.finditer(r'\bk\("([a-z_]+)"\)', gui_src)}
    rows = []
    for key, games in read_keys(repo / "docs/data/cfg.md"):
        k = key.lower()
        if used(k, ui):
            state = "drawn"
        elif used(k, gui):
            state = "typed"
        elif any(k.endswith(s) or re.sub(r"[_.]?#$", "", k).endswith(s) for s in built):
            state = "built"
        else:
            state = "missing"
        rows.append((panel_of(k), key, games, state))
    rows.sort(key=lambda r: (r[0], r[1].lower()))

    out = ["# Stock HUD cfg audit (UI-48)", ""]
    out += [
        "Generated by `tools/hud_cfg_audit.py` from the `gui_*.cfg` key list of",
        "[data/cfg.md](data/cfg.md) and the source; do not edit by hand. Digit runs in a key are `#`.",
        "",
        "| state | meaning |",
        "|---|---|",
        "| drawn | the key is a string literal in `crates/sta64/src/ui/` |",
        "| typed | the key is a literal in `crates/sta_content/src/gui_cfg.rs` only |",
        "| built | the key ends in a suffix the code appends to a panel or element name; which prefixes reach it is not checked |",
        "| missing | none of the above |",
        "",
        "A state says the name is used, not that the element is drawn correctly.",
        "Annotated missing rows record known implementation limits and still count as missing.",
        "Games: the stock data of Armada I (A1), Armada II (A2) and Fleet Operations (FO) that set the key.",
        "",
        "## Summary",
        "",
        "| panel | keys | " + " | ".join(STATES) + " |",
        "|---|---:|" + "---:|" * len(STATES),
    ]
    by_panel = OrderedDict()
    for p, _, _, s in rows:
        by_panel.setdefault(p, Counter())[s] += 1
    total = Counter()
    for p, c in by_panel.items():
        total.update(c)
        out.append(f"| {p} | {sum(c.values())} | " + " | ".join(str(c[s]) for s in STATES) + " |")
    out.append(f"| **all** | **{sum(total.values())}** | " + " | ".join(f"**{total[s]}**" for s in STATES) + " |")
    out += ["", "By game (a key set by several games counts in each):", "", "| game | keys | " + " | ".join(STATES) + " |", "|---|---:|" + "---:|" * len(STATES)]
    for g in ("A1", "A2", "FO"):
        c = Counter(s for _, _, games, s in rows if g in games)
        out.append(f"| {g} | {sum(c.values())} | " + " | ".join(str(c[s]) for s in STATES) + " |")
    for p in by_panel:
        out += ["", f"## {p}", "", "| key | games | state |", "|---|---|---|"]
        for q, key, games, state in rows:
            if q == p:
                reason = MISSING_REASONS.get(key.lower()) if state == "missing" else None
                detail = f"{state} ({reason})" if reason else state
                out.append(f"| `{key}` | {' '.join(games)} | {detail} |")
    return "\n".join(out) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default=".")
    ap.add_argument("--check", action="store_true", help="fail if the report on disk is stale")
    a = ap.parse_args()
    repo = Path(a.repo)
    text = report(repo)
    path = repo / "docs/STOCK_HUD_CFG_AUDIT.md"
    if a.check:
        if not path.exists() or path.read_text() != text:
            print(f"{path} is stale: run tools/hud_cfg_audit.py", file=sys.stderr)
            return 1
        return 0
    path.write_text(text)
    print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
