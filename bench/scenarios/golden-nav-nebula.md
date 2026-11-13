# golden-nav-nebula: nav-nebula with its committed digest

**Golden (TOOL-06).** The scene and commands of `nav-nebula`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: nav-nebula Seed=3

1. Scene "goto ship 0 0 3000"
2. Run 10 ticks
3. Run until tick 736
4. Run 30 ticks
5. Expect digest at tick 766 is "0fd52738b4fecc13"
