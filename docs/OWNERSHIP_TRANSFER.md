# Deterministic ownership transfer (SIM-19)

## Status and trigger

The source-audit and implementation proposal was recorded before core edits
in merged #257. The cargo prerequisite is merged in #258. The guarded
ownership stage below is implemented for finished units without production
involvement, extended by checked cleanup for entirely unstarted station queues
in the final section; full construction/capture support remains incomplete.
At audit time, `Edit::Team` existed but `Sim::apply_edit` rejected it. The longer installed A2
mission probe records rejected transfers in Invasion at tick 1440 and The
Cavalry at tick 720. `Entity.player = ...` already queues that edit; adding an
A2.SetRealTeam wrapper alone would conceal the missing simulation operation.
LUA-05 remains incomplete; its existence-query milestone is in PR #254.

Implement one simulation operation used by desktop, Android, missions, bench,
snapshots and replays. Keep the December 2026 desktop alpha priority. This
package changes neither platform input nor the legacy data files. It does not
implement perceived-team deception, mutable race, research-pod attachment,
assimilation weapons or the entire native capture system by implication.

## Source evidence

The owned, pinned A2 reference shows that SetRealTeam dispatches a virtual
ownership swap on a live object. The ordinary swap clears old-team state,
sets the new team through class-specific methods, then issues the same stop
command used by the native Stop wrapper. This also happens for a same-team
request; it is not merely an assignment guarded by old != new.

Observed class-specific work includes AI fleet/formation removal, old/new
team and technology accounting, producer build-queue cleanup, construction
rig cleanup of an in-progress child, and shipyard launch/rally cleanup.
Research stations also propagate a swap to attached research objects.
SetPerceivedTeam is a distinct operation. These findings establish work that
must be considered; they do not establish every refund, timing or side effect.
No native addresses, layouts, assembly or game assets belong in this document.

The original GOG A1 executable is the primary A1 reference. Its named
SetRealTeam and GetDilithiumOnFreighter exports are available for the remaining
cross-profile checks; A2 findings must not silently become A1 or FO defaults.

The owned A2FOExtensions A2FOTeamChangeWeapons module observes completed native
swaps. Its opt-in weapon ODF activateOnTeamChange defaults to zero and included
child zero overrides remain valid. Requests run on the next weapon pass for
the final owner, retain native weapon restrictions and existing targets, and
exclude same-team/alliance-only/race-only changes. It is useful evidence for
future hooks, not an implemented STA64 weapon feature or full live-game proof.

## Current shared-state dependencies

| State | Current implementation and transfer concern |
|---|---|
| Identity | world::Entity.owner is persisted and digested. Definition, original race, ID, label, position, hull and shields must not be replaced just to change owner. |
| Orders | Movement courses, attack orders and mining jobs are per entity. Stop clears several activities, but its cleanup is not a complete capture operation. |
| Cargo | The audit baseline combined job and physical cargo, so cancel destroyed both. The cargo prerequisite now retains loaded records in Idle; this is necessary before using cancellation during transfer. |
| Production | Queue/deploy items retain owner and paid state. cancel refunds paid costs to the recorded owner; whether capture should do so must be verified independently. Weld records connect unfinished units to builders. |
| Rally | Production rallies retain their issuing owner. Existing pruning drops a rally after owner mismatch, but a transfer needs deterministic cleanup even across multiple swaps in one tick. |
| Weapons/specials | Ordered targets are distinct from mount cooldowns, energy and active cloak state. Resetting all subsystem state would refill/reset physical state or alter authored effects. |
| Visibility/technology | Sensors and tech holdings derive from current ownership each tick. Player alliance assignments are separate and must not be changed by a unit transfer. |
| Victory | Elimination is permanent under the current contract and currently receives damage losses only. Capture-loss timing and scripted mission victory rules need an explicit decision. |
| Persistence | The audit baseline used snapshot version 8; the cargo stage writes version 9 with a narrow compatible-v8 read path. It stores entity owner and subsystem records. Commands already have a Team tag; existing tags must stay stable. |

## Proposed integration

