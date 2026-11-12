# golden-cloak: cloak with its committed digest

**Golden (TOOL-06).** The last step pins the digest; CI runs this scenario on all four
targets. If a sim or fake-content change moves it on purpose, replace the value
(the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: cloak Seed=5

Team 1's cloaker (`fx_cloaker`: beam fore, 600 special energy, cost 50, a
1 second engage and disengage) has an ally of team 2 beside it and an inert
enemy of team 3 400 ahead, inside beam range. Uncloaked it shoots the enemy
every 45 ticks. The order at tick 60 starts the engage: the cloak is up 30
steps later and `cloak ... on=1` is stamped on the last of them. From then on
the enemy no longer sees the ship while its ally does, and the ship does not
fire. The order at tick 200 shows it at once (`on=0`) but weapons stay off
until the 30 tick disengage is over.

1. Expect player 3 sees "warbird" at tick 0
2. Run until tick 60
3. Expect mark "hit obj=foe by=warbird" at tick 45
4. Scene "cloak warbird on"
5. Run until tick 200
6. Expect mark "cloak obj=warbird on=1" at tick 89
7. Expect player 3 sees "warbird" at tick 89
8. Expect player 3 does not see "warbird" at tick 90
9. Expect player 2 sees "warbird" at tick 90
10. Expect player 1 sees "warbird" at tick 90
11. Expect player 3 does not see "warbird" at tick 199
12. Expect no mark "hit obj=foe by=warbird" between ticks 46 and 199
13. Expect no mark "fire.start obj=warbird" between ticks 61 and 199
14. Scene "cloak warbird off"
15. Run until tick 320
16. Expect mark "cloak obj=warbird on=0" at tick 200
17. Expect player 3 does not see "warbird" at tick 200
18. Expect player 3 sees "warbird" at tick 201
19. Expect no mark "fire.start obj=warbird" between ticks 201 and 228
20. Expect mark "fire.start obj=warbird" at tick 229
21. Expect mark "hit obj=foe by=warbird" at tick 229
22. Expect mark "hit obj=foe by=warbird" at tick 274
23. Expect no mark "rejected" by tick 320
24. Expect digest at tick 320 is "276be9d057e049ef"
