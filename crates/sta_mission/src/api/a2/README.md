# LUA-05 presentation milestone

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
records the requested file and an unsupported diagnostic; it does not run AI.

The ignored `tests/a2_port_intro.rs` probes accept `STA64_A2_PORTS` pointing
to the external ports' `a2/missions` folder and `STA64_A2_OBJECTIVES`
pointing to an installed objectives directory. The first-goal probe loads
Into the Breach unmodified, uses synthetic craft, and places the Enterprise
inside the script's discovery region to exercise its actual trigger.
It proves Lua scheduling, file goals and state changes, not native map
movement, complete mission play, or presentation playback. A separate
ignored probe loads every text file in the supplied objectives directory
without embedding any game text in the repository.


## Initial DLL-named object bridge

The `A2` table exposes the single-handle forms `IsAlive`, `GetLocation`,
`GetRealTeam`, `RemoveObject`, `CraftCannotDie`, `Cloak` and `Decloak`.
The bridge maps handles to this runtime's `EntityId` values, available as
`Entity.id`; it does not accept native DLL pointers or saved engine handles.
Calls use dot syntax and the exact arity listed in `MISSION_DLL_API.md`.
No label/index overload of `GetLocation` is implemented in this slice.

Handles must be nonnegative integral values within `u32`; zero means no
entity. `IsAlive` returns false for absent or dying units. The other calls
return Lua errors for those units. These are explicit bridge policies,
not verified native error or sentinel conventions. `CraftCannotDie` requires
a boolean. `GetLocation` returns a fresh Vector and `GetRealTeam` returns the
snapshot owner number in the configured player-numbering mode. Writes are queued: `RemoveObject` and
`CraftCannotDie` use SCRIPT edits, while cloak orders use the unit's current
player owner and retain normal capability/energy checks in the sim.
Reads see the last world snapshot until the next controller tick. The sim
sorts commands by player and sequence, so edits and owner orders need not
apply in the order of interleaved Lua calls.

`SetRealTeam` is deliberately absent: although `Edit::Team` is in the
command enum, the simulation currently rejects it. Ownership transfer
needs an implementation that handles existing orders, build queues and
other owner-dependent state before this binding can be offered. Unknown
`A2` names return nil rather than implying a completed binding via a stub.
The inventory distinguishes these exact callable signatures from older
MMM job-equivalence mappings; neither classification proves native parity.


The shared order enum is available as both `Order` and `GameObject.Order`,
which refer to the same table. This supplies the namespace used in 104
calls in the converted A2 ports; it does not add behavior for order values
that the dispatcher does not implement, including Colonize. The ports'
Move spelling remains a separate unsupported gap.


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
`.odf` suffix), an owned player number in the configured mode or its Player
handle, and finite fixed-point coordinates. The default Slots mode uses 0..7;
opt-in ArmadaTeams uses 1..8 (neutral spawning is unsupported). The request uses heading +Z as a bridge default. It queues a
SCRIPT SpawnRequest rather than mutating the simulation or predicting an ID.

The returned handle has `.pending == true`, `.id == nil`, and `.valid == true`
for a locally validated request. Its getters expose uncommitted definition,
owner and position hints; it is not yet in EntityFinder or the world view.
A real ScriptSpawned receipt resolves its ID on the next controller view.
Rejected, missing or conflicting receipts invalidate it. A receipt may name
an entity already destroyed that tick: the real ID remains inspectable but
the handle is invalid and its pending actions are discarded.

Move/stop/attack/mine/special/cloak orders and the existing remove,
invincibility and hull edits wait for all pending references, including a
pending target. They dispatch once before that tick's Lua callbacks, through
the normal owner orders or SCRIPT edits. Later actions for the same unit wait
behind an earlier blocked action, preserving attack-then-stop intent order.
The sim still validates capabilities
and sorts commands by player/sequence; global Lua call order is not promised.
Name, position and shield writes remain unsupported. Owner transfer remains
unsupported by the sim; assigning a pending handle's player is diagnosed.
Camera spots and destruction monitors currently require a resolved ID.

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
for the numbering mode, bounded static Craft ancestry, and the three read-only
DLL-shaped health/ODF queries.

Named condition monitors now support the converted UI tutorial's observed
five-argument constructor and OnTrue hook form. The early ignored probe
checks timed rules on empty synthetic views, without incoming input or manually
invoking rule methods. A second probe supplies installed objective text and
naturally reaches tutorial step 1 at tick 511 after cinematic update/finish
callbacks, loading three goals and awaiting input. Debug/TextInput still use permissive global
fallbacks; their missing global/field lookups now appear in `unsupported()`.
These diagnostics do not implement input. Native monitor timing, input
advancement, forced skips and full tutorial play remain open.
