#!/usr/bin/env python3
"""TODO.md plus its per-branch fragments in todo.d/ (see todo.d/README.md).

    tools/todo.py [list] [--all] [PATTERN]   open check lines of TODO.md and todo.d/, as file:line: text
    tools/todo.py check                      fragments are well formed and their headings exist in TODO.md
    tools/todo.py fold [--dry-run]           move every fragment's lines into TODO.md and delete the fragments

A fragment is TODO.md's format: `## ` / `### ` headings copied exactly from
TODO.md, check lines under them. Lines before the first heading belong to
DEFAULT_SECTION. `fold` appends each section's lines at the end of that section's
own text in TODO.md (before the next heading of any level). Standard library only.
Self-test: python3 tools/test_todo.py
"""

import argparse
import pathlib
import re
import sys

DEFAULT_SECTION = "## Needs your check"
FRAGMENT_DIR = "todo.d"
RESERVED = {"README.md"}
CHECK = re.compile(r"^\s*- \[(.)\]")
OPEN_MARKS = {" ", "~", "?"}


def is_heading(line):
    return re.match(r"^#{1,6} ", line) is not None


def fragments(root):
    d = root / FRAGMENT_DIR
    if not d.is_dir():
        return []
    return sorted(p for p in d.glob("*.md") if p.name not in RESERVED and p.is_file())


def parse_fragment(text):
    """Ordered list of (heading, lines); blank lines at the ends of a section dropped."""
    sections = []
    current, lines = DEFAULT_SECTION, []
    for line in text.splitlines():
        if is_heading(line):
            sections.append((current, lines))
            current, lines = line.rstrip(), []
        else:
            lines.append(line.rstrip())
    sections.append((current, lines))
    out = []
    for heading, body in sections:
        while body and not body[0].strip():
            body.pop(0)
        while body and not body[-1].strip():
            body.pop()
        if body:
            out.append((heading, body))
    return out


def read(path):
    try:
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError as e:
        raise SystemExit(f"{path}: not UTF-8 ({e})")


def problems(root):
    todo = read(root / "TODO.md").splitlines()
    headings = {line.rstrip() for line in todo if is_heading(line)}
    found = []
    d = root / FRAGMENT_DIR
    if d.is_dir():
        for other in sorted(d.iterdir()):
            if other.name not in RESERVED and (other.suffix != ".md" or not other.is_file()):
                found.append(f"{other.relative_to(root)}: not a .md file, never listed or folded")
    for frag in fragments(root):
        rel = frag.relative_to(root)
        for heading, body in parse_fragment(read(frag)):
            if heading not in headings:
                found.append(f"{rel}: heading not in TODO.md: {heading!r}")
            if body and not CHECK.match(body[0]):
                found.append(f"{rel}: under {heading!r} the first line is not a check line: {body[0]!r}")
    return found


def cmd_list(root, show_all, pattern):
    rx = re.compile(pattern, re.IGNORECASE) if pattern else None
    for path in [root / "TODO.md"] + fragments(root):
        rel = path.relative_to(root)
        for n, line in enumerate(read(path).splitlines(), 1):
            m = CHECK.match(line)
            if not m or (not show_all and m.group(1) not in OPEN_MARKS):
                continue
            if rx and not rx.search(line):
                continue
            print(f"{rel}:{n}: {line.strip()}")
    return 0


def section_end(lines, heading):
    """Index after the last non-blank line of the section's own text, or None."""
    try:
        start = next(i for i, l in enumerate(lines) if l.rstrip() == heading)
    except StopIteration:
        return None
    end = next((i for i in range(start + 1, len(lines)) if is_heading(lines[i])), len(lines))
    while end > start + 1 and not lines[end - 1].strip():
        end -= 1
    return end


def cmd_fold(root, dry_run):
    found = problems(root)
    if found:
        print("\n".join(found), file=sys.stderr)
        print("fold: nothing changed", file=sys.stderr)
        return 1
    todo_path = root / "TODO.md"
    lines = read(todo_path).splitlines()
    frags = fragments(root)
    for frag in frags:
        for heading, body in parse_fragment(read(frag)):
            end = section_end(lines, heading)
            lines[end:end] = body
            print(f"{frag.relative_to(root)}: {len(body)} line(s) -> {heading}")
    if dry_run:
        return 0
    todo_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    for frag in frags:
        frag.unlink()
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parent.parent)
    sub = ap.add_subparsers(dest="cmd")
    ls = sub.add_parser("list")
    ls.add_argument("--all", action="store_true", help="also done [x] lines")
    ls.add_argument("pattern", nargs="?", help="case-insensitive regular expression")
    sub.add_parser("check")
    fold = sub.add_parser("fold")
    fold.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)
    root = args.root
    if args.cmd in (None, "list"):
        return cmd_list(root, getattr(args, "all", False), getattr(args, "pattern", None))
    if args.cmd == "check":
        found = problems(root)
        print("\n".join(found) if found else "todo.d: ok", file=sys.stderr if found else sys.stdout)
        return 1 if found else 0
    return cmd_fold(root, args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
