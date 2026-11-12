# Starting crew and crew capacity

Pre-code correction after the user's Classic starbase reference. Implements
D1/D2/D5/D9/D10/D11. crewCost is both the build cost and starting crew;
maximumCrew is a separate capacity, falling back to crewCost only when absent.
The installed fbase has crewCost150 and maximumCrew1500: its crew label stays
150 and its status is10%, below the faction's red threshold.

Add optional typed UnitDef.maximum_crew, defaultNone for Rust-authored content.
Read the effective inherited maximumCrew ODF value with the existing numeric
reader. Preserve explicit zero. Spawn the real current/capacity pair through
Entity::from_def, used by maps, starting fleets, scripts and production. No UI
capacity substitution, player-crew spending, regeneration or casualty changes.

Original A2 (read from the executable, static): `maximumCrew` is read into the
class capacity with `crewCost` as the missing-key fallback; the actor's maximum
is initialised from that capacity and its current crew from `crewCost`, with no
clamp between the two writes. Crew status divides actor current by actor maximum.

Keep explicit capacity below starting crew rather than silently rewriting
either authored value. The existing pool representation already holds both;
permit nonnegative over-capacity crew in snapshot validation so an authored
pool can save/restore itself. Negative pools, bad IDs/kinds/order/versions and
trailing bytes remain invalid. This relaxes one validation condition; record
layout/version stays unchanged. Missing old unit crew remains unknown on load.
Hash the effective capacity when it differs from the existing starting-crew
fallback, preserving Native fake hashes and equivalent explicit defaults.
Existing content-hash admission rejects stale interpretations; do not bypass it.

Tests cover missing/inherited/zero/partial capacity and initial crew, effective
hashes, production, nonnegative over-capacity persistence, unchanged negative
validation and genuine legacy fixtures. Verify the installed Classic pool and
red150 label/icon in a mounted-data GPU capture. Run shared/client regressions,
strict/workspace/ARM64 checks, full CI and an optimized manual candidate.
Original data and previous builds stay intact. December desktop alpha remains
the priority, with one shared simulation across platforms.

## Implementation evidence

All six focused content/simulation tests pass, including current150/capacity1500,
missing-key fallback, inherited zero, capacity-sensitive hashes, map/script/yard
creation and save continuation without refilling. Shared all-target and
bench/AI/mission suites pass; genuine legacy fixtures and Native default hashes
are retained. All468 client tests pass (one existing ignored).

Seven mounted Classic/FO4 NVIDIA captures pass. The explicit Classic fbase
fixture asserts a live150/1500 pool, red150 text and a red crew icon; the
framebuffer contains96 red text pixels and81 red icon pixels. Constructor
crew stays full/green. Strict content/sim/client all-target Clippy, workspace
and NDK29/API26 ARM64 client-library checks pass. No new APK/device claim.
The PR and private maximum-crew artifact receipt record the optimized build
and pending fresh-match visual/FPS acceptance.
