# construction-fo: a construction ship deploys under FO4

Tier: 1
Scene: construction-fo Seed=3

Profile FO4, the smooth movement model, on a construction ship with the
install's `fconst` values (`fx_fo_rig`: cruise 100, `tooCloseToTurn` 50). It
is sent to a site 400 ahead to deploy a station (1200, paid at once): the
smooth model brakes it to rest inside `tooCloseToTurn` of the site, so the
deploy has to accept a rig that stopped short. A station appears unfinished
where the rig deploys and is finished 1800 ticks later.

1. Scene "cargo yard dilithium 1500"
2. Run 1 ticks
3. Scene "build rig fx_station -1600 0 0"
4. Run 1 ticks
5. Expect player 1 resource dilithium at tick 2 is 300
6. Expect 1 stations on team 1
7. Wait up to 40 seconds until 2 stations are on team 1
8. Run 1 ticks
9. Run until tick 4000
10. Expect 2 stations on team 1
11. Expect "#3" is within 1 of -1600 0 0
