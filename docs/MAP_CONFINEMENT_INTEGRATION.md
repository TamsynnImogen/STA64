# Map confinement integration contract (SIM-22, proposed)

This is the next-stage design after the pure geometry prerequisite in #289,
now merged in main `3896abd`. It is not an implementation or a claim that
Blockade works. Keep the parent #287 open, and deliver the complete shared
simulation seam before a separate LUA-05 adapter. Desktop and Android consume
the same rules, state, commands and deterministic geometry. The December 2026
desktop alpha remains the immediate priority.

## Native scope established by the audit

Private pinned GOG A1, stock A2 and owned FO host evidence is retained outside
the repository. No native executable is run or modified. This contract uses
static bodies, named A2 symbols, independent same-image runtime RTTI and direct
virtual-method targets; it does not establish arbitrary hook admission or
complete live-game parity.

The A2 audit pairs 32 canonical runtime constructors with their actual primary
tables and class identities. Independent A1 and FO runtime tables cover 25 and
32 canonical actor types respectively. Null-factory upgrade/planet-shield types
and unresolved class aliases are not inferred to be ordinary actors.

The ordinary GameObject origin/update methods consume the confinement flag.
Craft families use those methods, including stationary stations and A2's
Craft-derived `planet`. Terrain families override origin/update and bypass that
geometry. Nebula update either returns or forwards to the terrain path. The
terrain constructor also supplies a different initial flag in A1. Native
background exemption is additional evidence, not permission to infer all of
this from Core's Craft/Station/Resource/Prop categories.

Use a content-side classification with these represented behaviours:

| Behaviour | Position edits and map-load position update | Ordinary tick position update |
|---|---|---|
| Legacy Craft family | Ordinary confinement geometry | Yes, including stationary finished actors |
| Legacy plain GameObject family, including construction placeholders | Ordinary confinement geometry | Only when its position actually updates; no invented Craft tick |
| Legacy Terrain family, including background and nebula | Preserve the flag but bypass ordinary confinement geometry | No ordinary confinement geometry |
| Native STA64 actor with authored capability enabled | Shared engine geometry when its flag is enabled | Yes |
| Unresolved legacy class/factory | Diagnose unsupported policy; do not guess a native dispatch path | No newly invented enforcement |

Class identity must come from the resolved definition, not model names, unit
names, player ownership or current Core actor kind. In particular, `planet`
and `planet_obj` are different families. Use these bounded canonical tables,
with case normalization supplied by ClassLabel:

- A1 Craft: `constructionrig`, `craft`, `freighter`, `hover`, `mining`, `pod`,
  `producer`, `research`, `scavenger`, `sensor`, `shipyard`, `starbase`, `wingman`.
- A2/FO Craft: `cargo_ship`, `colony`, `constructionrig`, `craft`, `evolver`,
  `freighter`, `minearray`, `mining`, `p_mining`, `planet`, `pod`, `producer`,
  `repairship`, `research`, `scavenger`, `sensor`, `shipyard`, `starbase`,
  `trading_station`.
- A1/A2/FO plain GameObject: `constplaceholder`.
- A1 Terrain: `areaeffect_obj`, `asteroidfield`, `background_obj`,
  `blackhole_obj`, `i76building`, `ionstorm_obj`, `nebula_obj`, `planet_obj`,
  `scrap`, `u_ball_obj`, `wormhole_obj`.
- A2/FO Terrain: the preceding terrain list plus `latinum`.

Native authored capability uses the represented STA64 actor's shared geometry;
it does not pretend that an arbitrary Native class has a legacy runtime type.
Do not silently classify `buildyard`, `fighter`, `moon`, `asteroid`, upgrade
factories or other unresolved legacy names from Core kind fallbacks.

## Initialization and lifecycle

A1's factory initializes geometry and calls virtual position update before
object Load. Load is followed by transform/position updates. Its `missionSave`
header mode skips reading the saved confinement flag for fresh maps. The
ordinary A1 update arms a false flag when the padded X/Z outside report is
false, independently of Y. Consequently the traced ordinary A1 fresh-map
actors use inclusive padded-X/Z entry, not A2's strict raw-X/Z interior test.

A2 and FO initialize/recompute the flag from strict raw X/Z interior, without
the 100-unit inset and independently of Y. Their ordinary position-update
methods lack A1's false-to-true rearm branch. Nonzero saved-state reader mode
preserves the read flag; the typed header mode and same-reader forwarding are
traced separately from the object body.

Proposed effective initial rules:

| Content profile/family | Capability default | Initial rule | Automatic rearm |
|---|---|---|---|
| Native | False; opt in through the owning ODF | Disabled; existing Native worlds stay unconfined | False |
| A1 ordinary GameObject/Craft | True | Inclusive padded X/Z interior | True on ordinary position update |
| A1 Terrain | True | Enabled constructor flag; spatial override still bypasses geometry | No invented ordinary update |
| A2/FO known GameObject/Craft/Terrain | True | Strict raw X/Z interior | False |

