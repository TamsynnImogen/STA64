# Per-instance hull customization (RND-33)

This package implements A2FO authoring keys as native presentation (D1, D2,
D9, D10). Source/documentation is the evidence grade; native visual parity is
not measured. Projection onto hull triangles is a deliberate STA64 design
requested by Tammy, rather than the extension's floating image planes.

## Shared data

`WorldView::appearance` carries owning-seat faction identifiers and actual
subsystem hit points, maxima and switches, indexed separately from ordinary
unit/player rows. No renderer reads mutable simulation internals. The five
systems are engines, weapons, shields, life support, sensors. Unknown state
stays unknown; hull health never substitutes for a subsystem's health.

Seat faction metadata comes from the resolved `GameSetup`, follows real
ownership on capture, survives snapshots, and remains cosmetic: no gameplay
rule or digest changes. A missing faction uses base materials and hides faction-specific subtrees; ordinary nodes are retained.
RND-35 reads SIM-54's existing `WorldView::attrs` race overrides: an explicit
unit race wins, then the current owning seat's race override, then that seat's
setup faction. Capture preserves an explicit unit override, as the simulation
already does; without one the ship follows its new owner's override or setup.
Identifiers use the existing case-insensitive race-name/stem catalogue. An
explicit override absent from that catalogue uses base materials and hides
faction subtrees; it does not fall through to a different faction. No class
race is inferred. These render reads add no simulation state, gameplay hash
contribution, save record, or view/wire revision. Exact live visual parity of
mission-driven changes remains unmeasured.
Snapshots append a versioned metadata record; view/log changes bump the seam
revision and reject prior encodings before decoding. Existing subsystem state
is already saved and hashed; projecting it adds no damage or repair law.
In this engine, ordinary weapon hits do not yet generate subsystem damage and
yards do not yet repair these pools: those existing simulation packages remain
prerequisites for automatic combat-to-rebuild behavior. Mission/bench subsystem
damage and repair edits exercise the live path today.

## Faction appearance

Race ODF `factionTextureSuffix` accepts at most 32 ASCII letters, digits,
underscores or hyphens. Empty/missing disables it. Per mesh, a suffixed DDS
wins over TGA; a miss retains the ordinary base/Borg material. Existing
normal/bump data is retained. Specular/emissive variants may use the same
role-first, faction-last naming rule, with independent base-map fallback.

SOD nodes matching a loaded race's internal `name` are shown for that owner
only (case insensitive); unmatched names remain ordinary nodes. The reserved
`borg` node retains the existing Borg policy. Every policy is per instance:
two owners of the same cached model cannot change each other's materials,
visibility or transforms. Capture is handled after attachment as well as at
spawn. Stock and Refit lighting remain their existing owners' responsibility.

## Subsystem parts

Sparse indices 1..64 of `sensorMeshN`, `engineMeshN`, `weaponMeshN`,
`lifeSupportMeshN`, `shieldGeneratorMeshN` name model nodes; a paired
`...MeshNexplosion` is optional. On destruction choose one valid entry from
entity ID, system and observed destruction count. Hide its entire subtree,
including sprites and emitters, without affecting siblings or shared models.
An initially destroyed state chooses a part without a false explosion.

As actual HP recovers, the chosen root grows with eased HP progress while
descendants remain hidden until complete. Other visibility constraints still
apply. Repair sparks use the real selected geometry; configured explosion
ODFs are presentation-only and never create gameplay damage. Counters are
transient presentation state, as in the extension; no exact visual-cycle
continuation through an arbitrary mid-repair save is promised.

## Surface projections

Existing hardpoint, Offset, Rotation and Size controls define a projector in
model space, with `ScaleSOD` applied once. Projector-local XY covers the image;
local Z is the finite projection depth. Projection clips actual hull triangles
against all six box planes, rejects back-facing surfaces, and derives UVs from
projector coordinates rather than the hull's UV map. No wall, underside or
nearby ship outside that volume receives the decal. A narrow normal offset
prevents z-fighting. Projected geometry stays attached to its receiving model
node and refreshes when relative projector/receiver animation changes.

`<placement>Depth` is an optional STA64 presentation key (positive finite model
units); absent defaults to one quarter of the smaller authored width/height.
Old engines ignore it (D3). Invalid/degenerate bounds produce no projection
and a diagnostic. Never draw a free-standing quad when no hull is hit.

Damage uses `damageThreshold` (default 0.1), `damageDecalPreview`, and sparse
`hullScorchN`, `sensorsScorchN`, `enginesScorchN`, `weaponsScorchN`,
`lifeSupportScorchN`, `shieldGeneratorScorchN` with the existing placement
suffixes. Entry N appears at N damage intervals. Legacy `scorchTextureN` and
system target-hardpoint lists are supported only where an unambiguous mapping
can be made; explicit entries take precedence.

Permanent `logoDecalN` projections select `logoFileNames` by the original
source row of the existing deterministic craft-name choice. Blank or short
lists stay blank; duplicate craft names do not collapse rows. Existing Suffix,
ColourKey and FlipU controls retain their image meaning. Direct registry text
from `possibleCraftRegistry` uses the same row and projection path, explicitly
selected by `logoDecalNText = "registry"`. `logoDecalNFont` names an installed
sprite font (default `FontSmall`); optional `logoDecalNColor` is an RGB or RGBA
byte tuple. Lettering reads from the receiving side by default, while authored
images keep their legacy U convention; `FlipU` reverses either. This does not
replace other placements using authored image rows.

All projected parts obey the receiving subtree, construction, cloak, fog and
match lifetime. Per-instance meshes/material handles are released on refresh,
despawn, mod switch and match exit. Geometry/image work is cached and refreshed
only when its inputs change; there is no full-model rebuild every frame.

## Acceptance

Synthetic tests cover simultaneous owners, capture/fallback, ancestor gating,
subsystem destruction/repair/load, image/name alignment, clipping and UVs on a
curved hull, facing/depth rejection, transforms/animation, hidden parts and
cleanup. Real software offscreen captures must demonstrate projection on
multiple hull triangles and independent simultaneous appearances. Changed
crate tests, strict Clippy and workspace all-target checks are required.
Native/manual visual acceptance remains separate.
