# golden-cloak: cloak with its committed digest

**Golden (TOOL-06).** The scene and commands of `cloak`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: cloak Seed=5

1. Run until tick 60
2. Scene "cloak warbird on"
3. Run until tick 200
4. Scene "cloak warbird off"
5. Run until tick 320
6. Expect digest at tick 320 is "cfc7194f948520f6"
