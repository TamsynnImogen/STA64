# LUA-05 completion audit and next bindings

Baseline main `d2c72c9`: 24 exact A2 callables, 108 missing signatures. The
original workplan's zero-missing gate remains authoritative. A partial binding
or an error-only/no-op stub does not close the parent issue.

## Pre-implementation contract

The now-merged ownership and confinement simulation operations allow these
adapters without changing Core, content, wire formats or gameplay defaults:

- `A2.SetForceOntoMap(id, boolean)`: exactly two arguments, strict unsigned
  integral handle and Boolean. Require a current non-dying object, then queue
  the existing SCRIPT ForceOntoMap edit. Actual capability, map geometry,
  zero/false requests and rearming remain Core decisions. Add no getter or
  guessed flag to WorldView.
- `Entity.forceOntoMap = boolean`: strict Boolean; use a typed deferred action
  for pending entity receipts, retaining actor-local deferred dispatch order and failed
  spawn handling. A resolved non-dying entity uses the same SCRIPT edit. This
  fixes the missing setter used by Blockade and other external ports.
  This is dispatch order, not cross-player application order: Core still sorts
  commands by player and sequence. Await a refreshed host view when an owner
  order must observe a prior script edit.
- `A2.SetRealTeam(id, team)`: exactly two arguments and unsigned integral values;
  decode the existing configured numbering mode, accepting real slots and
  neutral only. Queue the existing SCRIPT Team edit. Core retains destination,
  target, unfinished-child, production, cargo and victory guards. Do not
  mutate the cached mission view or implement a second ownership algorithm.
  Reads see the previous snapshot until the next host tick.
- `A2.ObjectivesDisplay_Set_Text_From_File(path, resetCompleted)`: exactly two
  arguments, UTF-8 string and strict Boolean. Reuse the merged atomic
  `load_with_completion` helper: true resets, false preserves only completed
  ordinal slots. Reuse its content-reader paths, parse/error/missing-file policy,
  fresh IDs and metadata. No direct filesystem access or duplicate parser.

Native stock A2 wrappers support the confinement Boolean write and real-team
dispatch on existing/non-dying objects. The pinned objective loader and own-image
FO observations support reset-versus-preserve completion. Strict Lua validation,
pending scheduling and diagnosed failures are explicit bridge policies; native
invalid-input/ABI and full runtime parity are not implied.

## Rejected command feedback

Only these new reported edits and the existing Entity.player transfer are
tracked. Each record retains its SCRIPT sequence, operation and an initially
unsubmitted state. Mark submission only when its actual command is drained to
the host. On the next strictly newer view, match SCRIPT rejection events by
sequence, emit a detailed runtime error containing sequence and edit, count a
fixed operation-specific unsupported key, and retire all settled records.
Unsubmitted, unrelated-player and unrelated-sequence events must not generate
false failures; repeated views/events must not duplicate settled diagnostics.
Use the runtime's existing 100-error cap. Do not change spawn receipts, general
command semantics, sim events, saves or replay encoding. The host must deliver
the complete events of its submitted batch with the next view, as already
required for deferred spawns.

Owned source additions are the narrow mission api/a2 modules and tests;
Entity/EntityAction/spawn integration; private edit_feedback state, api/mod and
runtime processing; exact inventory signatures/tests and regenerated report.
The existing omitted-target Mine helper must also use Core's shared
`mineable_def` admission for current and pending targets. SIM-19 separated
background bodies from mining nodes; the mission selector must not keep
choosing a background or resource-less producer which Core will reject.
This adopts the merged capability rule rather than introducing a new policy.
No Core/content/client/network/dependency changes are authorized by this slice.

## Remaining closure work

The regenerated branch report has 28 exact signatures and 104 missing
signatures, across the same 72 A2 and 59 A1 mission DLLs. The full per-signature
list remains generated in [MISSION_DLL_API.md](MISSION_DLL_API.md).

| Missing area | Signatures | Actual prerequisite |
|---|---:|---|
| Movies | 1 | Movie request backend exists; script handle, flags and completion contract still need a native-backed adapter. Playback belongs to MED. |
| Camera/cineractives | 11 | Existing host intents cover subsets; native overload/flag/path semantics and camera consumers remain. |
| Titles/text/objectives | 10 | Title/audio handles, removal flags, minimap/admiral-log state and completion reporting need contracts; recording a request is not playback. |
| Audio/music | 4 | Local typed requests exist, but native sound IDs/script handles/voice-command flags and completion feedback are not mapped. |
| Orders/AI | 24 | Stop/attack/mining subsets exist; native priority flags, path movement, colonization and dynamic AI/AIP control require shared-system work. |
| Crew | 2 | Current mutable crew does not exist. Definition crew and planet capacity are not a replacement; a SIM state/serialization contract is required. |
| Teams/relations | 3 | Shared perceived identity is merged in #327 and its Lua setter is implemented by the separate [perception adapter](LUA05_PERCEPTION_BINDINGS.md). Mutable race and native enemy-search semantics still need work. |
| Objects | 25 | Finder/view primitives exist, but native search masks/ties/sentinels need adapters; positioning, shield setters and target policy need supported shared edits. |
| Other | 24 | Alert/non-engine subsystem/decommission/derelict/rotation/transport/population/wormhole state is absent or rejected; several presentation/query adapters also remain. |

