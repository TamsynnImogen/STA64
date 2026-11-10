# move-start-stop-fo: start and stop under FO4

Tier: 1
Scene: move-fo

Profile FO4, the smooth movement model, on craft with the install's physics
values: escort (FO 4.0 `smooth_physics_150`, cruise 180), cruiser
(`smooth_physics_100`, cruise 120), battle (`smooth_physics_60`, cruise 72).
Each is sent 1500 straight ahead from rest. Acceleration is `1.3 combatSpeed
forwardAccel` and braking the same; a craft brakes to rest once inside
`tooCloseToTurn` of its point and below half its combat speed, so it may stop
a little short. Pins the speed after 10 and 30 ticks, the tick it comes to
rest and where.

1. Scene "goto escort 0 0 1500"
2. Scene "goto cruiser 5000 0 1500"
3. Scene "goto battle 10000 0 1500"
4. Run 2 ticks
5. Wait up to 60 seconds until "escort" speed is 0
6. Expect "escort" speed at tick 365 is above 0
7. Expect "escort" speed at tick 366 is 0
8. Expect "escort" z at tick 366 is 1500.0 within 0.01
9. Expect "escort" x at tick 366 is 0 within 0.01
10. Wait up to 60 seconds until "cruiser" speed is 0
11. Expect "cruiser" speed at tick 490 is above 0
12. Expect "cruiser" speed at tick 491 is 0
13. Expect "cruiser" z at tick 491 is 1500.0 within 0.01
14. Expect "cruiser" x at tick 491 is 5000 within 0.01
15. Wait up to 60 seconds until "battle" speed is 0
16. Expect "battle" speed at tick 740 is above 0
17. Expect "battle" speed at tick 741 is 0
18. Expect "battle" z at tick 741 is 1500.0 within 0.01
19. Expect "battle" x at tick 741 is 10000 within 0.01
20. Expect "escort" speed at tick 10 is 15.6 within 0.01
21. Expect "escort" speed at tick 30 is 46.8 within 0.01
22. Expect "cruiser" speed at tick 10 is 10.4 within 0.01
23. Expect "cruiser" speed at tick 30 is 31.2 within 0.01
24. Expect "battle" speed at tick 10 is 6.24 within 0.01
25. Expect "battle" speed at tick 30 is 18.72 within 0.01
