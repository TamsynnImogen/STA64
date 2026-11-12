# golden-crossing: crossing with its committed digest

**Golden (TOOL-06).** The scene and commands of `crossing`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: crossing Seed=3

1. Scene "goto a0 1500 0 -200"
2. Scene "goto a1 1500 0 0"
3. Scene "goto a2 1500 0 200"
4. Scene "goto b0 -200 0 1500"
5. Scene "goto b1 0 0 1500"
6. Scene "goto b2 200 0 1500"
7. Run 10 ticks
8. Wait up to 90 seconds until "a1" is within 1 of 1500 0 0
9. Wait up to 90 seconds until "b1" is within 1 of 0 0 1500
10. Wait up to 90 seconds until "a0" is within 1 of 1500 0 -200
11. Wait up to 90 seconds until "a2" is within 1 of 1500 0 200
12. Wait up to 90 seconds until "b0" is within 1 of -200 0 1500
13. Wait up to 90 seconds until "b2" is within 1 of 200 0 1500
14. Run 60 ticks
15. Expect digest at tick 1244 is "04c091a5cc31f78f"
