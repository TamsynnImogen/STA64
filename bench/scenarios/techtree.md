# techtree: a build is rejected until its prerequisite exists

Tier: 1
Scene: construction Seed=3

The fake content's tech tree lets everything be built from the start except
`fx_dread` (a cruiser the yard lists), which needs a `fx_station`. The player
is given 3000 dilithium. A dread is rejected: nothing is paid. The rig deploys
a station (1200); while it welds the dread is still rejected (a station being
built does not count). Once the weld finishes the same order is accepted and
its cost (600) leaves the stock.

1. Scene "cargo yard dilithium 3000"
2. Run 1 ticks
3. Scene "build yard fx_dread 0 0 0"
4. Run 1 ticks
5. Expect mark "rejected player=1" at tick 1
6. Expect player 1 resource dilithium at tick 2 is 3000
7. Expect "bench.log" contains "rejected by the sim: build yard fx_dread"
8. Scene "build rig fx_station -1600 0 0"
9. Run 1 ticks
10. Expect player 1 resource dilithium at tick 3 is 1800
11. Wait up to 20 seconds until 2 stations are on team 1
12. Scene "build yard fx_dread 0 0 0"
13. Run 1 ticks
14. Expect player 1 resource dilithium at tick 99 is 1800
15. Run until tick 2200
16. Expect 2 stations on team 1
17. Scene "build yard fx_dread 0 0 0"
18. Run 1 ticks
19. Expect player 1 resource dilithium at tick 2201 is 1200
