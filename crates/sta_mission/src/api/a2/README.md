# LUA-05 mission bridge

## Perceived identity

The merged shared perception seam backs `A2.SetPerceivedTeam(id, team)` and
`A2.GetPerceivedTeam(id)`. They use strict unsigned integral handles/numbers,
exact arity and the configured Slots/ArmadaTeams numbering. Both require a
current non-DYING object. The setter queues a correlated SCRIPT edit; the getter
reads the latest shared view. Neither changes real ownership or the Lua cache
optimistically. Missing/reserved/malformed arguments error before queuing.
As with `GetRealTeam`, ArmadaTeams returns nil for a synthetic reserved identity
it cannot encode; real slots and neutral return numbers. Native sentinel parity
is not claimed.

`Entity.perceivedTeam` reads a Player from the latest view, or nil while pending,
failed, absent or DYING. Assign a Player or strict integral player number (real
slots or neutral). Pending actors retain a typed action until their actual spawn
receipt arrives; failed receipts and rejected edits remain diagnosed. Actor-local
FIFO dispatch does not change Core's sorted cross-player application order.

The stock A2 native field is global and independent of real team. Its object
guard includes GameObject construction proxies; it is not the narrower crew
guard. Strict Lua validation, receipt handling, numbering and error policies are
bridge conventions, not native ABI/invalid-input parity claims. Automatic
weapon acquisition consumes shared perception; explicit Attack, control,
sensors, economy, victory and team colours retain real ownership. Additional
AI/client and native special-weapon perception policy is not claimed complete.
See [the pre-code contract](../../../../../docs/LUA05_PERCEPTION_BINDINGS.md).
The getter is not used by the currently inventoried DLL corpus and earns no
extra used-call coverage. No full campaign or playback acceptance is implied.

## Existing presentation bridge

`Cineractive`, `Media` and `UI` now queue `Edit::Present` commands from
`PlayerId::SCRIPT`. The sim raises the matching events; these bindings do
not render a shot/title or play a sound/movie themselves.

Call-shape evidence: Immy's clean-room `armada_missions` A2 ports (including
*Along the Neutral Zone*, *Into the Breach* and *UI Tutorial*) and the local
`A2FOMissions/missions/Recovery.lua`. No MMM implementation was copied.

- `Cineractive.active` and `.skippable` are readable/writable booleans.
- `:lookFrom(entity | Vector | Path(name), ...)` retains the source and
  `:lookAt(...)` completes a simple entity/vector pair with a zero-duration
  `Shot`. A new source does not produce a shot toward an earlier target.
  Other requests, including paths and extra arguments, stay generic `Call`
  events without adding an approximate shot. The number seen with paths is not
  interpreted as a duration or speed. `Path` currently keeps only a name.
- `:setCameraOffset(Vector)` produces `CameraOffset(relative=false)`;
  `:fadeIn(seconds)` / `:fadeOut(seconds)` fade from/to black. These are
  explicit bridge defaults. Unobserved overloads return errors.
- `Media:playSound(name)` returns a bridge sound id; `:stopSounds()` stops
  all sounds this runtime requested, in request order. `:playMusic(name)` /
  `:stopMusic()` and `:playMovie(name)` / `:stopMovie()` send typed events.
  Music defaults to looping and movies to skippable/fullscreen; native
  defaults and return-value compatibility have not been verified.
- `UI:addTitle(text)` / `:addSubtitle(text)` send typed titles. The observed
  `(text, x, y, seconds, boolean)` forms send `Call("UI.addTitle" |
  "UI.addSubtitle", [id, text, x, y, seconds, boolean])`. Placement and the
  final boolean are retained without guessing their interpretation. UI
  returns no handle. There is no evidenced title-removal API in this slice.
- Unknown methods retain permissive stubs and increment `unsupported()`;
  malformed calls to implemented methods return Lua errors.

The asset-free intro test drives a real `Sim`, reaches a synthetic first
objective, and replays its recorded presentation commands. It does not run
a full converted stock mission. Remaining LUA-05 gates include the
remaining DLL-named `A2` calls, unverified lifecycle defaults and forced input integration,
unverified Timer overloads and other missing MMM APIs, full converted-mission
tests, and client-side presentation/completion feedback. The package remains in progress.


## File-backed objectives and external validation

`Objectives:load(name)` (or `Objectives.load(name)`) reads through the same
caller-supplied content reader as the mission and its includes. The bridge
tries `objectives/name` and then the relative name. The provisional parser
accepts UTF-8 text
with an optional BOM, normalizes line endings, reads the first nonempty
line as the heading and each `*` line as a goal, and retains the briefing
and non-goal notes for host inspection. Missing files record an unsupported
call and preserve the previous list; malformed files return a Lua error
without partly replacing it. Assets remain outside the source tree.

Goals support one-based indexing and `.state = true` for completion, as
used by the converted ports; explicit active/complete/failed state names
are also accepted. Reloading replaces the list with fresh IDs, so handles
from an earlier file cannot accidentally modify a new goal. The runtime's
objective list and briefing remain host-only state; client display,
replication and objective presentation are still pending. `Player:loadAIP`
queues the requested file for the host (AI-06); a host without AI seats ignores it.

