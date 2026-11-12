# golden-skirmish: skirmish with its committed digest

**Golden (TOOL-06).** The scene and commands of `skirmish`, without its checks, and one pinned digest.
CI runs it on all four targets. If a sim or fake-content change moves the digest on purpose,
replace the value (the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: skirmish Seed=11

1. Scene "goto c1,c2,c3,c4,c5 1900 0 0"
2. Scene "goto f1 1900 0 -250"
3. Run 600 ticks
4. Scene "attack c1 base"
5. Scene "attack c2 base"
6. Scene "attack c3 base"
7. Scene "attack c4 base"
8. Scene "attack c5 base"
9. Run until tick 3600
10. Scene "attack c1 yard"
11. Scene "attack c2 yard"
12. Scene "attack c3 yard"
13. Scene "attack c4 yard"
14. Scene "attack c5 yard"
15. Run until tick 8000
16. Expect digest at tick 8000 is "405000c820d71837"
