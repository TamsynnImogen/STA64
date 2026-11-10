# construction-fo: a construction ship deploys under FO4

Tier: 1
Scene: construction-fo Seed=3

Profile FO4, the smooth movement model, on a construction ship with the
install's `fconst` values (`fx_fo_rig`: cruise 100, `tooCloseToTurn` 50).
Broke, the ship is refused its order up front (a rig that flew out and then
silently never built looked like a bug). Funded, it is sent to a site 400
ahead to deploy a station (1200, paid at once): the smooth model brakes it to
rest inside `tooCloseToTurn` of the site, so the deploy has to accept a rig
that stopped short. A station appears unfinished where the rig deploys and is
finished 1800 ticks later.

1. Scene "build rig fx_station -1600 0 0"
2. Run 2 ticks
4. Expect "bench.log" contains "rejected by the sim: build rig fx_station"
5. Scene "cargo yard dilithium 1500"
6. Run 1 ticks
7. Scene "build rig fx_station -1600 0 0"
8. Run 1 ticks
9. Expect player 1 resource dilithium at tick 4 is 300
10. Expect 1 stations on team 1
11. Wait up to 40 seconds until 2 stations are on team 1
12. Run 1 ticks
13. Run until tick 4000
14. Expect 2 stations on team 1
15. Expect "#3" is within 1 of -1600 0 0
