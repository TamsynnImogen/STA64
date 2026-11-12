# golden-firing: firing with its committed digest

**Golden (TOOL-06).** The scene and commands of `firing`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: combat-firing Seed=7

1. Run 200 ticks
2. Expect digest at tick 200 is "b16c259041841953"
