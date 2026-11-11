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
| ODF diagnostics | Preserved entries and duplicate/schema reports in `src/odf/` | Implemented the optional same-file duplicate report below, using RawOdf's original spelling, value and line. |
| Includes and mods | Filesystem resolver and `mod_info.rs` | STA64 has richer layered self-includes, OdfIndex, VFS mounts and mod inheritance. Keep those shared implementations. |
| Generated schema/defaults | Command tables and A1/A2 comparison tools | Discovery leads, not authoritative runtime defaults. Independently verify profile behaviour against the games. |
| A1 BZN header | `BznHeader` recognizes A1 binarySave separately from A2 BinaryMode | The audit exposed a missing branch. CON-25 added strict recognition; CON-26 adds known binary map bodies ([scope](A1_BZN_SUPPORT.md)). |
| A1 objects/conversion | Candidate-record scans and a conversion plan | Heuristic scaffolding. The template converter copies its supplied A2 template; it does not emit converted A1 objects. |
| A1 body boundary | Skips 145 opaque metadata bytes | Original GOG reader/writer checks established typed metadata and placements for CON-26. The fixed skip points into a record in an original map; it is not used. CON-28's checked MDF/precise-path resolver feeds the selected-map conversion seam; graphical/gameplay acceptance remains pending. |
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

## Implemented report and validation

Run from the workspace root:

```sh
cargo run -p sta_odf --bin sta-odf-duplicates -- path/to/unit.odf
cargo run -p sta_odf --bin sta-odf-duplicates -- --deny-duplicates path/to/unit.odf
printf 'Speed = 1\nspeed = 0\n' | cargo run -p sta_odf --bin sta-odf-duplicates -- -
```

The last example reports `-:2: speed overrides 1:Speed`. The ordinary report
exits 0; the strict form exits 1 when duplicates exist. This is a command-line
tool and library API; it is not yet integrated into the graphical editor.

On this implementation branch, the 11 ODF unit tests and four CLI subprocess
tests passed. The separate install-gated include test returned without running
because its environment was unset; it is not installed-mod resolution proof.
The content consumer suite reported 90 passes. Strict sta_odf Clippy,
`cargo check --workspace --all-targets`, the Android ARM64 sta_odf library
check, and an offline workspace lock refresh passed; Cargo.lock stayed unchanged.
The Android check does not link an executable or build an APK.

A read-only CLI sample covered five installed A1 addon ODFs and 100 installed
A2 loose ODFs. Both advisory and strict reports exited 0 with no findings in
that sample. Every input SHA-256 remained unchanged. Positive duplicate and
error-exit behaviour is exercised by the synthetic subprocess tests. Archives,
non-UTF-8 conversion and complete installed-mod validation remain outside this
tool's scope.

## A1 map investigation and resulting imports

A separate read-only probe compared the reference header reader and STA64's
audit-baseline parser on the original installed maps. The reference recognized all
49 A1 headers; STA64 accepted all 49 through its text fallback and returned
zero objects and starts. The A2 control set parsed 72 binary maps, 34,695
objects and 648 starts. Successful `parse()` alone therefore does not prove
A1 map support, and the older WORKPLAN statement that all installed maps are
read must not be interpreted as validated A1 import.

At the reference's candidate A1 body offset, STA64's typed-record framing
consumed all 49 streams without an error. A strict name/id/position/team/label
prefix scan found 17,595 candidates, each with a nearby transform. This is
structural evidence only: it does not establish complete object semantics,
map bounds or companion MDF starts. A six-scalar bounds scan at that offset
found none; metadata cannot simply be treated as the A2 body header.

The existing A2FOExtensions `a1_maps` reader provided an additional independent
lead for metadata, bounds and MDF framing. CON-25/#294 and the subsequent
[CON-26 import](A1_BZN_IMPORT.md) validate header/body fields against original
GOG readers/writers and installed maps, alongside malformed-input controls.
They do not establish full A1 gameplay or MDF/start support. Do not
import the opaque 145-byte skip as an unexplained format constant or use the
reference's template-copy converter as evidence of object conversion.

The subsequent [CON-28 start resolver](A1_START_LOCATIONS.md) checks canonical
MDF counts and exact first `Camera_i` points against original A1 new-game
placement. All 28 installed companion pairs yield 113 precise starts. The
[selected-map seam](SELECTED_MAP_CONVERSION.md) connects that resolver to shared
content conversion and both production client paths. Preview pixels are not inverted into world
positions, and the original files are not rewritten.

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
