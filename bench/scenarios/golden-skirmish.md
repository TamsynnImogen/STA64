# golden-skirmish: skirmish with its committed digest

**Golden (TOOL-06).** The last step pins the digest; CI runs this scenario on all four
targets. If a sim or fake-content change moves it on purpose, replace the value
(the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: skirmish Seed=11

A skirmish's first minutes, scripted with the orders a player gives. Player 1's
five cruisers and a frigate cross the map (`goto`, 600 ticks), then attack the
idle opponent (player 2: a station, a yard and a miner; it sends no command, its
station's weapons still answer). The frigate kills the miner on its own at tick 429; the
cruisers destroy the station at tick 3129 and, ordered on, the yard at tick 4368.
Player 2 is eliminated with exactly one `eliminated` mark, on that tick, and
player 1 loses nothing. On the fake content the station regenerates 1 shield a
tick, which is why the siege takes minutes.

1. Scene "goto c1,c2,c3,c4,c5 1900 0 0"
2. Scene "goto f1 1900 0 -250"
3. Run 600 ticks
4. Expect "c1" is within 20 of 1900 0 0
5. Expect player 2 is not eliminated
6. Scene "attack c1 base"
7. Scene "attack c2 base"
8. Scene "attack c3 base"
9. Scene "attack c4 base"
10. Scene "attack c5 base"
11. Run until tick 3600
12. Expect mark "death obj=miner" at tick 429
13. Expect mark "death obj=base" at tick 3129
14. Expect "yard" exists
15. Expect player 2 is not eliminated
16. Scene "attack c1 yard"
17. Scene "attack c2 yard"
18. Scene "attack c3 yard"
19. Scene "attack c4 yard"
20. Scene "attack c5 yard"
21. Run until tick 8000
22. Expect mark "death obj=yard" at tick 4368
23. Expect mark "eliminated player=2" at tick 4368
24. Expect no mark "eliminated player=2" between ticks 4369 and 8000
25. Expect player 2 is eliminated
26. Expect no ship is on team 2
27. Expect player 1 is not eliminated
28. Expect no mark "eliminated player=1" by tick 8000
29. Expect no mark "death obj=c1" by tick 8000
30. Expect 6 ships on team 1
31. Expect no mark "rejected" by tick 8000
32. Expect digest at tick 8000 is "405000c820d71837"
