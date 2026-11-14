# Planet roles and ownership

## Implemented shared ownership subset (SIM-19 completion)

Typed `Background` and `Producer` planets retain their requested map/spawn
owner and use the shared Team transaction. A background object with exactly
zero current/maximum hull remains a live transfer target unless marked dying.
Generic resources retain the SIM-18 neutral-owner guard. Kind and collision
behaviour are unchanged; planets do not become stations or survival assets.

Mining admission is independent: Background rejects Mine in Core and the client;
Producer requires an explicit resource slot and retains the existing single-slot
mining model. This fixes the absent-resource dilithium fallback on A1 Earth; it
does not claim complete native freighter/orbital or multi-resource semantics.
Producer planets can use stationary build queues, rally and Stop through the
shared implementation. Team cancels their queues with station payment rules.

Producer `maxCrewGain` is now a typed signed 32-bit integer. A2/FO default 10000,
other profiles default zero; the planet ODF may override it, including inherited
or child zero and signed values. Malformed/out-of-range values warn and use the
profile fallback. Parsing is strict decimal, not emulation of permissive native
CRT conversion. The effective producer value participates in content identity;
unused values on other roles do not. `Sim::planet_crew_capacity` derives the
current owner's planet contribution, including after save/load, without a
mutable credit accumulator. It is not crew stock or total crew capacity.

Colonization, population, recruitment, planetary shields/atmosphere simulation,
orbital links and multiple resource quantities remain planned. Ownership does
not fabricate these states, and `isColonizable` remains raw metadata rather than
a hidden Team gate. See [OWNERSHIP_TRANSFER.md](OWNERSHIP_TRANSFER.md) for the
transaction, persistence and explicitly unsupported capabilities.

## Typed planet numerics (CON-37)

`UnitDef` carries three effective values beside the role and `maxCrewGain`
(raw keys stay reachable). Each is read only for the role that uses it; other
roles neither read nor diagnose the key. An authored value, including zero and
including one inherited through an include, always wins over the fallback.

| Field | Role | Key | Absent key | Evidence |
|---|---|---|---|---|
| `planet_crew_modifier: Option<Fixed>` | Background | `crewModifier` | Armada I 1.2; `None` on other profiles | A1: read from the executable, static. Other profiles: not measured |
| `planet_colonizable: bool` | Producer | `isColonizable` | true on Sta2Classic/Fo4, false elsewhere | A2/FO: read from the executable, static |
| `planet_civilian_population_modifier: Fixed` | Producer | `civilianPopulationModifier` | 1 on Sta2Classic/Fo4, 0 elsewhere | A2/FO: read from the executable, static |

Installed data (read from data): the keys above exist in stock A1 and A2 planet
ODFs; the per-install inventory is in the PR body.
The "None / zero / false elsewhere" fallbacks for explicit cross-profile opt-ins are
conservative STA64 choices, not native measurements. Flags use strict
spelling (`0/1/true/false/yes/no`) and numbers must be finite; malformed
values warn and use the fallback. Numbers pass through their shortest decimal
text, so `1.2` equals the literal fallback.

Hash: effective `crewModifier` (marker `0xf4`, `sta64.planet-crew-modifier.v1`)
and the Producer pair (`0xf5`, `sta64.planet-colony.v1`); the Native fake hash
is unchanged. SIM-30 reads them (next section).

## Colonization (SIM-30)

Armada II family only (A1 and the Fleet Ops line have no colony ships or
population keys, so the code is dormant there). Implemented in
`sta_sim::colony`; every rule below is **read from the executable, static**
unless marked, with the numbers read from data. Nothing was run on the bench.

Data keys: planet ODF `maxPopulation` (unpopulated, sparse, light, medium,
heavy; case-insensitive; absent is unpopulated) is the typed
`UnitDef::planet_max_population`, hashed only when not zero. `RTS_CFG.h`
`cfgPOP_SPARSE/LIGHT/MEDIUM/HEAVY` and `cfgPLANET_CIVILIAN_RATE_MODIFIER` are
`Rules::population()` (all five present, else the colony rules are inactive;
hashed). A colony ship is class `colony` whose first weapon's ordnance class is
`ColonizerPod`; `partySize` (default 100) and `orbitRange` come from that
ordnance (`WeaponDef::colonize`), the interval is the launcher's `shotDelay`.
The race's `crewAccumulationRate` is the existing crew rule.

