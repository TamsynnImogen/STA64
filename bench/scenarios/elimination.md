# elimination: the last unit of a player falls

Tier: 1
Scene: combat-elim Seed=7

Player 2's only unit (target) dies at tick 477, as in `explosion`. Player 2 is
eliminated with exactly one `eliminated` mark, on that tick. The attacker's
player 1 and the bystander (player 3, a third party on its own team) are not.
A seat made of allies is not expressible: every scene team is its own team.

1. Run 600 ticks
2. Expect player 2 is not eliminated at tick 477
3. Expect player 2 is eliminated at tick 478
4. Expect player 2 is eliminated at tick 600
5. Expect no mark "eliminated player=2" by tick 476
6. Expect mark "eliminated player=2" at tick 477
7. Expect no mark "eliminated player=2" between ticks 478 and 600
8. Expect no mark "eliminated player=1" by tick 600
9. Expect no mark "eliminated player=3" by tick 600
10. Expect player 1 is not eliminated
11. Expect player 3 is not eliminated
12. Expect mark "death obj=target by=shooter" at tick 477
