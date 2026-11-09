# elimination-spare: a player with a unit left is not eliminated

Tier: 1
Scene: combat-elim-spare Seed=7

As `elimination`, but player 2 has a second unit (reserve) far from the fight.
Its first unit dies at tick 477 and no one is eliminated.

1. Run 600 ticks
2. Expect mark "death obj=target by=shooter" at tick 477
3. Expect "reserve" exists
4. Expect player 2 is not eliminated
5. Expect no mark "eliminated" by tick 600
