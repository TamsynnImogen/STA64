# Armada 1 BZN support

## Current scope

CON-26 adds the known binary map body to the shared `Map` reader: metadata,
count-checked placements, mission descriptors and retained AOI/AiPath data.
See [the import contract and checks](A1_BZN_IMPORT.md). A1 text bodies,
saved-game state, multiplayer start integration and runtime acceptance remain
pending. Named paths and AOIs are retained data, not enabled gameplay.

CON-27 lets the client mount the original flat A1 install directly with
`--install-root`, using the existing shared VFS. Data-based installs keep
priority, and an explicit mod is never discarded to select a flat root.
The built client mission report enumerated all 49 original A1 maps; this
early-exit check does not start the window/audio/input pipeline. Follow the
[install-root rules and manual checks](INSTALL_ROOTS.md). Original files need
not move. MDF/precise-path starts and graphical/gameplay acceptance remain pending.

CON-28 adds pure helpers for a checked canonical MDF start count and precise
first `Camera_i` points ([contract and limits](A1_START_LOCATIONS.md)). The
original 28 companion pairs resolve 113 starts without preview-grid inversion.
These helpers are not yet called by content conversion or the client; A1
skirmish start integration remains pending. Native malformed-MDF/default-eight
handling and configuration-backed missing-path fallback are not implemented.

## Confirmed behaviour before CON-25

STA64's BZN parser recognizes A2 `BinaryMode`, but not A1 `binarySave`.
A read-only probe of the installed original GOG A1 maps found 49 successful
text-fallback results with zero objects and starts. These are not successful
map imports. A2 controls yielded 72 binary maps, 34,695 objects and 648 starts.

The user-requested [OpenSourced-Space-RTS reference](https://github.com/TamsynnImogen/OpenSourced-Space-RTS/blob/d01118d86b15f0f252b56278d9606312f38cb428/src/bzn.rs)
recognizes A1 front matter. Its fixed 145-byte post-header skip is not an
established body boundary: in the installed `2blue.bzn`, the textual header
ends at 318, the first three metadata records end at 459, and 318 + 145 points
four bytes into the next record. A record walk can appear successful even
from that position; framing alone does not prove field semantics.

The existing A2FOExtensions Rust `a1_maps` module instead parses the textual
header and examines typed records at its exact end. A separate read-only
experiment found its filename/bool/100-byte-text/six-float bounds sequence
in all 49 A1 maps, with positive extents in all three dimensions. This is a
useful lead for a subsequent importer, not full A1 gameplay validation.

## CON-25 contract, recorded before implementation

This package makes recognition truthful without attempting object conversion.
Add `sta_bzn::a1::inspect_header` and a value-only header with:

- Version, restricted to the existing A2FO reader's known 2050–2053 range.
- The original 128 bytes decoded from the 256 hexadecimal `saveGameDesc`
  characters; no UTF-8 assumption, trimming or replacement.
- The literal `binarySave` Boolean (`true` or `false`). Inspecting a false
  value does not claim support for the text body.
- The byte offset immediately after the Boolean's terminating newline.
  Do not skip or interpret metadata, object records or companion files.

Accept LF and CRLF line endings. Require the named fields, complete lines,
bounded decimal version and exactly 256 valid hexadecimal characters. Reject
unknown versions, missing/malformed fields, invalid Boolean text and truncated
headers with explicit errors. Do not infer an A1 format from a filename,
arbitrary body string or a 205x version number alone.

In `sta_bzn::parse`, recognize the A1 marker in the third header line before
the existing A2 search. Validate identified A1 headers, then return a specific
unsupported-import error carrying their version. Malformed identified A1
headers return a header error. Neither binary nor text A1 bodies are passed
to the A2 text parser. Keep the existing A2 text/binary parser bodies and
outputs unchanged, including a marker occurring inside an A2 body.

Own only the new A1 module, narrow module/error/dispatch lines in lib.rs,
dedicated tests and this documentation/WORKPLAN row. No dependencies, content
hashes, simulation, mod data, renderer or default changes. This correction can
replace an empty A1 map with an error; it does not remove working A1 import.
Callers already propagate BZN errors through their existing error paths.

Validate synthetic LF/CRLF headers, lossless non-UTF-8 description bytes,
newline-like initial body bytes, every truncated prefix, wrong fields,
overflow/unsupported versions and false/invalid Boolean values. Include A2
control cases and an explicitly ignored, opt-in original A1 corpus test.
Run scoped/consumer tests, strict Clippy, workspace checking and Android ARM64
library checking. The library check is not an executable, APK or runtime test.
Never commit original map bytes or modify the installed files.

## CON-25 inspection and validation (historical stage)

`sta_bzn::a1::inspect_header(&bytes)` returns the lossless front matter.
At CON-25, `sta_bzn::parse(&bytes)` returned `UnsupportedArmada1Import { version }`
for identified valid A1 headers, including text-body headers. CON-26 supersedes
that result for supported binary map bodies. Header errors still use
the separate `a1::HeaderError` type. The existing map browser displays parser
errors through its current error path; skirmish map discovery skips unsupported
maps. No GUI/runtime acceptance is claimed.

Thirteen BZN tests passed with the opt-in corpus test enabled. They include
all 49 original A1 headers receiving explicit unsupported-import errors, all
72 original A2 maps parsing 34,695 objects, and zero A2 declared-object-count
mismatches. All 121 original input hashes remained unchanged. The corpus test
can be repeated with `STA64_TEST_A1_BZN_DIR` set to the A1 `bzn` directory and
`STA64_TEST_BZN_DIRS` set to the A2 control directory:

```sh
cargo test -p sta_bzn -- --include-ignored --nocapture
```

The content consumer suite reported 90 passes. Strict sta_bzn Clippy,
workspace all-targets checking and Android ARM64 library checking passed.
The Android result does not prove an executable, APK or device run. An
independent source review found a similar-field-name detection collision;
the detector now matches the full field token and a regression checks A2
text/binary behaviour with `saveGameDescription` and `binarySaveMode` fields.
Follow-up source review found no confirmed defect.

## Remaining integration

CON-26 validates known typed metadata, the type byte, placement prefixes,
transforms, labels/teams, mission data and bounds against original GOG/native
and A2FO evidence. Native class payload between placements is not a saved-state
importer; binary per-object light colours also need further work.
Companion MDF grid starts require an explicit conversion into STA64's neutral
slot and player-slot conventions, preserving precise Camera_i path positions.
A nearby transform or copied A2 template does not establish those meanings.
Do not silently substitute defaults for
unknown legacy fields or report a structurally scanned map as fully playable.

This bounded correction adds no release dependency or completion date. The
December 2026 desktop alpha remains the immediate priority. Full A1 gameplay integration
and runtime acceptance are still pending.
