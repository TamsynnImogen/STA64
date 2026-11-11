# Shared map-confinement core contract

Final pre-code integration contract under SIM-22/#287. It refines the
[geometry audit](MAP_CONFINEMENT.md) and
[integration proposal](MAP_CONFINEMENT_INTEGRATION.md) using the closed
factory-input, lifecycle and geometry gates. No Lua adapter belongs in this
core stage. Keep the December 2026 desktop alpha priority and one simulation
for desktop and Android.

## Closed gates and deliberately unchanged formats

Pinned A1, A2 and FO loaders read the prefix point and Matrix34 separately,
then pass the final matrix to the class factory. The new BZN prefix metadata
in #308 stays in the format layer. Do not add a content `MapObject` field for
this feature. Canonical A2/FO constructors and Init dispatch corroborate the
ordinary false constructor flag, terrain constructor exception and subsequent
raw-XZ initialization. A1's ordinary false constructor and terrain true flag
remain distinct. Private captures, image identities and primary vtable checks
are retained outside the repository; no native execution was used.

The map/replay body remains unchanged. Keep STAR version 1 and `SEAM_REV = 1`;
append `Edit::ForceOntoMap { id, enabled }` after Hull at tag 20 without changing
old tags or payloads. WORKPLAN permits appended variants within the milestone.
Use snapshot version 10 for the new private saved state. No STAR2 DTO,
provenance field, global seam bump, ViewLog migration or networking rewrite is
needed without a replay/view layout change. Revisit this decision if that
premise changes. The separate strict-reader #306 remains independently
reviewable; this branch uses main's existing API and does not edit replay.rs.

Genuine pre-change Native v8/v9 saves and a v1 replay are frozen with source,
library and file fingerprints. An independently frozen old snapshot decoder
accepts all three save controls at their recorded digests. Preserve these
originals, the old replay decoder and the old Native content hash. A real
tag-20 recording must be accepted by the new implementation and rejected by
the frozen old decoder. Header rewriting creates negative cases only.

## Rules and authoring

Use the canonical class tables and five STA64-owned ODF keys from the
integration proposal. Native capability defaults off. Known represented legacy
classes default on; unsupported legacy labels get an explicit diagnostic and
no invented spatial policy. The bounded A2FO source search does not establish
factory mappings for buildyard/fighter/moon/asteroid. Other source aliases
must not be treated as enabled native registrations without their own evidence.

Add a small override group to `UnitDef`. Parse it from the already resolved
owning unit ODF, preserving includes, parent-mod precedence, explicit/inherited
zero and missing values. Definitions built in code can supply the same typed
overrides. Do not claim an implemented native JSON5 unit loader. Resolve and
cache effective rules once in `ContentDb`, then prepare checked per-definition
volumes from the simulation bounds. No per-tick string classification, numeric
ODF parsing, new Features bits or duplicated gameplay implementation.

Reject malformed/nonfinite/negative/unrepresentable numeric overrides with a
file/key warning and the documented default. Do not silently clamp them through
an unrelated reader limit. Defaults remain 100 X/Z inset and native binary32
0.1 converted to Fixed raw 429496736; an authored zero remains zero. Terrain
spatial bypass stays structural, including bypass of ordinary automatic rearm.
Diagnose active settings under disabled/unsupported capability.

Hash enabled, represented effective rules with an explicit marker and stable
field order: spatial policy, initial mode, rearm, inset, upper epsilon.
Disabled capability contributes no new hash bytes. Preserve the genuine Native
fake database hash and existing Features layout exactly. Changes to automatic
legacy rules intentionally change content hashes; this is not general old
legacy-save migration.

## State, controls and actor lifecycle

Use the existing sorted `Table<bool>` sidecar. Keep false rows as well as true
for every live supported actor with enabled capability and a usable volume.
Do not borrow a UnitFlags bit or derive a saved flag from current position.
Provide actual `Sim` state inspection returning `Option<bool>`; unavailable is
distinct from false. No new WorldView field or fabricated Lua getter.

The privileged setter validates the live actor, definition, represented enabled
policy, usable volume and required row before writing only its boolean. Reject
invalid, disabled, unsupported or missing-state requests atomically, including
false and same-value requests. Preserve orders, velocity, cargo and ownership.
It must not teleport during command application.

All actual births share one initializer, including direct production births.
Keep map-load completion distinct from runtime creation:

| Profile/family | Initial rule | Ordinary callbacks after initialization |
|---|---|---|
| A1 ordinary | Inclusive padded X/Z entry; automatic rearm enabled | Factory callbacks, followed by map-load callbacks for placed objects |
| A1 terrain | Constructor-enabled flag | Spatial overrides bypass ordinary geometry/rearm |
| A2/FO known actors | Strict raw X/Z interior, independently of Y; no rearm | Runtime factory has no subsequent ordinary update after Init; map completion does |
| Native opt-in | Authored mode, disabled by default; no rearm by default | Defined shared Native birth update; no legacy timing claim |

Implement the A1 factory result with two ordinary updates and its map completion
updates. Implement A2/FO initialization followed by map completion updates only
for placed actors. These are explicit shared-engine translations of the audited
callbacks, not x87 or internal frame-count parity claims. An initial disabled
override does not suppress later A1 rearm; authors can separately disable rearm.

