# formation: a line abreast to a point

Tier: 1
Scene: formation Seed=3

Three frigates in a line abreast are sent to (0,0,1000). Each takes its own
slot, 60 units apart and centred on the point, and all three stop on them.

1. Scene "formation f1,f2,f3 1"
2. Scene "goto f1,f2,f3 0 0 1000"
3. Run 2 ticks
4. Wait up to 60 seconds until "f1" speed is 0
5. Run 30 ticks
6. Expect "f1" is within 0.01 of -60 0 1000
7. Expect "f2" is within 0.01 of 0 0 1000
8. Expect "f3" is within 0.01 of 60 0 1000
9. Expect "f1" speed is 0
10. Expect "f2" speed is 0
11. Expect "f3" speed is 0
