# planet: a Galaxy beside a class M planet

Tier: 1
Scene: planet Seed=7

The scene is built, both objects are listed, nothing moves, and the same
scene on a second run (planet-repeat) has the same digest.

1. Expect "planet" exists
2. Expect "ship" exists
3. Expect 1 planet on team 0
4. Expect 1 ship on team 1
5. Expect scene "objects" answers "objects 2 -> objects.jsonl"
6. Run 100 ticks
7. Expect "ship" is within 0.01 of 1650 0 1750
8. Expect "planet" is within 0.01 of 2000 0 2000
9. Expect "ship" speed is 0
10. Expect no mark "death obj=ship" by tick 100