Core welds retain an ID while native construction destroys its placeholder and
constructs the finished actor. Reset boundary initialization at completion,
before assigning the finished actor's movement target. Incomplete structures
use plain-GameObject update timing; their final definition must not grant an
unconditional Craft tick prematurely. Keep current IDs, labels and production
accounting. Despawn removes the row; ownership changes preserve it.

For unusable/missing/tiny/overflowing volumes, retain valid spawns, diagnose
unsupported automatic confinement and expose unavailable state. Controls reject.
Do not persist a fabricated usable row. Check all enabled represented definitions
at setup, including those only spawned later. Bounds and numeric policy remain
immutable for a running simulation.

## Movement and geometry ordering

After applying the complete input batch, constrain active targets and queued,
formation-expanded waypoints before obstacle steering. Do not prematurely
rewrite a target while a later command in that same batch can change the flag.
Apply the actor's own current flag to resolved economy/deploy/launch/rally
destinations. Preserve stored builder rally specifications; a produced actor's
rules govern its movement, not the builder's flag. Repeat the target pass after
late economy/production/course assignments. Disabled/bypassed actors keep the
existing path unchanged.

Treat changed kinematic positions as real position updates, then perform the
finished Craft-family ordinary tick even if immobile or engine-disabled.
Apply further ordinary updates after actual separation/field position changes.
Plain GameObject/incomplete timing is update-driven; terrain bypasses the
geometry. In one A1 update, a previously false flag can arm from padded X/Z
entry independently of Y without retroactively projecting that same update;
a subsequent ordinary update can project. Preserve that ordering.

Keep the original detour preference when it is legal. If it violates the
volume, try the existing alternate before computing a fallback. Never clip a
detour into its blocker. For a solid-adjusted destination outside the volume,
consider checked circle contacts/intersections with comparison-limit planes or
expanded-box face contacts. Validate bounds, all applicable solid clearances
and a free segment from the current side; prefer distance to the requested
point, then to the origin, then stable candidate order. Preserve the existing
ownership exemptions. The comparison upper limit is distinct from the upper
projection inset.

Use integer square roots for boundary-circle contacts with outward rounding,
exact high/low products for segment comparisons and an overflow-aware distance
key. Checked arithmetic failure does not prove a free segment. The private
prototype passed 17 fixtures and independent checks of 34,986 segments and
10,081 products; a short-leg rounding rejection was corrected before selection.
These are geometry checks, not integrated simulation proof.

If no checked legal contact is available, use temporary hold/retry steering and
retain the logical order/queue. Do not orbit along an invalid detour or report
arrival at a fabricated destination. After physical projection, prefer a legal
contact or valid previous position when clipping would create solid overlap.
Fully occluded space cannot guarantee collision-free placement; confinement
remains authoritative. Do not relax bounds, remove solids or impose unit/AI
limits to hide that case. This is a bounded improvement to the current heuristic,
not a new general pathfinder.

## Persistence and hashing

Whenever any attached definition has enabled represented capability, hash the
bounds Option and all six Fixed coordinates from setup, even in an empty world.
Hash every sidecar ID/boolean in ascending order. Existing entity progress
already hashes the lifecycle distinction. Disabled-only Native worlds keep the
old digest path exactly.

STAS10 writes the unchanged old Body, an explicit bounds Option and a mandatory
boundary record: version 1 and sorted `(EntityId, bool)` rows. Fully consume
the record. Reject unknown versions, duplicate/out-of-order/nonexistent IDs,
disabled/unsupported rows, rows claiming unusable volumes and missing required
usable rows. True outside-position rows are legitimate: a flag-only setter,
runtime birth or plain-object timing can precede the next position update.
Never recompute the flag on load.

Keep precise v8/v9 parsing, optional old bounds and the v8 mining-phase guard.
Accept old state only when every effective represented capability is disabled,
even if an old header carries a substituted matching new content hash. New
enabled rules require explicit v10 state. Old decoders must reject new files;
synthetic header downgrades must not hide even an empty boundary record.
Replay continues to reconstruct from setup and validate content hash/digests;
it has no missing saved boundary flag to migrate.

## Scope and acceptance

Freeze only the exact source paths declared in #287: content rule/definition/
load/cache/hash seams; sim state/control/birth/production/target/position/
contact/snapshot hooks; dedicated tests and own-authored fixtures. Update the
SIM22 WORKPLAN scope and later append the manual check. Leave BZN, content map
conversion, replay.rs, ownership, race/loadout, combat/victory, rendering,
audio, client, networking and Lua source outside this implementation.

Require real shared Sim tests for initialization and class/lifecycle timing,
setter privilege/atomicity, A1 entry/rearm versus A2/FO persistence, all axes and
edges, active/queued/formation/economy/deploy/rally targets, boundary/solid
routes and holds, overrides/inherited zero/invalid diagnostics, hashes, strict
snapshot rejection and genuine Native v8/v9/v1 continuation. Exercise actual
new command bytes against the frozen old replay decoder. Run shared consumers,
strict scoped Clippy, workspace and full platform CI; check Android ARM64 and
build an APK separately. Keep the dependency lock and original input hashes
unchanged. No compatibility/runtime claim follows merely from compilation.

Publish the complete core seam for user merge before its Lua adapter. Preserve
the separate ownership and campaign limitations. Do not mark #287 complete for
this contract or the pure helper alone.