1. Add a small ownership module and route the existing Team edit to it under
   the existing SCRIPT/BENCH authority check. Validate all inputs before any
   cleanup. Keep resource entities neutral and reject reserved command-author
   IDs as destinations. Document how valid seats and NEUTRAL are recognized,
   including the seat-less synthetic Sim used by existing tests.
2. Treat the operation as an ordered transaction: detach owner-bound activity,
   update owner, then expose the resulting state/events. Preserve physical
   state and original definition; do not reset health, shields, ammunition,
   special energy or cooldowns as a shortcut. A same-owner request still has
   the native-backed activity-reset semantics; a future ownership-change hook
   must distinguish that from a real owner change.
3. Separate suspended mining activity from carried cargo using the existing
   miner record where possible. An idle phase can preserve the cargo/resource
   while issuing no movement or income. A new order to a different resource
   must deliver the old cargo before changing its resource classification.
   Never silently convert or discard the carried resource.
4. Add production-specific transfer cleanup, not a call to the ordinary full
   refund path. Account for paid/unpaid queue heads and tails, pending deploys,
   active welds and already launched units. Resolve cancellation/refund and
   unfinished-child ownership from the appropriate reference before coding
   that part. Clear transferred builders' rallies without removing another
   builder's valid destination merely because it points at the captured unit.
5. Recompute derived visibility and technology through existing systems.
   Define end-of-batch ownership-loss/victory timing explicitly. If an owner
   event is needed, append its wire tag; do not renumber existing events or
   invent spawn/destruction events for a unit whose identity survives.
6. Keep serialization explicit. An idle phase fits the existing miner tuple,
   but an older decoder currently treats unknown phase codes as ToNode. New
   state must therefore be version-gated; a compatible v8 read path is possible
   only if its exact body and old phase meanings remain intact. Review snapshot
   version and SEAM_REV implications before adding any persisted fields. Do
   not claim cross-build replay compatibility from same-build replay tests.

## Gates to resolve before each implementation stage

- Verify physical cargo retention through native stop/swap on the original
  A1/A2 paths; do not infer it merely from the name of a getter.
- Trace producer construction removal separately from user cancellation.
  Establish who keeps/refunds paid costs and what happens to a rig's active
  child and a captured unfinished unit. Distinguish stock A2 and FO behavior.
- Set the legal-destination, same-owner, defeated-seat and victory-loss policy
  explicitly. Do not silently create a player, alter an alliance or revive an
  eliminated seat as a side effect of assigning one unit.
- Preserve the current explicit-attack policy unless a separate native-backed
  targeting change is justified. Autotargeting must use the new ownership;
  capture must not grant the old owner continued control of the actor.
- Keep unmodeled research attachments and ownership-sensitive effects visible
  as limitations. Do not fabricate their behavior through owner-field changes.
- Any newly verified default belongs in its owning ODF/CFG/AIP/profile path,
  with explicit/inherited zero distinguished from absence. Do not add guessed
  global defaults or change source game data.

## Cargo prerequisite decision

The first implementation stage preserves carried cargo when a mining job is
interrupted by Stop or a replacement Move. Original A1 and A2 getter paths
read cargo from the freighter object rather than the order/process; the GOG A1
swap and Stop wrappers use a distinct command encoding and must not borrow
A2 numeric codes. These static observations support separating physical cargo
from job lifetime; they are not full native runtime or effect-parity proof.
No source game data or numeric defaults change in this stage.

Use Idle phase code 4 in the existing miner tuple. Idle retains the cargo and
its resource, produces no income and does not override a manual movement
order. A stopped loaded miner can survive destruction of its old node. A new
Mine order first delivers the old resource to a station of its current owner,
then mines the newly requested resource. Reset partial-delivery event
accounting when a job is interrupted, without undoing stock already credited.

Write snapshot format 9 for the new phase semantics. Explicitly accept the
unchanged v8 body only with old phase codes 0..3; reject an Idle/new phase
masquerading as v8. Reject unknown phase codes at the file-loading boundary.
No tuple field or command tag is added. A synthetic loaded-miner snapshot was
created by the unchanged v8 implementation before these edits; its saved and
continued digests/stock are recorded with the fixture for a real compatibility
check. This is a narrow compatible-v8 read path, not general save migration
or cross-build replay compatibility.

