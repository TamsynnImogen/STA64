# nav-nebula: a ship ordered across a hazard nebula flies round it

Tier: 1
Scene: nav-nebula Seed=3

The map has one Metreon gas nebula (stock Armada II `mnebula7.odf` values:
`pathing_cost = 2000`, damaging, cloak-disabling), a sphere of 400 units at
(0,0,1500). A ship at the origin (body 75) is ordered to (0,0,3000),
straight through it. The path planner (SIM-23) costs the nebula's cells at
its `pathing_cost` against open space's 100, so the ship routes round it and
arrives without its body ever touching the nebula.

1. Scene "goto ship 0 0 3000"
2. Run 10 ticks
3. Expect "ship" speed is above 0
4. Wait up to 90 seconds until "ship" is within 1 of 0 0 3000
5. Run 30 ticks
6. Expect "ship" is within 1 of 0 0 3000
7. Expect "ship" was never within 475 of 0 0 1500
