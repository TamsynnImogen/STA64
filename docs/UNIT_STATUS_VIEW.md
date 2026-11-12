# Live unit status for the stock mouseover InfoBar

Pre-implementation contract, UI-18 continuation. Implements D1/D2/D5/D9.

The requested bar reads the hovered actor, independently of selection, and uses
resolved native `hull`, `shields`, `crew_amt`, `energy_amt`, `shipName`,
`icon_*` and `infoBarOffset_*` CFG fields. Current UnitView exposes hull and
shields but no unit crew or energy. Player crew stock and UnitDef crew capacity
must never substitute for the hovered actor's current crew.

## Shared prerequisite

Add `Entity.crew: Option<[Fixed; 2]>` for current and maximum crew. Crew is a
common physical unit pool like hull/shields. `Entity::new` keeps it unknown;
`Entity::from_def` initializes a real pool from nonnegative typed UnitDef.crew
for craft/stations, including explicit zero. All content-backed spawn paths,
including yard production and rig deployment, already use this constructor.
No player-stock debit, boarding, casualties or regeneration is added here.
The initialized pool is genuine canonical state; ownership changes preserve it,
removal removes it, and subsequent systems can modify it. No UI capacity fallback.

Expose optional `crew` and `energy` pairs in UnitView. Entity::view copies crew;
Sim::build_view joins existing Specials current energy and EnergyDef maximum.
An uninitialized energy row remains unknown until the existing special-energy
system initializes it; reads must not mutate or duplicate that system.

Keep EntityRec/Body snapshot serialization unchanged. Format 13 adds a mandatory
version-1 unit-crew suffix with ascending unique live EntityId and current/max
Fixed bits. Reject invalid bounds, unknown/dead IDs, duplicates, unsorted rows,
unknown suffix versions, missing suffix and trailing bytes. Versions 8–12 keep
all existing validation/migration rules and restore crew as unknown; never fill
historical crew from current definitions. Version 12 still consumes its player
personnel suffix; version 13 consumes both.

Hash a tagged, ordered crew suffix only when any known pool exists; include
known-zero presence. Unknown-only worlds preserve prior digests. Snapshot 13
round-trips known crew exactly. Bump SEAM_REV to 4; replay and view-log admission
must reject incompatible revision-3 histories. Update all view constructors and
synthetic fixtures. Multiplayer/replay simulation remains shared across platforms.

## Client consumer after the prerequisite merges

Use the existing Aim/unit_under picker, with fresh HUD hit testing and visible
shared-view admission. Clear hover when the cursor leaves, is over UI, the actor
is hidden/despawned, or the match changes. Hover never changes Selection or
issues commands. Render a separate native InfoBar at the current selected-panel
height; selection still owns the main detail card. Hover takes precedence in
that bar, with selected-single fallback when no actor is hovered. Read icons
through the existing GUI SPR chain. Respect zero-area fields and CFG colours;
verify native percentage formatting, sprite names and defaults before coding.
Unknown crew/energy stays blank rather than becoming a definition value.

## Validation and boundaries

Shared tests cover content/bare creation, known zero, production, ownership,
removal, live view/energy reads, hashing and mandatory/legacy snapshot admission.
Run relevant replay/view-log tests, strict Clippy and workspace/ARM64 checks.
Client tests cover hovered versus selected actors, fog/UI/strategic picking,
low/middle/tall origins, icons/colours, value changes and no stale data. Real
windowless mounted Classic/FO4 captures and an optimized candidate precede the
manual hover/selection/FPS check. Original files/executables remain read-only;
no original executable or hooks run and no GUI/input automation is used.

The existing December desktop alpha remains the priority. This is initialized
unit-status state and presentation, not full crew-economy/boarding/casualty parity.
The user merges the prerequisite before the client consumes it. Parent UI-18
remains open for its broader native-reference gate.


## Implementation evidence

The core/status snapshot suite passes, including a genuine 274-byte v12/seam3
fixture produced by preserved pre-change compiled libraries. The fixture and
`unit-status-v12.json` / `make-unit-status-v12.rs` record input hashes and the
producer guard; its original tick-5 and tick-35 digests still match. Player
crew stays 1200 and unit crew stays unknown despite a definition capacity of 300.

Current content-backed worlds now include initialized crew in their full
canonical hash. Nine earlier digest tests retain their old values as controls:
a test-only snapshot clone clears only the new crew pools, then checks the
original digest. Their new full digests are pinned separately. Historical
command extraction checks every old fixture digest through the same control;
the real replay path rejects incompatible/relabelled old history. Production
also checks that completed ships receive the canonical crew pool.

Core, simulation, bench, AI, mission and content unit suites pass (617 tests in
aggregate); the network crate has no library unit tests. Full client, strict
Clippy, workspace and Android checks are tracked in the prerequisite PR/private
receipts. These checks establish the data prerequisite, not the mouseover UI.