The cargo prerequisite itself does not implement Team, choose capture refunds or
resolve construction/victory behavior. Those stages retain the gates above.

## Verification and staged delivery

Use narrow commits for the cargo/persistence prerequisite, production cleanup,
central edit/event integration, and Lua adapter after the seam has merged.
Tests must exercise actual Sim steps and subsequent activity, not only owner
assignment or queued command shapes:

- SCRIPT/BENCH versus normal-player authority; bad/stale/dying/resource targets;
  legal and invalid destinations; same-owner and ordered repeated swaps.
- New-owner control and rejection of old-owner commands on subsequent ticks;
  movement/attack/mining cleanup without lost physical cargo or reset cooldowns.
- Old cargo unloading to the correct current owner, including a newly selected
  different resource, stopped loaded miners and missing old resource nodes.
- Paid/unpaid builds, active rig deploy/weld, capture during construction,
  original-payer accounting, no orphaned child or old-owner queue completion,
  and rally cleanup across swaps.
- Owner-based visibility, tech holdings and the decided victory behavior,
  including multiple transfers within a command batch and neutral ownership.
- Same-build replay event/digest equality, snapshots continuing mid-activity,
  explicit old-version acceptance/rejection, and stable old wire tags.
- The existing installed Invasion and Cavalry cases no longer reject Team
  while their other diagnostics remain visible. Blockade.forceOntoMap is a
  separate missing capability and must not be hidden by this work.

Run focused tests, shared regressions, strict Clippy and the required workspace
check with the repository's low-priority/two-job policy. Request the full OS
matrix for deterministic core changes. Native runtime, client visuals and
campaign completion remain separate acceptance evidence.

## Guarded ownership stage decision

After the cargo prerequisite, introduce the existing Team edit for live,
finished craft and stations. This is an incremental shared capability, not
completion of the producer/capture gates above. Before mutating anything,
reject a missing/dying/zero-hull/non-unit target, resources identified by
kind or definition metadata, any unfinished target, and
any unit with a production queue, pending deploy or weld involvement (as
builder or child). Even a same-owner request uses these guards. Rejection
must leave orders, cargo, rally, resources and ownership unchanged.

Accept NEUTRAL or a real player destination. In a configured game, a real
player must be an existing seat and must not be eliminated. A seat-less
synthetic Sim may use real IDs 0..7 without creating seats or changing the
seat list. SCRIPT, BENCH and other reserved IDs are never owners accepted by
this edit. Existing SCRIPT/BENCH edit authority remains the only entry.

For an accepted request, stop the actor's movement and queued course/formation, clear
its explicit attack order, suspend mining while retaining physical cargo,
and remove only its own rally. A different builder's rally pointing at this
still-live entity remains valid. Clear activity even for a same-owner request
and for repeated swaps within one ordered batch. Then assign the destination
without replacing identity, definition, race, label, position, hull/shields,
energy, cloak, mounts or cooldowns. A later Mine order uses the resulting
owner's station. New/old-owner command authorization, visibility and tech
holdings use the shared existing systems.

Keep the existing damage-only elimination rule in this stage. Losing the last
asset through a Team edit does not eliminate or revive a seat; capture-loss
victory policy remains a separate gate. No ownership event/hook, assimilation
or mutable-race feature is implied. Existing shots already in flight and
other actors' explicit attack orders retain their current shared-simulation
behavior; this is not measured native projectile/effect parity. The edit
must not clear projectiles or refill weapons as an ownership shortcut.

This stage adds no saved fields, command/event tags, dependencies or defaults.
Exercise actual steps, old/new-owner control, same-owner/repeated swaps,
neutral/configured/seat-less destinations, atomic construction rejection,
physical-state retention, idle cargo delivery, derived tech/visibility and
snapshot/replay continuation. Rerun the installed mission probes without
hiding any remaining rejected commands. Keep SIM-19 and LUA-05 incomplete
until their remaining gates are met.

