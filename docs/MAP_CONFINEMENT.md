# Map confinement and off-map mission control (SIM-22)

## Status and trigger

This is a staged proposal and native-audit record for issue #287. The first
implementation stage is the pure geometry operation specified below. Entity
state, new edits, defaults, movement integration and the Lua property remain
gated on their separate policy decisions. Deliver the integrated simulation
seam before its LUA-05 adapter; pure geometry alone does not fix Blockade.

The next proposed state/authoring/movement/persistence contract is in
[MAP_CONFINEMENT_INTEGRATION.md](MAP_CONFINEMENT_INTEGRATION.md). Its remaining
gates precede any entity integration or source freeze.

The longer installed campaign probe fails on Blockade's unsupported
Entity.forceOntoMap property. The external port has five true assignments and
one false assignment, alongside real orders. A fake property that only stores
a Lua boolean would hide the error without implementing its effects. The
30-tick startup sweep passing does not resolve this later failure.

Keep one shared simulation for desktop and Android. Preserve the December 2026
desktop alpha priority; this proposal adds no release gate or completion date.
No reference executable, extension project or original game data is modified.

## Implemented source baseline

MapDef holds bounds; Sim stores them as setup metadata and uses them for build
sites. Movement and kinematics do not currently enforce those bounds. SIM-21
has now merged unit/solid avoidance and separation; integrate with its actual
current order rather than replacing it. Current Lua Entity has no supported
forceOntoMap setter. Core UnitFlags is a serialized u8 and all eight bits are
assigned. There is no spare bit to reuse.

Snapshot version 9 carries the existing state body and an optional bounds
suffix. The reader also accepts a narrowly validated version-8 body. Bounds
are currently excluded from the simulation digest. These facts constrain the
new state and its compatibility checks; they are not proof that a boundary
implementation already exists.

## Private static reference findings

Pinned original GOG A1 is the primary A1 reference. Stock A2's named map and
the owned FO host corroborate the A2/FO paths. The owned FleetOps recovery's
serialized-field string catalogue and A2FO's A1 map-bounds conversion are
secondary evidence. Private hashes, type identity and bounded excerpts stay
outside the repository. No addresses, layouts, assembly or original assets
belong in this document. Static paths are distinct from live hook admission,
native execution and in-game acceptance.

The A2 setter looks up a live native GameObject and writes a boolean. It does
not itself move the object. Force-vector, position-update and origin-setting
paths consume that state; Craft process movement/formation targets also read
it. A setter-only shim is therefore insufficient.

For finite ordinary coordinates, the inspected force-vector helpers have:

- A native background-object exemption. Do not infer that exemption from Core
  Resource/Prop actor categories or exemption from all map rules.
- X/Z lower limits inset by 100 world units; a strict lower violation can
  project to that inset. Upper violations use the upper bound minus 100 and
  then a binary32 0.1 inset. Equality and the order of comparisons matter.
- Y checks against the original vertical limits, without the X/Z padding.
- A mode that reports an outside condition without changing the supplied
  vector. The X/Z outside indication differs from the overall changed/outside
  return: vertical violations do not set that same X/Z indication.

The stored binary32 0.1 is approximately 0.10000000149011612. A deterministic
Fixed translation must document its conversion and is not x87 bit parity.
The helpers do not establish arbitrary invalid-coordinate or tiny/inverted
map behavior. Audit their complete finite-domain branches before coding the
pure operation and overflow policy.

| Profile evidence | Established observation | Remaining initialization boundary |
|---|---|---|
| Original A1 | A type-confirmed GameObject position-update path can re-enable a false flag when the X/Z outside indication is false. Background geometry is exempt. Its field layout differs from A2. | Trace fresh-map load, initialization and serialized overrides before selecting spawn defaults. Do not borrow A2 offsets or initialization rules. |
| Stock A2 | Init derives the flag from strict interior of the raw X/Z bounds. The inspected UpdatePosition path has no A1 auto-rearm branch. | Determine when Init runs relative to native serialized map/save state and STA64 spawn/teleport flows. |
| Owned FO host | The inspected Init and UpdatePosition patterns match the A2 policy; force-vector geometry uses corresponding bounds and literals. | Keep hook/extension admission and native effects unclaimed; verify applicable class/default paths. |
| Native STA64 | Existing worlds are unconfined. | Choose and document an author-controlled default that preserves existing Native fixtures; do not assume an unverified legacy profile default. |

