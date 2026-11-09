# cloak-loud: firing from a cloak that allows it decloaks the ship

Tier: 1
Scene: cloak-loud Seed=5

`fx_cloaker_loud` cloaks at once and keeps its weapons on (`disableWeapons =
0`). The enemy starts 2000 away, outside the beam's 600, so the cloak holds
and the enemy does not see the ship even once inside its own 1200 sensor
range. When the enemy sails into range after tick 100, the ship's
first shot drops the cloak: `cloak on=0` is stamped on the same
tick and the enemy sees it from the next.

1. Scene "special loud fx_cloak_loud"
2. Run until tick 100
3. Expect mark "cloak obj=loud on=1" at tick 0
4. Expect player 2 does not see "loud" at tick 99
5. Expect no mark "fire.start obj=loud" by tick 99
6. Scene "goto foe 500 0 0"
7. Run until tick 600
8. Expect player 2 does not see "loud" at tick 319
9. Expect mark "fire.start obj=loud" at tick 319
10. Expect mark "cloak obj=loud on=0" at tick 319
11. Expect player 2 sees "loud" at tick 320
12. Expect no mark "rejected" by tick 600
