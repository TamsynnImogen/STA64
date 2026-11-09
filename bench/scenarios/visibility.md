# visibility: sensor range and shared vision

Tier: 1
Scene: visibility Seed=3

A station of team 1 has a sensor range of 1500; its ally, team 2, is far
away. An enemy cruiser (team 3, sensor range 1000) flies along the x axis
through the station's range. Players 1 and 2 see it exactly while it is
inside 1500 of the station (the ally shares vision); the enemy always sees
its own cruiser, sees the station only inside 1000, and never sees the far
ally. Cloak is in `cloak.md`.

1. Expect player 1 does not see "cruiser" at tick 0
2. Expect player 3 sees "cruiser" at tick 0
3. Expect player 1 sees "ally" at tick 0
4. Expect player 2 sees "station" at tick 0
5. Expect player 3 does not see "ally" at tick 0
6. Scene "goto cruiser 2000 0 0"
7. Run until tick 1500
8. Expect player 1 does not see "cruiser" at tick 121
9. Expect player 1 sees "cruiser" at tick 122
10. Expect player 2 sees "cruiser" at tick 122
11. Expect player 1 sees "cruiser" at tick 764
12. Expect player 1 does not see "cruiser" at tick 765
13. Expect player 2 does not see "cruiser" at tick 765
14. Expect player 3 sees "cruiser" at tick 767
15. Expect player 3 does not see "station" at tick 120
16. Expect player 3 sees "station" at tick 500
17. Expect player 3 does not see "station" at tick 1400
18. Expect player 3 does not see "ally" at tick 500
