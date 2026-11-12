# nav-barrier: a ship ordered across a map barrier goes through its gap

Tier: 1
Scene: nav-barrier Seed=3

A Fleet Operations style map barrier: hidden blocking fields
(`trigger_blocking`, `fieldRadius = 400`) every 600 units along z = 1500
from x = -6000 to 6000, with one gap between x = 1600 and 3200. A ship at
the origin (body 75) is ordered to (0,0,3000), straight into the wall. It
routes through the gap and arrives, never touching a field.

1. Scene "goto ship 0 0 3000"
2. Run 10 ticks
3. Expect "ship" speed is above 0
4. Wait up to 120 seconds until "ship" is within 1 of 0 0 3000
5. Run 30 ticks
6. Expect "ship" is within 1 of 0 0 3000
7. Expect "ship" was never within 475 of 0 0 1500
8. Expect "ship" was never within 475 of 1200 0 1500
9. Expect "ship" was never within 475 of 3600 0 1500
