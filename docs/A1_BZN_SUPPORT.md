# Armada 1 BZN support: recognition before import

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

## Next import milestone

Before creating `Map` values from A1, validate typed metadata and per-record
type flags, object boundaries and transforms, labels/teams, mission data,
lighting/backgrounds and bounds against original GOG/native and A2FO evidence.
Companion MDF grid starts require an explicit conversion into STA64's neutral
slot and player-slot conventions. A nearby transform or copied A2 template
does not establish those meanings. Do not silently substitute defaults for
unknown legacy fields or report a structurally scanned map as fully playable.

This bounded correction adds no release dependency or completion date. The
December 2026 desktop alpha remains the immediate priority. Full A1 map import
and runtime acceptance are still pending.