The ignored `tests/a2_port_intro.rs` probes accept `STA64_A2_PORTS` pointing
to the external ports' `a2/missions` folder and `STA64_A2_OBJECTIVES`
pointing to an installed objectives directory. The first-goal probe loads
Into the Breach unmodified, uses synthetic craft, and places the Enterprise
inside the script's discovery region to exercise its actual trigger.
It proves Lua scheduling, file goals and state changes, not native map
movement, complete mission play, or presentation playback. A separate
ignored probe loads every text file in the supplied objectives directory
without embedding any game text in the repository.


## DLL-named objective visibility

`A2.ObjectivesDisplay_Set_Visibility(visible)` requires exactly one Lua
boolean and sets the same runtime-local flag as `Objectives.visible`.
Objective contents, completion and briefing metadata stay intact. This updates
host mission state immediately and emits no sim or presentation command;
client rendering and native defaults are not acceptance claims. File-loading
and completion-selector DLL calls remain separate, unimplemented adapters.

## DLL-shaped random numbers

`A2.RandomGetUnsigned(upper)` takes exactly one positive unsigned 32-bit
integer (including an integral Lua number) and returns a Lua integer in
`[0, upper)`. Invalid arguments, zero and wrong arity raise errors without
advancing the generator. A bound of one still consumes a draw. It shares the
runtime's deterministic `math.random` stream and `math.randomseed` state,
captured before mission code runs; later replacing the global random function
does not redirect the binding. It emits no sim commands. The positive
upper-exclusive range is supported by the pinned A2 reference; native RNG
algorithm/sequence and mission save/resume parity remain unverified.

## Initial DLL-named object bridge

The `A2` table exposes the single-handle forms `Exists`, `DoesNotExist`,
`IsAlive`, `GetLocation`,
`GetRealTeam`, `RemoveObject`, `CraftCannotDie`, `Cloak` and `Decloak`.
The bridge maps handles to this runtime's `EntityId` values, available as
`Entity.id`; it does not accept native DLL pointers or saved engine handles.
Calls use dot syntax and the exact arity listed in `MISSION_DLL_API.md`.
No label/index overload of `GetLocation` is implemented in this slice.

Handles must be nonnegative integral values within `u32`; zero means no
entity. `Exists` reports whether a unit is present in the current world view;
`DoesNotExist` is its complement. A represented dying or zero-hull unit still
exists. `IsAlive` returns false for absent or dying units, while a represented
zero-hull unit without the dying flag is alive under this bridge's policy.
These three queries return booleans and emit no commands. The other calls
return Lua errors for absent or dying units. These are explicit bridge policies,
not verified native error or sentinel conventions. `CraftCannotDie` requires
a boolean. `GetLocation` returns a fresh Vector and `GetRealTeam` returns the
snapshot owner number in the configured player-numbering mode. Writes are queued: `RemoveObject` and
`CraftCannotDie` use SCRIPT edits, while cloak orders use the unit's current
player owner and retain normal capability/energy checks in the sim.
Reads, including both existence queries, see the last world snapshot until
the next controller tick supplies a refreshed view. Queuing or applying
`RemoveObject` does not change an existing mission view by itself. The sim
sorts commands by player and sequence, so edits and owner orders need not
apply in the order of interleaved Lua calls.

`A2.SetRealTeam(id, team)` takes exactly two unsigned integral numbers and
queues the merged shared SCRIPT Team operation. The configured Slots mode
accepts slots 0..7 or neutral 253; ArmadaTeams accepts 1..8 or neutral 0.
Reserved/malformed destinations and absent/dying handles raise Lua errors
without queuing. Core remains authoritative for configured/defeated seats,
supported targets, construction/cargo cleanup and capture-loss timing. Reads
remain on the old snapshot until the host advances; no mission-side ownership
algorithm or immediate cached-owner write is added. Direct unfinished-child
transfer remains unsupported in Core. Unknown
`A2` names return nil rather than implying a completed binding via a stub.
The inventory distinguishes these exact callable signatures from older
MMM job-equivalence mappings; neither classification proves native parity.

`A2.SetForceOntoMap(id, enabled)` takes exactly an unsigned integral live
handle and strict Boolean, then queues the shared SCRIPT ForceOntoMap edit.
`Entity.forceOntoMap = enabled` uses the same operation, including deferred
pending-spawn actions. Pending IDs are resolved from actual spawn receipts;
failed spawns retain existing diagnostics and actor-local deferred actions retain
dispatch order. Core still sorts SCRIPT and owner commands by player/sequence;
their application need not follow interleaved Lua call order. Await the next
host view when an owner order must observe a script edit.
Core decides capability, map geometry and entry/rearm behaviour even
for false/same-value writes. There is no Boolean getter: the current WorldView
does not carry the flag, and the bridge must not guess its value.

