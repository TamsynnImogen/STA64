#!/usr/bin/env python3
"""Inventory every data feature STA64 must understand in Armada I, Armada II
and Fleet Operations: ODF keys and class labels, techtree (.tt / .xml) forms,
.cfg keys and .aip variables. Writes Markdown + CSV reports.

Each game is one or more roots (folders, searched recursively). Armada I's
`.zfs` archives found in a root are read too (repeating 4-byte XOR with the
header key). Fleet Operations' `ODF.fpq` must be extracted first:

    cargo run -p sta_fpq --release --example fpq -- <FO>/Data/ODF.fpq extract <dir>/odf

then pass <dir> as a second FO root. Earlier roots win when the same relative
path exists twice (so list the loose install first).

usage:
  data_inventory.py --out docs/data --repo . \
      --game "A1=<A1 install>" --game "A2=<A2 install>" \
      --game "FO=<FO>/Data,<fpq extract dir>"

Only names, counts and short sample values are recorded; no file is copied.
"""

import argparse
import csv
import re
import struct
import sys
from collections import Counter, defaultdict
from pathlib import Path

TYPES = ("odf", "tt", "xml", "cfg", "aip")
DEFAULT_EXCLUDE = ("/mods/", "reshade-shaders", "_backup", "/backups/", "/save/")

# ---------------------------------------------------------------- sources


def read_zfs(path: Path):
    """Yield (name, bytes) from an Armada I ZFS archive."""
    b = path.read_bytes()
    magic, _ver, maxn, per, count, key, first = struct.unpack_from("<4s6I", b, 0)
    if magic != b"ZFSF":
        return
    k = struct.pack("<I", key)
    off, n = first, 0
    while off and n < count:
        (nxt,) = struct.unpack_from("<I", b, off)
        o = off + 4
        for _ in range(per):
            name = b[o : o + maxn].split(b"\0")[0].decode("latin1")
            fo, _idx, size, _t, flags = struct.unpack_from("<5I", b, o + maxn)
            o += maxn + 20
            if not name:
                continue
            n += 1
            data = b[fo : fo + size]
            if key:
                # Whole words are XOR'd with the key; the last size % 4 bytes are plain.
                whole = len(data) & ~3
                data = bytes(c ^ k[i & 3] for i, c in enumerate(data[:whole])) + data[whole:]
            if flags:
                print(f"warning: {path.name}:{name} compressed (flags {flags}), skipped", file=sys.stderr)
                continue
            yield name, data
        off = nxt


def collect(roots, exclude):
    """{type: {relpath_lower: (display_path, text)}} for one game."""
    files = {t: {} for t in TYPES}
    for root in roots:
        root = Path(root)
        for p in sorted(root.rglob("*")):
            if not p.is_file():
                continue
            rel = p.relative_to(root).as_posix()
            low = "/" + rel.lower()
            if any(x in low for x in exclude):
                continue
            ext = p.suffix.lower().lstrip(".")
            if ext == "zfs":
                for name, data in read_zfs(p):
                    e = name.rsplit(".", 1)[-1].lower()
                    if e in files:
                        key = f"{p.name}/{name}".lower()
                        files[e].setdefault(key, (f"{p.name}:{name}", data.decode("latin1")))
            elif ext in files:
                files[ext].setdefault(rel.lower(), (rel, p.read_bytes().decode("latin1")))
    return files


# ---------------------------------------------------------------- parsing


def strip_comment(line: str) -> str:
    out, q = [], False
    i = 0
    while i < len(line):
        c = line[i]
        if c == '"':
            q = not q
        elif not q and line.startswith("//", i):
            break
        out.append(c)
        i += 1
    return "".join(out).strip()


def strip_block_comments(text: str) -> str:
    return re.sub(r"/\*.*?\*/", " ", text, flags=re.S)


NUM = re.compile(r"^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?[fF]?$")
INT = re.compile(r"^[-+]?\d+$")


def kind_of(value: str) -> str:
    v = value.strip().strip(";").strip()
    if not v:
        return "empty"
    toks = v.split()
    if v.startswith('"'):
        return "string" if v.count('"') <= 2 else "string list"
    if len(toks) == 1:
        if INT.match(v):
            return "int"
        if NUM.match(v):
            return "float"
        if v.lower() in ("true", "false", "yes", "no", "on", "off"):
            return "bool"
        return "name"
    if all(NUM.match(t) for t in toks):
        return f"{len(toks)} numbers" if len(toks) <= 4 else "number list"
    return "list"


