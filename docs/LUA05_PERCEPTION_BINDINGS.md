# LUA-05 perceived-team bridge contract

Base: merged shared prerequisite #327 (`fbaa8d6`). No simulation, content,
client, save, network or dependency changes are part of this adapter.

## Before implementation

- `A2.SetPerceivedTeam(id, team)` takes exactly two unsigned integral arguments.
  Decode the configured Slots/ArmadaTeams numbering exactly as SetRealTeam,
  accepting real slots or neutral only. Require a current non-DYING handle.
  Queue the merged SCRIPT `Edit::PerceivedTeam` through correlated edit feedback.
  Configured-seat and target validation at application time remain Core's job.
- `A2.GetPerceivedTeam(id)` takes exactly one unsigned integral handle. Require
  a current non-DYING handle and encode the shared view's perceived owner in
  the configured numbering. Emit no command. This native method is not used by
  the current inventoried DLL corpus; do not inflate the used-call count.
- `Entity.perceivedTeam` reads as a Player from the latest shared view, or nil
  for a pending/failed/missing/DYING actor. It does not predict queued writes.
  Its setter accepts a Player or strict integral player number in the configured
  numbering, real slots/neutral only. Fractional/nonfinite numbers, strings,
  booleans, nil and reserved identities error without queuing commands.
- A pending entity retains a typed `PerceivedTeam(PlayerId)` action until its
  real spawn receipt resolves. Reuse existing actor-local FIFO dispatch and
  failed-spawn diagnostics. Resolved actors require a live view. Queue the same
  shared SCRIPT edit and correlate its actual sequence/rejection. No fake getter
  state or optimistic ownership/perception mutation is added to the Lua cache.
- SCRIPT commands retain sorted player/sequence application. Interleaved owner
  orders can run before SCRIPT edits; a refreshed host tick is necessary for
  an order that must observe the edit. Dispatch order is not a cross-player
  execution guarantee.

The pinned A2 setter/getter use a distinct global GameObject perceived-team
field, independently of real ownership. Construction proxies share that base;
the setter is not a finished-Craft-only operation. The bridge maps represented
sim entities to handles. Strict validation/errors, numbering translation,
pending receipts and missing-handle policy are explicit bridge conventions,
not native ABI/sentinel/invalid-input or all-class runtime parity claims.

Shared perception currently affects automatic weapon acquisition. Explicit
Attack and real-owner control/sensors/economy/victory/team colours keep their
existing meanings. Native special-weapon and additional AI/client perception
policy remains separate work; this adapter does not claim complete deception
behavior or full campaign acceptance.

## Scope and required checks

Narrow owned paths: mission api/a2 controls and new perception tests; Entity,
EntityAction and spawn dispatch; exact inventory mappings/tests/report and owning
documentation. Existing edit feedback needs no alternate store or sequence
scheme. No shared-state implementation is duplicated.

Test strict arity/types/ranges and both numbering modes; current-view reads and
refresh; real owner retained; pending FIFO dispatch through real receipts;
failed spawn and stale/missing/DYING actors; application-time rejection feedback;
construction actors; save/replay effects through merged Core; exact inventory
signature discrimination. Re-run the external 36-port installed-map 1800-tick
probe and check Blockade's handled perception warnings specifically. Preserve
all unrelated diagnostics. Run shared tests, strict Clippy, workspace and ARM64
checks. Logged requests, startup horizons and synthetic first-goal evidence are
not complete campaigns, playback or device acceptance.

Regenerate the inventory after real registration/tests. LUA-05 stays open until
the original zero-missing and mission acceptance gates are met.