These operations and the existing Entity.player write track the SCRIPT
sequence only when commands are drained to the host. On the next strictly
newer view, a matching SCRIPT rejection becomes a detailed runtime error and
a fixed operation-specific unsupported count. Successful/failed settled rows
are retired; unrelated players/sequences and unsubmitted/same-tick events do
not create false failures. Detailed errors share the runtime's existing
100-error cap, while all matching failures are counted. The host must deliver
the full events of the submitted batch with that next view, as for spawn
receipts. No speculative immediate success or state update is reported.

`A2.ObjectivesDisplay_Set_Text_From_File(path, resetCompleted)` requires
exactly a UTF-8 Lua string and strict Boolean and returns no values. It reuses
the existing atomic content-reader loader. True resets new goals to Active;
false preserves only completed ordinal slots. A successful replacement uses
fresh monotonic IDs and new metadata while retaining display visibility.
Missing files retain the old state with the existing diagnostic; invalid
paths, malformed files and allocation failures retain old state and error.
This is the documented bridge parser/path policy, not native ABI or save parity.

`A2.SetCurrentHealth(id, hull)` takes exactly one unsigned integral live ID
and one finite Lua number; malformed arguments and missing/dying objects raise
errors without commands. It queues the same absolute SCRIPT hull edit as
`Entity.hull = value`. Each assignment applies in command order, without using
a damage delta from a stale snapshot or restoring shields. The sim clamps the
fixed-point value to zero through maximum hull, with an immortal floor of one
bounded by maximum hull. Finite values beyond the fixed-point range saturate
before clamping; non-finite values are rejected. Reads see the old snapshot
until the host advances. Native invalid-input and float ABI parity are unverified.

`A2.SetSpecialEnergy(id, energy)` requires exactly two arguments: an unsigned
live positive-hull ID with a content-defined energy pool and a finite Lua
number. Invalid arguments or ineligible targets raise errors before queuing.
It queues an absolute SCRIPT `Edit::SetEnergy`; finite values saturate at the
fixed-point boundary and the sim clamps to zero through the configured maximum.
Normal regeneration and cloak cost/drain still run in their usual tick phases.
Maximum and refill rate come from the unit's resolved `maxSpecialEnergy` and
`specialEnergyRate` ODF keys. Child/include overrides remain authoritative:
maximum zero disables the pool and rate zero prevents regeneration. This
adapter adds no replacement maximum or forced refill default. Absolute input
units are supported by the pinned A2 reference; these validation/clamping
policies and native float ABI/default parity remain separate claims. No new
energy getter, view field or mission save/resume support is implied.


The shared order enum is available as both `Order` and `GameObject.Order`,
which refer to the same table. This supplies the namespace used in 104
calls in the converted A2 ports. `Move` explicitly aliases the existing `Go`
value. This spelling occurs at 17 observed calls across eight ports with vector
or entity targets. It inherits Go's point conversion at the time of the Lua call and
optional queue flag; it does not follow a moving target. Raw string `"Move"`,
path overloads, Colonize and other unimplemented orders remain separate gaps.
This is a STA64 port-compatibility policy, not native Go/Move equivalence.

`Entity:giveOrder(Order.Mine)` (also with an explicit nil target) selects
the nearest content-backed resource by exact fixed-point XZ distance at the
time of the Lua call. Height is ignored. Equal distances prefer existing
nodes by ID, then validated pending resources by request order. Definitions
must satisfy Core's shared mineable-definition rule; typed backgrounds and
resource-less producer planets are excluded for both current and pending
objects. Resource metadata alone and missing definitions do not qualify. Current DYING nodes
are excluded. This is an explicit STA64 rule, not a verified native default.
The selected typed reference waits for real spawn receipts and retains per-actor
order sequence. A failed or dead selected pending node drops the order without
retargeting. Explicit entity targets keep their existing behavior; other target types
or no candidate records an unsupported diagnostic.
The selection does not inspect visibility, remaining ore, cargo or depot state;
normal core ownership, miner eligibility and mining behavior still apply.


`A2.DisableEngines(handle, disabled)` and
`A2.DisableWeapons(handle, disabled)` require explicit booleans and use
SCRIPT engine/weapon edits. Passing true disables the system; false
reenables it. The engine edit clears its current movement target and speed
when disabled, following the existing sim behavior. These calls share the
object bridge's argument, live-handle and snapshot rules. Sensors, shields
and life support are not exposed by this slice because the simulation
rejects those edits.


## Queued Entity.add bridge

`Entity.add(odf, player, Vector)` accepts a known unit ODF (with optional
`.odf` suffix), a player number in the configured mode or its Player handle,
and finite fixed-point coordinates. Owned Slots numbers are 0..7; opt-in
ArmadaTeams uses 1..8. The neutral owner is 253 in Slots or 0 in ArmadaTeams.
Neutral spawning requires a resource definition: the canonical core kind is
Resource or the definition has explicit resource metadata. Neutral nonresources
and reserved SCRIPT/BENCH owners are rejected before request allocation.
Resources remain neutral even when given an owned player, matching the core's
existing normalization. The request uses heading +Z as a bridge default. It queues a
SCRIPT SpawnRequest rather than mutating the simulation or predicting an ID.

