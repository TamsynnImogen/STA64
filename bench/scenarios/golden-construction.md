# golden-construction: construction with its committed digest

**Golden (TOOL-06).** The scene and commands of `construction`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: construction Seed=3

1. Scene "cargo yard dilithium 1000"
2. Run 1 ticks
3. Scene "build yard fx_cruiser 0 0 0"
4. Scene "build yard fx_frigate 0 0 0"
5. Run 1 ticks
6. Scene "build yard fx_cruiser 0 0 0"
7. Scene "build yard fx_station 0 0 0"
8. Run 1 ticks
9. Run until tick 800
10. Run until tick 1000
11. Run until tick 1600
12. Scene "cargo yard dilithium 1500"
13. Run 1 ticks
14. Scene "build rig fx_station -1600 0 0"
15. Run 1 ticks
16. Run until tick 1677
17. Run 1 ticks
18. Run until tick 3900
19. Expect digest at tick 3900 is "c4edf52da5cd85f5"