Capability is immutable resolved content policy, separate from the mutable
per-entity flag. When capability is disabled, all ForceOntoMap edits reject,
including false: a runtime edit cannot opt an old Native world into new rules
whose bounds were never hashed. Enabling capability does not itself imply an
enabled initial flag. Thus Native authors can opt in without automatic
confinement, and existing Native fixtures retain the old future behaviour.

For ordinary methods, A1's factory can arm and then constrain on a subsequent
initialization callback. A2/FO runtime creation initializes the flag after
their earlier geometry callback; ordinary map loading subsequently performs a
transform/position update. Keep runtime birth and map-load completion distinct
where these timings affect position. Do not clamp terrain scenery at import.

The setter writes only the flag. It does not immediately teleport, stop or
clear a unit, alter velocity, change ownership, or reset orders/cargo/weapons.
A1 may rearm later during its ordinary position update. Teleport/origin edits
must apply the appropriate real position-update path, including that rearm.

Initialize state for every actual birth, including production paths that spawn
through `Entities` rather than `Sim::spawn_def`. Destruction removes its row;
ownership changes preserve it. Core construction retains an ID while native
construction replaces a placeholder with a new actor: the finish transition
needs an explicit boundary-state decision, not accidental flag inheritance.
The named A2 rig finish deletes its placeholder before the producer constructs
the product. Close the applicable factory/finish handling before coding this
transition; do not broaden ownership-transfer policy through this work.

## Authored rules and content hash

Use one small typed content rule/override group on UnitDef, resolved through
ContentDb from the profile, structural class behaviour and the already resolved
owning unit ODF. Numeric rules are not boolean Features. Avoid extending the
legacy Features bit array merely to carry these settings: appending even false
bits changes existing Native hashes. No physics, race/loadout or resource
schema rewrite is needed.

Proposed STA64-owned unit ODF keys, explicitly not claimed legacy keys:

| Key | Meaning and accepted values |
|---|---|
| `STA64_mapConfinement` | Boolean capability; explicit/inherited `0` disables this rule and rejects its runtime controls |
| `STA64_mapConfinementInitial` | `0`/`disabled`, `raw_xz`, `padded_xz`, `enabled` |
| `STA64_mapConfinementRearm` | Boolean; explicit/inherited `0` disables rearm |
| `STA64_mapConfinementInset` | Nonnegative finite X/Z inset; reference default 100 world units |
| `STA64_mapConfinementUpperEpsilon` | Nonnegative finite upper projection inset; reference default is the native binary32 0.1 |

Resolved ODF inheritance and child-over-parent mounting already establish
precedence. Missing values use the documented profile/family defaults; zero
must remain present, never become a missing-value fallback. Invalid values
produce a file/key warning and the documented default, rather than wrapping,
clamping silently or accepting NaN/infinity. Read numeric inputs through the
existing content conversion boundary; runtime geometry remains Fixed only.
The reference epsilon's exact Fixed raw value is `429496736`, not the result
of converting decimal 0.1 directly to Fixed.

Hash enabled capability and its effective rules with a new explicit marker
and fixed field order. Include every effective override that can affect a
later explicit command, even if initial flag initialization is disabled.
With capability disabled, no runtime control or automatic update can consume
the other settings, so they do not create a new effective rule. Diagnose
explicit active initialization settings under disabled capability rather than
silently presenting them as enabled. Class and profile participate in content
hashing; structural dispatch must not be guessed from mutable simulation kind.
Keep the genuine old Native fixture's content hash and no-op digest unchanged.
Legacy automatic confinement intentionally changes effective rules and hashes;
do not promise arbitrary old legacy saves/replays remain compatible.

## Shared state and commands

Keep UnitFlags' existing u8 layout. Use a deterministic sidecar keyed by stable
entity ID. Preserve initialized false as well as true for every represented
actor with enabled capability, so entering the map does not re-run A2/FO
initialization. Existing Native actors with disabled capability have no row
and cannot be explicitly controlled. Do not derive current flag values from
current position or definition defaults after initialization.

Append `Edit::ForceOntoMap { id, enabled }` after Hull without renumbering old
postcard tags. Retain script/bench privilege, live-target and definition guards;
require enabled capability, represented policy and usable bounds/rules before mutation. An invalid
request must be atomic, including false and same-value requests. A valid setter
does not project position immediately. Expose a real Core state inspection seam
for tests; a later Lua setter must submit this same operation. Lua read access
requires actual published state or an explicit unsupported diagnostic, never a
definition-derived invented value.