The returned handle has `.pending == true`, `.id == nil`, and `.valid == true`
for a locally validated request. Its getters expose uncommitted definition,
owner and position hints; it is not yet in EntityFinder or the world view.
A real ScriptSpawned receipt resolves its ID on the next controller view.
Rejected, missing or conflicting receipts invalidate it. A receipt may name
an entity already destroyed that tick: the real ID remains inspectable but
the handle is invalid and its pending actions are discarded.

Move/stop/attack/mine/special/cloak orders and the existing remove,
invincibility, hull and confinement edits wait for all pending references, including a
pending target. They dispatch once before that tick's Lua callbacks, through
the normal owner orders or SCRIPT edits. Later actions for the same unit wait
behind an earlier blocked action, preserving attack-then-stop intent order.
The sim still validates capabilities
and sorts commands by player/sequence; global Lua call order is not promised.
Name, position and shield writes remain unsupported. Assigning a pending
handle's player remains unsupported and diagnosed; resolved owner changes use
the shared guarded Team operation described above.
Camera spots can wait for real spawn receipts, with ordered requests and explicit
scene cancellation. Destroyed monitors also retain pending request identity,
then match the resolved ID against the same batch's death events. Failed receipts
stop those hooks without a death callback; stop/unhook cancellation before polling
prevents activation. See `docs/MISSIONS.md` for the existing batch callback rules.

A host must use one MissionRuntime for its SCRIPT command stream and deliver
the complete submitted-batch events with the next view. Tokens and command
sequences are checked; data held for pending actions contains no Lua values,
and resolved requests leave the outstanding registry. Save/resume of Lua
and pending handles, multi-runtime SCRIPT ownership and native immediate
spawn timing are not completion claims of this bridge.

The ignored external spawn-helper probe invokes Into the Breach's unmodified
spawnAttack method explicitly with a repository-only synthetic ship and toy
beam. It verifies a real receipt and accepted deferred Attack targeting the
Enterprise. This is not a naturally timed Borg wave, native ship data,
complete original-map play or client rendering proof.

## Static entity cleanup

`Entity.remove(entity)` requires exactly one Entity userdata and shares the
existing `entity:remove()` implementation. It queues a real SCRIPT Kill rather
than changing the current world view immediately. Pending handles wait for the
actual spawn receipt; failed requests discard the queued action. Stale resolved
handles retain the existing kill/rejection behavior, without an idempotence
guarantee. Numbers, batches and other userdata are not accepted by this static
entry point. This supplies the converted ports' cleanup call shape, without
claiming native timing or adding an A2 DLL-signature binding.

## Runtime inspection ownership

The runtime's inspection methods return only sealed owned scalars, strings,
options, vectors and tuples; they cannot export Lua Table/Function/userdata
handles or admit custom Rust conversions that retain the VM. `eval_with`
passes sealed arguments through Lua varargs without string interpolation.
The explicit mlua `error-send` feature and a compile-time guard keep errors
within that ownership boundary as well; mlua `send` is not enabled. Compile-
fail examples reject Lua handles and custom result/argument conversions,
and a positive test moves the runtime while retaining copied inspection data.

Filter fields and vector coordinates can invoke Lua metatables. They are
read before borrowing the context, so callbacks may queue orders or edits;
view iteration still snapshots candidates before predicate callbacks.

## Later headless validation

The opt-in installed-map test mounts actual STA2 Classic content and
`a2_fed05.bzn`, using the explicit ArmadaTeams numbering adapter. The unchanged
external port resolves the Enterprise, portal and base, then naturally spawns
four Borg cruisers at tick 6120 and issues their accepted attacks after the
actual receipts at tick 6121. This validates commands/receipts, not combat or
complete mission play. See [mission conventions](../../../../../docs/MISSIONS.md)
for the numbering mode, bounded static Craft ancestry, absolute hull writes,
and DLL-shaped health, definition-race, distance and visibility queries. Definition
race is the available data source; mutable SetRace/assimilation overrides are
not modeled. GetOriginalRace returns owned definition data, not a native pointer.

Named condition monitors now support the converted UI tutorial's observed
five-argument constructor and OnTrue hook form. The early ignored probe
checks timed rules on empty synthetic views, without incoming input or manually
invoking rule methods. A second probe supplies installed objective text and
naturally reaches tutorial step 1 at tick 511 after cinematic update/finish
callbacks, loading three goals and awaiting input. Debug still uses a permissive
fallback tracked by `unsupported()`. The bounded TextInput binding now consumes
accepted player MissionText commands through the normal sim/replay path. Further
empty-world probes use the authored NEXT fallback to reach UI Tutorial's step 15
and Economics Tutorial's delayed finish through 14 and 9 text commands. Those
checks establish objective flow, not physical tutorial actions or client input.
Native timing, forced skips and full tutorial play remain unverified.

