# odf_fmt: the ODF formatter (EDT-06)

Binary `odf_fmt` of the `sta_tools` crate (`crates/sta_tools/src/bin/odf_fmt.rs`,
logic in `crates/sta_tools/src/odf_fmt.rs`); it formats through the lossless
syntax tree in `sta_odf` (`crates/sta_odf/src/cst.rs`, grouped layout in
`cst/layout.rs`). `sta_tools` is a light crate (no Bevy); the windowed tool shell
of EDT-01 is not in it yet.

```sh
tools/agent-cargo --light run -p sta_tools --bin odf_fmt -- [--grouped] [--groups FILE] [--write | --check] [--report FILE] PATH...
```

`PATH` is an `.odf` file or a folder searched recursively (symlinks are skipped).

| Mode | Does |
|---|---|
| (none) | Dry run: lists the files that would change. Exit 1 if any would. |
| `--write` | Rewrites changed files atomically (temp file beside it, same permissions, fsync, rename). |
| `--check` | Writes nothing. Verifies per file that the tree rebuilds it byte for byte, that formatting is idempotent, and that includes, keys and values read the same afterwards. Failures name the file and the key, never content. Exit 1 on any failure. |
| `--grouped` | Also gathers the entries by key group (below). Combines with the modes above. |
| `--groups FILE` | `--grouped` with your own table instead of the built-in one. |
| `--report FILE` | JSON summary (files, mode, changed, errors, failed). |

Exit status: 0 nothing to change / written / checks passed, 1 files would change
or a check failed, 2 usage or I/O error.

## What it changes

* Whitespace only: `key = value` spacing, trailing blanks, one space before a
  trailing `//` comment, runs of blank lines collapsed. Values, comment text,
  key order, `#include` rows, continuation rows, encodings (a BOM, bytes that are
  not UTF-8) and **each file's own line endings** (CRLF stays CRLF, LF stays LF,
  a missing final newline stays missing) are untouched.
* `--grouped` additionally reorders entries by group, with one blank line between
  groups. An entry moves with the comment lines directly above it and the
  continuation rows below it; a comment set off by a blank line stays under the
  entry above it (or at the top). Entries never cross a `#` directive or a
  `[section]` row. Keys the table does not name form the last group, in file
  order. A file where a key occurs twice (any case) or with a line that is not
  UTF-8 is only whitespace-formatted, because order decides which duplicate wins.

## The group table

`crates/sta_odf/src/cst/fo_groups.txt` (key names only, format described in its
header): identity, interface, cost and production, durability, crew, armament,
movement, behaviour, events and sound, appearance. It was written for STA64 from
what the keys do. The Fleet Operations guide's own templates (guide.fleetops.net
in the FO readme) are not in the repository, not in any install and the site was
not reachable, so the guide's exact order is **not** reproduced: when someone
supplies it, only this file changes.

## Round-trip check over installs

```sh
export STA64_TEST_INSTALLS='/path/to/install1:/path/to/install2'   # PATH-style list
tools/agent-cargo --light test -p sta_tools --test install_round_trip -- --nocapture
```

Mounts each install bare and once per mod under `Data/Mods`, takes every layer
copy of every ODF once, and asserts the checks above for the whitespace format
and the grouped layout plus that `resolve_layered` returns the same values from
grouped copies as from originals. It prints per-install counts. Without the
variable (`STA64_TEST_INSTALL` is the single-root fallback) it does nothing; CI
runs the fixture tests (`tests/fixtures.rs`, unit tests) which use files written
for the repository. Game files are read at run time and never copied.