For invalid/tiny/overflowing map volumes, retain existing entities and report
unsupported automatic confinement at setup. Do not silently drop valid unit
spawns or introduce a new setup panic. Explicit controls reject unusable
volumes, and no fabricated valid boundary row is persisted. Core state
inspection returns unavailable for disabled capability, unsupported classes,
unusable volumes or missing required state; it does not invent false. For
enabled, represented actors with usable volumes, initialize and require a row
even when the flag is false. Snapshot validation rejects missing rows in that
usable case, allows their absence in a diagnosed unusable volume and rejects
rows that claim usable state for an invalid volume. Setup must check relevant
resolved policies, including definitions later spawned by production, so a
runtime birth does not introduce an unreported invalid-volume fallback.

## Movement and avoidance ordering

Cover persistent active targets, formation-expanded destinations, every queued
waypoint and their promotion, subsystem steering, launch/rally targets and
position edits. Admission-time projection alone misses economy and production
targets; physical clipping alone leaves unreachable destinations.

Project logical destinations before obstacle steering. Preserve the existing
temporary-target restoration semantics. Validate both detour candidates against
the effective volume: when the preferred side is outside but the other side is
usable, select the usable side instead of clipping a detour into its blocker.
After kinematics, separation and field push-out, perform applicable ordinary
position updates, with A1 rearm evaluated at that point. Constrain new targets
created later by economy/production/course advancement before the tick view.

Boundary/solid intersections require measured fixtures before selecting the
fallback. An impossible destination inside an obstacle near the edge must not
silently become a permanent camera-independent steering loop. Preserve no-rule
Native behaviour and existing obstacle preferences exactly. This work does not
claim a new general pathfinder or silently disable obstacle avoidance.

## Snapshot, digest and replay

Propose snapshot format 10 with the unchanged old Body, an explicit bounds
value and a mandatory separately versioned boundary record. Validate and fully
consume it; keep exact v8/v9 parsing rules, including their optional bounds
suffix and the v8 mining-phase restriction. Reject unknown boundary versions,
duplicate/out-of-order IDs, nonexistent/unsupported actors, unusable active
bounds, missing required initialized rows and trailing bytes. Loading must not
re-run spawn/map callbacks or recompute a saved flag from position.

Hash explicit boundary state. Whenever any definition in the attached content
database has enabled, represented capability, hash the bounds Option and all
six Fixed coordinates from setup, before any ForceOntoMap command or actor
birth. This includes empty worlds and initially false flags, so two worlds
that can react differently to the same new command already have different
digests. Preserve the old digest path only when all capability is disabled;
in that mode automatic behaviour is absent and the new controls reject.
New files must fail closed
in old decoders; changing only the format bytes to 8/9 must not hide a boundary
record. Retain the genuine v8 mining fixture and unchanged Native v9
continuation. Accept legacy v8/v9 continuation only when all effective
capability is disabled; new enabled policy requires its explicit v10 state,
even if a hand-crafted old header carries the new content hash. This is not
a general cross-build migration facility.

Replay has its own STAR format and SEAM_REV. Select and document command/seam
compatibility explicitly: an appended edit must make an old decoder reject
that new command, while old Native commands retain their meaning. A snapshot
version bump alone does not settle replay or future network compatibility.
Exercise actual recorded commands and final digests; no networking rewrite is
part of this seam.

The independent [strict replay framing prerequisite](REPLAY_FRAMING.md)
preserves STAR1/seam1 and valid bytes while rejecting unused input. The frozen
old decoder accepted both a byte suffix and a concatenated second replay;
do not assume old readers will reject an appended record. Replay serializes
the full `MapDef`/`MapObject`, so retaining a distinct source placement for
initialization also affects its positional body. That future layout needs an
explicit format/legacy-body decision; the framing fix does not authorize it.

## Source scope and acceptance gates

No source freeze or implementation is declared by this document. Proposed
paths are the boundary module/state/tests, narrow lib/spawn/movement/avoid
hooks, command append and snapshot/replay tests; typed content rules, UnitDef,
unit parsing and content hashing. Keep UI, rendering, audio, race/loadout,
multiplayer transport and Lua adapter source outside this stage.

Before source work, settle canonical family tables, exact birth/map/finish
timing, invalid-volume state validation, boundary/solid fallback and replay
compatibility. Record the final decisions and exact file freeze on #287.
Then test real movement/command batches, A1 entry/rearm versus persistent A2/FO
false, static Craft versus terrain, construction lifecycle, all axes/edges,
formations/queues, attack/mining/build/rally targets, obstacle border cases,
override/inherited-zero/invalid diagnostics, atomic rejection, hashes, snapshots,
replays and old Native continuation. Run shared consumers, strict scoped Clippy,
workspace and full platform CI; build an actual ARM64 APK separately.

Only after the integrated core seam merges should LUA-05 expose the setter and
rerun the long Blockade probe. Preserve Invasion's separate ownership failure
and all other diagnostics. Full campaigns, native runtime and Waydroid/device
acceptance remain separate checks.
