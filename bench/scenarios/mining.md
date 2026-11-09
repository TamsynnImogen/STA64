# mining: income per minute

Tier: 1
Scene: mining Seed=3

A freighter mines a moon that lies on its own station. A trip is 150 units
mined at 0.5 a tick plus 150 unloaded at 0.5 a tick, 600 ticks, so a minute
(1800 ticks) pays three trips, 450 dilithium. Deliveries land on ticks 599,
1199 and 1799; the stock fills while unloading, and no other resource moves.

1. Scene "mine miner moon"
2. Run 1800 ticks
3. Expect mark "delivered player=1 resource=0 amount=150" at tick 599
4. Expect mark "delivered player=1 resource=0 amount=150" at tick 1199
5. Expect mark "delivered player=1 resource=0 amount=150" at tick 1799
6. Expect no mark "delivered" by tick 598
7. Expect player 1 resource dilithium at tick 300 is below 5
8. Expect player 1 resource dilithium at tick 1800 is 450
9. Expect player 1 resource latinum at tick 1800 is 0
10. Expect "moon" exists