## Guarded-stage installed probe evidence

The optional 1800-tick startup sweep on guarded-stage source e9f55fe reaches
all 36 external ports and resolves all 36 installed maps without packaging
failures. It still fails overall with two diagnostics: Blockade's existing
forceOntoMap Lua error and Invasion's Team request at tick 1440. The Cavalry's
previous tick-720 Team rejection is absent. This is startup/command evidence,
not campaign completion, native runtime acceptance or working AIP behavior.

The rejected Invasion target is the Regula III planet. The after-step
diagnostic identifies pb_clssh with class planet, Resource kind, neutral
owner, full positive hull, no construction progress and resource metadata 0.
The current model represents it as a neutral deposit. Do not bypass the
resource guards for this map or ODF. Distinguishing colonizable worlds and
colony ownership from neutral deposits needs its own native-backed model;
this stage does not implement colony capture. The probe keeps both remaining
failures and its unsupported/handled-call diagnostics visible.

## Per-producer payment configuration prerequisite decision

The owned Fleet Ops source identifies queueInstantDeduct as the ProducerClass
ODF Boolean. Its default constructor writes zero; the ODF copy constructor
reads the key with the source class byte as fallback. The relevant constructor
windows and embedded key are byte-identical in the owned legacy and clean
FleetOpsHook references, and the clean window was also inspected directly.
The owned Boolean getter distinguishes absence from explicit false/zero and
normalizes an authored integer nonzero to true. These are static/source
findings, not live hook or gameplay acceptance. Do not copy native addresses
or treat this FO extension as a stock A1/A2 default.

Reuse the existing per-unit Features cache and its conditional content hash.
No new BuildDef, command or snapshot field is needed. On unit ODFs in Fo4 and
Native profiles only, map a valid queueInstantDeduct to pay_on_queue before
the existing Feature_<name> overrides. Precedence is profile defaults, mod
[Features] defaults, resolved native ODF key, then resolved explicit STA64
Feature_pay_on_queue. Existing ODF inheritance supplies parent values; a
child's zero overrides the same inherited native key. An inherited explicit
STA64 Feature override still has the higher documented precedence; its child
can override it through that same Feature key. The initial key reader accepts true/false and signed 32-bit decimal integers,
including noncanonical nonzero values such as 2. Other lexical forms warn
and retain the existing fallback; native invalid/overflow parsing is not
claimed as reproduced. Weapons and stock A1/A2 profiles do not
interpret the FO key. Raw ODF data remains available unchanged.

Production must consult the builder's resolved unit_features, rather than
only the database-wide switch. This also fixes the existing Feature_pay_on_queue
unit override being parsed but ignored. Do not use the product's flag, add
refund behavior, or relax ownership/construction guards. Existing paid and
started records already carry payment state. Absent keys keep the existing
profile/mod behavior and fake-content hash; explicit differences use the
existing feature hash marker and therefore the content guard on saves.

Test native-key inheritance and explicit zero, both namespace precedence
levels, invalid-value diagnostics, Fo4/Native versus stock profile gating,
weapon exclusion, absent-key hash stability, two builders with opposite
payment modes, pending rig deploys, failed-affordability behavior and the
real v8 fixture's continued digest. Run shared/consumer/workspace/Clippy
checks. Capture refund policy, colonizable worlds and victory remain gated.

## Unstarted station-queue transfer stage

This next stage extends the guarded operation only for a finished station
whose entire queue has not started. Active heads (including zero elapsed
ticks), pending rig deploys, weld builders and unfinished children still
reject. Construction identity, attached research, capture victory and active
refund policy remain unresolved; SIM-19 is not complete.

Pinned original A1 and A2 producer ownership cleanup calls queue clearing
outside the optional busy/construction-effect branch. Their inspected queue
clear bodies unlink nodes without direct stock credits. The owned FO host
has the same outer ordering. The matched clean/legacy FO callback's precharged
branch queries current class/team costs and credits every queued class before
structural clearing; its non-precharged branch has no local stock credit.
This supports clearing unstarted unpaid jobs without credit and returning
modeled prepaid costs. These are static local observations, not verified hook
installation, resource-provider contracts or native runtime parity. The owned
A2FO extension's added-resource events are separate from original stock.