The native field is serialized. STA64's current BZN MapObject and content
MapObject do not expose it. Native serialization alone does not tell us whether
fresh map loading preserves or reinitializes an authored flag. Resolve this
before silently replacing serialized state with a computed default.

Further A2 tracing narrows that gate: GameObject.Load reads serialized state
on a reader-mode branch, then its zero-mode tail recomputes the force flag
from strict raw X/Z interior; the other branch skips that recomputation.
The ordinary FileReader constructor initializes that mode to zero, while a
named ReplayReader constructor explicitly sets it to one on its FileReader.
The named A2 header loader derives the reader mode from its SaveDetails type,
and its caller forwards that same reader to the main load operation. This is
a typed object test, not evidence of a raw eight-byte BZN header tag. FO's
inspected object Load has the same zero-mode recompute/nonzero-mode preserve
branches. A1 instead uses a separate global gate around the serialized force
read, and its inspected Load tail lacks the A2 raw-X/Z recomputation. Paired
header read/write paths identify that A1 gate as `missionSave`; all 49 original
installed A1 maps have the corresponding metadata boolean true. The existing
A2FO A1-map parser corroborates the front-matter order. `saveGameDesc` alone
does not identify a file as a saved game: it also occurs in those maps. FO's
header path likewise derives its reader mode from the loaded SaveDetails type.
Exact constructor/position/Load/Update ordering across applicable classes and
FO's complete main-load forwarding still need closure. Do not infer that every
serialized flag must become a fresh-map authoring field from Save alone.

## Stage 1: pure geometry contract

The complete finite-coordinate comparisons are sufficient for an independent
geometry prerequisite. Implement only `sta_sim::boundary` and its dedicated
tests, plus the module declaration in `lib.rs`. It must not inspect or modify
Sim, entities, content, commands, movement, snapshots, hashes or profile
defaults. Initialization, background/class exemption and persistence remain
caller responsibilities for later stages. No currently running game changes
behaviour merely because this module is available.

`BoundaryVolume::new(min, max, inset, upper_epsilon)` accepts Fixed values and
returns a validated volume or an explicit error. It has no hardcoded tuning
defaults: later content integration must supply resolved author-controlled
rules. For X and Z, retain both the upper comparison limit and the distinct
upper projection value:

- Lower comparison/projection limit: `min + inset`.
- Upper comparison limit: `max - inset`.
- Upper projection value: `max - inset - upper_epsilon`.
- For Y, comparison/projection limits are the unmodified min/max.

Each lower violation is strictly below the lower limit; each upper violation
is strictly above the upper comparison limit. Equality is inside, including
equality at the upper comparison limit. A point above that upper limit projects
to the upper projection value, not to the comparison limit. Check X, Y and Z;
return both any-axis outside and X/Z-only outside indications. An inspect-only
operation returns the original point even when outside; enforcement returns
the projected point. Inputs are passed by value and never partially mutated.

The native 100-unit inset and binary32 0.1 upper inset are reference test
values, not module-owned configuration. That binary32 upper inset translates
exactly to Fixed raw bits `429496736`; converting a decimal 0.1 directly gives
a different value. All runtime geometry uses integer comparisons and checked
integer construction, without floats or platform math. This preserves the
documented inequalities, not x87 rounding at every intermediate operation.

STA64 rejects negative inset/epsilon, reversed raw bounds, overflow in derived
limits, or X/Z extents too small to contain their upper projection value.
Flat Y bounds and zero inset/epsilon are valid. Rejection of unsupported tiny
or overflowing volumes is an explicit STA64 contract, not a claim about the
legacy executables on invalid maps. Validate with widened/checked arithmetic;
Fixed's wrapping Add/Sub operators must not admit a wrapped volume. Valid
enforced results are inside on reinspection and enforcement is idempotent.

Test all axes, strict equality and neighbouring Fixed bits, asymmetric upper
projection, Y-only reports, inspect-only preservation, corners, zero values,
flat Y, rejected negative/reversed/tiny/overflowing volumes, valid extreme
coordinates, idempotence and containment over a deterministic point grid.
These tests establish the prerequisite only; later stages still require real
commands, movement, save/replay and the campaign probe.

## Proposed shared seam

