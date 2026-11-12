# golden-planet: planet with its committed digest

**Golden (TOOL-06).** The scene and commands of `planet`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: planet Seed=7

1. Run 100 ticks
2. Expect digest at tick 100 is "0000000000000000"
