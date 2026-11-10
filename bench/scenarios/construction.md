# construction: yard queue, costs, rejection, rig deploy

Tier: 1
Scene: construction Seed=3

The player is given 1000 dilithium. The yard queues a cruiser (600) and a
frigate (250); each cost leaves the stock when the order is accepted, so 150
is left. A third cruiser (broke) and a station the yard does not build are
rejected and cost nothing. The queue builds in order: the cruiser 900 ticks
after it was queued, the frigate 450 after that. Then the stock is set to
1500 and the rig is sent to a site 400 units off to deploy a station (1200):
paid at once, a station appears unfinished at the site while the rig parks
beside it (SIM-21: the footprint's edge plus a gap, so it deploys 10 ticks
earlier than when it had to reach the site itself), and it is finished 1800
ticks later.

1. Scene "cargo yard dilithium 1000"
2. Run 1 ticks
3. Scene "build yard fx_cruiser 0 0 0"
4. Scene "build yard fx_frigate 0 0 0"
5. Run 1 ticks
6. Expect player 1 resource dilithium at tick 2 is 150
7. Scene "build yard fx_cruiser 0 0 0"
8. Scene "build yard fx_station 0 0 0"
9. Run 1 ticks
10. Expect mark "rejected player=1" at tick 2
11. Expect player 1 resource dilithium at tick 3 is 150
12. Expect "bench.log" contains "rejected by the sim: build yard fx_cruiser"
13. Run until tick 800
14. Expect no mark "built obj=yard" by tick 800
15. Run until tick 1000
16. Expect mark "built obj=yard" at tick 901
17. Run until tick 1600
18. Expect mark "built obj=yard" at tick 1351
19. Expect 3 ships on team 1
20. Scene "cargo yard dilithium 1500"
21. Run 1 ticks
22. Scene "build rig fx_station -1600 0 0"
23. Run 1 ticks
24. Expect player 1 resource dilithium at tick 1602 is 300
25. Expect 1 stations on team 1
26. Wait up to 20 seconds until 2 stations are on team 1
27. Run 1 ticks
28. Expect mark "build.start obj=rig what=fx_station" at tick 1676
29. Run until tick 3900
30. Expect mark "built obj=rig" at tick 3476
31. Expect 2 stations on team 1
32. Expect player 1 resource dilithium at tick 3900 is 300
33. Expect "#5" is within 1 of -1600 0 0