Use a small deterministic per-entity boundary state table, following the
existing subsystem pattern. Keep UnitFlags' current width and meanings. Use
stable entity IDs and remove state on destruction. Append an edit rather than
renumbering old tags. Validate privilege, live target, supported represented
object and usable bounds/rules before mutation. False must remain an explicit
operation; a same-value request is not permission to skip required checks.

The final state model must distinguish default initialization from explicit
mission control and the original A1 rearm behavior. Do not turn all legacy
profiles into A1 or discard false because a unit has reached the map. Do not
reset unrelated orders, velocity, cargo, weapons or physical state merely to
set the flag. A plain flag write must not teleport immediately when the
native setter does not.

Apply the documented finite-domain constraint to the paths that actually use
it: appropriate movement targets/waypoints, origin/position changes and
physical position updates. Audit ordering with attacks, mining, rig deployment,
rallies, formations, separation and obstacle avoidance. A ship must not steer
forever toward an unreachable off-map destination because only its final
position was clipped. Static map fixtures and background objects require
explicit treatment; a blanket clamp of every entity is not justified.

Expose Lua only after this core seam merges. Entity.forceOntoMap assignments
and A2.SetForceOntoMap must submit the same real simulation operation. Define
strict types, missing/dead policies, deferred spawn actions and any unsupported
read access explicitly. Do not invent a getter value from definition defaults
when the state is per entity.

## Authoring and persistence decisions to close

There is no general numeric mod-rule override mechanism in the current
Profile/Overrides boolean feature table. Numeric unit/physics values use the
resolved owning ODF. Add an explicitly named STA64-owned numeric rule only in
an appropriate resolved content field and file; do not add a hidden global
constant or claim a legacy key exists. Verify the actual defaults against each
reference, retain explicit/inherited zero and diagnose invalid values. Decide
whether initialization/rearm policy belongs in the existing boolean features.
Record exact keys, precedence, hashing and no-op behavior before implementation.

Use explicit versioned boundary persistence. A version-10 snapshot with the
old body and a separately versioned/validated boundary record is a candidate;
it is not yet an implemented format. Preserve the genuine version-8 Native
mining fixture and current version-9 continuation where the old rules still
apply. Define which legacy rule changes intentionally invalidate content
hashes. Do not claim general cross-build save or replay migration.

Hash every effective rule and state that can change future simulation results,
including applicable bounds. Preserve the old no-op Native path only when its
future behavior is unchanged. Validate state IDs/order/duplicates, bounds,
resource-independent rule ranges and malformed/trailing data. New files must
fail closed in old decoders, and new decoders must not reinterpret old bytes as
new boundary state.

## Required gates and tests

Before integrating entity confinement, settle native fresh-map versus saved-state initialization,
background/class scope, target versus physical-update ordering, all numeric
finite-domain comparisons, authoring keys and persistence migration. Declare
only the exact paths needed by the resulting stage on #287. Keep race/loadout,
render/UI and unrelated AI work outside this seam.

Exercise real command batches and subsequent movement: default versus explicit
true/false, inside/outside/edge/corner positions, all axes, entering from off-map,
A1 rearm versus A2/FO persistence, background exemption, queued/formation orders,
attacks/mining/build approaches and new obstacle avoidance. Check invalid edits
atomically against an identical baseline tick. Test override/inheritance/zero,
invalid numeric diagnostics and content/digest differences. Verify snapshot and
same-build replay continuation, old fixtures and malformed input rejection.

Stage 1 local verification on 2026-11-10, with main `dd6db8a` merged forward:
all five geometry tests pass; the scoped `sta_core`, `sta_odf`, `sta_bzn`,
`sta_content`, `sta_sim` and `sta_mission` suites report 559 passes, zero
failures and ten ignored external-install probes. The genuine version-8 Native
mining fixture continuation passes. Strict simulation/mission all-targets
Clippy, workspace all-targets checking and formatting/diff checks pass. Offline
workspace lock regeneration changes zero packages. An independent design and
source/test review found no blocking defect. This is shared host validation;
branch-specific full desktop CI and Android APK validation remain separate.

After the shared seam merges, add the Lua adapter with actual movement tests
and rerun the longer Blockade probe. Preserve other diagnostics, including
Invasion's unresolved colonizable-world transfer. Full campaigns, native game
runtime, desktop visuals and Waydroid/device acceptance remain separate gates.
