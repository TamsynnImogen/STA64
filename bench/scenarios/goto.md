# goto: queued waypoints, arrival, stop

Tier: 1
Scene: goto Seed=3

A cruiser is ordered to (0,0,600), then (500,20,600) and (500,0,-200) are
queued behind it. It visits the waypoints in order, arrives at tick 545 and
stands still on the last one. A second cruiser given a stop mid-leg halts and
drops its queue.

1. Scene "goto ship 0 0 600"
2. Scene "goto ship 500 20 600 queue"
3. Scene "goto ship 500 0 -200 queue"
4. Scene "goto runner 2000 0 3000"
5. Scene "goto runner 2000 0 4000 queue"
6. Scene "at 60 stop runner"
7. Run 10 ticks
8. Expect "ship" speed is above 0
9. Wait up to 20 seconds until "ship" is within 1 of 0 0 600
10. Expect "ship" is within 0.01 of 0 0 600 at tick 151
11. Run until tick 600
12. Expect "ship" speed at tick 540 is above 0
13. Expect "ship" is within 0.01 of 500 0 -200 at tick 545
14. Expect "ship" is within 0.01 of 500 0 -200
15. Expect "ship" speed is 0
16. Expect "runner" speed at tick 59 is above 0
17. Expect "runner" speed is 0
18. Expect "runner" z is below 1000
19. Run 120 ticks
20. Expect "runner" speed is 0
21. Expect "runner" z is below 1000