def shape(line: str) -> str:
    """Abstract shape of an unparsed line, for the 'other syntax' tables."""
    s = re.sub(r'"[^"]*"', '"…"', line)
    s = re.sub(r"[-+]?\d+(\.\d+)?", "N", s)
    s = re.sub(r"[A-Za-z_][\w.]*", "w", s)
    return re.sub(r"\s+", " ", s)[:40]


ASSIGN = re.compile(r"^([A-Za-z_][\w.\[\]]*)\s*=\s*(.*)$")
CDECL = re.compile(r"^(?:(const|static|unsigned|signed)\s+)*(int|float|char|bool|double|long|short|string)\s+([A-Za-z_]\w*)\s*(\[[^\]]*\])?\s*=\s*(.*?);?$")
INCLUDE = re.compile(r'^#\s*include\s+[<"]?([^">\s]+)[">]?')
DIRECTIVE = re.compile(r"^#\s*(include|define|undef|ifdef|ifndef|if|elif|else|endif|pragma)\b", re.I)
HASH_COMMENT = re.compile(r"^#")
CONT = re.compile(r'^"[^"]*"')


def collapse(key: str) -> str:
    return re.sub(r"\d+", "#", key)


class Stat:
    __slots__ = ("files", "uses", "samples", "kinds", "spell", "where")

    def __init__(self):
        self.files = 0
        self.uses = 0
        self.samples = Counter()
        self.kinds = Counter()
        self.spell = Counter()
        self.where = Counter()

    def add(self, spelling, value, where=None):
        self.uses += 1
        self.spell[spelling] += 1
        v = value.strip().rstrip(";").strip()
        if len(v) > 40:
            v = v[:37] + "…"
        self.samples[v] += 1
        self.kinds[kind_of(value)] += 1
        if where:
            self.where[where] += 1


def odf_scan(files):
    keys = defaultdict(Stat)  # collapsed lower key -> Stat
    raw_keys = Counter()
    directives = Counter()
    other = Counter()
    other_ex = {}
    parsed = {}  # basename lower -> (includes, {key_lower: value})
    tables = defaultdict(Counter)  # collapsed key -> game rows (files with rows counted)
    for rel, (disp, text) in files.items():
        incs, own = [], {}
        last = None
        for ln in strip_block_comments(text).splitlines():
            s = strip_comment(ln)
            if not s:
                continue
            if CONT.match(s) and last and ln[:1] in " \t\"":
                f = "table rows after `key = …` (`\"name\" value…`)"
                other[f] += 1
                other_ex.setdefault(f, f"{disp}: {last} = … / {s[:50]}")
                tables[collapse(last.lower())][rel] += 1
                continue
            m = INCLUDE.match(s)
            if m:
                incs.append(m.group(1).lower().rsplit("/", 1)[-1].rsplit("\\", 1)[-1])
                directives["#include"] += 1
                continue
            m = DIRECTIVE.match(s)
            if m:
                directives["#" + m.group(1).lower()] += 1
                continue
            if HASH_COMMENT.match(s):
                directives["# comment line"] += 1
                continue
            m = ASSIGN.match(s)
            if m:
                own[m.group(1).lower()] = (m.group(1), m.group(2))
                last = m.group(1)
                continue
            sh = shape(s)
            other[sh] += 1
            other_ex.setdefault(sh, f"{disp}: {s[:60]}")
        parsed[rel.rsplit("/", 1)[-1]] = (incs, own)

    def class_of(name, seen=()):
        if name not in parsed or name in seen:
            return None
        incs, own = parsed[name]
        if "classlabel" in own:
            return own["classlabel"][1].strip().strip('"').lower()
        for inc in reversed(incs):
            c = class_of(inc, seen + (name,))
            if c:
                return c
        return None

    classes = Counter()
    class_keys = defaultdict(Counter)
    for name, (incs, own) in parsed.items():
        cl = class_of(name) or "(none)"
        classes[cl] += 1
        for kl, (spell, val) in own.items():
            ck = collapse(kl)
            st = keys[ck]
            st.files += 1
            st.add(collapse(spell), val, cl)
            raw_keys[kl] += 1
            class_keys[cl][ck] += 1
    return dict(keys=keys, raw=raw_keys, classes=classes, class_keys=class_keys, tables=tables,
                directives=directives, other=other, other_ex=other_ex, nfiles=len(files))


