# Shared perceived-team state: LUA-05 prerequisite

This is a separate seam change before Lua consumption. Real ownership remains
the authority for commands, economy, sensors, production, victory and team colour.
The stock A2 script setter writes a distinct, global perceived-team value on a
finished, non-dying Craft-family object. Native relation helpers consult the
target's perceived team from the observer's real team. This is not a per-viewer
disguise system. Native special-weapon exceptions are outside this small seam.
The private pinned-image audit is retained outside the repository.

## Contract before code

- Add an ascending entity-id table of perceived-owner overrides to `Sim`.
  Absence means current real owner. Spawn defaults to real owner. Normalize an
  equal-owner write by removing its row. Successful real-owner transfer preserves
  the previous perceived value, including the previously implicit value; rejected
  transfers change neither identity. Despawn removes the override.
- Append `Edit::PerceivedTeam(EntityId, PlayerId)` after all existing edit tags.
  Only SCRIPT/BENCH may write. Require an existing, non-DYING, finished entity;
  accept configured player seats or neutral (seatless synthetic sims accept
  slots 0..7). An eliminated seat may remain a perceived identity. This generic
  Core edit does not decide the native Craft-family adapter guard.
- Add a separate `WorldView.perceived_owners` override list and
  `WorldView::perceived_owner(id)` / `Sim::perceived_owner(id)` queries. Missing
  entities return None. Keep `UnitView.owner` authoritative and unchanged.
- Automatic weapon selection compares the shooter's real team to the target's
  perceived team. Explicit Attack remains explicit; cloak detection, sensors,
  projectile allegiance and control checks keep real teams. No client or AI
  behavior is guessed in this seam; their additional native policy audits remain.
- Hash nonempty normalized override rows under a new domain. An untouched sim
  retains the previous digest. All mutable identity affects replay/save state.
- Bump `SEAM_REV` to 2 for the changed postcard view and command surface.
  Existing replay/view-log readers explicitly reject revision 1; do not claim
  backward replay compatibility. Existing edit tags remain byte-for-byte pinned.
- Snapshot format 11 keeps the old Body and confinement suffix, then appends a
  mandatory version-1 perceived-team record. Validate sorted unique live IDs,
  valid perceived destinations and nonredundant rows. Reject unknown versions,
  truncation and trailing bytes. Read historical formats 8/9/10 only with their
  historical seam 1; synthesize no overrides. Preserve their existing mining and
  confinement compatibility restrictions, including the genuine v8 fixture.
  Format 11 requires seam 2. No content hash/default/schema/dependency changes.

## Scope and proof

Owned paths: narrow `sta_sim` lib/command/view/spawn/ownership/weapons/snapshot
integration, new perception module/tests, `sta_core` revision, affected view
literal constructors and existing snapshot fixtures/tests, this contract,
WORKPLAN seam revision notes, append-only TODO checks. No mission binding is
implemented until this separate seam is merged. No original assets or binaries
are edited, executed or distributed.

Required checks: privilege/validation atomicity; default/explicit/reset/transfer
identity; non-reused IDs and despawn cleanup; authoritative control/economy;
autotarget deception versus explicit attack and real-team cloak detection;
view queries; old edit tags; sorted command application; digest divergence and
default golden preservation; new snapshot corruption/round-trip/continuation;
historical snapshot admission and genuine v8 continuation; replay/view-log
revision rejection and new-state round trips. Run relevant shared tests,
strict Clippy, workspace all-target checks, ARM64 client compilation and full CI.
None of these checks is native in-game or Android device acceptance.

The LUA-05 zero-missing closure gate remains unchanged. This prerequisite adds
no exact A2 callable coverage until a separately tested adapter consumes it.