Use the existing per-item paid flag and owner, not the producer's current
payment feature, to determine liability. Content is immutable during the run;
credit each positive represented cost to the original owner, never the new
owner. This is STA64's current flat-cost accounting: native dynamic discounts,
crew/officer ledgers, external resource hooks and maximum-stock clamps are not
represented and are not implemented by this stage. No new numeric default or
legacy data rewrite is needed. Existing profile/ODF/feature payment precedence
and inherited zero remain unchanged.

Before changing any activity or balance, prepare an ephemeral transaction:

- Retain all existing authority, destination, life, kind and resource guards.
- Reject any deploy/weld involvement. A queue may belong only to a Station;
  every entry must have the actor's current owner, zero done ticks, started
  false and an existing definition. Reject malformed records atomically.
- For paid entries, accumulate positive costs by the four currently modeled
  resource slots. Reject any positive cost beyond those slots. Ignore zero
  and negative costs, as existing charging does. Check each signed 32.32
  addition before committing; rejection must preserve balances and queue.
  Existing negative balances are allowed if credits remain representable.
- Positive paid credits require a real old owner (0..7). Native neutral-team
  precharged clearing is a distinct hazardous branch; reject such credits
  rather than invent a neutral refund or confuse neutral with Core player0.
  Unpaid or zero-cost neutral queues can clear without monetary edits.
- No queue retains the old production-free transfer behavior, including a
  content-less synthetic Sim. No new persisted plan, command or event exists.

After all validation, consume the plan once: remove the station's queue,
apply the prepared final balances, then perform the existing actor cleanup
and ownership assignment. No production tick intervenes. Same-owner requests
also clear and credit once; later requests find no queue. Ordinary Stop/cancel
keeps its existing separate policy. Clearing a queue emits no fabricated
build, destruction or income event. The captured station can subsequently
accept new work under the new owner and existing command authorization.

Check prepaid multiple jobs cleared in the admission/transfer command batch,
unfunded deferred queues over later ticks, no completion or new-owner credit,
same-owner/repeated edits, old/new control, all modeled currencies, malformed
owner/definition/progress/resource and overflow rejection, active/deploy/weld
guards, and snapshot/replay continuation. Rejection comparisons allow the
ordinary rest of the tick by using an identical baseline simulation. Shared
regressions and full OS CI remain required. Android uses this same operation;
an APK build is distinct from Waydroid/device gameplay acceptance.

## Active station-queue transfer: pre-implementation contract

This stage extends the existing checked plan to a paid, started station head,
including zero elapsed ticks. It does not complete SIM-19. The exact owned
paths are `production/transfer.rs`, narrow explanatory comments in
`ownership.rs`, the obsolete active-head rejection case and a child-test
registration in `ownership_queue_tests.rs`, a new
`ownership_active_tests.rs`, the legacy combined queue/deploy/weld test in
`ownership_tests.rs`, and this document. That legacy test now needs to check
active queue acceptance while retaining all deploy/weld rejection assertions;
the wider ownership run exposed its obsolete blanket queue guard. No production tick, command,
snapshot, replay, content/default, client or Lua format/API changes are needed.
Open #309 owns separate boundary/production integration; its files are not
modified by this stage. This contract must be pushed before source changes.

### Native accounting decision

Fresh static checks use the pinned original GOG A1 executable, original A2
executable and its own matching map, and separately corroborated Fleet Ops
host/DLL paths. Private captures retain hashes and exact ranges. A1's export,
virtual ownership slots and class layout were traced independently; A2
offsets and the historical FO-like map are not used as A1/A2 substitutes.

The examined ordinary station start paths charge the whole cost before
marking the build active. They do not charge gradually as its timer advances.
The outer ownership operation clears old-team state before setting the new
team and issuing Stop. Its producer cleanup removes the construction effect,
clears the queue and resets active state. This differs from ordinary Cancel,
whose explicit monetary credits are not called by that producer clear path.
The ordinary deferred active cost is therefore forfeited on this ownership
clear; the later Stop cannot refund an already removed job. A started head
with zero elapsed time has already crossed the payment boundary.

