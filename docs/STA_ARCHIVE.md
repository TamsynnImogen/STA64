# STA64 archive format (`.sta`), version 1.0 — draft

One `.sta` file holds one mod: a full conversion or a small add-on, with its
ships, stations, textures, maps, scripts, audio, UI and translations. STA64
mounts it directly, reading each file on demand without unpacking the
archive. The same file works on every platform.

This is STA64's native counterpart of Fleet Operations' `.fpq` and Armada I's
`.zfs`. Unlike those it is **open**: this document is the whole format, and
anyone may write tools that read or create it (see "Licence").

Status: **draft**. Nothing here is implemented yet; the packages that build it
are CON-16 to CON-19 and EDT-08 in [WORKPLAN.md](WORKPLAN.md). Until a release
freezes 1.0, details may change; every change is recorded under "Changes".

## Goals

| Goal | How |
|---|---|
| Self-contained | One file is a whole mod; `manifest.toml` says what it is and what it needs. |
| Mountable | An index at a known place lists every file with its offset; each file is compressed on its own, so one read gives one file. |
| Cross-platform | Little-endian, UTF-8 paths with `/`, no platform metadata, case-insensitive lookup on every OS. |
| Add-ons as well as conversions | Dependencies and load order; a mod overrides or extends files of the mods below it, and can hide them. |
| Trustworthy | SHA-256 of every file and of the index; optional Ed25519 signature. |
| Legacy and native content | Any file type: SOD, ODF, DDS, TGA, BZN, SPR and the rest beside GLB/glTF, KTX2, JSON5, Lua, OGG. |
| Same content, same identity | The archive's identity depends on its paths and file contents only, never on the compressor or packing time, so multiplayer peers can compare it. |

## A mod's folders

Paths inside an archive are exactly the paths of a mod folder: the archive
root is the mod root. A legacy mod converts one-to-one, and its files land
where the engine already looks (`odf/`, `sod/`, `textures/`, `sprites/`,
`bzn/`, `techtree/`, `misc/`, `AI/`, `sounds/`, `missions/`). Native STA64
content uses these folders:

| Folder | Holds |
|---|---|
| `manifest.toml` | Identity, version, dependencies (required, at the root) |
| `assets/` | Models (GLB/glTF), textures (KTX2, PNG), animations, audio (OGG) |
| `data/` | Unit and weapon definitions (JSON5), tech trees, profiles |
| `maps/` | Campaign and multiplayer maps |
| `scripts/` | Lua mission and mod scripts |
| `ui/` | Menus, icons, HUD layouts |
| `localisation/` | Translations and text, one folder per language (`localisation/de/`) |

Both kinds may be mixed in one archive: a native mod can ship legacy files
and the other way round.

## File layout

All integers are little-endian. Offsets are from the start of the file.

```
+-------------------+  0
| header (64 bytes) |
+-------------------+  64
| file data         |  each file's bytes, stored or one Zstandard frame,
| ...               |  in index order, each starting at a multiple of 16
+-------------------+  index_offset
| index             |  possibly one Zstandard frame
+-------------------+  signature_offset (0 when unsigned)
| signature block   |
+-------------------+  end
```

The index sits after the data, so a writer can stream files out and write
the index last.

### Header

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | magic `STA64ARC` |
| 8 | 2 | major version (1) |
| 10 | 2 | minor version (0) |
| 12 | 4 | flags: bit 0 index compressed; others 0 |
| 16 | 8 | `index_offset` |
| 24 | 8 | `index_stored_size` |
| 32 | 8 | `index_size` (uncompressed) |
| 40 | 8 | `signature_offset` (0 = unsigned) |
| 48 | 16 | reserved, 0 |

A reader refuses a different major version and accepts any minor version of
its major. Within a major, minor versions only add things readers may ignore.

### Index

The (uncompressed) index is:

| Size | Field |
|---:|---|
| 4 | `entry_count` |
| 4 | `entry_size`: bytes per entry record (72 in 1.0; a later minor may grow it, and readers skip unknown trailing bytes) |
| 32 | `content_hash`: the archive identity (below) |
| `entry_count` × `entry_size` | entry records, sorted by `key` (below) |
| … | string pool: every path, UTF-8, no terminators |

Entry record (1.0):

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | `path_offset` into the string pool |
| 4 | 2 | `path_len` (bytes, at most 1024) |
| 6 | 1 | `storage`: 0 stored, 1 Zstandard |
| 7 | 1 | `kind`: 0 file, 1 whiteout (hides this path in mods below; no data) |
| 8 | 8 | `offset` of the data |
| 16 | 8 | `stored_size` |
| 24 | 8 | `size` (uncompressed) |
| 32 | 32 | SHA-256 of the uncompressed bytes (all zero for a whiteout) |
| 64 | 8 | reserved, 0 |

