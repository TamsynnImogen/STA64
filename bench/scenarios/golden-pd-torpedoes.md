# golden-pd-torpedoes: pd-torpedoes with its committed digest

**Golden (TOOL-06).** The scene and commands of `pd-torpedoes`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: combat-pd Seed=7

1. Run 200 ticks
2. Expect digest at tick 200 is "462fb1abc8dde988"