Rules: `Order::Colonize` (tag 17) is taken for a pod ship, a live
`isColonizable` Producer planet with no colonizer, unowned or the ship's own.
The ship carries `cfgPOP_SPARSE` colonists per order (not the ODF `colonists`),
travels with no distance limit until its centre distance is at most
`orbitRange + 2 x planet radius + ship radius - 15`, then becomes the colonizer
and launches a pod at once and every `shotDelay`. A pod carries
`min(colonists left, partySize)`; population becomes `clamp(P + payload, 0, cap)`.
Done at the cap or under one colonist left: the planet becomes the ship's
owner's (capacity follows through the shared transfer) and the ship is
consumed. Stop gives back the pods launched (never below the population at the
start); another replacing order or a dying ship only clears the colonizer.
Growth of an owned planet from `cfgPOP_SPARSE` up: `(4 P / cap + 1) x
crewAccumulationRate x civilianPopulationModifier x
cfgPLANET_CIVILIAN_RATE_MODIFIER x 0.2` per second, clamped to `[0, cap]`.
Each owned planet adds `P / cfgPOP_HEAVY x rate x 4` crew per second to its
owner. An owned planet with 0 people goes neutral next tick, people kept.
Weapon damage that reaches a populated planet's hull costs `0.1 x damage`
people and leaves the hull alone (shield absorption comes first, not measured).
`Edit::PlanetPopulation` (tag 46, mission `SetPlanetCivPop`) sets the people
raw; STA64 restricts it to Producer planets.

Not implemented: the Borg colonizing beam and its enemy-planet cleansing
(`colonybm`, `cleansingWeaponIndex`; the order is rejected); moving the ship's
crew to the planet (planets have no crew pool); pod flight time and the landing
spiral (pods land at once, the ship goes at the done tick: not measured); the
AI's planet choice; the map-load rule for an owned unpopulated planet (a planet
with no row is not neutralised: not measured); `requiresTerraform` and
`colonists` (not read by the executable). Evidence grades of the open numbers
(about 45 s for ten pods, spiral time, shield interaction) are "not measured".

## Original audit and broader proposal

Historical status: native/data audit and proposed integration sequence under SIM-19,
issue #256. This document precedes any planet implementation. It neither lifts
the current guards nor completes ownership, colonies or resource parity.
Baseline: `14ce74217b90d3a588d48668ec6c337fb36ef93a`. Pending active-station
accounting in PR #315 is independent; its reviewed source is preserved.

## Why the current classification is insufficient

`EntityKind::of` groups `planet`, `planet_obj`, moons and deposits as Resource.
`Entity::from_def` consequently replaces their supplied owner with neutral.
The Team guard then refuses them, while the Mine command admits Resource
targets and defaults an absent resource to dilithium. These are separate
capabilities hidden behind one kind, rather than native planet semantics.

Installed GOG A1 Earth (`mearth`) is a concrete counterexample. It loads as
`planet_obj`, with no resource metadata and zero hull. Its `unitName` admits
it through `odf::kind_of` even though `planet_obj` is not in `UNIT_CLASSES`.
Thus omission from that list does **not** mean this installed planet is absent
from STA64. All 15 indexed A1 planet ODFs in this audit use `planet_obj` and
have no resource or colonizability key.

The original A2 mission planet `pb_clssh` is different: class `planet`, a
`ResourcePlanetH` resource definition, `isColonizable = 1` and
`requiresTerraform = 1`. Current resource-ODF resolution selects slot 2
(metal), with the resource definition carrying 12000 metal and 16000
biomatter. The historical slot-0 diagnostic in ownership research came from
an older name-substring fallback, not missing resource metadata. A single
resource slot does not represent this multi-resource definition in full.

A private, reproducible shared-engine probe loaded the actual A1/A2 content
databases (352/655 definitions), left their planet definitions untouched,
and appended only synthetic miner/dock actors. A map object with owner slot 1
became neutral in both cases; scripted Team was rejected. Across 1200 ticks,
each Mine order was accepted and delivered 300 units: dilithium from A1
Earth's absent-resource fallback, metal from the A2 planet. An earlier
120-tick observation saw no deliveries because it did not cover a full trip.
The 1200-tick result proves current STA64 behaviour with these definitions
and synthetic actors; it is not original-game execution or proof that a stock
legacy freighter should mine either target. Exact carried cargo was not
observable through the public API.

