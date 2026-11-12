# nav-planet: a ship ordered through a planet routes round it

Tier: 1
Scene: avoid-planet Seed=3

The avoid-planet scene under the path planner: a ship (body 75) 1500 units
south of a planet (body 300) is ordered to the point 1500 units north of
it. The planner blocks the planet's cells (body radii times
`COLLISION_SAFETY_MARGIN`), the ship flies round, never touches the planet,
and arrives.

1. Scene "goto ship 0 0 1500"
2. Run 10 ticks
3. Expect "ship" speed is above 0
4. Wait up to 60 seconds until "ship" is within 1 of 0 0 1500
5. Run 30 ticks
6. Expect "ship" is within 1 of 0 0 1500
7. Expect "ship" was never within 375 of 0 0 0
