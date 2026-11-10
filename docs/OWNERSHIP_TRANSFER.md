# Deterministic ownership transfer (SIM-19)

## Status and trigger

This is a source-audit and implementation proposal, written before core edits.
`Edit::Team` exists but `Sim::apply_edit` rejects it. The longer installed A2
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
| Cargo | Economy::Miner combines job, carried resource and cargo. cancel currently destroys the record and cargo. Cargo retention cannot be implemented by calling that helper unchanged. |
| Production | Queue/deploy items retain owner and paid state. cancel refunds paid costs to the recorded owner; whether capture should do so must be verified independently. Weld records connect unfinished units to builders. |
| Rally | Production rallies retain their issuing owner. Existing pruning drops a rally after owner mismatch, but a transfer needs deterministic cleanup even across multiple swaps in one tick. |
| Weapons/specials | Ordered targets are distinct from mount cooldowns, energy and active cloak state. Resetting all subsystem state would refill/reset physical state or alter authored effects. |
| Visibility/technology | Sensors and tech holdings derive from current ownership each tick. Player alliance assignments are separate and must not be changed by a unit transfer. |
| Victory | Elimination is permanent under the current contract and currently receives damage losses only. Capture-loss timing and scripted mission victory rules need an explicit decision. |
| Persistence | Snapshot version 8 stores entity owner and subsystem records. Commands already have a Team tag; existing tags must stay stable. |

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

This prerequisite does not yet implement Team, choose capture refunds or
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