## Native distinctions established statically

The lead checked the actual original GOG A1 image, stock A2 image with its
matching original linker map, and Fleet Operations host image. Historical
FO-like symbol candidates were corroborated with the FO image's own
constructor table stores and table targets. A1 dispatch slots were checked
independently. No game or original DLL was executed.

| Native object | Established behaviour | Consequence for STA64 |
|---|---|---|
| A1 background planet | BackgroundObject/TerrainObject/GameObject constructor chain; its own table inherits ordinary GameObject team swap/set/clear. Constructor flags support live-object admission through the checked SetRealTeam wrapper. | Background planets cannot be rejected solely because they are non-colonizable or have zero hull. A universal Craft health test is unsuitable. |
| A2/FO background planet (`planet_obj`) | Distinct background class; its own table uses ordinary GameObject team callbacks. | Keep background identity separate from producer-backed planets and deposits. |
| A2/FO planet (`planet`) | Producer-backed object; ClearTeam subtracts its class crew-capacity contribution before producer cleanup; SetTeam uses craft/base handling and adds the new contribution. | An owner assignment alone would omit accounting and cleanup. |
| A2/FO colonization check | Separate eligibility predicate checks the class flag, team relation/neutral state and an existing colony-ship reference. The inspected Team dispatch/callbacks do not call that predicate. | `isColonizable = 0` disables that colonization path; it is not a general scripted-Team prohibition. |
| A2 planet persistence | Native save/load has colony-ship, civilian-population, shield and atmosphere state in addition to producer/base state. | These states must be represented explicitly before claiming complete colony or save parity. |
| A2 mining | Freighter target admission considers transfer/resource interfaces and resource-specific support; orbital mining has its own simulation path. | Do not replace these checks with a universal class-name or owner-based mining rule. Resource-bearing planets and neutral deposits need independent policies. |

This disproves treating the current neutral-resource guard as a universal
native invariant. It does not authorize Team on every Resource entity or
infer a complete mining, capture-victory, colony or orbital-station policy.

Pinned image identities, SHA-256:

| Image | Hash |
|---|---|
| GOG A1 `Armada.exe` | `1c37ee8e4a24380c950a91ce5a7f9f96c4fcf3d8ffc0cc7e58faec3835350be6` |
| Stock A2 `Armada2.exe` | `9bbe99dc58792c7c297a720560e74da193a420c91ac3f9233445320b8b4ecfd6` |
| A2 matching map | `3cf118fe348ca8e2dc3795dbdc68b1e624cab310cc3247cb5ec66b3f303a1606` |
| FO `ArmadaL.exe` | `63bd645d82aec75cdd7ee3e3271fb67bb54f4ae1d1636a736b0db620bd997e36` |

Owned A2FOExtensions A1 compatibility code provides additional discovery
leads, including non-planet victory assets. Its comments are not independent
proof of stock A1/A2 policy. Private captures, original-data identities and
probe logs remain outside the repository; no game bytes, recovered source,
layouts or native addresses are distributed here.

## Defaults and author control

These are observed native fallbacks, not newly implemented STA64 rules:

| Profile / field | Observed fallback | Owning data |
|---|---|---|
| A1 background-planet crew modifier | 1.2 | `crewModifier` in the planet ODF; keep distinct from A2 civilian-population mechanics. |
| A2/FO colonization permission | true | `isColonizable` in the planet ODF. Explicit/inherited zero must remain false. |
| A2/FO civilian population modifier | 1 | `civilianPopulationModifier` in the planet ODF. |
| A2/FO crew-capacity contribution | 10000 | `maxCrewGain` in the planet ODF. Explicit/inherited zero must remain zero. |

Other visual/rotation defaults and colony-state transitions require their
own typed contracts; do not infer them from a similarly named property in
another profile. Preserve existing include/mod precedence. Resolve values
once through shared content loading, warn on malformed authored values under
a documented parser policy, and hash every effective gameplay rule. Never
substitute a fallback merely because a valid authored value is zero.

Classification should follow verified class/profile semantics, with any
STA64-specific authored override explicitly documented in its owning ODF or
profile settings. No unit-name exception, Regula-specific path, inferred
gameplay constant from the viewer project, or hidden platform override is
acceptable. Native STA64 content must retain its existing semantics unless
its authors explicitly opt into a new role.

