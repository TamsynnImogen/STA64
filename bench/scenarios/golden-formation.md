# golden-formation: formation with its committed digest

**Golden (TOOL-06).** The scene and commands of `formation`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: formation Seed=3

1. Scene "formation f1,f2,f3 1"
2. Scene "goto f1,f2,f3 0 0 1000"
3. Run 2 ticks
4. Run until tick 165
5. Run 30 ticks
6. Expect digest at tick 195 is "472f2c2ce134e031"