The campaign startup sweep now covers all 36 header-selected installed maps for
30 ticks each. A source-only mount resolves 34 scripts; the staging helper adds
byte-identical BZN-stem aliases so all 36 resolve from an independently verified
package layer. Uncaught Lua errors and rejected commands are checked separately
from handled log messages (199 across six missions in the current inputs).
This leaves property/ownership, AI, later gameplay and client acceptance work open.

## IsStation contract and native-family boundary

The pre-implementation design for `bool IsStation(int)` was committed before
the binding. It is implemented with focused runtime and inventory tests.
Use the latest mission view and the already-resolved, hashed
`UnitDef.is_starbase` from CON-24. No Raw ODF reads, new core fields, actor-kind
changes, gameplay commands or platform branches belong in this query.

The pinned stock A2 wrapper checks for an object, the native Craft type and
its dead marker before reading the class flag. Static prototype-label/vtable
associations and RTTI in the owned A1/A2/FO inputs establish the following
bounded canonical families. Shared labels agree across inspected versions;
this union is a compatibility bridge, not a claim that every version admits
every listed class or every hook/alias was observed at runtime.

| Native family | Canonical labels |
|---|---|
| Craft | cargo_ship, colony, constructionrig, craft, evolver, freighter, hover, minearray, mining, p_mining, planet, pod, producer, repairship, research, scavenger, sensor, shipyard, starbase, trading_station, wingman |
| Outside Craft | areaeffect_obj, asteroidfield, background_obj, blackhole_obj, constplaceholder, i76building, ionstorm_obj, latinum, nebula_obj, planet_obj, planet_shield, scrap, u_ball_obj, upgrade, wormhole_obj |

Named A2 leaf constructors corroborate ResearchPod, MineArray and Colony's
Craft base; Producer and MiningStation preserve it for Planet/TradingStation.
Of the 36 union labels, hover and wingman occur only in the inspected A1
prototype table. The extended audit covers the remaining 34 A2 labels:
32 named instance constructors have matching base chains and local type-flag
writes. The upgrade and
planet_shield instance factories return null rather than creating an object.
Entity initializes its type flags to zero; the observed non-Craft constructor
chains do not add the Craft bit, and Craft descendants preserve it locally.
This is bounded static constructor evidence; arbitrary helper/hook behavior
and mutable runtime type flags are not proven by the table.
The native construction placeholder instead derives from GameObject without
the Craft type. In normal STA64 simulation, `Some` build progress represents
a production weld using the final definition; treating that proxy as a
finished native Craft would give a wrong answer. Stable identity across
construction completion remains a broader unresolved model difference.
Private hashes and bounded excerpts stay outside the repository. The owned
A2FO A1 compatibility policies do not replace the A2 wrapper's type guard.

The explicit STA64 bridge contract is:

- Exactly one unsigned 32-bit integer handle, using existing A2 validation;
  malformed type/arity errors occur before inspecting state.
- Missing or DYING objects return false using the existing view-liveness
  convention. This query adds no independent hull threshold.
- Any `Some` construction progress returns false, including a represented
  progress of one until finalization clears it.
- A finished represented object without a unit definition errors.
- A resolved false flag returns false, including an unclassified class:
  the conjunction is already false and no ancestry guess is needed.
- For a true flag, known Craft-family labels return true and known non-Craft
  labels false, independently of `EntityKind`. Match ASCII case-insensitively.
- An unclassified true-flag class errors and increments a stable per-class
  `unsupported()` diagnostic before returning the error. Catching it with
  `pcall` does not hide the gap. In particular buildyard, fighter, asteroid
  and moon aliases have no demonstrated native-family association in these
  references. Do not silently guess from STA64's class lists or movement.

All successful queries leave commands and simulation state untouched.
Argument errors, missing definitions and unclassified-class diagnostics are
STA64 policy, not claimed native invalid-input/sentinel/ABI behavior. The
query does not implement A1 ScriptInterface support, capture, colony
simulation or complete mod-class coverage. Android shares this code; device
runtime and the December desktop alpha remain separate acceptance gates.

Tests must exercise conflicting actor kind/class/flag combinations, known
native pods/planets/non-Craft types, metadata inherited/explicit zero,
construction proxies and finalization, missing/dying/definition-less views,
strict arguments, unknown false versus caught unknown true diagnostics,
read-only commands/digests and refreshed snapshots. Check exact signature
normalization against wrong return/argument/arity shapes, regenerate the
inventory, and run mission/sim regressions, strict Clippy and workspace checks.

## DLL-shaped objective completion contract

Pre-code contract for the independent LUA-05 objective-state slice. Expose
`A2.ObjectivesDisplaySetObjectiveComplete(slot, complete)` with exactly two
arguments. The slot is a zero-based ordinal in the current objective list,
not the one-based `Objectives.objectives` index or a persistent objective ID.
The pinned stock A2 wrapper passes the integer unchanged; its complete leaf
indexes the completion-byte vector directly and returns for a nonnegative
slot at or beyond the current list length. The independently checked own-image
FO wrapper/leaf corroborates this bounded behavior. Private static captures,
image hashes and receipts stay in `STA64-artifacts/native/LUA-05-objective-completion`;
no original bytes or MMM implementation are copied here.