## Proposed integration sequence

1. **Resolve immutable roles and capabilities in shared content.** Distinguish
   background planets, producer-backed colony planets and deposits. Separate
   ownership eligibility, resource interfaces, producer support and collision
   policy; do not turn every planet into a Station or infer mining from the
   presence of the word planet. Choose the narrow typed representation in a
   dedicated seam contract before editing `UnitDef`/`ContentDb`. Identify
   defaults/overrides, errors, hashes and public callers there.
2. **Correct background-object lifecycle and resource admission together.**
   A verified background object may have zero hull while still existing. Check
   damage/destruction, map spawning, Mine, context-menu targeting and ownership
   together. Retain established collision/avoidance behaviour unless native
   evidence justifies a change: merely relabelling Resource as Prop changes
   `avoid::obstacles`, which currently treats those kinds differently.
3. **Represent colony state and team accounting.** Define deterministic
   civilian/colony/shield state, team crew capacity, producer ownership cleanup
   and orbital relationships through shared simulation. Model the observed
   native fields and transitions rather than inventing population during
   capture or treating metal as a crew pool. Plan complete resource-interface
   quantities instead of discarding the second positive resource. Native
   colony initiation and privileged Team remain distinct operations.
4. **Integrate persistence and derived state.** Decide migration only after the
   actual merged snapshot baseline is known; pending confinement PR #309
   proposes a separate format change. Do not reuse its number or serialize
   native Rust enum/layout memory. Preserve explicit tags, semantic validation,
   atomic rejection, current-build continuation and versioned network/save
   compatibility. Rebuild visibility, holdings and resource/capacity views from
   the represented ownership/state. Verify capture victory independently.
5. **Wire mission and presentation consumers.** Only after the shared seam
   merges, enable the corresponding exact mission adapter and UI actions.
   Client input, Lua and multiplayer must invoke the same shared commands on
   Windows, Linux and Android. Update manual acceptance checks when a visual
   or real mission path actually changes.

Background support can be an independently tested stage of this sequence;
it does not stand in for the producer-backed A2 planet needed by the mission.
The next implementation claim must state its exact paths and unresolved
capabilities, with the design pushed before core edits. Larger state/persistence
work remains a separate seam stage, preserving existing functional systems.

## Required regression and parity evidence

- Authentic A1 `planet_obj` admission both with and without `unitName`, class
  aliases only where supported, authored map owners, neutral targets, repeated
  and same-owner transfers, zero-hull background lifecycle and command control.
- Mine rejection for non-resource backgrounds, and separately verified
  multi-resource/owner/resource-specific mining and orbital rules. No resource
  payout fabricated from an absent definition.
- Background/colony/deposit avoidance, selection, attackability, map/editor
  representation and the preservation of ordinary moon behaviour.
- Validated crew-capacity/population/producer accounting before colony Team;
  old/new owners, neutral/reserved actors, explicit/inherited zero, unsupported
  data, overflow, partial state and checked atomic failures.
- Visibility, tech holdings and victory policy. Owned planets must not become
  victory assets merely because a Station kind was convenient.
- Deterministic content hashes and explicit versioned snapshot/replay/wire
  validation; roundtrips must preserve new state and reject malformed inputs.
  Never relabel a freshly generated fixture as historical compatibility proof.
- Genuine existing Native fixtures and the fixed fake-content hash must remain
  valid when their rules are unchanged. `fake::db()` intentionally has a
  resource-like `fx_moon` with class `planet` and no resource metadata; a global
  class reinterpretation would silently break that existing Native contract.
- Scoped shared tests, downstream consumers, strict Clippy, workspace all-target
  checks and the existing cross-platform CI. APK/runtime claims require their
  own actual Android evidence; a headless Rust probe is not a device test.

## Remaining gates

The concrete background mining/neutralization problem is reproduced. Native
background team dispatch and colony team capacity effects are statically
corroborated across the pinned images. Full resource-specific mining admission,
orbital relationships, civilian/crew dynamics, background collision/health
policy, colony initiation and capture-victory rules are not closed. No new
planet API, rule field, owner exception, save version or default is implemented
by this document. SIM-19 and the A2 mission adapter remain incomplete.

This shared-engine work adds no Android-specific gameplay or new completion
date. The December 2026 desktop alpha remains the immediate priority.