TT_ENTRY = re.compile(r"^([\w.\-]+?)(\.odf)?\s+(-?\d+)\s*(.*)$", re.I)


def tt_scan(files):
    forms = Counter()
    ex = {}
    entries = 0
    deps = Counter()
    for rel, (disp, text) in files.items():
        for ln in strip_block_comments(text).splitlines():
            s = strip_comment(ln)
            if not s:
                continue
            m = INCLUDE.match(s)
            if m:
                f = "#include <file>"
            else:
                m = TT_ENTRY.match(s)
                if m:
                    entries += 1
                    n = int(m.group(3))
                    rest = m.group(4).split()
                    if not m.group(2):
                        f = "<name without .odf> N …  (A2 tech flags such as `gTechAssimBeam`, odd entries)"
                    elif n < 0:
                        f = "<odf> -1  (unbuildable; mission techtrees use it to withhold items)"
                    elif n == 0:
                        f = "<odf> 0  (buildable from the start)"
                    else:
                        f = "<odf> N <prereq.odf> ×N  (all N required)"
                        deps[n] += 1
                    if n >= 0 and len(rest) != max(n, 0):
                        f = "<odf> N <deps…>  (count ≠ listed deps)"
                else:
                    f = "other: " + shape(s)
            forms[f] += 1
            ex.setdefault(f, f"{disp}: {s[:70]}")
    return dict(forms=forms, ex=ex, entries=entries, deps=deps, nfiles=len(files))


def xml_scan(files):
    import xml.etree.ElementTree as ET

    elems = defaultdict(Stat)  # path
    attrs = defaultdict(Stat)  # path@attr
    errors = []
    for rel, (disp, text) in files.items():
        try:
            root = ET.fromstring(text)
        except ET.ParseError as e:
            errors.append(f"{disp}: {e}")
            continue

        def walk(el, path):
            p = f"{path}/{el.tag}" if path else el.tag
            st = elems[p]
            st.add(el.tag, (el.text or "").strip(), disp)
            for a, v in el.attrib.items():
                attrs[f"{p}@{a}"].add(a, v, disp)
            for c in el:
                walk(c, p)

        walk(root, "")
    return dict(elems=elems, attrs=attrs, errors=errors, nfiles=len(files))


def cfg_group(rel: str) -> str:
    base = rel.rsplit("/", 1)[-1].rsplit(":", 1)[-1].lower()
    stem = base.rsplit(".", 1)[0]
    if stem.startswith("gui_"):
        return "gui_*.cfg"
    return base


def decl_scan(files, group_fn):
    """For .cfg/.aip: both `key = value` and C-style `type NAME = value;`."""
    keys = defaultdict(Stat)  # (group, key lower) -> Stat
    types = defaultdict(Counter)
    directives = Counter()
    other = Counter()
    other_ex = {}
    groups = Counter()
    for rel, (disp, text) in files.items():
        g = group_fn(rel)
        groups[g] += 1
        seen = set()
        for ln in strip_block_comments(text).splitlines():
            s = strip_comment(ln)
            if not s:
                continue
            m = INCLUDE.match(s)
            if m:
                directives["#include"] += 1
                continue
            m = DIRECTIVE.match(s)
            if m:
                directives["#" + m.group(1).lower()] += 1
                continue
            if HASH_COMMENT.match(s):
                directives["# comment line"] += 1
                continue
            m = CDECL.match(s)
            if m:
                name, val = m.group(3), m.group(5)
                t = m.group(2) + ("[]" if m.group(4) else "")
                k = (g, collapse(name.lower()))
                types[k][t] += 1
            else:
                m = ASSIGN.match(s)
                if not m:
                    sh = shape(s)
                    other[(g, sh)] += 1
                    other_ex.setdefault((g, sh), f"{disp}: {s[:60]}")
                    continue
                name, val = m.group(1), m.group(2)
                k = (g, collapse(name.lower()))
                types[k]["key = value"] += 1
            st = keys[k]
            if k not in seen:
                st.files += 1
                seen.add(k)
            st.add(collapse(name), val)
    return dict(keys=keys, types=types, directives=directives, other=other,
                other_ex=other_ex, groups=groups, nfiles=len(files))