Require a nonnegative signed-i32-range Lua integer, or a finite integral Lua
number in that range, and a strict boolean. Negative/native unsafe indexing,
fractional/nonfinite/out-of-range numbers, coercions and wrong arity raise Lua
errors before mutation. A valid nonnegative slot outside the current list is
a successful no-op, including an empty list; it is not an unsupported call.
The native leaf lacks a negative-index lower-bound guard. Rejecting that input
is an explicit STA64 safety policy, not native invalid-input/ABI parity.

For an existing slot, true sets `ObjectiveState::Complete` and false sets
`ObjectiveState::Active`, including a previously Failed objective. This clears
the completion flag; it does not select a failure state. Update the existing
host state immediately, preserving the objective ID/title/description, other
objectives, briefing, visibility and ID allocator. Queue no simulation or
presentation command. Keep runtime instances independent. Do not route through
one-based indexing or interpret a selector as a stored handle after file reload.

This does not implement the separate objective-file loading boolean adapter,
native display colors/check-mark rendering, objective persistence, A1 calls or
complete mission playback. It has no dependency on unmerged SIM-22 and changes
no core/content/client/defaults/dependencies. Android uses the same adapter.

Tests must distinguish slots from one-based indices and nonsequential/reloaded
IDs; check true/false/Failed reactivation, empty/out-of-list no-ops, strict
invalid-input atomicity, runtime isolation and preserved host metadata with
no commands or unsupported credit. Extend only the exact inventory signature
and its source-coverage tests; regenerate the report rather than editing it.

The completion adapter is implemented under this contract. All five new
completion tests and three existing visibility tests pass. The full mission
suite reports 185 passes, zero failures and 10 ignored; ignored installed-port
probes are not campaign-playback evidence. All 17 Python inventory tests pass.
Regenerating against the original 72 A2 and 59 A1 DLLs reports 24 exact A2
callables and 108 missing signatures; all 133 binary/map/DLL input hashes remain
unchanged. This count is source-level callable coverage, not native GUI or
simulation parity. Independent read-only review found no confirmed defect.

Strict mission all-target Clippy, workspace all-target checking, Android ARM64
client-library checking through cargo-ndk/NDK 29 API 26 and changed-file
formatting/diff checks pass. The library check is not an APK or device result;
the separate SIM-22 APK does not contain this independent branch's adapter.
No new in-game eye-check applies to this host-state-only addition. The parent
LUA-05 package and complete mission playback remain unfinished.


## Rule-condition queries (LUA-10)

The stock Armada II rule DLLs (`R.drl`) call the engine from their rule
conditions (28 distinct signatures; `docs/MISSION_DLL_API.md` lists them with
rule-DLL counts and condition sites). `conditions.rs` binds all of them on
`A2`. Counts (`CountUnits`, `CountUnitsNearObject`, `CountVisibleUnits*`,
`CountStations`), `GetCrew`, `GetDilithium`/`GetLatinum`/`GetMetal`,
`GetTime`, `HasTimeElapsed`, `IsExploding` and `GetDistance(object, path,
point)` read the latest view (paths come from `A2.paths`, which the host
fills; a missing path raises and is counted). `UnderAttack(object, seconds)`
is a `Hit` on the object within that mission time (the runtime keeps the last
hit tick per object). `IsAudioMessageDone` / `IsMoviePlaying` follow
`Media:playSound` / `playMovie` and the client's reports
(`A2.AudioMessageDone(id)`, `A2.MovieDone()`).

Bound but raising (counted in `unsupported()` as `A2.<Name>: <why>`, never a
guessed value): `CountCapitalShips` (no capital-ship class in the content
data), `IsShipAliveAndOffMap`, `IsCargoShipReadyToLoad`,
`IsCargoShipFullAndReadyToSend`, `GetLatinumTraded` (no cargo, off-map or
trade state in the world view).

`IsCompromised(object, team)` (LUA-12) is true when the object has been lost
to `team`: gone or dying, not a craft or station, a derelict (crew under one
with a positive maximum), or owned by another team than `team` (numbered like
`GetRealTeam`). Read from the executable, static; not measured. Approximated:
the engine's derelict latch (the `CraftCannotDie` exemption, clearing only
when crew returns to one) and the original owner during a temporary takeover
(listed on SIM-59; `GetRealTeam` has the same approximation).

Evidence: signatures and argument constants read from the executable and the
rule DLLs, static. The team / radius / ODF-filter readings follow the `A1`
bindings and are not measured; the `UnderAttack` and `HasTimeElapsed` meanings
are not measured.

## Orders, AI, crew and team calls (`orders.rs`)

DLL-named `A2.Goto Stop Attack AI_Attack Mine SetImpulseSpeed ResetImpulseSpeed
GetImpulseSpeed SetRedAlert SetGreenAlert GetNearestEnemy
GetAttackLocation` and the `SetCommandWhat{Who,WhoParam,WhoWhere}VO` family.
Every call uses the exact argument shapes of the DLL (overloads by argument
type), strict validation, and the rules below, read from the executable's
`ScriptInterfaceImp` bodies (static) and from the stock scripts' argument
constants. Orders are sent as the unit's owner; edits are SCRIPT edits.

