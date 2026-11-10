#!/usr/bin/env python3
"""Stage clean-room A2 Lua ports into a new missions directory."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys


MAP_TOKEN = re.compile(
    r"(?<![A-Za-z0-9_.-])(?P<map>a2_[A-Za-z0-9_-]+\.bzn)"
    r"(?=$|[^A-Za-z0-9_.-]|[.](?![A-Za-z0-9_.-]))",
    re.IGNORECASE,
)


def _validate_source_name(name):
    if not name or "/" in name or "\\" in name or Path(name).name != name:
        raise ValueError(f"source filename is not a single safe name: {name!r}")


def _map_for(data, source_name):
    try:
        header = b"\n".join(data.splitlines()[:8]).decode("utf-8-sig").splitlines()
    except UnicodeDecodeError as exc:
        raise ValueError(f"{source_name}: first eight lines are not UTF-8") from exc
    maps = {}
    for line in header:
        for match in MAP_TOKEN.finditer(line):
            maps.setdefault(match.group("map").casefold(), match.group("map"))
    if len(maps) != 1:
        reason = "missing" if not maps else "ambiguous"
        raise ValueError(f"{source_name}: {reason} A2 BZN map token in first eight lines")
    return next(iter(maps.values()))


def _inside(path, root):
    try:
        return os.path.commonpath((str(path), str(root))) == str(root)
    except ValueError:
        return False


def _output_paths(source_name, map_name):
    original = f"missions/{source_name}"
    alias = f"missions/{map_name[:-4].lower()}.lua"
    if original.casefold() == alias.casefold():
        return [original]
    return [original, alias]


def _read_source(entry):
    before = os.stat(entry.path, follow_symlinks=False)
    if not stat.S_ISREG(before.st_mode):
        raise ValueError(f"{entry.name}: source must be a regular file, not a symlink")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    with os.fdopen(os.open(entry.path, flags), "rb") as stream:
        opened = os.fstat(stream.fileno())
        if not stat.S_ISREG(opened.st_mode) or (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
            raise ValueError(f"{entry.name}: source changed while opening")
        return stream.read()


def stage(ports, out):
    """Validate and copy top-level Lua ports into a new output directory.

    Returns the manifest object after writing it last. The source tree is only
    read; a failed write may leave an incomplete new destination without a manifest.
    """
    source_root = Path(ports).resolve(strict=True)
    if not source_root.is_dir():
        raise ValueError(f"source missions path is not a directory: {source_root}")

    requested_out = Path(out)
    if os.path.lexists(requested_out):
        raise FileExistsError(f"destination already exists: {requested_out}")
    parent = requested_out.parent.resolve(strict=True)
    if not parent.is_dir():
        raise ValueError(f"destination parent is not a directory: {parent}")
    destination = parent / requested_out.name
    if _inside(destination, source_root):
        raise ValueError("destination must be outside the source missions directory")
    if os.path.lexists(destination):
        raise FileExistsError(f"destination already exists: {destination}")

    sources = []
    with os.scandir(source_root) as entries:
        for entry in sorted(entries, key=lambda item: (item.name.casefold(), item.name)):
            if Path(entry.name).suffix.casefold() != ".lua":
                continue
            if entry.is_symlink():
                raise ValueError(f"{entry.name}: source symlinks are not supported")
            if not entry.is_file(follow_symlinks=False):
                continue
            _validate_source_name(entry.name)
            data = _read_source(entry)
            if not data:
                raise ValueError(f"{entry.name}: source Lua file is empty")
            map_name = _map_for(data, entry.name)
            sources.append((entry.name, map_name, data))
    if not sources:
        raise ValueError("source missions directory contains no top-level Lua files")

    seen_maps = {}
    seen_outputs = {}
    plan = []
    for source_name, map_name, data in sources:
        map_key = map_name.casefold()
        if map_key in seen_maps:
            raise ValueError(f"duplicate BZN map {map_name!r} in {seen_maps[map_key]!r} and {source_name!r}")
        seen_maps[map_key] = source_name

        paths = _output_paths(source_name, map_name)
        for output_path in paths:
            output_key = output_path.casefold()
            previous = seen_outputs.get(output_key)
            if previous is not None and previous != source_name:
                raise ValueError(f"output path collision: {previous!r} and {source_name!r} -> {output_path!r}")
            seen_outputs[output_key] = source_name
        plan.append((source_name, map_name, data, paths))

    # All validation is complete before this exclusive creation.
    os.mkdir(destination)
    missions_dir = destination / "missions"
    os.mkdir(missions_dir)
    manifest_entries = []
    for source_name, map_name, data, paths in plan:
        digest = hashlib.sha256(data).hexdigest()
        outputs = []
        for relative in paths:
            output = destination / relative
            with output.open("xb") as stream:
                stream.write(data)
            outputs.append({"path": relative, "sha256": digest, "bytes": len(data)})
        manifest_entries.append(
            {"source": source_name, "map": map_name, "sha256": digest, "bytes": len(data), "outputs": outputs}
        )

    manifest = {"entries": manifest_entries, "complete": True}
    with (destination / "manifest.json").open("xb") as stream:
        stream.write((json.dumps(manifest, indent=2) + "\n").encode("utf-8"))
    return manifest


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ports", required=True, help="source missions directory")
    parser.add_argument("--out", required=True, help="new destination directory")
    args = parser.parse_args(argv)
    try:
        manifest = stage(args.ports, args.out)
    except (OSError, ValueError) as exc:
        parser.error(str(exc))
    output_count = sum(len(entry["outputs"]) for entry in manifest["entries"])
    print(f"Staged {len(manifest['entries'])} Lua ports as {output_count} files")


if __name__ == "__main__":
    main()