**Paths.** As written, with `/` between folders, relative, no empty, `.` or
`..` components, no `\`, `:` or control characters. The **key** of a path is
its ASCII lower case; keys are unique within an archive, and records are
sorted by key bytewise, so a reader finds a file by binary search. Lookup by
key makes `.sta` case-insensitive everywhere, as Armada-era content requires.
Non-ASCII letters are kept and compared as bytes.

**Compression.** Each file is stored or one Zstandard frame (RFC 8878) of its
whole content. A writer stores a file when compression does not make it
smaller. Files that are themselves compressed (OGG, PNG, KTX2 with
supercompression) are normally stored.

### Archive identity (`content_hash`)

SHA-256 over, for every entry in key order: the key, a 0 byte, `kind`, `size`
as 8 bytes, and the entry's SHA-256. It covers what the archive contains, not
how it was compressed, so two archives with the same files have the same
identity whatever tool or Zstandard version packed them. STA64 is to use it
as the mod's identity in content checks (`ContentDb::hash`) and the
multiplayer handshake (WORKPLAN CON-18, NET-01).

### Signature block (optional)

| Size | Field |
|---:|---|
| 4 | block size |
| 1 | algorithm: 1 = Ed25519 |
| 32 | public key |
| 64 | signature over the 64-byte header (with `signature_offset` as written) followed by `content_hash` |
| 2 | signer name length, then the name (UTF-8) |

A signature says that the holder of that key published exactly this content.
STA64 shows an archive as signed by a key the player trusts, signed by an
unknown key, unsigned, or tampered with (a hash or signature that fails).
Unsigned archives are normal and load; what to do with the others is a
setting, not part of the format.

## Manifest (`manifest.toml`)

```toml
[mod]
id = "federation-expansion"       # unique, lower case, a-z 0-9 - _
name = "Federation Expansion"
version = "1.2.0"                 # semantic versioning
description = "Twelve new Federation ships and three campaigns."
authors = ["Immy"]
licence = "CC-BY-4.0"             # optional, SPDX
engine = ">=0.4"                  # STA64 versions it works with
profile = "fo4"                   # content profile, as WORKPLAN CON-05
kind = "addon"                    # "addon" (needs others) or "standalone"

[dependencies]                    # id = version requirement
"fleet-ops-4" = "^4.0"

[load]
after = ["better-textures"]       # optional ordering hints
before = []
```

- **Dependencies** name other mods by `id` with a semantic-versioning
  requirement. A missing or wrong version stops loading with a message naming
  it.
- **Legacy mods.** An `info.ini` `ParentMod` is a dependency on that mod. A
  converted mod keeps its `info.ini` beside the manifest, and the converter
  writes the matching `[dependencies]`.
- Unknown keys are kept and ignored, so a later STA64 can add fields.

## Mounting and load order

Every mod, archive or folder, is one layer of the [VFS](../crates/sta_vfs/src/lib.rs),
as FPQ and ZFS archives are today:

1. The active mod's dependency graph is resolved (versions checked, cycles
   refused) and ordered so every mod sits above everything it depends on.
   `[load] after/before` break ties, then `id`.
2. A file is served by the highest layer that has it. A whiteout hides the
   path in every layer below its own.
3. Inside one mod, loose files in its folder win over the same path in its
   `.sta`, so a modder can test a change without repacking.
4. The ODF index ([`OdfIndex`](../crates/sta_content/src/index.rs)) treats a
   `.sta` layer like any other: ODFs anywhere under `odf/`, mod root first.

## Writing

`sta-archive pack <dir> <out.sta>` (WORKPLAN EDT-08) builds an archive from a
mod folder; `sta-archive convert` turns an FPQ/ZFS-based mod into one.
Writers should be **reproducible**: entries in key order, data in index order,
one fixed Zstandard level per tool, no timestamps. Then the same folder packs to the same bytes with the same tool,
and to the same `content_hash` with any tool.

## Reading safely

A reader must refuse, rather than trust: offsets or sizes past the end, data
ranges that overlap the header, the index or each other, a Zstandard frame
that does not end exactly at `stored_size` or decodes to other than `size`
bytes,
paths breaking the rules above, duplicate keys, and unsorted records. A
SHA-256 mismatch is reported for that file; the rest of the archive stays
usable. `sta-archive verify` checks every file and the signature.

## Limits

Files up to 2^64 − 1 bytes, up to 2^32 − 1 entries, paths up to 1024 bytes.
Large media should be stored rather than compressed, so they can be read in
pieces without decompressing from the start; a later minor version may add
seekable frames.

## Licence

The specification is meant to be implemented freely, including by closed
tools. Proposed: publish this document under CC0 1.0 (no rights reserved),
while STA64's code stays MIT OR Apache-2.0. (To be confirmed by the user
before 1.0.)

## Changes

- 0.1 (2026-11-08): first draft.