- An absent or dying unit (or target) is left alone: no command, no error, as
  the engine does. A wrong argument shape is a Lua error and queues nothing.
- `Goto(unit, path|object, warp, int)` is GO (GO_WARP when `warp`); the `int`
  is never read. A path is `A2.paths[name]` (host-filled, as for
  `GetDistance`): `Order::Move` to its first point, the rest queued. An object
  is a snapshot of its position (whether the engine tracks it: not measured).
  `warp = true` is `Order::MoveWarp` on the first leg (SIM-56; the sim warps like
  an attack approach when the goal is over 800 away).
- `Patrol(unit, path)`, `SearchAndDestroy`, `Scout`, `AI_Attack(unit)` (attack
  what the sensors show) and `AI_Attack(unit, Vector)` (fly to weapon range of
  the point, firing nothing) are `Order::Patrol` / `SearchAndDestroy` / `Scout` /
  `AttackAny` / `AttackPoint` (SIM-56, rules in `sta_sim::orders`).
- `AllowAsTarget(object, team, bool)`, `CraftCanRepair`, `ButItCanBeDerelicted`,
  `CraftCanRecrew`, `HideObject`, `SetName` are typed edits (SIM-58, rules in
  `sta_sim::switches`).
- `Stop`, `Attack(unit, object, int)` and `AI_Attack(unit, object)` map to
  `Order::Stop` / `Order::Attack`; the extra `int` is never read. `Mine` maps
  to `Order::Mine`; an absent node means "mine without a target" (the nearest
  node in the view, as the omitted-target Mine helper chooses).
- `SetImpulseSpeed` / `ResetImpulseSpeed` / `SetRedAlert` / `SetGreenAlert`
  queue `Edit::MaxSpeed` / `Edit::Alert` through the edit-feedback tracker. The
  sim rejects both edits today (no movement or alert package applies them):
  the rejection is counted as `A2.<Name>: simulation rejected edit`, so a
  mission sees the call succeed while the world does not change. `GetImpulseSpeed`
  returns the value this runtime last set, else the class `impulseSpeed`.
- `BuildObject` lives in `dll_objects.rs` (next section). Its handle is a
  token (`0x40000000 + request`); `dll_objects::finish`, installed last, wraps
  every `A2` function so the token means the built unit once the spawn receipt
  is in. An order of this file sent to a token that is still pending, or whose
  spawn failed, raises and is counted (`A2.<Name>: handle is still being
  built; use it after the next tick` / `handle names no object: spawn
  failed`); an absent or dead real handle stays a silent no-op as in the
  engine.
- Path names are looked up exactly first, else by the one key that matches
  ignoring case; two such keys are an error (`dll_objects::lookup_path`).
  A name the map does not have (stock scripts name paths of other maps; read
  from data) is the engine's null lookup (read from the executable, static):
  `Goto`, `Patrol` and `PointAt` do nothing, `BuildObject` returns 0 and
  `GetLocation(path, i)` returns the default vector (also for a point out of
  range; the vector's value is not measured, `ABSENT_PATH_LOCATION`, zero). None
  of these is counted. Other path calls keep raising and counting (LUA-18).
- `GetNearestEnemy(unit, r)`: nearest enemy by XZ distance inside the square
  of half side `r` that can see the unit; 0 if none. `GetAttackLocation(a, b)`:
  the point of `a`'s bounding sphere (`UnitDef::radius`) toward `b`, or one
  radius along +x without `b`; raises when the radius is not in the content.
- `SetCommandWhat...VO` (bound here, not in `native_text.rs`): `WhatWhoVO` maps
  GO, ATTACK and MINE toward an object to orders; `WhatWhoWhereVO` maps BUILD at
  a position; both objects must be live or nothing happens. `WhatWhoParamVO`
  raises (a parameter has no order); other commands raise, counted. The voice
  flag is not measured and not applied.

Raising and counted (no seam): `Colonize
Special_Attack SetCommandMoveAutonomyLow PointAt SetCrew SetRace
GetMainEnemyBaseLocation AttackMainEnemyBase GetAllyStationAttacks`.
`AI_Team_Load_AIP` is accepted like `Player:loadAIP`: the request is queued
and the host gives it to the seat's AI (`MissionRuntime::take_aip_requests`,
`sta_ai::MissionAiHub`; AI-06). Each reason is in `unsupported()` as `A2.<Name>: <why>`.


## Object creation, searches and flags (step 2)

`dll_objects.rs`. Grades: exe = read from the executable, static; args = read
from the stock scripts' constants; not measured otherwise.

- `A2.SetSpecialForcesFlag(h, on)` (1800 stock calls) stores a mission-side
  flag on a live Craft and returns `h`; an invalid handle returns 0 with no
  effect (exe). The flag means "reserved for the script, not for AI teams";
  no AI team runs in a mission runtime, so its consumer today is
  `RecruitSpecialForces`. It emits no command.
