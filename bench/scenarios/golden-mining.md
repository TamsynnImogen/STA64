# golden-mining: mining with its committed digest

**Golden (TOOL-06).** The scene and commands of `mining`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: mining Seed=3

1. Scene "mine miner moon"
2. Run 1800 ticks
3. Expect digest at tick 1800 is "a1df541cdf292c61"