# ---------------------------------------------------------------- STA64 usage


def sta64_literals(repo: Path):
    lits = set()
    for f in (repo / "crates").rglob("*.rs"):
        src = f.read_text(errors="replace")
        # Char literals holding a quote would throw string pairing off.
        src = src.replace("'\"'", "").replace("'\\\"'", "").replace("b'\"'", "")
        for m in re.finditer(r'"((?:[^"\\]|\\.)*)"', src):
            s = m.group(1).lower()
            if 1 < len(s) < 64:
                lits.add(s)
    return lits


def used(key: str, lits) -> bool:
    key = key.lower()  # literals are lower-cased; keys keep their spelling
    if key in lits:
        return True
    if "#" in key:
        rx = re.compile("^" + re.escape(key).replace("\\#", r"(\d+|\{\}|\{\w*\})") + "$")
        pre = key.split("#", 1)[0]
        return any(rx.match(s) for s in lits) or (len(pre) > 3 and pre in lits)
    return False


# ---------------------------------------------------------------- output


def md_cell(s) -> str:
    return str(s).replace("|", "\\|").replace("\n", " ")


def top(counter, n=3):
    return ", ".join(f"`{md_cell(k)}`" for k, _ in counter.most_common(n) if k != "")


def write(path: Path, lines):
    path.write_text("\n".join(lines) + "\n")
    print(f"wrote {path}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--game", action="append", required=True, help="NAME=root[,root...]")
    ap.add_argument("--out", required=True)
    ap.add_argument("--repo", default=".")
    ap.add_argument("--exclude", action="append", default=list(DEFAULT_EXCLUDE))
    a = ap.parse_args()

    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    lits = sta64_literals(Path(a.repo))
    games = []
    data = {}
    for g in a.game:
        name, roots = g.split("=", 1)
        games.append(name)
        files = collect(roots.split(","), a.exclude)
        data[name] = dict(
            counts={t: len(files[t]) for t in TYPES},
            odf=odf_scan(files["odf"]),
            tt=tt_scan(files["tt"]),
            xml=xml_scan(files["xml"]),
            cfg=decl_scan(files["cfg"], cfg_group),
            aip=decl_scan(files["aip"], lambda r: "aip"),
        )
        print(f"{name}: " + ", ".join(f"{t} {n}" for t, n in data[name]["counts"].items()))

    G = games
    gh = " | ".join(G)
    gs = "|".join("---:" for _ in G)

    # ---- ODF keys
    allk = sorted(set().union(*(data[g]["odf"]["keys"] for g in G)))
    rows = []
    for k in allk:
        sts = [data[g]["odf"]["keys"].get(k) for g in G]
        merged = Stat()
        for st in sts:
            if st:
                merged.spell.update(st.spell)
                merged.samples.update(st.samples)
                merged.kinds.update(st.kinds)
                merged.where.update(st.where)
        rows.append((k, sts, merged))
    rows.sort(key=lambda r: (-sum(1 for s in r[1] if s), -sum(s.files for s in r[1] if s), r[0]))
    lines = [
        "# ODF keys",
        "",
        "Every key assigned in an ODF, across all games. Digit runs are collapsed to `#`",
        "(`weapon#Name`, `buildItem#`), so numbered families show once. Numbers are how",
        "many ODF files set the key. *Classes* are the resolved `classLabel`s of the ODFs",
        "that set it (most common first). *STA64* is ✓ when the key appears as a string",
        "literal in `crates/` (a rough signal that something reads it, not proof of full",
        "support). Generated by `tools/data_inventory.py`; do not edit by hand.",
        "",
        f"| key | {gh} | STA64 | kind | classes | sample values |",
        f"|---|{gs}|:-:|---|---|---|",
    ]
    with open(out / "odf_keys.csv", "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["key", *G, "sta64", "kind", "classes", "samples"])
        for k, sts, m in rows:
            spell = m.spell.most_common(1)[0][0]
            u = used(k, lits)
            cnt = [s.files if s else 0 for s in sts]
            kind = m.kinds.most_common(1)[0][0]
            lines.append(f"| `{md_cell(spell)}` | " + " | ".join(str(c) if c else "·" for c in cnt)
                         + f" | {'✓' if u else ''} | {kind} | {top(m.where, 3)} | {top(m.samples, 3)} |")
            w.writerow([spell, *cnt, int(u), kind, ";".join(c for c, _ in m.where.most_common(5)),
                        ";".join(s for s, _ in m.samples.most_common(3))])
    write(out / "odf_keys.md", lines)

    # ---- ODF classes
    allc = sorted(set().union(*(data[g]["odf"]["classes"] for g in G)),
                  key=lambda c: -sum(data[g]["odf"]["classes"].get(c, 0) for g in G))
    lines = ["# ODF class labels", "",
             "`classLabel` decides which engine class an ODF instantiates. Resolved through",
             "`#include` chains. Count = ODF files of that class. *Distinct keys* is how many",
             "different (collapsed) keys those files set; the top keys are listed.",
             "Generated by `tools/data_inventory.py`.", "",
             f"| classLabel | {gh} | STA64 | distinct keys | most-set keys |", f"|---|{gs}|:-:|---:|---|"]
    for c in allc:
        ck = Counter()
        for g in G:
            ck.update(data[g]["odf"]["class_keys"].get(c, {}))
        cnt = [data[g]["odf"]["classes"].get(c, 0) for g in G]
        lines.append(f"| `{md_cell(c)}` | " + " | ".join(str(x) if x else "·" for x in cnt)
                     + f" | {'✓' if c in lits else ''} | {len(ck)} | {top(ck, 8)} |")
    lines += ["", "## Keys per class", "",
              "Every key each class's ODFs set, with file counts (all games together).", ""]
    for c in allc:
        ck = Counter()
        for g in G:
            ck.update(data[g]["odf"]["class_keys"].get(c, {}))
        lines += [f"<details><summary><code>{c}</code> ({len(ck)} keys)</summary>", "",
                  ", ".join(f"`{md_cell(k)}` {n}" for k, n in sorted(ck.items(), key=lambda x: (-x[1], x[0]))),
                  "", "</details>", ""]
    write(out / "odf_classes.md", lines)

    # ---- techtree
    lines = ["# Techtree files (.tt and FO .xml)", "", "Generated by `tools/data_inventory.py`.", "",
             "## .tt line forms", "", f"| form | {gh} | example |", f"|---|{gs}|---|"]
    forms = sorted(set().union(*(data[g]["tt"]["forms"] for g in G)))
    for f in forms:
        ex = next((data[g]["tt"]["ex"][f] for g in G if f in data[g]["tt"]["ex"]), "")
        lines.append(f"| {md_cell(f)} | " + " | ".join(str(data[g]["tt"]["forms"].get(f, 0) or "·") for g in G)
                     + f" | `{md_cell(ex)}` |")
    lines += ["", f"| | {gh} |", f"|---|{gs}|",
              "| .tt files | " + " | ".join(str(data[g]["tt"]["nfiles"]) for g in G) + " |",
              "| ODF entries | " + " | ".join(str(data[g]["tt"]["entries"]) for g in G) + " |",
              "| max prerequisites on one entry | " + " | ".join(str(max(data[g]["tt"]["deps"] or [0])) for g in G) + " |",
              "", "## XML elements and attributes", ""]
    for g in G:
        x = data[g]["xml"]
        if not x["nfiles"]:
            continue
        lines += [f"### {g} ({x['nfiles']} files)", "", "| element path / @attribute | uses | files | sample values |", "|---|---:|---:|---|"]
        for p in sorted(set(x["elems"]) | set(x["attrs"])):
            st = x["elems"].get(p) or x["attrs"].get(p)
            lines.append(f"| `{md_cell(p)}` | {st.uses} | {len(st.where)} | {top(st.samples, 4)} |")
        for e in x["errors"]:
            lines.append(f"\n- parse error: {md_cell(e)}")
        lines.append("")
    write(out / "techtree.md", lines)

    # ---- cfg / aip
    for t, title, blurb in (
        ("cfg", "CFG keys", "`.cfg` files: GUI layout (`gui_*.cfg`, grouped) and engine tuning files. Both `key = value` and C-style `type NAME = value;` lines are read."),
        ("aip", "AIP variables", "AI player files (`.aip`): C-style `int NAME = value;` declarations, most overriding defaults from an `#include`d base AIP."),
    ):
        lines = [f"# {title}", "", blurb, "Digit runs are collapsed to `#`. Numbers are files that set the key.",
                 "Generated by `tools/data_inventory.py`.", ""]
        groups = sorted(set().union(*(data[g][t]["groups"] for g in G)))
        lines += [f"| file / group | {gh} |", f"|---|{gs}|"]
        for gr in groups:
            lines.append(f"| `{gr}` | " + " | ".join(str(data[g][t]["groups"].get(gr, 0) or "·") for g in G) + " |")
        dirs = Counter()
        for g in G:
            dirs.update(data[g][t]["directives"])
        if dirs:
            lines += ["", "Preprocessor directives: " + ", ".join(f"`{d}` ×{n}" for d, n in dirs.most_common())]
        for gr in groups:
            ks = sorted({k for g in G for (gg, k) in data[g][t]["keys"] if gg == gr})
            if not ks:
                continue
            lines += ["", f"## `{gr}`", "", f"| key | {gh} | STA64 | declared as | sample values |", f"|---|{gs}|:-:|---|---|"]
            for k in ks:
                sts = [data[g][t]["keys"].get((gr, k)) for g in G]
                sp, sm, ty = Counter(), Counter(), Counter()
                for g, st in zip(G, sts):
                    if st:
                        sp.update(st.spell)
                        sm.update(st.samples)
                        ty.update(data[g][t]["types"][(gr, k)])
                lines.append(f"| `{md_cell(sp.most_common(1)[0][0])}` | " + " | ".join(str(s.files) if s else "·" for s in sts)
                             + f" | {'✓' if used(k, lits) else ''} | {', '.join(ty)} | {top(sm, 3)} |")
        oth = Counter()
        ex = {}
        for g in G:
            oth.update(data[g][t]["other"])
            ex.update(data[g][t]["other_ex"])
        if oth:
            lines += ["", "## Other line shapes", "", "Lines that are neither assignments nor directives (`w` word, `N` number).", "",
                      "| group | shape | count | example |", "|---|---|---:|---|"]
            for (gr, sh), n in oth.most_common(60):
                lines.append(f"| `{gr}` | `{md_cell(sh)}` | {n} | `{md_cell(ex[(gr, sh)])}` |")
        write(out / f"{t}.md", lines)

    # ---- README / summary
    lines = ["# Data feature inventory", "",
             "What Armada I, Armada II and Fleet Operations put in their text data files,",
             "so STA64 can track what it reads and what it still ignores.", "",
             "| report | contents |", "|---|---|",
             "| [odf_keys.md](odf_keys.md) ([csv](odf_keys.csv)) | every ODF key, per game, with classes, value kind, samples, STA64 ✓ |",
             "| [odf_classes.md](odf_classes.md) | every `classLabel` and the keys its ODFs set |",
             "| [techtree.md](techtree.md) | `.tt` line forms and FO's techtree `.xml` schema |",
             "| [cfg.md](cfg.md) | `.cfg` keys (GUI layouts and engine tuning) |",
             "| [aip.md](aip.md) | `.aip` AI player variables |",
             "", "## Files scanned", "", f"| type | {gh} |", f"|---|{gs}|"]
    for t in TYPES:
        lines.append(f"| .{t} | " + " | ".join(str(data[g]["counts"][t]) for g in G) + " |")
    lines += ["", "## Coverage", "", f"| | {gh} |", f"|---|{gs}|"]

    def cov(g, t):
        ks = data[g][t]["keys"]
        names = [k if t == "odf" else k[1] for k in ks]
        u = sum(1 for k in names if used(k, lits))
        return f"{u} / {len(names)}"

    lines.append("| distinct ODF keys (STA64 ✓ / all) | " + " | ".join(cov(g, "odf") for g in G) + " |")
    lines.append("| ODF class labels | " + " | ".join(str(len(data[g]["odf"]["classes"])) for g in G) + " |")
    lines.append("| distinct CFG keys (✓ / all) | " + " | ".join(cov(g, "cfg") for g in G) + " |")
    lines.append("| distinct AIP variables (✓ / all) | " + " | ".join(cov(g, "aip") for g in G) + " |")
    excl = {}
    for g in G:
        for k in data[g]["odf"]["keys"]:
            excl.setdefault(k, set()).add(g)
    lines += ["", "Keys found only in one game: " + ", ".join(
        f"{g} {sum(1 for s in excl.values() if s == {g})}" for g in G) + "."]

    lines += ["", "## ODF syntax beyond `key = value`", "",
              "| | " + gh + " |", f"|---|{gs}|"]
    dk = sorted(set().union(*(data[g]["odf"]["directives"] for g in G)))
    for d in dk:
        lines.append(f"| `{d}` | " + " | ".join(str(data[g]["odf"]["directives"].get(d, 0) or "·") for g in G) + " |")
    oth = Counter()
    ex = {}
    for g in G:
        oth.update(data[g]["odf"]["other"])
        for k, v in data[g]["odf"]["other_ex"].items():
            ex.setdefault(k, v)
    if oth:
        lines += ["", "Unparsed ODF line shapes (`w` word, `N` number), most common first:", "",
                  "| shape | count | example |", "|---|---:|---|"]
        for sh, n in oth.most_common(25):
            lines.append(f"| `{md_cell(sh)}` | {n} | `{md_cell(ex[sh])}` |")

    tk = sorted(set().union(*(data[g]["odf"]["tables"] for g in G)))
    if tk:
        lines += ["", "## ODF keys followed by table rows", "",
                  "A key whose value is followed by indented `\"name\" value…` rows (FO's",
                  "`validtargets`/`supplyclasses` ODF tables, A1/A2 ship-name lists). Count = files.", "",
                  f"| key | {gh} |", f"|---|{gs}|"]
        for k in sorted(tk, key=lambda k: -sum(len(data[g]["odf"]["tables"].get(k, {})) for g in G)):
            lines.append(f"| `{k}` | " + " | ".join(str(len(data[g]["odf"]["tables"].get(k, {})) or "·") for g in G) + " |")

    lines += ["", "## Syntax notes", "",
              "- FO ODFs never `#include`; every ODF is flat. A1/A2 chain bases with `#include`.",
              "- `#` at the start of a line is a comment in A1/A2 ODFs (only real directives count above).",
              "- `events.odf` (A2/FO) is its own block format: `Name{ … }` with `Sound <race> \"x.wav\"`,",
              "  `Text`, `Marker …`, `SoundPriority N` lines. It needs a dedicated parser.",
              "- A1 `aster.odf`-style files carry BZ-engine `[GameObjectClass]` section headers.",
              "- AIP build lists are C-initialiser rows (`\"bor_assemblerZ\", 1,0, -1, 1,`) under an array",
              "  declaration; see the *Other line shapes* table in [aip.md](aip.md).",
              "- FO techtrees exist twice: `.tt` (A2 style) and `.xml` (capacity lists, requirement",
              "  trees with `mustNotHave`, ally races, caps). FO reads the XML.",
              "- `.tt` entry = `<odf> <N> <prereq>×N`; A2 mission techtrees `#include` a base and",
              "  set items to `-1`."]
    lines += ["", "## Sources and regeneration", "",
              "- Armada I: retail install; most ODFs and all techtrees are in `assets.zfs`",
              "  (ZFS v1, 100-entry blocks, 16-char names, whole words XOR'd with the 4-byte header key,",
              "  read by the script). `addon/` ODFs are loose.",
              "- Armada II: retail 1.1 install, loose files.",
              "- Fleet Operations: loose `Data/` plus `ODF.fpq` (extracted with `sta_fpq`'s",
              "  example). `Data/Mods/` (third-party mods), backups and saves are excluded.",
              "", "```sh", "cargo run -p sta_fpq --release --example fpq -- \"$FO/Data/ODF.fpq\" extract /tmp/fpq/odf",
              "python3 tools/data_inventory.py --out docs/data --repo . \\",
              "    --game \"A1=$A1\" --game \"A2=$A2\" --game \"FO=$FO/Data,/tmp/fpq\"", "```"]
    write(out / "README.md", lines)


if __name__ == "__main__":
    main()
