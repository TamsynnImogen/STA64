# avoid-planet: a ship ordered through a planet flies round it

Tier: 1
Scene: avoid-planet Seed=3

A ship 1500 units south of a planet (body radius 300) is ordered to the
point 1500 units north of it, straight through the planet. It steers round
the planet and arrives on the point, instead of stalling at the planet's edge.

1. Scene "goto ship 0 0 1500"
2. Run 10 ticks
3. Expect "ship" speed is above 0
4. Wait up to 60 seconds until "ship" is within 1 of 0 0 1500
5. Run 30 ticks
6. Expect "ship" is within 1 of 0 0 1500
