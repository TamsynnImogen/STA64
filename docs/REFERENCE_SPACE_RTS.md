# OpenSourced-Space-RTS reference audit

The user's earlier [STA Viewer repository](https://github.com/TamsynnImogen/OpenSourced-Space-RTS)
was inspected at `d01118d86b15f0f252b56278d9606312f38cb428`
(2026-07-25, initial clean-room codebase). Findings apply to that pinned source,
not a later local prototype or every Fleet Operations variant.

Its manifest uses Bevy 0.13.2 and edition 2021. STA64 retains Rust 1.97.1,
edition 2024 and Bevy 0.20.1. Asset/format inspection and diagnostic ideas can
be adapted without migrating its plugins or replacing working shared loaders.

## Implementation findings

| Area | Pinned reference | STA64 comparison and action |
|---|---|---|
| ODF diagnostics | Preserved entries and duplicate/schema reports in `src/odf/` | RawOdf already retains spelling, value and line. Add the optional same-file duplicate report below. |
| Includes and mods | Filesystem resolver and `mod_info.rs` | STA64 has richer layered self-includes, OdfIndex, VFS mounts and mod inheritance. Keep those shared implementations. |
| Generated schema/defaults | Command tables and A1/A2 comparison tools | Discovery leads, not authoritative runtime defaults. Independently verify profile behaviour against the games. |
| A1 BZN header | `BznHeader` recognizes A1 binarySave separately from A2 BinaryMode | STA64 has no A1 binarySave branch: a genuine gap for a separately validated format path. |
| A1 objects/conversion | Candidate-record scans and a conversion plan | Heuristic scaffolding. The template converter copies its supplied A2 template; it does not emit converted A1 objects. |
| A1 body boundary | Skips 145 opaque metadata bytes | Validate against original GOG A1/native/A2FO evidence before relying on this sample-driven boundary. Companion MDF starts and object records need separate validation. |
| FPQ | Reader/writer, flag-1 decoding and heuristic table discovery | STA64 already has exact v1/v2 parsing/writing and A1 ZFS support. Retain their validation and archive roundtrips. |
| SOD/M3D/materials/sprites | Viewer loaders and linked-asset diagnostics | STA64 already has shared models/materials, animated sprites/emitters and a viewer. Adapt targeted diagnostics for verified gaps. |
| OBJ/MTL export | Transformed geometry, UVs, normals and material groups | Useful future headless tool over `sta_assets::model::Model`; settle axes/winding and test a synthetic roundtrip first. |
| MS3D/MAX/navigation cube | Joint/keyframe reader, limited mesh import and viewer controls | Possible authoring conveniences; no complete MAX/animation claim or December alpha dependency. |
| SOD output | Flattened mesh nodes and minimal material data | Unsuitable as a preservation baseline for hierarchical/animated SOD content. |

The reference README separates project-authored source/tools from raw game/mod
assets, guide mirrors and recovered prototypes. The user explicitly requested
reuse of their project; this work adapts the authored diagnostic idea with
attribution. No root license file was found in the pinned tree. Raw assets,
guide prose, generated schema distributions and recovered prototypes are not
copied into STA64. The source reference checkout stays separate and unchanged.

## EDT-09: duplicate command report contract

This contract precedes implementation. Add an optional, read-only diagnostic
over RawOdf entries and a `sta-odf-duplicates` binary in the existing sta_odf
crate. Do not change parsing, numeric accessors, include resolution, content
hashes, simulation rules or defaults. No dependency/Bevy changes are needed.

`RawOdf::duplicates()` returns borrowed `DuplicateEntry` records containing
`previous` and `replacement` entries. Match keys using the same ASCII case
normalization as Odf lookups. Return replacements in source order; for three
assignments, report each against its immediately preceding assignment. Keep
both spellings, lines and values unmodified. An inherited key or mod-parent
override is not a same-file duplicate in the child.

The CLI accepts UTF-8 loose files and `-` for stdin, plus `--help`,
`--deny-duplicates` and `--` for option-like filenames. Report original key/line
locations; values need not be printed. Never rewrite files or resolve includes.
Directory recursion, mounted archives, schema validation and non-UTF-8
conversion are outside this first tool. Unsupported input produces an explicit
read/usage error, not a lossy rewrite or a claim of full ODF validation.

Repeated commands are legal: later values win. Ordinary reports exit 0 even
with duplicates. `--deny-duplicates` opts into exit 1 for duplicate findings.
Usage/read errors exit 2 and take precedence; help exits 0. A repeated key alone
does not mean the mod is invalid.

Checks cover original case/lines, source-ordered chains, explicit zero, quoted
comment markers, legal include overrides, advisory/strict stdin/file behaviour,
error precedence and no file mutation. Run scoped tests and strict Clippy,
workspace all-targets checking and an Android library compile where practical.
A library compile is not a full APK or device-runtime result.

## Source pointers

- [Diagnostics](https://github.com/TamsynnImogen/OpenSourced-Space-RTS/blob/d01118d86b15f0f252b56278d9606312f38cb428/src/odf/diagnostics.rs)
  and [manifest](https://github.com/TamsynnImogen/OpenSourced-Space-RTS/blob/d01118d86b15f0f252b56278d9606312f38cb428/Cargo.toml).
- [BZN parser](https://github.com/TamsynnImogen/OpenSourced-Space-RTS/blob/d01118d86b15f0f252b56278d9606312f38cb428/src/bzn.rs)
  and [FPQ parser](https://github.com/TamsynnImogen/OpenSourced-Space-RTS/blob/d01118d86b15f0f252b56278d9606312f38cb428/src/fpq.rs).
- [OBJ exporter](https://github.com/TamsynnImogen/OpenSourced-Space-RTS/blob/d01118d86b15f0f252b56278d9606312f38cb428/src/obj_plugin.rs).

SIM-22 geometry/design in #289/#290 and its pre-integration fixtures remain
preserved. State/movement/Lua integration is unfinished. This reference work
adds no release gate or completion date; the December 2026 desktop alpha
remains the immediate priority.