- `A2.RecruitSpecialForces(odf, team[, out])` returns `ok, handle`: the first
  unflagged craft of that team and ODF is flagged (exe).
- `A2.BuildObject` (path form and object form) queues a SCRIPT spawn and
  returns a pending handle (`0x4000_0000 + request`, as `A1`). `finish` wraps
  every `A2` function so a token with a spawn receipt reaches it as the real
  id; before the receipt it names no object (existence queries say no; the
  order calls of `orders.rs` raise and count). Path index out of range or an
  unknown ODF returns 0 (exe for the index; the ODF case is counted); a path
  missing from `A2.paths` raises and is counted (the engine's behaviour for a
  missing path is not measured). The object form
  accepts only a zero offset (rotation convention not measured).
- `A2.GetNearestObjectNearPosition` / `NearMe`: exe filters (radius < 0
  unlimited, team < 0 any, perceived flag, exclude handle); squared XZ
  distance, ties to the later unit. Mask other than -1 raises (not measured);
  derelict crafts are not skipped (no derelict state in the view).
- `A2.GetFirstCraft`, `A2.GetLocation(path, index)` (0-based, `A2.paths`).
- `DestroyCritical` queues a lethal `Damage`, `ExplodeNow` a `Kill`,
  `SetCanBeDecommissioned` the typed edit the sim rejects today (reported and
  counted by the edit feedback); `DisableShieldGenerator`, `SetShieldPercent`,
  `SetMaxShields` and `Damage<System>` are applied by the sim (SIM-55).
- Calls with no sim seam (position, per-team targeting, hidden/derelict/name/population/wormhole/cargo/fleet state) raise
  and are counted; see `seams()` for each reason.


## Script-taking presentation and title calls (LUA-05)

`native_flow.rs` binds `FadeIn`, `FadeOut`, `CineractiveStart`, `AudioMessage`,
`PlayMusic`, `StopAudioMessage`, `PlayCinematicMovie` and `WaitForEnterKey`;
`native_text.rs` binds `ShowTitle`, `ShowSubtitle`, `RemoveTitle` and
`RemoveSubtitle`. All queue SCRIPT `Edit::Present` events; there is no client
consumer yet, so nothing is played or shown.

The DLL's leading `script *` is the first Lua argument: any table that stands
for the calling script. In the engine the trailing bool is "don't wait": false
adds one reference to the script's current step, released when the fade ends,
the sound or movie finishes, or Enter is pressed. The binding records the same
count; `A2.ScriptWaits(script)` returns it so a converted step machine can
advance at zero. Nothing blocks the Lua call. Releases: a fade after
`delay + duration` mission seconds, a sound by `A2.AudioMessageDone(id)` or
`A2.StopAudioMessage(id)`, a movie by `A2.MovieDone()`, plus three host reports
that are not DLL calls: `A2.EnterKeyDone(id)` (id of the queued `WaitForKey`),
`A2.CineractiveStartDone()`. Until a host reports them those waits stay open.

- `FadeIn/FadeOut(script, duration, delay, white, dontWait)`: typed `Fade` when
  the delay is 0 (black, or white when the third float is non-zero), otherwise
  `Call A2.FadeIn|FadeOut [duration, delay, white]` since `Fade` has no delay.
- `AudioMessage` / `PlayMusic(script, name, dontWait)` return the overlay id
  (one counter from 1 for both); with a wait they return -1, playing nothing,
  once `Mission.finish` has run. Voice is `Sound { id }`, music is `Call
  A2.PlayMusic [id, name]` (the typed `Music` has no id); `StopAudioMessage(id)`
  queues `StopSound` only for an id still playing.
- `CineractiveStart` sets the shared `Cineractive.active`; `PlayCinematicMovie`
  queues a full-screen skippable `Movie` (skippability not measured) and
  `IsMoviePlaying` follows; `WaitForEnterKey` queues `WaitForKey { id }`.
- Titles: ids count from 0 for titles and subtitles together. A show queues the
  typed `Title` (label key as text) and `Call A2.ShowTitle|ShowSubtitle [id, x,
  y, fadeIn]`; negative x means centred (client). `RemoveTitle|RemoveSubtitle
  (id, fade)` queue `RemoveTitle` and a `Call [id, fade]`, only for an id on
  screen, as the engine ignores others.
- The `SetCommandWhat...VO` calls are in `orders.rs` (see above).

Evidence: argument roles, the don't-wait inversion, the fade fields, id
counters and the mission-over rule read from the executable, static. Not
measured: release of a stopped sound's wait, the cineractive wait's release,
counter reset on mission load, movie `int` (1 in every stock call).

## Dead-handle answers (LUA-21)

`GetDistance(a, b)`, `GetCrew`, `GetCurrentHealth`, `GetMaxHealth` and `GetLocation(handle)` answer the
executable's constants for a handle that names nothing live (1e30, -1.0, the zero vector) instead of
raising. Read from the executable, static; not measured. `GetDistance(object, path, i)` was not read and
still raises; `GetRealShieldPercent` and `InQueue` are unbound on A2.
