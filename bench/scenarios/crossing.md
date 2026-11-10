# crossing: two groups cross paths and all arrive

Tier: 1
Scene: crossing Seed=3

Three ships fly east along the lines z = -200, 0 and 200 and three fly north
along x = -200, 0 and 200, so the groups meet at the origin. Ships keep out
of each other (the overlap bound is a sta_sim test, `avoid::tests`) and every
ship still arrives on its own point.

1. Scene "goto a0 1500 0 -200"
2. Scene "goto a1 1500 0 0"
3. Scene "goto a2 1500 0 200"
4. Scene "goto b0 -200 0 1500"
5. Scene "goto b1 0 0 1500"
6. Scene "goto b2 200 0 1500"
7. Run 10 ticks
8. Wait up to 90 seconds until "a1" is within 1 of 1500 0 0
9. Wait up to 90 seconds until "b1" is within 1 of 0 0 1500
10. Wait up to 90 seconds until "a0" is within 1 of 1500 0 -200
11. Wait up to 90 seconds until "a2" is within 1 of 1500 0 200
12. Wait up to 90 seconds until "b0" is within 1 of -200 0 1500
13. Wait up to 90 seconds until "b2" is within 1 of 200 0 1500
14. Run 60 ticks
15. Expect "a1" speed is 0
16. Expect "b1" speed is 0
