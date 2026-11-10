# move-waypoints-fo: a waypoint chain under FO4

Tier: 1
Scene: move-fo

Profile FO4, the smooth movement model, on craft with the install's physics
values: escort (FO 4.0 `smooth_physics_150`, cruise 180), cruiser
(`smooth_physics_100`, cruise 120), battle (`smooth_physics_60`, cruise 72).
The first two are sent along (0,1500), (1500,1500), (1500,3000) from their
start, the last two legs queued. With `pathLeadDistance` they move on to the
next leg before reaching a waypoint and take both corners near cruise speed,
stopping only at the end. Pins cruise speed through both corners, the tick
each comes to rest and where.

1. Scene "goto escort 0 0 1500"
2. Scene "goto escort 1500 0 1500 queue"
3. Scene "goto escort 1500 0 3000 queue"
4. Scene "goto cruiser 5000 0 1500"
5. Scene "goto cruiser 6500 0 1500 queue"
6. Scene "goto cruiser 6500 0 3000 queue"
7. Run 2 ticks
8. Wait up to 60 seconds until "escort" speed is 0
9. Expect "escort" speed at tick 131 is above 162.0
10. Expect "escort" speed at tick 191 is above 162.0
11. Expect "escort" speed at tick 251 is above 162.0
12. Expect "escort" speed at tick 311 is above 162.0
13. Expect "escort" speed at tick 371 is above 162.0
14. Expect "escort" speed at tick 431 is above 162.0
15. Expect "escort" speed at tick 491 is above 162.0
16. Expect "escort" speed at tick 782 is above 0
17. Expect "escort" speed at tick 783 is 0
18. Expect "escort" x at tick 783 is 1501.819 within 0.01
19. Expect "escort" z at tick 783 is 2999.733 within 0.01
20. Wait up to 60 seconds until "cruiser" speed is 0
21. Expect "cruiser" speed at tick 131 is above 108.0
22. Expect "cruiser" speed at tick 191 is above 108.0
23. Expect "cruiser" speed at tick 251 is above 108.0
24. Expect "cruiser" speed at tick 311 is above 108.0
25. Expect "cruiser" speed at tick 371 is above 108.0
26. Expect "cruiser" speed at tick 431 is above 108.0
27. Expect "cruiser" speed at tick 491 is above 108.0
28. Expect "cruiser" speed at tick 551 is above 108.0
29. Expect "cruiser" speed at tick 611 is above 108.0
30. Expect "cruiser" speed at tick 671 is above 108.0
31. Expect "cruiser" speed at tick 731 is above 108.0
32. Expect "cruiser" speed at tick 791 is above 108.0
33. Expect "cruiser" speed at tick 1109 is above 0
34. Expect "cruiser" speed at tick 1110 is 0
35. Expect "cruiser" x at tick 1110 is 6501.843 within 0.01
36. Expect "cruiser" z at tick 1110 is 2999.7 within 0.01
