# snapshot-start: resume the saved freighters run

Tier: 1
Scene: snapshot-start

The second half: the world saved by `snapshot-save` on tick 1200 is loaded
(`Start=snapshot:`), including the freighter's mid-trip state, its order and
the player's stock. It runs on and has the same digest as the continued run
300 ticks later (tick 1500) and at tick 3000, the same delivery and the same
stock.

1. Expect "bench.log" contains "scene built at tick 1200"
2. Expect player 1 resource dilithium at tick 1200 is between 150 and 180
3. Run until tick 1500
4. Expect digest at tick 1500 to match run "snapshot-save"
5. Run until tick 3000
6. Expect digest at tick 3000 to match run "snapshot-save"
7. Expect mark "delivered player=1" between ticks 2850 and 2950
8. Expect player 1 resource dilithium at tick 3000 is between 300 and 330
9. Expect "bench.log" does not contain "rejected"