These are shared-engine or adapter prerequisites, not 104 claims that Core is
entirely absent. Small host-backed adapters can proceed after their native
argument/timing gates. Larger gameplay changes require owning simulation/content
seam proposals, tests and persistence contracts before further Lua bindings.
This tranche cannot honestly close LUA-05's unchanged zero-missing gate.

## Limits

The remaining missing signatures include orders and AI, mutable crew/shields,
unsupported actor edits, path movement and object/fleet APIs. Several require
unimplemented shared systems. Existing presentation intents can record requests,
but recording alone is not playback. Inventory counts, native semantics and
campaign progression must remain separate evidence. The full closure audit
must identify the owning prerequisite for every remaining group rather than
rename missing operations as implemented.

Verification includes strict argument failures without queued commands, real
sim confinement/ownership effects, pending spawn order, correlated rejection
feedback, host objective transactions, inventory source coverage, standard and
installed 1800-tick mission probes, Clippy/workspace and ARM64 checks. Keep all
other campaign diagnostics visible and preserve original reference inputs.

## Presentation-request slice (Cedric's session)

Partial slice of #149 over the existing `Edit::Present` / `Presentation::Call`
seam; no `sta_sim` change. Exact A2 inventory 28 used / 104 missing before,
43 / 89 after (`tools/mission_dll_inventory.py`).

- The eight camera signatures of [LUA05_CAMERA_REQUESTS.md](LUA05_CAMERA_REQUESTS.md)
  were bound and tested in `native_camera.rs` but not yet credited by the
  inventory tool; `A2_SIGNATURES` now lists them (name-level source check, as
  for every other entry).
- `api/a2/native_presentation.rs` binds seven more exact void requests with
  no handle, script pointer or completion contract: `AddTransporterEffect(int)`,
  `RemoveBillboardEffect(int)`, `GridVisible(bool)`, `RemoveFromAdLog(int)`,
  `Starfield_Load_New_Background_Geometry(char*, char*)` and both
  `MinimapMessage` overloads (`int,int,float` and `char*,int,float,int`, chosen
  by argument count). Each queues one SCRIPT `Edit::Present`
  `Call { name: "A2.<method>", args }`, validating the whole call first.
  Raw ints (signed 32-bit), floats (finite, signed 32.32), flags and UTF-8
  strings are retained, never interpreted; the entity of
  `AddTransporterEffect` must be a current non-dying object, as in the camera
  bridge. Whether native values mean a colour, id or duration is not claimed.
  No client consumer exists: this is a queued request, not playback.

### Needs a `sta_sim` seam before Lua can bind them

`Edit::Subsystem` (other than engines/weapons), `Alert`, `MaxSpeed`,
`Targetable` and `Decommissionable` exist in the seam but `Sim::apply_edit`
rejects them, so these calls cannot take effect yet:
`SetImpulseSpeed`/`ResetImpulseSpeed`/`GetImpulseSpeed` (20 scripts),
`SetCanBeDecommissioned` (28), `AllowAsTarget(id, team, bool)` (10; the edit has
no team argument), `DisableShieldGenerator`/`DisableLifeSupport`/`DisableSensors`,
`SetRedAlert`/`SetYellowAlert`/`SetGreenAlert`. Also needing sim state or
edits: `SetShieldPercent`/`SetMaxShields` (unmerged `seam/lua05-shield-edits`),
`SetCrew`/`GetCrew` (no mutable crew), `SetRace`, `SetPosition`, `HideObject`,
`SetAngularVelocity`, `Damage*`, `ExplodeNow`/`DestroyCritical`, `StuffIntoWormhole`
and wormhole/planet population calls, `BuildObject`, `SetName`, and the AI/AIP
calls. Script-pointer calls (`CineractiveStart`, `FadeIn/Out`, `AudioMessage`,
`PlayMusic`, `PlayCinematicMovie`, `WaitForEnterKey`) need the script-handle
lifecycle contract.