The reviewed Fleet Ops precharged-clear callback instead credits every linked
queue item to the old team before structural removal. The active head remains
in that queue and is included. Its deferred branch decrements an active
technology count and performs structural clearing, without that monetary
credit loop. Construction-effect removal and officer release are separate
from these currencies. No runtime hook admission or full original-game parity
is inferred from static paths.

A2FOExtensions supplies another distinct layer: its producer-event subscriber
credits four additional currencies on cancellation/deletion/clear events.
Those are Tritanium, Supply, Credits and Collective Connections, not the four
stock resource slots modeled by the current Economy. Do not reinterpret its
event-description comments as stock monetary credits. This stage neither
implements that additional ledger nor silently drops a positive unsupported
cost. Future resource integration must preserve these distinct contracts.

The original station start path uses a construction effect; product creation
happens at finish. The Core likewise has no child actor for an active station
queue item. Shipyard launch/rally and attached-research handling remain
separate native responsibilities; this stage handles only state represented
by the shared queue and does not claim attached-pod identity or full research
parity. Evolver/craft construction, rig deployments, weld builders/children,
unfinished targets, planet/resource ownership and capture-victory policy keep
their existing guards and unresolved gates.

### Checked Core mapping

Keep the existing all-unstarted behavior intact, including its paid-receipt
test under a deferred builder mode. For an active queue, only index zero may
be started; it must also be paid and have `done < build.ticks.max(1)`. Zero
elapsed is valid. Every unstarted item must still have done zero. Every item
must resolve its definition and retain the station's old owner. For an active
head, also require a valid station builder definition; never obtain charging
mode from a missing definition's fallback.

Use the builder's existing immutable `unit_features(...).pay_on_queue`,
already resolved from its profile/ODF overrides and included in the content
hash. A paid active head refunds its positive represented costs only in
precharged mode. In deferred mode those costs remain spent. All unstarted
paid entries retain the existing recorded-payer refund contract; unpaid
entries never earn credit. No new default or option is introduced, and the
existing authored payment-mode overrides and explicit/inherited zero remain
authoritative. Dynamic discounts, mutable native prices and paid-price
receipts remain outside the modeled flat-cost policy.

Check positive paid costs for unsupported slots and neutral/reserved owners
even when the active cost will be forfeited; accepting a dubious neutral
payment must not expose Core player-zero aliasing. Sum only actual refunds,
with checked Fixed-bit arithmetic. A deferred head does not cause refund
overflow when no addition is performed. Any malformed phase, payer,
definition, unsupported positive paid cost or refund overflow rejects before
any cleanup, ownership or resource mutation. Keep the plan single-use and
consume it in the existing same command application, before any tick can
advance. Ordinary Stop's full paid-item refund policy is unchanged.

Successful cleanup removes the entire queue and preserves the existing actor
cleanup/control rules. No child appears, no completion is fabricated and no
credit goes to the destination. Same-owner and repeated requests use these
same rules. Already completed products remain separate actors with their own
owners. The existing Item record already serializes definition, payer,
elapsed ticks, started and paid; add no fields, format tags or version bump.
Snapshot and replay checks prove same-build continuation, not cross-engine
replay equivalence for commands that an earlier implementation rejected.

### Required checks

Exercise both charging modes at zero and partial elapsed time, active head
with tails, all four currencies, same-owner/repeated requests, old/new-owner
control, transfer immediately before completion, an already completed child,
and an ordinary Stop control showing its different refund. Test unpaid active
heads, started tails, out-of-range progress, missing definitions/builders,
payer mismatch, neutral positive/zero cost, unsupported slots, refund overflow
and deferred non-refund overflow distinctions. Rejections must be atomic
relative to the normal rest of the tick. Preserve existing rig/weld/resource
guards and unstarted tests. Run active snapshot/replay continuation, shared
consumer tests, strict Clippy/workspace checks, full desktop OS CI and Android
ARM64 checks. An APK artifact remains separate from Waydroid/device acceptance.
