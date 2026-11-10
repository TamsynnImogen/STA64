# Map confinement and off-map mission control (SIM-22)

## Status and trigger

This is a pre-implementation proposal and native-audit record for issue #287.
No boundary state, new edit, default or Lua property is implemented by this
proposal. Resolve the gates below and record the final decisions before core
edits. Deliver the simulation seam before its LUA-05 adapter.

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

Before core code, settle native fresh-map versus saved-state initialization,
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

After the shared seam merges, add the Lua adapter with actual movement tests
and rerun the longer Blockade probe. Preserve other diagnostics, including
Invasion's unresolved colonizable-world transfer. Full campaigns, native game
runtime, desktop visuals and Waydroid/device acceptance remain separate gates.
