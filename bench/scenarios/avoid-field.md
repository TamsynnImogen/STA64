# avoid-field: a ship ordered through an asteroid field flies round it

Tier: 1
Scene: avoid-field Seed=3

The map has one asteroid field, a sphere of 300 units at (0,0,1500). A ship
at the origin is ordered to (0,0,3000), straight through it. It steers round
the field and arrives on the point.

1. Scene "goto ship 0 0 3000"
2. Run 10 ticks
3. Expect "ship" speed is above 0
4. Wait up to 60 seconds until "ship" is within 1 of 0 0 3000
5. Run 30 ticks
6. Expect "ship" is within 1 of 0 0 3000
