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

Pinned original-A2 static evidence (Armada2.exe SHA256
8d3ba6ec826198daf3e0c7b3435e9ce8195293e56da7c87e0af2634eaa4693b3):
0x4bf24d..0x4bf265 reads maximumCrew into class+0x200 with crewCost(class+0x84)
as the missing-key fallback. 0x4c1a6c..0x4c1a92 independently initializes
actor maximum+0x1c4 from that capacity and current+0x1dc from crewCost.
There is no constructor clamp between these two writes. Crew status at
0x4ed2d9..0x4ed2e5 divides actor current by actor maximum.

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
