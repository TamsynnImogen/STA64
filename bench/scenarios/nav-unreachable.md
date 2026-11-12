# nav-unreachable: an unreachable goal ends at the nearest reachable point

Tier: 1
Scene: nav-unreachable Seed=3

Twelve overlapping fields of 300 on a ring of 900 wall in (0,0,3000). A
ship at the origin (body 75) is ordered to the walled-in centre. It flies
to the reachable point nearest the goal (a notch between two fields), on its
own side of the ring, and
its order ends there: it stops and stays stopped, outside every field.

1. Scene "goto ship 0 0 3000"
2. Run 10 ticks
3. Expect "ship" speed is above 0
4. Run 2400 ticks
5. Expect "ship" speed is 0
6. Expect "ship" is within 1700 of 0 0 3000
7. Expect "ship" z is below 3000
8. Run 300 ticks
9. Expect "ship" speed is 0
10. Expect "ship" was never within 1100 of 0 0 3000
11. Expect "ship" was never within 374 of 0 0 2100
12. Expect "ship" was never within 374 of -450 0 2221
13. Expect "ship" was never within 374 of 450 0 2221
14. Expect "ship" was never within 374 of -779 0 2550
15. Expect "ship" was never within 374 of 779 0 2550
