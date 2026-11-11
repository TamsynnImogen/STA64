# A1 binary map-body import (CON-26)

This contract precedes implementation. CON-25 (#294) recognizes front matter;
this package adds the known binary **map** body, using the same shared `Map`
as the implemented A2 parser. Text A1 bodies and saved-game state remain explicitly unsupported.
No engine gameplay, content hashes, native data files or dependencies change.

## Evidence and limits

The original installed A1 executable's SHA-256 is
`1c37ee8e4a24380c950a91ce5a7f9f96c4fcf3d8ffc0cc7e58faec3835350be6`.
Static paired readers/writers establish the type byte, metadata order, object
placement fields, mission fields, AOIs and named AiPaths. Private instruction
captures stay outside the repository; no native executable was launched.

Read-only record walks of 49 original installed A1 maps found 17,595 placement
prefixes. Every map's preceding declared count matched, every transform was
the seventh prefix record, and the metadata prefix had one common shape. All
49 use missionSave=true, no resize and zero AOIs. A2 controls are 72 maps with
34,695 objects. Nonzero AOIs and resizing therefore need synthetic controls in
addition to the native static evidence; they are not corpus/runtime proof.

An inspected native binary field writer initializes only the type byte of its
four-byte tag slot; inspected readers compare that byte and the separate payload length.
Ignore the upper 24 bits for A1 only. They are not a stable field-name hash.
Use CON-25's exact metadata offset, not the reference's opaque 145-byte skip.

## Shared output and retained source data

Keep existing A2 binary/text parsing unchanged; SOL is planned, not implemented
in this checkout, and remains outside this package. Add one default-None field,
`Map.a1: Option<a1::MapData>`, rather than another simulation/content model.
The generic fields retain metadata, placements and scenery. The optional data
retains A1 information not yet consumed by the engine:

- `header: Header`, `terrain_name: String`, `source_bounds: ([f32; 3], [f32; 3])`,
  `resized: bool`, `declared_object_count: usize`, `mission_title: [u8; 64]`.
- `object_refs: Vec<u32>` in placement order. These are serialized reference
  tokens, never host addresses to dereference.
- `initial_positions: Vec<[f32; 3]>` in placement order, after any resize. The
  native factory consumes the separate prefix position before loading and
  applying the final matrix. SIM-22 research establishes that this ordering
  can affect confinement initialization. All 17,595 original placements have
  identical prefix/matrix positions, but authored maps must retain both rather
  than lose this distinction. This addition is recorded before implementing
  the retained-position field; it does not enable confinement here.
- `paths: Vec<AiPath>`: `reference: u32`, original `label: String`, ordered
  `points: Vec<[f32; 3]>`, raw `path_type: i32`, `asteroid_belt_speed: f32`.
- `areas: Vec<AreaOfInterest>`: `object_ref: u32`, `team: i32`,
  `interesting: bool`, `inside: bool`, `value: i32`, `force: i32`.

This preserves data for later mission/AI integration. It does not implement
path following, AOI rules, saved object health/orders or full campaign play.
Do not synthesize units or scenery from unsupported object state.

## Record grammar

Walk complete records through EOF with the existing framing checks. A1 field
matching uses the low type byte and exact sizes; all counts must be nonnegative
and bounded by the actual remaining data before allocation. No arbitrary fleet
or mobile-specific limit is introduced.

After the textual header:

1. Text(type2,size16) msn_filename; bool(1,1) missionSave. Reject saved-game
   state when the latter is zero before attempting its different metadata.
2. Text(2,100) TerrainName; six floats(5,4), minX/maxX/minY/maxY/minZ/maxZ.
3. Bool(1,1) resize. If nonzero, six further floats are the new bounds. Retain
   the first bounds as source bounds. Validate finite, positive extents. Do not
   apply A2FO's A2-host compatibility expansion of the A1 vertical interval.
4. Ambient RGB: three floats; BackgroundArt text100; techTreeFilename text128;
   race text128; declared object count int(4,4).

Object placements have a strict seven-record prefix: PrjID(7,8),
EntityID(4,4), pos(9,12), team(4,4), label(2,40), obj_addr(8,4),
transform(11,48). The native writer names these fields. Scan **framed records**
only, starting at the declared object list; retain original order and require
exact agreement with the count. Preserve the serialized reference token.
Native class payload between prefixes is not imported as simulation state.
The exact PrjID width comes from the original GameObject reader's eight-byte
argument and its exact-size check. The reference/A2FO signature allowance of
variable-width names is too permissive here; all 17,595 originals use eight bytes.

Require exactly one supported mission block after the object list:
class-name text40 (EmptyMission or Inst4XMission), reference(8,4), scriptName
text64, ruleName text64 and missionTitle raw(0,64). Preserve nonempty script
filenames in their original order, title bytes intact, and expose its text in
the common description. No legacy DLL is executed. Unknown/ambiguous mission
blocks fail explicitly instead of yielding a successful incomplete map.

Then an AOI count and six records per AOI: object reference(8,4), team(4,4),
interesting(1,1), inside(1,1), value(4,4), force(4,4). These are retained values,
not native references resolved or AI rules enabled.

Finally an AiPath count and, per path: old_ptr raw(0,4), label length int,
optional text of exactly that length (absent for zero), point count int,
vector(9,12*count), pathType raw(0,4), asteroid_belt_speed float(5,4). Retain
all points, empty paths and original labels without deduplication. Require the
tail to end exactly; trailing or inconsistent fields produce an error.

Decode strings only up to their first NUL, preserving bytes after it in the
raw mission title. Non-UTF-8 text before NUL is an explicit unsupported-encoding
error, not a lossy replacement. This is a documented legacy locale limitation.
Native binary bool values use nonzero semantics. Require finite geometry,
ambient and speed values; no normalization of valid rotations, labels or zeros.

## Resize and start locations

The native loader changes only position X/Z (including matrix translation and
every path point) from old to new bounds. Y and orientation stay unchanged.
For each horizontal axis use `newMin + (value-oldMin)/(oldMax-oldMin)*newSize`,
with f64 intermediate arithmetic and a final f32 store for a shared deterministic
reader. This is a stated numerical policy, not an unmeasured bit-exact x87 claim.
Unresized floats remain unchanged. Keep original and final bounds distinct.

No start locations are invented in this package. Multiplayer starts need a
subsequent shared VFS/companion MDF integration. Native MDF writing derives
preview pixels from Camera_i paths using a 118-unit grid. A2FO's inverse helper
uses 117; neither quantized reconstruction should silently replace the precise
BZN path points. Preserve all paths now; settle the start-selection/default
policy against the original native consumers before wiring it into gameplay.

## Validation and publication gates

Synthetic tests cover actual metadata/placement/mission/path/AOI grammar,
nonzero upper tag bytes, garbage after NUL, explicit zeros, signed IDs, zero
objects/paths, multiple groups, wrong counts/types/sizes, truncated frames,
ambiguous/unsupported mission blocks, saved-game/text refusal, finite values,
and resize X/Z with unchanged Y/orientation. Counts come from serialized fields,
not fixtures manufactured from the parser's own result.

Read all original 49 maps, require nonempty objects and count agreement, retain
paths/mission data, and hash inputs before/after. Re-run the 72 A2 controls with
unchanged counts and declared-count checks. Run scoped tests/strict Clippy,
content and mission consumers, workspace all-targets checking and Android ARM64
library checking. A library check is not an APK or device/runtime result.

Lead owns native semantics, body implementation and narrow module/error/Map
changes. A bounded worker may own only `tests/a1_body.rs`; independent review
follows the frozen implementation. No source game bytes enter test fixtures.
This package adds no release dependency or completion date; the December 2026
desktop alpha remains the immediate priority.

## Implementation and executed checks

The shared parser now produces nonempty A1 `Map` values and optional source
data under the contract above. Twenty BZN tests passed with the original
corpus enabled: 49 A1 maps with 17,595 placements and exact declared-count
agreement; 72 A2 maps with 34,695 objects and zero count mismatches. Synthetic
controls cover nonzero AOIs and resize; the original corpus has neither.
An additional aligned seven-byte PrjID case checks the original native
eight-byte width rather than relying on an accidental framing failure.

The existing content and mission library suites reported 90 and 122 passes.
The real A1 content-conversion test converted all 49 maps and accounted for
every object: 2,176 placed actors, 15,028 regions, 106 lights, 126 uncarried
props and 159 skipped objects. It reported 59 existing background-prop/wormhole
warnings, no unrecognized ODF warnings, and zero starts (not implemented here).
The A2 conversion control converted all 72 maps, including 35 multiplayer maps
with starts and no conversion problems. These tests do not prove rendering,
native game rules, AOI/path gameplay or campaign completion.

Strict sta_bzn Clippy, workspace all-targets checking and the Android ARM64
sta_bzn library check passed. The offline workspace lock refresh changed zero
packages. No APK or device test was performed for this change. Independent
source review, including the final native-width correction, found no confirmed
defect. Original-map hashes and static/native captures are kept privately;
original map bytes are not test fixtures in this repository.

At CON-26, the desktop `--install-root` path still required a child `Data/`;
its headless conversion test mounted the bare A1 folder directly.
[CON-27](INSTALL_ROOTS.md) extends the same VFS entry point to flat A1 roots
and checks an actual built client mission report. MDF/Camera_i start selection
remains a separate follow-up. Original data is not moved or rewritten.
