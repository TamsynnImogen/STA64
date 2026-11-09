# explosion: a unit at zero hull is destroyed

Tier: 1
Scene: combat-duel Seed=7

The same duel run to the end: the beam and torpedo take the aft shield down
at tick 117, then the hull in steps (299.25 at 118, 275 at 181, 221.75 at 208,
...) until the torpedo of tick 477 brings it to zero. The unit is destroyed
on the tick of the lethal hit, with the shooter as killer, and is gone after.

1. Run 500 ticks
2. Expect "target" exists at tick 477
3. Expect "target" hull at tick 477 is 42.5 within 0.01
4. Expect no mark "death obj=target" by tick 476
5. Expect mark "death obj=target by=shooter" at tick 477
6. Expect "target" does not exist at tick 478
7. Expect "target" does not exist at tick 500
8. Expect "shooter" exists at tick 500
9. Expect no mark "death obj=shooter" by tick 500
